//! The world around the plants: where each of its pieces stands on one install, and the fixed
//! scenery.
//!
//! Every position here comes from the install salt and the viewport's width and nothing else,
//! so it changes only when the width does. Each kind of piece has a fixed number of places,
//! spread one per stratum across the strip, and piece `i` always stands in place `i`: a count
//! that changes never moves another piece. Places on the ground spread with the width
//! ([`Viewport::spread`]); the sun, the moon and the stars keep the scene's own columns. Which
//! piece stands where is decided elsewhere, without the salt (I-2).

use crate::catalog::{Art, Catalog, Flags, SpriteId};
use crate::drift::{self, SOIL_PERIOD};
use crate::frame::{Area, Draw, GROUND_Y, NATIVE_HEIGHT, NATIVE_WIDTH, SOIL_Y, Viewport};
use crate::order::{KeyedDraw, Subject};
use crate::salt::{InstallSalt, Purpose};
use crate::sky::{self, Drift};
use crate::slots::{CELLS, boundary};
use crate::text::Label;
use crate::variants::{ArtTable, WorldArt};

const WIDTH: i32 = NATIVE_WIDTH.cast_signed();
const HEIGHT: i32 = NATIVE_HEIGHT.cast_signed();
const GROUND: i32 = GROUND_Y.cast_signed();
const SOIL: i32 = SOIL_Y.cast_signed();

/// The label plate's top-left in the viewport: top-left in the sky, just inside the left glass
/// edge, as the art places it.
pub const LABEL_X: i32 = 10;
pub const LABEL_Y: i32 = 2;

/// How many of each piece the world has places for.
pub const MONUMENTS: usize = 3;
pub const FOSSILS: usize = 6;
/// The fossil spots' strata, and how far the salt moves a spot into its stratum.
const FOSSILS_FROM: i32 = 24;
const FOSSILS_TO: i32 = 616;
const FOSSIL_JITTER: i32 = 91;
/// A crack keeps this far inside the scene's edges.
const CRACK_MARGIN: i32 = 24;
pub const PEBBLES: usize = 10;
pub const DRESSING: usize = 18;
pub const LITTER: usize = 24;
pub const FUNGUS: usize = 18;
/// The litter places' strata.
const LITTER_FROM: i32 = 24;
const LITTER_TO: i32 = 616;
/// Steps through the litter places for the fungus. Coprime with [`LITTER`], so no two fungi
/// share a place.
const FUNGUS_STEP: usize = 5;
pub const STARS: usize = 7;
/// Six stretches of the strip, one per resident (at most six).
pub const RESIDENTS: usize = 6;
/// The stretch the residents share: inside the glass, with room for a creature's half width.
const RESIDENTS_FROM: i32 = 24;
const RESIDENTS_TO: i32 = WIDTH - 24;

/// The cell boundaries the monuments stand at, one per third: each between a mid and a near
/// cell, in the open ground between the trees.
const MONUMENT_BOUNDARIES: [usize; MONUMENTS] = [15, 44, 87];
/// Each cloud path's pace: different, so the clouds pass one another.
const CLOUD_MS_PER_PX: [u32; sky::MAX_CLOUDS] = [3000, 3700, 2600];
/// The stars' left edge: right of the label plate.
const STARS_FROM: i32 = 106;
/// The row the sun and the moon stand on.
const BODY_ROW: i32 = 3;

/// What a salted world value is for, so each kind of piece draws its own values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Monument,
    Fossil,
    Pebble,
    Dressing,
    Litter,
    Sun,
    Moon,
    Star,
    Cloud,
    Weed,
    Fungus,
    Mark,
    Resident,
    /// What the ground's events leave: where a crack opens, which fresh patch and sprinkle.
    Ground,
}

impl Kind {
    const fn code(self) -> u64 {
        match self {
            Kind::Monument => 1,
            Kind::Fossil => 2,
            Kind::Pebble => 3,
            Kind::Dressing => 4,
            Kind::Litter => 5,
            Kind::Sun => 6,
            Kind::Moon => 7,
            Kind::Star => 8,
            Kind::Cloud => 9,
            Kind::Weed => 10,
            Kind::Fungus => 11,
            Kind::Mark => 12,
            Kind::Resident => 13,
            Kind::Ground => 14,
        }
    }

    /// The salt index of item `item` of this kind, `part` telling apart two values one item
    /// needs. The kind holds bits 40 and up, so kinds never meet. Within a kind, a place's
    /// item is a small count and its part sits at bit 32; a sprite choice uses part 0 and an
    /// item below 2^40 (a carved mark's is its monument id × 4 + the mark), so it reaches the
    /// part's bits only in kinds that never take a place.
    const fn index(self, item: u64, part: u64) -> u64 {
        self.code() << 40 | part << 32 | item
    }
}

