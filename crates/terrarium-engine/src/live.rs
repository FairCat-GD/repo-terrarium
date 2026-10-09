//! The live pipeline: a connected repository, refreshed. It has no thread and no watcher of its
//! own; `worker` runs it, and tests drive it directly.
//!
//! **Connect.** `--repo <path>` derives the default branch and the seed as `record-tests` does,
//! so the status file it writes is the one read here, and creates the terrarium or relinks the
//! one with that seed. A plain run takes the terrarium `repo.active` names, with the default
//! branch recorded at connect: `default_branch()` can follow `HEAD` from one checkout to
//! the next, so it is never asked again. Its seed is checked against the repository by the first
//! refresh.
//!
//! **Refresh.** Git first (`read_history`, then `read_since`), then the reports, then replay:
//! 1. A rewrite of history that moved the default branch's tip to a commit its old tip is not an
//!    ancestor of is recorded as a `ref_rewrite`, with the HEAD of the same refresh.
//! 2. A report is read only once its newest file has been still for [`DEBOUNCE`], so a file a
//!    tool is still writing is never read half-written (lcov has no end marker to say so); a
//!    report not yet still is read by a follow-up.
//! 3. A report is recorded only when its content differs from the newest stored observation of
//!    the same format. Per format rather than per kind and source: local weather has two
//!    writers, and an unchanged `JUnit` report must not be recorded again after a
//!    `record-tests` run, nor an older one when the newer cannot be read.
//! 4. Replay follows the cache contract in `terrarium_truth::replay`: the base state is the
//!    state before the open hour, resumed when truth accepts the inputs it consumed and replayed
//!    in full otherwise; the live state is a copy of it advanced through the open hour. The base
//!    is cached about once an hour.
//!
//! **Time** is the wall clock's, never lower than it was earlier in the session: a clock set
//! back neither rewinds the scene nor dates an observation before one already applied.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use terrarium_adapters::discovery::{self, Overrides};
use terrarium_adapters::git::{Change, GitError, GitRepo, History, Progress, RewriteReason};
use terrarium_adapters::local_tests;
use terrarium_adapters::report::{Format, Report};
use terrarium_store::identifiers::{
    SETTING_COVERAGE_REPORT_PATH, SETTING_DEAD_CODE_REPORT_PATH, SETTING_REPO_ACTIVE,
    SETTING_WEATHER_REPORT_PATH,
};
use terrarium_store::{
    DataDir, InstallKey, NewTerrarium, ObservationId, ObservationRecord, RepoLocation, SimCache,
    Store, StoreError, StoredObservation, Terrarium, TerrariumId,
};
use terrarium_truth::climate::ClimateReport;
use terrarium_truth::domain::{ObservationHead, merge_key};
use terrarium_truth::events::Candidate;
use terrarium_truth::scene::EventKind;
use terrarium_truth::{
    Basis, CommitId, Observation, Prepared, ReplayError, SIM_VERSION, SceneInput, SimState, Tick,
    observation_tick, project_showing,
};

use crate::clock::{Clock, SystemClock};
use crate::observe;
use crate::schedule::DEBOUNCE;
use crate::schedule::{Cause, Reach, Refresh};
use crate::watch::{Places, WatchSet};

/// How long a history read runs before its progress is shown: a short one never flickers a
/// note onto the label.
const PROGRESS_AFTER: Duration = Duration::from_secs(1);

/// What the engine connects, where its data lives, and the clock it reads.
#[derive(Clone)]
pub struct EngineConfig {
    pub data: DataDir,
    /// `--repo <path>`; `None` for the terrarium `repo.active` names.
    pub repo: Option<PathBuf>,
    pub clock: Arc<dyn Clock>,
    pub progress_after: Duration,
}

impl EngineConfig {
    pub fn new(data: DataDir, repo: Option<PathBuf>) -> EngineConfig {
        EngineConfig {
            data,
            repo,
            clock: Arc::new(SystemClock),
            progress_after: PROGRESS_AFTER,
        }
    }
}

/// Why nothing could be connected. Its wording is the app's.
#[derive(Debug)]
pub enum ConnectError {
    Store(StoreError),
    /// The path given is not a repository this build reads, or names no default branch.
    Git(GitError),
    /// The path given cannot be made absolute.
    Path(std::io::Error),
    /// The OS gave no random bytes for a new terrarium's salt.
    Salt(String),
    /// The worker thread could not start.
    Thread(std::io::Error),
    /// Connecting panicked (I-16).
    Halted,
}

