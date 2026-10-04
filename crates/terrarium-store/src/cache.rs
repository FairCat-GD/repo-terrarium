//! The replay cache: one row per terrarium, an optimisation and never authoritative. The store
//! keeps it; the engine decides whether its key still matches.

use rusqlite::{OptionalExtension, params};
use terrarium_truth::{SimVersion, Tick};

use crate::{ObservationId, Store, StoreError, TerrariumId};

/// A replay's saved state and the key it is valid for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimCache {
    pub sim_version: SimVersion,
    /// A hash of every local branch tip and tag.
    pub tips_hash: [u8; 32],
    /// The newest observation the state includes; `None` before the first. Stored as 0, which
    /// no row id takes.
    pub obs_high_water: Option<ObservationId>,
    pub tick: Tick,
    /// `postcard(SimState)`.
    pub state: Vec<u8>,
}

impl Store {
    pub fn sim_cache(&self, terrarium: TerrariumId) -> Result<Option<SimCache>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT sim_version, tips_hash, obs_high_water, tick, state \
                 FROM sim_cache WHERE terrarium_id = ?1",
                [terrarium.get()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((sim_version, tips_hash, high_water, tick, state)) = row else {
            return Ok(None);
        };
        Ok(Some(SimCache {
            sim_version: SimVersion(
                u32::try_from(sim_version).map_err(|_| StoreError::Corrupt("sim_version"))?,
            ),
            tips_hash: tips_hash
                .try_into()
                .map_err(|_| StoreError::Corrupt("tips_hash"))?,
            obs_high_water: match high_water {
                0 => None,
                id if id > 0 => Some(ObservationId::new(id)),
                _ => return Err(StoreError::Corrupt("obs_high_water")),
            },
            tick: Tick::new(u32::try_from(tick).map_err(|_| StoreError::Corrupt("tick"))?),
            state,
        }))
    }

    /// Replaces the terrarium's cached replay.
    pub fn write_sim_cache(
        &mut self,
        terrarium: TerrariumId,
        cache: &SimCache,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO sim_cache (terrarium_id, sim_version, tips_hash, obs_high_water, tick, \
             state) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT (terrarium_id) DO UPDATE SET sim_version = excluded.sim_version, \
             tips_hash = excluded.tips_hash, obs_high_water = excluded.obs_high_water, \
             tick = excluded.tick, state = excluded.state",
            params![
                terrarium.get(),
                cache.sim_version.0,
                cache.tips_hash,
                cache.obs_high_water.map_or(0, ObservationId::get),
                cache.tick.hours(),
                cache.state,
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Fixture, new_terrarium};

    fn cache(high_water: Option<i64>, tick: u32) -> SimCache {
        SimCache {
            sim_version: SimVersion(3),
            tips_hash: [9; 32],
            obs_high_water: high_water.map(ObservationId::new),
            tick: Tick::new(tick),
            state: vec![0, 1, 2, 255],
        }
    }

    #[test]
    fn the_cache_round_trips_and_a_write_replaces_it() {
        let mut fx = Fixture::new();
        let one = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        let two = fx.store.create_terrarium(&new_terrarium(2)).unwrap();
        assert_eq!(fx.store.sim_cache(one).unwrap(), None);
        fx.store.write_sim_cache(one, &cache(None, 0)).unwrap();
        assert_eq!(fx.store.sim_cache(one).unwrap(), Some(cache(None, 0)));
        let newer = cache(Some(41), u32::MAX);
        fx.store.write_sim_cache(one, &newer).unwrap();
        assert_eq!(fx.store.sim_cache(one).unwrap(), Some(newer));
        assert_eq!(fx.store.sim_cache(two).unwrap(), None);
    }

    #[test]
    fn a_cache_needs_a_known_terrarium() {
        let mut fx = Fixture::new();
        let err = fx
            .store
            .write_sim_cache(TerrariumId::new(3), &cache(None, 1))
            .unwrap_err();
        assert!(err.to_string().contains("FOREIGN KEY"), "{err}");
    }
}
