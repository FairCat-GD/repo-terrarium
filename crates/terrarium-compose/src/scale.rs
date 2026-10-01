//! The integer scale policy and the upscaler that applies it, shared by the presenter and every
//! export. Scaling is integer only, with nearest sampling, and the art is never cropped or
//! stretched (I-11).
//!
//! The upscaler can rewrite only what changed since the last image ([`Damage`]); the result is
//! the same, pixel for pixel, as a full redraw.

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

    /// Exactly `k` times the native image, with no padding: what exports are written at.
    pub fn exact(k: u32) -> Self {
        let scale = k.max(1);
        Self {
            scale,
            width: NATIVE_WIDTH.saturating_mul(scale),
            height: NATIVE_HEIGHT.saturating_mul(scale),
            image_x: 0,
        }
    }

    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }
}

/// Each native row's padding colour, continued to the monitor's edges either side.
pub type Padding = [Rgba; NATIVE_HEIGHT as usize];

/// What changed between two images: a rectangle of native pixels, and whether the padding
/// beside any of its rows changed too. Rows whose padding changed are rewritten whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damage {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub padding: bool,
}

impl Damage {
    /// Everything: a full redraw.
    pub const FULL: Damage = Damage {
        x: 0,
        y: 0,
        w: NATIVE_WIDTH,
        h: NATIVE_HEIGHT,
        padding: true,
    };

    /// The smallest damage that turns the first image into the second: the bounding box of
    /// the changed pixels and of the rows whose padding changed. `None` if nothing did.
    pub fn between(
        before: &Rgba640x74,
        before_padding: &Padding,
        after: &Rgba640x74,
        after_padding: &Padding,
    ) -> Option<Damage> {
        let mut rows: Option<(u32, u32)> = None;
        let mut columns: Option<(u32, u32)> = None;
        let mut padding = false;
        for (y, (old_pad, new_pad)) in
            (0..NATIVE_HEIGHT).zip(before_padding.iter().zip(after_padding))
        {
            let mut changed = (0..NATIVE_WIDTH)
                .zip(before.row(y).iter().zip(after.row(y)))
                .filter(|(_, (old, new))| old != new)
                .map(|(x, _)| x);
            let first = changed.next();
            if let Some(first) = first {
                let last = changed.next_back().unwrap_or(first);
                columns = Some(columns.map_or((first, last), |(l, r)| (l.min(first), r.max(last))));
            }
            let padded = old_pad != new_pad;
            if first.is_some() || padded {
                rows = Some(rows.map_or((y, y), |(top, _)| (top, y)));
            }
            padding |= padded;
        }
        let (top, bottom) = rows?;
        let (left, right) = columns.map_or((0, 0), |(l, r)| (l, r + 1));
        Some(Damage {
            x: left,
            y: top,
            w: right - left,
            h: bottom + 1 - top,
            padding,
        })
    }
}

/// A rectangle of the destination, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
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
    padding: &Padding,
    pack: impl Fn(Rgba) -> T,
    dst: &mut [T],
) -> Result<(), SizeMismatch> {
    upscale_damage_into(frame, scale, padding, pack, dst, &Damage::FULL).map(|_| ())
}

