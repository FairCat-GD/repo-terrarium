//! HEAD and the jobs beside the refresh: the GitHub Actions weather and the opt-in Knip run,
//! as this session follows them, and the rows what came back makes.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use terrarium_adapters::binding::RepoBinding;
use terrarium_adapters::ci::{CiError, Slug};
use terrarium_adapters::exec::KnipRun;
use terrarium_adapters::git::GitRepo;
use terrarium_adapters::report::{Format, Reading, Report};
use terrarium_store::raw::CiUnread;
use terrarium_store::{ObservationRecord, StoredObservation, Terrarium};
use terrarium_truth::domain::{ObservationHead, WeatherSource};
use terrarium_truth::{CommitId, ObservationKind};

use super::reports::{Held, Metered};
use super::{Fault, Live};
use crate::ci::{Answered, Ask, CiStatus, Read};
use crate::knip::{KnipCall, KnipDone, KnipJob, KnipStatus};
use crate::lifecycle::AdapterSettings;
use crate::observe;

/// What the settings window says of the tools that run beside the refresh: the Knip run and
/// the GitHub Actions poll.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tools {
    pub knip: KnipStatus,
    pub ci: CiStatus,
}

/// The newest stored weather of one CI source: when, its content hash, and whether it is known.
#[derive(Clone, Copy, Debug)]
pub(super) struct CiNewest {
    pub(super) at: (i64, i64),
    hash: [u8; 32],
    known: bool,
}

/// The GitHub Actions weather, as this session follows it.
#[derive(Default)]
pub(super) struct CiState {
    /// Whether the session polls at all: the app gave it a keychain and a client.
    pub(super) on: bool,
    /// The repository whose runs are asked for, as of the last refresh: the one typed for this
    /// repository, else the one `origin` names.
    pub(super) slug: Option<Slug>,
    /// GitHub answered for a repository earlier in the session (a token was saved), so losing
    /// the repository turns CI off.
    pub(super) read_here: bool,
    /// The newest stored row of each CI source, by branch.
    pub(super) newest: BTreeMap<String, CiNewest>,
    /// A poll's answer with something to record, and whether GitHub is out of reach.
    pub(super) answer: Option<(Answered, bool)>,
    /// HEAD's branch or the repository changed: the worker asks at once.
    pub(super) ask_now: bool,
    pub(super) status: CiStatus,
}

/// The opt-in Knip run, as this session follows it.
#[derive(Default)]
pub(super) struct KnipState {
    /// The tip a run going was started at.
    pub(super) running: Option<CommitId>,
    /// The tip Knip last ran at, whatever came of it: each tip runs once.
    pub(super) ran_at: Option<CommitId>,
    /// The opt-in was given since the tip last ran: it runs at the tip even if a dead-code row
    /// from a report is there.
    pub(super) forced: bool,
    /// A run that came back, waiting for a refresh to record it.
    pub(super) done: Option<KnipDone>,
    pub(super) status: KnipStatus,
}

impl Live {
    /// Notes where HEAD is and which GitHub repository the weather reads; either changing has
    /// the worker poll at once.
    pub(super) fn follow_head_and_repository(&mut self, git: &GitRepo, head: &ObservationHead) {
        if self.head.as_ref().map(|known| &known.branch) != Some(&head.branch) {
            self.ci.ask_now = true;
        }
        self.head = Some(head.clone());
        if !self.ci.on {
            return;
        }
        let here = self.here();
        let typed = self
            .adapters
            .ci_repository
            .as_ref()
            .filter(|typed| typed.binding == here)
            .map(|typed| typed.repo.clone());
        let slug = typed.or_else(|| git.origin_url().and_then(|url| Slug::from_remote_url(&url)));
        if slug != self.ci.slug {
            self.ci.ask_now = true;
        }
        match &slug {
            None => self.ci.status = CiStatus::NoRepository,
            Some(_) if self.ci.status == CiStatus::NoRepository => {
                self.ci.status = CiStatus::Quiet;
            }
            Some(_) => {}
        }
        self.ci.slug = slug;
    }

    /// This repository, as a setting given for it names it.
    fn here(&self) -> RepoBinding {
        RepoBinding::new(&self.terrarium.seed, &self.terrarium.repo.path)
    }
}

/// The jobs that run beside the refresh: the Knip run and the GitHub Actions poll. What the
/// worker asks here only reads or stashes; what came back is recorded by the next refresh.
impl Live {
    /// The poll to send, when a repository is known and HEAD has been read.
    pub fn ci_ask(&self) -> Option<Ask> {
        let slug = self.ci.slug.clone()?;
        let head = self.head.as_ref()?;
        let default = &self.terrarium.repo.default_branch;
        let branches = match &head.branch {
            Some(branch) if branch != default => vec![branch.clone(), default.clone()],
            _ => vec![default.clone()],
        };
        Some(Ask { slug, branches })
    }

