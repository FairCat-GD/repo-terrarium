//! Draw order: layers back to front, and within a layer by the world point's y, then its x,
//! then a stable id. The world point is where the anchor lands, the ground point for a plant,
//! never `Draw.y`, which is a top-left: foliage rises above its stem, so sorting on it would
//! draw the foliage first. The compositor paints in the order given and never sorts.

use crate::catalog::{Catalog, Layer};
use crate::frame::Draw;

/// What a draw belongs to: its stable id within a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Subject {
    /// Fixed scenery, by its place in the scenery list.
    Scenery(u16),
    /// The filler in a cell.
    Fill { cell: u8 },
    /// A plant in a cell. The cell is part of its identity, so a plant that ever stood
    /// elsewhere is another subject, fading in while the old one fades out.
    Plant { id: u32, cell: u8 },
}

impl Subject {
    /// The cell a filler or plant stands in.
    pub const fn cell(self) -> Option<u8> {
        match self {
            Subject::Scenery(_) => None,
            Subject::Fill { cell } | Subject::Plant { cell, .. } => Some(cell),
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyedDraw {
    pub key: DrawKey,
    pub draw: Draw,
}

impl KeyedDraw {
    /// `draw`, keyed by its layer and the world point its anchor lands on.
    pub fn new(catalog: &Catalog, subject: Subject, part: u8, look: u16, draw: Draw) -> Self {
        let sprite = catalog.sprite(draw.sprite);
        let anchor_x = if draw.flip_x {
            i32::from(sprite.rect.w) - 1 - i32::from(sprite.anchor.x)
        } else {
            i32::from(sprite.anchor.x)
        };
        KeyedDraw {
            key: DrawKey {
                layer: draw.layer,
                y: draw.y + i32::from(sprite.anchor.y),
                x: draw.x + anchor_x,
                subject,
                part,
                look,
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
