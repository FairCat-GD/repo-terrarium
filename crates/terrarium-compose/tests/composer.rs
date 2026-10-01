//! The composer's constraints over generated histories: occupancy (constraint 2), silhouette
//! rhythm (constraint 3), transitions only (constraint 7, I-15), the healthy subset (I-7) and
//! salt independence (I-2).
//!
//! Each case is a history of three scenes, A, B and C, played into a composer at times chosen to
//! land on tween boundaries and either side of them. B and C are often edits of the scene before,
//! so ids carry over and change state, stage, bloom or species. The cases build draw lists
//! only; compositing pixels is the snapshots' job.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use terrarium_compose::catalog::{Art, Layer};
use terrarium_compose::composer::Composer;
use terrarium_compose::frame::Draw;
use terrarium_compose::order::{KeyedDraw, Subject};
use terrarium_compose::place::{CEILING, FLOOR};
use terrarium_compose::preset;
use terrarium_compose::salt::InstallSalt;
use terrarium_compose::slots::{Band, PATTERN, band_cells, screen_third};
use terrarium_compose::tween::Millis;
use terrarium_compose::tween::durations::{APPEAR_MS, RESTYLE_MS, SHORTEST_MS};
use terrarium_compose::variants::{ArtTable, art_key};
use terrarium_truth::PresentationState;
use terrarium_truth::scene::{GrowthStage, PlantSpecies, PlantState, PlantView, SceneInput};

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 2000,
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
    let field = |shift: u32, n: u64| (bits >> shift) % n;
    PlantView {
        id: u32::try_from(field(0, 48)).unwrap(),
        species: pick(species, field(8, 64)),
        stage: pick(GrowthStage::ALL, field(16, 3)),
        state: pick(PlantState::ALL, field(20, 5)),
        bloom: field(24, 2) == 1,
        scorched: false,
        order: if field(25, 2) == 0 {
            u16::try_from(field(26, 8)).unwrap()
        } else {
            u16::try_from(field(32, 1 << 16)).unwrap()
        },
    }
}

/// Keeps the first plant of each id, as `SceneInput::validate` requires.
fn unique(mut plants: Vec<PlantView>) -> Vec<PlantView> {
    let mut seen = std::collections::BTreeSet::new();
    plants.retain(|plant| seen.insert(plant.id));
    plants
}

fn scene_with(state: PresentationState, plants: Vec<PlantView>) -> SceneInput {
    preset::bare(state, unique(plants))
}

/// A valid scene of 0 to 40 plants.
fn scene() -> impl Strategy<Value = SceneInput> {
    (
        any::<(u8, u8, u16)>(),
        prop::collection::vec(any::<u64>(), 0..=40),
    )
        .prop_map(|((state, kind, mask), bits)| {
            let species = species(kind, mask);
            let plants = bits.into_iter().map(|b| plant(&species, b)).collect();
            scene_with(pick(PresentationState::ALL, u64::from(state)), plants)
        })
}

/// The scene after another: a fresh one, the same again, or the old one edited plant by
/// plant, with newcomers from the pool and perhaps a new presentation state.
#[derive(Clone, Debug)]
enum Next {
    Fresh(SceneInput),
    Edited {
        edits: Vec<u32>,
        added: Vec<u64>,
        state: Option<u8>,
    },
    Same,
}

fn next() -> impl Strategy<Value = Next> {
    prop_oneof![
        2 => scene().prop_map(Next::Fresh),
        5 => (
            prop::collection::vec(any::<u32>(), 40),
            prop::collection::vec(any::<u64>(), 0..6),
            any::<Option<u8>>(),
        )
            .prop_map(|(edits, added, state)| Next::Edited { edits, added, state }),
        1 => Just(Next::Same),
    ]
}

impl Next {
    fn after(&self, before: &SceneInput) -> SceneInput {
        match self {
            Next::Fresh(scene) => scene.clone(),
            Next::Same => before.clone(),
            Next::Edited {
                edits,
                added,
                state,
            } => {
                // Each edit keeps the plant (most often), drops it, or changes its state, stage,
                // bloom or species.
                let mut plants: Vec<PlantView> = before
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
                let state = state.map_or(before.presentation.state, |s| {
                    pick(PresentationState::ALL, u64::from(s))
                });
                scene_with(state, plants)
            }
        }
    }
}

