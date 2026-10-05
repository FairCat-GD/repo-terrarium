//! Monuments and fossils: what the terrarium keeps of its history.
//!
//! - **Monuments.** A standing stone for each full year since the default branch's first
//!   commit, at most three, raised at the first close on or after the anniversary. Each
//!   weathers by its own age: young, lichened from two years, ancient from five. A stone's id is
//!   its place in that order, so nothing else can renumber one.
//! - **Carved marks.** Each new major release (its flowering) carves a mark at once on the
//!   newest stone with room, up to three a stone; with no stone standing, or every stone full,
//!   it waits and is carved when the next stone rises.
//! - **Fossils.** A commit on the default branch's first-parent chain that deletes a whole
//!   directory of 500 lines or more (a rename is not a deletion) leaves one fossil, of the form
//!   the seed draws for that commit. At most five are kept, the oldest leaving first. A side
//!   branch's commits leave none, as they add no terrain: a deletion counts once, when it
//!   reaches the default branch. A history rewrite that drops the commit drops its fossil,
//!   because replay never meets the commit again. A fossil's id is hashed from its commit, so a
//!   replay after a rewrite gives every fossil that survives the id it had, and a rewrite's
//!   sinkhole can name the fossil it took. Two kept fossils sharing an id would take 2⁻³² odds,
//!   which are accepted rather than probed around, since a probe would let a rewrite renumber a
//!   survivor.

use std::cmp::Reverse;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::canon::{canonical_struct, count_of, from_hex};
use crate::domain::CommitFacts;
use crate::identity::{CommitId, RepoSeed};
use crate::rng::{Pcg32, StreamTag, digest_u64, low_u32};
use crate::scene::{FossilForm, FossilView, MAX_MARKS, MonumentAge, MonumentView};
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
    /// Hashed from its commit ([`fossil_id`]).
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Landmarks {
    /// Oldest first.
    pub monuments: Vec<Monument>,
    /// Oldest first.
    pub fossils: Vec<Fossil>,
    /// Carved marks waiting for a stone with room.
    pub pending: u32,
}

canonical_struct!(Landmarks {
    monuments,
    fossils,
    pending,
});

/// A fossil left by a deletion: its id and the lines of the directory that names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buried {
    pub id: u32,
    pub lines: u64,
}

const fn years(n: u32) -> u32 {
    n.saturating_mul(MONUMENT_DAYS)
        .saturating_mul(TICKS_PER_DAY)
}

/// `bytes`' SHA-256, as a number.
fn hashed(bytes: &[u8]) -> u64 {
    digest_u64(&Sha256::digest(bytes).into())
}

/// Separates fossil ids from the form's stream key and every other SHA-256 truth computes.
const FOSSIL_ID_DOMAIN: &[u8] = b"repo-terrarium fossil id v1\0";

/// The id of the fossil `commit` leaves: the low 32 bits of a SHA-256 over the commit id's bytes.
pub fn fossil_id(commit: &CommitId) -> u32 {
    let digest: [u8; 32] = Sha256::new()
        .chain_update(FOSSIL_ID_DOMAIN)
        .chain_update(from_hex(commit.as_str()).unwrap_or_default())
        .finalize()
        .into();
    low_u32(digest_u64(&digest))
}

/// The commit id's bytes, hashed: the key of the streams drawn for one commit.
pub fn commit_key(commit: &CommitId) -> u64 {
    hashed(&from_hex(commit.as_str()).unwrap_or_default())
}

