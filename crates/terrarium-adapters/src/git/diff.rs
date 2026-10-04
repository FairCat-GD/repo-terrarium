//! One commit's changes against its first parent: lines added and removed over text files,
//! renames, and directories deleted whole.
//!
//! Only trees and blobs from the object database are read. The rename tracker gets a resource
//! cache with no attributes, no diff drivers and no filters, so nothing the repository configures
//! can run, and nothing install-local (`info/attributes`, the repository's config) or tied to
//! where `HEAD` is now can change an old commit's facts. Line counts are computed here, from the
//! blobs, with an explicit algorithm.

use std::collections::BTreeSet;

use gix::bstr::{BString, ByteSlice};
use gix::diff::blob::{Algorithm, Diff, InternedInput};
use gix::diff::tree_with_rewrites::ChangeRef;
use gix::objs::tree::EntryMode;
use terrarium_truth::domain::{DeletedDir, Rename};
use terrarium_truth::history::{
    LARGEST_TEXT_BLOB_BYTES, RENAME_LIMIT, RENAME_SIMILARITY_PERCENT, count_lines,
    counts_toward_terrain, looks_binary,
};

use super::{GitError, read_err};

/// The line diff, fixed: the repository's `diff.algorithm` is never read.
const ALGORITHM: Algorithm = Algorithm::Histogram;

/// What one commit changed, as truth counts it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Changes {
    pub added: u64,
    pub removed: u64,
    pub deleted_dirs: Vec<DeletedDir>,
    pub renames: Vec<Rename>,
    /// Blobs the object database does not have; they count no lines.
    pub missing: BTreeSet<gix::ObjectId>,
}

/// One tree change, owned, before its lines are counted.
enum Item {
    Added(BString, EntryMode, gix::ObjectId),
    Deleted(BString, EntryMode, gix::ObjectId),
    Modified {
        path: BString,
        old: (EntryMode, gix::ObjectId),
        new: (EntryMode, gix::ObjectId),
    },
    Renamed {
        from: BString,
        to: BString,
        old: (EntryMode, gix::ObjectId),
        new: (EntryMode, gix::ObjectId),
    },
}

/// Diffs trees, reusing its caches across commits.
pub(crate) struct Differ {
    resources: gix::diff::blob::Platform,
    tree_state: gix::diff::tree::State,
}

impl Differ {
    pub(crate) fn new(repo: &gix::Repository) -> Differ {
        use gix::worktree::stack::state::attributes::Source;

        let attributes = gix::worktree::stack::state::Attributes::new(
            gix::attrs::Search::default(),
            None,
            Source::IdMapping,
            gix::attrs::search::MetadataCollection::default(),
        );
        let attribute_stack = gix::worktree::Stack::new(
            repo.git_dir(),
            gix::worktree::stack::State::AttributesStack(attributes),
            gix::glob::pattern::Case::Sensitive,
            Vec::new(),
            Vec::new(),
        );
        let resources = gix::diff::blob::Platform::new(
            gix::diff::blob::platform::Options {
                algorithm: Some(ALGORITHM),
                skip_internal_diff_if_external_is_configured: false,
            },
            gix::diff::blob::Pipeline::new(
                gix::diff::blob::pipeline::WorktreeRoots::default(),
                gix::filter::plumbing::Pipeline::default(),
                Vec::new(),
                gix::diff::blob::pipeline::Options {
                    large_file_threshold_bytes: LARGEST_TEXT_BLOB_BYTES,
                    fs: gix::fs::Capabilities::default(),
                },
            ),
            gix::diff::blob::pipeline::Mode::ToGit,
            attribute_stack,
        );
        Differ {
            resources,
            tree_state: gix::diff::tree::State::default(),
        }
    }

    /// The changes from `parent` (or the empty tree) to `tree`.
    pub(crate) fn changes(
        &mut self,
        repo: &gix::Repository,
        parent: Option<gix::ObjectId>,
        tree: gix::ObjectId,
    ) -> Result<Changes, GitError> {
        if parent == Some(tree) {
            return Ok(Changes::default());
        }
        let hash = repo.object_hash();
        let old = match parent {
            Some(id) => repo.find_object(id).map_err(read_err)?.detach().data,
            None => Vec::new(),
        };
        let new = repo.find_object(tree).map_err(read_err)?.detach().data;
        let mut items = Vec::new();
        let options = gix::diff::tree_with_rewrites::Options {
            location: Some(gix::diff::tree::recorder::Location::Path),
            rewrites: Some(gix::diff::Rewrites {
                copies: None,
                percentage: Some(f32::from(RENAME_SIMILARITY_PERCENT) / 100.0),
                limit: RENAME_LIMIT,
                track_empty: false,
            }),
        };
        gix::diff::tree_with_rewrites(
            gix::objs::TreeRefIter::from_bytes(&old, hash),
            gix::objs::TreeRefIter::from_bytes(&new, hash),
            &mut self.resources,
            &mut self.tree_state,
            &repo.objects,
            |change| -> gix::ExnResult<gix::diff::tree_with_rewrites::Action> {
                items.push(own(change));
                Ok(std::ops::ControlFlow::Continue(()))
            },
            options,
        )
        .map_err(read_err)?;
        self.resources.clear_resource_cache_keep_allocation();
        count(repo, items)
    }
}

