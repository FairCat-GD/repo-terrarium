//! The compiled tuning values.
//!
//! Every value here is part of truth's output, so changing one bumps `SIM_VERSION`. They are
//! constants rather than settings: if a user could edit them, two installs would disagree about
//! what happened to the same repository.

use crate::fixed::{Q16, Q32};

pub const SECONDS_PER_DAY: i64 = 86_400;

/// The window every smoothed climate averages over, as an exponential moving average whose
/// weight is `2 / (14 + 1)`.
pub const SMOOTHING_ALPHA: Q32 = Q32::from_ratio(2, 15);

/// How many daily levels the rain and wind baselines take their median over.
pub const BASELINE_DAYS: usize = 90;

/// The lowest baseline rain and wind are compared with: `log2(1 + 1/7)`, one commit a week. A
/// test pins it to the logarithm it stands for.
pub const BASELINE_FLOOR: Q32 = Q32::from_raw(12_625 << 16);

/// History shorter than this (14 days) uses a neutral rain and wind ratio of 1, so activity is
/// a typical 0.5 while the baselines settle.
pub const SETTLING_TICKS: u32 = 14 * 24;

/// Activity is `0.5 × (0.75 × rain + 0.25 × wind)`, that is `(3 × rain + wind) / 8`.
pub const ACTIVITY_RAIN_WEIGHT: i64 = 3;
pub const ACTIVITY_WIND_WEIGHT: i64 = 1;
pub const ACTIVITY_DIVISOR: i64 = 8;

/// What an unknown light or soil counts as in health.
pub const NEUTRAL: Q16 = Q16::HALF;

/// Health from which the terrarium turns healthy, and below which it turns unhealthy; in
/// between, the previous class holds.
pub const HEALTHY_FROM: Q16 = Q16::from_ratio(55, 100);
pub const UNHEALTHY_BELOW: Q16 = Q16::from_ratio(45, 100);

/// Activity from which a repository that is not inactive counts as active.
pub const ACTIVE_FROM: Q16 = Q16::from_ratio(35, 100);

/// Thirty days without a commit on any branch is inactive.
pub const INACTIVE_TICKS: u32 = 30 * 24;

/// Recovering looks back 14 days, and needs health to have risen by 0.10 over them.
pub const RECOVERY_WINDOW_TICKS: u32 = 14 * 24;
pub const RECOVERY_GAIN: Q16 = Q16::from_ratio(10, 100);

/// The daily records kept: enough to reach 14 days back from anywhere in the newest day.
pub const DAILY_RECORDS: usize = 15;

/// A trend compares a reading with its value at least 7 days earlier; a change smaller than
/// 0.02 is flat.
pub const TREND_TICKS: u32 = 7 * 24;
pub const TREND_STEP: Q16 = Q16::from_ratio(2, 100);

/// Days in the dormant state at which condensation reaches levels 1, 2 and 3.
pub const CONDENSATION_DAYS: [u32; 3] = [0, 30, 90];
/// Days in the dead state at which dust reaches levels 1, 2 and 3.
pub const DUST_DAYS: [u32; 3] = [365, 540, 730];

/// The wind ratio from which the wind is breezy, and from which it is a gale: the windstorm's
/// ratio. A typical day, at ratio 1, is breezy.
pub const BREEZY_FROM: Q16 = Q16::from_ratio(7, 10);
pub const GALE_FROM: Q16 = Q16::from_int(3);

/// A coverage report that moved by at least 0.02 from the one before it (the sunburst and the
/// dusk) colours the sky for a week after it was recorded.
pub const LIGHT_SHIFT_STEP: Q16 = Q16::from_ratio(2, 100);
pub const LIGHT_SHIFT_TICKS: u32 = 7 * 24;

/// A weather reading at or above this is a pass.
pub const PASS_FROM: Q16 = Q16::HALF;

/// The stand-in vegetation's foliage: thriving from 0.60, alive from 0.35 (the vigor bands,
/// read from health).
pub const THRIVING_FROM: Q16 = Q16::from_ratio(60, 100);
pub const ALIVE_FROM: Q16 = Q16::from_ratio(35, 100);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_baseline_floor_is_log2_of_one_and_a_seventh() {
        let floor = Q32::from_ratio(8, 7).log2().expect("positive");
        assert_eq!(BASELINE_FLOOR, floor.to_q32());
        assert_eq!(floor.raw(), 12_625);
    }

    #[test]
    fn thresholds_are_the_ratios_they_name() {
        assert_eq!(SMOOTHING_ALPHA, Q32::from_ratio(2, 15));
        assert_eq!(HEALTHY_FROM.raw(), 36_045);
        assert_eq!(UNHEALTHY_BELOW.raw(), 29_491);
        assert_eq!(ACTIVE_FROM.raw(), 22_938);
        assert_eq!(RECOVERY_GAIN.raw(), 6_554);
        assert_eq!(TREND_STEP.raw(), 1_311);
        assert_eq!(BREEZY_FROM.raw(), 45_875);
        assert_eq!(GALE_FROM.raw(), 196_608);
        assert_eq!(THRIVING_FROM.raw(), 39_322);
        assert_eq!(ALIVE_FROM.raw(), 22_938);
        assert_eq!(LIGHT_SHIFT_STEP.raw(), 1_311);
        assert_eq!(NEUTRAL, Q16::from_ratio(1, 2));
    }
}