impl From<StoreError> for ConnectError {
    fn from(err: StoreError) -> ConnectError {
        ConnectError::Store(err)
    }
}

impl From<GitError> for ConnectError {
    fn from(err: GitError) -> ConnectError {
        ConnectError::Git(err)
    }
}

/// What the strip starts from, before the first refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    Connected {
        repo_name: String,
        /// Cosmetic only (I-2): the composer is built with it.
        install_salt: [u8; 16],
        /// The cached state's scene from the last session, if it is still valid: real truth,
        /// shown while the history is read.
        scene: Option<SceneInput>,
    },
    /// No repository is connected.
    NotConnected,
}

/// Where `HEAD` is, for the label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Branch {
    Named(String),
    Detached(CommitId),
}

/// How a refresh reached its state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replay {
    /// From the start: no usable cached state, or truth refused to resume it.
    Full,
    /// From the cached state before the open hour.
    Resumed,
    /// Nothing new and the same hour: the state shown stands.
    Unchanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefreshStats {
    pub cause: Cause,
    pub replay: Replay,
    /// The history changed or an observation was recorded.
    pub working: bool,
    /// From the adapters returning to the snapshot (N-04).
    pub adapters_to_snapshot: Duration,
}

/// Truth as of now, and the facts the label words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub scene: SceneInput,
    pub report: ClimateReport,
    pub branch: Branch,
    /// The simulation's version changed since this install last ran: shown for the session.
    pub sim_note: bool,
    pub stats: RefreshStats,
}

/// Something that went wrong in a refresh that the next one may not repeat. Its wording is the
/// app's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    Git(GitError),
    Store(String),
    Replay(ReplayError),
    /// The refresh found itself without something an earlier step always leaves: a bug, named
    /// by what was missing. The next refresh reads and replays afresh.
    Missing(&'static str),
}

impl From<StoreError> for Problem {
    fn from(err: StoreError) -> Problem {
        Problem::Store(err.to_string())
    }
}

/// How long a playing event stays in the scenes after the hour it was first shown in: through
/// the next hour, long enough for any burst of snapshots to reach the composer.
const PLAY_HOLD_TICKS: u32 = 1;

/// What a history read found.
struct HistoryRead {
    changed: bool,
    /// Read again in full because earlier facts changed.
    rewritten: bool,
    /// The default tip's move, when it moved without fast-forwarding; its fossils are filled
    /// in from the state last shown.
    moved: Option<observe::Rewrite>,
}

/// How a refresh ended.
#[derive(Debug, Default)]
pub struct Done {
    /// `None` when it was cancelled.
    pub reach: Option<Reach>,
    /// Sent when the strip would show something new, and after every refresh that recorded
    /// something or read new history.
    pub snapshot: Option<Snapshot>,
    /// A report was not still yet: read it again after this.
    pub follow_up: Option<Duration>,
    pub problem: Option<Problem>,
}

/// Why a refresh stopped early.
enum Fault {
    NotFound,
    /// Only `cancel`, set by the worker's stop, ends a refresh this way.
    Cancelled,
    Problem(Problem),
}

/// A history read leaves the history, and a replay leaves the view; meeting either missing is a
/// bug the worker reports, never a cancel, which would end it silently.
const MISSING_HISTORY: Fault = Fault::Problem(Problem::Missing("history"));
const MISSING_VIEW: Fault = Fault::Problem(Problem::Missing("view"));

impl From<StoreError> for Fault {
    fn from(err: StoreError) -> Fault {
        Fault::Problem(err.into())
    }
}

/// A git error during a refresh: the repository, its commits or its default branch gone is the
/// repository not found; anything else is a problem the next refresh may not have.
fn git_fault(err: GitError) -> Fault {
    match err {
        GitError::Open(_)
        | GitError::NoCommits
        | GitError::NoDefaultBranch
        | GitError::BranchMissing(_)
        | GitError::UnsupportedObjectFormat => Fault::NotFound,
        GitError::Cancelled => Fault::Cancelled,
        GitError::Read(_) => Fault::Problem(Problem::Git(err)),
    }
}

/// The newest stored observation a report format wrote: the largest (`recorded_at`, row id).
#[derive(Clone, Copy, Debug)]
struct Newest {
    at: (i64, i64),
    hash: [u8; 32],
}