/// A salted value in `0..n` for one item's place.
fn place(salt: InstallSalt, kind: Kind, item: usize, part: u64, n: i32) -> i32 {
    let item = u64::try_from(item).unwrap_or(0);
    let n = u64::try_from(n.max(1)).unwrap_or(1);
    let value = salt.below(Purpose::WorldPlace, kind.index(item, part), n);
    i32::try_from(value).unwrap_or(0)
}

/// The salt's choice among equivalent sprites for one item.
pub fn pick<T: Copy>(salt: InstallSalt, kind: Kind, item: u64, list: &[T]) -> Option<T> {
    let n = u64::try_from(list.len()).ok()?;
    let chosen = salt.below(Purpose::WorldVariant, kind.index(item, 0), n);
    list.get(usize::try_from(chosen).ok()?).copied()
}

/// Where every world piece stands on one install, in one viewport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldLayout {
    salt: InstallSalt,
    viewport: Viewport,
    monuments: [i32; MONUMENTS],
    /// Each fossil spot's left edge; fossils sit on the bottom row.
    fossils: [i32; FOSSILS],
    /// World points, inside the soil band below the edge row.
    pebbles: [(i32, i32); PEBBLES],
    dressing: [i32; DRESSING],
    litter: [i32; LITTER],
    sun: (i32, i32),
    moon: (i32, i32),
    stars: [(i32, i32); STARS],
    clouds: [Drift; sky::MAX_CLOUDS],
    /// Where a resident lives in each stretch when no vigorous plant grows there.
    residents: [i32; RESIDENTS],
    /// The art's sizes the places keep clear by: the sun and moon's side, and a star's.
    body_side: i32,
    star_side: i32,
}

impl WorldLayout {
    /// The places of one install in `viewport`, kept clear of one another by the art's own
    /// sizes.
    pub fn new(salt: InstallSalt, art: &WorldArt, viewport: Viewport) -> WorldLayout {
        WorldLayout::laid(salt, (art.body_side, art.star_side), viewport)
    }

    /// The same install's places in `viewport`.
    #[must_use]
    pub fn in_viewport(&self, viewport: Viewport) -> WorldLayout {
        WorldLayout::laid(self.salt, (self.body_side, self.star_side), viewport)
    }

    fn laid(salt: InstallSalt, (body, star): (i32, i32), viewport: Viewport) -> WorldLayout {
        let fossils: [i32; FOSSILS] = std::array::from_fn(|i| {
            stratum(FOSSILS_FROM, FOSSILS_TO, FOSSILS, i)
                + place(salt, Kind::Fossil, i, 0, FOSSIL_JITTER)
        });
        let sun = (480 + place(salt, Kind::Sun, 0, 0, 101), BODY_ROW);
        let moon = (150 + place(salt, Kind::Moon, 0, 0, 111), BODY_ROW);
        // A star stands anywhere in the body rows, or drops to the lowest of them, below the
        // sun and the moon, where it would touch either.
        let lowest = *sky::BODY_ROWS.end() - star + 1;
        let star_rows = lowest - *sky::BODY_ROWS.start() + 1;
        let per_dressing = CELLS / DRESSING;
        let litter_jitter = (LITTER_TO - LITTER_FROM) / i32::try_from(LITTER).unwrap_or(1) - 3;
        WorldLayout {
            salt,
            viewport,
            body_side: body,
            star_side: star,
            monuments: std::array::from_fn(|i| {
                viewport.spread(
                    boundary(MONUMENT_BOUNDARIES[i]) + place(salt, Kind::Monument, i, 0, 5) - 2,
                )
            }),
            pebbles: std::array::from_fn(|i| {
                let mut x = stratum(8, 632, PEBBLES, i) + 1 + place(salt, Kind::Pebble, i, 0, 58);
                // Clear of every fossil spot, so no pebble ever lies over a fossil.
                while x < WIDTH - 8 && fossils.iter().any(|&f| x + 2 >= f - 1 && x - 1 <= f + 8) {
                    x += 12;
                }
                (viewport.spread(x), 69 + place(salt, Kind::Pebble, i, 1, 4))
            }),
            fossils: fossils.map(|x| viewport.spread(x)),
            dressing: std::array::from_fn(|i| {
                let cell = per_dressing * i
                    + 1
                    + usize::try_from(place(salt, Kind::Dressing, i, 0, 4)).unwrap_or(0);
                viewport.spread(boundary(cell) + place(salt, Kind::Dressing, i, 1, 3) - 1)
            }),
            litter: std::array::from_fn(|i| {
                viewport.spread(
                    stratum(LITTER_FROM, LITTER_TO, LITTER, i)
                        + 2
                        + place(salt, Kind::Litter, i, 0, litter_jitter),
                )
            }),
            sun,
            moon,
            stars: std::array::from_fn(|i| {
                let x = stratum(STARS_FROM, 630, STARS, i) + place(salt, Kind::Star, i, 0, 71);
                let y = *sky::BODY_ROWS.start() + place(salt, Kind::Star, i, 1, star_rows);
                let touches = |(bx, by): (i32, i32)| {
                    x + star >= bx - 1 && x <= bx + body && y + star >= by - 1 && y <= by + body
                };
                (
                    x,
                    if touches(sun) || touches(moon) {
                        lowest
                    } else {
                        y
                    },
                )
            }),
            residents: std::array::from_fn(|i| {
                let (from, to) = resident_stretch(i);
                from + 12 + place(salt, Kind::Resident, i, 0, to - from - 24)
            }),
            clouds: std::array::from_fn(|i| Drift {
                start: place(salt, Kind::Cloud, i, 0, 673),
                y: sky::CLOUD_TOP
                    + sky::CLOUD_PATH_STEP * i32::try_from(i).unwrap_or(0)
                    + place(salt, Kind::Cloud, i, 1, 2),
                ms_per_px: CLOUD_MS_PER_PX[i],
            }),
        }
    }

