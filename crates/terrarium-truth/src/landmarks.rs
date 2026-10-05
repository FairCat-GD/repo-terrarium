//! Monuments and fossils: what the terrarium keeps of its history.
//!
//! - **Monuments.** A standing stone for each full year since the default branch's first
//!   commit, at most three, raised at the first close on or after the anniversary. Each
//!   weathers by its own age: young, lichened from two years, ancient from five. Its carved
//!   marks come with releases. A stone's id is its place in that order, so nothing else can
//!   renumber one.
//! - **Fossils.** A commit on the default branch's first-parent chain that deletes a whole
//!   directory of 500 lines or more (a rename is not a deletion) leaves one fossil, of the form
//!   the seed draws for that commit. At most five are kept, the oldest leaving first. A side
//!   branch's commits leave none, as they add no terrain: a deletion counts once, when it
//!   reaches the default branch. A history rewrite that drops the commit drops its fossil,
//!   because replay never meets the commit again.

use std::cmp::Reverse;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::canon::{canonical_struct, count_of, from_hex};
use crate::domain::CommitFacts;
use crate::identity::{CommitId, RepoSeed};
use crate::rng::{Pcg32, StreamTag, digest_u64};
use crate::scene::{FossilForm, FossilView, MonumentAge, MonumentView};
use crate::time::{TICKS_PER_DAY, Tick};
use crate::tuning::{
    ANCIENT_FROM_YEARS, FOSSIL_MIN_LINES, FOSSILS_MAX, LICHENED_FROM_YEARS, MONUMENT_DAYS,
    MONUMENTS_MAX,
};

/// A standing stone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Monument {
    /// Its place in the order the stones were raised, from 1.
    pub id: u32,
    /// The close it was raised at.
    pub founded: Tick,
    /// Carved marks, up to `MAX_MARKS`.
    pub marks: u8,
}

canonical_struct!(Monument { id, founded, marks });

/// A fossil.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fossil {
    /// Stable for the fossil's life, and never given to another.
    pub id: u32,
    pub form: FossilForm,
    /// The deleting commit's tick.
    pub tick: Tick,
    /// The deleting commit, which a history rewrite may drop.
    pub commit: CommitId,
    /// The deleted directory's path, hashed (the first 8 bytes of its SHA-256, little-endian);
    /// the path itself is among the commit's deleted directories.
    pub path: u64,
}

canonical_struct!(Fossil {
    id,
    form,
    tick,
    commit,
    path,
});

/// The terrarium's monuments and fossils.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Landmarks {
    /// Oldest first.
    pub monuments: Vec<Monument>,
    /// The id the next fossil takes.
    pub next_fossil: u32,
    /// Oldest first.
    pub fossils: Vec<Fossil>,
}

canonical_struct!(Landmarks {
    monuments,
    next_fossil,
    fossils,
});

impl Default for Landmarks {
    fn default() -> Landmarks {
        Landmarks {
            monuments: Vec::new(),
            next_fossil: 1,
            fossils: Vec::new(),
        }
    }
}

const fn years(n: u32) -> u32 {
    n.saturating_mul(MONUMENT_DAYS)
        .saturating_mul(TICKS_PER_DAY)
}

/// `bytes`' SHA-256, as a number.
fn hashed(bytes: &[u8]) -> u64 {
    digest_u64(&Sha256::digest(bytes).into())
}

/// The form the seed draws for a fossil left by `commit`. The stream is keyed by a hash of the
/// commit id's bytes, so every commit draws its own, whatever else applies in its tick.
pub fn fossil_form(seed: &RepoSeed, commit: &CommitId) -> FossilForm {
    let key = hashed(&from_hex(commit.as_str()).unwrap_or_default());
    let forms = FossilForm::ALL;
    let picked = u32::try_from(forms.len())
        .ok()
        .and_then(NonZeroU32::new)
        .map(|bound| Pcg32::stream(seed, StreamTag::Fossil, key).below(bound))
        .and_then(|index| usize::try_from(index).ok());
    picked
        .and_then(|index| forms.get(index).copied())
        .unwrap_or(FossilForm::Shell)
}