/// When the next scene arrives after the one before: on a tween's boundary or either side of
/// it, or once everything has settled.
#[derive(Clone, Copy, Debug)]
enum After {
    Ms(u32),
    Settled,
}

fn after() -> impl Strategy<Value = After> {
    let half = RESTYLE_MS / 2;
    let gaps = [1, half - 1, half, half + 1, RESTYLE_MS - 1, APPEAR_MS];
    (0_usize..8).prop_map(move |i| gaps.get(i).map_or(After::Settled, |&ms| After::Ms(ms)))
}

fn salt() -> impl Strategy<Value = InstallSalt> {
    any::<[u8; 16]>().prop_map(InstallSalt::new)
}

#[derive(Clone, Debug)]
struct History {
    scenes: [SceneInput; 3],
    gaps: [After; 2],
}

fn history() -> impl Strategy<Value = History> {
    (scene(), next(), next(), after(), after()).prop_map(|(a, b, c, ab, bc)| {
        let b = b.after(&a);
        let c = c.after(&b);
        for scene in [&a, &b, &c] {
            assert_eq!(scene.validate(), Ok(()), "the generator makes valid scenes");
        }
        History {
            scenes: [a, b, c],
            gaps: [ab, bc],
        }
    })
}

// ---- The checks ----------------------------------------------------------------------------

fn drawn(composer: &Composer, now: Millis) -> Vec<KeyedDraw> {
    composer.draws(art(), table(), now)
}

fn plain(keyed: &[KeyedDraw]) -> Vec<Draw> {
    keyed.iter().map(|keyed| keyed.draw).collect()
}

fn band(species: PlantSpecies) -> Band {
    Band::from_layer(table().plant_layer(species)).expect("a plant layer")
}

