//! The CPU compositor: paints a `Frame` into the 640×74 native image.
//!
//! Each frame first looks every palette slot up at its `palette_mix`, then paints its draws in
//! order over a sky of flat `sky.base`, each meeting the pixels below by the rule in
//! [`crate::blend`], and finally darkens the result by its `dim`. Nothing is linearized and
//! nothing is quantized back into the palette.

use crate::blend::{blend, dim};
use crate::catalog::{Art, Blend, Layer, PALETTE_SLOTS, Rgb};
use crate::frame::{Area, Draw, Frame, NATIVE_HEIGHT, NATIVE_WIDTH, Rgba, Rgba640x74};
use crate::palette::{Lut, Slot};

const OPAQUE: u8 = u8::MAX;
const FRAME_WIDTH: i32 = NATIVE_WIDTH.cast_signed();
const FRAME_HEIGHT: i32 = NATIVE_HEIGHT.cast_signed();

/// Paints `frame` over a sky of flat `sky.base`.
pub fn composite(art: &Art, frame: &Frame) -> Rgba640x74 {
    let mut out = Rgba640x74::filled([0, 0, 0, OPAQUE]);
    composite_into(art, frame, &mut out);
    out
}

/// Paints `frame` into `out`, replacing every pixel. The sky starts as flat `sky.base`, which
/// shows wherever no sky tile is drawn.
pub fn composite_into(art: &Art, frame: &Frame, out: &mut Rgba640x74) {
    composite_area(art, frame, out, FRAME);
}

/// As [`composite_into`], but paints only the pixels inside `area` and leaves the rest of
/// `out` as it was. Each of those pixels is exactly what a whole composite would give it.
pub fn composite_area(art: &Art, frame: &Frame, out: &mut Rgba640x74, area: Area) {
    let region = Visible::FRAME.within(Some(area));
    let (Ok(top), Ok(bottom), Ok(left), Ok(right)) = (
        u32::try_from(region.top),
        u32::try_from(region.bottom),
        usize::try_from(region.left),
        usize::try_from(region.right),
    ) else {
        return;
    };
    if left >= right {
        return;
    }
    let lut = Lut::new(&art.catalog.palette, frame.palette_mix);
    let [r, g, b] = lut.colour(Slot::SKY_BASE);
    for y in top..bottom {
        out.row_mut(y)[left..right].fill([r, g, b, OPAQUE]);
    }
    let mut layer = Layer::Sky;
    for draw in &frame.draws {
        debug_assert!(
            draw.layer >= layer,
            "a {:?} draw follows a {layer:?} one",
            draw.layer
        );
        layer = draw.layer;
        paint(art, &lut, draw, &region, out);
    }
    if frame.dim > 0 {
        for y in top..bottom {
            for pixel in &mut out.row_mut(y)[left..right] {
                *pixel = dim(*pixel, frame.dim);
            }
        }
    }
}

fn paint(art: &Art, lut: &Lut, draw: &Draw, region: &Visible, out: &mut Rgba640x74) {
    let opacity = draw.opacity;
    if opacity == 0 {
        return;
    }
    let colours = source_colours(lut, draw.tint);
    let bounds = region.within(draw.clip);
    // Each arm fixes the blend mode, so the pixel loop is compiled once per mode.
    match (draw.blend, opacity) {
        (Blend::Normal, OPAQUE) => each_pixel(art, draw, &bounds, out, |pixel, index| {
            let [r, g, b] = colours[index];
            *pixel = [r, g, b, OPAQUE];
        }),
        (Blend::Normal, _) => each_pixel(art, draw, &bounds, out, |pixel, index| {
            over(pixel, colours[index], opacity, Blend::Normal);
        }),
        (Blend::Multiply, _) => each_pixel(art, draw, &bounds, out, |pixel, index| {
            over(pixel, colours[index], opacity, Blend::Multiply);
        }),
        (Blend::Add, _) => each_pixel(art, draw, &bounds, out, |pixel, index| {
            over(pixel, colours[index], opacity, Blend::Add);
        }),
    }
}