    /// Takes in a poll's answer; `outage` says GitHub has been out of reach long enough for its
    /// weather to stop counting. True when it has something to record, kept for the refresh
    /// the worker then runs.
    pub fn ci_answered(&mut self, answered: Answered, outage: bool) -> bool {
        // An answer for a repository no longer asked about says nothing of this one.
        if self.ci.slug.as_ref() != Some(&answered.slug) {
            return false;
        }
        self.ci.status = CiStatus::of(&answered);
        if matches!(answered.read, Read::Decided(_) | Read::Failed(_)) {
            self.ci.read_here = true;
        }
        let Some(head) = self.head.clone() else {
            return false;
        };
        let wanted = !self.ci_rows(&answered.read, outage, &head).is_empty();
        if wanted {
            self.ci.answer = Some((answered, outage));
        }
        wanted
    }

    /// Whether HEAD's branch or the repository changed since the worker last asked.
    pub fn ci_ask_now(&mut self) -> bool {
        std::mem::take(&mut self.ci.ask_now)
    }

    /// What the worker does about Knip now: start it at the default tip when it may run there
    /// and has not, or stop a run no longer wanted.
    pub fn knip_call(&mut self) -> Option<KnipCall> {
        let Some(run) = knip_run(&self.adapters, &self.terrarium) else {
            self.knip.forced = false;
            self.knip.status = KnipStatus::Off;
            return self.knip.running.take().map(|_| KnipCall::Stop);
        };
        if self.knip.status == KnipStatus::Off {
            self.knip.status = KnipStatus::Waiting;
        }
        let head = self.head.clone()?;
        if self.knip.running.is_some() || self.view.is_none() || !head.at_default_tip {
            return None;
        }
        let tip = &head.commit;
        let due = self.knip.ran_at.as_ref() != Some(tip)
            && (self.knip.forced || !self.has_dead_code_at(tip));
        if !due {
            return None;
        }
        self.knip.forced = false;
        let Some(node) = (self.node)(run.root()) else {
            self.knip.ran_at = Some(tip.clone());
            self.knip.status = KnipStatus::NoNode;
            return None;
        };
        self.knip.running = Some(tip.clone());
        self.knip.status = KnipStatus::Running;
        Some(KnipCall::Start(KnipJob { run, node, head }))
    }

    /// Takes in a run that ended; true when it has something to record. A run stopped as no
    /// longer wanted has nothing.
    pub fn knip_ran(&mut self, done: KnipDone) -> bool {
        if self.knip.running.take().is_none() {
            return false;
        }
        self.knip.done = Some(done);
        true
    }

    /// What the settings window says of the Knip run and the GitHub Actions poll.
    pub fn tools(&self) -> Tools {
        Tools {
            knip: self.knip.status.clone(),
            ci: self.ci.status.clone(),
        }
    }

    /// Records what the last poll learned, with HEAD at `head`; and, when the repository whose
    /// runs were read is no longer known, records its CI weather as unknown.
    pub(super) fn record_ci(&mut self, head: &ObservationHead) -> Result<bool, Fault> {
        if !self.ci.on {
            return Ok(false);
        }
        let mut rows = Vec::new();
        if self.ci.slug.is_none() && std::mem::take(&mut self.ci.read_here) {
            rows = self.unread_rows(CiUnread::Off, head);
        }
        if let Some((answered, outage)) = self.ci.answer.take()
            && self.ci.slug.as_ref() == Some(&answered.slug)
        {
            rows.extend(self.ci_rows(&answered.read, outage, head));
        }
        let recorded = !rows.is_empty();
        for row in rows {
            self.append(None, row)?;
        }
        Ok(recorded)
    }

    /// The rows `read` makes with HEAD at `head` (see the module's doc).
    fn ci_rows(&self, read: &Read, outage: bool, head: &ObservationHead) -> Vec<ObservationRecord> {
        let moved = self.head_moved(head);
        match read {
            Read::Decided(Some((branch, verdict))) => {
                // Asked for HEAD's branch or the default one; HEAD has moved on since.
                let default = &self.terrarium.repo.default_branch;
                if head.branch.as_ref() != Some(branch) && branch != default {
                    return Vec::new();
                }
                let same = self
                    .ci
                    .newest
                    .get(branch)
                    .is_some_and(|newest| newest.hash == verdict.hash);
                if same && !moved {
                    return Vec::new();
                }
                vec![observe::ci_weather(branch, verdict, head.clone(), self.now)]
            }
            // No run decides on HEAD's branch or the default one: a row for HEAD's branch says
            // where HEAD is.
            Read::Decided(None) => match &head.branch {
                Some(branch) if moved => vec![observe::ci_unread(
                    branch,
                    CiUnread::NoRun,
                    head.clone(),
                    self.now,
                )],
                _ => Vec::new(),
            },
            Read::NoToken { removed: true } => self.unread_rows(CiUnread::Off, head),
            Read::Failed(CiError::Refused) => self.unread_rows(CiUnread::Refused, head),
            Read::Failed(CiError::NotFound) => self.unread_rows(CiUnread::NotFound, head),
            Read::Failed(_) if outage => self.unread_rows(CiUnread::Unreachable, head),
            Read::NoToken { removed: false } | Read::Keychain(_) | Read::Failed(_) => Vec::new(),
        }
    }