/// What the strip was last sent, minus the stats.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Shown {
    scene: SceneInput,
    report: ClimateReport,
    branch: Branch,
    sim_note: bool,
}

/// The state shown, and the tick it was advanced through.
struct View {
    live: SimState,
    through: Option<Tick>,
}

pub struct Live {
    store: Store,
    terrarium: Terrarium,
    clock: Arc<dyn Clock>,
    progress_after: Duration,
    overrides: Overrides,
    patterns: Vec<String>,
    status_dir: PathBuf,
    status_file: PathBuf,
    sim_note: bool,
    /// Whether the repository at the stored path has been checked to be this terrarium's.
    verified: bool,
    history: Option<History>,
    observations: Vec<Observation>,
    newest: BTreeMap<Format, Newest>,
    /// The state before the open hour.
    base: Option<SimState>,
    /// The `next_tick` of the cached state last written.
    cached_tick: Option<Tick>,
    view: Option<View>,
    shown: Option<Shown>,
    /// The event instances the state last projected held: what a later refresh has already
    /// had the chance to play. `None` before anything was projected and with no cached state.
    seen: Option<BTreeSet<(Tick, EventKind)>>,
    /// The event the scene plays, and the tick it was first shown at. It stays in the scenes
    /// through the hour after that, so a burst of snapshots never drops it, unless an unseen
    /// event outranks it; the composer plays each instance once.
    playing: Option<((Tick, EventKind), Tick)>,
    /// The session's time: the wall clock, never lower than before.
    now: i64,
    watch: Option<WatchSet>,
}

impl Live {
    /// Connects what `config` names, or `None` when it names nothing (a plain run with no
    /// repository connected).
    pub fn connect(config: &EngineConfig) -> Result<Option<Live>, ConnectError> {
        let mut store = Store::open(&config.data)?;
        let now = config.clock.wall_secs();
        let (terrarium, verified) = match &config.repo {
            None => {
                let Some(id) = store.setting::<i64>(SETTING_REPO_ACTIVE)? else {
                    return Ok(None);
                };
                let Some(terrarium) = store.terrarium(TerrariumId::new(id))? else {
                    return Ok(None);
                };
                (terrarium, false)
            }
            Some(path) => (connect_repo(&mut store, path, now)?, true),
        };
        let current = SIM_VERSION.0.to_string();
        let last = store.install_value(InstallKey::LastSimVersion)?;
        let sim_note = last.as_deref().is_some_and(|last| last != current);
        if last.as_deref() != Some(current.as_str()) {
            store.set_install_value(InstallKey::LastSimVersion, &current)?;
        }
        let overrides = Overrides {
            coverage: store.setting(SETTING_COVERAGE_REPORT_PATH)?,
            dead_code: store.setting(SETTING_DEAD_CODE_REPORT_PATH)?,
            weather: store.setting(SETTING_WEATHER_REPORT_PATH)?,
        };
        let status_dir = config.data.local_tests_dir();
        std::fs::create_dir_all(&status_dir).map_err(StoreError::Io)?;
        let stored = store.observations(terrarium.id, None)?;
        let mut newest = BTreeMap::new();
        let mut observations = Vec::with_capacity(stored.len());
        for row in &stored {
            note_newest(&mut newest, row);
            observations.push(row.to_observation()?);
        }
        let (base, cached_tick) = match cached_base(&store, &terrarium)? {
            Some((base, tick)) => (Some(base), Some(tick)),
            None => (None, None),
        };
        let seen = base.as_ref().map(|base| base.events.keys());
        Ok(Some(Live {
            status_file: config.data.local_tests(&terrarium.seed),
            patterns: discovery::report_patterns(&overrides),
            store,
            terrarium,
            clock: Arc::clone(&config.clock),
            progress_after: config.progress_after,
            overrides,
            status_dir,
            sim_note,
            verified,
            history: None,
            observations,
            newest,
            base,
            cached_tick,
            view: None,
            shown: None,
            seen,
            playing: None,
            now,
            watch: None,
        }))
    }

    /// What the strip starts from.
    pub fn start(&self) -> Start {
        Start::Connected {
            repo_name: self.terrarium.repo.name.clone(),
            install_salt: self.terrarium.install_salt,
            scene: self.base.as_ref().map(|base| project_showing(base, None)),
        }
    }

    pub fn terrarium(&self) -> &Terrarium {
        &self.terrarium
    }

