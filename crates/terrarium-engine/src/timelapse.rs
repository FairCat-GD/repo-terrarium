//! The time-lapse's replay: a repository's history and its recorded observations, read once,
//! then replayed from its first commit to give the scene at the close of each day the
//! time-lapse shows, with the event the live view would choose since the day shown before. The
//! same replay gives each day's climate and the presentation's timeline, which the climate
//! export writes.
//!
//! It reads the repository read-only and the store through a connection of its own, and writes
//! nothing: not the sim cache, not the events shown. The live engine's state is never touched.

use std::ops::ControlFlow;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use terrarium_adapters::git::{GitError, GitRepo, Progress};
use terrarium_store::{DataDir, Store, StoreError, StoredObservation, Terrarium};
use terrarium_truth::climate::{ClimateReport, DailyClimate};
use terrarium_truth::replay::{first_close, utc_day};
use terrarium_truth::time::TICKS_PER_DAY;
use terrarium_truth::timeline::TimelineEntry;
use terrarium_truth::{
    Basis, CommitFacts, Observation, Prepared, ReplayError, SceneInput, SimState, Tick,
};

use crate::showing::Showing;

/// A history to replay: its commits, its observations, the basis, and the tick it ends at.
pub struct Lapse {
    pub(crate) commits: Vec<CommitFacts>,
    pub(crate) observations: Vec<Observation>,
    pub(crate) basis: Basis,
    pub(crate) through: Tick,
}

/// One day's climate, as its close left it.
pub struct Day<'a> {
    pub close: Tick,
    /// The UTC day that ends at the close, as days since 1970-01-01.
    pub day: i64,
    /// Truth's record of the day: smoothed light and soil, the soil's components, rain and
    /// wind against their baselines, health, activity and terrain.
    pub record: &'a DailyClimate,
    /// The climate as the soil panel reads it then: the newest readings, whether each is
    /// stale, and the weather.
    pub report: ClimateReport,
}

#[derive(Debug)]
pub enum LapseError {
    Git(GitError),
    Store(StoreError),
    /// The path holds another repository than the terrarium's.
    NotThisRepository,
    /// The moment asked for is before the repository's first commit.
    BeforeHistory,
}

impl From<GitError> for LapseError {
    fn from(err: GitError) -> LapseError {
        LapseError::Git(err)
    }
}

impl From<StoreError> for LapseError {
    fn from(err: StoreError) -> LapseError {
        LapseError::Store(err)
    }
}