    /// An unknown row for each CI source whose newest row is known: none of them is read now.
    fn unread_rows(&self, why: CiUnread, head: &ObservationHead) -> Vec<ObservationRecord> {
        self.ci
            .newest
            .iter()
            .filter(|(_, newest)| newest.known)
            .map(|(branch, _)| observe::ci_unread(branch, why, head.clone(), self.now))
            .collect()
    }

    /// Whether HEAD at `head` is on another branch (or detached state) than the newest
    /// observation recorded, which is where truth thinks HEAD is; true with no observation.
    fn head_moved(&self, head: &ObservationHead) -> bool {
        self.observations
            .iter()
            .max_by_key(|o| (o.recorded_at, o.id))
            .is_none_or(|newest| newest.head.branch != head.branch)
    }

    /// Records the dead code of the Knip run that came back, with HEAD at `head`: only while
    /// HEAD is still at the tip it ran at, since Knip read the worktree there; otherwise the tip
    /// runs again once HEAD is back. Weighed as a Knip report is, once per output and commit.
    pub(super) fn record_knip(
        &mut self,
        git: &GitRepo,
        root: Option<&Path>,
        head: &ObservationHead,
        cancel: &AtomicBool,
        source: &mut Held,
    ) -> Result<Metered, Fault> {
        let Some(done) = self.knip.done.take() else {
            return Ok(Metered::Unchanged);
        };
        if done.head.commit != head.commit || !head.at_default_tip {
            if !matches!(self.knip.status, KnipStatus::Off) {
                self.knip.status = KnipStatus::Waiting;
            }
            return Ok(Metered::Unchanged);
        }
        // A bare repository has no worktree for Knip to read, nor a `node_modules`.
        let Some(root) = root else {
            self.knip.ran_at = Some(head.commit.clone());
            self.knip.status = KnipStatus::NotInstalled;
            return Ok(Metered::Unchanged);
        };
        if self.view.is_none() {
            self.knip.done = Some(done);
            return Ok(Metered::Deferred);
        }
        self.knip.ran_at = Some(head.commit.clone());
        let bytes = match done.outcome {
            Ok(bytes) => bytes,
            Err(err) => {
                self.knip.status = KnipStatus::of(&err);
                return Ok(Metered::Unchanged);
            }
        };
        let report = Report::from_output(
            root,
            Format::Knip,
            KNIP_OUTPUT,
            bytes,
            self.now,
            head.commit.as_str().as_bytes(),
        );
        self.knip.status = match report.known().map(|known| &known.reading) {
            Some(Reading::DeadCode(counts)) => KnipStatus::Ran {
                unused: counts.unused_exports,
            },
            _ => KnipStatus::NotAReport,
        };
        if !self.knip_is_new(&report, head) {
            return Ok(Metered::Unchanged);
        }
        self.weigh_dead_code(git, &report, head, cancel, source)
    }

    /// Whether a known Knip report is new: unlike the stored row's, and unlike the one weighed
    /// last at HEAD's commit.
    pub(super) fn knip_is_new(&self, report: &Report, head: &ObservationHead) -> bool {
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
    }

    /// Whether dead code was recorded at `tip`, by a run or from a report: the tip has its
    /// reading, and a relaunch does not run Knip there again.
    fn has_dead_code_at(&self, tip: &CommitId) -> bool {
        self.observations
            .iter()
            .any(|o| o.value.kind() == ObservationKind::DeadCode && o.head.commit == *tip)
    }
}

/// What a Knip run's report is called where a report's file name would be.
const KNIP_OUTPUT: &str = "knip --reporter json";

/// Knip, when it may run in `terrarium`'s repository: Dead code is on and the opt-in names it.
pub(super) fn knip_run(adapters: &AdapterSettings, terrarium: &Terrarium) -> Option<KnipRun> {
    let here = RepoBinding::new(&terrarium.seed, &terrarium.repo.path);
    adapters
        .enabled
        .dead_code
        .then(|| KnipRun::permitted(adapters.knip_run.as_ref(), &here))
        .flatten()
}

/// A stored row's CI source's branch and its newest-row facts, if it is CI weather.
pub(super) fn ci_newest_of(row: &StoredObservation) -> Option<(String, CiNewest)> {
    let Some(WeatherSource::Ci { branch }) = &row.record.source else {
        return None;
    };
    Some((
        branch.clone(),
        CiNewest {
            at: (row.record.recorded_at, row.id.get()),
            hash: row.record.content_hash,
            known: row.record.normalized.is_some(),
        },
    ))
}