    /// The live state last shown.
    pub fn state(&self) -> Option<&SimState> {
        self.view.as_ref().map(|view| &view.live)
    }

    /// What the repository's watcher should watch, as of the last refresh that reached it.
    pub fn watch_set(&self) -> Option<&WatchSet> {
        self.watch.as_ref()
    }

    /// Brings the terrarium up to now. `progress` hears a long history read's percent; `cancel`
    /// stops between steps and inside a read.
    pub fn refresh(
        &mut self,
        refresh: Refresh,
        progress: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
    ) -> Done {
        self.now = self.now.max(self.clock.wall_secs());
        let ran = self.run(refresh, progress, cancel);
        self.done(ran)
    }

    /// How a refresh that ran, or stopped early, ends. Only a cancel ends without a reach.
    fn done(&mut self, ran: Result<Done, Fault>) -> Done {
        match ran {
            Ok(done) => done,
            Err(Fault::NotFound) => {
                self.verified = false;
                self.shown = None;
                self.watch = None;
                Done {
                    reach: Some(Reach::NotFound),
                    ..Done::default()
                }
            }
            Err(Fault::Cancelled) => {
                self.view = None;
                Done::default()
            }
            Err(Fault::Problem(problem)) => {
                // It may have read new history or recorded something before it failed: the
                // next refresh replays rather than taking the state shown as current.
                self.view = None;
                Done {
                    reach: Some(Reach::Found),
                    problem: Some(problem),
                    ..Done::default()
                }
            }
        }
    }

    fn run(
        &mut self,
        refresh: Refresh,
        progress: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
    ) -> Result<Done, Fault> {
        let path = PathBuf::from(&self.terrarium.repo.path);
        let git = GitRepo::open(&path).map_err(git_fault)?;
        let branch = self.terrarium.repo.default_branch.clone();
        if !self.verified {
            let identity = git.identity(&branch).map_err(git_fault)?;
            if identity.seed != self.terrarium.seed {
                return Err(Fault::NotFound);
            }
            self.verified = true;
        }
        let head = git.head(&branch).map_err(git_fault)?;
        let read = self.read_history(&git, &branch, progress, cancel)?;
        if read.rewritten {
            // Rewritten history may be another repository at the same path.
            let identity = git.identity(&branch).map_err(git_fault)?;
            if identity.seed != self.terrarium.seed {
                self.history = None;
                return Err(Fault::NotFound);
            }
        }
        let history_changed = read.changed;
        let mut recorded = false;
        if let Some(moved) = read.moved {
            let rewrite = self.with_fossils(moved);
            let record = observe::ref_rewrite(&branch, rewrite, head.clone(), self.now);
            self.append(None, record)?;
            recorded = true;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(Fault::Cancelled);
        }
        let root = git.workdir().map(Path::to_path_buf);
        let mut follow_up = None;
        if refresh.reports {
            let (records, wait) = self.read_reports(root.as_deref(), &head);
            follow_up = wait;
            for (format, record) in records {
                self.append(Some(format), record)?;
                recorded = true;
            }
        }
        let adapters_done = Instant::now();
        let history = self.history.as_ref().ok_or(MISSING_HISTORY)?;
        let through = history.grid.tick_at(self.now);
        let unchanged = !history_changed
            && !recorded
            && self
                .view
                .as_ref()
                .is_some_and(|view| view.through == through);
        let (replay, problem) = if unchanged {
            (Replay::Unchanged, None)
        } else {
            self.replay(through)?
        };
        let scene = self.scene().ok_or(MISSING_VIEW)?;
        let view = self.view.as_ref().ok_or(MISSING_VIEW)?;
        let shown = Shown {
            scene,
            report: view.live.report(),
            branch: match head.branch {
                Some(name) => Branch::Named(name),
                None => Branch::Detached(head.commit),
            },
            sim_note: self.sim_note,
        };
        let working = history_changed || recorded;
        let snapshot = (working || self.shown.as_ref() != Some(&shown)).then(|| Snapshot {
            scene: shown.scene.clone(),
            report: shown.report.clone(),
            branch: shown.branch.clone(),
            sim_note: shown.sim_note,
            stats: RefreshStats {
                cause: refresh.cause,
                replay,
                working,
                adapters_to_snapshot: adapters_done.elapsed(),
            },
        });
        self.shown = Some(shown);
        // Only a refresh that looked at the reports can have seen a report directory appear;
        // the git directories never move under an open repository.
        if refresh.reports || self.watch.is_none() {
            self.watch = Some(WatchSet::new(&Places {
                git_dir: git.git_dir(),
                common_dir: git.common_dir(),
                root: root.as_deref(),
                patterns: &self.patterns,
                status_dir: &self.status_dir,
                status_name: &self.terrarium.seed.to_hex(),
            }));
        }
        Ok(Done {
            reach: Some(Reach::Found),
            snapshot,
            follow_up,
            problem,
        })
    }

