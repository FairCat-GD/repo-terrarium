//! The composer's constraints over generated histories: occupancy (constraint 2), silhouette
//! rhythm (constraint 3), palette discipline (constraint 4, on the draws), one message
//! (constraint 6), transitions only (constraint 7, I-15), the healthy subset (I-7) and salt
//! independence (I-2). A second property renders settled frames by day and by night for the
//! value structure (constraint 1) and palette discipline on the pixels (constraint 4).
//!
//! Each case is a history of three scenes, A, B and C, each with its overlay, played into a
//! composer at times chosen to land on tween boundaries, either side of them, and past a step
//! of the label's cycle. B and C are often edits of the scene before, so ids carry over and
//! change state, stage, bloom or species, and the world's weather, cover, glass, monuments and
//! fossils change by small steps. The rules the checks hold the composer to are written out
//! here, never taken from the crate, so a broken rule cannot agree with itself.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use terrarium_compose::catalog::{Art, Blend, Flags, Layer};
use terrarium_compose::composer::{Composer, OverlayInput};
use terrarium_compose::compositor::composite;
use terrarium_compose::frame::{Draw, Frame, NATIVE_HEIGHT, NATIVE_WIDTH, Rgba};
use terrarium_compose::order::{KeyedDraw, Subject};
use terrarium_compose::palette::Slot;
use terrarium_compose::place::{CEILING, FLOOR};
use terrarium_compose::preset;
use terrarium_compose::salt::InstallSalt;
use terrarium_compose::sky::{LocalDate, MoonPhase, moon_phase};
use terrarium_compose::slots::{Band, PATTERN, band_cells, screen_third};
use terrarium_compose::tween::Millis;
use terrarium_compose::tween::durations::{
    APPEAR_MS, LABEL_CYCLE_MS, LABEL_FADE_MS, RESTYLE_MS, SHORTEST_MS,
};
use terrarium_compose::variants::{ArtTable, art_key};
use terrarium_truth::PresentationState;
use terrarium_truth::scene::{
    FossilForm, FossilView, GrowthStage, Light, MonumentAge, MonumentView, PlantSpecies,
    PlantState, PlantView, SceneInput, Weather,
};

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
/// values, so they repeat, or any value.
fn plant(species: &[PlantSpecies], bits: u64) -> PlantView {
    PlantView {
        id: u32::try_from(field(bits, 0, 48)).unwrap(),
        species: pick(species, field(bits, 8, 64)),
        stage: pick(GrowthStage::ALL, field(bits, 16, 3)),
        state: pick(PlantState::ALL, field(bits, 20, 5)),
        bloom: field(bits, 24, 2) == 1,
        scorched: false,
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

fn fossil(bits: u64) -> FossilView {
    FossilView {
        id: u32::try_from(field(bits, 0, 10)).unwrap(),
        form: pick(FossilForm::ALL, field(bits, 4, 3)),
    }
}

/// The world of a scene from two words: its sky, cover, glass, monuments and fossils.
fn dress(scene: &mut SceneInput, world: u64, things: u64) {
    scene.weather = pick(Weather::ALL, field(world, 0, 4));
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

fn overlay(bits: u64) -> OverlayInput {
    let lines = usize::try_from(field(bits, 0, 4)).unwrap();
    OverlayInput {
        label: (0..lines)
            .map(|i| pick(&LINES, bits >> (2 + 3 * i)).to_owned())
            .collect(),
        date: LocalDate::new(
            2020 + i32::try_from(field(bits, 16, 11)).unwrap(),
            1 + small(bits, 24, 12),
            1 + small(bits, 32, 28),
        ),
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
        )
            .prop_map(|(edits, added, state, world, overlay)| Next::Edited {
                edits,
                added,
                state,
                world,
                overlay,
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
    ];
    (0_usize..9).prop_map(move |i| gaps.get(i).map_or(After::Settled, |&ms| After::Ms(ms)))
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
/// in the label's ink; nothing is dimmed.
fn check_palette_discipline(frame: &Frame) -> Result<(), TestCaseError> {
    prop_assert_eq!(frame.dim, 0);
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

/// The clouds each weather brings: clear, overcast, storm, haze.
fn weather_clouds(weather: Weather) -> usize {
    match weather {
        Weather::Clear => 3,
        Weather::Overcast => 4,
        Weather::Storm => 0,
        Weather::Haze => 2,
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
    let clouds = weather_clouds(weather).min(quiet.clouds);
    prop_assert_eq!(count(day, |s| matches!(s, Subject::Cloud(_))), clouds);
    prop_assert_eq!(
        count(day, |s| s == Subject::Sun),
        usize::from(lets_bodies && quiet.sun)
    );
    let moon = lets_bodies && moon_phase(overlay.date) != MoonPhase::New;
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
    Ok(())
}

/// The opacity a settled draw shows at: its sprite's own, gone by day if it shows only at
/// night and by night if only by day.
fn settled_opacity(draw: &Draw, mix: u8) -> u8 {
    let sprite = art().catalog.sprite(draw.sprite);
    let scale = |a: u8, b: u8| u8::try_from((u16::from(a) * u16::from(b) + 127) / 255).unwrap();
    if sprite.flags.contains(Flags::NIGHT_ONLY) {
        scale(sprite.opacity, mix)
    } else if sprite.flags.contains(Flags::DAY_ONLY) {
        scale(sprite.opacity, 255 - mix)
    } else {
        sprite.opacity
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
            // The label cycles past the moment it settled.
            if keyed.key.subject != Subject::Label {
                prop_assert_eq!(
                    keyed.draw.opacity,
                    settled_opacity(&keyed.draw, mix),
                    "{:?} at {}",
                    keyed.key,
                    mix
                );
            }
        }
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
    let scenery = composer.scenery(art(), table());
    for mix in [0, 255] {
        check_palette_discipline(&composer.frame(art(), table(), &scenery, mix, now))?;
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

/// I-15 between two moments with no update between them: no draw's opacity jumps further
/// than its fastest tween can move it, a cloud drifts no faster than its pace and wraps only
/// out of sight, and nothing else moves.
fn check_continuity(
    before: &[KeyedDraw],
    after: &[KeyedDraw],
    elapsed: u64,
) -> Result<(), TestCaseError> {
    let index = |draws: &[KeyedDraw]| -> BTreeMap<(Subject, u8, u16, u16), Draw> {
        draws
            .iter()
            .map(|keyed| {
                let key = keyed.key;
                ((key.subject, key.part, key.look, key.item), keyed.draw)
            })
            .collect()
    };
    let (old, new) = (index(before), index(after));
    for key in old.keys().chain(new.keys()) {
        let [a, b] = [old.get(key), new.get(key)];
        let own = u64::from(art().catalog.sprite(a.or(b).unwrap().sprite).opacity);
        // Smoothstep's steepest slope is 1.5 times the average.
        let bound = own * 3 * elapsed / (2 * fastest_ms(key.0)) + 1;
        let opacity = |draw: Option<&Draw>| u64::from(draw.map_or(0, |d| d.opacity));
        prop_assert!(opacity(a).abs_diff(opacity(b)) <= bound, "{:?} jumped", key);
        if let (Some(a), Some(b)) = (a, b) {
            prop_assert_eq!(
                (a.sprite, a.y, a.layer),
                (b.sprite, b.y, b.layer),
                "{:?} changed",
                key
            );
            if let Subject::Cloud(_) = key.0 {
                // Forward along its path, which wraps out of sight (the sky module's tests hold
                // that), no further than its pace allows.
                let drift = u64::try_from((b.x - a.x).rem_euclid(CLOUD_PATH)).unwrap();
                prop_assert!(
                    drift <= elapsed / CLOUD_MS_PER_PX + 1,
                    "{:?} drifted from {} to {}",
                    key,
                    a.x,
                    b.x
                );
            } else {
                prop_assert_eq!(a.x, b.x, "{:?} moved", key);
            }
        }
    }
    Ok(())
}

/// Moments worth looking at after an update: its tween boundaries and either side of them,
/// and the label's cycle steps from any label set raised at `raised`.
fn moments(update: Millis, until: Millis, raised: &[Millis]) -> Vec<Millis> {
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
            let mut previous: Option<(Millis, Vec<KeyedDraw>)> = None;
            for moment in moments(times[i], until, &raised) {
                let draws = drawn(&replay, mix, moment);
                check_subset(&draws, &showing)?;
                if let Some((then, before)) = &previous {
                    check_continuity(before, &draws, moment.0 - then.0)?;
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
        for moment in moments(times[2], end.plus(1), &[]).into_iter().chain([end]) {
            prop_assert_eq!(one.depicted(moment), two.depicted(moment));
        }
    }
}

// ---- The rendered frames -------------------------------------------------------------------

const WIDTH: i32 = NATIVE_WIDTH.cast_signed();
const HEIGHT: i32 = NATIVE_HEIGHT.cast_signed();
/// The soil band, inside the glass edges.
const SOIL_ROWS: std::ops::Range<i32> = 66..HEIGHT;
const INSIDE: std::ops::Range<i32> = 6..WIDTH - 6;
/// The values constraint 1 keeps apart by day, in luma out of 255.
const MARGIN: u64 = 8;

/// What a pixel shows once every opaque draw over it has painted: the sky, a plant, or
/// something else. Translucent draws (glass, films, light, glows) tint every class alike, so
/// they never change it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shows {
    Sky,
    Plant,
    Other,
}

/// Calls `ink` with each opaque pixel of a draw inside the frame, after flip, shear and clip,
/// as the compositor places them.
fn coverage(draw: &Draw, mut ink: impl FnMut(i32, i32)) {
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
                ink(x, y);
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
        coverage(&keyed.draw, |x, y| {
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
    /// plants, and the plants darker than the sky; at night the soil band stays the darkest.
    /// Constraint 4 on the pixels: with every translucent draw left out, each pixel is exactly
    /// a colour of its palette row.
    #[test]
    fn values_and_palette_hold_on_rendered_frames(
        (scene, overlay) in scene(),
        salt in salt(),
    ) {
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
                if mix == 0 {
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
    let overlay = OverlayInput {
        label: LINES.iter().map(|line| (*line).to_owned()).collect(),
        date: preset::DATE,
    };
    for scene in [empty, crowd] {
        let mut composer = Composer::new(table(), InstallSalt::new([7; 16]));
        composer.update(&scene, &overlay, Millis(0));
        check_placement(&composer, &scene).unwrap();
        check_settled(&composer, &scene, &overlay).unwrap();
    }
}