/// Constraints 2 and 3 on the placement, which every frame settles to.
fn check_placement(composer: &Composer, input: &SceneInput) -> Result<(), TestCaseError> {
    let placement = composer.placement();
    let occupied = placement.occupied();
    prop_assert!(
        (FLOOR..=CEILING).contains(&occupied),
        "occupancy {occupied}"
    );
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

/// I-7: the living looks drawn, thriving, pale or wilting, and the blooms, never outnumber
/// those of the scenes that could be showing.
fn check_subset(draws: &[KeyedDraw], scenes: &[&SceneInput]) -> Result<(), TestCaseError> {
    let mut allowed: BTreeMap<(PlantSpecies, GrowthStage, PlantState), usize> = BTreeMap::new();
    let mut blooms: BTreeMap<PlantSpecies, usize> = BTreeMap::new();
    for scene in scenes {
        for plant in &scene.plants {
            *allowed
                .entry((plant.species, plant.stage, plant.state))
                .or_default() += 1;
            // Written out rather than taken from the crate, so a broken rule cannot agree with
            // itself.
            if plant.bloom
                && plant.stage == GrowthStage::Mature
                && plant.state == PlantState::Thriving
            {
                *blooms.entry(plant.species).or_default() += 1;
            }
        }
    }
    let mut seen: BTreeMap<(PlantSpecies, GrowthStage, PlantState), usize> = BTreeMap::new();
    let mut seen_blooms: BTreeMap<PlantSpecies, usize> = BTreeMap::new();
    for keyed in draws {
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

/// Once settled: exactly the placement is drawn, fully; the healthy subset is exact; and each
/// third of the strip holds at most two mature trees, never two of one species.
fn check_settled(composer: &Composer, input: &SceneInput) -> Result<(), TestCaseError> {
    let now = composer.settled_at();
    let draws = drawn(composer, now);
    prop_assert!(draws.iter().all(|keyed| keyed.draw.opacity == u8::MAX));
    let subjects: std::collections::BTreeSet<Subject> =
        draws.iter().map(|keyed| keyed.key.subject).collect();
    let placement = composer.placement();
    let placed: std::collections::BTreeSet<Subject> = placement
        .plants
        .iter()
        .map(|(&id, placed)| Subject::Plant {
            id,
            cell: placed.cell,
        })
        .chain(placement.fill.keys().map(|&cell| Subject::Fill { cell }))
        .collect();
    prop_assert_eq!(subjects, placed);
    check_subset(&draws, &[input])?;
    let mut trees: BTreeMap<i32, Vec<PlantSpecies>> = BTreeMap::new();
    for keyed in &draws {
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
    Ok(())
}

/// I-15 between two moments with no update between them: no draw jumps further than the
/// fastest tween can move, and none moves.
fn check_continuity(
    before: &[KeyedDraw],
    after: &[KeyedDraw],
    elapsed: u64,
) -> Result<(), TestCaseError> {
    // Smoothstep's steepest slope is 1.5 times the average.
    let bound = 255 * 3 * elapsed / (2 * u64::from(SHORTEST_MS)) + 1;
    let index = |draws: &[KeyedDraw]| -> BTreeMap<(Subject, u8, u16), Draw> {
        draws
            .iter()
            .map(|keyed| {
                (
                    (keyed.key.subject, keyed.key.part, keyed.key.look),
                    keyed.draw,
                )
            })
            .collect()
    };
    let (old, new) = (index(before), index(after));
    for key in old.keys().chain(new.keys()) {
        let [a, b] = [old.get(key), new.get(key)];
        let opacity = |draw: Option<&Draw>| u64::from(draw.map_or(0, |d| d.opacity));
        prop_assert!(opacity(a).abs_diff(opacity(b)) <= bound, "{:?} jumped", key);
        if let (Some(a), Some(b)) = (a, b) {
            prop_assert_eq!(
                (a.sprite, a.x, a.y, a.layer),
                (b.sprite, b.x, b.y, b.layer),
                "{:?} changed",
                key
            );
        }
    }
    Ok(())
}

/// Moments worth looking at after an update: its tween boundaries and either side of them.
fn moments(update: Millis, until: Millis) -> Vec<Millis> {
    let half = RESTYLE_MS / 2;
    // Quarter steps keep the continuity bound near 96 of 255, so a cut mid-tween shows; the
    // boundaries and their neighbours catch a cut at a phase change.
    let mut moments: Vec<Millis> = [
        0,
        1,
        750,
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
    .filter(|&moment| moment < until)
    .collect();
    moments.dedup();
    moments
}

/// Plays a history into a composer, checking every constraint on the way; returns the time of
/// each update.
fn play(composer: &mut Composer, history: &History) -> Result<[Millis; 3], TestCaseError> {
    let mut times = [Millis(0); 3];
    let mut now = Millis(500);
    for (i, scene) in history.scenes.iter().enumerate() {
        if i > 0 {
            now = match history.gaps[i - 1] {
                After::Ms(ms) => now.plus(ms),
                After::Settled => {
                    check_settled(composer, &history.scenes[i - 1])?;
                    composer.settled_at().max(now)
                }
            };
        }
        let before = drawn(composer, now);
        composer.update(scene, now);
        let after = drawn(composer, now);
        prop_assert_eq!(plain(&before), plain(&after), "the update at {:?} cut", now);
        check_placement(composer, scene)?;
        times[i] = now;
    }
    Ok(times)
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn every_constraint_holds_through_generated_histories(
        history in history(),
        salt in salt(),
    ) {
        let mut composer = Composer::new(table(), salt);
        let times = play(&mut composer, &history)?;
        let last = &history.scenes[2];
        check_settled(&composer, last)?;

        // Replay, stopping at the moments around each update, for continuity and the subset.
        let mut replay = Composer::new(table(), salt);
        for (i, scene) in history.scenes.iter().enumerate() {
            replay.update(scene, times[i]);
            let until = times.get(i + 1).copied().unwrap_or_else(|| replay.settled_at().plus(1));
            let showing: Vec<&SceneInput> = history.scenes[..=i].iter().collect();
            let mut previous: Option<(Millis, Vec<KeyedDraw>)> = None;
            for moment in moments(times[i], until) {
                let draws = drawn(&replay, moment);
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
        let end = one.settled_at().max(two.settled_at());
        for moment in moments(times[2], end.plus(1)).into_iter().chain([end]) {
            prop_assert_eq!(one.depicted(moment), two.depicted(moment));
        }
    }
}

#[test]
fn the_generator_reaches_the_edges() {
    // The histories above cover what the constraints are hardest on: nothing at all, more
    // plants than the ceiling, a single species, and trees alone.
    let empty = scene_with(PresentationState::Dead, Vec::new());
    let crowd = scene_with(
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
    for scene in [empty, crowd] {
        let mut composer = Composer::new(table(), InstallSalt::new([7; 16]));
        composer.update(&scene, Millis(0));
        check_placement(&composer, &scene).unwrap();
        check_settled(&composer, &scene).unwrap();
    }
}