    /// The view's scene. It plays the best event the state last projected did not hold (with
    /// none projected, the newest tick's best), unless the one already playing is still within
    /// its hold and outranks it: one foreground event at a time, as arbitration among events
    /// that come close together.
    fn scene(&mut self) -> Option<SceneInput> {
        let view = self.view.as_ref()?;
        let now = view.live.now();
        let events = &view.live.events;
        let held = self
            .playing
            .filter(|&(_, shown)| now.hours_since(shown) <= PLAY_HOLD_TICKS)
            .and_then(|(key, _)| events.find(key));
        let fresh = match &self.seen {
            Some(seen) => events.unseen(seen),
            None => events.current(now),
        };
        if let Some(fresh) = fresh {
            let winner = terrarium_truth::events::arbitrate(held.into_iter().chain([fresh]));
            if winner.map(Candidate::key) == Some(fresh.key()) {
                self.playing = Some((fresh.key(), now));
            }
        }
        let playing = self
            .playing
            .filter(|&(_, shown)| now.hours_since(shown) <= PLAY_HOLD_TICKS)
            .and_then(|(key, _)| events.find(key));
        if playing.is_none() {
            self.playing = None;
        }
        self.seen = Some(events.keys());
        Some(project_showing(&view.live, playing))
    }

    /// `moved` with what it took: the fossils the state last shown (or, failing that, the
    /// cached one) held whose commit is no longer on the default branch's first-parent chain.
    fn with_fossils(&self, moved: observe::Rewrite) -> observe::Rewrite {
        let shown = self
            .view
            .as_ref()
            .map(|view| &view.live)
            .or(self.base.as_ref());
        let fossils = match (shown, self.history.as_ref()) {
            (Some(state), Some(history)) => {
                let chain = history.default_chain();
                state
                    .landmarks
                    .fossils
                    .iter()
                    .filter(|fossil| !chain.contains(&fossil.commit))
                    .map(|fossil| fossil.commit.clone())
                    .collect()
            }
            _ => Vec::new(),
        };
        observe::Rewrite { fossils, ..moved }
    }

    /// Reads the history in full the first time and what is new afterwards.
    fn read_history(
        &mut self,
        git: &GitRepo,
        branch: &str,
        progress: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
    ) -> Result<HistoryRead, Fault> {
        let started = Instant::now();
        let after = self.progress_after;
        let mut last = None;
        let mut on_progress = |p: Progress| {
            if cancel.load(Ordering::Relaxed) {
                return ControlFlow::Break(());
            }
            if p.total > 0 && started.elapsed() >= after {
                let percent = u8::try_from(p.diffed.saturating_mul(100) / p.total).unwrap_or(100);
                if last != Some(percent) {
                    last = Some(percent);
                    progress(percent);
                }
            }
            ControlFlow::Continue(())
        };
        let Some(history) = self.history.as_mut() else {
            let history = git
                .read_history(branch, &mut on_progress)
                .map_err(git_fault)?;
            // A move made while nothing watched: the cached state's default tip has left the
            // first-parent chain. How many commits left with it is no longer known.
            let moved = self
                .base
                .as_ref()
                .and_then(|base| base.habitat.default_tip.clone())
                .filter(|tip| !history.default_chain().contains(tip))
                .zip(history.state.branches.get(branch).cloned())
                .map(|(old, new)| observe::Rewrite {
                    old,
                    new,
                    rewritten: None,
                    fossils: Vec::new(),
                });
            self.history = Some(history);
            return Ok(HistoryRead {
                changed: true,
                rewritten: false,
                moved,
            });
        };
        let old = history.state.branches.get(branch).cloned();
        let change = git
            .read_since(history, branch, &mut on_progress)
            .map_err(git_fault)?;
        let moved = match &change {
            Change::Rewritten { reason, left_chain }
                if *reason != RewriteReason::DefaultBranchChanged =>
            {
                let new = history.state.branches.get(branch).cloned();
                old.zip(new)
                    .filter(|(old, new)| old != new && !history.is_ancestor(old, new))
                    .map(|(old, new)| observe::Rewrite {
                        old,
                        new,
                        rewritten: Some(u32::try_from(left_chain.len()).unwrap_or(u32::MAX)),
                        fossils: Vec::new(),
                    })
            }
            _ => None,
        };
        Ok(HistoryRead {
            changed: change != Change::Unchanged,
            rewritten: matches!(change, Change::Rewritten { .. }),
            moved,
        })
    }

