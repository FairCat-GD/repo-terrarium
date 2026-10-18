//! The adapters of a refresh: the committed source measured at the default tip, and the
//! reports read once they are still, each recorded only when it is new.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use terrarium_adapters::discovery::{self, Enabled};
use terrarium_adapters::git::{GitError, GitRepo};
use terrarium_adapters::local_tests;
use terrarium_adapters::metrics::Measurement;
use terrarium_adapters::report::{Format, Report};
use terrarium_store::{ObservationRecord, StoreError};
use terrarium_truth::CommitId;
use terrarium_truth::domain::ObservationHead;

use super::events::knip_run;
use super::{Fault, Live, Problem};
use crate::details::{DeadCodeFrom, Found};
use crate::knip::KnipStatus;
use crate::lifecycle::{AdapterSettings, stored_adapters};
use crate::observe;
use crate::schedule::DEBOUNCE;

/// The newest stored observation a report format wrote: the largest (`recorded_at`, row id).
#[derive(Clone, Copy, Debug)]
pub(super) struct Newest {
    pub(super) at: (i64, i64),
    pub(super) hash: [u8; 32],
}

/// What measuring the source did in a refresh.
pub(super) enum Metered {
    /// Nothing to measure, or nothing known to record.
    Unchanged,
    Recorded,
    /// Left to a follow-up: the session has no view yet.
    Deferred,
    /// The commit's tree could not be read; the refresh goes on without it.
    Failed(Problem),
}

/// The measurement a refresh made, and the commit it is of.
pub(super) type Held = Option<(CommitId, Measurement)>;

/// What the adapters of one refresh did.
#[derive(Default)]
pub(super) struct Observed {
    pub(super) recorded: bool,
    pub(super) follow_up: Option<Duration>,
    /// What kept the source from being measured.
    pub(super) problem: Option<Problem>,
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

impl Live {
    /// Takes the adapters' settings in again. An opt-in to run Knip just given runs it at the
    /// tip even where a report's dead code was recorded.
    pub(super) fn reload_adapters(&mut self) -> Result<(), StoreError> {
        let adapters = stored_adapters(&self.store)?;
        if adapters != self.adapters {
            let ran = knip_run(&self.adapters, &self.terrarium).is_some();
            let runs = knip_run(&adapters, &self.terrarium).is_some();
            if runs && !ran {
                self.knip.forced = true;
                self.knip.status = KnipStatus::Waiting;
            }
            self.patterns = patterns(&adapters, runs);
            self.adapters = adapters;
        }
        Ok(())
    }

