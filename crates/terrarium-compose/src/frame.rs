//! The native frame, and the `Frame` of draws the compositor paints into it. Every pixel is
//! composed at 640×74 and only ever scaled by an integer (I-11).

use std::fmt;

use crate::catalog::{Blend, Catalog, Layer, SpriteId};
use crate::palette::Slot;

pub const NATIVE_WIDTH: u32 = 640;
pub const NATIVE_HEIGHT: u32 = 74;
/// The ground line. The soil band is this row and the seven below it.
pub const GROUND_Y: u32 = 66;
/// Where plants and filler stand: the row above the ground line. Their anchor row is their
/// last, so they sit on the soil's opaque top edge rather than under it.
pub const PLANT_GROUND_Y: i32 = GROUND_Y.cast_signed() - 1;

/// One pixel: red, green, blue, alpha.
pub type Rgba = [u8; 4];

/// A native frame in RGBA8, row-major, top row first.
#[derive(Clone, PartialEq, Eq)]
pub struct Rgba640x74 {
    pixels: Box<[Rgba]>,
}

impl Rgba640x74 {
    pub fn filled(colour: Rgba) -> Self {
        Self {
            pixels: vec![colour; (NATIVE_WIDTH * NATIVE_HEIGHT) as usize].into_boxed_slice(),
        }
    }

    /// # Panics
    /// If `(x, y)` lies outside the frame.
    pub fn pixel(&self, x: u32, y: u32) -> Rgba {
        self.pixels[index(x, y)]
    }

    /// # Panics
    /// If `(x, y)` lies outside the frame.
    pub fn set_pixel(&mut self, x: u32, y: u32, colour: Rgba) {
        self.pixels[index(x, y)] = colour;
    }

    /// # Panics
    /// If `y` lies outside the frame.
    pub fn row(&self, y: u32) -> &[Rgba] {
        let start = index(0, y);
        &self.pixels[start..start + NATIVE_WIDTH as usize]
    }

    /// # Panics
    /// If `y` lies outside the frame.
    pub fn row_mut(&mut self, y: u32) -> &mut [Rgba] {
        let start = index(0, y);
        &mut self.pixels[start..start + NATIVE_WIDTH as usize]
    }

    /// Every pixel, row-major.
    pub fn pixels(&self) -> &[Rgba] {
        &self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [Rgba] {
        &mut self.pixels
    }
}

fn index(x: u32, y: u32) -> usize {
    assert!(
        x < NATIVE_WIDTH && y < NATIVE_HEIGHT,
        "({x}, {y}) is outside the native frame"
    );
    (y * NATIVE_WIDTH + x) as usize
}

impl fmt::Debug for Rgba640x74 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Rgba640x74 {{ .. }}")
    }
}

/// What the compositor paints: sprite draws in order, and the palette and brightness they are
/// painted with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// Painted in order. Layers never go backwards; within a layer, the order is the composer's
    /// stable key.
    pub draws: Vec<Draw>,
    /// 0 is day and 255 is night; between them, each slot's two colours are blended.
    pub palette_mix: u8,
    /// How much the finished frame is darkened toward black: 0 leaves it as it is.
    pub dim: u8,
}

/// One sprite, placed. Every field is applied by the compositor exactly as given; the
/// composer decides them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Draw {
    pub sprite: SpriteId,
    /// The depth it is painted at. A sprite's own layer, except for the UI sprites and glyphs,
    /// which have none, and scorch, which takes its host plant's.
    pub layer: Layer,
    /// The top-left of the sprite's pixels, in native pixels, with the anchor already applied.
    pub x: i32,
    pub y: i32,
    /// Mirrors the pixels within the sprite's own width.
    pub flip_x: bool,
    pub opacity: u8,
    pub blend: Blend,
    pub shear: Shear,
    /// Paints every opaque pixel in this one slot: glyph masks are tinted at draw time.
    pub tint: Option<Slot>,
    /// Pixels outside this area of the frame are not painted. It applies after flip and shear,
    /// and is how the nine-slice crops its final repeat.
    pub clip: Option<Area>,
}