    /// The viewport its places are laid in.
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// The side of the sun and the moon, and of a star.
    pub fn sides(&self) -> (i32, i32) {
        (self.body_side, self.star_side)
    }

    /// A monument spot's ground point x, in viewport columns.
    pub fn monument(&self, spot: u8) -> i32 {
        self.monuments[usize::from(spot) % MONUMENTS]
    }

    /// A fossil spot's left edge, in viewport columns.
    pub fn fossil(&self, spot: u8) -> i32 {
        self.fossils[usize::from(spot) % FOSSILS]
    }

    /// Where a crack `width` wide that truth's mark `id` names opens: its left edge, in
    /// viewport columns, a salted place kept clear of the edges.
    pub fn crack(&self, id: u32, width: i32) -> i32 {
        let item = usize::try_from(id).unwrap_or(0);
        self.viewport.spread(
            CRACK_MARGIN
                + place(
                    self.salt,
                    Kind::Ground,
                    item,
                    0,
                    WIDTH - 2 * CRACK_MARGIN - width,
                ),
        )
    }

    /// A decorative tuft's ground point x, in viewport columns.
    pub fn dressing(&self, i: u8) -> i32 {
        self.dressing[usize::from(i) % DRESSING]
    }

    /// A piece of litter's ground point x, in viewport columns.
    pub fn litter(&self, i: u8) -> i32 {
        self.litter[usize::from(i) % LITTER]
    }

    /// A fungus's ground point x: just right of a litter place, whether or not litter lies
    /// there, so it never moves with the litter count.
    pub fn fungus(&self, i: u8) -> i32 {
        self.litter[(FUNGUS_STEP * usize::from(i) + 1) % LITTER] + 8
    }

    /// The sun's top-left, in the scene's columns.
    pub fn sun(&self) -> (i32, i32) {
        self.sun
    }

    /// The moon's top-left, in the scene's columns.
    pub fn moon(&self) -> (i32, i32) {
        self.moon
    }

    /// A star's top-left, in the scene's columns.
    pub fn star(&self, i: u8) -> (i32, i32) {
        self.stars[usize::from(i) % STARS]
    }

    /// A stretch's resident home when no vigorous plant grows there, in the scene's columns:
    /// a resident keeps its home in the scene and is drawn where the width spreads it.
    pub fn resident(&self, spot: u8) -> i32 {
        self.residents[usize::from(spot) % RESIDENTS]
    }

    pub fn cloud(&self, i: u8) -> Drift {
        self.clouds[usize::from(i) % sky::MAX_CLOUDS]
    }

    pub fn pebbles(&self) -> &[(i32, i32); PEBBLES] {
        &self.pebbles
    }
}

/// The middle of fossil spot `spot` in the scene, before the width spreads it and the salt
/// moves it: the same on every install and at every width, so a choice made by it is too.
pub fn fossil_centre(spot: usize) -> i32 {
    stratum(FOSSILS_FROM, FOSSILS_TO, FOSSILS, spot) + FOSSIL_JITTER / 2
}

/// Where resident stretch `i` begins and ends in the scene: the same on every install and at
/// every width.
pub fn resident_stretch(i: usize) -> (i32, i32) {
    (
        stratum(RESIDENTS_FROM, RESIDENTS_TO, RESIDENTS, i),
        stratum(RESIDENTS_FROM, RESIDENTS_TO, RESIDENTS, i + 1),
    )
}