    /// Reads the coverage report, the test report and the status file that have been still
    /// long enough, and returns what is new among them, oldest file first, and when to look
    /// again at the rest.
    fn read_reports(
        &self,
        root: Option<&Path>,
        head: &ObservationHead,
    ) -> (Vec<(Format, ObservationRecord)>, Option<Duration>) {
        let mut wait: Option<Duration> = None;
        let mut defer = |left: Duration| wait = Some(wait.map_or(left, |w| w.min(left)));
        let mut reports: Vec<Report> = Vec::new();
        // Reports live in the worktree; the status file lives in the data directory, so a bare
        // repository has one too.
        if let Some(root) = root {
            let found = discovery::locate_reports(root, &self.overrides);
            for located in [found.coverage.located(), found.test_report.located()]
                .into_iter()
                .flatten()
            {
                match unsettled(located.files.iter().map(|file| root.join(file))) {
                    Some(left) => defer(left),
                    None => reports.push(Report::read(root, located.format, &located.files)),
                }
            }
        }
        if self.status_file.is_file() {
            match unsettled([self.status_file.clone()].into_iter()) {
                Some(left) => defer(left),
                None => reports.push(local_tests::read(&self.status_file)),
            }
        }
        let mut new: Vec<(Option<i64>, Format, ObservationRecord)> = reports
            .iter()
            .filter_map(|report| {
                let record = observe::record(report, head.clone(), self.now)?;
                let same = self
                    .newest
                    .get(&report.format)
                    .is_some_and(|newest| newest.hash == record.content_hash);
                (!same).then_some((record.report_mtime, report.format, record))
            })
            .collect();
        new.sort_by_key(|(mtime, format, _)| (*mtime, *format));
        (
            new.into_iter()
                .map(|(_, format, record)| (format, record))
                .collect(),
            wait,
        )
    }

    /// Stores `record` with a `recorded_at` no stored observation of its kind, source and head
    /// already has, and keeps it for replay.
    fn append(
        &mut self,
        producer: Option<Format>,
        mut record: ObservationRecord,
    ) -> Result<(), Fault> {
        while self.observations.iter().any(|o| same_key(o, &record)) {
            record.recorded_at += 1;
        }
        let id = self.store.append_observation(self.terrarium.id, &record)?;
        let stored = StoredObservation {
            id,
            terrarium: self.terrarium.id,
            record,
        };
        self.observations.push(stored.to_observation()?);
        // What this install just recorded is what the next read compares with, even when a
        // stored row (imported, or recorded while the clock was ahead) is dated later.
        if let Some(format) = producer {
            self.newest.insert(
                format,
                Newest {
                    at: (stored.record.recorded_at, stored.id.get()),
                    hash: stored.record.content_hash,
                },
            );
        }
        Ok(())
    }

    /// Replays through `through`: the base resumed or replayed in full through the hour before,
    /// then a copy advanced through `through`. Caches the base when it moved.
    fn replay(&mut self, through: Option<Tick>) -> Result<(Replay, Option<Problem>), Fault> {
        let history = self.history.as_ref().ok_or(MISSING_HISTORY)?;
        let basis = Basis::new(self.terrarium.seed, history.grid, &history.default_branch);
        let prepared = Prepared::new(basis.clone(), &history.commits, &self.observations)
            .map_err(|err| Fault::Problem(Problem::Replay(err)))?;
        let replay_err = |err| Fault::Problem(Problem::Replay(err));
        let mut replay = Replay::Resumed;
        let (base, live) = match through {
            None => (None, SimState::new(basis)),
            Some(tick) if tick == Tick::ZERO => {
                let mut live = SimState::new(basis);
                live.advance_through(&prepared, tick).map_err(replay_err)?;
                replay = Replay::Full;
                (None, live)
            }
            Some(tick) => {
                let before = Tick::new(tick.hours() - 1);
                let resumed = self
                    .base
                    .take()
                    .filter(|base| base.next_tick <= tick)
                    .and_then(|mut base| {
                        base.advance_through(&prepared, before).ok().map(|()| base)
                    });
                let base = if let Some(base) = resumed {
                    base
                } else {
                    replay = Replay::Full;
                    let mut base = SimState::new(basis);
                    base.advance_through(&prepared, before)
                        .map_err(replay_err)?;
                    base
                };
                let mut live = base.clone();
                live.advance_through(&prepared, tick).map_err(replay_err)?;
                (Some(base), live)
            }
        };
        let mut problem = None;
        if let Some(base) = &base
            && (replay == Replay::Full || self.cached_tick != Some(base.next_tick))
        {
            let cache = cache_of(base, history, &self.observations);
            match cache.and_then(|cache| self.store.write_sim_cache(self.terrarium.id, &cache)) {
                Ok(()) => self.cached_tick = Some(base.next_tick),
                Err(err) => problem = Some(err.into()),
            }
        }
        self.base = base;
        self.view = Some(View { live, through });
        Ok((replay, problem))
    }
}

