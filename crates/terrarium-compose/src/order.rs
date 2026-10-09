//! Draw order: layers back to front, and within a layer by the world point's y, then its x,
//! then a stable id. The world point is where the anchor lands, the ground point for a plant,
//! never `Draw.y`, which is a top-left: foliage rises above its stem, so sorting on it would
//! draw the foliage first. The compositor paints in the order given and never sorts.

use crate::catalog::{Catalog, Layer};
use crate::frame::Draw;
use crate::variants::GlassFilm;

/// What a draw belongs to: its stable id within a layer. Variant order breaks ties between
/// draws at one point: a glass film paints under the glass edges, and a plant over a filler
/// fading out where it now stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Subject {
    /// A film on the glass: condensation or dust.
    Film(GlassFilm),
    /// Fixed scenery, by its place in the scenery list.
    Scenery(u16),
    /// The soil band, its tiles and its top edge, drifting.
    Soil,
    /// The weather's backdrop: the whole scene behind everything.
    Weather,
    /// A cloud, by its path.
    Cloud(u8),
    Sun,
    Moon,
    Star(u8),
    /// The storm front's curtain while it crosses the sky.
    Front,
    /// A monument at its spot. Like a plant's cell, the spot is part of its identity.
    Monument {
        id: u32,
        spot: u8,
    },
    /// A fossil at its spot in the soil band.
    Fossil {
        id: u32,
        spot: u8,
    },
    /// A decorative tuft between the slots: dressing, never filler.
    Dressing(u8),
    /// The weeds in a cell.
    Weed {
        cell: u8,
    },
    /// The filler in a cell.
    Fill {
        cell: u8,
    },
    /// A plant in a cell. The cell is part of its identity, so a plant that ever stood
    /// elsewhere is another subject, fading in while the old one fades out.
    Plant {
        id: u32,
        cell: u8,
    },
    /// A resident in its stretch of the strip. Like a plant's cell, the stretch is part of its
    /// identity.
    Resident {
        id: u32,
        spot: u8,
    },
    /// A piece of litter, by its place along the ground.
    Litter(u8),
    /// A fungus, by its place along the ground.
    Fungus(u8),
    /// Lightning's bolt.
    Bolt,
    /// Rain after drought's streaks, across the viewport.
    Rain,
    /// A leaf a windstorm carries, by its place in the flurry.
    Leaf(u8),
    /// The dusk or sunburst light over the scene.
    Light,
    /// The label's text; the plate is scenery.
    Label,
    /// The faint reflection that crosses the glass now and then.
    Sweep,
}

impl Subject {
    /// Whether it stands in the 640-wide scene, which a wider viewport shows in its middle.
    /// The rest belong to the viewport and reach its edges: the backdrop, the clouds, the
    /// soil, the glass and its films and sweep, the light, the label, and an event's pieces,
    /// which the event places across the viewport itself.
    pub const fn in_scene(self) -> bool {
        match self {
            Subject::Sun
            | Subject::Moon
            | Subject::Star(_)
            | Subject::Monument { .. }
            | Subject::Fossil { .. }
            | Subject::Dressing(_)
            | Subject::Weed { .. }
            | Subject::Fill { .. }
            | Subject::Plant { .. }
            | Subject::Resident { .. }
            | Subject::Litter(_)
            | Subject::Fungus(_) => true,
            Subject::Film(_)
            | Subject::Scenery(_)
            | Subject::Soil
            | Subject::Weather
            | Subject::Cloud(_)
            | Subject::Front
            | Subject::Bolt
            | Subject::Rain
            | Subject::Leaf(_)
            | Subject::Light
            | Subject::Label
            | Subject::Sweep => false,
        }
    }

    /// The cell a plant, a filler or a weed stands in.
    pub const fn cell(self) -> Option<u8> {
        match self {
            Subject::Weed { cell } | Subject::Fill { cell } | Subject::Plant { cell, .. } => {
                Some(cell)
            }
            _ => None,
        }
    }
}

/// The sort key of one draw. Field order is sort order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawKey {
    pub layer: Layer,
    pub y: i32,
    pub x: i32,
    pub subject: Subject,
    /// The part, bottom first: stem, foliage, bloom.
    pub part: u8,
    /// The look within the part, oldest first, so a new look covers the one it replaces.
    pub look: u16,
    /// One draw of a look that has several: a weather tile, a glyph. 0 otherwise.
    pub item: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyedDraw {
    pub key: DrawKey,
    pub draw: Draw,
}

/// Where a keyed draw belongs: its subject, part, look and item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub subject: Subject,
    pub part: u8,
    pub look: u16,
    pub item: u16,
}

impl Place {
    pub const fn new(subject: Subject, part: u8, look: u16) -> Place {
        Place {
            subject,
            part,
            look,
            item: 0,
        }
    }

    #[must_use]
    pub const fn item(self, item: u16) -> Place {
        Place { item, ..self }
    }
}

impl KeyedDraw {
    /// `draw`, keyed by its layer and the world point its anchor lands on.
    pub fn new(catalog: &Catalog, subject: Subject, part: u8, look: u16, draw: Draw) -> Self {
        KeyedDraw::placed(catalog, Place::new(subject, part, look), draw)
    }

    /// As [`KeyedDraw::new`], with an item for a look that has several draws.
    pub fn placed(catalog: &Catalog, place: Place, draw: Draw) -> Self {
        let sprite = catalog.sprite(draw.sprite);
        let anchor_x = if draw.flip_x {
            i32::from(sprite.rect.w) - 1 - i32::from(sprite.anchor.x)
        } else {
            i32::from(sprite.anchor.x)
        };
        let point = (draw.x + anchor_x, draw.y + i32::from(sprite.anchor.y));
        KeyedDraw::at(place, point, draw)
    }