/// The left edge of stratum `i` of `n` spanning `from..to`.
fn stratum(from: i32, to: i32, n: usize, i: usize) -> i32 {
    let (n, i) = (
        i32::try_from(n).unwrap_or(1).max(1),
        i32::try_from(i).unwrap_or(0),
    );
    from + (to - from) * i / n
}

/// The draws that never change between frames, keyed and sorted once: the pebbles, the glass
/// edges and corners, and the label plate. The soil band drifts, so the composer draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scenery {
    keyed: Vec<KeyedDraw>,
    viewport: Viewport,
}

impl Scenery {
    /// The scenery of one install, in its layout's viewport: the glass at its edges, the label
    /// at its left, and the pebbles spread across it.
    ///
    /// # Panics
    /// If the catalog lacks the label plate, which the embedded art has.
    pub fn new(art: &Art, table: &ArtTable, layout: &WorldLayout) -> Scenery {
        let viewport = layout.viewport;
        let catalog = &art.catalog;
        let world = table.world();
        let mut draws = glass(art, world.corners, viewport);
        for (i, &(x, y)) in layout.pebbles.iter().enumerate() {
            let item = u64::try_from(i).unwrap_or(0);
            if let Some(pebble) = pick(layout.salt, Kind::Pebble, item, &world.pebbles) {
                draws.push(Draw::anchored(catalog, pebble, x, y));
            }
        }
        let label = Label::new(catalog).expect("the delivered label plate");
        draws.push(label.plate(LABEL_X, LABEL_Y));
        let mut keyed: Vec<KeyedDraw> = (0_u16..)
            .zip(draws)
            .map(|(i, draw)| KeyedDraw::new(catalog, Subject::Scenery(i), 0, 0, draw))
            .collect();
        keyed.sort_by_key(|keyed| keyed.key);
        Scenery { keyed, viewport }
    }

    /// The draws in depth order, with their keys.
    pub fn keyed(&self) -> &[KeyedDraw] {
        &self.keyed
    }

    /// The viewport it stands in.
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }
}

/// The soil band and its top edge across `viewport`, then the glass at its edges.
///
/// # Panics
/// If the catalog lacks the soil or the glass, which the embedded art has.
pub(crate) fn soil_and_glass(art: &Art, viewport: Viewport) -> Vec<Draw> {
    let catalog = &art.catalog;
    let width = i32::try_from(viewport.width()).unwrap_or(WIDTH);
    let mut draws: Vec<Draw> = soil(catalog, width, 0)
        .into_iter()
        .map(|(_, _, draw)| draw)
        .collect();
    let corners = [
        "tmp.glass.corner.top-left",
        "tmp.glass.corner.top-right",
        "tmp.glass.corner.bottom-left",
        "tmp.glass.corner.bottom-right",
    ];
    draws.extend(glass(
        art,
        corners.map(|name| sprite(catalog, name)),
        viewport,
    ));
    draws
}

/// The soil band's tiles (part 0) and its top edge (part 1) from the left edge to `width`, the
/// texture drifted `offset` pixels right, each with its column's index. A column keeps its
/// index as the texture drifts, so a tile only ever moves a pixel at a time, and only enters
/// or leaves at an edge of the frame.
pub(crate) fn soil(catalog: &Catalog, width: i32, offset: i32) -> Vec<(u8, u16, Draw)> {
    let tiles = [
        sprite(catalog, "tmp.soil.tile.01"),
        sprite(catalog, "tmp.soil.tile.02"),
    ];
    let edges = [
        sprite(catalog, "tmp.soil.edge.01"),
        sprite(catalog, "tmp.soil.edge.02"),
    ];
    let step = i32::from(catalog.sprite(tiles[0]).rect.w).max(1);
    // Every third tile is the second variant, offset between tile and edge, so the band does
    // not repeat every tile. The pattern's period is three tiles, so the drift wraps there.
    let variant = |column: u16, shift: u16| usize::from((column + shift) % 3 == 2);
    let columns = drift::columns(offset, step, SOIL_PERIOD / step, width);
    let mut draws: Vec<(u8, u16, Draw)> = columns
        .iter()
        .map(|&(column, x)| {
            (
                0,
                column,
                Draw::new(catalog, tiles[variant(column, 0)], x, SOIL),
            )
        })
        .collect();
    draws.extend(columns.iter().map(|&(column, x)| {
        let edge = edges[variant(column, 1)];
        (1, column, Draw::new(catalog, edge, x, GROUND))
    }));
    draws
}