/// The cache row for `base`. Its high-water mark is the newest observation recorded before the
/// base's next tick; truth's own check of what the state consumed is what decides whether it
/// resumes.
fn cache_of(
    base: &SimState,
    history: &History,
    observations: &[Observation],
) -> Result<SimCache, StoreError> {
    let high_water = observations
        .iter()
        .filter(|o| {
            observation_tick(history.grid, o.recorded_at).is_some_and(|t| t < base.next_tick)
        })
        .map(|o| o.id)
        .max()
        .map(ObservationId::new);
    Ok(SimCache {
        sim_version: SIM_VERSION,
        tips_hash: history.state.state_hash(),
        obs_high_water: high_water,
        tick: base.next_tick,
        state: postcard::to_allocvec(base).map_err(|_| StoreError::Corrupt("state"))?,
    })
}

/// Creates the terrarium for the repository at `path`, or relinks the one with its seed, and
/// makes it the active one.
fn connect_repo(store: &mut Store, path: &Path, now: i64) -> Result<Terrarium, ConnectError> {
    let path = normalized(&std::path::absolute(path).map_err(ConnectError::Path)?);
    let (default_branch, identity) = GitRepo::open(&path)?.default_identity()?;
    let seed = identity.seed;
    let location = RepoLocation {
        path: path.display().to_string(),
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        default_branch,
    };
    let id = if let Some(known) = store.terrarium_by_seed(&seed)? {
        store.activate(known.id, &location)?;
        known.id
    } else {
        let mut install_salt = [0; 16];
        getrandom::fill(&mut install_salt).map_err(|err| ConnectError::Salt(err.to_string()))?;
        store.create_terrarium(&NewTerrarium {
            seed,
            repo: location,
            install_salt,
            connected_at: now,
        })?
    };
    store.set_setting(SETTING_REPO_ACTIVE, &id.get())?;
    Ok(store
        .terrarium(id)?
        .ok_or(StoreError::NoSuchTerrarium(id))?)
}

/// `path` with its `.` and `..` resolved as text, so `--repo ..` names the folder it means.
/// Canonicalizing would also resolve symlinks, and on Windows give `\\?\` paths.
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            part => out.push(part),
        }
    }
    out
}

/// The cached base state, when it was written under this `SIM_VERSION` for this terrarium's
/// seed and default branch and still decodes; and the tick it was cached at.
fn cached_base(
    store: &Store,
    terrarium: &Terrarium,
) -> Result<Option<(SimState, Tick)>, StoreError> {
    let Some(cache) = store.sim_cache(terrarium.id)? else {
        return Ok(None);
    };
    if cache.sim_version != SIM_VERSION {
        return Ok(None);
    }
    Ok(postcard::from_bytes::<SimState>(&cache.state)
        .ok()
        .filter(|state| {
            state.basis.sim_version == SIM_VERSION
                && state.basis.seed == terrarium.seed
                && state.basis.default_branch == terrarium.repo.default_branch
                && state.next_tick == cache.tick
        })
        .map(|state| (state, cache.tick)))
}

/// Notes `row` as its format's newest stored observation, if it is: at connect, the largest
/// (`recorded_at`, row id).
fn note_newest(newest: &mut BTreeMap<Format, Newest>, row: &StoredObservation) {
    let Some(format) = observe::producer(&row.record) else {
        return;
    };
    let at = (row.record.recorded_at, row.id.get());
    if newest.get(&format).is_none_or(|known| known.at < at) {
        newest.insert(
            format,
            Newest {
                at,
                hash: row.record.content_hash,
            },
        );
    }
}