#[inline]
fn over(pixel: &mut Rgba, src: Rgb, opacity: u8, mode: Blend) {
    for (dst, src) in pixel.iter_mut().zip(src) {
        *dst = blend(*dst, src, opacity, mode);
    }
}

/// Each atlas byte's colour for this draw: its slot's, or the tint's. Byte 0 is transparent
/// and never painted.
fn source_colours(lut: &Lut, tint: Option<Slot>) -> [Rgb; PALETTE_SLOTS + 1] {
    std::array::from_fn(|index| {
        let slot = u8::try_from(index).ok().and_then(Slot::from_atlas);
        slot.map_or([0; 3], |slot| lut.colour(tint.unwrap_or(slot)))
    })
}

/// Calls `ink` with every opaque pixel of the draw that lands inside `visible`, after flip and
/// shear, and the pixel's atlas byte.
fn each_pixel(
    art: &Art,
    draw: &Draw,
    visible: &Visible,
    out: &mut Rgba640x74,
    mut ink: impl FnMut(&mut Rgba, usize),
) {
    let width = i32::from(art.catalog.sprite(draw.sprite).rect.w);
    for (row, pixels) in (0_i32..).zip(art.rows(draw.sprite)) {
        let y = draw.y + row;
        if y < visible.top {
            continue;
        }
        if y >= visible.bottom {
            break;
        }
        let x = draw.x + draw.shear.offset(row);
        // The sprite's columns whose pixels land in `left..right`; flipped, column c lands at
        // x + width − 1 − c.
        let (from, to) = if draw.flip_x {
            (x + width - visible.right, x + width - visible.left)
        } else {
            (visible.left - x, visible.right - x)
        };
        let (from, to) = (from.clamp(0, width), to.clamp(0, width));
        let start = if draw.flip_x {
            x + width - to
        } else {
            x + from
        };
        let (Ok(y), Ok(from), Ok(to), Ok(start)) = (
            u32::try_from(y),
            usize::try_from(from),
            usize::try_from(to),
            usize::try_from(start),
        ) else {
            continue;
        };
        if from >= to {
            continue;
        }
        let dst = &mut out.row_mut(y)[start..start + (to - from)];
        let src = &pixels[from..to];
        let mut put = |(pixel, &index): (&mut Rgba, &u8)| {
            if index != 0 {
                ink(pixel, usize::from(index));
            }
        };
        if draw.flip_x {
            dst.iter_mut().zip(src.iter().rev()).for_each(&mut put);
        } else {
            dst.iter_mut().zip(src).for_each(&mut put);
        }
    }
}

/// The whole native frame, as an area.
const FRAME: Area = Area {
    x: 0,
    y: 0,
    w: FRAME_WIDTH,
    h: FRAME_HEIGHT,
};

/// The part of the frame a draw may paint, as half-open bounds.
struct Visible {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl Visible {
    const FRAME: Visible = Visible {
        left: 0,
        top: 0,
        right: FRAME_WIDTH,
        bottom: FRAME_HEIGHT,
    };

