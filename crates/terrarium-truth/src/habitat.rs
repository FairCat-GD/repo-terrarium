//! The habitat: terrain and age, and the facts about the default branch the rest of replay
//! reads.
//!
//! Terrain is the running line total along the default branch's first-parent chain. Each
//! commit there is diffed against its first parent, so the sum is the text lines of the default
//! tip as of each tick, merges bringing their branch's lines in once. Side branches add no
//! terrain until they are merged.

use serde::{Deserialize, Serialize};

use crate::canon::canonical_struct;
use crate::domain::{CommitFacts, Habitat};
use crate::identity::CommitId;
use crate::time::Tick;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HabitatState {
    /// Lines added less lines removed along the default branch's first-parent chain,
    /// saturating, and read as zero while negative.
    pub terrain: i64,
    /// The first commit on that chain, from which age counts.
    pub root: Option<Tick>,
    /// The newest commit on that chain so far: the default tip.
    pub default_tip: Option<CommitId>,
    /// The newest commit on any branch so far, merges included.
    pub last_commit: Option<Tick>,
}

canonical_struct!(HabitatState {
    terrain,
    root,
    default_tip,
    last_commit
});

impl HabitatState {
    pub fn apply(&mut self, commit: &CommitFacts) {
        self.last_commit = Some(commit.tick);
        if !commit.on_default_first_parent {
            return;
        }
        let added = i64::try_from(commit.added).unwrap_or(i64::MAX);
        let removed = i64::try_from(commit.removed).unwrap_or(i64::MAX);
        self.terrain = self.terrain.saturating_add(added).saturating_sub(removed);
        self.root.get_or_insert(commit.tick);
        self.default_tip = Some(commit.id.clone());
    }

    /// The habitat at `now`.
    pub fn at(&self, now: Tick) -> Habitat {
        Habitat {
            terrain: u64::try_from(self.terrain).unwrap_or(0),
            age: Tick::new(self.root.map_or(0, |root| now.hours_since(root))),
        }
    }
}
