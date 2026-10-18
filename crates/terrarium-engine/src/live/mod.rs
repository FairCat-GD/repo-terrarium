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
//! 5. What came back from a job is recorded with HEAD as this refresh reads it: a Knip run's
//!    report, weighed as a Knip report is, and only while HEAD is still at the tip it ran at;
//!    and a GitHub Actions poll's weather (`observe::ci_weather`), recorded when its runs
//!    changed, or when HEAD's branch is not the one the newest observation recorded, since
//!    truth learns where HEAD is only from a row. Weather that can no longer be read is
//!    recorded as unknown (`observe::ci_unread`), so the local results take over.
//! 6. Replay follows the cache contract in `terrarium_truth::replay`: the base state is the
//!    state before the open hour, resumed when truth accepts the inputs it consumed and replayed
//!    in full otherwise; the live state is a copy of it advanced through the open hour. The base
//!    is cached about once an hour.
//!
//! **Time** is the wall clock's, never lower than it was earlier in the session: a clock set
//! back neither rewinds the scene nor dates an observation before one already applied.
//!
//! **Where it lives.** This module runs a refresh and holds its recorder; `connect` makes the
//! pipeline, `replay` reads the history and replays it, `reports` measures the source and reads
//! the reports, and `events` follows HEAD and the jobs beside the refresh.
//!
//! [`DEBOUNCE`]: crate::schedule::DEBOUNCE

mod connect;
mod events;
mod replay;
mod reports;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use terrarium_adapters::discovery::Enabled;
use terrarium_adapters::git::{GitError, GitRepo, History};
use terrarium_adapters::metrics::Metrics;
use terrarium_adapters::report::Format;
use terrarium_store::{
    InstallKey, ObservationId, ObservationRecord, Store, StoreError, StoredObservation, Terrarium,
};
use terrarium_truth::climate::ClimateReport;
use terrarium_truth::domain::{ObservationHead, merge_key};
use terrarium_truth::{
    CommitId, Observation, ReplayError, SIM_VERSION, SceneInput, SimState, Tick,
};

pub use connect::{ConnectError, EngineConfig, NodeLookup, Start};
pub use events::Tools;
pub use replay::Replay;

use crate::clock::Clock;
use crate::details::{self, Details, FileCache, Found, Sources};
use crate::knip::KnipStatus;
use crate::lifecycle::AdapterSettings;
use crate::observe;
use crate::schedule::{Cause, Reach, Refresh};
use crate::showing::Showing;
use crate::shown::Remembered;
use crate::watch::{Places, WatchSet};
use events::{CiState, KnipState, ci_newest_of};
use replay::View;
use reports::{Newest, report_root};

/// Where `HEAD` is, for the label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Branch {
    Named(String),
    Detached(CommitId),
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

/// What the strip was last sent, minus the stats.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Shown {
    scene: SceneInput,
    report: ClimateReport,
    details: Details,
    branch: Branch,
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
    /// Where HEAD was at the last refresh that reached the repository.
    head: Option<ObservationHead>,
    node: NodeLookup,
    ci: CiState,
    knip: KnipState,
}

impl Live {
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
                // The worker stops a run going while the repository is missing; the tip runs
                // again once it is back.
                if self.knip.running.take().is_some() {
                    self.knip.status = KnipStatus::Waiting;
                }
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
        self.follow_head_and_repository(&git, &head);
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
        let root = report_root(&git, &path);
        let observed = self.observe(&git, root.as_deref(), &head, refresh.reports, cancel)?;
        recorded |= observed.recorded;
        recorded |= self.record_ci(&head)?;
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

    /// Whether an observation at another head is held from the same second. Replay orders the
    /// observations of one second by their merge key, not by when they were recorded, and truth
    /// keeps the last reading of each kind and follows the newest observation's HEAD: a row
    /// recorded at a new head could otherwise lose to the one before it.
    fn ties_head(&self, record: &ObservationRecord) -> bool {
        self.observations
            .iter()
            .any(|o| o.recorded_at == record.recorded_at && o.head != record.head)
    }

    /// Stores `record` with a `recorded_at` no stored observation of its kind, source and head
    /// already has, and none at another head has: a second later each time. Keeps it for
    /// replay.
    fn append(
        &mut self,
        producer: Option<Format>,
        mut record: ObservationRecord,
    ) -> Result<(), Fault> {
        while self.holds_key_of(&record) || self.ties_head(&record) {
            record.recorded_at += 1;
        }
        let id = self.store.append_observation(self.terrarium.id, &record)?;
        let stored = StoredObservation {
            id,
            terrarium: self.terrarium.id,
            record,
        };
        self.observations.push(stored.to_observation()?);
        // As for the reports: what this install just recorded is what the next poll compares
        // with.
        if let Some((branch, newest)) = ci_newest_of(&stored) {
            self.ci.newest.insert(branch, newest);
        }
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
}