    /// Runs the adapters: the source at the default tip, then, when `reports` asks, the reports
    /// under `root`; records what is new.
    pub(super) fn observe(
        &mut self,
        git: &GitRepo,
        root: Option<&Path>,
        head: &ObservationHead,
        reports: bool,
        cancel: &AtomicBool,
    ) -> Result<Observed, Fault> {
        let mut observed = Observed::default();
        let mut measured = None;
        observed.take(self.measure_tip(git, head, cancel, &mut measured)?);
        observed.take(self.record_knip(git, root, head, cancel, &mut measured)?);
        if reports {
            let read = self.read_reports(root, head);
            self.found = read.found;
            for (format, record) in read.records {
                self.append(Some(format), record)?;
                observed.recorded = true;
            }
            if let Some(knip) = read.knip {
                observed.take(self.weigh_dead_code(git, &knip, head, cancel, &mut measured)?);
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
        source: &mut Held,
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
        if let Err(problem) = self.measure(git, &head.commit, cancel, source)? {
            self.measured = Some((head.commit.clone(), measured));
            return Ok(Metered::Failed(problem));
        }
        let Some((_, measurement)) = source.as_ref() else {
            return Err(Fault::Problem(Problem::Missing("measurement")));
        };
        self.found.source_code = Some(measurement.has_source());
        let mut metered = Metered::Unchanged;
        for record in observe::metrics(measurement, due, head, self.now) {
            self.append(None, record)?;
            metered = Metered::Recorded;
        }
        self.measured = Some((head.commit.clone(), measured));
        Ok(metered)
    }

    /// Records the dead code a Knip `report` makes against the exports counted in HEAD's
    /// commit, once per report and commit, recorded or not.
    pub(super) fn weigh_dead_code(
        &mut self,
        git: &GitRepo,
        report: &Report,
        head: &ObservationHead,
        cancel: &AtomicBool,
        source: &mut Held,
    ) -> Result<Metered, Fault> {
        if self.view.is_none() {
            return Ok(Metered::Deferred);
        }
        let Some(known) = report.known() else {
            return Ok(Metered::Unchanged);
        };
        self.knip_weighed = Some((known.content_hash, head.commit.clone()));
        if let Err(problem) = self.measure(git, &head.commit, cancel, source)? {
            return Ok(Metered::Failed(problem));
        }
        let Some((_, measurement)) = source.as_ref() else {
            return Err(Fault::Problem(Problem::Missing("measurement")));
        };
        let Some(record) = observe::dead_code(report, measurement, head.clone(), self.now) else {
            return Ok(Metered::Unchanged);
        };
        self.append(Some(Format::Knip), record)?;
        Ok(Metered::Recorded)
    }

    /// Leaves the source files of `commit` in `source`, measured once a refresh: a measurement
    /// already held for it is used again, else only files not met before are parsed. A stop is
    /// a cancel; any other error is a problem the refresh carries.
    fn measure(
        &mut self,
        git: &GitRepo,
        commit: &CommitId,
        cancel: &AtomicBool,
        source: &mut Held,
    ) -> Result<Result<(), Problem>, Fault> {
        if source.as_ref().is_some_and(|(at, _)| at == commit) {
            return Ok(Ok(()));
        }
        match self
            .metrics
            .measure(git, commit, &mut || cancel.load(Ordering::Relaxed))
        {
            Ok(measurement) => {
                *source = Some((commit.clone(), measurement));
                Ok(Ok(()))
            }
            Err(GitError::Cancelled) => Err(Fault::Cancelled),
            Err(err) => Ok(Err(Problem::Git(err))),
        }
    }

    /// Reads the coverage, test and Knip reports and the status file that have been still long
    /// enough, and returns what is new among them, when to look again at the rest, and which
    /// reports the worktree has.
    /// The content hash of the newest report of `format` recorded: one that hashes the same
    /// is not read again.
    fn seen(&self, format: Format) -> Option<&[u8; 32]> {
        self.newest.get(&format).map(|newest| &newest.hash)
    }

    fn read_reports(&self, root: Option<&Path>, head: &ObservationHead) -> ReportsRead {
        let mut wait: Option<Duration> = None;
        let enabled = self.adapters.enabled;
        // While Knip runs here, its output is the one dead-code report.
        let knip_runs = knip_run(&self.adapters, &self.terrarium).is_some();
        let mut found = Found {
            enabled,
            source_code: self.found.source_code,
            dead_code: if knip_runs {
                DeadCodeFrom::KnipRun
            } else {
                DeadCodeFrom::Nothing
            },
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
            if !knip_runs && located.dead_code.located().is_some() {
                found.dead_code = DeadCodeFrom::Report;
            }
            if let Some(located) = located
                .dead_code
                .located()
                .filter(|_| enabled.dead_code && !knip_runs)
            {
                match unsettled(located.files.iter().map(|file| root.join(file))) {
                    Some(left) => defer(left),
                    None => {
                        knip = Report::read_unless(
                            root,
                            located.format,
                            &located.files,
                            self.seen(located.format),
                        );
                    }
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
                    None => reports.extend(Report::read_unless(
                        root,
                        located.format,
                        &located.files,
                        self.seen(located.format),
                    )),
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
        let knip = knip.filter(|report| self.knip_is_new(report, head));
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
}

/// The reports the watcher watches: while Knip runs, its report file is not one of them.
pub(super) fn patterns(adapters: &AdapterSettings, knip_runs: bool) -> Vec<String> {
    let enabled = Enabled {
        dead_code: adapters.enabled.dead_code && !knip_runs,
        ..adapters.enabled
    };
    discovery::report_patterns(&adapters.overrides, enabled)
}

/// How much longer the newest of `files` must stay untouched before it is read, or `None` when
/// it has been still for [`DEBOUNCE`]. A file modified in the future, or gone, waits for nothing.
pub(super) fn unsettled(files: impl Iterator<Item = PathBuf>) -> Option<Duration> {
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

/// Where the reports are read: the folder the person connected, when the repository has a
/// worktree. Never where the repository's own config places its worktree (`core.worktree`),
/// which can name any folder on the disk; discovery and the Knip run take the same folder.
pub(super) fn report_root(git: &GitRepo, connected: &Path) -> Option<PathBuf> {
    git.workdir().map(|_| connected.to_path_buf())
}
