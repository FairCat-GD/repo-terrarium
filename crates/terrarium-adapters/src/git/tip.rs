//! The files of one commit, read from the object database alone (I-5): what the source metrics
//! parse. The worktree is never looked at, so what they read is what was committed.

use gix::bstr::BString;
use terrarium_truth::CommitId;
use terrarium_truth::history::{counts_toward_terrain, looks_binary};

use super::{GitError, GitRepo, read_err, refs};

/// A file of a commit's tree: a regular or executable blob at a path terrain counts. Links and
/// submodules are not files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TreeFile {
    /// Slash-separated, relative to the repository root.
    pub path: BString,
    pub id: gix::ObjectId,
}

/// One blob as it was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Blob {
    Text(Vec<u8>),
    /// Larger than the cap the reader asked for: never read.
    Oversized,
    Binary,
    /// The object database does not have it (a partial clone), or could not read it.
    Unreadable,
}

impl GitRepo {
    /// Every file of `commit`'s tree that terrain counts, in the tree's order.
    pub(crate) fn files_at(&self, commit: &CommitId) -> Result<Vec<TreeFile>, GitError> {
        let commit = self
            .repo
            .find_commit(refs::to_object_id(commit))
            .map_err(read_err)?;
        let entries = commit
            .tree()
            .map_err(read_err)?
            .traverse()
            .breadthfirst
            .files()
            .map_err(read_err)?;
        Ok(entries
            .into_iter()
            .filter(|entry| entry.mode.is_blob() && counts_toward_terrain(&entry.filepath))
            .map(|entry| TreeFile {
                path: entry.filepath,
                id: entry.oid,
            })
            .collect())
    }

    /// The blob `id`, unless it is over `cap` bytes or binary. A blob that cannot be read is
    /// [`Blob::Unreadable`] rather than an error, so one bad object costs one file.
    pub(crate) fn blob(&self, id: gix::ObjectId, cap: u64) -> Blob {
        let Ok(Some(header)) = self.repo.try_find_header(id) else {
            return Blob::Unreadable;
        };
        if header.size() > cap {
            return Blob::Oversized;
        }
        let Ok(Some(object)) = self.repo.try_find_object(id) else {
            return Blob::Unreadable;
        };
        let data = object.detach().data;
        if looks_binary(header.size(), &data) {
            Blob::Binary
        } else {
            Blob::Text(data)
        }
    }
}
