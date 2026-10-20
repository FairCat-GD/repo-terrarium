//! The native frame, and the `Frame` of draws the compositor paints into it. Every pixel is
//! composed natively and only ever scaled by an integer (I-11). The scene is 640×74; a screen
//! whose width is no multiple of 640 shows a wider [`Viewport`] of it.

use std::fmt;

use crate::catalog::{Blend, Catalog, Layer, SpriteId};
use crate::palette::Slot;

/// The scene's width: what every screen shows, and all a screen 640 times an integer wide shows.
pub const NATIVE_WIDTH: u32 = 640;
pub const NATIVE_HEIGHT: u32 = 74;
/// The widest viewport the art covers: the weather backdrops' width.
pub const WIDEST: u32 = 840;

/// The native columns a strip shows: the scene's 640, or more, up to [`WIDEST`], on a screen
/// whose width is no multiple of 640, so nothing is ever stretched across its spare width. The
/// ground spreads across it: every place on the ground stands in proportion to the width
/// ([`Viewport::spread`]), while its art keeps its pixels. The sun, the moon and the stars
/// stand in the middle. The backdrop, the soil, the glass, the label and the sky's drifting
/// pieces reach the viewport's edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Viewport {
    width: u32,
}

impl Viewport {
    pub const SCENE: Viewport = Viewport {
        width: NATIVE_WIDTH,
    };

    /// `columns`, kept between the scene's width and the widest the art covers.
    pub fn new(columns: u32) -> Viewport {
        Viewport {
            width: columns.clamp(NATIVE_WIDTH, WIDEST),
        }
    }

    pub fn width(self) -> u32 {
        self.width
    }

    /// Where the scene's left edge stands: half the extra columns, an odd one on the left, as
    /// the backdrop's crop has it.
    pub fn inset(self) -> i32 {
        i32::try_from((self.width - NATIVE_WIDTH).div_ceil(2)).unwrap_or(0)
    }

    /// The columns beyond the scene, both sides together.
    pub fn extra(self) -> i32 {
        i32::try_from(self.width - NATIVE_WIDTH).unwrap_or(0)
    }

    /// Where the scene's column `x` on the ground stands in the viewport: in proportion to its
    /// width, rounded to the nearest column. It is `x` itself at 640, and it only ever grows
    /// with `x`, so places keep their order and their distance from the glass at either edge
    /// grows with the width.
    pub fn spread(self, x: i32) -> i32 {
        let scene = NATIVE_WIDTH.cast_signed();
        x + (2 * x * self.extra() + scene).div_euclid(2 * scene)
    }

    /// Which third of the viewport column `x` falls in: 0, 1 or 2.
    pub fn third(self, x: i32) -> i32 {
        x * 3 / self.width.cast_signed()
    }
}

impl Default for Viewport {
    fn default() -> Viewport {
        Viewport::SCENE
    }
}
/// The ground line: the soil's top edge, the first of its two rows. The soil band is this row
/// and the eight below it.
pub const GROUND_Y: u32 = 65;
/// The soil's body: the edge's second row and the seven below it.
pub const SOIL_Y: u32 = 66;
/// Where plants and filler stand: the row above the ground line. Their anchor row is their
/// last, so they sit on the soil's opaque top edge rather than under it.
pub const PLANT_GROUND_Y: i32 = GROUND_Y.cast_signed() - 1;

/// One pixel: red, green, blue, alpha.
pub type Rgba = [u8; 4];

/// A native frame in RGBA8, row-major, top row first, as wide as its viewport.
#[derive(Clone, PartialEq, Eq)]
pub struct NativeImage {
    width: u32,
    pixels: Box<[Rgba]>,
}

impl NativeImage {
    /// The scene's width, one colour.
    pub fn filled(colour: Rgba) -> Self {
        Self::new(Viewport::SCENE, colour)
    }

