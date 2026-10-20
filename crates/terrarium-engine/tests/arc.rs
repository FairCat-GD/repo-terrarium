//! The fixture arc, replayed through the headless pipeline: it reaches every presentation state,
//! its plants thrive, wilt, go pale, die and grow back, its residents arrive, turn over, leave
//! and return, a cached state resumes to the same result as a full replay, and neither the order
//! of an import nor the install salt changes anything.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use terrarium_adapters::fixture::{self, ARC_REPLAY_END_SECS, Phase};
use terrarium_adapters::git::{GitRepo, History};
use terrarium_engine::headless::{Request, daily_scenes, replay_dump};
use terrarium_store::{DataDir, NewTerrarium, RepoLocation, Store, StoredObservation};
use terrarium_truth::habitat::{plant_capacity, resident_capacity};
use terrarium_truth::replay::first_close;
use terrarium_truth::scene::{Energy, EventKind, LifeStage, PlantState, Strength};
use terrarium_truth::{
    Basis, Observation, Prepared, PresentationState, ReplayDump, ReplayError, RepoSeed, SimState,
    Tick, canonical_dump, observation_tick, project,
};

/// `lines`, each ended with LF.
fn joined<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let mut text = String::new();
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text
}

fn committed_observations() -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/arc/observations.jsonl");
    std::fs::read_to_string(path).expect("the committed observations")
}

/// A temporary directory holding the arc at `repo`.
struct ArcDir {
    dir: tempfile::TempDir,
}

impl ArcDir {
    fn new() -> ArcDir {
        let dir = tempfile::tempdir().expect("a temp dir");
        fixture::arc(&dir.path().join("repo")).expect("the arc");
        ArcDir { dir }
    }