fn own(change: ChangeRef<'_>) -> Item {
    match change {
        // Copies are never tracked; were one reported, its destination is simply new.
        ChangeRef::Addition {
            location,
            entry_mode,
            id,
            ..
        }
        | ChangeRef::Rewrite {
            location,
            entry_mode,
            id,
            copy: true,
            ..
        } => Item::Added(location.to_owned(), entry_mode, id),
        ChangeRef::Deletion {
            location,
            entry_mode,
            id,
            ..
        } => Item::Deleted(location.to_owned(), entry_mode, id),
        ChangeRef::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
        } => Item::Modified {
            path: location.to_owned(),
            old: (previous_entry_mode, previous_id),
            new: (entry_mode, id),
        },
        ChangeRef::Rewrite {
            source_location,
            source_entry_mode,
            source_id,
            location,
            entry_mode,
            id,
            ..
        } => Item::Renamed {
            from: source_location.to_owned(),
            to: location.to_owned(),
            old: (source_entry_mode, source_id),
            new: (entry_mode, id),
        },
    }
}

/// A side of a change that counts: a blob at a counted path, holding text.
fn text(
    repo: &gix::Repository,
    path: &[u8],
    (mode, id): (EntryMode, gix::ObjectId),
    missing: &mut BTreeSet<gix::ObjectId>,
) -> Result<Option<Vec<u8>>, GitError> {
    if !mode.is_blob() || !counts_toward_terrain(path) {
        return Ok(None);
    }
    let Some(header) = repo.try_find_header(id).map_err(read_err)? else {
        missing.insert(id);
        return Ok(None);
    };
    if header.size() > LARGEST_TEXT_BLOB_BYTES {
        return Ok(None);
    }
    let Some(object) = repo.try_find_object(id).map_err(read_err)? else {
        missing.insert(id);
        return Ok(None);
    };
    let data = object.detach().data;
    Ok((!looks_binary(header.size(), &data)).then_some(data))
}

/// Lines added and removed between two sides, either of which may not count.
fn lines(old: Option<&[u8]>, new: Option<&[u8]>) -> (u64, u64) {
    match (old, new) {
        (None, None) => (0, 0),
        (None, Some(new)) => (count_lines(new), 0),
        (Some(old), None) => (0, count_lines(old)),
        (Some(old), Some(new)) => {
            let input = InternedInput::new(old, new);
            let diff = Diff::compute(ALGORITHM, &input);
            (
                u64::from(diff.count_additions()),
                u64::from(diff.count_removals()),
            )
        }
    }
}

fn lossy(path: &[u8]) -> String {
    String::from_utf8_lossy(path).into_owned()
}

fn count(repo: &gix::Repository, mut items: Vec<Item>) -> Result<Changes, GitError> {
    let mut out = Changes::default();
    let mut deleted_trees: BTreeSet<BString> = BTreeSet::new();
    // Path of each deleted blob, with the counted lines it held.
    let mut deleted_blobs: Vec<(BString, u64)> = Vec::new();
    items.sort_by(|a, b| key(a).cmp(key(b)));
    for item in items {
        match item {
            Item::Added(path, mode, id) => {
                let new = text(repo, &path, (mode, id), &mut out.missing)?;
                out.added += lines(None, new.as_deref()).0;
            }
            Item::Deleted(path, mode, id) => {
                if mode.is_tree() {
                    deleted_trees.insert(path);
                    continue;
                }
                let old = text(repo, &path, (mode, id), &mut out.missing)?;
                let removed = lines(old.as_deref(), None).1;
                out.removed += removed;
                if mode.is_blob() {
                    deleted_blobs.push((path, removed));
                }
            }
            Item::Modified { path, old, new } => {
                let old = text(repo, &path, old, &mut out.missing)?;
                let new = text(repo, &path, new, &mut out.missing)?;
                let (added, removed) = lines(old.as_deref(), new.as_deref());
                out.added += added;
                out.removed += removed;
            }
            Item::Renamed { from, to, old, new } => {
                // gix pairs a deleted directory with an added one as a directory rename as soon
                // as one file moved between them; the source directory is gone all the same.
                if old.0.is_tree() {
                    deleted_trees.insert(from);
                    continue;
                }
                let old_text = text(repo, &from, old, &mut out.missing)?;
                let new_text = text(repo, &to, new, &mut out.missing)?;
                let (added, removed) = lines(old_text.as_deref(), new_text.as_deref());
                out.added += added;
                out.removed += removed;
                let both_blobs = old.0.is_blob() && new.0.is_blob();
                if both_blobs && (counts_toward_terrain(&from) || counts_toward_terrain(&to)) {
                    out.renames.push(Rename {
                        from: lossy(&from),
                        to: lossy(&to),
                    });
                }
            }
        }
    }
    out.deleted_dirs = deleted_dirs(&deleted_trees, &deleted_blobs);
    out.renames
        .sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
    Ok(out)
}

fn key(item: &Item) -> &BString {
    match item {
        Item::Added(path, ..) | Item::Deleted(path, ..) | Item::Modified { path, .. } => path,
        Item::Renamed { to, .. } => to,
    }
}

/// The topmost deleted directories that lost at least one file outright (a file renamed out of
/// a directory is not a deletion), each with the counted lines of the files deleted inside it.
fn deleted_dirs(trees: &BTreeSet<BString>, blobs: &[(BString, u64)]) -> Vec<DeletedDir> {
    let inside = |dir: &BString, path: &BString| {
        path.len() > dir.len() && path.starts_with(dir.as_slice()) && path[dir.len()] == b'/'
    };
    trees
        .iter()
        .filter(|dir| !trees.iter().any(|outer| inside(outer, dir)))
        .filter_map(|dir| {
            let mut files = blobs
                .iter()
                .filter(|(path, _)| inside(dir, path))
                .peekable();
            files.peek()?;
            Some(DeletedDir {
                path: lossy(dir.as_bytes()),
                lines: files.map(|(_, lines)| *lines).sum(),
            })
        })
        .collect()
}
