//! A repository's history as truth reads it, and how it is kept up to date.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;

use terrarium_truth::domain::TagFact;
use terrarium_truth::history::EARLIEST_COMMIT_SECS;
use terrarium_truth::{CommitFacts, CommitId, Tick, TickGrid};

use super::diff::Differ;
use super::refs::{self, RepoState, to_commit_id, to_object_id};
use super::walk::{self, Node, Outside, Placed};
use super::{GitError, read_err};

/// How far a read has come: `diffed` of `total` commits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub diffed: usize,
    pub total: usize,
}

/// What a read had to leave out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryNotes {
    /// Commits whose parents a shallow clone lacks; each stands in for a root.
    pub shallow_boundary: Vec<CommitId>,
    /// Objects the repository does not have (parents outside a shallow file, or blobs of a
    /// partial clone). A missing parent ends the walk there; a missing blob counts no lines.
    pub missing_objects: Vec<CommitId>,
}

/// Every commit reachable from a local branch, as [`CommitFacts`] in time order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    pub default_branch: String,
    /// Tick 0 is the hour holding the earliest commit.
    pub grid: TickGrid,
    /// Ordered by clamped time, then generation, then id: parents before children.
    pub commits: Vec<CommitFacts>,
    /// The state this history was read from.
    pub state: RepoState,
    pub notes: HistoryNotes,
    index: BTreeMap<gix::ObjectId, Indexed>,
}

/// What the incremental read needs to know about each commit already read.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Indexed {
    placed: Placed,
    walked_parents: Vec<gix::ObjectId>,
    first_parent: Option<gix::ObjectId>,
}

/// How a history changed when it was brought up to date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Unchanged,
    /// Only new commits: every earlier fact holds. New commits may sort before old ones (an old
    /// branch restored, or an earlier-dated commit merged), so `earliest_tick` is the earliest
    /// tick among them.
    Appended {
        ids: Vec<CommitId>,
        earliest_tick: Tick,
    },
    /// The history was read again in full, because earlier facts changed.
    Rewritten(RewriteReason),
}

/// Why a history was read again in full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RewriteReason {
    /// The default branch asked for is not the one the history was read for.
    DefaultBranchChanged,
    /// A tag was added, removed or moved.
    TagsChanged,
    /// The shallow boundary moved.
    ShallowChanged,
    /// An object that was missing is there now.
    ObjectAppeared,
    /// A commit read before is no longer reachable from any local branch.
    CommitsDropped,
    /// A commit older than tick 0 was reached, which moves the grid.
    GridMoved,
    /// A commit read before joined or left the default branch's first-parent chain.
    FirstParentChanged,
}

/// The ticks of `grid` for a UTC time, saturating at both ends.
fn tick_of(grid: TickGrid, secs: i64) -> Tick {
    if secs < grid.origin_secs() {
        Tick::ZERO
    } else {
        grid.tick_at(secs).unwrap_or(Tick::new(u32::MAX))
    }
}

/// Each tagged commit's tags: name and date in UTC seconds.
type TagsByCommit = BTreeMap<gix::ObjectId, Vec<(String, Option<i64>)>>;

/// Each tagged commit's tags: name and date in UTC seconds, for tags that peel to a commit.
/// An annotated tag's date is its tagger's; a lightweight tag (or a tagger without a date) has
/// none, and takes the commit's.
fn resolve_tags(
    repo: &gix::Repository,
    tags: &BTreeMap<String, CommitId>,
) -> Result<TagsByCommit, GitError> {
    let mut out = TagsByCommit::new();
    for (name, target) in tags {
        let mut id = to_object_id(target);
        let mut date = None;
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(id) {
                break;
            }
            let Some(header) = repo.try_find_header(id).map_err(read_err)? else {
                break;
            };
            match header.kind() {
                gix::objs::Kind::Commit => {
                    out.entry(id).or_default().push((name.clone(), date));
                    break;
                }
                gix::objs::Kind::Tag => {
                    let tag = repo.find_tag(id).map_err(read_err)?;
                    let decoded = tag.decode().map_err(read_err)?;
                    if date.is_none() {
                        date = decoded
                            .tagger()
                            .ok()
                            .flatten()
                            .and_then(|sig| sig.time().ok())
                            .map(|time| time.seconds.max(EARLIEST_COMMIT_SECS));
                    }
                    id = decoded.target();
                }
                gix::objs::Kind::Tree | gix::objs::Kind::Blob => break,
            }
        }
    }
    Ok(out)
}