    fn repo(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    /// A fresh data directory for each replay.
    fn data(&self, name: &str) -> DataDir {
        DataDir::at(self.dir.path().join(name))
    }

    /// The arc's history and its committed observations, as a replay reads them, with the
    /// basis and the tick the golden replay ends at.
    fn inputs(&self) -> (History, Vec<Observation>, Basis, Tick) {
        let git = GitRepo::open(&self.repo()).expect("opens");
        let history = git
            .read_history("main", &mut |_| ControlFlow::Continue(()))
            .expect("reads");
        let seed = RepoSeed::from_root_commit(&history.commits[0].id);
        let mut store = Store::open(&self.data("data")).expect("a store");
        let terrarium = store
            .create_terrarium(&NewTerrarium {
                seed,
                repo: RepoLocation {
                    path: self.repo().display().to_string(),
                    name: "arc".into(),
                    default_branch: "main".into(),
                },
                install_salt: [0; 16],
                connected_at: 0,
            })
            .expect("a terrarium");
        store
            .import_observations(terrarium, committed_observations().as_bytes())
            .expect("imports");
        let observations = store
            .observations(terrarium, None)
            .expect("reads")
            .iter()
            .map(|stored: &StoredObservation| stored.to_observation().expect("converts"))
            .collect();
        let basis = Basis::new(seed, history.grid, "main");
        let through = history
            .grid
            .tick_at(ARC_REPLAY_END_SECS)
            .expect("on the grid");
        (history, observations, basis, through)
    }

    fn dump(&self, name: &str, observations: &str, salt: u8) -> ReplayDump {
        let mut input: &[u8] = observations.as_bytes();
        replay_dump(Request {
            repo: &self.repo(),
            data: &self.data(name),
            observations: Some(&mut input as &mut dyn BufRead),
            at_secs: ARC_REPLAY_END_SECS,
            install_salt: [salt; 16],
        })
        .expect("replays")
    }
}

#[test]
fn the_arc_reaches_every_presentation_state() {
    let arc = ArcDir::new();
    let dump = arc.dump("data", &committed_observations(), 0);
    let reached: BTreeSet<PresentationState> = dump
        .state
        .timeline
        .entries
        .iter()
        .map(|entry| entry.state)
        .collect();
    assert_eq!(
        reached,
        PresentationState::ALL.iter().copied().collect(),
        "{:#?}",
        dump.state.timeline.entries
    );
    assert_eq!(dump.scene.presentation.state, PresentationState::Dormant);
    assert_eq!(dump.scene.condensation, 3);
    assert_eq!(dump.scene.validate(), Ok(()));
}

#[test]
fn the_day_by_day_replay_shows_every_state_and_keeps_the_golden_s_timeline() {
    let arc = ArcDir::new();
    let observations = committed_observations();
    let mut input: &[u8] = observations.as_bytes();
    let days = daily_scenes(Request {
        repo: &arc.repo(),
        data: &arc.data("days"),
        observations: Some(&mut input as &mut dyn BufRead),
        at_secs: ARC_REPLAY_END_SECS,
        install_salt: [0; 16],
    })
    .expect("replays");
    let dump = arc.dump("data", &committed_observations(), 0);
    let (last, _) = *days.days.last().expect("days");
    let timeline: Vec<_> = dump
        .state
        .timeline
        .entries
        .iter()
        .filter(|entry| entry.tick <= last)
        .copied()
        .collect();
    assert_eq!(days.timeline, timeline);
    // The scenes themselves show all six, each day's as its timeline has it.
    let shown: BTreeSet<PresentationState> = days
        .days
        .iter()
        .map(|(_, scene)| scene.presentation.state)
        .collect();
    assert_eq!(shown, PresentationState::ALL.iter().copied().collect());
    assert!(
        days.days
            .iter()
            .all(|(_, scene)| scene.validate() == Ok(()))
    );
    // Each event the story holds is played on some day, as the live view chooses it.
    let played: BTreeSet<EventKind> = days
        .days
        .iter()
        .filter_map(|(_, scene)| scene.event.map(|event| event.kind))
        .collect();
    assert!(played.contains(&EventKind::Sinkhole));
    assert!(played.contains(&EventKind::ClearingBurn));
    assert!(played.contains(&EventKind::Flowering));
}

#[test]
fn the_arcs_events_follow_its_story() {
    use EventKind::{
        Clearing, ClearingBurn, Dusk, Flowering, Lightning, RainAfterDrought, Sinkhole, Storm,
        Sunburst, Windstorm,
    };
    let arc = ArcDir::new();
    let dump = arc.dump("data", &committed_observations(), 0);
    let at = |kind: EventKind| -> Vec<u32> {
        dump.events
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| e.started_tick.hours())
            .collect()
    };
    // Tick 0 is day 0 at 09:00 UTC: a tick's day is (tick + 9) / 24.
    let days = |kind: EventKind| -> Vec<u32> { at(kind).iter().map(|t| (t + 9) / 24).collect() };
    // Releases: v0.1.0, v0.2.0 (lightweight, dated by its commit on day 123), v1.0.0, v2.0.0.
    assert_eq!(days(Flowering), [52, 123, 260, 705]);
    // CI turns red on day 301 and stays red to day 315: a storm, then lightning on the third
    // day and every third day after it, then the clearing. One more red run on day 357 storms
    // and clears.
    assert_eq!(days(Storm), [301, 357]);
    assert_eq!(days(Lightning), [304, 307, 310, 313]);
    assert!(at(Lightning).windows(2).all(|pair| pair[1] - pair[0] == 72));
    assert_eq!(days(Clearing), [315, 360]);
    // The legacy deletion opens a sinkhole naming its fossil, and its churn is a windstorm.
    assert_eq!(at(Sinkhole), [7_451]);
    let sinkhole = dump
        .events
        .iter()
        .find(|e| e.kind == Sinkhole)
        .expect("one");
    assert_eq!(
        sinkhole.fossil,
        dump.state.landmarks.fossils.first().map(|f| f.id)
    );
    assert!(at(Windstorm).contains(&7_455));
    // Coverage slips every week of the decline; the return is rain after a long drought, a
    // clearing of the dead code that piled up, and coverage leaping back.
    assert_eq!(days(Dusk).len(), 15);
    assert_eq!(at(RainAfterDrought), [13_706]);
    assert_eq!(
        dump.events
            .iter()
            .find(|e| e.kind == RainAfterDrought)
            .map(|e| e.strength),
        Some(Strength::High)
    );
    assert_eq!(days(ClearingBurn), [571, 641]);
    assert_eq!(days(Sunburst), [576]);
    // v1.0.0 waits for the first stone and is carved when it rises; v2.0.0 is carved on it at
    // once, the second stone rising a month later.
    let marks: Vec<u8> = dump.scene.monuments.iter().map(|m| m.marks).collect();
    assert_eq!(marks, [2, 0]);
    assert_eq!(dump.state.landmarks.pending, 0);
    // No switch of the weather in effect fired anything: the source moves from local to CI on
    // day 267, passes over an unknown run on day 385 and comes back on day 388.
    let fired: BTreeSet<u32> = dump
        .events
        .iter()
        .map(|e| (e.started_tick.hours() + 9) / 24)
        .collect();
    for day in [267, 385, 388] {
        assert!(!fired.contains(&day), "an event on day {day}");
    }
}

#[test]
fn side_branch_readings_change_no_health() {
    let arc = ArcDir::new();
    let all = committed_observations();
    let main_only = joined(
        all.lines()
            .filter(|line| !line.contains("\"at_default_tip\":false")),
    );
    assert_eq!(all.lines().count() - main_only.lines().count(), 2);
    let with = arc.dump("with", &all, 0);
    let without = arc.dump("without", &main_only, 0);
    assert_eq!(with.samples, without.samples);
    assert_eq!(with.state.daily, without.state.daily);
    assert_eq!(with.state.timeline, without.state.timeline);
}

