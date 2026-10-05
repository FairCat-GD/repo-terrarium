//! The habitat: terrain and age, and the facts about the default branch the rest of replay
//! reads.
//!
//! Terrain is the running line total along the default branch's first-parent chain. Each
//! commit there is diffed against its first parent, so the sum is the text lines of the default
//! tip as of each tick, merges bringing their branch's lines in once. Side branches add no
//! terrain until they are merged.
//!
//! Terrain sets the carrying capacities: how many plants and weeds the terrarium holds.

use serde::{Deserialize, Serialize};

use crate::canon::canonical_struct;
use crate::domain::{CommitFacts, Habitat};
use crate::fixed::{Q16, Q32};
use crate::identity::CommitId;
use crate::time::Tick;
use crate::tuning::{
    LINES_PER_KLOC, PLANTS_MAX, PLANTS_MIN, PLANTS_PER_DOUBLING, WEEDS_MAX, WEEDS_MIN,
    WEEDS_PER_PLANTS,
};

/// How many plants `terrain` lines hold: `clamp(6 + 4 × log2(1 + KLOC), 6, 24)`, rounded down.
pub fn plant_capacity(terrain: u64) -> u32 {
    let lines = i64::try_from(terrain).unwrap_or(i64::MAX);
    let doublings = Q32::from_ratio(lines.saturating_add(LINES_PER_KLOC), LINES_PER_KLOC)
        .log2()
        .unwrap_or(Q16::ZERO);
    let plants = PLANTS_MIN
        .saturating_add(doublings.saturating_mul(PLANTS_PER_DOUBLING).floor_to_int())
        .clamp(PLANTS_MIN, PLANTS_MAX);
    u32::try_from(plants).unwrap_or(0)
}

/// How many weeds a terrarium of `plants` capacity holds: `clamp(P / 3, 2, 8)`.
pub fn weed_capacity(plants: u32) -> u32 {
    (plants / WEEDS_PER_PLANTS).clamp(WEEDS_MIN, WEEDS_MAX)
}

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

    /// The text lines at the default tip, zero while the running sum is negative.
    pub fn terrain_lines(&self) -> u64 {
        u64::try_from(self.terrain).unwrap_or(0)
    }

    /// The habitat at `now`.
    pub fn at(&self, now: Tick) -> Habitat {
        Habitat {
            terrain: self.terrain_lines(),
            age: Tick::new(self.root.map_or(0, |root| now.hours_since(root))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacities_follow_the_terrain() {
        // KLOC 0, 0.5, 1, 3, 3.921 (the arc's end), 15, 21.6 (just short of 24) and beyond.
        let plants: Vec<u32> = [
            0,
            500,
            1_000,
            3_000,
            3_921,
            15_000,
            21_000,
            22_000,
            u64::MAX,
        ]
        .into_iter()
        .map(plant_capacity)
        .collect();
        assert_eq!(plants, [6, 8, 10, 14, 15, 22, 23, 24, 24]);
        let weeds: Vec<u32> = [6, 8, 14, 15, 24].into_iter().map(weed_capacity).collect();
        assert_eq!(weeds, [2, 2, 4, 5, 8]);
    }
}
