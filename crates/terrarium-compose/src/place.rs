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
//! - **Trees.** Each third has four tree cells, standing in a pair, a grove or alone, so a
//!   third holds at most four mature trees, and a tree matures where it stands.
//! - **Ceiling.** At most [`CEILING`] plants are placed, 70% of the cells, which is more than
//!   truth ever grows. Were it not, a terrarium would grow taller and fuller rather than into
//!   more sprites: the waiting plants are placed mature first, and the rest are not drawn,
//!   which never adds health (I-7).
//! - **Weeds.** Truth's weeds stand in free cells, at most [`WEED_CELLS`] and never past the
//!   ceiling, and they count toward the window: real ground cover, not invented. Above the
//!   cap they merge, a cell at a time, into patches. Plants are placed first and never wait
//!   for a weed.
//! - **Floor.** Filler brings the occupied cells up to [`FLOOR`], 35%. Its form follows the
//!   scene's state and is never a thriving, pale or wilting plant: ground tufts in a living
//!   scene, the terrarium's own species dormant or dead in a dormant or dead one, else dead-stem
//!   litter in a dead one.
//!
//! Litter, fungus and the decorative tufts take no cell and are never counted. Monuments and
//! fossils keep spots of their own ([`assign_spots`]).

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use terrarium_truth::PresentationState;
use terrarium_truth::scene::{GrowthStage, PlantSpecies, PlantState, PlantView, SceneInput};

use crate::slots::{Band, CELLS, PATTERN, band_cells};
use crate::variants::ArtTable;

/// The fewest occupied cells: 35% of 108, rounded up.
pub const FLOOR: usize = 38;
/// The most plants placed: 70% of 108, rounded down.
pub const CEILING: usize = 75;
// The window is constraint 2's: the fewest cells at or above 35%, and the most at or below 70%.
const _: () = assert!(FLOOR * 100 >= 35 * CELLS && (FLOOR - 1) * 100 < 35 * CELLS);
const _: () = assert!(CEILING * 100 <= 70 * CELLS && (CEILING + 1) * 100 > 70 * CELLS);

/// Steps through the cells when choosing filler, so it spreads along the strip. Coprime with
/// [`CELLS`], so it reaches every cell.
const FILL_STRIDE: usize = 41;
/// The most cells weeds take; above it they merge into patches.
pub const WEED_CELLS: usize = 12;
/// Steps through the cells when choosing weed cells, from [`WEED_START`]: the first four near
/// each third and the middle, the rest between them, apart from the filler's walk. Coprime
/// with [`CELLS`].
const WEED_STRIDE: usize = 25;
const WEED_START: usize = 4;

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

/// What a weed cell shows: one weed, or a patch merged from several.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WeedForm {
    Single,
    Patch,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    pub plants: BTreeMap<u32, Placed>,
    pub weeds: BTreeMap<u8, WeedForm>,
    pub fill: BTreeMap<u8, FillForm>,
}