    /// `viewport`'s width, one colour.
    pub fn new(viewport: Viewport, colour: Rgba) -> Self {
        let width = viewport.width();
        Self {
            width,
            pixels: vec![colour; (width * NATIVE_HEIGHT) as usize].into_boxed_slice(),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    /// # Panics
    /// If `(x, y)` lies outside the frame.
    pub fn pixel(&self, x: u32, y: u32) -> Rgba {
        self.pixels[self.index(x, y)]
    }

    /// # Panics
    /// If `(x, y)` lies outside the frame.
    pub fn set_pixel(&mut self, x: u32, y: u32, colour: Rgba) {
        let at = self.index(x, y);
        self.pixels[at] = colour;
    }

    /// # Panics
    /// If `y` lies outside the frame.
    pub fn row(&self, y: u32) -> &[Rgba] {
        let start = self.index(0, y);
        &self.pixels[start..start + self.width as usize]
    }

    /// # Panics
    /// If `y` lies outside the frame.
    pub fn row_mut(&mut self, y: u32) -> &mut [Rgba] {
        let start = self.index(0, y);
        let width = self.width as usize;
        &mut self.pixels[start..start + width]
    }

    /// Every pixel, row-major.
    pub fn pixels(&self) -> &[Rgba] {
        &self.pixels
    }
}

impl NativeImage {
    fn index(&self, x: u32, y: u32) -> usize {
        assert!(
            x < self.width && y < NATIVE_HEIGHT,
            "({x}, {y}) is outside the native frame"
        );
        (y * self.width + x) as usize
    }
}

impl fmt::Debug for NativeImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NativeImage {{ {}×{NATIVE_HEIGHT} }}", self.width)
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
    /// The columns it is composed for; draws stand in its coordinates.
    pub viewport: Viewport,
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
        let mut frame = NativeImage::filled([0, 0, 0, 255]);
        frame.set_pixel(639, 73, [1, 2, 3, 255]);
        frame.set_pixel(5, 1, [9, 9, 9, 255]);
        assert_eq!(frame.pixel(639, 73), [1, 2, 3, 255]);
        assert_eq!(frame.row(1)[5], [9, 9, 9, 255]);
        assert_eq!(frame.row(73).len(), 640);
        frame.row_mut(2)[7] = [4, 4, 4, 255];
        assert_eq!(frame.pixels()[2 * 640 + 7], [4, 4, 4, 255]);
    }

    #[test]
    fn the_ground_spreads_in_proportion_and_stands_still_at_640() {
        for x in -10..650 {
            assert_eq!(Viewport::SCENE.spread(x), x);
        }
        for columns in [641, 651, 735, 756, 840] {
            let viewport = Viewport::new(columns);
            let width = viewport.width().cast_signed();
            assert_eq!((viewport.spread(0), viewport.spread(640)), (0, width));
            // A step of the scene is a step or more, never less, and never more than the
            // width's own ratio rounded up: places keep their order and grow apart.
            for x in 0..640 {
                let step = viewport.spread(x + 1) - viewport.spread(x);
                assert!((1..=2).contains(&step), "{x} at {columns}: {step}");
            }
            // The scene's thirds are the viewport's, but for a column's rounding at a border.
            for x in (1..639).filter(|x| (x - 1) * 3 / 640 == (x + 1) * 3 / 640) {
                assert_eq!(
                    viewport.third(viewport.spread(x)),
                    x * 3 / 640,
                    "{x} at {columns}"
                );
            }
        }
    }

    #[test]
    #[should_panic(expected = "outside the native frame")]
    fn a_pixel_outside_the_frame_panics() {
        let _ = NativeImage::filled([0; 4]).pixel(640, 0);
    }

    #[test]
    fn a_draw_takes_the_sprites_own_layer_blend_and_opacity() {
        let catalog = &Art::embedded().catalog;
        let edge = catalog.id("tmp.glass.edge.left").unwrap();
        let draw = Draw::new(catalog, edge, 0, 0);
        assert_eq!(
            (draw.layer, draw.blend, draw.opacity),
            (Layer::Glass, Blend::Normal, 65)
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
