//! Putting composed pixels on screen. The `Presenter` is swappable: softbuffer first, a wgpu
//! backend later, chosen by measurement.

use std::num::NonZeroU32;
use std::rc::Rc;

use softbuffer::{Context, SoftBufferError, Surface};
use terrarium_compose::frame::{NativeImage, Rgba};
use terrarium_compose::scale::{SizeMismatch, StripScale, upscale_into};
use winit::window::Window;

pub trait Presenter {
    /// Scales `frame`, `scale.columns` wide, by `scale` into the window, and shows it.
    fn present(&mut self, frame: &NativeImage, scale: &StripScale) -> Result<(), PresentError>;
}

/// Its wording is in the copy module.
#[derive(Debug)]
pub enum PresentError {
    Surface(SoftBufferError),
    Size(SizeMismatch),
}

impl From<SoftBufferError> for PresentError {
    fn from(err: SoftBufferError) -> Self {
        Self::Surface(err)
    }
}

/// Presents through a CPU surface. The surface is always exactly the strip's size, so the
/// image is never stretched, even while the window is still settling on its size.
pub struct SoftbufferPresenter {
    surface: Surface<Rc<Window>, Rc<Window>>,
}

impl SoftbufferPresenter {
    pub fn new(window: &Rc<Window>) -> Result<Self, PresentError> {
        let context = Context::new(Rc::clone(window))?;
        Ok(Self {
            surface: Surface::new(&context, Rc::clone(window))?,
        })
    }
}

impl Presenter for SoftbufferPresenter {
    fn present(&mut self, frame: &NativeImage, scale: &StripScale) -> Result<(), PresentError> {
        let (Some(width), Some(height)) =
            (NonZeroU32::new(scale.width), NonZeroU32::new(scale.height))
        else {
            return Ok(());
        };
        self.surface.resize(width, height)?;
        let mut buffer = self.surface.buffer_mut()?;
        upscale_into(frame, scale, zero_rgb, &mut buffer).map_err(PresentError::Size)?;
        buffer.present()?;
        Ok(())
    }
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
    presented: u64,
}

impl FrameGate {
    /// A redraw arrived: whether to present it now.
    pub fn should_present(&mut self) -> bool {
        if self.occluded {
            self.held = true;
        }
        !self.occluded
    }

    pub fn presented(&mut self) {
        self.presented += 1;
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
        self.presented
    }
}

#[cfg(test)]
mod tests {
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
        gate.presented();
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
}
