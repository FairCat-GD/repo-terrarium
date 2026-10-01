//! The slots: thirty places along the strip where a plant or a filler can stand, in three depth
//! bands (constraint 3).
//!
//! The structure is fixed: each cell's band, and so the rhythm, never changes. Each third of
//! the strip has ten cells, two of them for trees. The tree spacing differs between the thirds,
//! so the trees never stand in a regimented row, and no two neighbouring cells share a band, so
//! the skyline steps tall, medium, small. The salt only moves each slot within its cell: a
//! stratified jitter, the Poisson-disk style with a guaranteed gap. Placement reads cells,
//! never positions, so what is drawn never depends on the salt (I-2).

use std::sync::OnceLock;

use crate::catalog::Layer;
use crate::frame::NATIVE_WIDTH;
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

pub const CELLS: usize = 30;
pub const CELLS_PER_THIRD: usize = 10;

const F: Band = Band::Far;
const M: Band = Band::Mid;
const N: Band = Band::Near;

/// Each cell's band, left to right.
pub const PATTERN: [Band; CELLS] = [
    N, M, F, N, M, N, F, N, M, N, //
    M, N, F, N, M, N, M, F, N, M, //
    N, F, N, M, N, M, F, N, M, N,
];

/// The cells span x 16..624: inside the 6-pixel glass edges, with room for the widest crown.
const LEFT: i32 = 16;
const SPAN: i32 = 608;
/// How far the salt may move a slot from its cell's centre, either way.
pub const JITTER: i32 = 7;

/// Where a cell's plant or filler stands: the x of its ground point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub band: Band,
    pub x: i32,
}

/// The thirty slots of one install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    slots: [Slot; CELLS],
}

impl Layout {
    pub fn new(salt: InstallSalt) -> Layout {
        let span = u64::try_from(2 * JITTER + 1).unwrap_or(1);
        Layout {
            slots: std::array::from_fn(|cell| {
                let index = u64::try_from(cell).unwrap_or(0);
                let shift = salt.below(Purpose::SlotJitter, index, span);
                Slot {
                    band: PATTERN[cell],
                    x: centre(cell) + i32::try_from(shift).unwrap_or(JITTER) - JITTER,
                }
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
}

/// A cell's centre before the salt moves it.
pub fn centre(cell: usize) -> i32 {
    let doubled = i32::try_from(2 * cell + 1).unwrap_or(0);
    let cells = i32::try_from(2 * CELLS).unwrap_or(1);
    LEFT + (SPAN * doubled + cells / 2) / cells
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

/// Which third of the strip an x falls in: 0, 1 or 2.
pub fn screen_third(x: i32) -> i32 {
    x * 3 / NATIVE_WIDTH.cast_signed()
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

    #[test]
    fn the_pattern_has_the_band_counts_and_never_repeats_a_band() {
        let count = |band| PATTERN.iter().filter(|&&b| b == band).count();
        assert_eq!((count(F), count(M), count(N)), (6, 10, 14));
        assert_eq!(band_cells(Band::Far), [2, 6, 12, 17, 21, 26]);
        assert!(PATTERN.windows(2).all(|pair| pair[0] != pair[1]));
        for t in 0..3 {
            let trees = band_cells(Band::Far)
                .iter()
                .filter(|&&c| third(c) == t)
                .count();
            assert_eq!(trees, 2, "third {t}");
        }
    }

    #[test]
    fn the_salt_moves_slots_within_their_cells_only() {
        let a = Layout::new(salts().next().unwrap());
        let b = Layout::new(salts().nth(1).unwrap());
        assert_ne!(a, b);
        for salt in salts() {
            let layout = Layout::new(salt);
            for (cell, slot) in layout.slots().iter().enumerate() {
                assert_eq!(slot.band, PATTERN[cell]);
                assert!((slot.x - centre(cell)).abs() <= JITTER);
            }
        }
    }

    #[test]
    fn slots_keep_their_gaps_for_every_salt() {
        for salt in salts() {
            let layout = Layout::new(salt);
            let xs: Vec<i32> = layout.slots().iter().map(|s| s.x).collect();
            assert!(xs.windows(2).all(|pair| pair[1] - pair[0] >= 6), "{xs:?}");
            for (band, gap) in [(Band::Far, 67), (Band::Mid, 26), (Band::Near, 26)] {
                let cells = band_cells(band);
                for pair in cells.windows(2) {
                    let apart = layout.slot(pair[1]).x - layout.slot(pair[0]).x;
                    assert!(apart >= gap, "{band:?} {pair:?}: {apart}");
                }
            }
        }
    }

    #[test]
    fn every_tree_cell_stays_inside_its_third_of_the_strip() {
        for &cell in band_cells(Band::Far) {
            for x in [
                centre(usize::from(cell)) - JITTER,
                centre(usize::from(cell)) + JITTER,
            ] {
                assert_eq!(
                    screen_third(x),
                    i32::try_from(third(cell)).unwrap(),
                    "{cell}"
                );
            }
        }
    }

    #[test]
    fn the_widest_mature_plant_of_each_band_stays_inside_the_glass() {
        // Measured from the art: each band's furthest reach left and right of the ground
        // point, over every mature look and bloom, at the jitter's extremes.
        let art = Art::embedded();
        let table = ArtTable::new(&art.catalog).unwrap();
        let glass = 6;
        let right_glass = NATIVE_WIDTH.cast_signed() - glass;
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
            for &cell in band_cells(band) {
                let centre = centre(usize::from(cell));
                assert!(centre - JITTER - reach.0 >= glass, "{band:?} {cell}");
                assert!(centre + JITTER + reach.1 < right_glass, "{band:?} {cell}");
            }
        }
    }
}
