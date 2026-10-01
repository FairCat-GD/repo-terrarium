//! Placement: which plant stands in which cell, and which cells hold filler (I-7).
//!
//! It is a pure function of the previous placement and the input, over cell indices only. It
//! reads no tween, no clock and no salt, so the same history depicts the same plants on every
//! install, and only their positions differ (I-2). The rules:
//!
//! - **Bands.** A plant stands in a cell of its species' band.
//! - **Kept.** A plant stays in its cell for as long as truth keeps its id and species. Nothing
//!   ever pushes it out, so a plant never jumps.
//! - **No neighbours alike.** Neighbouring cells of a band never hold one species. Placement
//!   checks it, and a removal can never break it.
//! - **Hero plus company.** Each third has two tree cells, and they are band neighbours, so a
//!   third holds at most two mature trees and never two of one species, and a tree matures
//!   where it stands. The two are not ranked by size.
//! - **Ceiling.** At most [`CEILING`] plants are placed, 70% of the cells. Above it a terrarium
//!   grows taller and fuller, not into more sprites: the waiting plants are placed mature
//!   first, and the rest are not drawn, which never adds health (I-7).
//! - **Floor.** Filler brings the occupied cells up to [`FLOOR`], 35%. Its form follows the
//!   scene's state and is never a thriving, pale or wilting plant: ground tufts in a living
//!   scene, the terrarium's own species dormant or dead in a dormant or dead one, else dead-stem
//!   litter in a dead one.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use terrarium_truth::PresentationState;
use terrarium_truth::scene::{GrowthStage, PlantSpecies, PlantState, PlantView, SceneInput};

use crate::slots::{Band, CELLS, PATTERN, band_cells};
use crate::variants::ArtTable;

/// The fewest occupied cells: 35% of 30, rounded up.
pub const FLOOR: usize = 11;
/// The most plants placed: 70% of 30.
pub const CEILING: usize = 21;
// The window is constraint 2's: the fewest cells at or above 35%, and the most at or below 70%.
const _: () = assert!(FLOOR * 100 >= 35 * CELLS && (FLOOR - 1) * 100 < 35 * CELLS);
const _: () = assert!(CEILING * 100 <= 70 * CELLS && (CEILING + 1) * 100 > 70 * CELLS);

/// Steps through the cells when choosing filler, so it spreads along the strip. Coprime with
/// [`CELLS`], so it reaches every cell.
const FILL_STRIDE: usize = 11;

/// Each species' band, read once from the art.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bands(BTreeMap<PlantSpecies, Band>);

impl Bands {
    /// # Panics
    /// If a species stands outside the plant layers, which `ArtTable::new` refuses.
    pub fn new(table: &ArtTable) -> Bands {
        Bands(
            PlantSpecies::ALL
                .iter()
                .map(|&species| {
                    let layer = table.plant_layer(species);
                    let band = Band::from_layer(layer)
                        .unwrap_or_else(|| panic!("{species:?} stands in {layer:?}"));
                    (species, band)
                })
                .collect(),
        )
    }

    pub fn of(&self, species: PlantSpecies) -> Band {
        self.0[&species]
    }
}

/// A placed plant: its cell, and the plant as the scene that placed it gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub cell: u8,
    pub view: PlantView,
}

/// What a filler cell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FillForm {
    /// A ground tuft: the lower vegetation of a living scene.
    Tuft,
    /// Dead-stem litter, for a dead scene with no species of the cell's band.
    DeadStems,
    /// One of the terrarium's own species at [`FillForm::STAGE`], dormant or dead.
    Plant {
        species: PlantSpecies,
        state: PlantState,
    },
}

impl FillForm {
    /// Filler plants are young: lower forms, never a tree that could take the stage.
    pub const STAGE: GrowthStage = GrowthStage::Young;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    pub plants: BTreeMap<u32, Placed>,
    pub fill: BTreeMap<u8, FillForm>,
}

impl Placement {
    /// The cells holding a plant or filler.
    pub fn occupied(&self) -> usize {
        self.plants.len() + self.fill.len()
    }

    /// The species standing in each cell, plants and filler plants alike.
    pub fn species_by_cell(&self) -> [Option<PlantSpecies>; CELLS] {
        let mut cells = [None; CELLS];
        for placed in self.plants.values() {
            cells[usize::from(placed.cell)] = Some(placed.view.species);
        }
        for (&cell, form) in &self.fill {
            if let FillForm::Plant { species, .. } = *form {
                cells[usize::from(cell)] = Some(species);
            }
        }
        cells
    }
}

