//! Terrariums: one per connected repository, keyed by its seed so a moved repository relinks to
//! its history.

use rusqlite::{OptionalExtension, Row, params};
use terrarium_truth::RepoSeed;

use crate::error::is_unique_violation;
use crate::identifiers::{INSTALL_DELETING_TERRARIUM, SETTING_REPO_ACTIVE};
use crate::{Store, StoreError};

/// A terrarium's row id. It is local to this install; the seed is what identifies a
/// terrarium across installs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TerrariumId(i64);

impl TerrariumId {
    pub const fn new(id: i64) -> TerrariumId {
        TerrariumId(id)
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

/// Where a repository was last connected from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoLocation {
    /// The absolute path.
    pub path: String,
    pub name: String,
    /// Recorded at connect; a change forces a full replay.
    pub default_branch: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewTerrarium {
    pub seed: RepoSeed,
    pub repo: RepoLocation,
    /// Cosmetics only (I-2).
    pub install_salt: [u8; 16],
    /// Unix seconds, UTC.
    pub connected_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terrarium {
    pub id: TerrariumId,
    pub seed: RepoSeed,
    pub repo: RepoLocation,
    pub install_salt: [u8; 16],
    pub connected_at: i64,
    /// Connected, as opposed to disconnected with its observations kept.
    pub active: bool,
}

const COLUMNS: &str =
    "id, seed, repo_path, repo_name, default_branch, install_salt, connected_at, active";

fn read_terrarium(row: &Row<'_>) -> rusqlite::Result<Terrarium> {
    Ok(Terrarium {
        id: TerrariumId(row.get(0)?),
        seed: RepoSeed::from_bytes(row.get(1)?),
        repo: RepoLocation {
            path: row.get(2)?,
            name: row.get(3)?,
            default_branch: row.get(4)?,
        },
        install_salt: row.get(5)?,
        connected_at: row.get(6)?,
        active: row.get(7)?,
    })
}

impl Store {
    /// Records a newly connected repository, active.
    pub fn create_terrarium(&mut self, new: &NewTerrarium) -> Result<TerrariumId, StoreError> {
        let inserted = self.conn.execute(
            "INSERT INTO terrariums \
             (seed, repo_path, repo_name, default_branch, install_salt, connected_at, active) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
            params![
                new.seed.as_bytes(),
                new.repo.path,
                new.repo.name,
                new.repo.default_branch,
                new.install_salt,
                new.connected_at,
            ],
        );
        match inserted {
            Ok(_) => Ok(TerrariumId(self.conn.last_insert_rowid())),
            Err(err) if is_unique_violation(&err) => Err(StoreError::SeedTaken),
            Err(err) => Err(err.into()),
        }
    }

    pub fn terrarium(&self, id: TerrariumId) -> Result<Option<Terrarium>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM terrariums WHERE id = ?1");
        Ok(self
            .conn
            .query_row(&sql, [id.0], read_terrarium)
            .optional()?)
    }

    /// The terrarium a repository with this seed was connected as, so reconnecting it, from
    /// wherever it now lives, keeps its history.
    pub fn terrarium_by_seed(&self, seed: &RepoSeed) -> Result<Option<Terrarium>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM terrariums WHERE seed = ?1");
        Ok(self
            .conn
            .query_row(&sql, [seed.as_bytes()], read_terrarium)
            .optional()?)
    }

    /// Connects a known terrarium again from `repo`. A different default branch drops the
    /// cached replay as well: it forces a full replay, and the cached state, whose basis names
    /// the old branch, would never resume.
    pub fn activate(&mut self, id: TerrariumId, repo: &RepoLocation) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        let branch: Option<String> = tx
            .query_row(
                "SELECT default_branch FROM terrariums WHERE id = ?1",
                [id.0],
                |row| row.get(0),
            )
            .optional()?;
        let Some(branch) = branch else {
            return Err(StoreError::NoSuchTerrarium(id));
        };
        tx.execute(
            "UPDATE terrariums SET active = 1, repo_path = ?2, repo_name = ?3, \
             default_branch = ?4 WHERE id = ?1",
            params![id.0, repo.path, repo.name, repo.default_branch],
        )?;
        if branch != repo.default_branch {
            tx.execute("DELETE FROM sim_cache WHERE terrarium_id = ?1", [id.0])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// A new terrarium for the same repository (re-seed): a new install salt, so everything the
    /// salt places takes a new place, and the cached replay dropped. Its observations, and with
    /// them its truth, are kept.
    pub fn reseed(&mut self, id: TerrariumId, install_salt: [u8; 16]) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        if tx.execute(
            "UPDATE terrariums SET install_salt = ?2 WHERE id = ?1",
            params![id.0, install_salt],
        )? != 1
        {
            return Err(StoreError::NoSuchTerrarium(id));
        }
        tx.execute("DELETE FROM sim_cache WHERE terrarium_id = ?1", [id.0])?;
        tx.commit()?;
        Ok(())
    }

    /// Disconnects a terrarium and keeps its observations: the row stays, so connecting the
    /// same repository again relinks to it. A plain run connects nothing after.
    pub fn disconnect(&mut self, id: TerrariumId) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        if tx.execute("UPDATE terrariums SET active = 0 WHERE id = ?1", [id.0])? != 1 {
            return Err(StoreError::NoSuchTerrarium(id));
        }
        tx.execute("DELETE FROM sim_cache WHERE terrarium_id = ?1", [id.0])?;
        forget_active(&tx, id)?;
        tx.commit()?;
        Ok(())
    }