#[test]
fn neither_import_order_nor_salt_changes_the_dump() {
    let arc = ArcDir::new();
    let committed = committed_observations();
    let mut lines: Vec<&str> = committed.lines().collect();
    lines[1..].reverse();
    let reversed = joined(lines.into_iter());
    let forward = canonical_dump(&arc.dump("forward", &committed, 0));
    assert_eq!(forward, canonical_dump(&arc.dump("reversed", &reversed, 0)));
    assert_eq!(
        forward,
        canonical_dump(&arc.dump("salted", &committed, 0xa5))
    );
}

/// In vegetation terms the arc reaches thriving, wilting (soil limits),
/// pale (light limits) and dead, in that order and before it recovers, and once it recovers a
/// new generation fills the capacity and thrives.
#[test]
fn the_arcs_plants_thrive_wilt_go_pale_die_and_grow_back() {
    let arc = ArcDir::new();
    let (history, observations, basis, through) = arc.inputs();
    let prepared = Prepared::new(basis.clone(), &history.commits, &observations).expect("valid");
    let recovering = history
        .grid
        .tick_at(Phase::Recovering.start_secs())
        .expect("on the grid");
    let mut state = SimState::new(basis.clone());
    let mut first: BTreeMap<PlantState, Tick> = BTreeMap::new();
    let mut regrown = None;
    let mut close = first_close(basis.grid);
    while close <= through {
        state.advance_through(&prepared, close).expect("replays");
        let scene = project(&state);
        for plant in &scene.plants {
            first.entry(plant.state).or_insert(close);
        }
        let plants = &state.vegetation.plants;
        let full = u32::try_from(plants.len()) == Ok(plant_capacity(state.habitat().terrain));
        let new_and_thriving = plants.iter().all(|plant| plant.born >= recovering)
            && scene.plants.iter().all(|p| p.state == PlantState::Thriving);
        if regrown.is_none() && close > recovering && full && new_and_thriving {
            regrown = Some(close);
        }
        close = close.saturating_add(24);
    }
    let at = |look| {
        first
            .get(&look)
            .copied()
            .unwrap_or_else(|| panic!("never {look:?}"))
    };
    assert!(
        at(PlantState::Thriving) < at(PlantState::Wilting),
        "{first:?}"
    );
    assert!(at(PlantState::Wilting) < at(PlantState::Pale), "{first:?}");
    assert!(at(PlantState::Pale) < at(PlantState::Dead), "{first:?}");
    assert!(at(PlantState::Dead) < recovering, "{first:?}");
    assert!(at(PlantState::Dormant) > recovering, "{first:?}");
    let regrown = regrown.expect("the plants grow back");
    // Within 60 days of the first commit back (N-11).
    assert!(regrown.hours_since(recovering) <= 60 * 24, "{regrown:?}");
}

/// In resident terms the arc's founders arrive once its plants are up, a generation turns over
/// before the silence, the cast goes hungry and leaves once the plants die (no decomposer is
/// left to live on the decay), returns within 60 days of the first commit back, and sleeps
/// through the dormancy unchanged. Two stones stand, and the legacy deletion lies as a fossil.
#[test]
fn the_arcs_residents_arrive_leave_and_return() {
    let arc = ArcDir::new();
    let (history, observations, basis, through) = arc.inputs();
    let prepared = Prepared::new(basis.clone(), &history.commits, &observations).expect("valid");
    let tick = |phase: Phase| {
        history
            .grid
            .tick_at(phase.start_secs())
            .expect("on the grid")
    };
    let (silence, recovering) = (tick(Phase::Dead), tick(Phase::Recovering));
    let mut state = SimState::new(basis.clone());
    let (mut arrived, mut hatched, mut died, mut bare, mut emptied, mut returned) =
        (None, None, None, None, None, None);
    let mut dormant: Option<Vec<u32>> = None;
    let mut close = first_close(basis.grid);
    while close <= through {
        state.advance_through(&prepared, close).expect("replays");
        let scene = project(&state);
        assert_eq!(scene.validate(), Ok(()));
        let mark = |step: &mut Option<Tick>, reached: bool| {
            if step.is_none() && reached {
                *step = Some(close);
            }
        };
        let residents = &state.fauna.residents;
        mark(&mut arrived, !residents.is_empty());
        mark(
            &mut hatched,
            residents.iter().any(|r| r.stage == LifeStage::Juvenile),
        );
        mark(&mut died, state.fauna.died > 0);
        let no_plants = !state
            .vegetation
            .plants
            .iter()
            .any(terrarium_truth::vegetation::Plant::alive);
        mark(&mut bare, close > silence && no_plants);
        // A month after the plants die, only a decomposer can still be fed.
        if no_plants && bare.is_some_and(|bare| close.hours_since(bare) >= 30 * 24) {
            assert!(
                residents.iter().all(|r| r.species.decomposer()),
                "{residents:?}"
            );
        }
        mark(
            &mut emptied,
            close > silence && no_plants && residents.is_empty(),
        );
        let full = u32::try_from(residents.len()) == Ok(resident_capacity(state.habitat().terrain));
        let well = scene.residents.iter().all(|r| r.energy == Energy::High);
        mark(&mut returned, close > recovering && full && well);
        let ids: Vec<u32> = residents.iter().map(|r| r.id).collect();
        if scene.presentation.state == PresentationState::Dormant {
            assert_eq!(dormant.get_or_insert_with(|| ids.clone()), &ids);
        }
        close = close.saturating_add(24);
    }
    // Tick 0 is day 0 at 09:00 UTC, so tick `t` falls in day `(t + 9) / 24`.
    let day = |step: Option<Tick>| (step.expect("reached").hours() + 9) / 24;
    assert!(day(arrived) < 14, "{arrived:?}");
    assert!(
        hatched < Some(silence) && died < Some(silence),
        "{hatched:?} {died:?}"
    );
    assert!(
        bare < emptied && emptied < Some(recovering),
        "{bare:?} {emptied:?}"
    );
    let returned = returned.expect("the cast returns");
    assert!(returned.hours_since(recovering) <= 60 * 24, "{returned:?}");
    assert!(dormant.is_some_and(|cast| !cast.is_empty()));
    let stones: Vec<Tick> = state
        .landmarks
        .monuments
        .iter()
        .map(|m| m.founded)
        .collect();
    // The first closes on or after the first commit's first and second anniversaries.
    assert_eq!(stones, [Tick::new(8_775), Tick::new(17_535)]);
    let fossils: Vec<Tick> = state.landmarks.fossils.iter().map(|f| f.tick).collect();
    assert_eq!(fossils, [Tick::new(7_451)]);
}