    /// `draw`, keyed at a world point of the caller's: a subject whose parts must sort
    /// together, like a stone and the marks carved on it, shares one point. The layer is
    /// always the draw's own, so the depth order can never go backwards.
    pub fn at(place: Place, (x, y): (i32, i32), draw: Draw) -> Self {
        KeyedDraw {
            key: DrawKey {
                layer: draw.layer,
                y,
                x,
                subject: place.subject,
                part: place.part,
                look: place.look,
                item: place.item,
            },
            draw,
        }
    }
}

/// Sorts by key, stably, and drops the keys.
pub fn in_order(mut keyed: Vec<KeyedDraw>) -> Vec<Draw> {
    keyed.sort_by_key(|keyed| keyed.key);
    keyed.into_iter().map(|keyed| keyed.draw).collect()
}

/// Two key-sorted lists as one, in key order; on equal keys the first list's draw goes first.
pub fn merged(a: &[KeyedDraw], b: &[KeyedDraw]) -> Vec<Draw> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(a.len() + b.len());
    while i < a.len() || j < b.len() {
        let take_a = j == b.len() || (i < a.len() && a[i].key <= b[j].key);
        if take_a {
            out.push(a[i].draw);
            i += 1;
        } else {
            out.push(b[j].draw);
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Art;
    use crate::frame::PLANT_GROUND_Y;

    #[test]
    fn a_plant_s_foliage_follows_its_stem_though_it_starts_higher() {
        let catalog = &Art::embedded().catalog;
        let stem = catalog.id("tmp.plant.oak.stem.mature").unwrap();
        let foliage = catalog.id("tmp.plant.oak.foliage.mature.thriving").unwrap();
        let plant = Subject::Plant { id: 3, cell: 2 };
        let stem = KeyedDraw::new(
            catalog,
            plant,
            0,
            0,
            Draw::anchored(catalog, stem, 100, PLANT_GROUND_Y),
        );
        let foliage = KeyedDraw::new(
            catalog,
            plant,
            1,
            0,
            Draw::anchored(catalog, foliage, 100, PLANT_GROUND_Y),
        );
        // The foliage's top-left is far above the stem's, yet both keys hold the ground point.
        assert!(foliage.draw.y < stem.draw.y);
        assert_eq!((stem.key.x, stem.key.y), (100, PLANT_GROUND_Y));
        assert_eq!((foliage.key.x, foliage.key.y), (100, PLANT_GROUND_Y));
        assert_eq!(in_order(vec![foliage, stem]), [stem.draw, foliage.draw]);
    }

    #[test]
    fn layers_come_first_then_y_then_x_then_the_subject() {
        let catalog = &Art::embedded().catalog;
        let tuft = catalog.id("tmp.ground.tuft.01").unwrap();
        let oak = catalog.id("tmp.plant.oak.seedling.thriving").unwrap();
        let at = |sprite, x, subject| {
            KeyedDraw::new(
                catalog,
                subject,
                0,
                0,
                Draw::anchored(catalog, sprite, x, PLANT_GROUND_Y),
            )
        };
        let near_left = at(tuft, 10, Subject::Fill { cell: 0 });
        let near_right = at(tuft, 20, Subject::Fill { cell: 1 });
        let far_right = at(oak, 30, Subject::Plant { id: 1, cell: 2 });
        // A plant fading in where a filler fades out: the same point, so the subject decides.
        let plant = at(tuft, 20, Subject::Plant { id: 0, cell: 1 });
        assert_eq!(
            in_order(vec![plant, near_right, near_left, far_right]),
            [far_right.draw, near_left.draw, near_right.draw, plant.draw]
        );
    }

    #[test]
    fn merging_two_sorted_lists_equals_sorting_them_together() {
        let catalog = &Art::embedded().catalog;
        let tuft = catalog.id("tmp.ground.tuft.01").unwrap();
        let at = |x, subject| {
            KeyedDraw::new(
                catalog,
                subject,
                0,
                0,
                Draw::anchored(catalog, tuft, x, PLANT_GROUND_Y),
            )
        };
        let a = [
            at(10, Subject::Fill { cell: 0 }),
            at(30, Subject::Fill { cell: 2 }),
        ];
        let b = [
            at(5, Subject::Scenery(0)),
            at(30, Subject::Scenery(1)),
            at(40, Subject::Scenery(2)),
        ];
        let together: Vec<KeyedDraw> = a.iter().chain(&b).copied().collect();
        assert_eq!(merged(&a, &b), in_order(together));
        assert_eq!(merged(&[], &b), in_order(b.to_vec()));
    }

    #[test]
    fn a_mirrored_draw_is_keyed_by_its_mirrored_anchor() {
        let catalog = &Art::embedded().catalog;
        let (index, _) = catalog
            .sprites
            .iter()
            .enumerate()
            .find(|(_, s)| s.rect.w == 32 && s.anchor.x == 16)
            .expect("a resident frame");
        let frame = crate::catalog::SpriteId::from_index(index).unwrap();
        let left = Draw::anchored(catalog, frame, 200, 60).mirrored(catalog);
        let keyed = KeyedDraw::new(catalog, Subject::Scenery(0), 0, 0, left);
        assert_eq!((keyed.key.x, keyed.key.y), (200, 60));
    }
}