impl Lapse {
    /// The terrarium's history through `at_secs`, read from its repository on the default
    /// branch recorded at connect, with its stored observations. `progress` hears how far the
    /// history's read has come, in percent; a set `cancel` stops the read.
    pub fn for_terrarium(
        data: &DataDir,
        terrarium: &Terrarium,
        at_secs: i64,
        progress: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
    ) -> Result<Lapse, LapseError> {
        let git = GitRepo::open(Path::new(&terrarium.repo.path))?;
        let branch = &terrarium.repo.default_branch;
        if git.identity(branch)?.seed != terrarium.seed {
            return Err(LapseError::NotThisRepository);
        }
        let mut last = None;
        let history = git.read_history(branch, &mut |read: Progress| {
            if cancel.load(Ordering::Relaxed) {
                return ControlFlow::Break(());
            }
            if let Some(share) = read.diffed.saturating_mul(100).checked_div(read.total) {
                let percent = u8::try_from(share).unwrap_or(100);
                if last != Some(percent) {
                    last = Some(percent);
                    progress(percent);
                }
            }
            ControlFlow::Continue(())
        })?;
        let through = history
            .grid
            .tick_at(at_secs)
            .ok_or(LapseError::BeforeHistory)?;
        let observations = Store::open(data)?
            .observations(terrarium.id, None)?
            .iter()
            .map(StoredObservation::to_observation)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Lapse {
            basis: Basis::new(terrarium.seed, history.grid, &history.default_branch),
            commits: history.commits,
            observations,
            through,
        })
    }

    /// The close of every day the history has finished, in order: each tick that begins a UTC
    /// day, from the first after the first commit to the last at or before the end.
    pub fn closes(&self) -> Vec<Tick> {
        let mut closes = Vec::new();
        let mut close = first_close(self.basis.grid);
        while close <= self.through {
            closes.push(close);
            close = close.saturating_add(TICKS_PER_DAY);
        }
        closes
    }

    /// The UTC day that ends at `close`, as days since 1970-01-01: the day its scene sums up.
    pub fn day(&self, close: Tick) -> i64 {
        utc_day(self.basis.grid, close) - 1
    }

    /// The scene at each of `closes`, in order, as a strip that looked only then would show
    /// it: each plays the event the live view would choose among those since the close before.
    /// The first close is seen too, so a time-lapse can start just after it.
    ///
    /// # Panics
    /// If `closes` is not in order.
    pub fn scenes(&self, closes: &[Tick]) -> Result<Vec<SceneInput>, ReplayError> {
        self.replay(closes).map(|(scenes, _)| scenes)
    }

    /// The scenes at `closes`, and the state at the last.
    pub(crate) fn replay(
        &self,
        closes: &[Tick],
    ) -> Result<(Vec<SceneInput>, SimState), ReplayError> {
        let mut showing = Showing::default();
        let mut scenes = Vec::with_capacity(closes.len());
        let state = self.walk(closes, |state, _| {
            scenes.push(showing.scene(state));
            Ok::<(), ReplayError>(())
        })?;
        Ok((scenes, state))
    }

    /// Each day the history has finished, in order, handed to `each` as it is replayed, so
    /// nothing is held but the day at hand; an error from `each` stops the replay. Returns
    /// every change of the presentation, oldest first.
    pub fn days<E: From<ReplayError>>(
        &self,
        each: &mut dyn FnMut(Day<'_>) -> Result<(), E>,
    ) -> Result<Vec<TimelineEntry>, E> {
        let state = self.walk(&self.closes(), |state, close| {
            // A close always records its day; one without its record has nothing to say.
            let Some(record) = state.daily.last().filter(|record| record.tick == close) else {
                return Ok(());
            };
            each(Day {
                close,
                day: self.day(close),
                record,
                report: state.report(),
            })
        })?;
        Ok(state.timeline.entries)
    }

    /// When `tick` begins, in Unix seconds.
    pub fn tick_secs(&self, tick: Tick) -> i64 {
        self.basis.grid.start_secs(tick)
    }

    /// Replays from tick 0 through each of `closes` in turn, showing `at` the state at each.
    ///
    /// # Panics
    /// If `closes` is not in order.
    fn walk<E: From<ReplayError>>(
        &self,
        closes: &[Tick],
        mut at: impl FnMut(&SimState, Tick) -> Result<(), E>,
    ) -> Result<SimState, E> {
        assert!(closes.is_sorted(), "closes in order");
        let prepared = Prepared::new(self.basis.clone(), &self.commits, &self.observations)?;
        let mut state = SimState::new(self.basis.clone());
        for &close in closes {
            state.advance_through(&prepared, close)?;
            at(&state, close)?;
        }
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use terrarium_adapters::fixture::{self, Edit, FixtureRepo, Stamp};
    use terrarium_store::{NewTerrarium, RepoLocation};
    use terrarium_truth::RepoSeed;

    use super::*;
    use crate::headless::Request;

    const DAY: i64 = 86_400;

    /// A small repository with a commit a day for `days` days from the arc's start.
    fn repo(days: i64) -> (tempfile::TempDir, FixtureRepo) {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = FixtureRepo::init(dir.path()).unwrap();
        for day in 0..days {
            let text = "a line\n".repeat(usize::try_from(day).unwrap() + 1);
            repo.commit(
                "main",
                Stamp::at(fixture::ARC_START_SECS + day * DAY + 3600),
                &format!("note {day}"),
                &[Edit::write("notes.txt", &text)],
            )
            .unwrap();
        }
        (dir, repo)
    }

    /// The fixture arc and its committed observations, as `replay` reads them.
    fn arc() -> (tempfile::TempDir, Lapse) {
        let dir = tempfile::tempdir().unwrap();
        fixture::arc(&dir.path().join("repo")).unwrap();
        let observations = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/arc/observations.jsonl"),
        )
        .unwrap();
        let lapse = crate::headless::lapse(Request {
            repo: &dir.path().join("repo"),
            data: &DataDir::at(dir.path().join("data")),
            observations: Some(&mut observations.as_slice()),
            at_secs: fixture::ARC_REPLAY_END_SECS,
            install_salt: [0; 16],
        })
        .unwrap();
        (dir, lapse)
    }

    fn connect(data: &DataDir, repo: &FixtureRepo, seed: RepoSeed) -> Terrarium {
        let mut store = Store::open(data).unwrap();
        let id = store
            .create_terrarium(&NewTerrarium {
                seed,
                repo: RepoLocation {
                    path: repo.path().display().to_string(),
                    name: "fern".to_owned(),
                    default_branch: "main".to_owned(),
                },
                install_salt: [0; 16],
                connected_at: fixture::ARC_START_SECS,
            })
            .unwrap();
        store.terrarium(id).unwrap().unwrap()
    }

    fn seed(repo: &FixtureRepo) -> RepoSeed {
        GitRepo::open(repo.path())
            .unwrap()
            .identity("main")
            .unwrap()
            .seed
    }

    #[test]
    fn a_terrarium_s_lapse_reads_its_rows_and_writes_nothing() {
        let (_dir, repo) = repo(5);
        let data_dir = tempfile::tempdir().unwrap();
        let data = DataDir::at(data_dir.path());
        let terrarium = connect(&data, &repo, seed(&repo));
        let file = data_dir.path().join("terrarium.sqlite3");
        let before = std::fs::read(&file).unwrap();
        let mut heard = Vec::new();
        let lapse = Lapse::for_terrarium(
            &data,
            &terrarium,
            fixture::ARC_START_SECS + 10 * DAY,
            &mut |percent| heard.push(percent),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(lapse.commits.len(), 5);
        assert_eq!(heard.last(), Some(&100));
        // Nine closes after the first commit's hour, through day ten.
        let closes = lapse.closes();
        assert_eq!(closes.len(), 10);
        assert_eq!(
            lapse.day(closes[0]) * DAY,
            fixture::ARC_START_SECS - fixture::ARC_START_SECS.rem_euclid(DAY)
        );
        assert_eq!(lapse.scenes(&closes).unwrap().len(), 10);
        assert_eq!(
            std::fs::read(&file).unwrap(),
            before,
            "the store is unchanged"
        );
        assert!(
            Store::open(&data)
                .unwrap()
                .sim_cache(terrarium.id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn another_repository_at_the_path_is_refused_and_a_set_cancel_stops_the_read() {
        let (_dir, repo) = repo(3);
        let data_dir = tempfile::tempdir().unwrap();
        let data = DataDir::at(data_dir.path());
        let terrarium = connect(&data, &repo, RepoSeed::from_bytes([7; 32]));
        let read = |terrarium: &Terrarium, cancel| {
            Lapse::for_terrarium(
                &data,
                terrarium,
                fixture::ARC_START_SECS + 10 * DAY,
                &mut |_| {},
                &AtomicBool::new(cancel),
            )
        };
        assert!(matches!(
            read(&terrarium, false),
            Err(LapseError::NotThisRepository)
        ));
        let terrarium = Terrarium {
            seed: seed(&repo),
            ..terrarium
        };
        assert!(matches!(
            read(&terrarium, true),
            Err(LapseError::Git(GitError::Cancelled))
        ));
        assert!(matches!(
            Lapse::for_terrarium(
                &data,
                &terrarium,
                fixture::ARC_START_SECS - DAY,
                &mut |_| {},
                &AtomicBool::new(false),
            ),
            Err(LapseError::BeforeHistory)
        ));
    }

    #[test]
    fn a_history_younger_than_a_day_has_no_close() {
        let (_dir, repo) = repo(1);
        let data_dir = tempfile::tempdir().unwrap();
        let data = DataDir::at(data_dir.path());
        let terrarium = connect(&data, &repo, seed(&repo));
        let lapse = Lapse::for_terrarium(
            &data,
            &terrarium,
            fixture::ARC_START_SECS + 7200,
            &mut |_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(lapse.closes().is_empty());
    }

    /// Why a test's visitor stopped the replay.
    #[derive(Debug, PartialEq, Eq)]
    enum Stop {
        Asked,
        Replay(ReplayError),
    }

    impl From<ReplayError> for Stop {
        fn from(err: ReplayError) -> Stop {
            Stop::Replay(err)
        }
    }

    #[test]
    fn every_finished_day_gives_its_climate_in_order_then_the_timeline() {
        let (_dir, arc) = arc();
        let closes = arc.closes();
        let mut seen = Vec::new();
        let mut known_light = 0;
        let timeline = arc
            .days(&mut |day: Day<'_>| -> Result<(), Stop> {
                assert_eq!(day.record.tick, day.close);
                assert_eq!(day.day, arc.day(day.close));
                assert_eq!(
                    day.record.light.is_some(),
                    day.report.climate.light.known_value().is_some()
                );
                known_light += usize::from(day.record.light.is_some());
                seen.push(day.close);
                Ok(())
            })
            .unwrap();
        assert_eq!(seen, closes, "one day per close");
        assert!(
            known_light > 0 && known_light < closes.len(),
            "light is unknown before its first report"
        );
        let (_, state) = arc.replay(&closes).unwrap();
        assert_eq!(timeline, state.timeline.entries);
        assert!(timeline.len() > 3);
        assert_eq!(
            arc.tick_secs(closes[0]),
            (arc.day(closes[0]) + 1) * DAY,
            "a close begins its UTC day"
        );
        // A visitor's error ends the replay there.
        let mut heard = 0;
        let stopped = arc.days(&mut |_| {
            heard += 1;
            if heard == 3 { Err(Stop::Asked) } else { Ok(()) }
        });
        assert_eq!(stopped, Err(Stop::Asked));
        assert_eq!(heard, 3);
    }

    #[test]
    fn a_day_skipped_still_plays_its_event_at_the_close_shown_after_it() {
        let (_dir, arc) = arc();
        let closes = arc.closes();
        let daily = arc.scenes(&closes).unwrap();
        let events = |scenes: &[SceneInput]| {
            scenes
                .iter()
                .filter_map(|scene| scene.event.map(|event| (event.kind, event.started_tick)))
                .collect::<BTreeSet<_>>()
        };
        // Every other close: what a day skipped played shows at the next one, unless a
        // stronger event on that day takes the stage.
        let odd: Vec<Tick> = closes.iter().copied().skip(1).step_by(2).collect();
        let sparse = arc.scenes(&odd).unwrap();
        let shown_daily = events(&daily);
        let shown_sparse = events(&sparse);
        assert!(shown_sparse.is_subset(&shown_daily));
        assert!(
            shown_sparse.len() * 10 >= shown_daily.len() * 7,
            "{} of {}",
            shown_sparse.len(),
            shown_daily.len()
        );
        // Each sparse scene's event began after the close shown before it.
        for (i, scene) in sparse.iter().enumerate().skip(1) {
            if let Some(event) = scene.event {
                assert!(event.started_tick > odd[i - 1], "{event:?} at {:?}", odd[i]);
            }
        }
    }
}
