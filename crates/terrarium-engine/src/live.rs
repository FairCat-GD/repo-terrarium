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
//! **Refresh.** Git first (`read_history`, then `read_since`), then the source, then the
//! reports, then replay:
//! 1. A rewrite of history that moved the default branch's tip to a commit its old tip is not an
//!    ancestor of is recorded as a `ref_rewrite`, with the HEAD of the same refresh.
//! 2. The committed source at the default tip is measured once per tip, while HEAD is at it:
//!    its complexity and TODO density carry HEAD as every observation does, and truth follows
//!    HEAD's branch. Only blobs not met before are parsed. The first refresh of a session leaves
//!    it to an immediate follow-up, so a cold parse never holds back the first snapshot.
//! 3. A report is read only once its newest file has been still for [`DEBOUNCE`], so a file a
//!    tool is still writing is never read half-written (lcov has no end marker to say so); a
//!    report not yet still is read by a follow-up.
//! 4. A report is recorded only when its content differs from the newest stored observation of
//!    the same format. Per format rather than per kind and source: local weather has two
//!    writers, and an unchanged `JUnit` report must not be recorded again after a
//!    `record-tests` run, nor an older one when the newer cannot be read. A Knip report's dead
//!    code is weighed against the exports counted in HEAD's commit, once per report and commit.
//! 5. Replay follows the cache contract in `terrarium_truth::replay`: the base state is the
//!    state before the open hour, resumed when truth accepts the inputs it consumed and replayed
//!    in full otherwise; the live state is a copy of it advanced through the open hour. The base
//!    is cached about once an hour.
//!
//! **Time** is the wall clock's, never lower than it was earlier in the session: a clock set
//! back neither rewinds the scene nor dates an observation before one already applied.

use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use terrarium_adapters::discovery::{self, Enabled};
use terrarium_adapters::git::{Change, GitError, GitRepo, History, Progress, RewriteReason};
use terrarium_adapters::local_tests;
use terrarium_adapters::metrics::{METRICS_VERSION, Measurement, Metrics};
use terrarium_adapters::report::{Format, Report};
use terrarium_store::identifiers::SETTING_REPO_ACTIVE;
use terrarium_store::{
    DataDir, InstallKey, NewTerrarium, ObservationId, ObservationRecord, SimCache, Store,
    StoreError, StoredObservation, Terrarium, TerrariumId,
};
use terrarium_truth::climate::ClimateReport;
use terrarium_truth::domain::{LeftCommit, ObservationHead, merge_key};
use terrarium_truth::{
    Basis, CommitId, Observation, ObservationKind, Prepared, ReplayError, SIM_VERSION, SceneInput,
    SimState, Tick, observation_tick, project_showing,
};

use crate::clock::{Clock, SystemClock};
use crate::details::{self, Details, FileCache, Found, Sources};
use crate::lifecycle::{self, AdapterSettings, stored_adapters};
use crate::observe;
use crate::schedule::DEBOUNCE;
use crate::schedule::{Cause, Reach, Refresh};
use crate::showing::Showing;
use crate::shown::Remembered;
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
    /// The folder given to relink a terrarium holds another repository.
    OtherRepository,
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
        /// Its name, where it lives, and its install salt, which is cosmetic only (I-2): the
        /// composer is built with it.
        terrarium: Box<Terrarium>,
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
    /// What the soil panel and the hover labels say beside the report.
    pub details: Details,
    pub branch: Branch,
    /// What happened that the label notes once; each is carried by one snapshot.
    pub notes: News,
    pub stats: RefreshStats,
}

/// What happened that the label notes once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct News {
    /// The simulation's version changed since this install last showed a terrarium.
    pub sim_updated: bool,
    /// The default branch moved without fast-forwarding, and the history replayed from it.
    pub history_rewritten: bool,
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

