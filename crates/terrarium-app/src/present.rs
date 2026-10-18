//! Putting composed pixels on screen. The `Presenter` is swappable: softbuffer scales on the
//! CPU, wgpu on the GPU, and both show the same pixels.

use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub mod gpu;
#[cfg(target_os = "macos")]
pub mod layer;

use softbuffer::{Context, Rect, Surface};
use terrarium_compose::frame::{NATIVE_HEIGHT, NativeImage, Rgba};
use terrarium_compose::scale::{Damage, SizeMismatch, StripScale, upscale_damage_into};
use terrarium_compose::schedule::MIN_FRAME_MS;
use winit::window::Window;

/// The ways the strip can be put on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// A CPU surface: the image is scaled on the CPU and handed to the OS.
    Softbuffer,
    /// A GPU surface: the native image is uploaded and the GPU scales it.
    Wgpu,
    /// macOS: a Core Animation layer handed the native image, which the window server scales.
    /// Elsewhere it is softbuffer.
    Layer,
}

impl Backend {
    /// The presenter the strip uses unless asked for another.
    pub const DEFAULT: Backend = if cfg!(target_os = "macos") {
        Backend::Layer
    } else {
        Backend::Softbuffer
    };
}

pub trait Presenter {
    /// Scales `frame`, `scale.columns` wide, by `scale` into the window, and shows it. With
    /// `whole`, everything is drawn afresh; otherwise only what changed since this presenter
    /// last showed a frame at this scale need be.
    fn present(
        &mut self,
        frame: &NativeImage,
        scale: &StripScale,
        whole: bool,
    ) -> Result<Shown, PresentError>;

    fn backend(&self) -> Backend;

    /// Whether this presenter's pixels for `frame` at `scale` equal the CPU upscale's, read
    /// back from where it draws; none where there is nothing to read back.
    fn pixels_equal(&mut self, _frame: &NativeImage, _scale: &StripScale) -> Option<bool> {
        None
    }
}

/// What a present did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    /// A frame went to the screen.
    Presented,
    /// The screen already held these pixels: nothing went.
    Unchanged,
    /// The OS had no surface for it now (covered, or not ready): nothing went, and the frame is
    /// owed.
    Skipped,
}

/// Its wording is in the copy module.
#[derive(Debug)]
pub enum PresentError {
    /// The window's surface could not be made, sized or shown: the backend's own words.
    Surface(String),
    Size(SizeMismatch),
    /// No GPU device could be had.
    Gpu(crate::gpu::GpuError),
    /// The GPU's surface offers no format that holds the pixels as they are.
    NoFormat,
    /// The GPU device or its surface was lost, or reported an error.
    Lost,
    /// The GPU's surface refused the frame it was asked for.
    Validation,
    /// The window's view could not take a layer for the picture (macOS).
    #[cfg(target_os = "macos")]
    NoLayer,
    /// The OS could not make an image of the picture (macOS).
    #[cfg(target_os = "macos")]
    NoImage,
}

impl From<softbuffer::SoftBufferError> for PresentError {
    fn from(err: softbuffer::SoftBufferError) -> Self {
        Self::Surface(err.to_string())
    }
}

/// Presents through a CPU surface. The surface is always exactly the strip's size, so the
/// image is never stretched, even while the window is still settling on its size.
pub struct SoftbufferPresenter {
    surface: Surface<Arc<Window>, Arc<Window>>,
    /// What the surface last showed, and at what scale: where damage is measured from.
    shown: Option<(StripScale, NativeImage)>,
}

impl SoftbufferPresenter {
    pub fn new(window: &Arc<Window>) -> Result<Self, PresentError> {
        let context = Context::new(Arc::clone(window))?;
        Ok(Self {
            surface: Surface::new(&context, Arc::clone(window))?,
            shown: None,
        })
    }

    fn remember(&mut self, frame: &NativeImage, scale: &StripScale) {
        match &mut self.shown {
            Some((shown_scale, image)) if image.width() == frame.width() => {
                *shown_scale = *scale;
                for y in 0..NATIVE_HEIGHT {
                    image.row_mut(y).copy_from_slice(frame.row(y));
                }
            }
            shown => *shown = Some((*scale, frame.clone())),
        }
    }
}

impl Presenter for SoftbufferPresenter {
    fn present(
        &mut self,
        frame: &NativeImage,
        scale: &StripScale,
        whole: bool,
    ) -> Result<Shown, PresentError> {
        let (Some(width), Some(height)) =
            (NonZeroU32::new(scale.width), NonZeroU32::new(scale.height))
        else {
            return Ok(Shown::Skipped);
        };
        self.surface.resize(width, height)?;
        let mut buffer = self.surface.buffer_mut()?;
        let last = self
            .shown
            .as_ref()
            .filter(|(shown, _)| shown == scale)
            .map(|(_, image)| image);
        let damage = match Rewrite::of(buffer.age(), whole, last, frame) {
            Rewrite::Nothing => return Ok(Shown::Unchanged),
            Rewrite::All => Damage::FULL,
            Rewrite::Area(damage) => damage,
        };
        let written = upscale_damage_into(frame, scale, zero_rgb, &mut buffer, &damage)
            .map_err(PresentError::Size)?;
        match written.and_then(rect).filter(|_| damage != Damage::FULL) {
            Some(rect) => buffer.present_with_damage(&[rect])?,
            None => buffer.present()?,
        }
        self.remember(frame, scale);
        Ok(Shown::Presented)
    }

