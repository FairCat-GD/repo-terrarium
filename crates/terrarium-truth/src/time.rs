//! Truth's clock: a one-hour UTC tick grid.
//!
//! Tick 0 is the hour containing the earliest counted commit, so no counted commit has a
//! negative tick. Local time, time zones and day and night belong to the composer; truth only
//! ever sees UTC seconds turned into ticks.

use serde::{Deserialize, Serialize};

pub const SECONDS_PER_TICK: i64 = 3600;
pub const TICKS_PER_DAY: u32 = 24;

/// Hours since tick 0 of a terrarium's grid. `u32` covers about 490,000 years.
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Tick(u32);

impl Tick {
    pub const ZERO: Tick = Tick(0);

    pub const fn new(hours: u32) -> Tick {
        Tick(hours)
    }

    pub const fn hours(self) -> u32 {
        self.0
    }

    pub const fn saturating_add(self, hours: u32) -> Tick {
        Tick(self.0.saturating_add(hours))
    }

    /// Whole hours from `earlier` to `self`, or 0 if `earlier` is later.
    pub const fn hours_since(self, earlier: Tick) -> u32 {
        self.0.saturating_sub(earlier.0)
    }
}

/// Maps UTC seconds onto one terrarium's ticks. It is encoded as its origin, and decoding
/// refuses an origin that is not a whole hour.
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "i64", try_from = "i64")]
pub struct TickGrid {
    /// UTC seconds at the start of tick 0, a whole hour.
    origin: i64,
}

impl TickGrid {
    /// The grid whose tick 0 is the hour containing `earliest_utc_secs`. Within the first
    /// hour of `i64`, whose start is not representable, it is the first whole hour instead.
    pub const fn containing(earliest_utc_secs: i64) -> TickGrid {
        let hour = earliest_utc_secs.div_euclid(SECONDS_PER_TICK);
        let origin = match hour.checked_mul(SECONDS_PER_TICK) {
            Some(origin) => origin,
            None => (hour + 1) * SECONDS_PER_TICK,
        };
        TickGrid { origin }
    }

    /// The grid whose tick 0 starts at `origin_secs`, which must be a whole hour.
    pub const fn from_origin(origin_secs: i64) -> Option<TickGrid> {
        if origin_secs.rem_euclid(SECONDS_PER_TICK) == 0 {
            Some(TickGrid {
                origin: origin_secs,
            })
        } else {
            None
        }
    }

    /// UTC seconds at the start of tick 0.
    pub const fn origin_secs(self) -> i64 {
        self.origin
    }

    /// The tick containing `utc_secs`: floored, so the truth of "now" is `tick_at(now)`.
    /// `None` before tick 0 or past the last tick.
    pub fn tick_at(self, utc_secs: i64) -> Option<Tick> {
        let since_origin = utc_secs.checked_sub(self.origin)?;
        if since_origin < 0 {
            return None;
        }
        u32::try_from(since_origin / SECONDS_PER_TICK)
            .ok()
            .map(Tick)
    }

    /// UTC seconds at the start of `tick`.
    pub fn start_secs(self, tick: Tick) -> i64 {
        self.origin
            .saturating_add(i64::from(tick.0).saturating_mul(SECONDS_PER_TICK))
    }
}

impl From<TickGrid> for i64 {
    fn from(grid: TickGrid) -> i64 {
        grid.origin
    }
}

impl TryFrom<i64> for TickGrid {
    type Error = &'static str;

    fn try_from(origin_secs: i64) -> Result<TickGrid, Self::Error> {
        TickGrid::from_origin(origin_secs).ok_or("a tick grid starts on a whole hour")
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::test_support::proptest_config;

    #[test]
    fn tick_zero_is_the_hour_containing_the_earliest_commit() {
        assert_eq!(TickGrid::containing(0).origin_secs(), 0);
        assert_eq!(TickGrid::containing(3_599).origin_secs(), 0);
        assert_eq!(TickGrid::containing(3_600).origin_secs(), 3_600);
        // Pre-1970 times floor too, rather than truncating toward zero.
        assert_eq!(TickGrid::containing(-1).origin_secs(), -3_600);
        assert_eq!(TickGrid::containing(-3_600).origin_secs(), -3_600);
        for extreme in [i64::MIN, i64::MAX] {
            let origin = TickGrid::containing(extreme).origin_secs();
            assert_eq!(
                TickGrid::from_origin(origin).map(TickGrid::origin_secs),
                Some(origin)
            );
        }
    }

    #[test]
    fn times_floor_onto_ticks() {
        let grid = TickGrid::containing(1_700_000_000);
        let origin = grid.origin_secs();
        assert_eq!(grid.tick_at(origin - 1), None);
        assert_eq!(grid.tick_at(origin), Some(Tick::ZERO));
        assert_eq!(grid.tick_at(origin + 3_599), Some(Tick::ZERO));
        assert_eq!(grid.tick_at(origin + 3_600), Some(Tick::new(1)));
        assert_eq!(grid.tick_at(i64::MAX), None);
        assert_eq!(TickGrid::containing(i64::MAX).tick_at(i64::MIN), None);
        assert_eq!(grid.start_secs(Tick::new(24)), origin + 86_400);
        assert_eq!(Tick::new(5).hours_since(Tick::new(2)), 3);
        assert_eq!(Tick::new(2).hours_since(Tick::new(5)), 0);
        assert_eq!(Tick::new(u32::MAX).saturating_add(1), Tick::new(u32::MAX));
    }

    #[test]
    fn a_decoded_grid_starts_on_a_whole_hour() {
        assert_eq!(TickGrid::from_origin(1_800), None);
        assert_eq!(TickGrid::from_origin(-3_599), None);
        assert_eq!(
            TickGrid::from_origin(-3_600),
            Some(TickGrid::containing(-3_600))
        );
        let grid = TickGrid::containing(1_700_000_000);
        let bytes = postcard::to_allocvec(&grid).expect("encodes");
        assert_eq!(postcard::from_bytes::<TickGrid>(&bytes), Ok(grid));
        let off_hour = postcard::to_allocvec(&1_800_i64).expect("encodes");
        assert!(postcard::from_bytes::<TickGrid>(&off_hour).is_err());
    }

    proptest! {
        #![proptest_config(proptest_config())]

        #[test]
        fn a_tick_starts_where_it_is_found(earliest in -(1_i64 << 40)..(1_i64 << 40), hours: u32) {
            let grid = TickGrid::containing(earliest);
            prop_assert!(grid.origin_secs() <= earliest);
            prop_assert!(earliest - grid.origin_secs() < SECONDS_PER_TICK);
            let tick = Tick::new(hours);
            let start = grid.start_secs(tick);
            prop_assert_eq!(grid.tick_at(start), Some(tick));
            prop_assert_eq!(grid.tick_at(start + SECONDS_PER_TICK - 1), Some(tick));
        }
    }
}
