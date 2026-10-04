//! The git history reader: gix in process, never the git command line.
//!
//! A repository is opened read-only and isolated: no system, global or user configuration, no
//! environment, no includes and no replace refs. Nothing it holds can run: diffs read blobs from
//! the object database with no filters, text conversion, external diff or attributes, and
//! nothing here writes into the repository.

mod diff;
mod history;
mod refs;
#[cfg(test)]
mod tests;
mod walk;

use std::fmt;
use std::path::Path;

use terrarium_truth::{CommitId, RepoSeed};

pub use history::{Change, History, HistoryNotes, HistoryUpdate, Progress, RewriteReason};
pub use refs::RepoState;

/// Bytes of decoded objects kept in memory while reading: trees are read again for every
/// commit that touches them.
const OBJECT_CACHE_BYTES: usize = 32 * 1024 * 1024;
/// Bytes of resolved pack deltas kept while reading a packed repository.
const PACK_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// What went wrong reading a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// The path is not a git repository gix can open.
    Open(String),
    /// The repository has no commit on any local branch.
    NoCommits,
    /// Neither `origin/HEAD`, `main`, `master` nor `HEAD` names a local branch.
    NoDefaultBranch,
    /// The recorded default branch does not exist (any more).
    BranchMissing(String),
    /// A SHA-256 repository: this build reads SHA-1 repositories only.
    UnsupportedObjectFormat,
    /// The progress callback asked to stop.
    Cancelled,
    /// gix failed while reading.
    Read(String),
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitError::Open(why) => write!(f, "cannot open the repository: {why}"),
            GitError::NoCommits => f.write_str("the repository has no commits"),
            GitError::NoDefaultBranch => f.write_str("no default branch"),
            GitError::BranchMissing(name) => write!(f, "branch '{name}' does not exist"),
            GitError::UnsupportedObjectFormat => f.write_str("SHA-256 repositories are not read"),
            GitError::Cancelled => f.write_str("cancelled"),
            GitError::Read(why) => write!(f, "reading the repository: {why}"),
        }
    }
}

impl std::error::Error for GitError {}

/// Turns any gix error into [`GitError::Read`].
pub(crate) fn read_err(err: impl fmt::Display) -> GitError {
    GitError::Read(err.to_string())
}

/// What a terrarium is keyed by, read once at connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoIdentity {
    /// Where the first-parent walk from the default tip ends.
    pub root: CommitId,
    pub seed: RepoSeed,
    /// The walk ended at a shallow boundary, which stands in for the root.
    pub shallow: bool,
}

/// An open repository.
pub struct GitRepo {
    repo: gix::Repository,
}

impl GitRepo {
    /// Opens the repository at `path` (its worktree or its git directory), read-only and
    /// isolated.
    pub fn open(path: &Path) -> Result<GitRepo, GitError> {
        let options = gix::open::Options::isolated()
            // Replace refs are install-local and would make two clones disagree. They are looked
            // up under a namespace nothing writes to, which holds however gix reads
            // `core.useReplaceRefs` (0.88 reads its value inverted).
            .config_overrides(["gitoxide.objects.replaceRefBase=refs/terrarium/no-replacements/"]);
        let mut repo =
            gix::open_opts(path, options).map_err(|err| GitError::Open(err.to_string()))?;
        if repo.object_hash() != gix::hash::Kind::Sha1 {
            return Err(GitError::UnsupportedObjectFormat);
        }
        repo.object_cache_size(OBJECT_CACHE_BYTES);
        repo.objects.set_pack_cache(|| {
            Box::new(gix::odb::pack::cache::lru::MemoryCappedHashmap::new(
                PACK_CACHE_BYTES,
            ))
        });
        Ok(GitRepo { repo })
    }

    /// The branch climate follows, chosen at connect: the local branch `origin/HEAD` points at,
    /// then `main`, then `master`, then the branch `HEAD` is on.
    pub fn default_branch(&self) -> Result<String, GitError> {
        refs::default_branch(&self.repo)
    }

    /// The root and seed, by a first-parent walk from the tip of `default_branch`.
    pub fn identity(&self, default_branch: &str) -> Result<RepoIdentity, GitError> {
        walk::identity(&self.repo, default_branch)
    }

    /// The local branches, tags and shallow boundary as they are now.
    pub fn state(&self) -> Result<RepoState, GitError> {
        refs::state(&self.repo)
    }

    /// Reads every commit reachable from a local branch into [`CommitFacts`].
    ///
    /// [`CommitFacts`]: terrarium_truth::CommitFacts
    pub fn read_history(
        &self,
        default_branch: &str,
        progress: &mut dyn FnMut(Progress) -> std::ops::ControlFlow<()>,
    ) -> Result<History, GitError> {
        history::read(&self.repo, default_branch, progress)
    }

    /// Brings `previous` up to date, reading only new commits when the repository grew and
    /// everything again when its history changed.
    pub fn read_since(
        &self,
        previous: &History,
        default_branch: &str,
        progress: &mut dyn FnMut(Progress) -> std::ops::ControlFlow<()>,
    ) -> Result<HistoryUpdate, GitError> {
        history::read_since(&self.repo, previous, default_branch, progress)
    }
}