#[test]
fn a_cached_state_resumes_to_the_full_replay() {
    let arc = ArcDir::new();
    let (history, observations, basis, through) = arc.inputs();
    let everything = Prepared::new(basis.clone(), &history.commits, &observations).expect("valid");
    // The full replay, close by close, noting the close at which a resident first dies of old
    // age.
    let mut full = SimState::new(basis.clone());
    let mut dying = None;
    let mut close = first_close(basis.grid);
    while close <= through {
        full.advance_through(&everything, close).expect("replays");
        if dying.is_none()
            && full
                .fauna
                .residents
                .iter()
                .any(|r| r.stage == LifeStage::Dying)
        {
            dying = Some(close.hours());
        }
        close = close.saturating_add(24);
    }
    full.advance_through(&everything, through).expect("replays");
    let dying = dying.expect("a resident dies of old age");
    let tick_of = |o: &Observation| observation_tick(history.grid, o.recorded_at).expect("on grid");
    // Tick 0 is day 0 of the arc at 09:00 UTC. The splits: mid-thriving, the first
    // flowering, the merge's own tick, the first lightning and the one after, the legacy
    // deletion's tick and the one after, the windstorm's close, the first stone's close, a
    // resident's dying close and its burial the next, a midnight in the silence, the first
    // commit back, and the first clearing burn.
    for split in [
        1_000,
        1_257,
        3_560,
        7_305,
        7_329,
        7_451,
        7_452,
        7_455,
        8_775,
        dying,
        dying + 24,
        10_791,
        13_705,
        13_706,
        13_712,
    ] {
        let split = Tick::new(split);
        let seen_commits: Vec<_> = history
            .commits
            .iter()
            .filter(|c| c.tick < split)
            .cloned()
            .collect();
        let seen_observations: Vec<_> = observations
            .iter()
            .filter(|o| tick_of(o) < split)
            .cloned()
            .collect();
        let prefix =
            Prepared::new(basis.clone(), &seen_commits, &seen_observations).expect("valid");
        let mut cached = SimState::new(basis.clone());
        cached
            .advance_through(&prefix, Tick::new(split.hours() - 1))
            .expect("replays");
        let bytes = postcard::to_allocvec(&cached).expect("encodes");
        let mut resumed: SimState = postcard::from_bytes(&bytes).expect("decodes");
        resumed
            .advance_through(&everything, through)
            .expect("resumes");
        assert!(resumed == full, "resuming at {split:?} diverged");
    }
    // A cache that missed a reading it should have seen is refused.
    let mut fewer = observations.clone();
    fewer.remove(0);
    let short = Prepared::new(basis.clone(), &history.commits, &fewer).expect("valid");
    let mut stale = SimState::new(basis);
    stale
        .advance_through(&short, Tick::new(5_000))
        .expect("replays");
    assert_eq!(
        stale.advance_through(&everything, through),
        Err(ReplayError::NeedsFullReplay)
    );
}
