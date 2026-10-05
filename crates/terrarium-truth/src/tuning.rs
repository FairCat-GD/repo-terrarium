//! The compiled tuning values.
//!
//! Every value here is part of truth's output, so changing one bumps `SIM_VERSION`. They are
//! constants rather than settings: if a user could edit them, two installs would disagree about
//! what happened to the same repository.

use crate::fixed::{Q16, Q32};
use crate::scene::ResidentSpecies;

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

/// Vigor's bands: the foliage is thriving from 0.60; below 0.35 a plant is failing, and after
/// `DYING_DAYS` there it dies.
pub const THRIVING_FROM: Q16 = Q16::from_ratio(60, 100);
pub const FAILING_BELOW: Q16 = Q16::from_ratio(35, 100);
pub const DYING_DAYS: u32 = 21;

/// A plant's vigor target is health scaled by 4/3, so the foliage
/// is thriving from health 0.45, where the terrarium turns unhealthy, and a repository
/// with no light or soil tool (health 0.5, I-3) grows thriving plants. Plants fail below health
/// 0.2625. It is the ratio of the two thresholds as stored, which is 4/3 to within 2⁻¹⁴, so that
/// health 0.45 reaches vigor 0.60 exactly rather than one step short.
pub const VIGOR_PER_HEALTH: Q16 = Q16::from_ratio(THRIVING_FROM.raw(), UNHEALTHY_BELOW.raw());

/// Tempo is the rain ratio held within `[1/4, 2]`: a silent repository still changes, slowly,
/// and a burst cannot rush it. Each day vigor moves `tempo × GROWTH_RATE` of the way to its
/// target, and that share of the free capacity germinates, rounded up.
pub const TEMPO_MIN: Q16 = Q16::from_ratio(1, 4);
pub const TEMPO_MAX: Q16 = Q16::from_int(2);
pub const GROWTH_RATE: Q16 = Q16::from_ratio(1, 7);

/// Plant capacity: `clamp(6 + 4 × log2(1 + KLOC), 6, 24)`, rounded down.
pub const PLANTS_MIN: i32 = 6;
pub const PLANTS_MAX: i32 = 24;
pub const PLANTS_PER_DOUBLING: Q16 = Q16::from_int(4);
pub const LINES_PER_KLOC: i64 = 1_000;

/// Weed capacity: `clamp(P / 3, 2, 8)`.
pub const WEEDS_PER_PLANTS: u32 = 3;
pub const WEEDS_MIN: u32 = 2;
pub const WEEDS_MAX: u32 = 8;

/// Weeds germinate, one a day, while the soil is below 0.45, and wither, one a day, while it is
/// unknown or at least 0.55; in between they hold.
pub const WEEDS_BELOW: Q16 = Q16::from_ratio(45, 100);
pub const WEEDS_WITHER_FROM: Q16 = Q16::from_ratio(55, 100);

/// Days grown (at vigor 0.35 or more, while the repository is not inactive) at which a plant
/// turns young and mature, by layer: trees grow slowest.
pub const FAR_STAGE_DAYS: [u32; 2] = [14, 60];
pub const MID_STAGE_DAYS: [u32; 2] = [10, 30];
pub const NEAR_STAGE_DAYS: [u32; 2] = [7, 21];

/// A dead plant stands this long, then falls into litter and frees its slot.
pub const DEAD_STANDING_DAYS: u32 = 30;
/// Litter becomes fungus after 14 days; fungus is gone after 60 more.
pub const LITTER_DAYS: u32 = 14;
pub const FUNGUS_DAYS: u32 = 60;

/// The seed bank: each pool species starts with 4 seeds, never has fewer than 2, gains one a
/// day while a mature plant of it thrives, and holds at most 12.
pub const SEED_BANK_START: u32 = 4;
pub const SEED_BANK_FLOOR: u32 = 2;
pub const SEED_BANK_CAP: u32 = 12;

/// Resident capacity: `clamp(1 + log2(1 + KLOC), 1, 6)`, rounded down. The composer draws at
/// most six.
pub const RESIDENTS_MIN: i32 = 1;
pub const RESIDENTS_MAX: i32 = 6;

/// The seed picks five of the eight resident species.
pub const RESIDENT_POOL: u32 = 5;

/// The days a resident spends in each stage while the repository is not inactive: egg (the
/// woodlouse's brood, the earthworm's cocoon), juvenile, pupa (none where the species has no
/// pupa) and adult. Adults live about 3 to 12 months, and egg to adult takes 3 to 8 weeks: the
/// springtail and the firefly live shortest, the earthworm and the beetle longest.
pub const fn life_days(species: ResidentSpecies) -> [u32; 4] {
    match species {
        ResidentSpecies::Snail => [14, 35, 0, 240],
        ResidentSpecies::Ladybird => [5, 14, 7, 150],
        ResidentSpecies::Beetle => [7, 28, 14, 330],
        ResidentSpecies::Firefly => [14, 32, 10, 90],
        ResidentSpecies::Woodlouse => [14, 28, 0, 270],
        ResidentSpecies::Springtail => [7, 14, 0, 90],
        ResidentSpecies::Earthworm => [21, 35, 0, 360],
        ResidentSpecies::Harvestman => [14, 35, 0, 180],
    }
}

/// Energy reads in vigor's bands: high from 0.60, low below 0.35.
pub const ENERGY_HIGH_FROM: Q16 = THRIVING_FROM;
pub const ENERGY_LOW_BELOW: Q16 = FAILING_BELOW;

