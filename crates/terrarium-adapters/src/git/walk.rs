//! The commit graph: which commits count, when each one happened, and in what order.
//!
//! Counted commits are those reachable from any local branch. A commit's time is its committer
//! time clamped monotonic along topological order: never earlier than any of its parents. A
//! parent the repository lacks, behind a shallow boundary or simply missing, is outside the
//! walk; a commit with every parent outside starts the clamp at its own time.

use std::collections::{BTreeMap, BTreeSet};

use terrarium_truth::history::EARLIEST_COMMIT_SECS;
use terrarium_truth::{RepoSeed, TickGrid};

use super::refs::{to_commit_id, to_object_id};
use super::{GitError, RepoIdentity, read_err};

/// One commit as read from the object database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Node {
    /// Committer time in UTC seconds, raised to the earliest allowed; `None` when it does not
    /// parse.
    pub own_secs: Option<i64>,
    pub tree: gix::ObjectId,
    /// Parents the commit object lists, followed or not.
    pub listed_parents: u32,
    /// Listed parents inside the walk, in order.
    pub walked_parents: Vec<gix::ObjectId>,
    /// The first listed parent, when it is inside the walk: what the commit is diffed against.
    pub first_parent: Option<gix::ObjectId>,
}

/// Where a commit sits in time and in the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Placed {
    /// Clamped committer time, UTC seconds.
    pub clamped: i64,
    /// The longest path from a commit with no walked parents, which has 0.
    pub generation: u32,
}

/// What a walk leaves outside.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Outside {
    /// Commits whose parents were cut by the shallow file.
    pub shallow_boundary: BTreeSet<gix::ObjectId>,
    /// Parents the object database does not have.
    pub missing_parents: BTreeSet<gix::ObjectId>,
}

/// Reads every commit reachable from `tips` that `known` does not already hold.
pub(crate) fn walk(
    repo: &gix::Repository,
    tips: impl IntoIterator<Item = gix::ObjectId>,
    known: &dyn Fn(&gix::ObjectId) -> bool,
    shallow: &BTreeSet<gix::ObjectId>,
) -> Result<(BTreeMap<gix::ObjectId, Node>, Outside), GitError> {
    let mut nodes = BTreeMap::new();
    let mut outside = Outside::default();
    let mut stack: Vec<gix::ObjectId> = tips.into_iter().filter(|id| !known(id)).collect();
    while let Some(id) = stack.pop() {
        if nodes.contains_key(&id) {
            continue;
        }
        let node = read_node(repo, id, shallow, &mut outside)?;
        stack.extend(
            node.walked_parents
                .iter()
                .filter(|p| !known(p) && !nodes.contains_key(*p)),
        );
        nodes.insert(id, node);
    }
    Ok((nodes, outside))
}

fn read_node(
    repo: &gix::Repository,
    id: gix::ObjectId,
    shallow: &BTreeSet<gix::ObjectId>,
    outside: &mut Outside,
) -> Result<Node, GitError> {
    let commit = repo.find_commit(id).map_err(read_err)?;
    let decoded = commit.decode().map_err(read_err)?;
    let own_secs = decoded
        .committer()
        .ok()
        .and_then(|sig| sig.time().ok())
        .map(|time| time.seconds.max(EARLIEST_COMMIT_SECS));
    let listed: Vec<gix::ObjectId> = decoded.parents().collect();
    let cut = shallow.contains(&id);
    if cut {
        outside.shallow_boundary.insert(id);
    }
    let mut walked_parents = Vec::new();
    for parent in &listed {
        if cut {
            continue;
        }
        if repo.has_object(parent) {
            if !walked_parents.contains(parent) {
                walked_parents.push(*parent);
            }
        } else {
            outside.missing_parents.insert(*parent);
        }
    }
    let first_parent = listed
        .first()
        .filter(|p| walked_parents.contains(p))
        .copied();
    Ok(Node {
        own_secs,
        tree: decoded.tree(),
        listed_parents: u32::try_from(listed.len()).unwrap_or(u32::MAX),
        walked_parents,
        first_parent,
    })
}