    fn backend(&self) -> Backend {
        Backend::Softbuffer
    }
}

/// What a present rewrites of a buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rewrite {
    All,
    /// Only this area of native pixels: the buffer holds the last frame shown.
    Area(Damage),
    /// Nothing: the buffer holds these very pixels.
    Nothing,
}

impl Rewrite {
    /// For a buffer `age` presents old (softbuffer's count: 0 says nothing is known of it, 1
    /// that it holds the last frame shown), when the last frame shown at this scale was `last`.
    /// macOS hands out a fresh buffer every time, so only Windows ever rewrites less than all.
    fn of(age: u8, whole: bool, last: Option<&NativeImage>, frame: &NativeImage) -> Rewrite {
        match last {
            Some(last) if age == 1 && !whole => {
                Damage::between(last, frame).map_or(Rewrite::Nothing, Rewrite::Area)
            }
            _ => Rewrite::All,
        }
    }
}

/// softbuffer's rectangle for an area written, if it has any size.
fn rect(written: terrarium_compose::scale::PhysicalRect) -> Option<Rect> {
    Some(Rect {
        x: written.x,
        y: written.y,
        width: NonZeroU32::new(written.w)?,
        height: NonZeroU32::new(written.h)?,
    })
}

/// softbuffer's pixel format on every platform: `0x00RRGGBB`.
fn zero_rgb([r, g, b, _]: Rgba) -> u32 {
    u32::from_be_bytes([0, r, g, b])
}

/// Counts frames, and holds back those asked for while the strip is fully covered. Frames are
/// asked for on resize, re-placement and the OS's own expose requests, and by the living
/// strip only when its pixels change, so a still strip presents none (I-10).
#[derive(Debug, Default)]
pub struct FrameGate {
    occluded: bool,
    held: bool,
    /// Frames presented because their pixels changed.
    changed: u64,
    /// Frames presented again, unchanged, because the OS or the strip's own move asked.
    again: u64,
    /// When each frame of the last second went.
    recent: VecDeque<Instant>,
    /// The most frames that went in any one second, changed or again: the rate I-10 caps.
    busiest: u32,
    /// When the last frame went.
    last: Option<Instant>,
}

impl FrameGate {
    /// A redraw arrived: whether to present it now.
    pub fn should_present(&mut self) -> bool {
        if self.occluded {
            self.held = true;
        }
        !self.occluded
    }

    /// A frame went at `now`: its pixels `changed`, or the same pixels again.
    pub fn presented(&mut self, now: Instant, changed: bool) {
        self.last = Some(now);
        if changed {
            self.changed += 1;
        } else {
            self.again += 1;
        }
        let second = Duration::from_secs(1);
        while self
            .recent
            .front()
            .is_some_and(|&at| now.saturating_duration_since(at) >= second)
        {
            self.recent.pop_front();
        }
        self.recent.push_back(now);
        self.busiest = self
            .busiest
            .max(u32::try_from(self.recent.len()).unwrap_or(u32::MAX));
    }

    /// When a frame asked for at `now` may go, if not now: a fifteenth of a second after the
    /// last (I-10).
    pub fn paced(&self, now: Instant) -> Option<Instant> {
        let next = self.last? + Duration::from_millis(MIN_FRAME_MS);
        (now < next).then_some(next)
    }

    /// The OS had no surface for the frame: it is owed, as if the strip had been covered.
    pub fn skipped(&mut self) {
        self.held = true;
    }

    /// The strip was covered or uncovered. True when a held frame must now be asked for.
    pub fn set_occluded(&mut self, occluded: bool) -> bool {
        self.occluded = occluded;
        !occluded && std::mem::take(&mut self.held)
    }

    /// Whether the strip is fully covered.
    pub fn occluded(&self) -> bool {
        self.occluded
    }

    /// Frames presented since launch.
    pub fn count(&self) -> u64 {
        self.changed + self.again
    }

    /// Frames presented since launch: for a change of pixels, and again unchanged.
    pub fn counts(&self) -> (u64, u64) {
        (self.changed, self.again)
    }

    /// The most frames presented in any one second since launch.
    pub fn busiest_second(&self) -> u32 {
        self.busiest
    }
}

#[cfg(test)]
mod tests {
    use terrarium_compose::frame::Viewport;
    use terrarium_compose::scale::{SizeStep, upscale_into};