/// What a history read found.
struct HistoryRead {
    changed: bool,
    /// Read again in full because earlier facts changed.
    rewritten: bool,
    /// The default tip's move, when it moved without fast-forwarding, and what it took off.
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

/// What a move made while nothing watched took off the chain, as read back: a read the
/// repository cannot complete leaves it unknown, as when the commits are gone, rather than
/// failing the refresh, which would fail the same way every time after. Only a stop ends it.
fn read_back(
    read: Result<Option<Vec<LeftCommit>>, GitError>,
) -> Result<Option<Vec<LeftCommit>>, Fault> {
    match read {
        Ok(left) => Ok(left),
        Err(GitError::Cancelled) => Err(Fault::Cancelled),
        Err(_) => Ok(None),
    }
}

/// The newest stored observation a report format wrote: the largest (`recorded_at`, row id).
#[derive(Clone, Copy, Debug)]
struct Newest {
    at: (i64, i64),
    hash: [u8; 32],
}

/// What measuring the source did in a refresh.
enum Metered {
    /// Nothing to measure, or nothing known to record.
    Unchanged,
    Recorded,
    /// Left to a follow-up: the session has no view yet.
    Deferred,
    /// The commit's tree could not be read; the refresh goes on without it.
    Failed(Problem),
}

/// What the adapters of one refresh did.
#[derive(Default)]
struct Observed {
    recorded: bool,
    follow_up: Option<Duration>,
    /// What kept the source from being measured.
    problem: Option<Problem>,
}

impl Observed {
    fn take(&mut self, metered: Metered) {
        match metered {
            Metered::Unchanged => {}
            Metered::Recorded => self.recorded = true,
            Metered::Deferred => self.follow_up = Some(Duration::ZERO),
            Metered::Failed(problem) => self.problem = Some(problem),
        }
    }
}

/// What a look at the reports found.
struct ReportsRead {
    /// New readings, oldest file first.
    records: Vec<(Format, ObservationRecord)>,
    /// When to look again at the reports not yet still.
    wait: Option<Duration>,
    found: Found,
    /// A known Knip report not yet weighed at HEAD's commit.
    knip: Option<Report>,
}

/// The sooner of two follow-ups.
fn sooner(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// What the strip was last sent, minus the stats.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Shown {
    scene: SceneInput,
    report: ClimateReport,
    details: Details,
    branch: Branch,
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
    /// What the settings say of each adapter, as of the last refresh asked for.
    adapters: AdapterSettings,
    patterns: Vec<String>,
    status_dir: PathBuf,
    status_file: PathBuf,
    /// What the next snapshot sent carries.
    news: News,
    /// Whether the repository at the stored path has been checked to be this terrarium's.
    verified: bool,
    history: Option<History>,
    observations: Vec<Observation>,
    newest: BTreeMap<Format, Newest>,
    /// The source metrics' parsers, and the facts of the files they have read this session.
    metrics: Metrics,
    /// The default tip whose source was measured last, and the metrics switched on then, so
    /// each tip is measured once, and again only for a metric switched on since.
    measured: Option<(CommitId, Enabled)>,
    /// The Knip report last weighed (its content hash), and HEAD's commit it was weighed at.
    knip_weighed: Option<([u8; 32], CommitId)>,
    /// Which reports the last look at the worktree found.
    found: Found,
    /// The files behind the readings shown, by observation.
    files: FileCache,
    /// The state before the open hour.
    base: Option<SimState>,
    /// The `next_tick` of the cached state last written.
    cached_tick: Option<Tick>,
    view: Option<View>,
    shown: Option<Shown>,
    /// Which event the scenes play.
    showing: Showing,
    /// What an earlier session's scenes played, until the history it is keyed to is read.
    remembered: Option<Remembered>,
    /// What the scenes had played when it was last stored.
    stored_seen: Option<crate::shown::Seen>,
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
                let Some(terrarium) = store
                    .terrarium(TerrariumId::new(id))?
                    .filter(|terrarium| terrarium.active)
                else {
                    return Ok(None);
                };
                (terrarium, false)
            }
            Some(path) => (connect_repo(&mut store, path, now)?, true),
        };
        // The note waits for a snapshot to carry it, and the version is recorded only then. A
        // first install has nothing to note.
        let current = SIM_VERSION.0.to_string();
        let last = store.install_value(InstallKey::LastSimVersion)?;
        if last.is_none() {
            store.set_install_value(InstallKey::LastSimVersion, &current)?;
        }
        let news = News {
            sim_updated: last.is_some_and(|last| last != current),
            history_rewritten: false,
        };
        let adapters = stored_adapters(&store)?;
        let status_dir = config.data.local_tests_dir();
        std::fs::create_dir_all(&status_dir).map_err(StoreError::Io)?;
        let stored = store.observations(terrarium.id, None)?;
        let mut newest = BTreeMap::new();
        let mut observations = Vec::with_capacity(stored.len());
        for row in &stored {
            note_newest(&mut newest, row);
            observations.push(row.to_observation()?);
        }
        // One measurement records both metrics, so the newest of either says which tip it was.
        let measured = stored
            .iter()
            .filter(|row| {
                let record = &row.record;
                matches!(
                    record.kind,
                    ObservationKind::Complexity | ObservationKind::TodoDensity
                ) && record.adapter_version == METRICS_VERSION
                    && record.head.at_default_tip
            })
            .max_by_key(|row| (row.record.recorded_at, row.id.get()))
            .map(|row| (row.record.head.commit.clone(), adapters.enabled));
        let (base, cached_tick) = match cached_base(&store, &terrarium)? {
            Some((base, tick)) => (Some(base), Some(tick)),
            None => (None, None),
        };
        let showing = Showing::after(base.as_ref().map(|base| base.events.keys()));
        let remembered = Remembered::read(&store)?;
        Ok(Some(Live {
            status_file: config.data.local_tests(&terrarium.seed),
            patterns: discovery::report_patterns(&adapters.overrides, adapters.enabled),
            store,
            terrarium,
            clock: Arc::clone(&config.clock),
            progress_after: config.progress_after,
            adapters,
            status_dir,
            news,
            verified,
            history: None,
            observations,
            newest,
            metrics: Metrics::new(),
            measured,
            knip_weighed: None,
            found: Found::default(),
            files: FileCache::default(),
            base,
            cached_tick,
            view: None,
            shown: None,
            showing,
            remembered,
            stored_seen: None,
            now,
            watch: None,
        }))
    }

    /// What the strip starts from.
    pub fn start(&self) -> Start {
        Start::Connected {
            terrarium: Box::new(self.terrarium.clone()),
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
        // Every refresh that reads the reports reads the adapters' settings first, so a change
        // is taken in whichever refresh comes next: the one asked for, or a retry or a watch
        // batch that the scheduler ran in its place.
        if refresh.reports {
            self.reload_adapters()?;
        }
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
            let record = observe::ref_rewrite(&branch, moved, head.clone(), self.now);
            self.append(None, record)?;
            self.news.history_rewritten = true;
            recorded = true;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(Fault::Cancelled);
        }
        let root = git.workdir().map(Path::to_path_buf);
        let observed = self.observe(&git, root.as_deref(), &head, refresh.reports, cancel)?;
        recorded |= observed.recorded;
        let follow_up = observed.follow_up;
        let adapters_done = Instant::now();
        let history = self.history.as_ref().ok_or(MISSING_HISTORY)?;
        let through = history.grid.tick_at(self.now);
        let unchanged = !history_changed
            && !recorded
            && self
                .view
                .as_ref()
                .is_some_and(|view| view.through == through);
        let (replay, mut problem) = if unchanged {
            (Replay::Unchanged, None)
        } else {
            self.replay(through)?
        };
        if let Some(unmeasured) = observed.problem {
            problem.get_or_insert(unmeasured);
        }
        let scene = self.scene().ok_or(MISSING_VIEW)?;
        if let Err(err) = self.remember_seen() {
            problem.get_or_insert(err.into());
        }
        let details = self.details(&head)?;
        let view = self.view.as_ref().ok_or(MISSING_VIEW)?;
        let shown = Shown {
            scene,
            report: view.live.report(),
            details,
            branch: match head.branch {
                Some(name) => Branch::Named(name),
                None => Branch::Detached(head.commit),
            },
        };
        let stats = RefreshStats {
            cause: refresh.cause,
            replay,
            working: history_changed || recorded,
            adapters_to_snapshot: adapters_done.elapsed(),
        };
        let snapshot = self.snapshot(&shown, stats, &mut problem);
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

    /// Takes the adapters' settings in again.
    fn reload_adapters(&mut self) -> Result<(), StoreError> {
        let adapters = stored_adapters(&self.store)?;
        if adapters != self.adapters {
            self.patterns = discovery::report_patterns(&adapters.overrides, adapters.enabled);
            self.adapters = adapters;
        }
        Ok(())
    }

    /// Runs the adapters: the source at the default tip, then, when `reports` asks, the reports
    /// under `root`; records what is new.
    fn observe(
        &mut self,
        git: &GitRepo,
        root: Option<&Path>,
        head: &ObservationHead,
        reports: bool,
        cancel: &AtomicBool,
    ) -> Result<Observed, Fault> {
        let mut observed = Observed::default();
        observed.take(self.measure_tip(git, head, cancel)?);
        if reports {
            let read = self.read_reports(root, head);
            self.found = read.found;
            for (format, record) in read.records {
                self.append(Some(format), record)?;
                observed.recorded = true;
            }
            if let Some(knip) = read.knip {
                observed.take(self.weigh_dead_code(git, &knip, head, cancel)?);
            }
            observed.follow_up = sooner(observed.follow_up, read.wait);
        }
        Ok(observed)
    }

    /// Measures the committed source at the default tip and records its complexity and TODO
    /// density, once per tip and only while HEAD is at it, with HEAD's head; a metric switched
    /// on since is recorded at the same tip on its own. A tip whose tree cannot be read is not
    /// tried again until it moves.
    fn measure_tip(
        &mut self,
        git: &GitRepo,
        head: &ObservationHead,
        cancel: &AtomicBool,
    ) -> Result<Metered, Fault> {
        let on = self.adapters.enabled;
        let done = match &self.measured {
            Some((tip, under)) if *tip == head.commit => *under,
            _ => Enabled {
                complexity: false,
                todo_density: false,
                ..on
            },
        };
        let due = Enabled {
            complexity: on.complexity && !done.complexity,
            todo_density: on.todo_density && !done.todo_density,
            ..on
        };
        if !(due.complexity || due.todo_density) || !head.at_default_tip {
            return Ok(Metered::Unchanged);
        }
        if self.view.is_none() {
            return Ok(Metered::Deferred);
        }
        let measured = Enabled {
            complexity: done.complexity || on.complexity,
            todo_density: done.todo_density || on.todo_density,
            ..on
        };
        let measurement = match self.measurement(git, &head.commit, cancel)? {
            Ok(measurement) => measurement,
            Err(problem) => {
                self.measured = Some((head.commit.clone(), measured));
                return Ok(Metered::Failed(problem));
            }
        };
        self.found.source_code = Some(measurement.has_source());
        // Replay orders rows of one second by their commit, so a newer tip's reading is dated
        // after the last one, as a repeated key is.
        let last = self
            .observations
            .iter()
            .filter(|o| {
                matches!(
                    o.value.kind(),
                    ObservationKind::Complexity | ObservationKind::TodoDensity
                )
            })
            .map(|o| o.recorded_at)
            .max();
        let at = last.map_or(self.now, |last| self.now.max(last + 1));
        let mut metered = Metered::Unchanged;
        for record in observe::metrics(&measurement, due, head, at) {
            self.append(None, record)?;
            metered = Metered::Recorded;
        }
        self.measured = Some((head.commit.clone(), measured));
        Ok(metered)
    }

    /// Records the dead code a Knip `report` makes against the exports counted in HEAD's
    /// commit, once per report and commit, recorded or not.
    fn weigh_dead_code(
        &mut self,
        git: &GitRepo,
        report: &Report,
        head: &ObservationHead,
        cancel: &AtomicBool,
    ) -> Result<Metered, Fault> {
        if self.view.is_none() {
            return Ok(Metered::Deferred);
        }
        let Some(known) = report.known() else {
            return Ok(Metered::Unchanged);
        };
        self.knip_weighed = Some((known.content_hash, head.commit.clone()));
        let measurement = match self.measurement(git, &head.commit, cancel)? {
            Ok(measurement) => measurement,
            Err(problem) => return Ok(Metered::Failed(problem)),
        };
        let Some(record) = observe::dead_code(report, &measurement, head.clone(), self.now) else {
            return Ok(Metered::Unchanged);
        };
        self.append(Some(Format::Knip), record)?;
        Ok(Metered::Recorded)
    }

    /// The source files of `commit`, parsing only those not met before. A stop is a cancel;
    /// any other error is a problem the refresh carries.
    fn measurement(
        &mut self,
        git: &GitRepo,
        commit: &CommitId,
        cancel: &AtomicBool,
    ) -> Result<Result<Measurement, Problem>, Fault> {
        match self
            .metrics
            .measure(git, commit, &mut || cancel.load(Ordering::Relaxed))
        {
            Ok(measurement) => Ok(Ok(measurement)),
            Err(GitError::Cancelled) => Err(Fault::Cancelled),
            Err(err) => Ok(Err(Problem::Git(err))),
        }
    }

    /// What the panel and the hover labels say of the state shown, with HEAD at `head`.
    fn details(&mut self, head: &ObservationHead) -> Result<Details, Fault> {
        let Live {
            store,
            terrarium,
            history,
            observations,
            found,
            files,
            view,
            ..
        } = self;
        let view = view.as_ref().ok_or(MISSING_VIEW)?;
        let history = history.as_ref().ok_or(MISSING_HISTORY)?;
        let sources = Sources {
            state: &view.live,
            grid: history.grid,
            commits: &history.commits,
            observations,
            head,
            found: *found,
        };
        let terrarium = terrarium.id;
        Ok(details::details(&sources, files, &mut |id| {
            store.contributors(terrarium, ObservationId::new(id))
        })?)
    }

    /// The snapshot of `shown`, when the strip needs one: it shows something new, or the
    /// refresh read new history or recorded something. It carries the news held; a simulation
    /// note it carries is recorded as given for this version.
    fn snapshot(
        &mut self,
        shown: &Shown,
        stats: RefreshStats,
        problem: &mut Option<Problem>,
    ) -> Option<Snapshot> {
        if !stats.working && self.shown.as_ref() == Some(shown) {
            return None;
        }
        let notes = std::mem::take(&mut self.news);
        if notes.sim_updated
            && let Err(err) = self
                .store
                .set_install_value(InstallKey::LastSimVersion, &SIM_VERSION.0.to_string())
        {
            problem.get_or_insert(err.into());
        }
        Some(Snapshot {
            scene: shown.scene.clone(),
            report: shown.report.clone(),
            details: shown.details.clone(),
            branch: shown.branch.clone(),
            notes,
            stats,
        })
    }

    /// The view's scene, playing the event [`Showing`] chooses. What an earlier session's
    /// scenes played counts as seen once the history it is keyed to is read.
    fn scene(&mut self) -> Option<SceneInput> {
        let view = self.view.as_ref()?;
        if let (Some(remembered), Some(history)) = (self.remembered.take(), &self.history)
            && let Some(seen) = remembered.seen_for(
                &self.terrarium.seed,
                &self.terrarium.repo.default_branch,
                history.grid,
            )
        {
            self.showing.also_seen(seen);
        }
        Some(self.showing.scene(&view.live))
    }

    /// Stores what the scenes have had the chance to play, when it changed.
    fn remember_seen(&mut self) -> Result<(), StoreError> {
        let (Some(seen), Some(history)) = (self.showing.seen(), &self.history) else {
            return Ok(());
        };
        if self.stored_seen.as_ref() == Some(seen) {
            return Ok(());
        }
        Remembered::new(
            &self.terrarium.seed,
            &self.terrarium.repo.default_branch,
            history.grid,
            seen.clone(),
        )
        .write(&mut self.store)?;
        self.stored_seen = Some(seen.clone());
        Ok(())
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
            // first-parent chain without the branch fast-forwarding past it. What it took off
            // is read back from the object database, which keeps it until it is collected.
            let moved = match self
                .base
                .as_ref()
                .and_then(|base| base.habitat.default_tip.clone())
                .filter(|tip| !history.on_default_chain(tip))
                .zip(history.state.branches.get(branch).cloned())
                .filter(|(old, new)| !history.is_ancestor(old, new))
            {
                Some((old, new)) => {
                    // Its own pace, not the history's: the read's percent is not shown again.
                    let mut stop = |_: Progress| {
                        if cancel.load(Ordering::Relaxed) {
                            ControlFlow::Break(())
                        } else {
                            ControlFlow::Continue(())
                        }
                    };
                    let left = read_back(git.left_between(&old, &history, &mut stop))?;
                    Some(observe::Rewrite { old, new, left })
                }
                None => None,
            };
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
                        left: Some(left_chain.clone()),
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

    /// Reads the coverage, test and Knip reports and the status file that have been still long
    /// enough, and returns what is new among them, when to look again at the rest, and which
    /// reports the worktree has.
    fn read_reports(&self, root: Option<&Path>, head: &ObservationHead) -> ReportsRead {
        let mut wait: Option<Duration> = None;
        let enabled = self.adapters.enabled;
        let mut found = Found {
            enabled,
            source_code: self.found.source_code,
            ..Found::default()
        };
        let mut knip = None;
        let mut defer = |left: Duration| wait = Some(wait.map_or(left, |w| w.min(left)));
        let mut reports: Vec<Report> = Vec::new();
        // Reports live in the worktree; the status file lives in the data directory, so a bare
        // repository has one too. An adapter the settings have off reads nothing.
        if let Some(root) = root {
            let located = discovery::locate_reports(root, &self.adapters.overrides);
            found.coverage = located.coverage.located().is_some();
            found.test_report = located.test_report.located().is_some();
            found.dead_code = located.dead_code.located().is_some();
            if let Some(located) = located.dead_code.located().filter(|_| enabled.dead_code) {
                match unsettled(located.files.iter().map(|file| root.join(file))) {
                    Some(left) => defer(left),
                    None => knip = Some(Report::read(root, located.format, &located.files)),
                }
            }
            for located in [
                located.coverage.located().filter(|_| enabled.coverage),
                located.test_report.located().filter(|_| enabled.weather),
            ]
            .into_iter()
            .flatten()
            {
                match unsettled(located.files.iter().map(|file| root.join(file))) {
                    Some(left) => defer(left),
                    None => reports.push(Report::read(root, located.format, &located.files)),
                }
            }
        }
        if enabled.weather && self.status_file.is_file() {
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
        // A Knip report is weighed when it is new: unlike the stored row's, and unlike the one
        // weighed last at this commit.
        let knip = knip.filter(|report| {
            report.known().is_some_and(|known| {
                let hash = known.content_hash;
                let stored = self
                    .newest
                    .get(&Format::Knip)
                    .is_some_and(|newest| newest.hash == hash);
                let weighed = self
                    .knip_weighed
                    .as_ref()
                    .is_some_and(|(weighed, at)| *weighed == hash && *at == head.commit);
                !stored && !weighed
            })
        });
        ReportsRead {
            records: new
                .into_iter()
                .map(|(_, format, record)| (format, record))
                .collect(),
            wait,
            found,
            knip,
        }
    }

    /// Whether an observation held has `record`'s merge key. The key is built once, and only an
    /// observation at the same second builds its own.
    fn holds_key_of(&self, record: &ObservationRecord) -> bool {
        let key = merge_key(
            record.recorded_at,
            record.kind,
            record.source.as_ref(),
            &record.head.commit,
        );
        self.observations
            .iter()
            .any(|o| o.recorded_at == record.recorded_at && o.merge_key() == key)
    }

    /// Stores `record` with a `recorded_at` no stored observation of its kind, source and head
    /// already has, and keeps it for replay.
    fn append(
        &mut self,
        producer: Option<Format>,
        mut record: ObservationRecord,
    ) -> Result<(), Fault> {
        while self.holds_key_of(&record) {
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
    let path = lifecycle::absolute(path)?;
    let (default_branch, identity) = GitRepo::open(&path)?.default_identity()?;
    let seed = identity.seed;
    let location = lifecycle::location(&path, default_branch);
    let id = if let Some(known) = store.terrarium_by_seed(&seed)? {
        store.activate(known.id, &location)?;
        known.id
    } else {
        store.create_terrarium(&NewTerrarium {
            seed,
            repo: location,
            install_salt: lifecycle::new_salt()?,
            connected_at: now,
        })?
    };
    store.set_setting(SETTING_REPO_ACTIVE, &id.get())?;
    Ok(store
        .terrarium(id)?
        .ok_or(StoreError::NoSuchTerrarium(id))?)
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
    fn a_move_that_cannot_be_read_back_is_unknown_and_only_a_stop_ends_the_refresh() {
        let left = vec![LeftCommit {
            commit: CommitId::from_hex(&"a".repeat(40)).unwrap(),
            time: 0,
            deleted_dirs: Vec::new(),
        }];
        assert!(matches!(read_back(Ok(Some(left.clone()))), Ok(Some(l)) if l == left));
        assert!(matches!(read_back(Ok(None)), Ok(None)));
        assert!(matches!(
            read_back(Err(GitError::Read("a tree is gone".into()))),
            Ok(None)
        ));
        assert!(matches!(read_back(Err(GitError::NoCommits)), Ok(None)));
        assert!(matches!(
            read_back(Err(GitError::Cancelled)),
            Err(Fault::Cancelled)
        ));
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