/// Everything a set of commits needs to become facts.
struct Graph<'a> {
    nodes: &'a BTreeMap<gix::ObjectId, Node>,
    placed: &'a BTreeMap<gix::ObjectId, Placed>,
    grid: TickGrid,
    first_parent_chain: &'a BTreeSet<gix::ObjectId>,
    tags: &'a TagsByCommit,
}

/// Diffs each commit in `ids` (in order) into facts, and notes any missing blobs.
fn facts(
    repo: &gix::Repository,
    graph: &Graph<'_>,
    ids: &[gix::ObjectId],
    missing: &mut BTreeSet<gix::ObjectId>,
    progress: &mut dyn FnMut(Progress) -> ControlFlow<()>,
) -> Result<Vec<CommitFacts>, GitError> {
    let mut differ = Differ::new(repo);
    let mut out = Vec::with_capacity(ids.len());
    for (done, id) in ids.iter().enumerate() {
        if progress(Progress {
            diffed: done,
            total: ids.len(),
        })
        .is_break()
        {
            return Err(GitError::Cancelled);
        }
        let node = &graph.nodes[id];
        let placed = graph.placed[id];
        let parent_tree = match node.first_parent {
            Some(parent) => Some(tree_of(repo, graph.nodes, parent)?),
            None => None,
        };
        let changes = differ.changes(repo, parent_tree, node.tree)?;
        missing.extend(changes.missing);
        let tick = tick_of(graph.grid, placed.clamped);
        let mut tags: Vec<TagFact> = graph
            .tags
            .get(id)
            .into_iter()
            .flatten()
            .map(|(name, date)| TagFact {
                name: name.clone(),
                tick: tick_of(graph.grid, date.unwrap_or(placed.clamped)).max(tick),
            })
            .collect();
        tags.sort_by(|a, b| a.name.cmp(&b.name));
        out.push(CommitFacts {
            id: to_commit_id(*id),
            tick,
            parents: node.listed_parents,
            on_default_first_parent: graph.first_parent_chain.contains(id),
            added: changes.added,
            removed: changes.removed,
            deleted_dirs: changes.deleted_dirs,
            renames: changes.renames,
            tags,
        });
    }
    // The read is complete, so a request to stop here changes nothing.
    let _ = progress(Progress {
        diffed: ids.len(),
        total: ids.len(),
    });
    Ok(out)
}

/// The tree of a commit already read, or read now (a first parent known from before).
fn tree_of(
    repo: &gix::Repository,
    nodes: &BTreeMap<gix::ObjectId, Node>,
    id: gix::ObjectId,
) -> Result<gix::ObjectId, GitError> {
    if let Some(node) = nodes.get(&id) {
        return Ok(node.tree);
    }
    let commit = repo.find_commit(id).map_err(read_err)?;
    walk::header(&commit.data, repo.object_hash())
        .map(|header| header.tree)
        .ok_or_else(|| GitError::Read(format!("commit {} has no tree", id.to_hex())))
}

fn sort_key(
    placed: &BTreeMap<gix::ObjectId, Placed>,
    id: &gix::ObjectId,
) -> (i64, u32, gix::ObjectId) {
    let p = placed[id];
    (p.clamped, p.generation, *id)
}

fn notes(outside: &Outside, missing_blobs: &BTreeSet<gix::ObjectId>) -> HistoryNotes {
    let mut missing: BTreeSet<gix::ObjectId> = outside.missing_parents.clone();
    missing.extend(missing_blobs);
    HistoryNotes {
        shallow_boundary: outside
            .shallow_boundary
            .iter()
            .copied()
            .map(to_commit_id)
            .collect(),
        missing_objects: missing.into_iter().map(to_commit_id).collect(),
    }
}

fn default_tip(state: &RepoState, default_branch: &str) -> Result<gix::ObjectId, GitError> {
    state
        .branches
        .get(default_branch)
        .map(to_object_id)
        .ok_or_else(|| GitError::BranchMissing(default_branch.to_owned()))
}

fn shallow_ids(state: &RepoState) -> BTreeSet<gix::ObjectId> {
    state.shallow.iter().map(to_object_id).collect()
}