    /// These bounds cut down to an area, if there is one.
    fn within(&self, clip: Option<Area>) -> Visible {
        match clip {
            None => Visible { ..*self },
            Some(area) => Visible {
                left: area.x.max(self.left),
                top: area.y.max(self.top),
                right: area.x.saturating_add(area.w).min(self.right),
                bottom: area.y.saturating_add(area.h).min(self.bottom),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::SpriteId;
    use crate::frame::Shear;

    fn art() -> &'static Art<'static> {
        Art::embedded()
    }

    fn id(name: &str) -> SpriteId {
        art().catalog.id(name).expect(name)
    }

    fn frame(draws: Vec<Draw>) -> Frame {
        Frame {
            draws,
            ..Frame::default()
        }
    }

    fn opaque([r, g, b]: Rgb) -> Rgba {
        [r, g, b, OPAQUE]
    }

    fn sky(mix_by: u8) -> Rgba {
        opaque(Lut::new(&art().catalog.palette, mix_by).colour(Slot::SKY_BASE))
    }

    #[test]
    fn the_glass_edge_over_the_sky_is_the_reference_colour() {
        // Slot 4 at opacity 150 over sky.base is #A5A38D by day, #48565C by night.
        let edge = id("tmp.glass.edge.left");
        let stone_shadow = Slot::STONE_SHADOW.get() + 1;
        let (row, column) = art()
            .rows(edge)
            .enumerate()
            .find_map(|(y, row)| row.iter().position(|&i| i == stone_shadow).map(|x| (y, x)))
            .expect("the edge has a slot-4 pixel");
        let (x, y) = (u32::try_from(column).unwrap(), u32::try_from(row).unwrap());
        let draw = Draw::new(&art().catalog, edge, 0, 0);
        assert_eq!(draw.opacity, 150, "the catalog's own opacity");
        let mut day = frame(vec![draw]);
        assert_eq!(composite(art(), &day).pixel(x, y), [0xA5, 0xA3, 0x8D, 255]);
        day.palette_mix = 255;
        assert_eq!(composite(art(), &day).pixel(x, y), [0x48, 0x56, 0x5C, 255]);
    }

    #[test]
    fn an_empty_frame_is_flat_sky_and_opaque() {
        let out = composite(art(), &Frame::default());
        assert!(out.pixels().iter().all(|&p| p == sky(0)));
        let night = Frame {
            palette_mix: 255,
            ..Frame::default()
        };
        assert!(
            composite(art(), &night)
                .pixels()
                .iter()
                .all(|&p| p == sky(255))
        );
    }

    #[test]
    fn transparent_pixels_and_a_zero_opacity_leave_the_sky() {
        let catalog = &art().catalog;
        let right = Draw::new(catalog, id("tmp.glass.edge.right"), 100, 0);
        let out = composite(art(), &frame(vec![right]));
        // Column 0 of the right edge is transparent; column 5 is its stone line.
        assert_eq!(out.pixel(100, 10), sky(0));
        assert_ne!(out.pixel(105, 10), sky(0));
        let hidden = Draw {
            opacity: 0,
            ..right
        };
        assert_eq!(
            composite(art(), &frame(vec![hidden])),
            composite(art(), &Frame::default())
        );
    }

    #[test]
    fn a_flipped_left_edge_is_the_right_edge() {
        // The delivered right edge is the left edge mirrored, column for column.
        let catalog = &art().catalog;
        let flipped = Draw {
            flip_x: true,
            ..Draw::new(catalog, id("tmp.glass.edge.left"), 300, 0)
        };
        let right = Draw::new(catalog, id("tmp.glass.edge.right"), 300, 0);
        assert_eq!(
            composite(art(), &frame(vec![flipped])),
            composite(art(), &frame(vec![right]))
        );
    }

    #[test]
    fn a_shear_slides_each_row_about_the_pivot() {
        let catalog = &art().catalog;
        let edge = Draw::new(catalog, id("tmp.glass.edge.left"), 300, 0);
        let line = composite(art(), &frame(vec![edge])).pixel(300, 0);
        let sheared = Draw {
            // One pixel per row: row 20 stays, row 10 slides right by 10, row 30 left by 10.
            shear: Shear {
                pivot_y: 20,
                slope: 256,
            },
            ..edge
        };
        let out = composite(art(), &frame(vec![sheared]));
        for (row, x) in [(20, 300), (10, 310), (30, 290), (0, 320)] {
            assert_eq!(out.pixel(x, row), line, "row {row}");
            assert_eq!(out.pixel(x - 1, row), sky(0), "row {row}");
        }
    }

    #[test]
    fn a_clip_bounds_what_is_painted_after_flip_and_shear() {
        let catalog = &art().catalog;
        let draw = Draw {
            flip_x: true,
            shear: Shear {
                pivot_y: 0,
                slope: -256,
            },
            ..Draw::new(catalog, id("tmp.glass.edge.left"), 400, 0)
        };
        let whole = composite(art(), &frame(vec![draw]));
        // Row r lands at 400 + r, flipped: rows 3..7 reach columns 403..411.
        let area = Area {
            x: 404,
            y: 3,
            w: 5,
            h: 4,
        };
        let clipped = composite(
            art(),
            &frame(vec![Draw {
                clip: Some(area),
                ..draw
            }]),
        );
        for y in 0..NATIVE_HEIGHT {
            for x in 380..420 {
                let inside = (404..409).contains(&x) && (3..7).contains(&y);
                let want = if inside { whole.pixel(x, y) } else { sky(0) };
                assert_eq!(clipped.pixel(x, y), want, "({x}, {y})");
            }
        }
        assert_ne!(whole.pixel(404, 3), sky(0), "the clip holds painted pixels");
    }

    #[test]
    fn draws_hanging_off_any_side_are_cut_at_the_frame() {
        let catalog = &art().catalog;
        let edge = id("tmp.glass.edge.left");
        let draws = vec![
            Draw::new(catalog, edge, -3, -10),
            Draw::new(catalog, edge, 637, 40),
            Draw::new(catalog, edge, -50, 0),
            Draw::new(catalog, edge, 700, 0),
        ];
        let out = composite(art(), &frame(draws));
        // The left edge's columns 3.. land at 0.. and its stone line at column 0 is off-frame.
        assert_eq!(
            out.pixel(0, 0),
            composite(art(), &Frame::default()).pixel(0, 0)
        );
        assert_ne!(out.pixel(637, 50), sky(0));
    }

    #[test]
    fn a_tint_paints_every_opaque_pixel_in_one_slot() {
        let catalog = &art().catalog;
        let draw = Draw {
            tint: Some(Slot::SOIL_DEEP),
            opacity: 255,
            ..Draw::new(catalog, id("tmp.glass.edge.left"), 10, 0)
        };
        let out = composite(art(), &frame(vec![draw]));
        let ink = opaque(catalog.palette.day[0]);
        for (x, column) in (10..16).zip(art().rows(draw.sprite).next().unwrap()) {
            let want = if *column == 0 { sky(0) } else { ink };
            assert_eq!(out.pixel(x, 5), want);
        }
    }

    #[test]
    fn dim_darkens_the_finished_frame_toward_black() {
        let dimmed = |by| {
            composite(
                art(),
                &Frame {
                    dim: by,
                    ..Frame::default()
                },
            )
            .pixel(0, 0)
        };
        assert_eq!(dimmed(0), sky(0));
        assert_eq!(dimmed(255), [0, 0, 0, 255]);
        // sky.base #E5DEC1 at half: 229 × 127 / 255 = 114.05, and so on.
        assert_eq!(dimmed(128), [114, 111, 96, 255]);
        assert_eq!(dim(sky(0), 128), dimmed(128));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "follows a Glass one")]
    fn layers_never_go_backwards() {
        let catalog = &art().catalog;
        let glass = Draw::new(catalog, id("tmp.glass.edge.left"), 0, 0);
        let soil = Draw::new(catalog, id("tmp.soil.tile.01"), 0, 66);
        let _ = composite(art(), &frame(vec![glass, soil]));
    }

    #[test]
    fn an_area_paints_exactly_what_a_whole_composite_does_there() {
        let catalog = &art().catalog;
        let draws = vec![
            Draw::new(catalog, id("tmp.soil.tile.01"), 0, 66),
            Draw::new(catalog, id("tmp.glass.edge.left"), 0, 0),
            Draw::new(catalog, id("tmp.glass.edge.right"), 634, 0),
        ];
        let frame = Frame {
            draws,
            palette_mix: 90,
            dim: 40,
        };
        let whole = composite(art(), &frame);
        let area = Area {
            x: 1,
            y: 60,
            w: 4,
            h: 30,
        };
        // Bright, so a dim pass that spilled outside the area would show on it.
        let marker = [200, 120, 60, 255];
        let mut out = Rgba640x74::filled(marker);
        composite_area(art(), &frame, &mut out, area);
        for y in 0..NATIVE_HEIGHT {
            for x in 0..NATIVE_WIDTH {
                let inside = (1..5).contains(&x) && (60..74).contains(&y);
                let want = if inside { whole.pixel(x, y) } else { marker };
                assert_eq!(out.pixel(x, y), want, "({x}, {y})");
            }
        }
    }
}