impl Landmarks {
    /// At a close: a stone for a full year of age not yet marked, while fewer than three stand.
    /// `root` is the default branch's first commit.
    pub fn raise(&mut self, now: Tick, root: Option<Tick>) {
        let Some(root) = root else {
            return;
        };
        let raised = count_of(self.monuments.len());
        let due = now.hours_since(root) / years(1);
        if self.monuments.len() < MONUMENTS_MAX && due > raised {
            self.monuments.push(Monument {
                id: raised.saturating_add(1),
                founded: now,
                marks: 0,
            });
        }
    }

    /// As `commit` applies: a fossil, if it deletes a whole directory of 500 lines or more on
    /// the default branch's first-parent chain. The largest such directory names it, the first
    /// by path among equals.
    pub fn bury(&mut self, seed: &RepoSeed, commit: &CommitFacts) {
        if !commit.on_default_first_parent {
            return;
        }
        let Some(dir) = commit
            .deleted_dirs
            .iter()
            .filter(|dir| dir.lines >= FOSSIL_MIN_LINES)
            .min_by_key(|dir| (Reverse(dir.lines), dir.path.as_str()))
        else {
            return;
        };
        self.fossils.push(Fossil {
            id: self.next_fossil,
            form: fossil_form(seed, &commit.id),
            tick: commit.tick,
            commit: commit.id.clone(),
            path: hashed(dir.path.as_bytes()),
        });
        self.next_fossil = self.next_fossil.saturating_add(1);
        let excess = self.fossils.len().saturating_sub(FOSSILS_MAX);
        self.fossils.drain(..excess);
    }

    /// The monuments as the composer sees them at `now`, each weathered by its own age.
    pub fn monument_views(&self, now: Tick) -> Vec<MonumentView> {
        self.monuments
            .iter()
            .map(|monument| {
                let age = now.hours_since(monument.founded);
                MonumentView {
                    id: monument.id,
                    age: if age >= years(ANCIENT_FROM_YEARS) {
                        MonumentAge::Ancient
                    } else if age >= years(LICHENED_FROM_YEARS) {
                        MonumentAge::Lichened
                    } else {
                        MonumentAge::Young
                    },
                    marks: monument.marks,
                }
            })
            .collect()
    }