/// The form the seed draws for a fossil left by `commit`. The stream is keyed by a hash of the
/// commit id's bytes, so every commit draws its own, whatever else applies in its tick.
pub fn fossil_form(seed: &RepoSeed, commit: &CommitId) -> FossilForm {
    let key = commit_key(commit);
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
            self.carve();
        }
    }

    /// A new major release: a mark, carved now if a stone has room.
    pub fn release(&mut self) {
        self.pending = self.pending.saturating_add(1);
        self.carve();
    }

    /// Carves waiting marks on the newest stones with room.
    fn carve(&mut self) {
        while self.pending > 0 {
            let Some(stone) = self
                .monuments
                .iter_mut()
                .rev()
                .find(|stone| stone.marks < MAX_MARKS)
            else {
                return;
            };
            stone.marks = stone.marks.saturating_add(1);
            self.pending -= 1;
        }
    }

    /// As `commit` applies: a fossil, if it deletes a whole directory of 500 lines or more on
    /// the default branch's first-parent chain. The largest such directory names it, the first
    /// by path among equals.
    pub fn bury(&mut self, seed: &RepoSeed, commit: &CommitFacts) -> Option<Buried> {
        if !commit.on_default_first_parent {
            return None;
        }
        let dir = commit
            .deleted_dirs
            .iter()
            .filter(|dir| dir.lines >= FOSSIL_MIN_LINES)
            .min_by_key(|dir| (Reverse(dir.lines), dir.path.as_str()))?;
        let id = fossil_id(&commit.id);
        self.fossils.push(Fossil {
            id,
            form: fossil_form(seed, &commit.id),
            tick: commit.tick,
            commit: commit.id.clone(),
            path: hashed(dir.path.as_bytes()),
        });
        let excess = self.fossils.len().saturating_sub(FOSSILS_MAX);
        self.fossils.drain(..excess);
        Some(Buried {
            id,
            lines: dir.lines,
        })
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
            (fossil_id(&id(4)), Tick::new(8), &id(4))
        );
        assert_eq!(fossil.path, hashed(b"src/legacy"));
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
        let kept: Vec<u32> = (3..=7).map(|n| fossil_id(&id(n))).collect();
        assert_eq!(ids, kept);
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

    #[test]
    fn a_release_is_carved_on_the_newest_stone_with_room_or_waits_for_one() {
        let mut landmarks = Landmarks::default();
        let marks = |l: &Landmarks| -> Vec<u8> { l.monuments.iter().map(|m| m.marks).collect() };
        // Before any stone, releases wait.
        landmarks.release();
        landmarks.release();
        assert_eq!(landmarks.pending, 2);
        let root = Some(Tick::ZERO);
        landmarks.raise(Tick::new(YEAR), root);
        assert_eq!((marks(&landmarks), landmarks.pending), (vec![2], 0));
        landmarks.raise(Tick::new(2 * YEAR), root);
        // The newest stone takes the next, though the older has room.
        landmarks.release();
        assert_eq!(marks(&landmarks), [2, 1]);
        landmarks.release();
        landmarks.release();
        assert_eq!(marks(&landmarks), [2, 3]);
        // A full one passes to an older one with room.
        landmarks.release();
        assert_eq!(marks(&landmarks), [3, 3]);
        // Every stone full: it waits for the third.
        landmarks.release();
        assert_eq!(landmarks.pending, 1);
        landmarks.raise(Tick::new(3 * YEAR), root);
        assert_eq!((marks(&landmarks), landmarks.pending), (vec![3, 3, 1], 0));
        assert!(
            landmarks
                .monument_views(Tick::new(3 * YEAR))
                .iter()
                .all(|view| view.marks <= MAX_MARKS)
        );
    }

    #[test]
    fn a_fossil_keeps_its_id_whatever_else_was_buried() {
        let seed = seed();
        let mut one = Landmarks::default();
        one.bury(&seed, &deleting(1, 5, &[("a", 600)]));
        one.bury(&seed, &deleting(2, 6, &[("b", 900)]));
        // The history without the first deletion: the second keeps its id.
        let mut other = Landmarks::default();
        let buried = other.bury(&seed, &deleting(2, 6, &[("b", 900)]));
        assert_eq!(
            buried,
            Some(Buried {
                id: fossil_id(&id(2)),
                lines: 900
            })
        );
        assert_eq!(one.fossils[1].id, other.fossils[0].id);
        assert_ne!(one.fossils[0].id, one.fossils[1].id);
    }
}