/// A resident whose energy stays low this long leaves: an egg or pupa fails.
pub const RETREAT_DAYS: u32 = 14;

/// An adult at high energy lays an egg every 14 days at the usual pace: its brood grows by the
/// day's tempo, and an egg is due at `BROOD_DAYS`.
pub const BROOD_DAYS: Q16 = Q16::from_int(14);

/// Arrivals bank the day's tempo while the plants feed well, up to `ARRIVAL_DAYS` for each
/// resident the terrarium holds; each arrival spends `ARRIVAL_DAYS`, so one comes every two
/// days at the usual pace.
pub const ARRIVAL_DAYS: Q16 = Q16::from_int(2);

/// A species lays or arrives only while it has fewer living members than this. The pool's
/// species at this cap always outnumber the most residents, so it never keeps a cast short.
pub const KIN_MAX: u32 = 2;

/// Plant food is the living plants' vigor summed over at least `⌈P / FOOD_SHARE⌉` of them: a
/// full terrarium feeds at its mean vigor, a sparse one less.
pub const FOOD_SHARE: u32 = 2;

/// What decay (dead plants, litter, fungus) feeds a decomposer: enough to stay, never enough
/// to breed.
pub const DETRITUS_FOOD: Q16 = Q16::HALF;

/// One monument for each full 365 days of age, at most 3. Each weathers by its own age:
/// lichened from 2 years, ancient from 5.
pub const MONUMENT_DAYS: u32 = 365;
pub const MONUMENTS_MAX: usize = 3;
pub const LICHENED_FROM_YEARS: u32 = 2;
pub const ANCIENT_FROM_YEARS: u32 = 5;

/// A commit that deletes a whole directory of at least 500 lines leaves a fossil (a sinkhole);
/// at most 5 are kept, the oldest leaving first.
pub const FOSSIL_MIN_LINES: u64 = 500;
pub const FOSSILS_MAX: usize = 5;

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
        assert_eq!(FAILING_BELOW.raw(), 22_938);
        assert_eq!(VIGOR_PER_HEALTH.raw(), 87_383);
        assert_eq!(
            UNHEALTHY_BELOW.saturating_mul(VIGOR_PER_HEALTH),
            THRIVING_FROM
        );
        let four_thirds = Q16::from_ratio(4, 3);
        assert!(VIGOR_PER_HEALTH.saturating_sub(four_thirds) <= Q16::from_raw(2));
        assert_eq!(TEMPO_MIN.raw(), 16_384);
        assert_eq!(TEMPO_MAX.raw(), 131_072);
        assert_eq!(GROWTH_RATE.raw(), 9_362);
        assert_eq!(PLANTS_PER_DOUBLING, Q16::from_int(4));
        assert_eq!(WEEDS_BELOW.raw(), 29_491);
        assert_eq!(WEEDS_WITHER_FROM.raw(), 36_045);
        assert_eq!(LIGHT_SHIFT_STEP.raw(), 1_311);
        assert_eq!(NEUTRAL, Q16::from_ratio(1, 2));
        assert_eq!(ENERGY_HIGH_FROM.raw(), 39_322);
        assert_eq!(ENERGY_LOW_BELOW.raw(), 22_938);
        assert_eq!(BROOD_DAYS, Q16::from_int(14));
        assert_eq!(ARRIVAL_DAYS, Q16::from_int(2));
        assert_eq!(DETRITUS_FOOD.raw(), 32_768);
    }

    #[test]
    fn residents_live_in_seasons() {
        let days: Vec<(&str, [u32; 4])> = ResidentSpecies::ALL
            .iter()
            .map(|&species| (species.name(), life_days(species)))
            .collect();
        assert_eq!(
            days,
            [
                ("Snail", [14, 35, 0, 240]),
                ("Ladybird", [5, 14, 7, 150]),
                ("Beetle", [7, 28, 14, 330]),
                ("Firefly", [14, 32, 10, 90]),
                ("Woodlouse", [14, 28, 0, 270]),
                ("Springtail", [7, 14, 0, 90]),
                ("Earthworm", [21, 35, 0, 360]),
                ("Harvestman", [14, 35, 0, 180]),
            ]
        );
        for &species in ResidentSpecies::ALL {
            let [egg, juvenile, pupa, adult] = life_days(species);
            // A pupa exactly where the art has one; 3 to 8 weeks to adult; 3 to 12 months adult.
            assert_eq!(pupa > 0, species.has_pupa(), "{}", species.name());
            assert!(
                (21..=56).contains(&(egg + juvenile + pupa)),
                "{}",
                species.name()
            );
            assert!((90..=365).contains(&adult), "{}", species.name());
        }
        // The kin cap never keeps a full cast from forming out of a five-species pool.
        assert!(KIN_MAX * RESIDENT_POOL >= u32::try_from(RESIDENTS_MAX).expect("positive"));
        assert_eq!(
            (
                RETREAT_DAYS,
                KIN_MAX,
                FOOD_SHARE,
                MONUMENT_DAYS,
                MONUMENTS_MAX
            ),
            (14, 2, 2, 365, 3)
        );
        assert_eq!((LICHENED_FROM_YEARS, ANCIENT_FROM_YEARS), (2, 5));
        assert_eq!((FOSSIL_MIN_LINES, FOSSILS_MAX), (500, 5));
    }
}
