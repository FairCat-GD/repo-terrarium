//! The integer scale policy and the upscaler that applies it, shared by the presenter and every
//! export. Scaling is integer only, with nearest sampling, and the art is never stretched
//! (I-11).
//!
//! A screen whose width is no multiple of 640 shows a wider viewport of the terrarium, as many
//! native columns as reach its edge, up to the widest the art covers: the last column is cut
//! where the screen ends. Only a screen wider still is letterboxed either side.
//!
//! The upscaler can rewrite only what changed since the last image ([`Damage`]); the result is
//! the same, pixel for pixel, as a full redraw.

use crate::frame::{NATIVE_HEIGHT, NATIVE_WIDTH, NativeImage, Rgba, Viewport, WIDEST};

/// The colour beyond the widest viewport the art covers.
pub const LETTERBOX: Rgba = [0, 0, 0, u8::MAX];

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
    /// The strip's width: the monitor's, or the scene's if the monitor is narrower.
    pub width: u32,
    /// `74 × scale`: the strip is exactly as tall as the scaled image.
    pub height: u32,
    /// The native columns composed: the viewport's width.
    pub columns: u32,
    /// Where the scaled image starts. Only an image narrower than the strip leaves letterbox
    /// either side; a wider one is cut at the strip's right edge, inside its last column.
    pub image_x: u32,
}

impl StripScale {
    /// The largest integer scale whose scene fits `width`, one step smaller if asked, and never
    /// below 1, with as many columns as reach the strip's edge. A monitor narrower than 640
    /// physical pixels still gets the whole 1× scene.
    pub fn for_physical_width(width: u32, step: SizeStep) -> Self {
        let fit = (width / NATIVE_WIDTH).max(1);
        let scale = match step {
            SizeStep::Full => fit,
            SizeStep::OneSmaller => (fit - 1).max(1),
        };
        let columns = Viewport::new(width.div_ceil(scale)).width();
        let image_width = columns.saturating_mul(scale);
        Self {
            scale,
            width: width.max(NATIVE_WIDTH.saturating_mul(scale)),
            height: NATIVE_HEIGHT.saturating_mul(scale),
            columns,
            image_x: width.saturating_sub(image_width) / 2,
        }
    }

    /// Exactly `k` times the scene, with no letterbox: what exports are written at.
    pub fn exact(k: u32) -> Self {
        StripScale::exact_viewport(k, Viewport::SCENE)
    }

    /// Exactly `k` times `viewport`'s columns.
    pub fn exact_viewport(k: u32, viewport: Viewport) -> Self {
        let scale = k.max(1);
        Self {
            scale,
            width: viewport.width().saturating_mul(scale),
            height: NATIVE_HEIGHT.saturating_mul(scale),
            columns: viewport.width(),
            image_x: 0,
        }
    }

    /// The viewport the strip composes.
    pub fn viewport(&self) -> Viewport {
        Viewport::new(self.columns)
    }

    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }
}

/// What changed between two images: a rectangle of native pixels, and whether its rows are
/// rewritten whole, letterbox included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damage {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub whole_rows: bool,
}

impl Damage {
    /// Everything: a full redraw.
    pub const FULL: Damage = Damage {
        x: 0,
        y: 0,
        w: WIDEST,
        h: NATIVE_HEIGHT,
        whole_rows: true,
    };