impl Placement {
    /// The cells holding a plant, weeds or filler.
    pub fn occupied(&self) -> usize {
        self.plants.len() + self.weeds.len() + self.fill.len()
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
    let mut taken = cells.map(|species| species.is_some());
    let weeds = weeds(&mut taken, plants.len(), input.weeds);
    let fill = filler(cells, taken, plants.len() + weeds.len(), input, bands);
    Placement {
        plants,
        weeds,
        fill,
    }
}

/// The weed cells for `count` weeds: as many cells as the cap and the ceiling allow, and above
/// the cap, patches in place of single weeds, one cell at a time in cell order.
fn weeds(taken: &mut [bool; CELLS], plants: usize, count: u8) -> BTreeMap<u8, WeedForm> {
    let count = usize::from(count);
    let cells = count.min(WEED_CELLS).min(CEILING.saturating_sub(plants));
    let chosen: BTreeSet<u8> = weed_walk()
        .map(usize::from)
        .filter(|&cell| !taken[cell])
        .take(cells)
        .filter_map(|cell| u8::try_from(cell).ok())
        .collect();
    let patches = cells.min(count - cells);
    chosen
        .into_iter()
        .enumerate()
        .map(|(i, cell)| {
            taken[usize::from(cell)] = true;
            let form = if i < patches {
                WeedForm::Patch
            } else {
                WeedForm::Single
            };
            (cell, form)
        })
        .collect()
}

/// The cells weeds take, in the order they take them: every cell once.
pub fn weed_walk() -> impl Iterator<Item = u8> {
    (0..CELLS).filter_map(|i| u8::try_from((WEED_START + i * WEED_STRIDE) % CELLS).ok())
}

/// Each id's spot among `n`: kept while the id stays, a new id preferring `id × stride mod n`
/// and probing on from there, as plants take cells. Ids past the free spots wait undrawn. A
/// pure function of the previous assignment and the ids, so the salt never touches it.
pub fn assign_spots(
    prev: &BTreeMap<u32, u8>,
    ids: impl IntoIterator<Item = u32>,
    n: usize,
) -> BTreeMap<u32, u8> {
    let ids: BTreeSet<u32> = ids.into_iter().collect();
    let mut used = vec![false; n];
    let mut spots = BTreeMap::new();
    for (&id, &spot) in prev {
        if ids.contains(&id) && usize::from(spot) < n {
            used[usize::from(spot)] = true;
            spots.insert(id, spot);
        }
    }
    for id in ids {
        if spots.contains_key(&id) {
            continue;
        }
        let start = usize::try_from(id).unwrap_or(0) % n.max(1) * stride(n) % n.max(1);
        let free = (0..n).map(|k| (start + k) % n).find(|&spot| !used[spot]);
        if let Some(spot) = free.and_then(|spot| u8::try_from(spot).ok()) {
            used[usize::from(spot)] = true;
            spots.insert(id, spot);
        }
    }
    spots
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

/// The filler that brings the occupied cells up to the floor. `cells` holds the species
/// standing in each cell, for the neighbour rule; `taken` also marks the weed cells.
fn filler(
    mut cells: [Option<PlantSpecies>; CELLS],
    taken: [bool; CELLS],
    occupied: usize,
    input: &SceneInput,
    bands: &Bands,
) -> BTreeMap<u8, FillForm> {
    let needed = FLOOR.saturating_sub(occupied);
    let chosen: BTreeSet<u8> = (0..CELLS)
        .map(|i| i * FILL_STRIDE % CELLS)
        .filter(|&cell| !taken[cell])
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
        // Each band's species in turn, at every stage: six trees of each stage, as many as
        // their band holds mature, and thirty herbs and thirty ground plants of each stage.
        // That is more mature and young plants than the ceiling, and seedlings beyond them.
        let bands: [(&[PlantSpecies], u16); 3] = [
            (&[PlantSpecies::Oak, PlantSpecies::Rowan], 6),
            (
                &[
                    PlantSpecies::Fern,
                    PlantSpecies::Foxglove,
                    PlantSpecies::Reed,
                ],
                30,
            ),
            (
                &[
                    PlantSpecies::MossMound,
                    PlantSpecies::Clover,
                    PlantSpecies::Thistle,
                ],
                30,
            ),
        ];
        let mut plants = Vec::new();
        for stage in GrowthStage::ALL.iter().copied() {
            for &(pool, count) in &bands {
                for i in 0..count {
                    let n = u16::try_from(plants.len()).unwrap();
                    let species = pool[usize::from(i) % pool.len()];
                    plants.push(view(u32::from(n), species, stage, n));
                }
            }
        }
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
        let oaks = (0..20)
            .map(|i| view(i, PlantSpecies::Oak, GrowthStage::Mature, 0))
            .collect();
        let placement = fresh(&scene(PresentationState::Thriving, oaks));
        let cells: Vec<u8> = placement.plants.values().map(|p| p.cell).collect();
        assert_eq!(cells.len(), 6, "{cells:?}");
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
        let seedlings: Vec<PlantView> = (0..100)
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
        assert_eq!(first.plants.len(), CEILING);
        let mut plants = seedlings;
        plants.extend((200..210).map(|i| view(i, PlantSpecies::Oak, GrowthStage::Mature, 0)));
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
    fn weeds_take_free_cells_up_to_the_cap_then_merge_into_patches() {
        let forms = |weeds: u8| {
            let mut input = scene(PresentationState::Struggling, Vec::new());
            input.weeds = weeds;
            fresh(&input)
                .weeds
                .values()
                .copied()
                .collect::<Vec<WeedForm>>()
        };
        let singles = |n| vec![WeedForm::Single; n];
        let cap = u8::try_from(WEED_CELLS).unwrap();
        assert_eq!(forms(0), singles(0));
        assert_eq!(forms(3), singles(3));
        assert_eq!(forms(cap), singles(WEED_CELLS));
        // Each weed past the cap merges one more cell into a patch, in cell order.
        let mut two = vec![WeedForm::Patch; 2];
        two.extend(singles(WEED_CELLS - 2));
        assert_eq!(forms(cap + 2), two);
        assert_eq!(forms(2 * cap), [WeedForm::Patch; WEED_CELLS]);
        assert_eq!(forms(200), [WeedForm::Patch; WEED_CELLS]);
    }

    #[test]
    fn weeds_count_toward_the_floor_and_never_pass_the_ceiling() {
        let mut input = scene(PresentationState::Struggling, Vec::new());
        input.weeds = 3;
        let placement = fresh(&input);
        assert_eq!(placement.occupied(), FLOOR);
        assert_eq!(placement.fill.len(), FLOOR - 3);
        assert!(
            placement
                .fill
                .keys()
                .all(|cell| !placement.weeds.contains_key(cell))
        );
        // A crowd at the ceiling leaves no room for weeds; a placed plant never waits for one.
        let crowd: Vec<PlantView> = (0..120_u16)
            .map(|i| {
                let species = PlantSpecies::ALL[usize::from(i) % PlantSpecies::ALL.len()];
                view(u32::from(i), species, GrowthStage::Mature, i)
            })
            .collect();
        let mut full = scene(PresentationState::Thriving, crowd.clone());
        let bare = fresh(&full);
        full.weeds = 24;
        let weedy = fresh(&full);
        assert_eq!(weedy.plants, bare.plants);
        assert!(weedy.occupied() <= CEILING);
        assert_eq!(weedy.weeds.len(), CEILING - weedy.plants.len());
    }

    #[test]
    fn spots_are_kept_by_id_and_new_ids_probe_from_their_own() {
        let first = assign_spots(&BTreeMap::new(), [4, 9], 3);
        assert_eq!(first.len(), 2);
        assert_ne!(first[&4], first[&9]);
        // 9 leaves, 4 stays where it was, and two newcomers take what is free.
        let next = assign_spots(&first, [4, 1, 2], 3);
        assert_eq!(next[&4], first[&4]);
        let spots: BTreeSet<u8> = next.values().copied().collect();
        assert_eq!(spots.len(), 3);
        // Past the spots, the rest wait.
        let crowded = assign_spots(&next, [4, 1, 2, 7], 3);
        assert_eq!(crowded.len(), 3);
        assert!(!crowded.contains_key(&7));
        // The same ids from nothing land in the same spots: the order they come in is not read.
        assert_eq!(
            assign_spots(&BTreeMap::new(), [2, 1], 6),
            assign_spots(&BTreeMap::new(), [1, 2], 6)
        );
    }

    #[test]
    fn every_plant_truth_grows_has_a_cell_of_its_band() {
        use terrarium_truth::habitat::plant_capacity;
        use terrarium_truth::vegetation::Layer;
        let most = plant_capacity(u64::MAX);
        assert!(CEILING >= usize::try_from(most).unwrap());
        // At the most, each of truth's layers has as many slots as its band has cells or fewer.
        let slots = |layer| {
            (0..u16::try_from(most).unwrap())
                .filter(|&slot| Layer::of_slot(slot) == layer)
                .count()
        };
        assert_eq!(slots(Layer::Far), band_cells(Band::Far).len());
        assert!(slots(Layer::Mid) <= band_cells(Band::Mid).len());
        assert!(slots(Layer::Near) <= band_cells(Band::Near).len());
    }

    #[test]
    fn strides_are_coprime_and_spread() {
        for n in 1..=CELLS {
            assert_eq!(gcd(stride(n), n), 1, "{n}");
        }
        assert_eq!([stride(6), stride(10), stride(14)], [5, 7, 9]);
        let bands = Band::ALL.map(|band| stride(band_cells(band).len()));
        assert_eq!(bands, [7, 29, 30]);
        assert_eq!(gcd(FILL_STRIDE, CELLS), 1);
        assert_eq!(gcd(WEED_STRIDE, CELLS), 1);
    }
}
