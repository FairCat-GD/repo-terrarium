//! The pointer over the strip, in native pixels.

use serde::Serialize;
use terrarium_compose::frame::{NATIVE_HEIGHT, NATIVE_WIDTH};
use terrarium_compose::scale::StripScale;

/// Where the pointer is, in the scene's own 640×74 pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePoint {
    Scene {
        x: u32,
        y: u32,
    },
    /// Over the padding either side of the scaled image.
    Padding,
}

/// Maps a position in the window's physical pixels to the native scene.
pub fn to_native(x: f64, y: f64, scale: &StripScale) -> NativePoint {
    let (Some(x), Some(y)) = (whole_pixel(x), whole_pixel(y)) else {
        return NativePoint::Padding;
    };
    let Some(image_x) = x.checked_sub(scale.image_x) else {
        return NativePoint::Padding;
    };
    let (x, y) = (image_x / scale.scale, y / scale.scale);
    if x < NATIVE_WIDTH && y < NATIVE_HEIGHT {
        NativePoint::Scene { x, y }
    } else {
        NativePoint::Padding
    }
}

/// The pixel a coordinate falls in, or `None` left of or above the window.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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

    /// 4× with 232 px of padding each side, as on a 3024 px wide display.
    fn scale() -> StripScale {
        StripScale::for_physical_width(3024, SizeStep::Full)
    }

    #[test]
    fn physical_pixels_map_to_native_pixels() {
        let scale = scale();
        assert_eq!(
            to_native(232.0, 0.0, &scale),
            NativePoint::Scene { x: 0, y: 0 }
        );
        assert_eq!(
            to_native(235.9, 3.9, &scale),
            NativePoint::Scene { x: 0, y: 0 }
        );
        assert_eq!(
            to_native(236.0, 4.0, &scale),
            NativePoint::Scene { x: 1, y: 1 }
        );
        assert_eq!(
            to_native(2791.5, 295.5, &scale),
            NativePoint::Scene { x: 639, y: 73 }
        );
    }

    #[test]
    fn the_padding_and_the_outside_are_not_the_scene() {
        let scale = scale();
        assert_eq!(to_native(231.9, 10.0, &scale), NativePoint::Padding);
        assert_eq!(to_native(2792.0, 10.0, &scale), NativePoint::Padding);
        assert_eq!(to_native(500.0, 296.0, &scale), NativePoint::Padding);
        assert_eq!(to_native(-0.5, 10.0, &scale), NativePoint::Padding);
        assert_eq!(to_native(500.0, f64::NAN, &scale), NativePoint::Padding);
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
        assert_eq!(point, NativePoint::Scene { x: 17, y: 2 });
        assert_eq!(hover.last(), Some(point));
        assert!(!hover.enter());
        assert!(hover.leave());
        assert_eq!(hover.last(), None);
    }
}