    /// The fossils as the composer sees them.
    pub fn fossil_views(&self) -> Vec<FossilView> {
        self.fossils
            .iter()
            .map(|fossil| FossilView {
                id: fossil.id,
                form: fossil.form,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::DeletedDir;
    use crate::test_support::{commit, id};

    fn seed() -> RepoSeed {
        RepoSeed::from_bytes([7; 32])
    }

    fn deleting(n: u32, tick: u32, dirs: &[(&str, u64)]) -> CommitFacts {
        CommitFacts {
            deleted_dirs: dirs
                .iter()
                .map(|&(path, lines)| DeletedDir {
                    path: path.to_owned(),
                    lines,
                })
                .collect(),
            ..commit(n, tick)
        }
    }

    const YEAR: u32 = 365 * 24;

    #[test]
    fn a_stone_rises_each_full_year_up_to_three() {
        let mut landmarks = Landmarks::default();
        // A root at tick 10; closes every 24 hours from tick 15.
        let root = Some(Tick::new(10));
        let mut founded = Vec::new();
        for close in (15..6 * YEAR).step_by(24) {
            landmarks.raise(Tick::new(close), root);
            if landmarks.monuments.len() > founded.len() {
                founded.push(close);
            }
        }
        // The first close on or after each anniversary: tick 10 + n years, rounded up to a
        // close.
        assert_eq!(founded, [YEAR + 15, 2 * YEAR + 15, 3 * YEAR + 15]);
        let ids: Vec<u32> = landmarks.monuments.iter().map(|m| m.id).collect();
        assert_eq!(ids, [1, 2, 3]);
        // No root, no stones.
        let mut bare = Landmarks::default();
        bare.raise(Tick::new(10 * YEAR), None);
        assert!(bare.monuments.is_empty());
    }

    #[test]
    fn each_stone_weathers_by_its_own_age() {
        use MonumentAge::{Ancient, Lichened, Young};
        let mut landmarks = Landmarks::default();
        for close in (24..4 * YEAR).step_by(24) {
            landmarks.raise(Tick::new(close), Some(Tick::ZERO));
        }
        let ages = |now: u32| -> Vec<MonumentAge> {
            landmarks
                .monument_views(Tick::new(now))
                .iter()
                .map(|view| view.age)
                .collect()
        };
        // Raised at years 1, 2 and 3 (the closes at exactly 365 days, since tick 0 is a close).
        assert_eq!(ages(3 * YEAR), [Lichened, Young, Young]);
        assert_eq!(ages(3 * YEAR - 1), [Young, Young, Young]);
        assert_eq!(ages(6 * YEAR - 1), [Lichened, Lichened, Lichened]);
        assert_eq!(ages(6 * YEAR), [Ancient, Lichened, Lichened]);
        assert_eq!(ages(8 * YEAR), [Ancient, Ancient, Ancient]);
        assert!(
            landmarks
                .monument_views(Tick::new(8 * YEAR))
                .iter()
                .all(|view| view.marks == 0)
        );
    }

    #[test]
    fn only_a_large_deletion_on_the_default_chain_leaves_a_fossil() {
        let seed = seed();
        let mut landmarks = Landmarks::default();
        landmarks.bury(&seed, &deleting(1, 5, &[("src/small", 499)]));
        landmarks.bury(&seed, &commit(2, 6));
        let side = CommitFacts {
            on_default_first_parent: false,
            ..deleting(3, 7, &[("src/feature", 900)])
        };
        landmarks.bury(&seed, &side);
        assert!(landmarks.fossils.is_empty());
        landmarks.bury(&seed, &deleting(4, 8, &[("src/legacy", 500)]));
        assert_eq!(landmarks.fossils.len(), 1);
        let fossil = &landmarks.fossils[0];
        assert_eq!(
            (fossil.id, fossil.tick, &fossil.commit),
            (1, Tick::new(8), &id(4))
        );
        assert_eq!(fossil.path, hashed(b"src/legacy"));
        assert_eq!(landmarks.next_fossil, 2);
    }

    #[test]
    fn the_largest_directory_names_the_fossil_and_ties_go_to_the_first_path() {
        let seed = seed();
        let named = |dirs: &[(&str, u64)]| {
            let mut landmarks = Landmarks::default();
            landmarks.bury(&seed, &deleting(1, 5, dirs));
            landmarks.fossils[0].path
        };
        assert_eq!(named(&[("a", 600), ("b", 900), ("c", 700)]), hashed(b"b"));
        // One fossil per commit, whatever it deletes.
        let mut landmarks = Landmarks::default();
        landmarks.bury(&seed, &deleting(1, 5, &[("a", 600), ("b", 900)]));
        assert_eq!(landmarks.fossils.len(), 1);
        // A tie goes to the first path, in either input order.
        assert_eq!(named(&[("x", 800), ("y", 800)]), hashed(b"x"));
        assert_eq!(named(&[("y", 800), ("x", 800)]), hashed(b"x"));
    }

    #[test]
    fn at_most_five_fossils_stay_and_the_oldest_leaves_first() {
        let seed = seed();
        let mut landmarks = Landmarks::default();
        for n in 1..=7 {
            landmarks.bury(&seed, &deleting(n, n * 10, &[("old", 1_000)]));
        }
        let ids: Vec<u32> = landmarks.fossils.iter().map(|f| f.id).collect();
        assert_eq!(ids, [3, 4, 5, 6, 7]);
        assert_eq!(landmarks.fossil_views().len(), 5);
    }

    // Expected forms below come from an independent Python model of SHA-256, the stream
    // derivation, SplitMix64, PCG32 and Lemire's ranges.

    #[test]
    fn the_fossil_form_is_known_for_each_commit() {
        let seed = seed();
        // Ids that differ only in their last bytes, as two commits in one tick may.
        let forms: Vec<FossilForm> = (1..=6).map(|n| fossil_form(&seed, &id(n))).collect();
        assert_eq!(forms, FOSSIL_FORMS);
        let other = RepoSeed::from_bytes([8; 32]);
        assert_eq!(fossil_form(&other, &id(1)), OTHER_SEED_FORM);
    }

    const FOSSIL_FORMS: [FossilForm; 6] = [
        FossilForm::LeafPrint,
        FossilForm::LeafPrint,
        FossilForm::Bone,
        FossilForm::Shell,
        FossilForm::Bone,
        FossilForm::Bone,
    ];
    const OTHER_SEED_FORM: FossilForm = FossilForm::LeafPrint;
}