/// The glass at its places in `viewport`: the left and right edges, then the four corners.
/// Each edge is clipped to the rows between its corners, so a corner replaces the edge's
/// pixels rather than stacking on them, and the glass is never denser where they meet.
fn glass(art: &Art, corners: [SpriteId; 4], viewport: Viewport) -> Vec<Draw> {
    let catalog = &art.catalog;
    let corners: Vec<Draw> = corners.iter().map(|&id| fixed(art, id, viewport)).collect();
    let mut draws: Vec<Draw> = ["tmp.glass.edge.left", "tmp.glass.edge.right"]
        .iter()
        .map(|name| {
            let edge = fixed(art, sprite(catalog, name), viewport);
            let width = i32::from(catalog.sprite(edge.sprite).rect.w);
            let placed: Vec<(i32, i32, i32)> = corners
                .iter()
                .map(|c| (c.x, c.y, i32::from(catalog.sprite(c.sprite).rect.h)))
                .collect();
            let columns = i32::try_from(viewport.width()).unwrap_or(WIDTH);
            let (top, bottom) = edge_rows(edge.x, &placed, columns);
            Draw {
                clip: Some(Area {
                    x: edge.x,
                    y: top,
                    w: width,
                    h: bottom - top,
                }),
                ..edge
            }
        })
        .collect();
    draws.extend(corners);
    draws
}

/// The rows an edge at `edge_x` keeps in a viewport `columns` wide: below the top corners and
/// above the bottom ones on its side, each corner given as its `(x, y, height)`. A side, not an
/// exact column, matches them, so pieces their own blank columns set out differently still meet.
fn edge_rows(edge_x: i32, corners: &[(i32, i32, i32)], columns: i32) -> (i32, i32) {
    let left = |x: i32| x < columns / 2;
    let (mut top, mut bottom) = (0, HEIGHT);
    for &(_, y, height) in corners.iter().filter(|&&(x, ..)| left(x) == left(edge_x)) {
        if y == 0 {
            top = top.max(height);
        } else {
            bottom = bottom.min(y);
        }
    }
    (top, bottom)
}

/// A piece with a fixed place, drawn in `viewport`. A backdrop wider than the scene is cropped
/// about its centre, so the scene's part of it stands under the scene. Any other piece stands
/// at its side of the viewport, flush with the edge: the glass on the right at the right edge,
/// and the blank columns a piece has towards its edge lying beyond it.
///
/// # Panics
/// If the sprite has no place, which every delivered backdrop and piece of glass has.
pub(crate) fn fixed(art: &Art, id: SpriteId, viewport: Viewport) -> Draw {
    let catalog = &art.catalog;
    let sprite = catalog.sprite(id);
    let at = sprite
        .at
        .unwrap_or_else(|| panic!("the delivered {} has its place", sprite.id));
    let (x, w) = (i32::from(at.x), i32::from(sprite.rect.w));
    let x = match sprite.scene {
        Some(scene) => x + viewport.inset() - i32::from(scene.x),
        None if 2 * x + w > WIDTH => x + viewport.extra() + blank_columns(art, id).1,
        None => x - blank_columns(art, id).0,
    };
    Draw::new(catalog, id, x, i32::from(at.y))
}

/// How many of a sprite's columns are wholly transparent on its left, and on its right.
pub(crate) fn blank_columns(art: &Art, id: SpriteId) -> (i32, i32) {
    let w = usize::from(art.catalog.sprite(id).rect.w);
    let blank = |column: usize| art.rows(id).all(|row| row[column] == 0);
    let left = (0..w).take_while(|&c| blank(c)).count();
    let right = (0..w).rev().take_while(|&c| blank(c)).count().min(w - left);
    (
        i32::try_from(left).unwrap_or(0),
        i32::try_from(right).unwrap_or(0),
    )
}

/// A sprite across a viewport `width` wide from its left edge: repeated edge to edge where the
/// art tiles it (`tileX`), and once otherwise.
pub(crate) fn tiled(catalog: &Catalog, id: SpriteId, width: i32) -> Vec<Draw> {
    let sprite = catalog.sprite(id);
    let step = i32::from(sprite.rect.w).max(1);
    let copies = if sprite.flags.contains(Flags::TILE_X) {
        (width + step - 1) / step
    } else {
        1
    };
    (0..copies)
        .map(|i| Draw::new(catalog, id, i * step, 0))
        .collect()
}

/// A sprite the delivered art is known to hold, by its ID.
///
/// # Panics
/// If the catalog lacks it.
pub(crate) fn sprite(catalog: &Catalog, name: &str) -> SpriteId {
    catalog
        .id(name)
        .unwrap_or_else(|| panic!("the delivered {name}"))
}