/// Whether `observation` has `record`'s merge key.
fn same_key(observation: &Observation, record: &ObservationRecord) -> bool {
    observation.merge_key()
        == merge_key(
            record.recorded_at,
            record.kind,
            record.source.as_ref(),
            &record.head.commit,
        )
}

/// How much longer the newest of `files` must stay untouched before it is read, or `None` when
/// it has been still for [`DEBOUNCE`]. A file modified in the future, or gone, waits for nothing.
fn unsettled(files: impl Iterator<Item = PathBuf>) -> Option<Duration> {
    let newest = files
        .filter_map(|file| {
            std::fs::metadata(file)
                .and_then(|meta| meta.modified())
                .ok()
        })
        .max()?;
    let age = SystemTime::now().duration_since(newest).ok()?;
    DEBOUNCE.checked_sub(age).filter(|left| !left.is_zero())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::thread;

    use terrarium_adapters::fixture::{Edit, FixtureRepo, Stamp};

    use super::*;
    use crate::worker::{Engine, Pipeline, Update};

    /// A connected pipeline that replays before any history is read: the state no refresh
    /// reaches, met head on.
    struct Unread(Live);

    impl Pipeline for Unread {
        fn refresh(&mut self, _: Refresh, _: &mut dyn FnMut(u8), _: &AtomicBool) -> Done {
            let ran = self.0.replay(None).map(|_| Done::default());
            self.0.done(ran)
        }

        fn watch_set(&self) -> Option<&WatchSet> {
            None
        }
    }

    #[test]
    fn a_refresh_missing_its_history_is_reported_and_never_ends_the_engine_silently() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        FixtureRepo::init(&repo)
            .unwrap()
            .commit(
                "main",
                Stamp::at(1_704_704_400),
                "tend",
                &[Edit::write("src/fern.rs", "fn fern() {}\n")],
            )
            .unwrap();
        let config = EngineConfig::new(DataDir::at(dir.path().join("data")), Some(repo));
        let clock = Arc::clone(&config.clock);
        let heard = Arc::new(Mutex::new(Vec::new()));
        let into = Arc::clone(&heard);
        let (_engine, start) = Engine::spawn_pipeline(
            move || {
                Ok(Live::connect(&config)?.map(|live| {
                    let start = live.start();
                    (Unread(live), start)
                }))
            },
            clock,
            Box::new(move |update| into.lock().unwrap().push(update)),
        )
        .unwrap();
        assert!(matches!(start, Start::Connected { .. }));
        let until = Instant::now() + Duration::from_secs(10);
        while !heard
            .lock()
            .unwrap()
            .iter()
            .any(|update| matches!(update, Update::Problem(Problem::Missing("history"))))
        {
            assert!(Instant::now() < until, "{:?}", heard.lock().unwrap());
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_path_is_normalized_as_text() {
        let base = std::path::absolute("moss").unwrap();
        assert_eq!(normalized(&base.join("sub/..")), base);
        assert_eq!(normalized(&base.join("./a/./b/../c")), base.join("a/c"));
        assert_eq!(
            normalized(&base.join("..")).file_name(),
            base.parent().unwrap().file_name()
        );
    }

    #[test]
    fn a_file_still_being_written_waits_and_a_still_one_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let fresh = dir.path().join("fresh.info");
        std::fs::write(&fresh, "SF:a\n").unwrap();
        let left = unsettled([fresh.clone()].into_iter()).expect("waits");
        assert!(left <= DEBOUNCE);
        let still = dir.path().join("still.info");
        std::fs::write(&still, "SF:a\n").unwrap();
        let file = std::fs::File::options().write(true).open(&still).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(2))
            .unwrap();
        assert_eq!(unsettled([still.clone()].into_iter()), None);
        // The newest of several decides.
        assert!(unsettled([still, fresh].into_iter()).is_some());
        assert_eq!(unsettled([dir.path().join("gone")].into_iter()), None);
        let future = dir.path().join("future.info");
        std::fs::write(&future, "SF:a\n").unwrap();
        let file = std::fs::File::options().write(true).open(&future).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(60))
            .unwrap();
        assert_eq!(unsettled([future].into_iter()), None);
    }
}