impl Draw {
    /// The sprite in its own layer, with its own blend and opacity, its top-left at `(x, y)`.
    ///
    /// # Panics
    /// If the sprite has no layer: the UI sprites and glyphs, which take [`Draw::in_layer`].
    pub fn new(catalog: &Catalog, sprite: SpriteId, x: i32, y: i32) -> Draw {
        let art = catalog.sprite(sprite);
        let layer = art
            .layer
            .unwrap_or_else(|| panic!("{} has no layer of its own", art.id));
        Draw::in_layer(catalog, sprite, layer, x, y)
    }

    /// The sprite in a layer the composer chooses, with its own blend and opacity.
    pub fn in_layer(catalog: &Catalog, sprite: SpriteId, layer: Layer, x: i32, y: i32) -> Draw {
        let art = catalog.sprite(sprite);
        Draw {
            sprite,
            layer,
            x,
            y,
            flip_x: false,
            opacity: art.opacity,
            blend: art.blend,
            shear: Shear::NONE,
            tint: None,
            clip: None,
        }
    }

    /// The sprite in its own layer, its anchor on the world point `(x, y)`.
    ///
    /// # Panics
    /// If the sprite has no layer.
    pub fn anchored(catalog: &Catalog, sprite: SpriteId, x: i32, y: i32) -> Draw {
        let anchor = catalog.sprite(sprite).anchor;
        Draw::new(
            catalog,
            sprite,
            x - i32::from(anchor.x),
            y - i32::from(anchor.y),
        )
    }

    /// Faces the other way about the anchor: the pixels mirror, and the mirrored anchor,
    /// `width − 1 − anchor.x`, lands where the anchor was. For a 32-pixel resident anchored at
    /// x = 16 that is the left-facing anchor x = 15, so it is never applied twice.
    #[must_use]
    pub fn mirrored(self, catalog: &Catalog) -> Draw {
        let art = catalog.sprite(self.sprite);
        let width = i32::from(art.rect.w);
        let anchor_x = i32::from(art.anchor.x);
        let shift = 2 * anchor_x - (width - 1);
        let x = if self.flip_x {
            self.x - shift
        } else {
            self.x + shift
        };
        Draw {
            x,
            flip_x: !self.flip_x,
            ..self
        }
    }
}

/// An integer shear about a row: each row slides sideways by an amount that grows with its
/// distance from the pivot row, which never moves. Foliage sways this way about its
/// `shear_pivot`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shear {
    /// The pivot row, in the sprite's own pixel space.
    pub pivot_y: i16,
    /// The slide of the row one above the pivot, in 1/256 of a pixel. Positive slides rows
    /// above the pivot to the right and rows below it to the left.
    pub slope: i16,
}

impl Shear {
    pub const NONE: Shear = Shear {
        pivot_y: 0,
        slope: 0,
    };

    /// How far a row of the sprite slides: `(pivot_y − row) × slope / 256`, rounded half away
    /// from zero, so a mirrored slope mirrors every row.
    pub fn offset(self, row: i32) -> i32 {
        let scaled = (i32::from(self.pivot_y) - row) * i32::from(self.slope);
        let magnitude = (scaled.abs() + 128) / 256;
        if scaled < 0 { -magnitude } else { magnitude }
    }
}

