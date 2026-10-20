//! The pointer over the strip, in native pixels: as the probe records it, and as the soil panel
//! and the hover labels are hit-tested in, both in the viewport's columns.

use serde::Serialize;
use terrarium_compose::frame::NATIVE_HEIGHT;
use terrarium_compose::scale::StripScale;

/// Where the pointer is, in the strip's native pixels: the viewport's columns, which the
/// ground spreads across, so a point names what stands under the pointer at any width. Only the
/// letterbox, and anything outside the strip, is not the scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePoint {
    Scene {
        x: u32,
        y: u32,
    },
    /// Beside the scene: the letterbox, or outside the strip.
    Padding,
}

/// Maps a position in the window's physical pixels to the strip's native pixels.
pub fn to_native(x: f64, y: f64, scale: &StripScale) -> NativePoint {
    match to_viewport(x, y, scale) {
        Some(point) => NativePoint::Scene {
            x: point.x.cast_unsigned(),
            y: point.y.cast_unsigned(),
        },
        None => NativePoint::Padding,
    }
}

/// A pixel of the viewport: the scene and the columns either side of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportPoint {
    pub x: i32,
    pub y: i32,
}

/// Maps a position in the window's physical pixels to the viewport, after the integer scale;
/// `None` in the letterbox or outside the window.
pub fn to_viewport(x: f64, y: f64, scale: &StripScale) -> Option<ViewportPoint> {
    let (x, y) = (whole_pixel(x)?, whole_pixel(y)?);
    let x = x.checked_sub(scale.image_x)? / scale.scale;
    let y = y / scale.scale;
    (x < scale.viewport().width() && y < NATIVE_HEIGHT).then(|| ViewportPoint {
        x: x.cast_signed(),
        y: y.cast_signed(),
    })
}

/// The pixel a coordinate falls in, or `None` left of or above the window.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "checked to lie in u32's range, and floored"
)]
fn whole_pixel(coordinate: f64) -> Option<u32> {
    (coordinate >= 0.0 && coordinate < f64::from(u32::MAX)).then(|| coordinate.floor() as u32)
}

/// Whether the pointer is over the strip. Enter and leave can each be reported twice, by
/// winit's tracking rectangle and by the always-active tracking area, so repeats are ignored.
#[derive(Debug, Default)]
pub struct HoverTracker {
    inside: bool,
    last: Option<NativePoint>,
}

impl HoverTracker {
    /// True when the pointer was outside.
    pub fn enter(&mut self) -> bool {
        !std::mem::replace(&mut self.inside, true)
    }

    /// True when the pointer was inside.
    pub fn leave(&mut self) -> bool {
        self.last = None;
        std::mem::replace(&mut self.inside, false)
    }

    /// A move is inside by definition, even if the enter was missed.
    pub fn moved(&mut self, x: f64, y: f64, scale: &StripScale) -> NativePoint {
        let point = to_native(x, y, scale);
        self.inside = true;
        self.last = Some(point);
        point
    }

    pub fn last(&self) -> Option<NativePoint> {
        self.last
    }
}

#[cfg(test)]
mod tests {
    use terrarium_compose::scale::SizeStep;

    use super::*;

    /// 4× on a 3024 px wide display: 756 columns, the scene 58 of them (232 px) in.
    fn scale() -> StripScale {
        StripScale::for_physical_width(3024, SizeStep::Full)
    }

    #[test]
    fn physical_pixels_map_to_native_pixels_across_the_viewport() {
        let scale = scale();
        assert_eq!(
            to_native(0.0, 0.0, &scale),
            NativePoint::Scene { x: 0, y: 0 }
        );
        assert_eq!(
            to_native(235.9, 3.9, &scale),
            NativePoint::Scene { x: 58, y: 0 }
        );
        assert_eq!(
            to_native(236.0, 4.0, &scale),
            NativePoint::Scene { x: 59, y: 1 }
        );
        assert_eq!(
            to_native(3023.5, 295.5, &scale),
            NativePoint::Scene { x: 755, y: 73 }
        );
    }

    #[test]
    fn only_outside_the_strip_is_not_the_scene() {
        let scale = scale();
        assert_eq!(to_native(3024.0, 10.0, &scale), NativePoint::Padding);
        assert_eq!(to_native(500.0, 296.0, &scale), NativePoint::Padding);
        assert_eq!(to_native(-0.5, 10.0, &scale), NativePoint::Padding);
        assert_eq!(to_native(500.0, f64::NAN, &scale), NativePoint::Padding);
        // A screen wider than the widest viewport letterboxes either side.
        let wide = StripScale::for_physical_width(1700, SizeStep::Full);
        assert!(wide.image_x > 0);
        assert_eq!(
            to_native(f64::from(wide.image_x) - 1.0, 10.0, &wide),
            NativePoint::Padding
        );
    }

    #[test]
    fn physical_pixels_map_to_the_viewport_beside_the_scene_too() {
        let scale = scale();
        let at = |x, y| to_viewport(x, y, &scale);
        assert_eq!(at(0.0, 0.0), Some(ViewportPoint { x: 0, y: 0 }));
        assert_eq!(at(3.9, 3.9), Some(ViewportPoint { x: 0, y: 0 }));
        assert_eq!(at(4.0, 4.0), Some(ViewportPoint { x: 1, y: 1 }));
        // The scene's first column is the viewport's 58th.
        assert_eq!(at(232.0, 292.0), Some(ViewportPoint { x: 58, y: 73 }));
        assert_eq!(at(3023.9, 295.9), Some(ViewportPoint { x: 755, y: 73 }));
        assert_eq!(at(3024.0, 10.0), None);
        assert_eq!(at(10.0, 296.0), None);
        assert_eq!(at(-0.5, 10.0), None);
        assert_eq!(at(f64::NAN, 10.0), None);
    }

    #[test]
    fn repeated_enters_and_leaves_are_ignored() {
        let mut hover = HoverTracker::default();
        assert!(hover.enter());
        assert!(!hover.enter());
        assert!(hover.leave());
        assert!(!hover.leave());
    }

    #[test]
    fn a_move_without_an_enter_counts_as_inside() {
        let mut hover = HoverTracker::default();
        let point = hover.moved(300.0, 8.0, &scale());
        assert_eq!(point, NativePoint::Scene { x: 75, y: 2 });
        assert_eq!(hover.last(), Some(point));
        assert!(!hover.enter());
        assert!(hover.leave());
        assert_eq!(hover.last(), None);
    }
}
