//! A replay with no window, no watcher and no cache: read a repository's history, take the
//! observations from an export, and dump what truth makes of both, or the scene of each day as
//! a strip looking once a day would show it, or the days a time-lapse plays.
//! `repo-terrarium replay`, the golden replay, `cargo xtask arc` and `cargo xtask hero` run
//! through here.
//!
//! It writes only into the data directory it is given, which its callers make temporary, so
//! it never touches a person's stored observations.

use std::io::BufRead;
use std::ops::ControlFlow;
use std::path::Path;

use terrarium_adapters::git::{GitError, GitRepo};
use terrarium_store::{
    DataDir, ImportError, NewTerrarium, RepoLocation, Store, StoreError, StoredObservation,
};
use terrarium_truth::timeline::TimelineEntry;
use terrarium_truth::{Basis, Prepared, ReplayDump, ReplayError, SceneInput, Tick};

use crate::timelapse::Lapse;

/// One headless replay.
pub struct Request<'a> {
    pub repo: &'a Path,
    /// Where the temporary store lives.
    pub data: &'a DataDir,
    /// An observations export (JSONL) to replay with; without one, light, soil and weather
    /// stay unknown.
    pub observations: Option<&'a mut dyn BufRead>,
    /// The moment to replay through, as Unix seconds, UTC.
    pub at_secs: i64,
    /// Cosmetic only, so it changes nothing in the dump (I-2).
    pub install_salt: [u8; 16],
}

#[derive(Debug)]
pub enum HeadlessError {
    Git(GitError),
    Store(StoreError),
    Import(ImportError),
    Replay(ReplayError),
    /// The moment asked for is before the repository's first commit.
    BeforeHistory,
}

impl From<GitError> for HeadlessError {
    fn from(err: GitError) -> HeadlessError {
        HeadlessError::Git(err)
    }
}

impl From<StoreError> for HeadlessError {
    fn from(err: StoreError) -> HeadlessError {
        HeadlessError::Store(err)
    }
}

impl From<ImportError> for HeadlessError {
    fn from(err: ImportError) -> HeadlessError {
        HeadlessError::Import(err)
    }
}

impl From<ReplayError> for HeadlessError {
    fn from(err: ReplayError) -> HeadlessError {
        HeadlessError::Replay(err)
    }
}

/// The repository at `request.repo` and its observations, read for a time-lapse through
/// `request.at_secs`.
pub fn lapse(request: Request<'_>) -> Result<Lapse, HeadlessError> {
    let git = GitRepo::open(request.repo)?;
    let (default_branch, identity) = git.default_identity()?;
    let history = git.read_history(&default_branch, &mut |_| ControlFlow::Continue(()))?;
    let through = history
        .grid
        .tick_at(request.at_secs)
        .ok_or(HeadlessError::BeforeHistory)?;
    let mut store = Store::open(request.data)?;
    let terrarium = store.create_terrarium(&NewTerrarium {
        seed: identity.seed,
        repo: RepoLocation {
            path: request.repo.display().to_string(),
            name: request
                .repo
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            default_branch: default_branch.clone(),
        },
        install_salt: request.install_salt,
        connected_at: request.at_secs,
    })?;
    if let Some(input) = request.observations {
        store.import_observations(terrarium, input)?;
    }
    let observations = store
        .observations(terrarium, None)?
        .iter()
        .map(StoredObservation::to_observation)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Lapse {
        basis: Basis::new(identity.seed, history.grid, &history.default_branch),
        commits: history.commits,
        observations,
        through,
    })
}

/// Replays the repository at `request.repo` from its first commit through `request.at_secs`.
pub fn replay_dump(request: Request<'_>) -> Result<ReplayDump, HeadlessError> {
    let inputs = lapse(request)?;
    let prepared = Prepared::new(inputs.basis, &inputs.commits, &inputs.observations)?;
    Ok(terrarium_truth::dump(&prepared, inputs.through)?)
}

/// A replay day by day: each day's scene at its close, as a strip that looks once a day shows
/// it, playing the event the live view would choose; and the presentation timeline at the end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Days {
    pub days: Vec<(Tick, SceneInput)>,
    pub timeline: Vec<TimelineEntry>,
}

/// Replays the repository at `request.repo` a day at a time through `request.at_secs`.
pub fn daily_scenes(request: Request<'_>) -> Result<Days, HeadlessError> {
    let lapse = lapse(request)?;
    let closes = lapse.closes();
    let (scenes, state) = lapse.replay(&closes)?;
    Ok(Days {
        days: closes.into_iter().zip(scenes).collect(),
        timeline: state.timeline.entries,
    })
}
