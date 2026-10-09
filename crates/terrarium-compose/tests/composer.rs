//! The composer's constraints over generated histories: occupancy (constraint 2), silhouette
//! rhythm (constraint 3), palette discipline (constraint 4, on the draws), one message
//! (constraint 6), transitions only (constraint 7, I-15), the healthy subset (I-7) and salt
//! independence (I-2). A second property renders settled frames by day and by night for the
//! value structure (constraint 1) and palette discipline on the pixels (constraint 4).
//!
//! Each case is a history of three scenes, A, B and C, each with its overlay, played into a
//! composer at times chosen to land on tween boundaries, either side of them, and past a step
//! of the label's cycle. B and C are often edits of the scene before, so ids carry over and
//! change state, stage, bloom or species, and the world's weather, cover, glass, monuments,
//! fossils and the marks in the ground change by small steps. Scenes carry events now and then,
//! new instances and ones the scene before carried, with what truth would bring with them or
//! without it. The rules the
//! checks hold the composer to are written out here, never taken from the crate, so a broken
//! rule cannot agree with itself.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use terrarium_compose::catalog::{Art, Blend, Flags, Layer};
use terrarium_compose::clock::LocalClock;
use terrarium_compose::composer::{Composer, OverlayInput};
use terrarium_compose::compositor::composite;
use terrarium_compose::frame::{
    Draw, Frame, NATIVE_HEIGHT, NATIVE_WIDTH, PLANT_GROUND_Y, Rgba, Viewport,
};
use terrarium_compose::order::{KeyedDraw, Subject};
use terrarium_compose::palette::Slot;
use terrarium_compose::place::{CEILING, FLOOR};
use terrarium_compose::salt::InstallSalt;
use terrarium_compose::sky::{LocalDate, MoonPhase, moon_phase};
use terrarium_compose::slots::{Band, PATTERN, band_cells, screen_third};
use terrarium_compose::tween::Millis;
use terrarium_compose::tween::durations::{
    APPEAR_MS, LABEL_CYCLE_MS, LABEL_FADE_MS, RESTYLE_MS, SHORTEST_MS, SKY_MS,
};
use terrarium_compose::variants::{ArtTable, art_key};
use terrarium_compose::world::Scenery;
use terrarium_compose::{preset, synthetic};
use terrarium_truth::scene::{
    Energy, EventKind, FossilForm, FossilView, GroundMark, GroundMarkView, GrowthStage, LifeStage,
    Light, MonumentAge, MonumentView, PlantSpecies, PlantState, PlantView, ResidentSpecies,
    ResidentView, SceneInput, Side, Strength, Weather, Wind,
};
use terrarium_truth::{PresentationState, Tick};

fn config(cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases,
        rng_seed: RngSeed::Fixed(0x7465_7272_6172_6975),
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

fn art() -> &'static Art<'static> {
    Art::embedded()
}

fn table() -> &'static ArtTable {
    static TABLE: std::sync::OnceLock<ArtTable> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| ArtTable::new(&art().catalog).expect("the embedded art is complete"))
}

// ---- The generator -------------------------------------------------------------------------
//
// Scenes are decoded from plain integers, which proptest generates and shrinks cheaply, rather
// than built from nested strategies.

use PlantSpecies::{Birch, Clover, Fern, Foxglove, MossMound, Oak, Reed, Rowan, Thistle};

/// A seeded pool as truth would pick it: two far, three mid, three near.
const POOL: [PlantSpecies; 8] = [Oak, Rowan, Fern, Foxglove, Reed, MossMound, Clover, Thistle];
const FAR: [PlantSpecies; 3] = [Oak, Birch, Rowan];

/// Picks `value % len` from a list.
fn pick<T: Copy>(list: &[T], value: u64) -> T {
    list[usize::try_from(value % u64::try_from(list.len()).unwrap()).unwrap()]
}

/// `n` values of `bits` from bit `shift`, as a field: `(bits >> shift) % n`.
fn field(bits: u64, shift: u32, n: u64) -> u64 {
    (bits >> shift) % n
}

fn small(bits: u64, shift: u32, n: u64) -> u8 {
    u8::try_from(field(bits, shift, n)).unwrap()
}

/// The species a scene draws from: the pool, one to three of all twelve (monocultures
/// included), the trees alone, or all twelve.
fn species(kind: u8, mask: u16) -> Vec<PlantSpecies> {
    let some = |all: &[PlantSpecies], most: usize| -> Vec<PlantSpecies> {
        let mut chosen: Vec<PlantSpecies> = all
            .iter()
            .enumerate()
            .filter(|(i, _)| (mask >> i) & 1 == 1)
            .map(|(_, &s)| s)
            .take(most)
            .collect();
        if chosen.is_empty() {
            chosen.push(pick(all, u64::from(mask)));
        }
        chosen
    };
    match kind % 9 {
        0..=2 => POOL.to_vec(),
        3..=5 => some(PlantSpecies::ALL, 3),
        6 | 7 => some(&FAR, 3),
        _ => PlantSpecies::ALL.to_vec(),
    }
}

/// A plant from 64 bits: an id in 0..48, so scenes share ids; an `order` among a few small
/// values, so they repeat, or any value; now and then the one lightning strikes.
fn plant(species: &[PlantSpecies], bits: u64) -> PlantView {
    PlantView {
        id: u32::try_from(field(bits, 0, 48)).unwrap(),
        species: pick(species, field(bits, 8, 64)),
        stage: pick(GrowthStage::ALL, field(bits, 16, 3)),
        state: pick(PlantState::ALL, field(bits, 20, 5)),
        bloom: field(bits, 24, 2) == 1,
        scorched: field(bits, 50, 6) == 0,
        order: if field(bits, 25, 2) == 0 {
            u16::try_from(field(bits, 26, 8)).unwrap()
        } else {
            u16::try_from(field(bits, 32, 1 << 16)).unwrap()
        },
    }
}

/// Keeps the first of each id, as `SceneInput::validate` requires.
fn unique<T>(mut items: Vec<T>, id: impl Fn(&T) -> u32) -> Vec<T> {
    let mut seen = BTreeSet::new();
    items.retain(|item| seen.insert(id(item)));
    items
}

fn scene_with(state: PresentationState, plants: Vec<PlantView>) -> SceneInput {
    preset::bare(state, unique(plants, |p| p.id))
}

/// Light that is neutral half the time, so the pixels are checked under every overlay and
/// without one.
const LIGHTS: [Light; 4] = [Light::Neutral, Light::Neutral, Light::Dusk, Light::Sunburst];
/// Glass that is clean half the time.
const LEVELS: [u8; 6] = [0, 0, 0, 1, 2, 3];

fn monument(bits: u64) -> MonumentView {
    MonumentView {
        id: u32::try_from(field(bits, 0, 6)).unwrap(),
        age: pick(MonumentAge::ALL, field(bits, 3, 3)),
        marks: small(bits, 5, 4),
    }
}

/// Marks in the ground from 64 bits, now and then: one to three, each a crack, fresh soil or a
/// sinkhole naming one of `fossils` or one gone, with ids from a few, so scenes share them.
fn ground_marks(bits: u64, fossils: &[FossilView]) -> Vec<GroundMarkView> {
    if field(bits, 0, 3) != 0 {
        return Vec::new();
    }
    let marks = (0..=field(bits, 2, 3))
        .map(|i| {
            ground_mark(
                bits.rotate_right(u32::try_from(i * 13 + 4).unwrap()),
                fossils,
            )
        })
        .collect();
    unique(marks, |m| m.id)
}

fn ground_mark(bits: u64, fossils: &[FossilView]) -> GroundMarkView {
    let kind = pick(GroundMark::ALL, field(bits, 0, 3));
    let fossil = (kind == GroundMark::Sinkhole).then(|| {
        if fossils.is_empty() || field(bits, 2, 3) == 0 {
            20 + u32::try_from(field(bits, 4, 4)).unwrap()
        } else {
            pick(fossils, field(bits, 8, 8)).id
        }
    });
    GroundMarkView {
        id: u32::try_from(field(bits, 16, 8)).unwrap(),
        kind,
        fossil,
    }
}

fn fossil(bits: u64) -> FossilView {
    FossilView {
        id: u32::try_from(field(bits, 0, 10)).unwrap(),
        form: pick(FossilForm::ALL, field(bits, 4, 3)),
    }
}

/// A resident from 64 bits: an id in 0..12, so scenes share ids; any species and stage, a
/// pupa only where the species has one.
fn resident(bits: u64) -> ResidentView {
    let species = pick(ResidentSpecies::ALL, field(bits, 4, 8));
    let mut stage = pick(LifeStage::ALL, field(bits, 8, 5));
    if stage == LifeStage::Pupa && !species.has_pupa() {
        stage = LifeStage::Juvenile;
    }
    ResidentView {
        id: u32::try_from(field(bits, 0, 12)).unwrap(),
        species,
        stage,
        energy: pick(Energy::ALL, field(bits, 12, 3)),
    }
}

/// The stage after a resident's, as truth would step it on.
fn grown(view: ResidentView) -> ResidentView {
    let stage = match view.stage {
        LifeStage::Egg => LifeStage::Juvenile,
        LifeStage::Juvenile if view.species.has_pupa() => LifeStage::Pupa,
        LifeStage::Juvenile | LifeStage::Pupa => LifeStage::Adult,
        LifeStage::Adult | LifeStage::Dying => LifeStage::Dying,
    };
    ResidentView { stage, ..view }
}

/// The world of a scene from two words: its sky, cover, glass, monuments, fossils and
/// residents.
fn dress(scene: &mut SceneInput, world: u64, things: u64) {
    scene.weather = pick(Weather::ALL, field(world, 0, 4));
    scene.wind = pick(Wind::ALL, field(world, 32, 3));
    let residents = usize::try_from(field(world, 36, 8)).unwrap();
    scene.residents = unique(
        (0..residents)
            .map(|i| resident(things.rotate_left(u32::try_from(i * 11 + 5).unwrap()) ^ world))
            .collect(),
        |r| r.id,
    );
    scene.light = pick(&LIGHTS, field(world, 2, 4));
    scene.weeds = small(world, 4, 13);
    scene.litter = small(world, 8, 13);
    scene.fungus = small(world, 12, 10);
    scene.condensation = pick(&LEVELS, field(world, 16, 6));
    scene.dust = pick(&LEVELS, field(world, 20, 6));
    let monuments = usize::from(small(world, 24, 5));
    let fossils = usize::from(small(world, 28, 8));
    scene.monuments = unique(
        (0..monuments)
            .map(|i| monument(things.rotate_right(u32::try_from(i * 8).unwrap())))
            .collect(),
        |m| m.id,
    );
    scene.fossils = unique(
        (0..fossils)
            .map(|i| fossil(things.rotate_left(u32::try_from(i * 7 + 3).unwrap())))
            .collect(),
        |f| f.id,
    );
    scene.ground_marks = ground_marks(things.rotate_left(29) ^ world, &scene.fossils);
    cue(scene, world >> 40);
}

/// An event or none from a word: any kind, an instance from a few ticks, so scenes carry the
/// same one again, any strength and side. Mostly the scene carries what truth would bring with
/// it, in the sky and on the ground; now and then the scene stays as it was, which the composer
/// must take too.
fn cue(scene: &mut SceneInput, bits: u64) {
    if field(bits, 0, 5) >= 2 {
        scene.event = None;
        return;
    }
    let before = scene.clone();
    let tick = Tick::new(u32::try_from(field(bits, 3, 3)).unwrap());
    synthetic::set_event(scene, pick(EventKind::ALL, field(bits, 5, 11)), tick);
    if field(bits, 9, 4) == 0 {
        *scene = SceneInput {
            event: scene.event,
            ..before
        };
    }
    if let Some(event) = &mut scene.event {
        event.strength = pick(Strength::ALL, field(bits, 11, 3));
        event.side = pick(Side::ALL, field(bits, 13, 2));
    }
}

