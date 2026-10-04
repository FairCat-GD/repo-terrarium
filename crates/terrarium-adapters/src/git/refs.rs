//! References: the default branch, and the state of local branches, tags and the shallow
//! boundary. Remote branches never count.

use std::collections::{BTreeMap, BTreeSet};

use gix::bstr::ByteSlice;
use gix::refs::TargetRef;
use sha2::{Digest, Sha256};
use terrarium_truth::CommitId;

use super::{GitError, read_err};

/// What decides a repository's history apart from its objects. Two equal states over the same
/// objects read to the same history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoState {
    /// Local branch name (without `refs/heads/`) to the commit at its tip.
    pub branches: BTreeMap<String, CommitId>,
    /// Tag name (without `refs/tags/`) to the object it names: a commit, or an annotated tag.
    pub tags: BTreeMap<String, CommitId>,
    /// Commits whose parents a shallow clone lacks.
    pub shallow: BTreeSet<CommitId>,
}

impl RepoState {
    /// The SHA-256 of a canonical text of the whole state: the part of the replay cache's key
    /// that stands for git.
    pub fn state_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        for (name, id) in &self.branches {
            hasher.update(format!("branch {name} {}\n", id.as_str()));
        }
        for (name, id) in &self.tags {
            hasher.update(format!("tag {name} {}\n", id.as_str()));
        }
        for id in &self.shallow {
            hasher.update(format!("shallow {}\n", id.as_str()));
        }
        hasher.finalize().into()
    }
}

pub(crate) fn to_commit_id(id: gix::ObjectId) -> CommitId {
    CommitId::from_hex(&id.to_hex().to_string()).expect("gix prints lowercase hex")
}

pub(crate) fn to_object_id(id: &CommitId) -> gix::ObjectId {
    gix::ObjectId::from_hex(id.as_str().as_bytes()).expect("a commit id is lowercase hex")
}

pub(crate) fn state(repo: &gix::Repository) -> Result<RepoState, GitError> {
    let platform = repo.references().map_err(read_err)?;
    let mut branches = BTreeMap::new();
    // A ref that does not parse, or does not resolve to an object the repository has, holds no
    // history: it is skipped, as git skips a dangling ref, rather than failing the whole read.
    for reference in platform.local_branches().map_err(read_err)? {
        let Ok(mut reference) = reference else {
            continue;
        };
        let name = reference.name().shorten().to_str_lossy().into_owned();
        let Ok(id) = reference.peel_to_id() else {
            continue;
        };
        let id = id.detach();
        // A branch names a commit; one that names anything else holds no history.
        let is_commit = repo
            .try_find_header(id)
            .map_err(read_err)?
            .is_some_and(|header| header.kind() == gix::objs::Kind::Commit);
        if is_commit {
            branches.insert(name, to_commit_id(id));
        }
    }
    let mut tags = BTreeMap::new();
    for reference in platform.tags().map_err(read_err)? {
        let Ok(mut reference) = reference else {
            continue;
        };
        let name = reference.name().shorten().to_str_lossy().into_owned();
        if let Ok(id) = reference.follow_to_object() {
            tags.insert(name, to_commit_id(id.detach()));
        }
    }
    let shallow = repo
        .shallow_commits()
        .map_err(read_err)?
        .map(|commits| commits.iter().copied().map(to_commit_id).collect())
        .unwrap_or_default();
    Ok(RepoState {
        branches,
        tags,
        shallow,
    })
}

pub(crate) fn default_branch(repo: &gix::Repository) -> Result<String, GitError> {
    let branches = state(repo)?.branches;
    if branches.is_empty() {
        return Err(GitError::NoCommits);
    }
    let origin_head = repo
        .try_find_reference("refs/remotes/origin/HEAD")
        .map_err(read_err)?
        .and_then(|reference| match reference.target() {
            TargetRef::Symbolic(name) => name
                .as_bstr()
                .strip_prefix(b"refs/remotes/origin/")
                .map(|short| short.to_str_lossy().into_owned()),
            TargetRef::Object(_) => None,
        });
    let head = repo.head_name().map_err(read_err)?.and_then(|name| {
        name.as_bstr()
            .strip_prefix(b"refs/heads/")
            .map(|short| short.to_str_lossy().into_owned())
    });
    [
        origin_head,
        Some("main".to_owned()),
        Some("master".to_owned()),
        head,
    ]
    .into_iter()
    .flatten()
    .find(|name| branches.contains_key(name))
    .ok_or(GitError::NoDefaultBranch)
}