/// A rectangle of the native frame. It may reach outside the frame; only its part inside
/// counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Art;

    #[test]
    fn pixels_are_row_major() {
        let mut frame = Rgba640x74::filled([0, 0, 0, 255]);
        frame.set_pixel(639, 73, [1, 2, 3, 255]);
        frame.set_pixel(5, 1, [9, 9, 9, 255]);
        assert_eq!(frame.pixel(639, 73), [1, 2, 3, 255]);
        assert_eq!(frame.row(1)[5], [9, 9, 9, 255]);
        assert_eq!(frame.row(73).len(), 640);
        frame.row_mut(2)[7] = [4, 4, 4, 255];
        assert_eq!(frame.pixels()[2 * 640 + 7], [4, 4, 4, 255]);
    }

    #[test]
    #[should_panic(expected = "outside the native frame")]
    fn a_pixel_outside_the_frame_panics() {
        let _ = Rgba640x74::filled([0; 4]).pixel(640, 0);
    }

    #[test]
    fn a_draw_takes_the_sprites_own_layer_blend_and_opacity() {
        let catalog = &Art::embedded().catalog;
        let edge = catalog.id("tmp.glass.edge.left").unwrap();
        let draw = Draw::new(catalog, edge, 0, 0);
        assert_eq!(
            (draw.layer, draw.blend, draw.opacity),
            (Layer::Glass, Blend::Normal, 150)
        );
        let dusk = catalog.id("tmp.sky.light.dusk").unwrap();
        let draw = Draw::new(catalog, dusk, 0, 0);
        assert_eq!((draw.blend, draw.opacity), (Blend::Multiply, 34));
    }

    #[test]
    #[should_panic(expected = "has no layer of its own")]
    fn a_ui_sprite_needs_a_layer_chosen_for_it() {
        let catalog = &Art::embedded().catalog;
        let tile = catalog.id("tmp.ui.panel.c").unwrap();
        let _ = Draw::new(catalog, tile, 0, 0);
    }

    #[test]
    fn an_anchored_draw_puts_the_anchor_on_the_world_point() {
        let catalog = &Art::embedded().catalog;
        let pebble = catalog.id("tmp.ground.pebble.03").unwrap();
        let draw = Draw::anchored(catalog, pebble, 100, 70);
        assert_eq!((draw.x, draw.y), (99, 69));
    }

    #[test]
    fn mirroring_keeps_the_anchor_and_uses_the_mirrored_anchor_x() {
        let catalog = &Art::embedded().catalog;
        // Every resident frame is 32 wide and anchored at x = 16.
        let (walk, sprite) = catalog
            .sprites
            .iter()
            .enumerate()
            .find(|(_, s)| s.rect.w == 32 && s.anchor.x == 16)
            .expect("a resident frame");
        assert_eq!(sprite.rect.w, 32);
        let id = SpriteId::from_index(walk).unwrap();
        let right = Draw::anchored(catalog, id, 200, 60);
        assert_eq!(right.x, 200 - 16);
        let left = right.mirrored(catalog);
        assert!(left.flip_x);
        // Facing left, the anchor is x = 15 of the mirrored image.
        assert_eq!(left.x, 200 - 15);
        assert_eq!(left.mirrored(catalog), right);
    }

    #[test]
    fn the_pivot_row_never_moves_and_rows_slide_with_their_distance() {
        let shear = Shear {
            pivot_y: 10,
            slope: 64,
        };
        assert_eq!(shear.offset(10), 0);
        // A quarter pixel per row: rows 2 and 6 above the pivot.
        assert_eq!(shear.offset(8), 1);
        assert_eq!(shear.offset(4), 2);
        // Below the pivot the slide is the other way.
        assert_eq!(shear.offset(14), -1);
        assert_eq!(Shear::NONE.offset(-40), 0);
    }

    #[test]
    fn a_mirrored_slope_mirrors_every_row_and_halves_round_away_from_zero() {
        for slope in [-300, -128, -1, 0, 1, 127, 128, 129, 255, 511] {
            let right = Shear { pivot_y: 3, slope };
            let left = Shear {
                pivot_y: 3,
                slope: -slope,
            };
            for row in -20..40 {
                assert_eq!(left.offset(row), -right.offset(row), "{slope} at {row}");
            }
        }
        // Exactly half a pixel rounds away from zero, on both sides of the pivot.
        let half = Shear {
            pivot_y: 0,
            slope: 128,
        };
        assert_eq!(half.offset(-1), 1);
        assert_eq!(half.offset(1), -1);
        let under_half = Shear {
            pivot_y: 0,
            slope: 127,
        };
        assert_eq!(under_half.offset(-1), 0);
        assert_eq!(under_half.offset(1), 0);
    }
}