/// The world a little changed: the weather and light now and then, the cover by a few, the
/// glass by a level, a monument aged, marked, lost or found, a fossil lost or found.
fn nudge(scene: &mut SceneInput, bits: u64) {
    let step = |value: u8, shift: u32| -> u8 {
        let delta = small(bits, shift, 7);
        (value + delta).saturating_sub(3).min(12)
    };
    if field(bits, 0, 3) == 0 {
        scene.weather = pick(Weather::ALL, field(bits, 2, 4));
    }
    if field(bits, 4, 4) == 0 {
        scene.light = pick(&LIGHTS, field(bits, 6, 4));
    }
    scene.weeds = step(scene.weeds, 8);
    scene.litter = step(scene.litter, 11);
    scene.fungus = step(scene.fungus, 14);
    if field(bits, 17, 3) == 0 {
        scene.condensation = pick(&LEVELS, field(bits, 19, 6));
    }
    if field(bits, 22, 3) == 0 {
        scene.dust = pick(&LEVELS, field(bits, 24, 6));
    }
    for (i, monument) in scene.monuments.iter_mut().enumerate() {
        let edit = field(bits, 27 + u32::try_from(i).unwrap() * 3, 8);
        match edit {
            0 => monument.age = older(monument.age),
            1 => monument.marks = (monument.marks + 1).min(3),
            _ => {}
        }
    }
    if field(bits, 40, 6) == 0 {
        scene.monuments.pop();
    }
    if field(bits, 43, 5) == 0 {
        scene.monuments.push(monument(bits >> 46));
        scene.monuments = unique(std::mem::take(&mut scene.monuments), |m| m.id);
    }
    if field(bits, 54, 5) == 0 {
        scene.fossils.pop();
    }
    if field(bits, 57, 4) == 0 {
        scene.fossils.push(fossil(bits >> 59));
        scene.fossils = unique(std::mem::take(&mut scene.fossils), |f| f.id);
    }
    // A mark washed away or left, now and then.
    let marks = bits.rotate_left(29);
    if field(marks, 0, 6) == 0 {
        scene.ground_marks.pop();
    }
    if field(marks, 3, 6) == 0 {
        scene
            .ground_marks
            .push(ground_mark(marks >> 6, &scene.fossils));
        scene.ground_marks = unique(std::mem::take(&mut scene.ground_marks), |m| m.id);
    }
    // Residents grow, tire or liven, now and then leave or arrive; the wind changes.
    for (i, resident) in scene.residents.iter_mut().enumerate() {
        match field(bits.rotate_left(u32::try_from(i * 5).unwrap()), 0, 6) {
            0 => *resident = grown(*resident),
            1 => resident.energy = pick(Energy::ALL, field(bits, 10, 3)),
            _ => {}
        }
    }
    if field(bits, 61, 3) == 0 {
        scene.residents.pop();
    }
    if field(bits, 1, 5) == 0 {
        scene.residents.push(resident(bits.rotate_right(23)));
        scene.residents = unique(std::mem::take(&mut scene.residents), |r| r.id);
    }
    if field(bits, 3, 4) == 0 {
        scene.wind = pick(Wind::ALL, field(bits, 7, 3));
    }
}

/// A monument a tier older, or as old as it gets.
fn older(age: MonumentAge) -> MonumentAge {
    match age {
        MonumentAge::Young => MonumentAge::Lichened,
        MonumentAge::Lichened | MonumentAge::Ancient => MonumentAge::Ancient,
    }
}

/// Lines the label could be given, long and non-ASCII among them.
const LINES: [&str; 6] = [
    "repo-terrarium",
    "main - clear",
    "feature/a-very-long-branch-name",
    "caf\u{e9} - storm",
    "settling 9 days",
    "",
];

/// Local times well clear of the hours when day and night take over, so a history never
/// crosses one: noon, midnight, mid-afternoon and the small hours.
const TIMES: [u32; 4] = [12 * HOUR, 0, 15 * HOUR, 3 * HOUR];
const HOUR: u32 = 3_600_000;

fn overlay(bits: u64) -> OverlayInput {
    let lines = usize::try_from(field(bits, 0, 4)).unwrap();
    let date = LocalDate::new(
        2020 + i32::try_from(field(bits, 16, 11)).unwrap(),
        1 + small(bits, 24, 12),
        1 + small(bits, 32, 28),
    );
    OverlayInput {
        dim: field(bits, 40, 5) == 0,
        ..OverlayInput::new(
            (0..lines)
                .map(|i| pick(&LINES, bits >> (2 + 3 * i)).to_owned())
                .collect(),
            LocalClock::new(date, pick(&TIMES, bits >> 45)),
        )
    }
}

/// A valid scene of 0 to 40 plants, dressed, with its overlay.
fn scene() -> impl Strategy<Value = (SceneInput, OverlayInput)> {
    (
        any::<(u8, u8, u16)>(),
        prop::collection::vec(any::<u64>(), 0..=40),
        any::<(u64, u64, u64)>(),
    )
        .prop_map(|((state, kind, mask), bits, (world, things, label))| {
            let species = species(kind, mask);
            let plants = bits.into_iter().map(|b| plant(&species, b)).collect();
            let mut scene = scene_with(pick(PresentationState::ALL, u64::from(state)), plants);
            dress(&mut scene, world, things);
            (scene, overlay(label))
        })
}

/// The scene after another: a fresh one, the same again, or the old one edited plant by
/// plant, with newcomers from the pool, perhaps a new presentation state, the world nudged,
/// and perhaps new label lines or a new date.
#[derive(Clone, Debug)]
enum Next {
    Fresh(SceneInput, OverlayInput),
    Edited {
        edits: Vec<u32>,
        added: Vec<u64>,
        state: Option<u8>,
        world: u64,
        overlay: Option<u64>,
        event: u64,
    },
    Same,
}

fn next() -> impl Strategy<Value = Next> {
    prop_oneof![
        2 => scene().prop_map(|(scene, overlay)| Next::Fresh(scene, overlay)),
        5 => (
            prop::collection::vec(any::<u32>(), 40),
            prop::collection::vec(any::<u64>(), 0..6),
            any::<Option<u8>>(),
            any::<u64>(),
            prop::option::weighted(0.3, any::<u64>()),
            any::<u64>(),
        )
            .prop_map(|(edits, added, state, world, overlay, event)| Next::Edited {
                edits,
                added,
                state,
                world,
                overlay,
                event,
            }),
        1 => Just(Next::Same),
    ]
}

impl Next {
    fn after(&self, before: &(SceneInput, OverlayInput)) -> (SceneInput, OverlayInput) {
        let (scene, overlay_before) = before;
        match self {
            Next::Fresh(scene, overlay) => (scene.clone(), overlay.clone()),
            Next::Same => before.clone(),
            Next::Edited {
                edits,
                added,
                state,
                world,
                overlay: new_overlay,
                event,
            } => {
                // Each edit keeps the plant (most often), drops it, or changes its state, stage,
                // bloom or species.
                let mut plants: Vec<PlantView> = scene
                    .plants
                    .iter()
                    .zip(edits.iter().cycle())
                    .filter_map(|(&plant, &edit)| {
                        let to = u64::from(edit >> 8);
                        match edit % 14 {
                            0..=5 => Some(plant),
                            6 | 7 => None,
                            8 | 9 => Some(PlantView {
                                state: pick(PlantState::ALL, to),
                                ..plant
                            }),
                            10 | 11 => Some(PlantView {
                                stage: pick(GrowthStage::ALL, to),
                                ..plant
                            }),
                            12 => Some(PlantView {
                                bloom: !plant.bloom,
                                ..plant
                            }),
                            _ => Some(PlantView {
                                species: pick(PlantSpecies::ALL, to),
                                ..plant
                            }),
                        }
                    })
                    .collect();
                plants.extend(added.iter().map(|&bits| plant(&POOL, bits)));
                let state = state.map_or(scene.presentation.state, |s| {
                    pick(PresentationState::ALL, u64::from(s))
                });
                let mut next = SceneInput {
                    presentation: scene.presentation,
                    ..scene_with(state, plants)
                };
                next.presentation.state = state;
                next.weather = scene.weather;
                next.light = scene.light;
                next.weeds = scene.weeds;
                next.litter = scene.litter;
                next.fungus = scene.fungus;
                next.condensation = scene.condensation;
                next.dust = scene.dust;
                next.monuments.clone_from(&scene.monuments);
                next.fossils.clone_from(&scene.fossils);
                next.ground_marks.clone_from(&scene.ground_marks);
                next.residents.clone_from(&scene.residents);
                next.wind = scene.wind;
                // The event carried on, as the engine keeps it, or a new one, or none.
                next.event = scene.event;
                match field(*event, 0, 4) {
                    0 => cue(&mut next, *event >> 2),
                    1 => next.event = None,
                    _ => {}
                }
                nudge(&mut next, *world);
                let overlay = new_overlay.map_or_else(|| overlay_before.clone(), overlay);
                (next, overlay)
            }
        }
    }
}

/// When the next scene arrives after the one before: on a tween's boundary or either side of
/// it, past a step of the label's cycle, or once everything has settled.
#[derive(Clone, Copy, Debug)]
enum After {
    Ms(u32),
    Settled,
}

fn after() -> impl Strategy<Value = After> {
    let half = RESTYLE_MS / 2;
    let gaps = [
        1,
        half - 1,
        half,
        half + 1,
        RESTYLE_MS - 1,
        APPEAR_MS,
        LABEL_CYCLE_MS + 500,
        // Into the turns after, where residents and plants move.
        2 * TURN_MS_U32 + 700,
    ];
    (0_usize..10).prop_map(move |i| gaps.get(i).map_or(After::Settled, |&ms| After::Ms(ms)))
}

fn salt() -> impl Strategy<Value = InstallSalt> {
    any::<[u8; 16]>().prop_map(InstallSalt::new)
}

#[derive(Clone, Debug)]
struct History {
    scenes: [(SceneInput, OverlayInput); 3],
    gaps: [After; 2],
}

fn history() -> impl Strategy<Value = History> {
    (scene(), next(), next(), after(), after()).prop_map(|(a, b, c, ab, bc)| {
        let b = b.after(&a);
        let c = c.after(&b);
        for (scene, _) in [&a, &b, &c] {
            assert_eq!(scene.validate(), Ok(()), "the generator makes valid scenes");
        }
        History {
            scenes: [a, b, c],
            gaps: [ab, bc],
        }
    })
}

// ---- The checks ----------------------------------------------------------------------------

fn drawn(composer: &Composer, mix: u8, now: Millis) -> Vec<KeyedDraw> {
    composer.draws(art(), table(), mix, now)
}

fn plain(keyed: &[KeyedDraw]) -> Vec<Draw> {
    keyed.iter().map(|keyed| keyed.draw).collect()
}

fn band(species: PlantSpecies) -> Band {
    Band::from_layer(table().plant_layer(species)).expect("a plant layer")
}

/// The most weed cells, and the most of each kind the world has places for.
const WEED_CELLS: usize = 4;
const MONUMENTS: usize = 3;
const FOSSILS: usize = 6;
const LITTER: usize = 8;
const FUNGUS: usize = 6;
const DRESSING: usize = 6;
const STARS: usize = 7;

