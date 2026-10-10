//! What the connected terrarium's scenes last had the chance to play, kept across restarts. The
//! cached state is the one before the open hour, so without it a restart within an event's hour
//! would play that event again.
//!
//! It is the install's memory of what it showed, not truth: it is kept for one terrarium at a
//! time, and read back only for the same seed, simulation version, default branch and tick
//! grid, which together give its event keys their meaning.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use terrarium_store::{InstallKey, Store, StoreError};
use terrarium_truth::scene::EventKind;
use terrarium_truth::{RepoSeed, SIM_VERSION, Tick, TickGrid};

/// The event instances a scene had the chance to play.
pub type Seen = BTreeSet<(Tick, EventKind)>;

/// The stored record (the `shown_events` install value), as JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Remembered {
    seed: String,
    sim_version: u32,
    default_branch: String,
    grid_origin: i64,
    seen: Seen,
}

impl Remembered {
    pub fn new(seed: &RepoSeed, default_branch: &str, grid: TickGrid, played: Seen) -> Remembered {
        Remembered {
            seed: seed.to_hex(),
            sim_version: SIM_VERSION.0,
            default_branch: default_branch.to_owned(),
            grid_origin: grid.origin_secs(),
            seen: played,
        }
    }

    /// The record stored, if one is and it reads.
    pub fn read(store: &Store) -> Result<Option<Remembered>, StoreError> {
        Ok(store
            .install_value(InstallKey::ShownEvents)?
            .and_then(|text| serde_json::from_str(&text).ok()))
    }

    pub fn write(&self, store: &mut Store) -> Result<(), StoreError> {
        let text = serde_json::to_string(self)?;
        store.set_install_value(InstallKey::ShownEvents, &text)
    }

    /// What it holds, if it was kept for `seed` on `default_branch` and `grid` under this
    /// simulation version.
    pub fn seen_for(self, seed: &RepoSeed, default_branch: &str, grid: TickGrid) -> Option<Seen> {
        (self.seed == seed.to_hex()
            && self.sim_version == SIM_VERSION.0
            && self.default_branch == default_branch
            && self.grid_origin == grid.origin_secs())
        .then_some(self.seen)
    }
}

/// Whether the stored record was kept for `seed`.
pub(crate) fn names(store: &Store, seed: &RepoSeed) -> Result<bool, StoreError> {
    Ok(Remembered::read(store)?.is_some_and(|r| r.seed == seed.to_hex()))
}

#[cfg(test)]
mod tests {
    use terrarium_store::DataDir;

    use super::*;

    #[test]
    fn the_record_is_pinned_and_read_back_only_where_it_was_kept() {
        let seed = RepoSeed::from_bytes([3; 32]);
        let grid = TickGrid::from_origin(1_704_672_000).unwrap();
        let played: Seen = [(Tick::new(12), EventKind::Earthquake)].into();
        let remembered = Remembered::new(&seed, "main", grid, played.clone());
        // Stored data: this text never changes.
        assert_eq!(
            serde_json::to_string(&remembered).unwrap(),
            format!(
                r#"{{"seed":"{}","sim_version":{},"default_branch":"main","grid_origin":1704672000,"seen":[[12,"Earthquake"]]}}"#,
                "03".repeat(32),
                SIM_VERSION.0
            )
        );
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&DataDir::at(dir.path())).unwrap();
        assert_eq!(Remembered::read(&store).unwrap(), None);
        remembered.write(&mut store).unwrap();
        let read = Remembered::read(&store).unwrap().unwrap();
        assert!(names(&store, &seed).unwrap());
        assert!(!names(&store, &RepoSeed::from_bytes([4; 32])).unwrap());
        let other_grid = TickGrid::from_origin(1_704_675_600).unwrap();
        assert_eq!(read.clone().seen_for(&seed, "main", grid), Some(played));
        assert_eq!(read.clone().seen_for(&seed, "trunk", grid), None);
        assert_eq!(read.clone().seen_for(&seed, "main", other_grid), None);
        assert_eq!(
            read.seen_for(&RepoSeed::from_bytes([4; 32]), "main", grid),
            None
        );
        let older = Remembered {
            sim_version: SIM_VERSION.0 - 1,
            ..remembered
        };
        assert_eq!(older.seen_for(&seed, "main", grid), None);
        // A record that does not read is no record.
        store
            .set_install_value(InstallKey::ShownEvents, "{\"seed\":1}")
            .unwrap();
        assert_eq!(Remembered::read(&store).unwrap(), None);
    }
}