/// Places every node: its clamped time and generation, given the already placed `known`.
/// The result does not depend on the order nodes are visited in.
pub(crate) fn place(
    nodes: &BTreeMap<gix::ObjectId, Node>,
    known: &dyn Fn(&gix::ObjectId) -> Option<Placed>,
) -> BTreeMap<gix::ObjectId, Placed> {
    // Kahn's algorithm over the new nodes; parents already known are placed.
    let mut waiting: BTreeMap<gix::ObjectId, usize> = BTreeMap::new();
    let mut children: BTreeMap<gix::ObjectId, Vec<gix::ObjectId>> = BTreeMap::new();
    for (id, node) in nodes {
        let pending = node
            .walked_parents
            .iter()
            .filter(|p| nodes.contains_key(*p))
            .inspect(|p| children.entry(**p).or_default().push(*id))
            .count();
        waiting.insert(*id, pending);
    }
    let mut ready: Vec<gix::ObjectId> = waiting
        .iter()
        .filter(|(_, pending)| **pending == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut placed: BTreeMap<gix::ObjectId, Placed> = BTreeMap::new();
    while let Some(id) = ready.pop() {
        let node = &nodes[&id];
        let parents: Vec<Placed> = node
            .walked_parents
            .iter()
            .map(|p| {
                placed
                    .get(p)
                    .copied()
                    .or_else(|| known(p))
                    .expect("parents are placed before their children")
            })
            .collect();
        let latest_parent = parents.iter().map(|p| p.clamped).max();
        let own = node
            .own_secs
            .or(latest_parent)
            .unwrap_or(EARLIEST_COMMIT_SECS);
        let clamped = latest_parent.map_or(own, |latest| own.max(latest));
        let generation = parents
            .iter()
            .map(|p| p.generation.saturating_add(1))
            .max()
            .unwrap_or(0);
        placed.insert(
            id,
            Placed {
                clamped,
                generation,
            },
        );
        for child in children.get(&id).into_iter().flatten() {
            let pending = waiting.get_mut(child).expect("every node waits");
            *pending -= 1;
            if *pending == 0 {
                ready.push(*child);
            }
        }
    }
    placed
}

/// The grid whose tick 0 holds the earliest commit.
pub(crate) fn grid(placed: impl Iterator<Item = Placed>) -> TickGrid {
    TickGrid::containing(
        placed
            .map(|p| p.clamped)
            .min()
            .unwrap_or(EARLIEST_COMMIT_SECS),
    )
}

/// The commits on the first-parent chain from `tip`, ending where a first parent is outside.
pub(crate) fn first_parent_chain(
    tip: gix::ObjectId,
    first_parent: &dyn Fn(&gix::ObjectId) -> Option<gix::ObjectId>,
) -> Vec<gix::ObjectId> {
    let mut chain = vec![tip];
    let mut seen = BTreeSet::from([tip]);
    let mut at = tip;
    while let Some(parent) = first_parent(&at) {
        if !seen.insert(parent) {
            break;
        }
        chain.push(parent);
        at = parent;
    }
    chain
}

pub(crate) fn identity(
    repo: &gix::Repository,
    default_branch: &str,
) -> Result<RepoIdentity, GitError> {
    let state = super::refs::state(repo)?;
    let tip = state
        .branches
        .get(default_branch)
        .ok_or_else(|| GitError::BranchMissing(default_branch.to_owned()))?;
    let shallow: BTreeSet<gix::ObjectId> = state.shallow.iter().map(to_object_id).collect();
    let mut outside = Outside::default();
    let mut at = to_object_id(tip);
    let mut seen = BTreeSet::new();
    loop {
        seen.insert(at);
        match read_node(repo, at, &shallow, &mut outside)?.first_parent {
            Some(parent) if !seen.contains(&parent) => at = parent,
            _ => break,
        }
    }
    let root = to_commit_id(at);
    Ok(RepoIdentity {
        seed: RepoSeed::from_root_commit(&root),
        root,
        shallow: outside.shallow_boundary.contains(&at),
    })
}