/// Constraints 2 and 3 on the placement, which every frame settles to.
fn check_placement(composer: &Composer, input: &SceneInput) -> Result<(), TestCaseError> {
    let placement = composer.placement();
    let occupied = placement.occupied();
    prop_assert!(
        (FLOOR..=CEILING).contains(&occupied),
        "occupancy {occupied}"
    );
    let weeds = placement.weeds.len();
    let room = CEILING.saturating_sub(placement.plants.len());
    prop_assert_eq!(
        weeds,
        usize::from(input.weeds).min(WEED_CELLS).min(room),
        "weed cells"
    );
    let plant_cells: BTreeSet<u8> = placement.plants.values().map(|p| p.cell).collect();
    let weed_cells: BTreeSet<u8> = placement.weeds.keys().copied().collect();
    let fill_cells: BTreeSet<u8> = placement.fill.keys().copied().collect();
    prop_assert!(plant_cells.is_disjoint(&weed_cells) && plant_cells.is_disjoint(&fill_cells));
    prop_assert!(weed_cells.is_disjoint(&fill_cells));
    let views: BTreeMap<u32, &PlantView> = input.plants.iter().map(|p| (p.id, p)).collect();
    for (id, placed) in &placement.plants {
        let view = views.get(id);
        prop_assert_eq!(view.map(|v| v.species), Some(placed.view.species));
        prop_assert_eq!(PATTERN[usize::from(placed.cell)], band(placed.view.species));
    }
    let species = placement.species_by_cell();
    for band in Band::ALL {
        for pair in band_cells(band).windows(2) {
            let [a, b] = [pair[0], pair[1]].map(|cell| species[usize::from(cell)]);
            prop_assert!(a.is_none() || a != b, "{:?} side by side in {:?}", a, pair);
        }
    }
    let layout = composer.layout();
    for pair in layout.slots().windows(2) {
        prop_assert!(pair[0].x < pair[1].x && pair[0].band != pair[1].band);
    }
    Ok(())
}

/// What a sprite depicts, read once from its art keys.
#[derive(Clone, Copy, Debug)]
enum Class {
    /// A plant's foliage or combined seedling.
    Foliage(PlantSpecies, GrowthStage, PlantState),
    Bloom(PlantSpecies),
    Other,
}

/// The variant whose art key is `key`, named as the crate names it.
fn find<T: Copy>(key: Option<&str>, all: &[T], name: fn(T) -> &'static str) -> Option<T> {
    all.iter()
        .copied()
        .find(|&variant| key == Some(art_key(name(variant)).as_str()))
}

fn class(draw: &Draw) -> Class {
    static CLASSES: std::sync::OnceLock<Vec<Class>> = std::sync::OnceLock::new();
    let classes = CLASSES.get_or_init(|| {
        art()
            .catalog
            .sprites
            .iter()
            .map(|sprite| {
                let keys = &sprite.keys;
                if keys.event.is_some() {
                    return Class::Other;
                }
                let species = find(
                    keys.species.as_deref(),
                    PlantSpecies::ALL,
                    PlantSpecies::name,
                );
                let growth = find(keys.stage.as_deref(), GrowthStage::ALL, GrowthStage::name);
                let health = find(keys.state.as_deref(), PlantState::ALL, PlantState::name);
                match (keys.part.as_deref(), species, growth, health) {
                    (Some("foliage" | "combined"), Some(sp), Some(gr), Some(he)) => {
                        Class::Foliage(sp, gr, he)
                    }
                    (Some("bloom"), Some(sp), _, _) => Class::Bloom(sp),
                    _ => Class::Other,
                }
            })
            .collect()
    });
    classes[draw.sprite.index()]
}

/// A plant's glow is its fourth part.
const PLANT_GLOW: u8 = 3;

/// I-7: the living looks drawn, thriving, pale or wilting, and the blooms, never outnumber
/// those of the scenes that could be showing; and a plant glows only where one of those scenes
/// has it young or mature and thriving.
fn check_subset(draws: &[KeyedDraw], scenes: &[&SceneInput]) -> Result<(), TestCaseError> {
    let mut allowed: BTreeMap<(PlantSpecies, GrowthStage, PlantState), usize> = BTreeMap::new();
    let mut blooms: BTreeMap<PlantSpecies, usize> = BTreeMap::new();
    let mut glowing: BTreeSet<u32> = BTreeSet::new();
    for scene in scenes {
        for plant in &scene.plants {
            *allowed
                .entry((plant.species, plant.stage, plant.state))
                .or_default() += 1;
            if plant.bloom
                && plant.stage == GrowthStage::Mature
                && plant.state == PlantState::Thriving
            {
                *blooms.entry(plant.species).or_default() += 1;
            }
            if plant.stage != GrowthStage::Seedling && plant.state == PlantState::Thriving {
                glowing.insert(plant.id);
            }
        }
    }
    let mut seen: BTreeMap<(PlantSpecies, GrowthStage, PlantState), usize> = BTreeMap::new();
    let mut seen_blooms: BTreeMap<PlantSpecies, usize> = BTreeMap::new();
    for keyed in draws {
        if keyed.draw.sprite == table().world().plant_glow {
            let id = match keyed.key.subject {
                Subject::Plant { id, .. } if keyed.key.part == PLANT_GLOW => Some(id),
                _ => None,
            };
            prop_assert!(
                id.is_some_and(|id| glowing.contains(&id)),
                "{:?} glows",
                keyed.key
            );
        }
        match class(&keyed.draw) {
            Class::Foliage(species, growth, health)
                if matches!(
                    health,
                    PlantState::Thriving | PlantState::Pale | PlantState::Wilting
                ) =>
            {
                *seen.entry((species, growth, health)).or_default() += 1;
            }
            Class::Bloom(species) => *seen_blooms.entry(species).or_default() += 1,
            Class::Foliage(..) | Class::Other => {}
        }
    }
    for (key, count) in seen {
        let bound = allowed.get(&key).copied().unwrap_or(0);
        prop_assert!(
            count <= bound,
            "{:?} drawn {} times, truth has {}",
            key,
            count,
            bound
        );
    }
    for (species, count) in seen_blooms {
        let bound = blooms.get(&species).copied().unwrap_or(0);
        prop_assert!(
            count <= bound,
            "{:?} blooms {} times, truth allows {}",
            species,
            count,
            bound
        );
    }
    Ok(())
}

/// Constraint 4 on the draws: the composer never brings a colour the art does not. Each draw
/// keeps its sprite's blend, is never more opaque than its sprite, and only glyphs are tinted,
/// in the label's ink; the frame is dimmed only in dim mode, to 40% of every colour.
fn check_palette_discipline(frame: &Frame, dim: bool) -> Result<(), TestCaseError> {
    prop_assert_eq!(frame.dim, if dim { 153 } else { 0 });
    for draw in &frame.draws {
        let sprite = art().catalog.sprite(draw.sprite);
        prop_assert_eq!(draw.blend, sprite.blend, "{} blends", &sprite.id);
        prop_assert!(draw.opacity <= sprite.opacity, "{} brightened", &sprite.id);
        if draw.tint.is_some() {
            prop_assert_eq!(sprite.keys.part.as_deref(), Some("glyph"));
            prop_assert_eq!(draw.tint, Some(Slot::SOIL_DEEP));
            prop_assert_eq!(draw.layer, Layer::Label);
        }
    }
    Ok(())
}

/// The one-message table: what the composer may show of its own dressing in each state.
struct Dressing {
    clouds: usize,
    sun: bool,
    stars: bool,
    tufts: bool,
}

fn dressing(state: PresentationState) -> Dressing {
    let (clouds, sun, stars, tufts) = match state {
        PresentationState::Thriving => (usize::MAX, true, true, true),
        PresentationState::Mature | PresentationState::Recovering => (1, true, true, true),
        PresentationState::Struggling => (1, true, true, false),
        PresentationState::Dormant => (0, false, false, false),
        PresentationState::Dead => (0, false, true, false),
    };
    Dressing {
        clouds,
        sun,
        stars,
        tufts,
    }
}

/// The clouds each weather brings: only a clear sky has fair-weather clouds.
fn weather_clouds(weather: Weather) -> usize {
    match weather {
        Weather::Clear => 3,
        Weather::Overcast | Weather::Storm | Weather::Haze => 0,
    }
}

/// How many draws stand for each kind of subject.
fn count(draws: &[KeyedDraw], kind: impl Fn(Subject) -> bool) -> usize {
    draws
        .iter()
        .map(|keyed| keyed.key.subject)
        .filter(|&s| kind(s))
        .collect::<BTreeSet<Subject>>()
        .len()
}

