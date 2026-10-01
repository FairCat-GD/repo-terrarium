//! The one blend rule: how a colour meets the colour below it.
//!
//! Everything happens in gamma-encoded sRGB, with nothing linearized. Each draw meets the
//! pixels below it as
//!
//! ```text
//! out = dst + (blend(dst, src) − dst) × opacity / 255
//!
//! blend: normal    src
//!        multiply  dst × src / 255
//!        add       min(dst + src, 255)
//! ```
//!
//! computed exactly in integers and rounded to the nearest value once, at the end:
//!
//! ```text
//! out = (dst × (255 − opacity) × 255 + B × opacity + 32512) / 65025
//! B   = src × 255 | dst × src | min(dst + src, 255) × 255
//! ```
//!
//! 65025 is odd, so a result never lies exactly halfway and the rounding has no tie to break.
//! The palette crossfade and dim use the same rule: a normal blend toward the night colour, or
//! toward black.

use crate::catalog::Blend;
use crate::frame::Rgba;

const OPAQUE: u8 = u8::MAX;
const FULL: u32 = 255 * 255;
const HALF: u32 = FULL / 2;

/// One channel of `src` drawn over `dst` at `opacity` with `mode`.
#[inline]
pub fn blend(dst: u8, src: u8, opacity: u8, mode: Blend) -> u8 {
    let (d, s, a) = (u32::from(dst), u32::from(src), u32::from(opacity));
    let blended = match mode {
        Blend::Normal => s * 255,
        Blend::Multiply => d * s,
        Blend::Add => (d + s).min(255) * 255,
    };
    // The exact value lies in 0..=255, so the conversion never saturates.
    u8::try_from((d * (255 - a) * 255 + blended * a + HALF) / FULL).unwrap_or(u8::MAX)
}

/// A channel of the palette crossfade: day at 0, night at 255.
pub fn mix(day: u8, night: u8, by: u8) -> u8 {
    blend(day, night, by, Blend::Normal)
}

/// A colour darkened toward black by `dim`, as the frame's `dim` darkens every pixel.
pub fn dim(colour: Rgba, by: u8) -> Rgba {
    let [r, g, b, _] = colour;
    [darken(r, by), darken(g, by), darken(b, by), OPAQUE]
}

fn darken(channel: u8, by: u8) -> u8 {
    blend(channel, 0, by, Blend::Normal)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The nearest integer to the exact blend, computed independently in floating point.
    fn reference(dst: u8, src: u8, opacity: u8, mode: Blend) -> f64 {
        let (d, s, a) = (f64::from(dst), f64::from(src), f64::from(opacity) / 255.0);
        let blended = match mode {
            Blend::Normal => s,
            Blend::Multiply => d * s / 255.0,
            Blend::Add => (d + s).min(255.0),
        };
        d + (blended - d) * a
    }

    #[test]
    fn every_blend_is_the_exact_value_rounded_to_nearest() {
        for mode in [Blend::Normal, Blend::Multiply, Blend::Add] {
            for opacity in [1, 26, 34, 64, 100, 127, 128, 150, 191, 254, 255] {
                for dst in 0..=255 {
                    for src in 0..=255 {
                        let got = f64::from(blend(dst, src, opacity, mode));
                        let exact = reference(dst, src, opacity, mode);
                        assert!(
                            (got - exact).abs() < 0.5,
                            "{mode:?} {dst} {src} at {opacity}: {got} for {exact}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_blend_modes_meet_their_definitions_at_the_extremes() {
        for value in [0, 1, 77, 128, 254, 255] {
            for mode in [Blend::Normal, Blend::Multiply, Blend::Add] {
                assert_eq!(blend(value, 200, 0, mode), value);
            }
            assert_eq!(blend(value, 200, 255, Blend::Normal), 200);
            assert_eq!(blend(value, 255, 255, Blend::Multiply), value);
            assert_eq!(blend(value, 0, 255, Blend::Multiply), 0);
            assert_eq!(blend(value, 0, 255, Blend::Add), value);
            assert_eq!(blend(value, 255, 255, Blend::Add), 255);
        }
        // Hand-checked: 100 × 200 / 255 = 78.43; 200 + 100 clamps to 255; half of each.
        assert_eq!(blend(100, 200, 255, Blend::Multiply), 78);
        assert_eq!(blend(200, 100, 255, Blend::Add), 255);
        assert_eq!(blend(100, 200, 128, Blend::Multiply), 89);
        assert_eq!(blend(100, 50, 128, Blend::Add), 125);
    }
}