/// The placement for `input`, keeping every plant `prev` placed whose id and species truth
/// still has.
pub fn place(prev: &Placement, input: &SceneInput, bands: &Bands) -> Placement {
    let views: BTreeMap<u32, &PlantView> = input.plants.iter().map(|p| (p.id, p)).collect();
    let mut cells: [Option<PlantSpecies>; CELLS] = [None; CELLS];
    let mut plants = BTreeMap::new();
    for (&id, placed) in &prev.plants {
        if let Some(&&view) = views.get(&id)
            && view.species == placed.view.species
        {
            cells[usize::from(placed.cell)] = Some(view.species);
            plants.insert(id, Placed { view, ..*placed });
        }
    }
    let mut waiting: Vec<&PlantView> = views
        .values()
        .filter(|view| !plants.contains_key(&view.id))
        .copied()
        .collect();
    waiting.sort_by_key(|view| (Reverse(view.stage), view.order, view.id));
    for view in waiting {
        if plants.len() >= CEILING {
            break;
        }
        if let Some(cell) = free_cell(&cells, bands.of(view.species), view) {
            cells[usize::from(cell)] = Some(view.species);
            plants.insert(view.id, Placed { cell, view: *view });
        }
    }
    let fill = filler(cells, plants.len(), input, bands);
    Placement { plants, fill }
}

/// The first acceptable cell of the band, probing from the plant's preferred one.
fn free_cell(cells: &[Option<PlantSpecies>; CELLS], band: Band, view: &PlantView) -> Option<u8> {
    let list = band_cells(band);
    let n = list.len();
    let start = usize::from(view.order) * stride(n) % n.max(1);
    (0..n).map(|k| (start + k) % n).find_map(|i| {
        let cell = list[i];
        let free = cells[usize::from(cell)].is_none();
        (free && !beside(cells, list, i, view.species)).then_some(cell)
    })
}

/// Whether a band neighbour of the band's `i`-th cell holds `species`.
fn beside(
    cells: &[Option<PlantSpecies>; CELLS],
    list: &[u8],
    i: usize,
    species: PlantSpecies,
) -> bool {
    [i.checked_sub(1), Some(i + 1)]
        .into_iter()
        .flatten()
        .filter_map(|j| list.get(j))
        .any(|&cell| cells[usize::from(cell)] == Some(species))
}

/// Spreads consecutive `order`s across a band: the step nearest 0.618 of its length that is
/// coprime with it, so the probe still reaches every cell.
fn stride(n: usize) -> usize {
    let golden = (n * 618 + 500) / 1000;
    (golden.max(1)..=n.max(1))
        .find(|&k| gcd(k, n) == 1)
        .unwrap_or(1)
}

const fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The filler that brings the occupied cells up to the floor.
fn filler(
    mut cells: [Option<PlantSpecies>; CELLS],
    placed: usize,
    input: &SceneInput,
    bands: &Bands,
) -> BTreeMap<u8, FillForm> {
    let needed = FLOOR.saturating_sub(placed);
    let chosen: BTreeSet<u8> = (0..CELLS)
        .map(|i| i * FILL_STRIDE % CELLS)
        .filter(|&cell| cells[cell].is_none())
        .take(needed)
        .filter_map(|cell| u8::try_from(cell).ok())
        .collect();
    let mut pools: BTreeMap<Band, BTreeSet<PlantSpecies>> = BTreeMap::new();
    for view in &input.plants {
        pools
            .entry(bands.of(view.species))
            .or_default()
            .insert(view.species);
    }
    let mut fill = BTreeMap::new();
    for cell in chosen {
        let band = PATTERN[usize::from(cell)];
        let list = band_cells(band);
        let i = list.iter().position(|&c| c == cell).unwrap_or(0);
        let own = |state| {
            pools
                .get(&band)
                .into_iter()
                .flatten()
                .find(|&&species| !beside(&cells, list, i, species))
                .map(|&species| FillForm::Plant { species, state })
        };
        let form = match input.presentation.state {
            PresentationState::Dormant => own(PlantState::Dormant).unwrap_or(FillForm::Tuft),
            PresentationState::Dead => own(PlantState::Dead).unwrap_or(FillForm::DeadStems),
            PresentationState::Thriving
            | PresentationState::Struggling
            | PresentationState::Mature
            | PresentationState::Recovering => FillForm::Tuft,
        };
        if let FillForm::Plant { species, .. } = form {
            cells[usize::from(cell)] = Some(species);
        }
        fill.insert(cell, form);
    }
    fill
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Art;
    use crate::slots::third;

    fn bands() -> Bands {
        Bands::new(&ArtTable::new(&Art::embedded().catalog).unwrap())
    }

    fn view(id: u32, species: PlantSpecies, stage: GrowthStage, order: u16) -> PlantView {
        PlantView {
            id,
            species,
            stage,
            state: PlantState::Thriving,
            bloom: false,
            scorched: false,
            order,
        }
    }

    fn scene(state: PresentationState, plants: Vec<PlantView>) -> SceneInput {
        crate::preset::bare(state, plants)
    }

    fn fresh(input: &SceneInput) -> Placement {
        place(&Placement::default(), input, &bands())
    }

    #[test]
    fn an_empty_scene_is_filled_to_the_floor_with_tufts() {
        let placement = fresh(&scene(PresentationState::Thriving, Vec::new()));
        assert_eq!(placement.occupied(), FLOOR);
        assert!(placement.fill.values().all(|&f| f == FillForm::Tuft));
        // Spread along the strip: every third has some.
        for t in 0..3 {
            assert!(placement.fill.keys().any(|&c| third(c) == t));
        }
    }

    #[test]
    fn above_the_ceiling_the_mature_plants_are_placed_first() {
        let pool = [
            PlantSpecies::Oak,
            PlantSpecies::Rowan,
            PlantSpecies::Fern,
            PlantSpecies::Foxglove,
            PlantSpecies::Reed,
            PlantSpecies::MossMound,
            PlantSpecies::Clover,
            PlantSpecies::Thistle,
        ];
        // 14 seedlings, 13 young and 13 mature plants over the pool: more than the ceiling.
        let plants: Vec<PlantView> = (0..40_u16)
            .map(|i| {
                let stage = GrowthStage::ALL[usize::from(i % 3)];
                view(u32::from(i), pool[usize::from(i) % 8], stage, i)
            })
            .collect();
        let placement = fresh(&scene(PresentationState::Thriving, plants.clone()));
        assert_eq!(placement.plants.len(), CEILING);
        assert!(placement.fill.is_empty());
        for plant in &plants {
            let placed = placement.plants.contains_key(&plant.id);
            match plant.stage {
                GrowthStage::Mature => assert!(placed, "{plant:?}"),
                GrowthStage::Seedling => assert!(!placed, "{plant:?}"),
                GrowthStage::Young => {}
            }
        }
    }

    #[test]
    fn a_monoculture_never_stands_side_by_side() {
        let oaks = (0..10)
            .map(|i| view(i, PlantSpecies::Oak, GrowthStage::Mature, 0))
            .collect();
        let placement = fresh(&scene(PresentationState::Thriving, oaks));
        let cells: Vec<u8> = placement.plants.values().map(|p| p.cell).collect();
        assert_eq!(cells.len(), 3, "{cells:?}");
        let far = band_cells(Band::Far);
        for pair in far.windows(2) {
            assert!(
                !(cells.contains(&pair[0]) && cells.contains(&pair[1])),
                "{cells:?}"
            );
        }
        assert_eq!(placement.occupied(), FLOOR);
    }

    #[test]
    fn a_placed_plant_keeps_its_cell_as_it_matures_and_as_others_come() {
        let young = view(5, PlantSpecies::Rowan, GrowthStage::Young, 3);
        let first = fresh(&scene(PresentationState::Thriving, vec![young]));
        let cell = first.plants[&5].cell;
        // Mature now, and twenty-five newcomers with lower orders compete for cells.
        let mut plants: Vec<PlantView> = (100..125)
            .map(|id| view(id, PlantSpecies::Oak, GrowthStage::Mature, 0))
            .collect();
        plants.push(PlantView {
            stage: GrowthStage::Mature,
            ..young
        });
        let next = place(
            &first,
            &scene(PresentationState::Thriving, plants),
            &bands(),
        );
        assert_eq!(next.plants[&5].cell, cell);
    }

    #[test]
    fn the_ceiling_never_pushes_out_a_placed_plant() {
        let pool = [
            PlantSpecies::Clover,
            PlantSpecies::Thistle,
            PlantSpecies::Fern,
            PlantSpecies::Reed,
        ];
        let seedlings: Vec<PlantView> = (0..30)
            .map(|i| {
                view(
                    i,
                    pool[usize::try_from(i).unwrap() % 4],
                    GrowthStage::Seedling,
                    0,
                )
            })
            .collect();
        let first = fresh(&scene(PresentationState::Thriving, seedlings.clone()));
        let mut plants = seedlings;
        plants.extend((50..60).map(|i| view(i, PlantSpecies::Oak, GrowthStage::Mature, 0)));
        let next = place(
            &first,
            &scene(PresentationState::Thriving, plants),
            &bands(),
        );
        for (id, placed) in &first.plants {
            assert_eq!(next.plants.get(id), Some(placed));
        }
        assert!(next.plants.len() <= CEILING);
    }

    #[test]
    fn a_species_change_is_a_new_plant() {
        let fern = view(1, PlantSpecies::Fern, GrowthStage::Mature, 0);
        let first = fresh(&scene(PresentationState::Thriving, vec![fern]));
        let reed = PlantView {
            species: PlantSpecies::Reed,
            ..fern
        };
        let next = place(
            &first,
            &scene(PresentationState::Thriving, vec![reed]),
            &bands(),
        );
        assert_eq!(next.plants[&1].view.species, PlantSpecies::Reed);
        let oak = PlantView {
            species: PlantSpecies::Oak,
            ..fern
        };
        let next = place(
            &first,
            &scene(PresentationState::Thriving, vec![oak]),
            &bands(),
        );
        assert_eq!(PATTERN[usize::from(next.plants[&1].cell)], Band::Far);
    }

    #[test]
    fn a_removed_id_that_returns_takes_its_preferred_cell_again() {
        let fern = view(1, PlantSpecies::Fern, GrowthStage::Mature, 4);
        let first = fresh(&scene(PresentationState::Thriving, vec![fern]));
        let gone = place(
            &first,
            &scene(PresentationState::Thriving, Vec::new()),
            &bands(),
        );
        assert!(gone.plants.is_empty());
        let back = place(
            &gone,
            &scene(PresentationState::Thriving, vec![fern]),
            &bands(),
        );
        assert_eq!(back.plants[&1], first.plants[&1]);
    }

    #[test]
    fn dormant_and_dead_scenes_fill_with_their_own_species_never_side_by_side() {
        let plants = vec![
            view(1, PlantSpecies::Oak, GrowthStage::Mature, 0),
            view(2, PlantSpecies::Fern, GrowthStage::Mature, 0),
        ];
        for (state, form_state) in [
            (PresentationState::Dormant, PlantState::Dormant),
            (PresentationState::Dead, PlantState::Dead),
        ] {
            let placement = fresh(&scene(state, plants.clone()));
            assert_eq!(placement.occupied(), FLOOR);
            for (&cell, &form) in &placement.fill {
                match (PATTERN[usize::from(cell)], form) {
                    (Band::Far, FillForm::Plant { species, state }) => {
                        assert_eq!((species, state), (PlantSpecies::Oak, form_state));
                    }
                    (Band::Mid, FillForm::Plant { species, state }) => {
                        assert_eq!((species, state), (PlantSpecies::Fern, form_state));
                    }
                    (Band::Near, form) => {
                        let expected = if state == PresentationState::Dead {
                            FillForm::DeadStems
                        } else {
                            FillForm::Tuft
                        };
                        assert_eq!(form, expected);
                    }
                    (band, form) => {
                        // A cell whose band neighbour already holds the species falls back.
                        assert!(
                            matches!(form, FillForm::Tuft | FillForm::DeadStems),
                            "{band:?}"
                        );
                    }
                }
            }
            let species = placement.species_by_cell();
            for band in Band::ALL {
                for pair in band_cells(band).windows(2) {
                    let [a, b] = [pair[0], pair[1]].map(|c| species[usize::from(c)]);
                    assert!(a.is_none() || a != b, "{state:?} {pair:?}");
                }
            }
        }
    }

    #[test]
    fn strides_are_coprime_and_spread() {
        for n in 1..=30 {
            assert_eq!(gcd(stride(n), n), 1, "{n}");
        }
        assert_eq!([stride(6), stride(10), stride(14)], [5, 7, 9]);
        assert_eq!(gcd(FILL_STRIDE, CELLS), 1);
    }
}