    use super::*;

    #[test]
    fn softbuffer_pixels_are_zero_red_green_blue() {
        assert_eq!(zero_rgb([0x12, 0x34, 0x56, 0x78]), 0x0012_3456);
    }

    #[test]
    fn a_visible_strip_presents_what_it_is_asked_for() {
        let mut gate = FrameGate::default();
        assert_eq!(gate.count(), 0);
        assert!(gate.should_present());
        gate.presented(Instant::now(), true);
        assert_eq!(gate.count(), 1);
    }

    #[test]
    fn a_covered_strip_holds_its_frame_until_uncovered() {
        let mut gate = FrameGate::default();
        assert!(!gate.set_occluded(true));
        assert!(!gate.should_present());
        assert!(!gate.should_present());
        assert!(gate.set_occluded(false));
        assert!(!gate.set_occluded(false));
        assert_eq!(gate.count(), 0);
    }

    #[test]
    fn uncovering_without_a_held_frame_asks_for_none() {
        let mut gate = FrameGate::default();
        gate.set_occluded(true);
        assert!(!gate.set_occluded(false));
    }

    #[test]
    fn a_frame_waits_a_fifteenth_of_a_second_after_the_last() {
        let mut gate = FrameGate::default();
        let start = Instant::now();
        assert_eq!(gate.paced(start), None);
        gate.presented(start, false);
        let next = start + Duration::from_millis(67);
        assert_eq!(gate.paced(start + Duration::from_millis(10)), Some(next));
        assert_eq!(gate.paced(next), None);
    }

    #[test]
    fn a_skipped_frame_is_owed_once_the_surface_is_back() {
        let mut gate = FrameGate::default();
        gate.skipped();
        assert!(gate.set_occluded(false));
    }

    #[test]
    fn frames_are_counted_by_cause_and_the_busiest_second_is_kept() {
        let mut gate = FrameGate::default();
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        // Fifteen changes 67 ms apart; three frames again inside them count in the second too.
        for i in 0..15 {
            gate.presented(at(i * 67), true);
        }
        assert_eq!(gate.busiest_second(), 15);
        for i in 0..3 {
            gate.presented(at(1_000 + i * 30), false);
        }
        assert_eq!(gate.counts(), (15, 3));
        assert_eq!(gate.busiest_second(), 17);
        // Eighteen inside one second is an eighteen.
        for i in 0..18 {
            gate.presented(at(10_000 + i * 50), true);
        }
        assert_eq!(gate.busiest_second(), 18);
    }

    fn image(colour: u8) -> NativeImage {
        NativeImage::new(Viewport::SCENE, [colour, colour, colour, u8::MAX])
    }

    #[test]
    fn only_a_buffer_holding_the_last_frame_is_rewritten_in_part() {
        let last = image(10);
        let mut next = last.clone();
        next.set_pixel(5, 6, [1, 2, 3, u8::MAX]);
        let area = Rewrite::of(1, false, Some(&last), &next);
        assert_eq!(
            area,
            Rewrite::Area(Damage {
                x: 5,
                y: 6,
                w: 1,
                h: 1,
                whole_rows: false
            })
        );
        // A fresh buffer, a forced redraw, or no frame shown at this scale: all of it.
        assert_eq!(Rewrite::of(0, false, Some(&last), &next), Rewrite::All);
        assert_eq!(Rewrite::of(1, true, Some(&last), &next), Rewrite::All);
        assert_eq!(Rewrite::of(1, false, None, &next), Rewrite::All);
        assert_eq!(Rewrite::of(2, false, Some(&last), &next), Rewrite::All);
        // The same pixels again rewrite nothing.
        assert_eq!(Rewrite::of(1, false, Some(&next), &next), Rewrite::Nothing);
    }

    #[test]
    fn a_partial_rewrite_over_the_last_frame_equals_a_whole_one() {
        let scale = StripScale::for_physical_width(1301, SizeStep::Full);
        let viewport = scale.viewport();
        let last = NativeImage::new(viewport, [10, 20, 30, u8::MAX]);
        let mut next = last.clone();
        next.set_pixel(viewport.width() - 1, 73, [200, 100, 50, u8::MAX]);
        next.set_pixel(3, 0, [1, 1, 1, u8::MAX]);
        let mut screen = vec![0_u32; scale.pixel_count()];
        upscale_into(&last, &scale, zero_rgb, &mut screen).expect("sized");
        let Rewrite::Area(damage) = Rewrite::of(1, false, Some(&last), &next) else {
            panic!("two pixels changed");
        };
        upscale_damage_into(&next, &scale, zero_rgb, &mut screen, &damage).expect("sized");
        let mut whole = vec![0_u32; scale.pixel_count()];
        upscale_into(&next, &scale, zero_rgb, &mut whole).expect("sized");
        let first_difference = screen.iter().zip(&whole).position(|(a, b)| a != b);
        assert_eq!(first_difference, None);
    }
}
