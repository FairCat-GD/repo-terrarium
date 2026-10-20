//! The slots: a hundred and eight places along the strip where a plant or a filler can stand,
//! in three depth bands (constraint 3).
//!
//! The structure is fixed: each cell's band, and so the rhythm, never changes. Each third of
//! the strip has thirty-six cells, four of them for trees, standing as a wood does: in a pair,
//! a grove or alone, never the same way in two thirds. No two neighbouring cells share
//! a band, so the skyline steps tall, medium, small. The cells span the scene's 640 columns,
//! and a wider viewport spreads them to its edges ([`Viewport::spread`]). The salt only moves
//! each slot within its cell: a stratified jitter, the Poisson-disk style with a guaranteed gap.
//! Placement reads cells, never positions, so what is drawn never depends on the salt (I-2) or
//! on the width.

use std::sync::OnceLock;

use crate::catalog::Layer;
use crate::frame::Viewport;
use crate::salt::{InstallSalt, Purpose};

/// A depth band: which plant layer a cell holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Band {
    Far,
    Mid,
    Near,
}

impl Band {
    pub const ALL: [Band; 3] = [Band::Far, Band::Mid, Band::Near];

    pub const fn layer(self) -> Layer {
        match self {
            Band::Far => Layer::FarPlants,
            Band::Mid => Layer::MidPlants,
            Band::Near => Layer::NearPlants,
        }
    }

    pub fn from_layer(layer: Layer) -> Option<Band> {
        Band::ALL.into_iter().find(|band| band.layer() == layer)
    }
}

pub const CELLS: usize = 108;
pub const CELLS_PER_THIRD: usize = 36;

const F: Band = Band::Far;
const M: Band = Band::Mid;
const N: Band = Band::Near;

/// Each cell's band, left to right, a third to a row: trees in a pair and alone, then alone
/// and in a grove of three, then in two pairs.
#[rustfmt::skip]
pub const PATTERN: [Band; CELLS] = [
    N, M, N, M, N, F, M, N, M, N, F, N, M, N, M, N, M, N, M, N, F, M, N, M, N, M, N, M, N, M, F, N, M, N, M, N,
    M, N, M, F, M, N, M, N, M, N, M, N, M, F, N, M, N, F, M, N, M, N, F, N, M, N, M, N, M, N, M, N, M, N, M, N,
    M, N, M, F, M, N, M, N, F, N, M, N, M, N, M, N, M, N, M, N, M, N, F, M, N, M, F, N, M, N, M, N, M, N, M, N,
];

/// The cells span x 28..612 of the scene: inside the 6-pixel glass edges, with room for the
/// widest crown of each band at its first and last cells.
const LEFT: i32 = 28;
const SPAN: i32 = 584;
/// How far the salt may move a slot from its cell's centre, either way.
pub const JITTER: i32 = 2;

/// Where a cell's plant or filler stands: the x of its ground point, in viewport columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub band: Band,
    pub x: i32,
}

/// The slots of one install in one viewport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Each slot's x in the scene, where the salt moved it.
    scene: [i32; CELLS],
    slots: [Slot; CELLS],
}

impl Layout {
    pub fn new(salt: InstallSalt, viewport: Viewport) -> Layout {
        let span = u64::try_from(2 * JITTER + 1).unwrap_or(1);
        let scene: [i32; CELLS] = std::array::from_fn(|cell| {
            let index = u64::try_from(cell).unwrap_or(0);
            let shift = salt.below(Purpose::SlotJitter, index, span);
            centre(cell) + i32::try_from(shift).unwrap_or(JITTER) - JITTER
        });
        Layout {
            scene,
            slots: std::array::from_fn(|cell| Slot {
                band: PATTERN[cell],
                x: viewport.spread(scene[cell]),
            }),
        }
    }

    /// # Panics
    /// If `cell` is not below [`CELLS`].
    pub fn slot(&self, cell: u8) -> Slot {
        self.slots[usize::from(cell)]
    }

    pub fn slots(&self) -> &[Slot; CELLS] {
        &self.slots
    }

    /// Where a cell's slot stands in the scene, before the width spreads it: what a resident
    /// beside its plant keeps as home, so its home holds through a change of width.
    ///
    /// # Panics
    /// If `cell` is not below [`CELLS`].
    pub fn scene_x(&self, cell: u8) -> i32 {
        self.scene[usize::from(cell)]
    }
}

/// A cell's centre in the scene, before the width spreads it and the salt moves it.
pub fn centre(cell: usize) -> i32 {
    let doubled = i32::try_from(2 * cell + 1).unwrap_or(0);
    let cells = i32::try_from(2 * CELLS).unwrap_or(1);
    LEFT + (SPAN * doubled + cells / 2) / cells
}

/// The x in the scene where cell `k` begins, which is where cell `k − 1` ends: the points
/// between slots.
pub fn boundary(k: usize) -> i32 {
    let k = i32::try_from(k).unwrap_or(0);
    let cells = i32::try_from(CELLS).unwrap_or(1);
    LEFT + SPAN * k / cells
}

/// A band's cells, left to right. Neighbours in this list are neighbours in the band.
pub fn band_cells(band: Band) -> &'static [u8] {
    static CELLS_BY_BAND: OnceLock<[Vec<u8>; 3]> = OnceLock::new();
    let cells = CELLS_BY_BAND.get_or_init(|| {
        Band::ALL.map(|band| {
            (0..CELLS)
                .filter(|&cell| PATTERN[cell] == band)
                .filter_map(|cell| u8::try_from(cell).ok())
                .collect()
        })
    });
    match band {
        Band::Far => &cells[0],
        Band::Mid => &cells[1],
        Band::Near => &cells[2],
    }
}