pub(crate) fn read(
    repo: &gix::Repository,
    default_branch: &str,
    progress: &mut dyn FnMut(Progress) -> ControlFlow<()>,
) -> Result<History, GitError> {
    let state = refs::state(repo)?;
    let tip = default_tip(&state, default_branch)?;
    let tips = state.branches.values().map(to_object_id);
    let (nodes, outside) = walk::walk(repo, tips, &|_| false, &shallow_ids(&state))?;
    let placed = walk::place(&nodes, &|_| None);
    let grid = walk::grid(placed.values().copied());
    let chain: BTreeSet<gix::ObjectId> =
        walk::first_parent_chain(tip, &|id| nodes.get(id).and_then(|n| n.first_parent))
            .into_iter()
            .collect();
    let tags = resolve_tags(repo, &state.tags)?;
    let mut ids: Vec<gix::ObjectId> = nodes.keys().copied().collect();
    ids.sort_by_key(|id| sort_key(&placed, id));
    let graph = Graph {
        nodes: &nodes,
        placed: &placed,
        grid,
        first_parent_chain: &chain,
        tags: &tags,
    };
    let mut missing = BTreeSet::new();
    let commits = facts(repo, &graph, &ids, &mut missing, progress)?;
    let index = nodes
        .iter()
        .map(|(id, node)| {
            (
                *id,
                Indexed {
                    placed: placed[id],
                    walked_parents: node.walked_parents.clone(),
                    first_parent: node.first_parent,
                },
            )
        })
        .collect();
    Ok(History {
        default_branch: default_branch.to_owned(),
        grid,
        commits,
        state,
        notes: notes(&outside, &missing),
        index,
    })
}

pub(crate) fn read_since(
    repo: &gix::Repository,
    history: &mut History,
    default_branch: &str,
    progress: &mut dyn FnMut(Progress) -> ControlFlow<()>,
) -> Result<Change, GitError> {
    let state = refs::state(repo)?;
    let appeared = || {
        history
            .notes
            .missing_objects
            .iter()
            .any(|id| repo.has_object(to_object_id(id)))
    };
    let reason = if default_branch != history.default_branch {
        Some(RewriteReason::DefaultBranchChanged)
    } else if appeared() {
        Some(RewriteReason::ObjectAppeared)
    } else if state == history.state {
        default_tip(&state, default_branch)?;
        return Ok(Change::Unchanged);
    } else if state.tags != history.state.tags {
        Some(RewriteReason::TagsChanged)
    } else if state.shallow != history.state.shallow {
        Some(RewriteReason::ShallowChanged)
    } else {
        None
    };
    let reason = match reason {
        Some(reason) => reason,
        None => match grow(repo, history, state, progress)? {
            Ok(change) => return Ok(change),
            Err(reason) => reason,
        },
    };
    *history = read(repo, default_branch, progress)?;
    Ok(Change::Rewritten(reason))
}

/// Reads only the commits `history` lacks into it, or says why every earlier fact no longer
/// holds, leaving `history` as it was.
fn grow(
    repo: &gix::Repository,
    history: &mut History,
    state: RepoState,
    progress: &mut dyn FnMut(Progress) -> ControlFlow<()>,
) -> Result<Result<Change, RewriteReason>, GitError> {
    let known = &history.index;
    let tip = default_tip(&state, &history.default_branch)?;
    let tips: Vec<gix::ObjectId> = state.branches.values().map(to_object_id).collect();
    let (nodes, outside) = walk::walk(
        repo,
        tips.iter().copied(),
        &|id| known.contains_key(id),
        &shallow_ids(&state),
    )?;
    if drops_commits(known, &tips, &nodes) {
        return Ok(Err(RewriteReason::CommitsDropped));
    }
    let placed = walk::place(&nodes, &|id| known.get(id).map(|k| k.placed));
    let grid = walk::grid(
        known
            .values()
            .map(|k| k.placed)
            .chain(placed.values().copied()),
    );
    if grid != history.grid {
        return Ok(Err(RewriteReason::GridMoved));
    }
    let first_parent = |id: &gix::ObjectId| {
        nodes
            .get(id)
            .map(|n| n.first_parent)
            .or_else(|| known.get(id).map(|k| k.first_parent))
            .flatten()
    };
    let chain: BTreeSet<gix::ObjectId> = walk::first_parent_chain(tip, &first_parent)
        .into_iter()
        .collect();
    if chain_moved(history, &chain) {
        return Ok(Err(RewriteReason::FirstParentChanged));
    }

    let tags = resolve_tags(repo, &state.tags)?;
    let mut new_ids: Vec<gix::ObjectId> = nodes.keys().copied().collect();
    new_ids.sort_by_key(|id| sort_key(&placed, id));
    let graph = Graph {
        nodes: &nodes,
        placed: &placed,
        grid,
        first_parent_chain: &chain,
        tags: &tags,
    };
    let mut missing = BTreeSet::new();
    let new_facts = facts(repo, &graph, &new_ids, &mut missing, progress)?;
    Ok(Ok(extend(
        history, state, &graph, new_facts, &outside, &missing,
    )))
}