    /// The smallest damage that turns the first image into the second: the bounding box of
    /// the changed pixels, or everything if the two differ in width. `None` if nothing changed.
    pub fn between(before: &NativeImage, after: &NativeImage) -> Option<Damage> {
        if before.width() != after.width() {
            return Some(Damage::FULL);
        }
        let mut rows: Option<(u32, u32)> = None;
        let mut columns: Option<(u32, u32)> = None;
        for y in 0..NATIVE_HEIGHT {
            let mut changed = (0..before.width())
                .zip(before.row(y).iter().zip(after.row(y)))
                .filter(|(_, (old, new))| old != new)
                .map(|(x, _)| x);
            if let Some(first) = changed.next() {
                let last = changed.next_back().unwrap_or(first);
                columns = Some(columns.map_or((first, last), |(l, r)| (l.min(first), r.max(last))));
                rows = Some(rows.map_or((y, y), |(top, _)| (top, y)));
            }
        }
        let (top, bottom) = rows?;
        let (left, right) = columns?;
        Some(Damage {
            x: left,
            y: top,
            w: right + 1 - left,
            h: bottom + 1 - top,
            whole_rows: false,
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

/// Scales `frame` into `dst` (row-major, `scale.width` wide) with nearest sampling, cut at the
/// strip's edge, and letterboxes any width beyond it. `pack` turns a colour into the
/// destination's pixel format.
///
/// # Panics
/// If `frame` is not `scale.columns` wide.
pub fn upscale_into<T: Copy>(
    frame: &NativeImage,
    scale: &StripScale,
    pack: impl Fn(Rgba) -> T,
    dst: &mut [T],
) -> Result<(), SizeMismatch> {
    upscale_damage_into(frame, scale, pack, dst, &Damage::FULL).map(|_| ())
}

/// As [`upscale_into`], but writes only what `damage` covers, and returns that area of `dst`,
/// or `None` when it covers nothing.
/// Over a destination that holds the image the damage was measured from, the result equals a
/// full redraw. A damage reaching past the frame is cut at its edge.
///
/// # Panics
/// If `frame` is not `scale.columns` wide.
pub fn upscale_damage_into<T: Copy>(
    frame: &NativeImage,
    scale: &StripScale,
    pack: impl Fn(Rgba) -> T,
    dst: &mut [T],
    damage: &Damage,
) -> Result<Option<PhysicalRect>, SizeMismatch> {
    assert_eq!(
        frame.width(),
        scale.columns,
        "the frame is not as wide as the strip's columns"
    );
    if dst.len() != scale.pixel_count() {
        return Err(SizeMismatch {
            expected: scale.pixel_count(),
            actual: dst.len(),
        });
    }
    let columns = frame.width();
    let top = damage.y.min(NATIVE_HEIGHT);
    let bottom = damage.y.saturating_add(damage.h).min(NATIVE_HEIGHT);
    let (left, right) = if damage.whole_rows {
        (0, columns)
    } else {
        let left = damage.x.min(columns);
        (left, damage.x.saturating_add(damage.w).clamp(left, columns))
    };
    let k = scale.scale;
    let width = scale.width as usize;
    let k_rows = k as usize;
    let image_x = scale.image_x as usize;
    let image_end = (image_x + columns as usize * k_rows).min(width);
    let span = if damage.whole_rows {
        0..width
    } else {
        (image_x + left as usize * k_rows).min(width)
            ..(image_x + right as usize * k_rows).min(width)
    };
    for native_y in top..bottom {
        let start = native_y as usize * k_rows * width;
        let rows = &mut dst[start..start + k_rows * width];
        let (first, rest) = rows.split_at_mut(width);
        if damage.whole_rows {
            first[..image_x].fill(pack(LETTERBOX));
            first[image_end..].fill(pack(LETTERBOX));
        }
        let source = &frame.row(native_y)[left as usize..right as usize];
        let from = (image_x + left as usize * k_rows).min(width);
        let to = (image_x + right as usize * k_rows).min(width);
        // The last block is cut where the strip ends.
        for (block, &colour) in first[from..to].chunks_mut(k_rows).zip(source) {
            block.fill(pack(colour));
        }
        for row in rest.chunks_exact_mut(width) {
            row[span.clone()].copy_from_slice(&first[span.clone()]);
        }
    }
    let written = PhysicalRect {
        x: u32::try_from(span.start).unwrap_or(0),
        y: top * k,
        w: u32::try_from(span.len()).unwrap_or(0),
        h: (bottom - top) * k,
    };
    Ok((written.w > 0 && written.h > 0).then_some(written))
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
    fn the_scale_is_the_largest_integer_whose_scene_fits_and_the_columns_reach_the_edge() {
        // 3024 at 4×: 756 columns, 3024 exactly.
        assert_eq!(
            full(3024),
            StripScale {
                scale: 4,
                width: 3024,
                height: 296,
                columns: 756,
                image_x: 0
            }
        );
        // 2940 at 4×: 735 columns, exactly 2940.
        assert_eq!((full(2940).scale, full(2940).columns), (4, 735));
        // 1301 at 2×: 651 columns, the last cut to 1 of its 2 pixels.
        assert_eq!((full(1301).columns, full(1301).image_x), (651, 0));
        assert_eq!(full(1920).scale, 3);
        assert_eq!((full(1920).columns, full(1920).image_x), (640, 0));
        assert_eq!(full(3840).scale, 6);
        assert_eq!(full(1279).scale, 1);
        assert_eq!(full(1280).scale, 2);
    }

    #[test]
    fn past_the_widest_viewport_the_strip_is_letterboxed_either_side() {
        // 1900 at 2× would need 950 columns; the art covers 840.
        let scale = full(1900);
        assert_eq!((scale.scale, scale.columns), (2, WIDEST));
        assert_eq!(scale.image_x, (1900 - 2 * WIDEST) / 2);
        // An odd remainder puts the extra pixel on the right.
        let scale = full(1683);
        assert_eq!((scale.columns, scale.image_x), (WIDEST, 1));
    }

    #[test]
    fn a_monitor_narrower_than_the_scene_gets_the_whole_1x_scene() {
        assert_eq!(
            full(600),
            StripScale {
                scale: 1,
                width: 640,
                height: 74,
                columns: 640,
                image_x: 0
            }
        );
        assert_eq!(full(0).scale, 1);
    }

    #[test]
    fn one_step_smaller_never_goes_below_1x() {
        let smaller = |w| StripScale::for_physical_width(w, SizeStep::OneSmaller);
        assert_eq!((smaller(3024).scale, smaller(3024).columns), (3, WIDEST));
        assert_eq!(smaller(3024).image_x, (3024 - 3 * WIDEST) / 2);
        assert_eq!(smaller(1280).scale, 1);
        assert_eq!(smaller(640).scale, 1);
    }

    /// A frame `columns` wide where every pixel differs, so a sampling error cannot hide.
    fn gradient(columns: u32) -> NativeImage {
        let mut frame = NativeImage::new(Viewport::new(columns), [0; 4]);
        for y in 0..NATIVE_HEIGHT {
            for x in 0..columns {
                let [lo, hi, _, _] = u32::to_le_bytes(x);
                frame.set_pixel(x, y, [lo, hi, u8::try_from(y).unwrap(), 255]);
            }
        }
        frame
    }

    #[test]
    fn every_destination_pixel_is_its_native_source_or_letterbox() {
        for width in [1301, 1900, 2940] {
            let scale = full(width);
            let frame = gradient(scale.columns);
            let mut dst = vec![[0u8; 4]; scale.pixel_count()];
            upscale_into(&frame, &scale, |c| c, &mut dst).unwrap();
            for y in 0..scale.height {
                for x in 0..scale.width {
                    let got = dst[(y * scale.width + x) as usize];
                    let native_y = y / scale.scale;
                    let inside =
                        x >= scale.image_x && x < scale.image_x + scale.columns * scale.scale;
                    let want = if inside {
                        frame.pixel((x - scale.image_x) / scale.scale, native_y)
                    } else {
                        LETTERBOX
                    };
                    assert_eq!(got, want, "at ({x}, {y}) of {width}");
                }
            }
        }
    }

    #[test]
    fn the_pack_function_sets_the_destination_format() {
        let scale = full(640);
        let mut dst = vec![0u32; scale.pixel_count()];
        let pack = |[r, g, b, _]: Rgba| u32::from_be_bytes([0, r, g, b]);
        upscale_into(&gradient(640), &scale, pack, &mut dst).unwrap();
        // Native (1, 1) is red 1, green 0, blue 1 (its row).
        assert_eq!(dst[641], 0x0001_0001);
    }

    #[test]
    fn a_destination_of_the_wrong_size_is_refused() {
        let scale = full(1280);
        let mut dst = vec![[0u8; 4]; scale.pixel_count() - 1];
        assert_eq!(
            upscale_into(&gradient(640), &scale, |c| c, &mut dst),
            Err(SizeMismatch {
                expected: scale.pixel_count(),
                actual: scale.pixel_count() - 1
            })
        );
    }

    #[test]
    #[should_panic(expected = "not as wide as the strip's columns")]
    fn a_frame_of_another_width_panics() {
        let scale = full(1301);
        let mut dst = vec![[0u8; 4]; scale.pixel_count()];
        let _ = upscale_into(&gradient(640), &scale, |c| c, &mut dst);
    }

    #[test]
    fn exact_scales_have_no_letterbox() {
        assert_eq!(
            StripScale::exact(3),
            StripScale {
                scale: 3,
                width: 1920,
                height: 222,
                columns: 640,
                image_x: 0
            }
        );
        assert_eq!(StripScale::exact(0).scale, 1);
        let wide = StripScale::exact_viewport(3, Viewport::new(735));
        assert_eq!((wide.width, wide.columns), (2205, 735));
    }

    #[test]
    fn damage_is_the_bounding_box_of_what_changed() {
        let before = gradient(640);
        assert_eq!(Damage::between(&before, &before), None);

        let mut after = before.clone();
        after.set_pixel(7, 3, [1, 1, 1, 255]);
        after.set_pixel(20, 9, [1, 1, 1, 255]);
        assert_eq!(
            Damage::between(&before, &after),
            Some(Damage {
                x: 7,
                y: 3,
                w: 14,
                h: 7,
                whole_rows: false
            })
        );
        // Another width is a full redraw.
        assert_eq!(Damage::between(&before, &gradient(700)), Some(Damage::FULL));
    }

    #[test]
    fn a_damage_write_reports_the_physical_area_it_covered() {
        let scale = full(1900);
        let frame = gradient(scale.columns);
        let mut dst = vec![[0u8; 4]; scale.pixel_count()];
        let damage = Damage {
            x: 5,
            y: 2,
            w: 3,
            h: 4,
            whole_rows: false,
        };
        let area = upscale_damage_into(&frame, &scale, |c| c, &mut dst, &damage);
        assert_eq!(
            area,
            Ok(Some(PhysicalRect {
                x: scale.image_x + 10,
                y: 4,
                w: 6,
                h: 8
            }))
        );
        let rows = Damage {
            whole_rows: true,
            ..damage
        };
        let area = upscale_damage_into(&frame, &scale, |c| c, &mut dst, &rows);
        assert_eq!(
            area,
            Ok(Some(PhysicalRect {
                x: 0,
                y: 4,
                w: 1900,
                h: 8
            }))
        );
        // Past the strip's right edge, a damage is cut where the strip ends.
        let cut = full(1301);
        let mut dst = vec![[0u8; 4]; cut.pixel_count()];
        let last = Damage { x: 650, ..damage };
        let area = upscale_damage_into(&gradient(cut.columns), &cut, |c| c, &mut dst, &last);
        assert_eq!(area.unwrap().map(|r| (r.x, r.w)), Some((1300, 1)));
        // Nothing to write, or only past the frame: no area.
        let empty = Damage { w: 0, ..damage };
        let past = Damage { y: 80, ..damage };
        for nothing in [empty, past] {
            let area = upscale_damage_into(
                &frame,
                &scale,
                |c| c,
                &mut vec![[0u8; 4]; scale.pixel_count()],
                &nothing,
            );
            assert_eq!(area, Ok(None));
        }
    }

    fn redraw(frame: &NativeImage, scale: &StripScale) -> Vec<Rgba> {
        let mut dst = vec![[0u8; 4]; scale.pixel_count()];
        upscale_into(frame, scale, |c| c, &mut dst).unwrap();
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
            width in prop::sample::select(vec![640_u32, 1281, 1301, 1925, 2940]),
            edits in prop::collection::vec((0_u32..840, 0_u32..74, 1_u32..80, 1_u32..20, any::<u8>()), 0..4),
        ) {
            let scale = full(width);
            let before = gradient(scale.columns);
            let mut after = before.clone();
            for (x, y, w, h, value) in edits {
                for yy in y..(y + h).min(NATIVE_HEIGHT) {
                    for xx in x..(x + w).min(scale.columns) {
                        after.set_pixel(xx, yy, [value, value, value, 255]);
                    }
                }
            }
            let mut dst = redraw(&before, &scale);
            if let Some(damage) = Damage::between(&before, &after) {
                upscale_damage_into(&after, &scale, |c| c, &mut dst, &damage).unwrap();
            }
            prop_assert!(dst == redraw(&after, &scale));
        }
    }
}