/// The label plate's area.
pub fn label_area(catalog: &Catalog) -> Option<Area> {
    let plate = catalog.sprite(catalog.id(Label::PLATE)?).rect;
    Some(Area {
        x: LABEL_X,
        y: LABEL_Y,
        w: i32::from(plate.w),
        h: i32::from(plate.h),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::catalog::{Flags, Layer, Point};
    use crate::frame::PLANT_GROUND_Y;
    use terrarium_truth::scene::Weather;

    fn salts() -> impl Iterator<Item = InstallSalt> {
        (0..300_u128).map(|n| {
            InstallSalt::new(
                n.wrapping_mul(0x0f1e_2d3c_4b5a_6978_8796_a5b4_c3d2_e1f0)
                    .to_le_bytes(),
            )
        })
    }

    fn table() -> ArtTable {
        ArtTable::new(&Art::embedded().catalog).unwrap()
    }

    /// Whether two rectangles, `(x, y, w, h)`, touch or overlap.
    fn touch(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
        a.0 <= b.0 + b.2 && b.0 <= a.0 + a.2 && a.1 <= b.1 + b.3 && b.1 <= a.1 + a.3
    }

    #[test]
    fn salt_indices_of_different_kinds_never_meet() {
        let kinds = [
            Kind::Monument,
            Kind::Fossil,
            Kind::Pebble,
            Kind::Dressing,
            Kind::Litter,
            Kind::Sun,
            Kind::Moon,
            Kind::Star,
            Kind::Cloud,
            Kind::Weed,
            Kind::Fungus,
            Kind::Mark,
            Kind::Resident,
        ];
        let codes: BTreeSet<u64> = kinds.iter().map(|k| k.code()).collect();
        assert_eq!(codes.len(), kinds.len());
        // The largest item any sprite choice draws: the last mark of the largest monument id.
        let largest = (u64::from(u32::MAX) << 2) | 3;
        for kind in kinds {
            assert_eq!(kind.index(largest, 0) >> 40, kind.code());
            assert_eq!(kind.index(15, 1) >> 40, kind.code());
        }
        // A place never shares an index with another place or a part of the same item.
        assert_ne!(Kind::Star.index(3, 0), Kind::Star.index(3, 1));
    }

    #[test]
    fn every_place_stays_inside_the_glass_and_monuments_keep_one_per_third_at_every_width() {
        for columns in [640, 651, 735, 756, 840] {
            let viewport = Viewport::new(columns);
            let width = viewport.width().cast_signed();
            for salt in salts() {
                let layout = WorldLayout::new(salt, table().world(), viewport);
                for spot in 0..3_u8 {
                    let x = layout.monument(spot);
                    assert_eq!(viewport.third(x), i32::from(spot), "{x} at {columns}");
                }
                let count = |n: usize| 0..u8::try_from(n).unwrap();
                let xs = count(FOSSILS)
                    .map(|i| layout.fossil(i))
                    .chain(count(DRESSING).map(|i| layout.dressing(i)))
                    .chain(count(LITTER).map(|i| layout.litter(i)))
                    .chain(count(FUNGUS).map(|i| layout.fungus(i)))
                    .chain(count(RESIDENTS).map(|i| viewport.spread(layout.resident(i))))
                    .chain(layout.pebbles().iter().map(|&(x, _)| x));
                for x in xs {
                    assert!((8..width - 8).contains(&x), "{x} at {columns}");
                }
                assert!(
                    layout
                        .pebbles()
                        .iter()
                        .all(|&(_, y)| (69..=72).contains(&y))
                );
                // The sky's places are the scene's, at every width.
                let scene = WorldLayout::new(salt, table().world(), Viewport::SCENE);
                assert_eq!((layout.sun(), layout.moon()), (scene.sun(), scene.moon()));
            }
        }
    }

    #[test]
    fn the_ground_spreads_with_the_width_and_stands_still_at_640() {
        let salt = salts().next().unwrap();
        let scene = WorldLayout::new(salt, table().world(), Viewport::SCENE);
        let wide = scene.in_viewport(Viewport::new(840));
        assert_eq!(
            wide,
            WorldLayout::new(salt, table().world(), Viewport::new(840))
        );
        assert_eq!(scene.in_viewport(Viewport::SCENE), scene);
        for i in 0..u8::try_from(LITTER).unwrap() {
            assert_eq!(wide.litter(i), Viewport::new(840).spread(scene.litter(i)));
        }
        // Spread apart, never closer: the first and last litter are further from the edges.
        let last = u8::try_from(LITTER - 1).unwrap();
        assert!(wide.litter(0) >= scene.litter(0));
        assert!(840 - wide.litter(last) >= 640 - scene.litter(last));
        assert!(wide.litter(last) - wide.litter(0) > scene.litter(last) - scene.litter(0));
    }

    #[test]
    fn stars_never_touch_the_sun_the_moon_or_the_label() {
        let table = table();
        for salt in salts() {
            let layout = WorldLayout::new(salt, table.world(), Viewport::SCENE);
            let (side, star_side) = layout.sides();
            let body = |(x, y): (i32, i32)| (x, y, side, side);
            let label = (LABEL_X, LABEL_Y, 90, 9);
            for i in 0..7_u8 {
                let (x, y) = layout.star(i);
                let star = (x, y, star_side, star_side);
                assert!(!touch(star, body(layout.sun())), "{salt:?} {i}");
                assert!(!touch(star, body(layout.moon())), "{salt:?} {i}");
                assert!(!touch(star, label));
                assert!(
                    sky::BODY_ROWS.contains(&y) && sky::BODY_ROWS.contains(&(y + star_side - 1))
                );
            }
            for body in [layout.sun(), layout.moon()] {
                assert!(!touch((body.0, body.1, side, side), label));
                assert!(sky::BODY_ROWS.contains(&(body.1 + side - 1)));
            }
        }
    }

    #[test]
    fn no_pebble_lies_over_a_fossil_spot() {
        for columns in [640, 735, 840] {
            for salt in salts() {
                let layout = WorldLayout::new(salt, table().world(), Viewport::new(columns));
                for &(x, _) in layout.pebbles() {
                    for spot in 0..6_u8 {
                        let fossil = layout.fossil(spot);
                        assert!(
                            x + 2 < fossil - 1 || x - 1 > fossil + 8,
                            "{x} over {fossil} at {columns}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn clouds_keep_rows_of_their_own_between_the_bodies_and_the_hills() {
        let catalog = &Art::embedded().catalog;
        let table = table();
        let tallest = [
            sky::CloudSize::Small,
            sky::CloudSize::Medium,
            sky::CloudSize::Large,
        ]
        .iter()
        .flat_map(|&size| table.world().clouds(size))
        .map(|&id| i32::from(catalog.sprite(id).rect.h))
        .max()
        .unwrap();
        let label = (LABEL_X, LABEL_Y, 90, 9);
        for salt in salts() {
            let layout = WorldLayout::new(salt, table.world(), Viewport::SCENE);
            let paths = u8::try_from(sky::MAX_CLOUDS).unwrap();
            let rows: Vec<i32> = (0..paths).map(|i| layout.cloud(i).y).collect();
            assert!(rows.windows(2).all(|pair| pair[0] < pair[1]), "{rows:?}");
            for y in rows {
                assert!(y > *sky::BODY_ROWS.end() && y >= sky::CLOUD_TOP, "{y}");
                // Any cloud on any path stays above the floor and clear of the label's rows.
                assert!(y + tallest <= sky::CLOUD_FLOOR, "{y} + {tallest}");
                assert!(!touch((0, y, WIDTH, tallest), label), "{y}");
            }
        }
    }

    #[test]
    fn the_corners_replace_the_edge_pixels_and_the_films_lie_under_both() {
        let art = Art::embedded();
        let table = table();
        let scenery = Scenery::new(
            art,
            &table,
            &WorldLayout::new(crate::preset::SALT, table.world(), Viewport::SCENE),
        );
        let glass: Vec<&KeyedDraw> = scenery
            .keyed()
            .iter()
            .filter(|keyed| keyed.key.layer == Layer::Glass)
            .collect();
        assert_eq!(
            glass.len(),
            6,
            "two edges and four corners, no top or bottom edge"
        );
        // No pixel of an edge is painted where a corner stands.
        let area = |keyed: &KeyedDraw| {
            let rect = art.catalog.sprite(keyed.draw.sprite).rect;
            let mut area = (
                keyed.draw.x,
                keyed.draw.y,
                i32::from(rect.w),
                i32::from(rect.h),
            );
            if let Some(clip) = keyed.draw.clip {
                let bottom = (area.1 + area.3).min(clip.y + clip.h);
                area.1 = area.1.max(clip.y);
                area.3 = bottom - area.1;
            }
            area
        };
        let overlap = |a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)| {
            a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
        };
        let (edges, corners): (Vec<&KeyedDraw>, Vec<&KeyedDraw>) = glass
            .iter()
            .partition(|keyed| art.catalog.sprite(keyed.draw.sprite).id.contains(".edge."));
        assert_eq!((edges.len(), corners.len()), (2, 4));
        for edge in &edges {
            let edge = area(edge);
            assert_eq!(
                edge.3,
                HEIGHT - 12,
                "an edge spans the rows between its corners"
            );
            for corner in &corners {
                assert!(!overlap(edge, area(corner)));
            }
        }
        // A film keys at the frame's top-left, like the left edge: its subject goes first.
        assert!(Subject::Film(crate::variants::GlassFilm::Dust) < Subject::Scenery(0));
        let plate = scenery
            .keyed()
            .iter()
            .find(|keyed| keyed.key.layer == Layer::Label)
            .unwrap();
        assert_eq!((plate.draw.x, plate.draw.y), (LABEL_X, LABEL_Y));
    }

    #[test]
    fn the_background_keeps_the_sizes_the_composer_draws_it_at() {
        let art = Art::embedded();
        let catalog = &art.catalog;
        let table = table();
        let size = |id: SpriteId| {
            let rect = catalog.sprite(id).rect;
            (i32::from(rect.w), i32::from(rect.h))
        };
        let at = |id: SpriteId| {
            catalog
                .sprite(id)
                .at
                .map(|p| (i32::from(p.x), i32::from(p.y)))
        };
        // Each weather's backdrop is the widest viewport from its top-left, with the scene in
        // its middle, so every viewport crops it about its centre.
        let widest = i32::try_from(crate::frame::WIDEST).unwrap();
        for weather in Weather::ALL {
            let backdrop = table.weather(*weather)[0];
            let sprite = catalog.sprite(backdrop);
            assert_eq!(size(backdrop), (widest, HEIGHT), "{}", sprite.id);
            assert!(sprite.flags.contains(Flags::BACKDROP), "{}", sprite.id);
            assert_eq!(sprite.at, Some(Point::default()), "{}", sprite.id);
            let scene = sprite.scene.unwrap();
            assert_eq!(
                (i32::from(scene.x), i32::from(scene.w), i32::from(scene.h)),
                ((widest - WIDTH) / 2, WIDTH, HEIGHT),
                "{}",
                sprite.id
            );
            for columns in [640, 641, 735, 840] {
                let viewport = Viewport::new(columns);
                let draw = fixed(art, backdrop, viewport);
                let right = i32::try_from(columns).unwrap();
                assert!(draw.x <= 0 && draw.x + widest >= right, "{columns}");
                assert_eq!(
                    draw.x + i32::from(scene.x),
                    viewport.inset(),
                    "the scene's part under it"
                );
            }
        }
        // The soil's pattern is three tiles, so its drift wraps at three tile widths. The body
        // fills the frame below the edge's first row, and the edge is the ground line and the
        // row below it.
        for name in ["tmp.soil.tile.01", "tmp.soil.tile.02"] {
            assert_eq!(
                size(sprite(catalog, name)),
                (SOIL_PERIOD / 3, HEIGHT - SOIL)
            );
        }
        for name in ["tmp.soil.edge.01", "tmp.soil.edge.02"] {
            assert_eq!(
                size(sprite(catalog, name)),
                (SOIL_PERIOD / 3, SOIL - GROUND + 1),
                "{name}"
            );
        }
        // The glass runs the frame's height at its two ends, and its corners are one square
        // size at the frame's four corners.
        for name in ["tmp.glass.edge.left", "tmp.glass.edge.right"] {
            let edge = sprite(catalog, name);
            assert_eq!(size(edge).1, HEIGHT, "{name}");
            let x = at(edge).unwrap().0;
            assert!(x == 0 || x == WIDTH - size(edge).0, "{name} at {x}");
        }
        let corners = table.world().corners;
        let corner = size(corners[0]).0;
        assert!(corners.iter().all(|&c| size(c) == (corner, corner)));
        let places: BTreeSet<(i32, i32)> = corners.iter().filter_map(|&c| at(c)).collect();
        let frame_corners = [0, WIDTH - corner]
            .into_iter()
            .flat_map(|x| [(x, 0), (x, HEIGHT - corner)])
            .collect();
        assert_eq!(places, frame_corners);
        let plate = label_area(catalog).unwrap();
        assert_eq!((plate.w, plate.h), (90, 9));
    }

    #[test]
    fn an_edge_keeps_the_rows_between_the_corners_on_its_side() {
        // The corners stand a column further in than their edges: the side still matches them.
        let corners = [(0, 0, 6), (0, 68, 6), (729, 0, 6), (729, 68, 6)];
        assert_eq!(edge_rows(-1, &corners, 735), (6, 68));
        assert_eq!(edge_rows(730, &corners, 735), (6, 68));
        // A side without corners keeps the full height.
        assert_eq!(edge_rows(730, &corners[..2], 735), (0, HEIGHT));
    }

    #[test]
    fn plants_stand_on_the_row_above_the_soil_s_edge() {
        let catalog = &Art::embedded().catalog;
        assert_eq!((PLANT_GROUND_Y, GROUND, SOIL), (64, 65, 66));
        for (part, _, draw) in soil(catalog, WIDTH, 0) {
            let y = if part == 0 { SOIL } else { GROUND };
            assert_eq!(draw.y, y, "{}", catalog.sprite(draw.sprite).id);
        }
    }
}