/// Whether some commit read before is reachable from no branch now (a deleted or rewound branch
/// takes its commits with it).
fn drops_commits(
    known: &BTreeMap<gix::ObjectId, Indexed>,
    tips: &[gix::ObjectId],
    nodes: &BTreeMap<gix::ObjectId, Node>,
) -> bool {
    let mut reachable: BTreeSet<gix::ObjectId> = BTreeSet::new();
    let mut stack: Vec<gix::ObjectId> = tips
        .iter()
        .chain(nodes.values().flat_map(|n| n.walked_parents.iter()))
        .filter(|id| known.contains_key(*id))
        .copied()
        .collect();
    while let Some(id) = stack.pop() {
        if reachable.insert(id) {
            stack.extend(known[&id].walked_parents.iter().copied());
        }
    }
    reachable.len() != known.len()
}

/// Whether a commit read before joined or left the default branch's first-parent chain.
fn chain_moved(history: &History, chain: &BTreeSet<gix::ObjectId>) -> bool {
    let was: BTreeSet<CommitId> = history
        .commits
        .iter()
        .filter(|c| c.on_default_first_parent)
        .map(|c| c.id.clone())
        .collect();
    let is: BTreeSet<CommitId> = chain
        .iter()
        .filter(|id| history.index.contains_key(*id))
        .copied()
        .map(to_commit_id)
        .collect();
    was != is
}

/// Merges `new_facts` (in order) into `history`, and says what changed.
fn extend(
    history: &mut History,
    state: RepoState,
    graph: &Graph<'_>,
    new_facts: Vec<CommitFacts>,
    outside: &Outside,
    missing: &BTreeSet<gix::ObjectId>,
) -> Change {
    for (id, node) in graph.nodes {
        history.index.insert(
            *id,
            Indexed {
                placed: graph.placed[id],
                walked_parents: node.walked_parents.clone(),
                first_parent: node.first_parent,
            },
        );
    }
    let change = match new_facts.iter().map(|c| c.tick).min() {
        Some(earliest_tick) => Change::Appended {
            ids: new_facts.iter().map(|c| c.id.clone()).collect(),
            earliest_tick,
        },
        None => Change::Unchanged,
    };
    // Both lists are already in order: one merge, each key computed once.
    let old = std::mem::take(&mut history.commits);
    let keyed = |list: Vec<CommitFacts>, index: &BTreeMap<gix::ObjectId, Indexed>| {
        list.into_iter()
            .map(|c| {
                let id = to_object_id(&c.id);
                let placed = index[&id].placed;
                ((placed.clamped, placed.generation, id), c)
            })
            .collect::<Vec<_>>()
    };
    let mut old = keyed(old, &history.index).into_iter().peekable();
    let mut new = keyed(new_facts, &history.index).into_iter().peekable();
    let mut merged = Vec::with_capacity(old.len() + new.len());
    loop {
        let take_old = match (old.peek(), new.peek()) {
            (Some((a, _)), Some((b, _))) => a <= b,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        let next = if take_old { old.next() } else { new.next() };
        merged.extend(next.map(|(_, c)| c));
    }
    history.commits = merged;

    let mut outside_all = outside.clone();
    outside_all
        .shallow_boundary
        .extend(history.notes.shallow_boundary.iter().map(to_object_id));
    let mut missing_all: BTreeSet<gix::ObjectId> = history
        .notes
        .missing_objects
        .iter()
        .map(to_object_id)
        .collect();
    missing_all.extend(missing);
    history.notes = notes(&outside_all, &missing_all);
    history.grid = graph.grid;
    history.state = state;
    change
}