    /// Deletes a terrarium with its observations and cache: the only path that deletes
    /// observations. It opens the append-only trigger's guard inside one transaction and closes
    /// it before committing, so the guard is never seen open outside. The app asks for
    /// confirmation first.
    pub fn delete_terrarium(&mut self, id: TerrariumId) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO install (key, value) VALUES (?1, ?2)",
            params![INSTALL_DELETING_TERRARIUM, id.0.to_string()],
        )?;
        tx.execute("DELETE FROM sim_cache WHERE terrarium_id = ?1", [id.0])?;
        tx.execute("DELETE FROM observations WHERE terrarium_id = ?1", [id.0])?;
        if tx.execute("DELETE FROM terrariums WHERE id = ?1", [id.0])? != 1 {
            // Dropping the transaction rolls the guard back with everything else.
            return Err(StoreError::NoSuchTerrarium(id));
        }
        tx.execute(
            "DELETE FROM install WHERE key = ?1",
            [INSTALL_DELETING_TERRARIUM],
        )?;
        forget_active(&tx, id)?;
        tx.commit()?;
        Ok(())
    }
}

/// Clears `repo.active` when it names `id`.
fn forget_active(tx: &rusqlite::Transaction<'_>, id: TerrariumId) -> Result<(), StoreError> {
    let named = serde_json::to_string(&id.0)?;
    tx.execute(
        "DELETE FROM settings WHERE key = ?1 AND value = ?2",
        params![SETTING_REPO_ACTIVE, named],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use terrarium_truth::ObservationKind;

    use super::*;
    use crate::test_support::{Fixture, location, new_terrarium, record, seed, sim_cache as cache};

    fn count(fx: &Fixture, sql: &str) -> i64 {
        fx.store.conn.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    fn guard_is_closed(fx: &Fixture) -> bool {
        count(
            fx,
            "SELECT count(*) FROM install WHERE key = 'deleting_terrarium'",
        ) == 0
    }

    #[test]
    fn a_terrarium_is_found_by_id_and_by_seed() {
        let mut fx = Fixture::new();
        let new = new_terrarium(1);
        let id = fx.store.create_terrarium(&new).unwrap();
        let expected = Terrarium {
            id,
            seed: new.seed,
            repo: new.repo.clone(),
            install_salt: new.install_salt,
            connected_at: new.connected_at,
            active: true,
        };
        assert_eq!(fx.store.terrarium(id).unwrap(), Some(expected.clone()));
        assert_eq!(
            fx.store.terrarium_by_seed(&new.seed).unwrap(),
            Some(expected)
        );
        assert_eq!(fx.store.terrarium_by_seed(&seed(2)).unwrap(), None);
        assert_eq!(fx.store.terrarium(TerrariumId(id.0 + 1)).unwrap(), None);
    }

    #[test]
    fn a_seed_names_one_terrarium() {
        let mut fx = Fixture::new();
        fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        let mut again = new_terrarium(1);
        again.repo.path = "/elsewhere/meadow".to_owned();
        assert!(matches!(
            fx.store.create_terrarium(&again),
            Err(StoreError::SeedTaken)
        ));
    }

    #[test]
    fn disconnecting_keeps_the_row_and_observations_and_drops_the_cache() {
        let mut fx = Fixture::new();
        let id = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        fx.store
            .append_observation(id, &record(ObservationKind::Coverage, 100))
            .unwrap();
        fx.store.write_sim_cache(id, &cache(Some(1), 40)).unwrap();
        fx.store.disconnect(id).unwrap();
        assert!(!fx.store.terrarium(id).unwrap().unwrap().active);
        assert_eq!(fx.store.sim_cache(id).unwrap(), None);
        assert_eq!(fx.store.observations(id, None).unwrap().len(), 1);
    }

    #[test]
    fn reseeding_replaces_the_salt_drops_the_cache_and_keeps_the_rest() {
        let mut fx = Fixture::new();
        let id = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        let other = fx.store.create_terrarium(&new_terrarium(2)).unwrap();
        fx.store
            .append_observation(id, &record(ObservationKind::Coverage, 100))
            .unwrap();
        for terrarium in [id, other] {
            fx.store
                .write_sim_cache(terrarium, &cache(Some(1), 40))
                .unwrap();
        }
        let before = fx.store.terrarium(id).unwrap().unwrap();
        fx.store.reseed(id, [42; 16]).unwrap();
        let after = fx.store.terrarium(id).unwrap().unwrap();
        assert_eq!(after.install_salt, [42; 16]);
        assert_eq!(
            Terrarium {
                install_salt: before.install_salt,
                ..after
            },
            before,
            "nothing else"
        );
        assert_eq!(fx.store.sim_cache(id).unwrap(), None);
        assert_eq!(fx.store.observations(id, None).unwrap().len(), 1);
        assert_eq!(fx.store.sim_cache(other).unwrap(), Some(cache(Some(1), 40)));
        assert!(matches!(
            fx.store.reseed(TerrariumId(9), [0; 16]),
            Err(StoreError::NoSuchTerrarium(_))
        ));
    }

    #[test]
    fn disconnecting_either_way_forgets_the_active_terrarium_only_when_it_is_that_one() {
        for delete in [false, true] {
            let mut fx = Fixture::new();
            let one = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
            let two = fx.store.create_terrarium(&new_terrarium(2)).unwrap();
            fx.store.set_setting(SETTING_REPO_ACTIVE, &two.0).unwrap();
            let disconnect = |fx: &mut Fixture, id| {
                if delete {
                    fx.store.delete_terrarium(id)
                } else {
                    fx.store.disconnect(id)
                }
            };
            disconnect(&mut fx, one).unwrap();
            assert_eq!(
                fx.store.setting::<i64>(SETTING_REPO_ACTIVE).unwrap(),
                Some(two.0),
                "another terrarium stays active"
            );
            disconnect(&mut fx, two).unwrap();
            assert_eq!(fx.store.setting::<i64>(SETTING_REPO_ACTIVE).unwrap(), None);
        }
    }

    #[test]
    fn activating_relinks_and_a_new_default_branch_drops_the_cache() {
        let mut fx = Fixture::new();
        let id = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        fx.store.write_sim_cache(id, &cache(Some(1), 40)).unwrap();
        fx.store.disconnect(id).unwrap();

        let mut moved = location(1);
        moved.path = "/moved/meadow".to_owned();
        fx.store.write_sim_cache(id, &cache(Some(1), 40)).unwrap();
        fx.store.activate(id, &moved).unwrap();
        let relinked = fx.store.terrarium(id).unwrap().unwrap();
        assert!(relinked.active);
        assert_eq!(relinked.repo, moved);
        assert_eq!(
            fx.store.sim_cache(id).unwrap(),
            Some(cache(Some(1), 40)),
            "same branch"
        );

        moved.default_branch = "trunk".to_owned();
        fx.store.activate(id, &moved).unwrap();
        assert_eq!(
            fx.store.terrarium(id).unwrap().unwrap().repo.default_branch,
            "trunk"
        );
        assert_eq!(fx.store.sim_cache(id).unwrap(), None, "new default branch");
    }

    #[test]
    fn an_unknown_terrarium_cannot_be_activated_disconnected_or_deleted() {
        let mut fx = Fixture::new();
        let ghost = TerrariumId(9);
        assert!(matches!(
            fx.store.activate(ghost, &location(1)),
            Err(StoreError::NoSuchTerrarium(id)) if id == ghost
        ));
        assert!(matches!(
            fx.store.disconnect(ghost),
            Err(StoreError::NoSuchTerrarium(_))
        ));
        assert!(matches!(
            fx.store.delete_terrarium(ghost),
            Err(StoreError::NoSuchTerrarium(_))
        ));
        assert!(
            guard_is_closed(&fx),
            "the failed delete rolled its guard back"
        );
    }

    #[test]
    fn the_guarded_delete_removes_one_terrarium_and_closes_its_guard() {
        let mut fx = Fixture::new();
        let gone = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        let kept = fx.store.create_terrarium(&new_terrarium(2)).unwrap();
        for id in [gone, kept] {
            for at in [100, 200] {
                fx.store
                    .append_observation(id, &record(ObservationKind::Coverage, at))
                    .unwrap();
            }
            fx.store.write_sim_cache(id, &cache(Some(1), 40)).unwrap();
        }

        fx.store.delete_terrarium(gone).unwrap();
        assert_eq!(fx.store.terrarium(gone).unwrap(), None);
        assert_eq!(fx.store.observations(gone, None).unwrap(), vec![]);
        assert_eq!(fx.store.sim_cache(gone).unwrap(), None);
        assert_eq!(fx.store.observations(kept, None).unwrap().len(), 2);
        assert_eq!(fx.store.sim_cache(kept).unwrap(), Some(cache(Some(1), 40)));
        assert!(guard_is_closed(&fx));

        let raw_delete = fx.store.conn.execute("DELETE FROM observations", []);
        assert!(
            raw_delete
                .unwrap_err()
                .to_string()
                .contains("observations are append-only"),
            "the guard is closed again"
        );
        assert_eq!(count(&fx, "SELECT count(*) FROM observations"), 2);
    }

    #[test]
    fn a_terrarium_with_observations_cannot_be_deleted_around_the_guard() {
        let mut fx = Fixture::new();
        let id = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        fx.store
            .append_observation(id, &record(ObservationKind::Coverage, 100))
            .unwrap();
        let raw = fx
            .store
            .conn
            .execute("DELETE FROM terrariums WHERE id = ?1", [id.0]);
        assert!(
            raw.unwrap_err().to_string().contains("FOREIGN KEY"),
            "the observations still point at it"
        );
        assert!(fx.store.terrarium(id).unwrap().is_some());
    }
}
