//! The integer scale policy and the upscaler that applies it. Scaling is integer only, with
//! nearest sampling, and the art is never cropped or stretched (I-11).

use crate::frame::{NATIVE_HEIGHT, NATIVE_WIDTH, Rgba, Rgba640x74};

/// The "one step smaller" choice (`strip.size_step`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SizeStep {
    #[default]
    Full,
    OneSmaller,
}

/// The strip's size in physical pixels on one monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StripScale {
    /// Physical pixels per native pixel, at least 1.
    pub scale: u32,
    /// The strip's width: the monitor's, or the native image's if the monitor is narrower.
    pub width: u32,
    /// `74 × scale`: the strip is exactly as tall as the scaled image.
    pub height: u32,
    /// Where the scaled image starts; the columns either side of it are padding.
    pub image_x: u32,
}

impl StripScale {
    /// The largest integer scale whose image fits `width`, one step smaller if asked, and never
    /// below 1. A monitor narrower than 640 physical pixels still gets the whole 1× image.
    pub fn for_physical_width(width: u32, step: SizeStep) -> Self {
        let fit = (width / NATIVE_WIDTH).max(1);
        let scale = match step {
            SizeStep::Full => fit,
            SizeStep::OneSmaller => (fit - 1).max(1),
        };
        let image_width = NATIVE_WIDTH.saturating_mul(scale);
        Self {
            scale,
            width: width.max(image_width),
            height: NATIVE_HEIGHT.saturating_mul(scale),
            image_x: width.saturating_sub(image_width) / 2,
        }
    }

    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }
}

/// The destination does not hold exactly `width × height` pixels.
#[derive(Debug, PartialEq, Eq)]
pub struct SizeMismatch {
    pub expected: usize,
    pub actual: usize,
}

/// Scales `frame` into `dst` (row-major, `scale.width` wide) with nearest sampling. Each
/// padding pixel takes its native row's colour from `padding`. `pack` turns a colour into the
/// destination's pixel format.
pub fn upscale_into<T: Copy>(
    frame: &Rgba640x74,
    scale: &StripScale,
    padding: &[Rgba; NATIVE_HEIGHT as usize],
    pack: impl Fn(Rgba) -> T,
    dst: &mut [T],
) -> Result<(), SizeMismatch> {
    if dst.len() != scale.pixel_count() {
        return Err(SizeMismatch {
            expected: scale.pixel_count(),
            actual: dst.len(),
        });
    }
    let width = scale.width as usize;
    let k = scale.scale as usize;
    let image_x = scale.image_x as usize;
    let image_end = image_x + NATIVE_WIDTH as usize * k;
    for (rows, native_y) in dst.chunks_exact_mut(width * k).zip(0..NATIVE_HEIGHT) {
        let (first, rest) = rows.split_at_mut(width);
        let pad = pack(padding[native_y as usize]);
        first[..image_x].fill(pad);
        first[image_end..].fill(pad);
        let source = frame.row(native_y);
        for (block, &colour) in first[image_x..image_end].chunks_exact_mut(k).zip(source) {
            block.fill(pack(colour));
        }
        for row in rest.chunks_exact_mut(width) {
            row.copy_from_slice(first);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full(width: u32) -> StripScale {
        StripScale::for_physical_width(width, SizeStep::Full)
    }

    #[test]
    fn the_scale_is_the_largest_integer_that_fits() {
        assert_eq!(
            full(3024),
            StripScale {
                scale: 4,
                width: 3024,
                height: 296,
                image_x: 232
            }
        );
        assert_eq!(full(1920).scale, 3);
        assert_eq!(full(1920).image_x, 0);
        assert_eq!(full(3840).scale, 6);
        assert_eq!(full(1279).scale, 1);
        assert_eq!(full(1280).scale, 2);
    }

    #[test]
    fn an_odd_remainder_puts_the_extra_column_on_the_right() {
        let scale = full(1281);
        assert_eq!((scale.scale, scale.image_x), (2, 0));
        let scale = full(643);
        assert_eq!((scale.scale, scale.image_x), (1, 1));
        assert_eq!(643 - 640 - scale.image_x, 2);
    }

    #[test]
    fn a_monitor_narrower_than_the_native_width_gets_the_whole_1x_image() {
        assert_eq!(
            full(600),
            StripScale {
                scale: 1,
                width: 640,
                height: 74,
                image_x: 0
            }
        );
        assert_eq!(full(0).scale, 1);
    }

    #[test]
    fn one_step_smaller_never_goes_below_1x() {
        let smaller = |w| StripScale::for_physical_width(w, SizeStep::OneSmaller).scale;
        assert_eq!(smaller(3024), 3);
        assert_eq!(smaller(1280), 1);
        assert_eq!(smaller(640), 1);
        assert_eq!(
            StripScale::for_physical_width(3024, SizeStep::OneSmaller).image_x,
            (3024 - 1920) / 2
        );
    }

    /// A frame where every pixel differs, so a sampling error cannot hide.
    fn gradient() -> Rgba640x74 {
        let mut frame = Rgba640x74::filled([0; 4]);
        for y in 0..NATIVE_HEIGHT {
            for x in 0..NATIVE_WIDTH {
                let [lo, hi, _, _] = u32::to_le_bytes(x);
                frame.set_pixel(x, y, [lo, hi, u8::try_from(y).unwrap(), 255]);
            }
        }
        frame
    }

    fn padding() -> [Rgba; 74] {
        std::array::from_fn(|y| [0xEE, 0xEE, u8::try_from(y).unwrap(), 0])
    }

    #[test]
    fn every_destination_pixel_is_its_native_source_or_its_row_padding() {
        let frame = gradient();
        let padding = padding();
        let scale = full(1301);
        assert_eq!((scale.scale, scale.image_x), (2, 10));
        let mut dst = vec![[0u8; 4]; scale.pixel_count()];
        upscale_into(&frame, &scale, &padding, |c| c, &mut dst).unwrap();
        for y in 0..scale.height {
            for x in 0..scale.width {
                let got = dst[(y * scale.width + x) as usize];
                let native_y = y / scale.scale;
                let inside = x >= scale.image_x && x < scale.image_x + 640 * scale.scale;
                let want = if inside {
                    frame.pixel((x - scale.image_x) / scale.scale, native_y)
                } else {
                    padding[native_y as usize]
                };
                assert_eq!(got, want, "at ({x}, {y})");
            }
        }
    }

    #[test]
    fn the_pack_function_sets_the_destination_format() {
        let scale = full(640);
        let mut dst = vec![0u32; scale.pixel_count()];
        let pack = |[r, g, b, _]: Rgba| u32::from_be_bytes([0, r, g, b]);
        upscale_into(&gradient(), &scale, &padding(), pack, &mut dst).unwrap();
        // Native (1, 1) is red 1, green 0, blue 1 (its row).
        assert_eq!(dst[641], 0x0001_0001);
    }

    #[test]
    fn a_destination_of_the_wrong_size_is_refused() {
        let scale = full(1280);
        let mut dst = vec![[0u8; 4]; scale.pixel_count() - 1];
        assert_eq!(
            upscale_into(&gradient(), &scale, &padding(), |c| c, &mut dst),
            Err(SizeMismatch {
                expected: scale.pixel_count(),
                actual: scale.pixel_count() - 1
            })
        );
    }
}
