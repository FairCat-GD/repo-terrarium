//! The fixture arc, replayed through the headless pipeline: it reaches every presentation state,
//! its plants thrive, wilt, go pale, die and grow back, a cached state resumes to the same result
//! as a full replay, and neither the order of an import nor the install salt changes anything.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use terrarium_adapters::fixture::{self, ARC_REPLAY_END_SECS, Phase};
use terrarium_adapters::git::{GitRepo, History};
use terrarium_engine::headless::{Request, replay_dump};
use terrarium_store::{DataDir, NewTerrarium, RepoLocation, Store, StoredObservation};
use terrarium_truth::habitat::plant_capacity;
use terrarium_truth::replay::first_close;
use terrarium_truth::scene::PlantState;
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
            .observations(terrarium, None, None)
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

/// In vegetation terms the arc reaches thriving, wilting (soil limits), pale (light limits) and
/// dead, in that order and before it recovers, and once it recovers a new generation fills the
/// capacity and thrives.
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

#[test]
fn a_cached_state_resumes_to_the_full_replay() {
    let arc = ArcDir::new();
    let (history, observations, basis, through) = arc.inputs();
    let everything = Prepared::new(basis.clone(), &history.commits, &observations).expect("valid");
    let mut full = SimState::new(basis.clone());
    full.advance_through(&everything, through).expect("replays");
    let tick_of = |o: &Observation| observation_tick(history.grid, o.recorded_at).expect("on grid");
    // Tick 0 is day 0 of the arc at 09:00 UTC. The splits: mid-thriving, the merge's own
    // tick, a midnight in the silence, and the first commit back.
    for split in [1_000, 3_560, 10_791, 13_705] {
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