/// Which third of the strip a cell belongs to: 0, 1 or 2.
pub fn third(cell: u8) -> usize {
    usize::from(cell) / CELLS_PER_THIRD
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Art;
    use crate::plant::Parts;
    use crate::variants::ArtTable;
    use terrarium_truth::scene::{GrowthStage, PlantSpecies, PlantState};

    fn salts() -> impl Iterator<Item = InstallSalt> {
        (0..200_u128).map(|n| InstallSalt::new((n * 0x0123_4567_89ab_cdef_0f1e_2d3c).to_le_bytes()))
    }

    /// The widths a strip takes, from the scene's to the widest the art covers.
    fn widths() -> impl Iterator<Item = Viewport> {
        [640, 651, 735, 756, 840].into_iter().map(Viewport::new)
    }

    #[test]
    fn the_pattern_has_the_band_counts_and_never_repeats_a_band() {
        let count = |band| PATTERN.iter().filter(|&&b| b == band).count();
        assert_eq!((count(F), count(M), count(N)), (12, 47, 49));
        assert_eq!(
            band_cells(Band::Far),
            [5, 10, 20, 30, 39, 49, 53, 58, 75, 80, 94, 98]
        );
        assert!(PATTERN.windows(2).all(|pair| pair[0] != pair[1]));
        let mut arrangements = Vec::new();
        for t in 0..3 {
            let trees: Vec<u8> = band_cells(Band::Far)
                .iter()
                .copied()
                .filter(|&c| third(c) == t)
                .collect();
            assert_eq!(trees.len(), 4, "third {t}");
            // Their spacing: never so close the trunks merge, and never the same twice.
            let gaps: Vec<u8> = trees.windows(2).map(|pair| pair[1] - pair[0]).collect();
            assert!(gaps.iter().all(|&gap| gap >= 4), "third {t}: {gaps:?}");
            arrangements.push(gaps);
        }
        arrangements.dedup();
        assert_eq!(arrangements.len(), 3);
    }

    #[test]
    fn the_salt_moves_slots_within_their_cells_only() {
        let a = Layout::new(salts().next().unwrap(), Viewport::SCENE);
        let b = Layout::new(salts().nth(1).unwrap(), Viewport::SCENE);
        assert_ne!(a, b);
        for viewport in widths() {
            for salt in salts() {
                let layout = Layout::new(salt, viewport);
                for (cell, slot) in layout.slots().iter().enumerate() {
                    let scene = layout.scene_x(u8::try_from(cell).unwrap());
                    assert_eq!(slot.band, PATTERN[cell]);
                    assert!((scene - centre(cell)).abs() <= JITTER);
                    assert_eq!(slot.x, viewport.spread(scene));
                }
            }
        }
    }

    #[test]
    fn slots_keep_their_order_and_gaps_for_every_salt_and_width() {
        for viewport in widths() {
            for salt in salts() {
                let layout = Layout::new(salt, viewport);
                let xs: Vec<i32> = layout.slots().iter().map(|s| s.x).collect();
                assert!(xs.windows(2).all(|pair| pair[1] > pair[0]), "{xs:?}");
                for (band, gap) in [(Band::Far, 17), (Band::Mid, 6), (Band::Near, 6)] {
                    let cells = band_cells(band);
                    for pair in cells.windows(2) {
                        let apart = layout.slot(pair[1]).x - layout.slot(pair[0]).x;
                        assert!(apart >= gap, "{band:?} {pair:?}: {apart}");
                    }
                }
            }
        }
    }

    #[test]
    fn every_tree_cell_stays_inside_its_third_at_every_width() {
        for viewport in widths() {
            for &cell in band_cells(Band::Far) {
                let x = centre(usize::from(cell));
                for x in [x - JITTER, x + JITTER].map(|x| viewport.spread(x)) {
                    assert_eq!(
                        viewport.third(x),
                        i32::try_from(third(cell)).unwrap(),
                        "{cell} at {}",
                        viewport.width()
                    );
                }
            }
        }
    }

    #[test]
    fn the_widest_mature_plant_of_each_band_stays_inside_the_glass_at_every_width() {
        // Measured from the art: each band's furthest reach left and right of the ground
        // point, over every mature look and bloom, at the jitter's extremes.
        let art = Art::embedded();
        let table = ArtTable::new(&art.catalog).unwrap();
        let glass = 6;
        for band in Band::ALL {
            let mut reach = (0, 0);
            for &species in PlantSpecies::ALL {
                if Band::from_layer(table.plant_layer(species)) != Some(band) {
                    continue;
                }
                let mut sprites = vec![table.bloom(species)];
                for &state in PlantState::ALL {
                    let parts = Parts::new(&table, species, GrowthStage::Mature, state, false);
                    sprites.extend(parts.sprites());
                }
                for id in sprites {
                    let sprite = art.catalog.sprite(id);
                    let left = i32::from(sprite.anchor.x);
                    let right = i32::from(sprite.rect.w) - 1 - left;
                    reach = (reach.0.max(left), reach.1.max(right));
                }
            }
            for viewport in widths() {
                let right_glass = viewport.width().cast_signed() - glass;
                for &cell in band_cells(band) {
                    let x = centre(usize::from(cell));
                    let [left, right] = [x - JITTER, x + JITTER].map(|x| viewport.spread(x));
                    let at = viewport.width();
                    assert!(left - reach.0 >= glass, "{band:?} {cell} at {at}");
                    assert!(right + reach.1 < right_glass, "{band:?} {cell} at {at}");
                }
            }
        }
    }
}