/// As [`upscale_into`], but writes only what `damage` covers, and returns that area of `dst`.
/// Over a destination that holds the image the damage was measured from, the result equals a
/// full redraw. A damage reaching past the frame is cut at its edge.
pub fn upscale_damage_into<T: Copy>(
    frame: &Rgba640x74,
    scale: &StripScale,
    padding: &Padding,
    pack: impl Fn(Rgba) -> T,
    dst: &mut [T],
    damage: &Damage,
) -> Result<PhysicalRect, SizeMismatch> {
    if dst.len() != scale.pixel_count() {
        return Err(SizeMismatch {
            expected: scale.pixel_count(),
            actual: dst.len(),
        });
    }
    let top = damage.y.min(NATIVE_HEIGHT);
    let bottom = damage.y.saturating_add(damage.h).min(NATIVE_HEIGHT);
    // Rows whose padding changed are written whole: padding and every native column.
    let (left, right) = if damage.padding {
        (0, NATIVE_WIDTH)
    } else {
        let left = damage.x.min(NATIVE_WIDTH);
        (
            left,
            damage.x.saturating_add(damage.w).clamp(left, NATIVE_WIDTH),
        )
    };
    let k = scale.scale;
    let (span_start, span_end) = if damage.padding {
        (0, scale.width)
    } else {
        (scale.image_x + left * k, scale.image_x + right * k)
    };
    let width = scale.width as usize;
    let k_rows = k as usize;
    let image_x = scale.image_x as usize;
    let image_end = image_x + NATIVE_WIDTH as usize * k_rows;
    let span = span_start as usize..span_end as usize;
    for native_y in top..bottom {
        let start = native_y as usize * k_rows * width;
        let rows = &mut dst[start..start + k_rows * width];
        let (first, rest) = rows.split_at_mut(width);
        if damage.padding {
            let pad = pack(padding[native_y as usize]);
            first[..image_x].fill(pad);
            first[image_end..].fill(pad);
        }
        let source = &frame.row(native_y)[left as usize..right as usize];
        let target =
            &mut first[image_x + left as usize * k_rows..image_x + right as usize * k_rows];
        for (block, &colour) in target.chunks_exact_mut(k_rows).zip(source) {
            block.fill(pack(colour));
        }
        for row in rest.chunks_exact_mut(width) {
            row[span.clone()].copy_from_slice(&first[span.clone()]);
        }
    }
    Ok(PhysicalRect {
        x: span_start,
        y: top * k,
        w: span_end - span_start,
        h: (bottom - top) * k,
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use proptest::test_runner::RngSeed;

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

    #[test]
    fn exact_scales_have_no_padding() {
        assert_eq!(
            StripScale::exact(3),
            StripScale {
                scale: 3,
                width: 1920,
                height: 222,
                image_x: 0
            }
        );
        assert_eq!(StripScale::exact(0).scale, 1);
    }

    #[test]
    fn damage_is_the_bounding_box_of_what_changed() {
        let before = gradient();
        let pad = padding();
        assert_eq!(Damage::between(&before, &pad, &before, &pad), None);

        let mut after = before.clone();
        after.set_pixel(7, 3, [1, 1, 1, 255]);
        after.set_pixel(20, 9, [1, 1, 1, 255]);
        assert_eq!(
            Damage::between(&before, &pad, &after, &pad),
            Some(Damage {
                x: 7,
                y: 3,
                w: 14,
                h: 7,
                padding: false
            })
        );

        let mut moved = pad;
        moved[40] = [0, 0, 0, 0];
        assert_eq!(
            Damage::between(&before, &pad, &before, &moved),
            Some(Damage {
                x: 0,
                y: 40,
                w: 0,
                h: 1,
                padding: true
            })
        );
    }

    #[test]
    fn a_damage_write_reports_the_physical_area_it_covered() {
        let scale = full(1301);
        let frame = gradient();
        let mut dst = vec![[0u8; 4]; scale.pixel_count()];
        let damage = Damage {
            x: 5,
            y: 2,
            w: 3,
            h: 4,
            padding: false,
        };
        let area = upscale_damage_into(&frame, &scale, &padding(), |c| c, &mut dst, &damage);
        assert_eq!(
            area,
            Ok(PhysicalRect {
                x: 10 + 10,
                y: 4,
                w: 6,
                h: 8
            })
        );
        let rows = Damage {
            padding: true,
            ..damage
        };
        let area = upscale_damage_into(&frame, &scale, &padding(), |c| c, &mut dst, &rows);
        assert_eq!(
            area,
            Ok(PhysicalRect {
                x: 0,
                y: 4,
                w: 1301,
                h: 8
            })
        );
    }

    fn redraw(frame: &Rgba640x74, scale: &StripScale, pad: &Padding) -> Vec<Rgba> {
        let mut dst = vec![[0u8; 4]; scale.pixel_count()];
        upscale_into(frame, scale, pad, |c| c, &mut dst).unwrap();
        dst
    }

    /// Property tests run from a fixed seed and write no regression files into the tree.
    fn config() -> ProptestConfig {
        ProptestConfig {
            cases: 48,
            rng_seed: RngSeed::Fixed(0x7465_7272_6172_6975),
            failure_persistence: None,
            ..ProptestConfig::default()
        }
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn writing_only_the_damage_equals_a_full_redraw(
            width in prop::sample::select(vec![640_u32, 1281, 1301, 1925]),
            edits in prop::collection::vec((0_u32..640, 0_u32..74, 1_u32..80, 1_u32..20, any::<u8>()), 0..4),
            padded in prop::collection::vec((0_usize..74, any::<u8>()), 0..3),
        ) {
            let scale = full(width);
            let (before, before_pad) = (gradient(), padding());
            let mut after = before.clone();
            for (x, y, w, h, value) in edits {
                for yy in y..(y + h).min(NATIVE_HEIGHT) {
                    for xx in x..(x + w).min(NATIVE_WIDTH) {
                        after.set_pixel(xx, yy, [value, value, value, 255]);
                    }
                }
            }
            let mut after_pad = before_pad;
            for (y, value) in padded {
                after_pad[y] = [value, 0, 0, 255];
            }
            let mut dst = redraw(&before, &scale, &before_pad);
            if let Some(damage) = Damage::between(&before, &before_pad, &after, &after_pad) {
                upscale_damage_into(&after, &scale, &after_pad, |c| c, &mut dst, &damage).unwrap();
            }
            prop_assert!(dst == redraw(&after, &scale, &after_pad));
        }
    }
}