/// Constraint 6 once settled: the dressing quieted as the table says, the weather's gates
/// kept, and everything truth reports drawn.
fn check_one_message(
    day: &[KeyedDraw],
    night: &[KeyedDraw],
    input: &SceneInput,
    overlay: &OverlayInput,
    plants: usize,
) -> Result<(), TestCaseError> {
    let quiet = dressing(input.presentation.state);
    let weather = input.weather;
    let lets_bodies = matches!(weather, Weather::Clear | Weather::Haze);
    // Every cloud is depicted (`check_settled` counts them); one wholly out of sight is not
    // drawn.
    let clouds = weather_clouds(weather).min(quiet.clouds);
    prop_assert!(count(day, |s| matches!(s, Subject::Cloud(_))) <= clouds);
    prop_assert_eq!(
        count(day, |s| s == Subject::Sun),
        usize::from(lets_bodies && quiet.sun)
    );
    let moon = lets_bodies && moon_phase(overlay.clock.date) != MoonPhase::New;
    prop_assert_eq!(count(night, |s| s == Subject::Moon), usize::from(moon));
    let stars = if weather == Weather::Clear && quiet.stars {
        STARS
    } else {
        0
    };
    prop_assert_eq!(count(night, |s| matches!(s, Subject::Star(_))), stars);
    let tufts = if quiet.tufts { DRESSING } else { 0 };
    prop_assert_eq!(count(day, |s| matches!(s, Subject::Dressing(_))), tufts);
    // Truth's signals always show.
    let weeds = usize::from(input.weeds)
        .min(WEED_CELLS)
        .min(CEILING.saturating_sub(plants));
    prop_assert_eq!(count(day, |s| matches!(s, Subject::Weed { .. })), weeds);
    prop_assert_eq!(
        count(day, |s| matches!(s, Subject::Litter(_))),
        usize::from(input.litter).min(LITTER)
    );
    prop_assert_eq!(
        count(day, |s| matches!(s, Subject::Fungus(_))),
        usize::from(input.fungus).min(FUNGUS)
    );
    prop_assert_eq!(
        count(day, |s| matches!(s, Subject::Monument { .. })),
        input.monuments.len().min(MONUMENTS)
    );
    prop_assert_eq!(
        count(day, |s| matches!(s, Subject::Fossil { .. })),
        input.fossils.len().min(FOSSILS)
    );
    prop_assert_eq!(
        count(day, |s| s == Subject::Light),
        usize::from(input.light != Light::Neutral)
    );
    prop_assert_eq!(
        count(day, |s| matches!(s, Subject::Film(_))),
        usize::from(input.condensation > 0) + usize::from(input.dust > 0)
    );
    prop_assert_eq!(count(day, |s| s == Subject::Weather), 1);
    let marks: BTreeSet<u32> = day
        .iter()
        .filter_map(|keyed| match keyed.key.subject {
            Subject::Mark { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    prop_assert_eq!(marks, input.ground_marks.iter().map(|m| m.id).collect());
    Ok(())
}

/// The opacity a settled draw shows at: its sprite's own by day and its night opacity, where
/// it has one, by night; gone by day if it shows only at night and by night if only by day.
fn settled_opacity(draw: &Draw, mix: u8) -> u8 {
    let sprite = art().catalog.sprite(draw.sprite);
    let scale = |a: u8, b: u8| u8::try_from((u16::from(a) * u16::from(b) + 127) / 255).unwrap();
    let own = match (sprite.night_opacity, mix) {
        (Some(night), 255) => night,
        _ => sprite.opacity,
    };
    if sprite.flags.contains(Flags::NIGHT_ONLY) {
        scale(own, mix)
    } else if sprite.flags.contains(Flags::DAY_ONLY) {
        scale(own, 255 - mix)
    } else {
        own
    }
}

/// Once settled: every draw at its own opacity, exactly the placement drawn, the healthy
/// subset exact, each third of the strip with at most two mature trees and never two of one
/// species, and the one-message table kept.
fn check_settled(
    composer: &Composer,
    input: &SceneInput,
    overlay: &OverlayInput,
) -> Result<(), TestCaseError> {
    let now = composer.settled_at();
    let [day, night] = [0, 255].map(|mix| drawn(composer, mix, now));
    for (mix, draws) in [(0, &day), (255, &night)] {
        for keyed in draws {
            // The label cycles past the moment it settled; residents are checked below.
            if !matches!(keyed.key.subject, Subject::Label | Subject::Resident { .. }) {
                prop_assert_eq!(
                    keyed.draw.opacity,
                    settled_opacity(&keyed.draw, mix),
                    "{:?} at {}",
                    keyed.key,
                    mix
                );
            }
        }
        check_resident_opacity(composer, mix, now, draws)?;
    }
    let subjects: BTreeSet<Subject> = day
        .iter()
        .map(|keyed| keyed.key.subject)
        .filter(|s| {
            matches!(
                s,
                Subject::Plant { .. } | Subject::Fill { .. } | Subject::Weed { .. }
            )
        })
        .collect();
    let placement = composer.placement();
    let placed: BTreeSet<Subject> = placement
        .plants
        .iter()
        .map(|(&id, placed)| Subject::Plant {
            id,
            cell: placed.cell,
        })
        .chain(placement.fill.keys().map(|&cell| Subject::Fill { cell }))
        .chain(placement.weeds.keys().map(|&cell| Subject::Weed { cell }))
        .collect();
    prop_assert_eq!(subjects, placed);
    check_subset(&night, &[input])?;
    let mut trees: BTreeMap<i32, Vec<PlantSpecies>> = BTreeMap::new();
    for keyed in &day {
        if let Class::Foliage(species, GrowthStage::Mature, _) = class(&keyed.draw)
            && keyed.key.layer == Layer::FarPlants
        {
            trees
                .entry(screen_third(keyed.key.x))
                .or_default()
                .push(species);
        }
    }
    for (third, mut species) in trees {
        prop_assert!(species.len() <= 2, "third {}: {:?}", third, species);
        species.dedup();
        prop_assert!(species.len() == 1 || species[0] != species[1]);
    }
    check_one_message(&day, &night, input, overlay, placement.plants.len())?;
    let clouds: BTreeSet<Subject> = composer
        .depicted(now)
        .iter()
        .map(|depicted| depicted.subject)
        .filter(|subject| matches!(subject, Subject::Cloud(_)))
        .collect();
    let quiet = dressing(input.presentation.state);
    prop_assert_eq!(
        clouds.len(),
        weather_clouds(input.weather).min(quiet.clouds)
    );
    check_residents_settled(&day, input)?;
    let scenery = composer.scenery(art(), table());
    for mix in [0, 255] {
        check_palette_discipline(
            &composer.frame(art(), table(), &scenery, mix, now),
            overlay.dim,
        )?;
    }
    Ok(())
}

/// The art's life stages for a resident's: an egg is a brood or a cocoon for some species,
/// and a dying resident plays its adult death.
fn art_stages(stage: LifeStage) -> &'static [&'static str] {
    match stage {
        LifeStage::Egg => &["egg", "brood", "cocoon"],
        LifeStage::Juvenile => &["juvenile"],
        LifeStage::Pupa => &["pupa"],
        LifeStage::Adult | LifeStage::Dying => &["adult"],
    }
}

/// Once settled, residents show what truth says they are: only the scene's own, each in art
/// of its own species and stage (a juvenile never in adult art), a dying one in its death;
/// none travelling in a dormant scene, and every adult in hiding in a storm.
fn check_residents_settled(day: &[KeyedDraw], input: &SceneInput) -> Result<(), TestCaseError> {
    let views: BTreeMap<u32, &ResidentView> = input.residents.iter().map(|r| (r.id, r)).collect();
    for keyed in day {
        let Subject::Resident { id, .. } = keyed.key.subject else {
            continue;
        };
        let view = views.get(&id);
        prop_assert!(view.is_some(), "resident {} is not in the scene", id);
        let view = view.unwrap();
        if keyed.key.part != 0 {
            continue;
        }
        let keys = &art().catalog.sprite(keyed.draw.sprite).keys;
        let routine = keys.state.as_deref().unwrap_or("");
        let species = art_key(view.species.name());
        prop_assert_eq!(keys.species.as_deref(), Some(species.as_str()));
        let stage = keys.life_stage.as_deref().unwrap_or("");
        prop_assert!(
            art_stages(view.stage).contains(&stage),
            "{:?} shown as {}",
            view,
            stage
        );
        if view.stage == LifeStage::Dying {
            prop_assert_eq!(routine, "death");
        }
        let travelling = matches!(routine, "move" | "takeoff" | "flight" | "land" | "hop");
        if input.presentation.state == PresentationState::Dormant {
            prop_assert!(!travelling, "{:?} travels while dormant", view);
        }
        if input.weather == Weather::Storm && view.stage == LifeStage::Adult {
            prop_assert!(
                matches!(routine, "hide" | "withdraw" | "curl" | "retreat"),
                "{:?} out in a storm: {}",
                view,
                routine
            );
        }
    }
    Ok(())
}

/// The fastest a draw of `subject` may change: the light crossfades over the art's own time,
/// and a label line can crossfade while its whole set does, which doubles its pace.
fn fastest_ms(subject: Subject) -> u64 {
    match subject {
        Subject::Light => u64::from(table().world().light_ms),
        Subject::Label => u64::from(LABEL_FADE_MS) / 2,
        _ => u64::from(SHORTEST_MS),
    }
}

/// The fastest a cloud drifts: a pixel in this many milliseconds.
const CLOUD_MS_PER_PX: u64 = 2600;
/// The positions a cloud's path runs through before it starts again.
const CLOUD_PATH: i32 = 673;
/// The soil's texture drifts a pixel right every 3 min, its pattern repeating every 144 pixels;
/// the sweep moves 2 pixels every 100 ms. The backdrop holds still.
const SOIL_MS_PER_PX: u64 = 180_000;
const SOIL_PERIOD: i32 = 144;
const SWEEP_PX_PER_100_MS: u64 = 2;
/// Motion comes in turns of 24 s; an update moves things from the turn after its own.
const TURN_MS: u64 = 24_000;
const TURN_MS_U32: u32 = 24_000;

/// Whether a tile `w` wide at `x` stands at an edge of the frame, where a drifting tile may
/// enter or leave.
fn at_edge(x: i32, w: i32) -> bool {
    x + w <= w.max(1) || x >= WIDTH - w
}

/// The storm front crosses a column in no less than a frame's time; a windstorm's leaf
/// crosses one in no less than this, and bobs a row in no less than this; rain fades in and
/// out over this long.
const FRONT_MS_PER_PX: u64 = 67;
const LEAF_MS_PER_PX: u64 = 16;
const LEAF_BOB_MS: u64 = 250;
const RAIN_FADE_MS: u64 = 3000;
/// The storm front's curtain and a leaf, in columns.
const CURTAIN: i32 = 24;
const LEAF: i32 = 3;
/// A sprinkle of flowers comes in over this long and goes over longer; a scorch comes and goes
/// over this long at its quickest, and the plant it hides goes and comes with it, on top of its
/// own tweens at their quickest.
const SPRINKLE_MS: u64 = 900;
const SCORCH_MS: u64 = 600;
const SCORCHED_MS: u64 = 500;

/// Where a draw's anchor lands across: what holds still when its frames change.
fn anchor_x(draw: &Draw) -> i32 {
    let sprite = art().catalog.sprite(draw.sprite);
    if draw.flip_x {
        i32::from(sprite.rect.w) - 1 - i32::from(sprite.anchor.x)
    } else {
        i32::from(sprite.anchor.x)
    }
}

/// The columns a draw shows in the frame, after its clip; empty as `(x, x)`.
fn columns(draw: &Draw) -> (i32, i32) {
    let w = i32::from(art().catalog.sprite(draw.sprite).rect.w);
    let (mut from, mut to) = (draw.x.max(0), (draw.x + w).min(WIDTH));
    if let Some(clip) = draw.clip {
        (from, to) = (from.max(clip.x), to.min(clip.x + clip.w));
    }
    (from, to.max(from))
}

/// Whether a sprite is one of an event's, by its art's name.
fn plays(draw: &Draw, art_name: &str) -> bool {
    art().catalog.sprite(draw.sprite).id.starts_with(art_name)
}

/// Whether a draw is a frame a crack or a sinkhole opens in before it settles.
fn opening(draw: &Draw) -> bool {
    let id = &art().catalog.sprite(draw.sprite).id;
    [
        "tmp.ground.crack.f0",
        "tmp.ground.sinkhole.f01",
        "tmp.ground.sinkhole.f02",
        "tmp.ground.sinkhole.f03",
    ]
    .iter()
    .any(|frame| id.starts_with(frame))
}

/// The family a mark's art belongs to: a crack, a sinkhole or fresh soil.
fn family(draw: &Draw) -> &'static str {
    let id = &art().catalog.sprite(draw.sprite).id;
    [
        "tmp.ground.crack.",
        "tmp.ground.sinkhole.",
        "tmp.soil.fresh-patch.",
    ]
    .into_iter()
    .find(|prefix| id.starts_with(prefix))
    .unwrap_or("")
}

/// Whether a draw within `reach` columns of the frame's edges could come or go there.
fn by_an_edge(draw: &Draw, width: i32, reach: i32) -> bool {
    draw.x < reach || draw.x + width > WIDTH - reach
}

/// I-15 for an event's own pieces, which play their art: the curtain crosses at its pace and
/// comes and goes only at an edge; lightning's two frames show and go as drawn (I-11: a bolt
/// has no fade); rain's frames fall in place, fading in and out; a leaf crosses and bobs at its
/// pace, and comes and goes only at an edge. On the ground, burn-off's frames show and go as
/// drawn in place; a sprinkle and a scorch stay put, fading at their art's pace; a crack or a
/// sinkhole opens in place through its own frames into its settled one.
fn check_event_piece(
    subject: Subject,
    a: Option<&Draw>,
    b: Option<&Draw>,
    elapsed: u64,
) -> Result<(), TestCaseError> {
    let steps = |ms_per_px: u64| i32::try_from(elapsed / ms_per_px + 1).unwrap();
    match (subject, a, b) {
        (Subject::Front, Some(a), Some(b)) => {
            prop_assert_eq!((a.sprite, a.y, a.flip_x), (b.sprite, b.y, b.flip_x));
            prop_assert!(
                (b.x - a.x).abs() <= steps(FRONT_MS_PER_PX),
                "front {} to {}",
                a.x,
                b.x
            );
        }
        (Subject::Front, Some(at), None) | (Subject::Front, None, Some(at)) => {
            prop_assert!(
                by_an_edge(at, CURTAIN, steps(FRONT_MS_PER_PX)),
                "front at {}",
                at.x
            );
        }
        (Subject::Bolt, ..) => {
            for draw in a.into_iter().chain(b) {
                prop_assert!(plays(draw, "tmp.sky.lightning."), "bolt");
            }
            if let (Some(a), Some(b)) = (a, b) {
                prop_assert_eq!((a.x, a.y), (b.x, b.y));
            }
        }
        (Subject::Rain, ..) => {
            let own = u64::from(art().catalog.sprite(a.or(b).unwrap().sprite).opacity);
            let bound = own * 3 * elapsed / (2 * RAIN_FADE_MS) + 1;
            let opacity = |draw: Option<&Draw>| u64::from(draw.map_or(0, |d| d.opacity));
            prop_assert!(opacity(a).abs_diff(opacity(b)) <= bound, "rain jumped");
            for draw in a.into_iter().chain(b) {
                prop_assert!(plays(draw, "tmp.sky.rain."), "rain");
            }
            if let (Some(a), Some(b)) = (a, b) {
                prop_assert_eq!((a.x, a.y), (b.x, b.y));
            }
        }
        (Subject::Leaf(_), Some(a), Some(b)) => {
            prop_assert!(plays(a, "tmp.sky.wind-leaf.") && plays(b, "tmp.sky.wind-leaf."));
            prop_assert_eq!(a.flip_x, b.flip_x);
            prop_assert!(
                (b.x - a.x).abs() <= steps(LEAF_MS_PER_PX),
                "leaf {} to {}",
                a.x,
                b.x
            );
            let bob = steps(LEAF_BOB_MS).min(4);
            prop_assert!((b.y - a.y).abs() <= bob, "leaf {} to {}", a.y, b.y);
        }
        (Subject::Leaf(_), Some(at), None) | (Subject::Leaf(_), None, Some(at)) => {
            prop_assert!(
                by_an_edge(at, LEAF, steps(LEAF_MS_PER_PX) + LEAF),
                "leaf at {}",
                at.x
            );
        }
        (Subject::Burn(_), ..) => {
            for draw in a.into_iter().chain(b) {
                prop_assert!(plays(draw, "tmp.event.burnoff."), "burn-off");
            }
            if let (Some(a), Some(b)) = (a, b) {
                prop_assert_eq!((a.x + anchor_x(a), a.y), (b.x + anchor_x(b), b.y));
            }
        }
        (Subject::Sprinkle(_) | Subject::Scorch { .. }, ..) => {
            let (art_name, fastest) = match subject {
                Subject::Sprinkle(_) => ("tmp.event.flower-sprinkle.", SPRINKLE_MS),
                _ => ("tmp.event.scorch.", SCORCH_MS),
            };
            let drawn = a.or(b).unwrap();
            let own = u64::from(art().catalog.sprite(drawn.sprite).opacity);
            let bound = own * 3 * elapsed / (2 * fastest) + 1;
            let opacity = |draw: Option<&Draw>| u64::from(draw.map_or(0, |d| d.opacity));
            prop_assert!(
                opacity(a).abs_diff(opacity(b)) <= bound,
                "{:?} jumped",
                subject
            );
            for draw in a.into_iter().chain(b) {
                prop_assert!(plays(draw, art_name), "{:?}", subject);
            }
            if let (Some(a), Some(b)) = (a, b) {
                prop_assert_eq!((a.sprite, a.x, a.y, a.layer), (b.sprite, b.x, b.y, b.layer));
            }
        }
        (Subject::Mark { .. }, ..) => {
            // A frame shows whole, as drawn, and gives way to the next or to the settled one.
            if let (Some(a), Some(b)) = (a, b) {
                prop_assert_eq!(family(a), family(b));
                prop_assert_eq!((a.x + anchor_x(a), a.y), (b.x + anchor_x(b), b.y));
            }
        }
        _ => prop_assert!(false, "{:?} is no event's piece", subject),
    }
    Ok(())
}

/// One event takes the stage at a time: the pieces drawn are of one choreography at most.
fn check_one_show(draws: &[KeyedDraw]) -> Result<(), TestCaseError> {
    let shows: BTreeSet<u8> = draws
        .iter()
        .filter_map(|keyed| match keyed.key.subject {
            Subject::Front => Some(0),
            Subject::Bolt | Subject::Scorch { .. } => Some(1),
            Subject::Rain => Some(2),
            Subject::Leaf(_) => Some(3),
            Subject::Burn(_) | Subject::Sprinkle(_) => Some(4),
            Subject::Mark { .. } if opening(&keyed.draw) => Some(5),
            _ => None,
        })
        .collect();
    prop_assert!(shows.len() <= 1, "{:?} play at once", shows);
    Ok(())
}

/// I-15 between two moments with no update between them: no draw's opacity jumps further
/// than its fastest tween can move it, a cloud drifts no faster than its pace and wraps only
/// out of sight, an event's pieces play as their art does, and while the storm front crosses,
/// the sky on either side of it is cut no faster than it moves; nothing else moves. A burn steps
/// the weeds out and the fresh soil in by the art's keys on the cells it burns, and a scorch
/// hides its plant's foliage, bloom and glow at its own pace.
fn check_continuity(
    before: &[KeyedDraw],
    after: &[KeyedDraw],
    elapsed: u64,
) -> Result<(), TestCaseError> {
    // While the storm front crosses, the sky's draws are cut at it.
    let wiping = [before, after].iter().any(|draws| {
        draws.iter().any(|keyed| {
            keyed.key.subject == Subject::Front
                || (keyed.key.layer == Layer::Sky && keyed.draw.clip.is_some())
        })
    });
    let front = i32::try_from(elapsed / FRONT_MS_PER_PX + 1).unwrap();
    let index = |draws: &[KeyedDraw]| -> BTreeMap<(Subject, u8, u16, u16), Draw> {
        draws
            .iter()
            .map(|keyed| {
                let key = keyed.key;
                ((key.subject, key.part, key.look, key.item), keyed.draw)
            })
            .collect()
    };
    check_residents_continuity(before, after, elapsed)?;
    let (burning, scorched) = ground_shows(before, after);
    let (old, new) = (index(before), index(after));
    for key in old.keys().chain(new.keys()) {
        if matches!(key.0, Subject::Resident { .. }) {
            continue;
        }
        let [a, b] = [old.get(key), new.get(key)];
        let drawn = a.or(b).unwrap();
        let settling = matches!(key.0, Subject::Mark { .. })
            && (a.into_iter().chain(b).any(opening)
                || a.zip(b).is_some_and(|(a, b)| a.sprite != b.sprite));
        if settling
            || matches!(
                key.0,
                Subject::Front
                    | Subject::Bolt
                    | Subject::Rain
                    | Subject::Leaf(_)
                    | Subject::Burn(_)
                    | Subject::Sprinkle(_)
                    | Subject::Scorch { .. }
            )
        {
            check_event_piece(key.0, a, b, elapsed)?;
            continue;
        }
        // Burn-off steps its weeds out and its fresh soil in, as the art keys them.
        let burned = match key.0 {
            Subject::Weed { cell } if burning.contains(&cell) => Some(false),
            Subject::Mark { at, .. }
                if family(drawn) == "tmp.soil.fresh-patch." && burning.contains(&at) =>
            {
                Some(true)
            }
            _ => None,
        };
        if let Some(soil) = burned {
            check_burned(key, a, b, soil, elapsed)?;
            continue;
        }
        if wiping && drawn.layer == Layer::Sky {
            // Uncovered or covered as the curtain passes, no wider than it moved; anything
            // wider comes or goes as any look does.
            let (from, to) = columns(drawn);
            if (a.is_none() || b.is_none()) && to - from <= front {
                continue;
            }
            if let (Some(a), Some(b)) = (a, b) {
                let ((a0, a1), (b0, b1)) = (columns(a), columns(b));
                prop_assert!(
                    (b0 - a0).abs() <= front && (b1 - a1).abs() <= front,
                    "{:?} cut from {:?} to {:?}",
                    key,
                    (a0, a1),
                    (b0, b1)
                );
            }
        }
        let width = i32::from(art().catalog.sprite(drawn.sprite).rect.w);
        // A drifting tile, a cloud or the sweep may come and go at an edge of the frame;
        // anywhere else it fades like any other draw.
        let drifting = matches!(key.0, Subject::Soil | Subject::Sweep | Subject::Cloud(_));
        if drifting && (a.is_none() || b.is_none()) && at_edge(drawn.x, width) {
            continue;
        }
        let bound = opacity_bound((key.0, key.1), drawn, elapsed, &scorched);
        let opacity = |draw: Option<&Draw>| u64::from(draw.map_or(0, |d| d.opacity));
        prop_assert!(
            opacity(a).abs_diff(opacity(b)) <= bound,
            "{:?} jumped from {} to {} past {}",
            key,
            opacity(a),
            opacity(b),
            bound
        );
        if let (Some(a), Some(b)) = (a, b) {
            check_place(key.0, a, b, elapsed)?;
        }
    }
    Ok(())
}

/// On a cell a burn plays on, a weed only falls or goes, and fresh soil only rises or comes, at
/// the art's keys; but fresh soil the scene stopped holding mid-burn fades out over what is left
/// of the show, at least the sprinkles' 2.7 s.
fn check_burned(
    key: &(Subject, u8, u16, u16),
    a: Option<&Draw>,
    b: Option<&Draw>,
    soil: bool,
    elapsed: u64,
) -> Result<(), TestCaseError> {
    let opacity = |draw: Option<&Draw>| u64::from(draw.map_or(0, |d| d.opacity));
    let (from, to) = (opacity(a), opacity(b));
    let fading = 255 * 3 * elapsed / (2 * LEAVING_MS) + 2;
    prop_assert!(
        if soil {
            to >= from || from - to <= fading
        } else {
            to <= from
        },
        "{:?} stepped from {} to {} against the burn",
        key,
        from,
        to
    );
    if let (Some(a), Some(b)) = (a, b) {
        prop_assert_eq!((a.sprite, a.x, a.y), (b.sprite, b.x, b.y), "{:?}", key);
    }
    Ok(())
}

/// The shortest a burn's fresh soil the scene stopped holding takes to fade out: the show left
/// after the ash clears, the sprinkles' 900, 600 and 1200 ms.
const LEAVING_MS: u64 = 2700;

/// The cells a burn plays on, and the plants a scorch stands over, at either of two moments.
fn ground_shows(before: &[KeyedDraw], after: &[KeyedDraw]) -> (BTreeSet<u8>, BTreeSet<u32>) {
    let subjects = || [before, after].into_iter().flatten().map(|k| k.key.subject);
    let burning = subjects()
        .filter_map(|subject| match subject {
            Subject::Burn(cell) => Some(cell),
            _ => None,
        })
        .collect();
    let scorched = subjects()
        .filter_map(|subject| match subject {
            Subject::Scorch { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    (burning, scorched)
}

/// How far a draw's opacity may move in `elapsed`: by its fastest tween, where a scorch hides
/// its plant's foliage (part 1), bloom (2) and glow (3) at its own pace.
fn opacity_bound(
    (subject, part): (Subject, u8),
    drawn: &Draw,
    elapsed: u64,
    scorched: &BTreeSet<u32>,
) -> u64 {
    let own = u64::from(art().catalog.sprite(drawn.sprite).opacity);
    let veiled = matches!(subject, Subject::Plant { id, .. } if scorched.contains(&id))
        && (1..=3).contains(&part);
    let fastest = if veiled {
        SCORCHED_MS
    } else {
        fastest_ms(subject)
    };
    // Smoothstep's steepest slope is 1.5 times the average. A label line, and a look a scorch
    // hides, is drawn at the product of two tweens, each rounded, so both may round a step in
    // the same millisecond.
    let rounding = if veiled || subject == Subject::Label {
        2
    } else {
        1
    };
    own * 3 * elapsed / (2 * fastest) + rounding
}

/// A draw at two moments holds its sprite, row and layer, and its column but for a drift,
/// which carries it forward along its path, wrapping out of sight, no further than its pace.
fn check_place(subject: Subject, a: &Draw, b: &Draw, elapsed: u64) -> Result<(), TestCaseError> {
    prop_assert_eq!(
        (a.sprite, a.y, a.layer),
        (b.sprite, b.y, b.layer),
        "{:?} changed",
        subject
    );
    let drift = |period: i32, ms_per_px: u64| -> Result<(), TestCaseError> {
        let moved = u64::try_from((b.x - a.x).rem_euclid(period)).unwrap();
        prop_assert!(
            moved <= elapsed / ms_per_px + 1,
            "{:?} drifted from {} to {}",
            subject,
            a.x,
            b.x
        );
        Ok(())
    };
    match subject {
        Subject::Cloud(_) => drift(CLOUD_PATH, CLOUD_MS_PER_PX)?,
        Subject::Soil => drift(SOIL_PERIOD, SOIL_MS_PER_PX)?,
        Subject::Sweep => {
            let moved = u64::try_from(b.x - a.x).unwrap_or(u64::MAX);
            prop_assert!(moved <= SWEEP_PX_PER_100_MS * (elapsed / 100 + 1));
        }
        _ => prop_assert_eq!(a.x, b.x, "{:?} moved", subject),
    }
    Ok(())
}

/// The widest step a creature's frame carries it, in pixels (the art's), and the shortest
/// frame, in milliseconds.
const MOST_ROOT_DELTA: u64 = 4;
const SHORTEST_FRAME_MS: u64 = 167;
/// A resident's cover rises and falls over half the art's 1200 ms each.
const RESIDENT_FADE_MS: u64 = 600;

/// I-15 for residents, which play their art: between two moments a resident's frames may
/// change, but only to frames of its own species, on the ground or in the air; it never
/// rises or sinks; it moves along the ground no faster than its frames carry it; and its
/// strongest look never jumps, so a cover or a move never shows a gap or a pop.
fn check_residents_continuity(
    before: &[KeyedDraw],
    after: &[KeyedDraw],
    elapsed: u64,
) -> Result<(), TestCaseError> {
    let looks = |draws: &[KeyedDraw]| -> BTreeMap<Subject, Vec<Draw>> {
        let mut looks: BTreeMap<Subject, Vec<Draw>> = BTreeMap::new();
        for keyed in draws {
            if matches!(keyed.key.subject, Subject::Resident { .. }) && keyed.key.part == 0 {
                looks.entry(keyed.key.subject).or_default().push(keyed.draw);
            }
        }
        looks
    };
    let (old, new) = (looks(before), looks(after));
    let fade = 255 * 3 * elapsed / (2 * RESIDENT_FADE_MS) + 1;
    let reach = i32::try_from((elapsed / SHORTEST_FRAME_MS + 1) * MOST_ROOT_DELTA + 1).unwrap();
    for subject in old.keys().chain(new.keys()) {
        let [a, b] = [old.get(subject), new.get(subject)];
        let strongest = |draws: Option<&Vec<Draw>>| {
            u64::from(
                draws
                    .and_then(|d| d.iter().map(|d| d.opacity).max())
                    .unwrap_or(0),
            )
        };
        prop_assert!(
            strongest(a).abs_diff(strongest(b)) <= fade,
            "{:?} jumped from {} to {} in {} ms: {:?}",
            subject,
            strongest(a),
            strongest(b),
            elapsed,
            a.or(b).map(|d| d
                .iter()
                .map(|d| art().catalog.sprite(d.sprite).id.clone())
                .collect::<Vec<_>>())
        );
        for draw in a.into_iter().chain(b).flatten() {
            let sprite = art().catalog.sprite(draw.sprite);
            prop_assert!(matches!(
                draw.layer,
                Layer::ResidentsGround | Layer::ResidentsAir
            ));
            prop_assert_eq!(
                draw.y,
                PLANT_GROUND_Y - i32::from(sprite.anchor.y),
                "{:?} left the ground line",
                subject
            );
        }
        if let (Some(a), Some(b)) = (a, b) {
            let species = |draws: &Vec<Draw>| -> BTreeSet<Option<String>> {
                draws
                    .iter()
                    .map(|d| art().catalog.sprite(d.sprite).keys.species.clone())
                    .collect()
            };
            // An id that becomes another species crossfades: the two overlap while it does.
            prop_assert!(
                !species(a).is_disjoint(&species(b)),
                "{:?} changed species",
                subject
            );
            // The top looks, compared at their anchors: mirrored or not, the ground point.
            let point = |draw: &Draw| {
                let sprite = art().catalog.sprite(draw.sprite);
                let anchor = if draw.flip_x {
                    i32::from(sprite.rect.w) - 1 - i32::from(sprite.anchor.x)
                } else {
                    i32::from(sprite.anchor.x)
                };
                draw.x + anchor
            };
            let (from, to) = (point(a.last().unwrap()), point(b.last().unwrap()));
            // A move of home crossfades between two places; anything else walks.
            if a.len() == 1 && b.len() == 1 {
                let sprites = |draws: &Vec<Draw>| -> Vec<String> {
                    draws
                        .iter()
                        .map(|d| format!("{}@{}", art().catalog.sprite(d.sprite).id, d.opacity))
                        .collect()
                };
                prop_assert!(
                    (to - from).abs() <= reach,
                    "{:?} slid from {} to {} in {} ms: {:?} → {:?}",
                    subject,
                    from,
                    to,
                    elapsed,
                    sprites(a),
                    sprites(b)
                );
            }
        }
    }
    Ok(())
}

/// Every resident's looks at their own opacity once settled; of a resident that has travelled
/// in this turn, which ends its trip by covering its last pose with its rest, only the
/// strongest look.
fn check_resident_opacity(
    composer: &Composer,
    mix: u8,
    now: Millis,
    draws: &[KeyedDraw],
) -> Result<(), TestCaseError> {
    let mut residents: BTreeMap<(Subject, u8), Vec<&KeyedDraw>> = BTreeMap::new();
    for keyed in draws {
        if matches!(keyed.key.subject, Subject::Resident { .. }) {
            residents
                .entry((keyed.key.subject, keyed.key.part))
                .or_default()
                .push(keyed);
        }
    }
    for ((subject, _), looks) in residents {
        let unsettled = |keyed: &KeyedDraw| keyed.draw.opacity != settled_opacity(&keyed.draw, mix);
        let Some(off) = looks.iter().copied().find(|keyed| unsettled(keyed)) else {
            continue;
        };
        let held = if travelled_this_turn(composer, mix, now, subject) {
            looks
                .iter()
                .copied()
                .max_by_key(|keyed| keyed.draw.opacity)
                .unwrap_or(off)
        } else {
            off
        };
        prop_assert_eq!(
            held.draw.opacity,
            settled_opacity(&held.draw, mix),
            "{:?} at {}",
            held.key,
            mix
        );
    }
    Ok(())
}

/// Whether the resident `subject` has travelled or signalled since its turn began, up to `now`:
/// either ends by covering its last pose with its rest.
fn travelled_this_turn(composer: &Composer, mix: u8, now: Millis, subject: Subject) -> bool {
    let start = now.0 / TURN_MS * TURN_MS;
    (start..now.0).step_by(50).any(|ms| {
        drawn(composer, mix, Millis(ms)).iter().any(|keyed| {
            let routine = art()
                .catalog
                .sprite(keyed.draw.sprite)
                .keys
                .state
                .as_deref();
            keyed.key.subject == subject
                && keyed.key.part == 0
                && matches!(
                    routine,
                    Some("move" | "takeoff" | "flight" | "land" | "hop" | "signal")
                )
        })
    })
}

/// When an event's choreography turns, from the update that cues it: lightning's frames, rain's
/// fades (held 12, 24 or 36 s), the leaves on their way, and the storm front across 640
/// columns and its curtain's 24, a column in 67 ms; on the ground, burn-off's frames from a
/// millisecond on (180, 220, 240 and 300 ms) and the sprinkles after them (900, 600 and
/// 1200 ms), the scorch from the bolt's end (600, 900 and two halves of 1200 ms), the crack's
/// and the sinkhole's frames (166 ms each) and a flowering's minute. The frame-holds property
/// adds the end of the front's crossing for each of its viewports.
const CUES: [u64; 54] = [
    68, 134, 166, 167, 180, 181, 332, 333, 334, 400, 401, 497, 498, 499, 500, 640, 641, 933, 934,
    939, 940, 941, 1833, 1834, 1840, 1841, 2000, 2433, 2434, 2440, 2441, 2999, 3001, 3033, 3034,
    3640, 3641, 3642, 9000, 15_000, 16_500, 18_000, 21_000, 27_000, 30_000, 33_000, 39_000, 42_000,
    44_487, 44_488, 44_489, 50_000, 60_000, 64_000,
];

/// Moments worth looking at after an update: its tween boundaries and either side of them,
/// the label's cycle steps from any label set raised at `raised`, and the turns of the
/// choreography each update at `cued` that carried an event may have begun, which plays on
/// through the updates after it.
fn moments(update: Millis, until: Millis, raised: &[Millis], cued: &[Millis]) -> Vec<Millis> {
    let half = RESTYLE_MS / 2;
    // Quarter steps keep the plants' continuity bound near 96 of 255, so a cut mid-tween
    // shows; tenths of the light's crossfade do the same for it; the boundaries and their
    // neighbours catch a cut at a phase change.
    let mut moments: Vec<Millis> = [
        0,
        1,
        300,
        600,
        750,
        900,
        1500,
        2250,
        half - 1,
        half,
        half + 1,
        3750,
        APPEAR_MS,
        4500,
        5250,
        RESTYLE_MS - 1,
        RESTYLE_MS + 1,
    ]
    .into_iter()
    .map(|ms| update.plus(ms))
    .collect();
    // Through the start of the first turn the update moves in, where changes of rest play.
    let moving = (update.0 / TURN_MS + 1) * TURN_MS;
    for offset in (0..=7000).step_by(500).chain([1, 20_000]) {
        moments.push(Millis(moving + offset));
    }
    for &cue in cued {
        // On a first update the choreography waits for the fade-in.
        for after in [0, u64::from(SKY_MS)] {
            moments.extend(CUES.iter().map(|&ms| Millis(cue.0 + after + ms)));
        }
        // Residents an earthquake sends into hiding come out a turn later.
        let after = (cue.0 / TURN_MS + 2) * TURN_MS;
        moments.extend((0..=7000).step_by(500).map(|offset| Millis(after + offset)));
    }
    let cycle = u64::from(LABEL_CYCLE_MS);
    for &since in raised {
        for step in 1..=2 {
            let at = since.0 + step * cycle;
            for offset in [-1, 0, 1, 375, 750, 1125] {
                moments.push(Millis(at.saturating_add_signed(offset)));
            }
        }
    }
    moments.retain(|&moment| moment >= update && moment < until);
    moments.sort_unstable();
    moments.dedup();
    moments
}

/// The times of the updates through the `last` whose scenes carried an event.
fn cued(history: &History, times: &[Millis; 3], last: usize) -> Vec<Millis> {
    (0..=last)
        .filter(|&i| history.scenes[i].0.event.is_some())
        .map(|i| times[i])
        .collect()
}

/// Plays a history into a composer, checking every constraint on the way; returns the time of
/// each update.
fn play(composer: &mut Composer, history: &History) -> Result<[Millis; 3], TestCaseError> {
    let mut times = [Millis(0); 3];
    let mut now = Millis(500);
    for (i, (scene, overlay)) in history.scenes.iter().enumerate() {
        if i > 0 {
            now = match history.gaps[i - 1] {
                After::Ms(ms) => now.plus(ms),
                After::Settled => {
                    let (scene, overlay) = &history.scenes[i - 1];
                    check_settled(composer, scene, overlay)?;
                    composer.settled_at().max(now)
                }
            };
        }
        for mix in [0, 255] {
            let before = drawn(composer, mix, now);
            let mut updated = composer.clone();
            updated.update(scene, overlay, now);
            let after = drawn(&updated, mix, now);
            prop_assert_eq!(plain(&before), plain(&after), "the update at {:?} cut", now);
            // Nor does it move the palette or the dimming at that moment.
            prop_assert_eq!(composer.palette_mix(now), updated.palette_mix(now));
            prop_assert_eq!(composer.dim(now), updated.dim(now));
        }
        composer.update(scene, overlay, now);
        check_placement(composer, scene)?;
        times[i] = now;
    }
    Ok(times)
}

proptest! {
    #![proptest_config(config(2000))]

    #[test]
    fn every_constraint_holds_through_generated_histories(
        history in history(),
        salt in salt(),
    ) {
        let mut composer = Composer::new(table(), salt);
        let times = play(&mut composer, &history)?;
        let (last, overlay) = &history.scenes[2];
        check_settled(&composer, last, overlay)?;

        // Replay, stopping at the moments around each update, for continuity and the subset,
        // by day or by night as the salt falls.
        let mix = if salt.bytes()[0] % 2 == 0 { 0 } else { 255 };
        let mut replay = Composer::new(table(), salt);
        let mut raised = Vec::new();
        for (i, (scene, overlay)) in history.scenes.iter().enumerate() {
            replay.update(scene, overlay, times[i]);
            if i == 0 || history.scenes[i - 1].1.label != overlay.label {
                raised.push(times[i]);
            }
            let until = times.get(i + 1).copied().unwrap_or_else(|| replay.settled_at().plus(1));
            let showing: Vec<&SceneInput> = history.scenes[..=i].iter().map(|(s, _)| s).collect();
            let cued = cued(&history, &times, i);
            let mut previous: Option<(Millis, Vec<KeyedDraw>)> = None;
            for moment in moments(times[i], until, &raised, &cued) {
                let draws = drawn(&replay, mix, moment);
                check_subset(&draws, &showing)?;
                check_one_show(&draws)?;
                if let Some((then, before)) = &previous {
                    check_continuity(before, &draws, moment.0 - then.0).map_err(|err| {
                        TestCaseError::fail(format!("{err} from {then:?} to {moment:?}; updates at {times:?}"))
                    })?;
                }
                previous = Some((moment, draws));
            }
        }
    }

    #[test]
    fn the_salt_moves_slots_and_nothing_else(
        history in history(),
        salts in (salt(), salt()),
    ) {
        let mut one = Composer::new(table(), salts.0);
        let mut two = Composer::new(table(), salts.1);
        let times = play(&mut one, &history)?;
        play(&mut two, &history)?;
        prop_assert_eq!(one.placement(), two.placement());
        prop_assert_eq!(one.monument_spots(), two.monument_spots());
        prop_assert_eq!(one.fossil_spots(), two.fossil_spots());
        let end = one.settled_at().max(two.settled_at());
        let cued = cued(&history, &times, 2);
        for moment in moments(times[2], end.plus(1), &[], &cued).into_iter().chain([end]) {
            prop_assert_eq!(one.depicted(moment), two.depicted(moment));
        }
    }
}

/// Where two frames first differ, for a failure's message.
fn difference(a: &Frame, b: &Frame) -> String {
    let name = |draw: Option<&Draw>| {
        draw.map(|draw| format!("{} {:?}", art().catalog.sprite(draw.sprite).id, draw))
    };
    if (a.palette_mix, a.dim) != (b.palette_mix, b.dim) {
        return format!(
            "mix and dim {:?} → {:?}",
            (a.palette_mix, a.dim),
            (b.palette_mix, b.dim)
        );
    }
    let at = (0..a.draws.len().max(b.draws.len())).find(|&i| a.draws.get(i) != b.draws.get(i));
    at.map_or_else(String::new, |i| {
        format!("{:?} → {:?}", name(a.draws.get(i)), name(b.draws.get(i)))
    })
}

/// The whole frame at `now`, as the app draws it: in the composer's own palette and dim.
fn live_frame(composer: &Composer, scenery: &Scenery, now: Millis) -> Frame {
    composer.frame(art(), table(), scenery, composer.palette_mix(now), now)
}

proptest! {
    #![proptest_config(config(300))]

    /// I-10: the composer names every moment its frame may change, so the app can sleep until
    /// then. Between a moment and the next change it names, the frame holds, in the scene's
    /// width and in wider viewports, where the clouds and the sweep cross more columns.
    #[test]
    fn the_frame_holds_until_the_next_change(
        history in history(),
        salt in salt(),
        columns in prop::sample::select(vec![640_u32, 651, 735, 840]),
    ) {
        let mut composer = Composer::new(table(), salt);
        composer.set_viewport(Viewport::new(columns));
        let scenery = composer.scenery(art(), table());
        let mut now = Millis(500);
        for (i, (scene, overlay)) in history.scenes.iter().enumerate() {
            if i > 0 {
                now = match history.gaps[i - 1] {
                    After::Ms(ms) => now.plus(ms),
                    After::Settled => composer.settled_at().max(now),
                };
            }
            composer.update(scene, overlay, now);
            // From the update, mid-tween, once settled and past a step of the label's cycle,
            // walk the changes the composer names, checking each span.
            let settled = composer.settled_at().max(now);
            // And into the first turn this scene moves in, where gusts and walks begin.
            let moving = Millis((now.0 / TURN_MS + 1) * TURN_MS);
            let starts = [
                now,
                now.plus(1500),
                settled,
                settled.plus(LABEL_CYCLE_MS + 700),
                moving.plus(900),
                moving.plus(9900),
            ];
            // And through an event's choreography, where it turns, the front's crossing
            // ending by this viewport's width.
            let across = (u64::from(columns) + 24) * FRONT_MS_PER_PX;
            let cues = scene.event.iter().flat_map(|_| {
                CUES.iter()
                    .chain(&[across - 1, across, across + 1])
                    .flat_map(|&ms| [ms, ms + u64::from(SKY_MS)])
                    .map(|ms| Millis(now.0 + ms - 40))
                    .collect::<Vec<_>>()
            });
            for start in starts.into_iter().chain(cues) {
                let mut moment = start;
                for _ in 0..12 {
                    let Some(next) = composer.next_change(art(), table(), moment) else {
                        break;
                    };
                    prop_assert!(next > moment, "{:?} named {:?}", moment, next);
                    let frame = live_frame(&composer, &scenery, moment);
                    for inside in [moment.0 + 1, moment.0 + (next.0 - moment.0) / 2, next.0 - 1] {
                        if inside > moment.0 && inside < next.0 {
                            let later = live_frame(&composer, &scenery, Millis(inside));
                            prop_assert!(
                                later == frame,
                                "changed at {} before {:?}: {}",
                                inside,
                                next,
                                difference(&frame, &later)
                            );
                        }
                    }
                    moment = next;
                }
            }
        }
    }
}

/// The motion budget, as the art proposes it: at most three residents travelling at once, at
/// most one of them flying or signalling; at most six plants leaning, none in still air (a
/// storm sways them a step harder, and a windstorm at a gale); and in dim mode, travel and sway
/// only in every fourth turn.
const MOST_TRAVELLING: usize = 3;
const MOST_AIRBORNE: usize = 1;
const MOST_SWAYING: usize = 6;

/// Whether the plants stand in still air: a calm that no storm or windstorm stirs.
fn still(scene: &SceneInput) -> bool {
    let stirred = matches!(
        scene.event.map(|event| event.kind),
        Some(EventKind::Storm | EventKind::Windstorm)
    );
    scene.wind == Wind::Calm && !stirred
}

/// How many residents travel at a moment, and how many of them fly or signal; and how many
/// plants lean.
fn motion_at(draws: &[KeyedDraw]) -> (usize, usize, usize) {
    let mut travelling = BTreeSet::new();
    let mut airborne = BTreeSet::new();
    let mut leaning = BTreeSet::new();
    for keyed in draws {
        match keyed.key.subject {
            Subject::Resident { id, .. } if keyed.key.part == 0 => {
                let routine = art().catalog.sprite(keyed.draw.sprite).keys.state.clone();
                match routine.as_deref() {
                    Some("move") => {
                        travelling.insert(id);
                    }
                    Some("takeoff" | "flight" | "land" | "hop" | "signal") => {
                        travelling.insert(id);
                        airborne.insert(id);
                    }
                    _ => {}
                }
            }
            Subject::Plant { id, .. } | Subject::Scorch { id, .. }
                if keyed.draw.shear.slope != 0 =>
            {
                leaning.insert(id);
            }
            _ => {}
        }
    }
    (travelling.len(), airborne.len(), leaning.len())
}

proptest! {
    #![proptest_config(config(300))]

    #[test]
    fn the_motion_budget_holds(history in history(), salt in salt()) {
        let mut composer = Composer::new(table(), salt);
        let times = play(&mut composer, &history)?;
        // Then everyone livened: as much travel as the scene's residents could want, by day
        // or by night as the salt falls, so the caps are pressed.
        let (scene, overlay) = &history.scenes[2];
        let mut scene = scene.clone();
        for resident in &mut scene.residents {
            resident.energy = Energy::High;
        }
        if scene.presentation.state != PresentationState::Dormant {
            scene.presentation.state = PresentationState::Thriving;
        }
        if scene.weather == Weather::Storm {
            scene.weather = Weather::Clear;
        }
        let hours = if salt.bytes()[1] % 2 == 0 { 0 } else { 12 * HOUR };
        let overlay = OverlayInput {
            clock: LocalClock::new(overlay.clock.date, hours),
            ..overlay.clone()
        };
        let livened = times[2].plus(1);
        composer.update(&scene, &overlay, livened);
        let scene = &scene;
        let overlay = &overlay;
        // Three turns after the update stands, every frame duration apart.
        let from = (livened.0 / TURN_MS + 1) * TURN_MS;
        for ms in (from..from + 3 * TURN_MS).step_by(167) {
            let draws = drawn(&composer, 255, Millis(ms));
            let (travelling, airborne, leaning) = motion_at(&draws);
            prop_assert!(travelling <= MOST_TRAVELLING, "{} travel at {}", travelling, ms);
            prop_assert!(airborne <= MOST_AIRBORNE, "{} fly at {}", airborne, ms);
            prop_assert!(leaning <= MOST_SWAYING, "{} lean at {}", leaning, ms);
            if still(scene) {
                prop_assert_eq!(leaning, 0, "still air at {}", ms);
            }
            let fourth = (ms / TURN_MS).is_multiple_of(4);
            if overlay.dim && !fourth {
                prop_assert_eq!((travelling, leaning), (0, 0), "dimmed, at {}", ms);
            }
            if scene.presentation.state == PresentationState::Dormant {
                prop_assert_eq!(travelling, 0, "dormant, at {}", ms);
            }
        }
    }

    /// I-2 for motion: what every resident does in every turn is the same on every install.
    #[test]
    fn the_salt_never_times_motion(history in history(), salts in (salt(), salt())) {
        let mut one = Composer::new(table(), salts.0);
        let mut two = Composer::new(table(), salts.1);
        let times = play(&mut one, &history)?;
        play(&mut two, &history)?;
        let last = times[2].0 / TURN_MS;
        for w in last..last + 3 {
            let (a, b) = (one.turn_plans(art(), table(), w), two.turn_plans(art(), table(), w));
            let differ = a.iter().zip(&b).find(|(a, b)| a != b);
            prop_assert!(a.len() == b.len() && differ.is_none(), "turn {}: {:?}", w, differ);
        }
    }
}

// ---- The rendered frames -------------------------------------------------------------------

const WIDTH: i32 = NATIVE_WIDTH.cast_signed();
const HEIGHT: i32 = NATIVE_HEIGHT.cast_signed();
/// The soil band, its top edge included, inside the glass edges.
const SOIL_ROWS: std::ops::Range<i32> = 65..HEIGHT;
const INSIDE: std::ops::Range<i32> = 6..WIDTH - 6;
/// The values constraint 1 keeps apart by day, in luma out of 255.
const MARGIN: u64 = 8;

/// What a pixel shows once every opaque draw over it has painted: the sky, a plant, or
/// something else. The sky is the sky's pieces and the backdrop's open sky, its top row's
/// colour; the backdrop's hills are something else. Translucent draws (glass, films, light,
/// glows) tint every class alike, so they never change it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shows {
    Sky,
    Plant,
    Other,
}

/// Calls `ink` with each opaque pixel of a draw inside the frame, after flip, shear and clip,
/// as the compositor places them, and its atlas byte.
fn coverage(draw: &Draw, mut ink: impl FnMut(i32, i32, u8)) {
    let width = i32::from(art().catalog.sprite(draw.sprite).rect.w);
    for (row, pixels) in (0_i32..).zip(art().rows(draw.sprite)) {
        let y = draw.y + row;
        let x0 = draw.x + draw.shear.offset(row);
        for (column, &index) in (0_i32..).zip(pixels) {
            let x = if draw.flip_x {
                x0 + width - 1 - column
            } else {
                x0 + column
            };
            let clipped = draw
                .clip
                .is_some_and(|c| x < c.x || y < c.y || x >= c.x + c.w || y >= c.y + c.h);
            if index != 0 && (0..WIDTH).contains(&x) && (0..HEIGHT).contains(&y) && !clipped {
                ink(x, y, index);
            }
        }
    }
}

fn opaque(draw: &Draw) -> bool {
    draw.opacity == u8::MAX && draw.blend == Blend::Normal
}

/// Each pixel's class, from the keyed draws and the scenery in depth order.
fn shows(draws: &[KeyedDraw], scenery: &[KeyedDraw]) -> Vec<Shows> {
    let mut all: Vec<&KeyedDraw> = draws.iter().chain(scenery).collect();
    all.sort_by_key(|keyed| keyed.key);
    let mut map = vec![Shows::Sky; (NATIVE_WIDTH * NATIVE_HEIGHT) as usize];
    for keyed in all {
        if !opaque(&keyed.draw) {
            continue;
        }
        let plant_part = keyed.key.part < PLANT_GLOW;
        let class = match keyed.key.subject {
            Subject::Plant { .. } | Subject::Fill { .. } if plant_part => Shows::Plant,
            _ if keyed.draw.layer == Layer::Sky => Shows::Sky,
            _ => Shows::Other,
        };
        let sprite = keyed.draw.sprite;
        let open_sky = art()
            .catalog
            .sprite(sprite)
            .flags
            .contains(Flags::BACKDROP)
            .then(|| {
                art()
                    .rows(sprite)
                    .next()
                    .and_then(|row| row.first().copied())
            })
            .flatten();
        coverage(&keyed.draw, |x, y, index| {
            let class = match open_sky {
                Some(sky) if index != sky => Shows::Other,
                _ => class,
            };
            map[usize::try_from(y * WIDTH + x).unwrap()] = class;
        });
    }
    map
}

fn luma([r, g, b, _]: Rgba) -> u64 {
    (299 * u64::from(r) + 587 * u64::from(g) + 114 * u64::from(b)) / 1000
}

/// The mean luma over the pixels `which` keeps, or `None` for none.
fn mean(image: &[Rgba], which: impl Fn(i32, i32) -> bool) -> Option<u64> {
    let (mut sum, mut n) = (0, 0);
    for (i, &pixel) in image.iter().enumerate() {
        let i = i32::try_from(i).unwrap();
        if which(i % WIDTH, i / WIDTH) {
            sum += luma(pixel);
            n += 1;
        }
    }
    (n > 0).then(|| sum / n)
}

proptest! {
    #![proptest_config(config(2000))]

    /// Constraint 1, measured on rendered frames: by day the soil band is darker than the
    /// plants, and the plants darker than the open sky but a storm's; at night the soil band
    /// stays the darkest.
    /// Constraint 4 on the pixels: with every translucent draw left out, each pixel is exactly
    /// a colour of its palette row.
    #[test]
    fn values_and_palette_hold_on_rendered_frames(
        (scene, overlay) in scene(),
        salt in salt(),
    ) {
        // Undimmed: dim mode darkens every value alike, which would only shrink the margins.
        let overlay = OverlayInput { dim: false, ..overlay };
        let mut composer = Composer::new(table(), salt);
        composer.update(&scene, &overlay, Millis(0));
        let now = composer.settled_at();
        let scenery = composer.scenery(art(), table());
        let map = shows(&drawn(&composer, 0, now), scenery.keyed());
        let class = |x: i32, y: i32| map[usize::try_from(y * WIDTH + x).unwrap()];
        let palette = &art().catalog.palette;
        for (mix, row) in [(0, &palette.day), (255, &palette.night)] {
            let frame = composer.frame(art(), table(), &scenery, mix, now);
            let image = composite(art(), &frame);
            let pixels = image.pixels();
            let soil = mean(pixels, |x, y| SOIL_ROWS.contains(&y) && INSIDE.contains(&x)).unwrap();
            let sky = mean(pixels, |x, y| y < SOIL_ROWS.start && class(x, y) == Shows::Sky);
            let plants = mean(pixels, |x, y| class(x, y) == Shows::Plant);
            let sky = sky.expect("some sky always shows");
            prop_assert!(soil + MARGIN <= sky, "soil {} against sky {} at {}", soil, sky, mix);
            if let Some(plants) = plants {
                prop_assert!(soil + MARGIN <= plants, "soil {} against plants {} at {}", soil, plants, mix);
                // A storm's sky may be as dark as the plants.
                if mix == 0 && scene.weather != Weather::Storm {
                    prop_assert!(plants + MARGIN <= sky, "plants {} against sky {}", plants, sky);
                }
            }
            let opaque_only = Frame {
                draws: frame.draws.iter().copied().filter(opaque).collect(),
                ..frame
            };
            let colours: BTreeSet<[u8; 3]> = row.iter().copied().collect();
            for pixel in composite(art(), &opaque_only).pixels() {
                prop_assert!(colours.contains(&[pixel[0], pixel[1], pixel[2]]), "{:?} at {}", pixel, mix);
            }
        }
    }
}

#[test]
fn the_generator_reaches_the_edges() {
    // The histories above cover what the constraints are hardest on: nothing at all, more
    // plants than the ceiling, a single species, trees alone, and a world full to every cap.
    let empty = scene_with(PresentationState::Dead, Vec::new());
    let mut crowd = scene_with(
        PresentationState::Thriving,
        (0..40)
            .map(|id| PlantView {
                id,
                species: POOL[usize::try_from(id).unwrap() % 8],
                stage: GrowthStage::Mature,
                state: PlantState::Thriving,
                bloom: true,
                scorched: false,
                order: 0,
            })
            .collect(),
    );
    crowd.weeds = 12;
    crowd.litter = 12;
    crowd.fungus = 9;
    crowd.monuments = (0..4)
        .map(|id| MonumentView {
            id,
            age: MonumentAge::Ancient,
            marks: 3,
        })
        .collect();
    crowd.fossils = (0..7)
        .map(|id| FossilView {
            id,
            form: FossilForm::Bone,
        })
        .collect();
    let overlay = OverlayInput::new(
        LINES.iter().map(|line| (*line).to_owned()).collect(),
        LocalClock::noon(preset::DATE),
    );
    for scene in [empty, crowd] {
        let mut composer = Composer::new(table(), InstallSalt::new([7; 16]));
        composer.update(&scene, &overlay, Millis(0));
        check_placement(&composer, &scene).unwrap();
        check_settled(&composer, &scene, &overlay).unwrap();
    }
}

/// The histories reach every choreography: in the sky the storm front, lightning, rain and a
/// windstorm's leaves; on the ground burn-off and its sprinkles, the scorch, the crack and the
/// sinkhole opening, and a flowering's blooms opening across a minute. Each plays in some of
/// them, so the properties above hold it to I-15 and I-10.
#[test]
fn the_histories_play_every_event() {
    use proptest::strategy::ValueTree;
    let mut runner = proptest::test_runner::TestRunner::new(config(300));
    let mut seen = BTreeSet::new();
    let blooms = |composer: &Composer, at: Millis| {
        drawn(composer, 0, at)
            .iter()
            .filter(|keyed| matches!(keyed.key.subject, Subject::Plant { .. }))
            .filter(|keyed| matches!(class(&keyed.draw), Class::Bloom(_)))
            .filter(|keyed| keyed.draw.opacity == 255)
            .count()
    };
    for _ in 0..300 {
        let history = history().new_tree(&mut runner).unwrap().current();
        let mut composer = Composer::new(table(), InstallSalt::new([3; 16]));
        let mut now = Millis(500);
        for (i, (scene, overlay)) in history.scenes.iter().enumerate() {
            if i > 0 {
                now = match history.gaps[i - 1] {
                    After::Ms(ms) => now.plus(ms),
                    After::Settled => composer.settled_at().max(now),
                };
            }
            composer.update(scene, overlay, now);
            for ms in [200, 1000, 2000, 5000, 20_000] {
                for keyed in drawn(&composer, 0, now.plus(ms)) {
                    let show = match keyed.key.subject {
                        Subject::Front => "front",
                        Subject::Bolt => "bolt",
                        Subject::Rain => "rain",
                        Subject::Leaf(_) => "leaves",
                        Subject::Burn(_) => "burn",
                        Subject::Sprinkle(_) => "sprinkles",
                        Subject::Scorch { .. } => "scorch",
                        Subject::Mark { .. } if opening(&keyed.draw) => family(&keyed.draw),
                        _ => continue,
                    };
                    seen.insert(show);
                }
            }
            // Only a flowering opens blooms that long after the update that brought them.
            if blooms(&composer, now.plus(65_000)) > blooms(&composer, now.plus(10_000)) {
                seen.insert("flowering");
            }
        }
    }
    assert_eq!(
        seen,
        BTreeSet::from([
            "bolt",
            "burn",
            "flowering",
            "front",
            "leaves",
            "rain",
            "scorch",
            "sprinkles",
            "tmp.ground.crack.",
            "tmp.ground.sinkhole."
        ])
    );
}
