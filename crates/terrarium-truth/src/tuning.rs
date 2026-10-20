//! The compiled tuning values.
//!
//! Every value here is part of truth's output, so changing one bumps `SIM_VERSION`. They are
//! constants rather than settings: if a user could edit them, two installs would disagree about
//! what happened to the same repository.

use crate::fixed::{Q16, Q32};
use crate::scene::{EventKind, ResidentSpecies};

pub const SECONDS_PER_DAY: i64 = 86_400;

/// The window every smoothed climate averages over, as an exponential moving average whose
/// weight is `2 / (14 + 1)`.
pub(crate) const SMOOTHING_ALPHA: Q32 = Q32::from_ratio(2, 15);

/// How many daily levels the rain and wind baselines take their median over.
pub(crate) const BASELINE_DAYS: usize = 90;

/// The lowest baseline rain and wind are compared with: `log2(1 + 1/7)`, one commit a week. A
/// test pins it to the logarithm it stands for.
pub(crate) const BASELINE_FLOOR: Q32 = Q32::from_raw(12_625 << 16);

/// History shorter than this (14 days) uses a neutral rain and wind ratio of 1, so activity is
/// a typical 0.5 while the baselines settle.
pub(crate) const SETTLING_TICKS: u32 = 14 * 24;

/// Activity is `0.5 × (0.75 × rain + 0.25 × wind)`, that is `(3 × rain + wind) / 8`.
pub(crate) const ACTIVITY_RAIN_WEIGHT: i64 = 3;
pub(crate) const ACTIVITY_WIND_WEIGHT: i64 = 1;
pub(crate) const ACTIVITY_DIVISOR: i64 = 8;

/// What an unknown light or soil counts as in health.
pub(crate) const NEUTRAL: Q16 = Q16::HALF;

/// Health from which the terrarium turns healthy, and below which it turns unhealthy; in
/// between, the previous class holds.
pub const HEALTHY_FROM: Q16 = Q16::from_ratio(55, 100);
pub(crate) const UNHEALTHY_BELOW: Q16 = Q16::from_ratio(45, 100);

/// Activity from which a repository that is not inactive counts as active, and below which it
/// counts as quiet; in between, the previous class holds, as health's does, so activity
/// hovering at the threshold never flickers the presentation.
pub const ACTIVE_FROM: Q16 = Q16::from_ratio(35, 100);
pub(crate) const QUIET_BELOW: Q16 = Q16::from_ratio(30, 100);

/// Thirty days without a commit on any branch is inactive.
pub(crate) const INACTIVE_TICKS: u32 = 30 * 24;

/// Recovering looks back 14 days, and needs health to have risen by 0.10 over them.
pub(crate) const RECOVERY_WINDOW_TICKS: u32 = 14 * 24;
pub(crate) const RECOVERY_GAIN: Q16 = Q16::from_ratio(10, 100);

/// The daily records kept: enough to reach 14 days back from anywhere in the newest day.
pub(crate) const DAILY_RECORDS: usize = 15;

/// A trend compares a reading with its value at least 7 days earlier; a change smaller than
/// 0.02 is flat.
pub(crate) const TREND_TICKS: u32 = 7 * 24;
pub(crate) const TREND_STEP: Q16 = Q16::from_ratio(2, 100);
/// Terrain's trend: a change of less than this percent of its lines over the week is flat.
pub(crate) const TERRAIN_TREND_PERCENT: u64 = 2;

/// Days in the dormant state at which condensation reaches levels 1, 2 and 3.
pub const CONDENSATION_DAYS: [u32; 3] = [0, 30, 90];
/// Days in the dead state at which dust reaches levels 1, 2 and 3.
pub const DUST_DAYS: [u32; 3] = [365, 540, 730];

/// The wind ratio from which the wind is breezy, and from which it is a gale. A typical day, at
/// ratio 1, is breezy.
pub const BREEZY_FROM: Q16 = Q16::from_ratio(7, 10);
pub const GALE_FROM: Q16 = Q16::from_int(3);

/// A coverage report that moved by at least 0.02 from the one before it (the sunburst and the
/// dusk) colours the sky for a week after it was recorded.
pub(crate) const LIGHT_SHIFT_STEP: Q16 = Q16::from_ratio(2, 100);
pub(crate) const LIGHT_SHIFT_TICKS: u32 = 7 * 24;

/// A weather reading at or above this is a pass.
pub(crate) const PASS_FROM: Q16 = Q16::HALF;

/// Vigor's bands: the foliage is thriving from 0.60; below 0.35 a plant is failing, and after
/// `DYING_DAYS` there it dies.
pub const THRIVING_FROM: Q16 = Q16::from_ratio(60, 100);
pub(crate) const FAILING_BELOW: Q16 = Q16::from_ratio(35, 100);
pub(crate) const DYING_DAYS: u32 = 21;

/// A plant's vigor target is health scaled by 4/3, so the foliage
/// is thriving from health 0.45, where the terrarium turns unhealthy, and a repository
/// with no light or soil tool (health 0.5, I-3) grows thriving plants. Plants fail below health
/// 0.2625. It is the ratio of the two thresholds as stored, which is 4/3 to within 2⁻¹⁴, so that
/// health 0.45 reaches vigor 0.60 exactly rather than one step short.
pub(crate) const VIGOR_PER_HEALTH: Q16 =
    Q16::from_ratio(THRIVING_FROM.raw(), UNHEALTHY_BELOW.raw());

/// Tempo is the rain ratio held within `[1/4, 2]`: a silent repository still changes, slowly,
/// and a burst cannot rush it. Each day vigor moves `tempo × GROWTH_RATE` of the way to its
/// target, and that share of the free capacity germinates, rounded up.
pub(crate) const TEMPO_MIN: Q16 = Q16::from_ratio(1, 4);
pub(crate) const TEMPO_MAX: Q16 = Q16::from_int(2);
pub(crate) const GROWTH_RATE: Q16 = Q16::from_ratio(1, 7);

/// Plant capacity: `clamp(6 + 4 × log2(1 + KLOC), 6, 24)`, rounded down.
pub(crate) const PLANTS_MIN: i32 = 6;
pub(crate) const PLANTS_MAX: i32 = 24;
pub(crate) const PLANTS_PER_DOUBLING: Q16 = Q16::from_int(4);
pub(crate) const LINES_PER_KLOC: i64 = 1_000;

/// Weed capacity: `clamp(P / 3, 2, 8)`.
pub(crate) const WEEDS_PER_PLANTS: u32 = 3;
pub(crate) const WEEDS_MIN: u32 = 2;
pub(crate) const WEEDS_MAX: u32 = 8;

/// Weeds germinate, one a day, while the soil is below 0.45, and wither, one a day, while it is
/// unknown or at least 0.55; in between they hold.
pub const WEEDS_BELOW: Q16 = Q16::from_ratio(45, 100);
pub(crate) const WEEDS_WITHER_FROM: Q16 = Q16::from_ratio(55, 100);

/// Days grown (at vigor 0.35 or more, while the repository is not inactive) at which a plant
/// turns young and mature, by layer: trees grow slowest.
pub(crate) const FAR_STAGE_DAYS: [u32; 2] = [14, 60];
pub(crate) const MID_STAGE_DAYS: [u32; 2] = [10, 30];
pub(crate) const NEAR_STAGE_DAYS: [u32; 2] = [7, 21];

/// A dead plant stands this long, then falls into litter and frees its slot.
pub(crate) const DEAD_STANDING_DAYS: u32 = 30;
/// Litter becomes fungus after 14 days; fungus is gone after 60 more.
pub(crate) const LITTER_DAYS: u32 = 14;
pub(crate) const FUNGUS_DAYS: u32 = 60;

/// The seed bank: each pool species starts with 4 seeds, never has fewer than 2, gains one a
/// day while a mature plant of it thrives, and holds at most 12.
pub(crate) const SEED_BANK_START: u32 = 4;
pub(crate) const SEED_BANK_FLOOR: u32 = 2;
pub(crate) const SEED_BANK_CAP: u32 = 12;

/// Resident capacity: `clamp(1 + log2(1 + KLOC), 1, 6)`, rounded down. The composer draws at
/// most six.
pub(crate) const RESIDENTS_MIN: i32 = 1;
pub(crate) const RESIDENTS_MAX: i32 = 6;

/// The seed picks five of the eight resident species.
pub(crate) const RESIDENT_POOL: u32 = 5;

/// The days a resident spends in each stage while the repository is not inactive: egg (the
/// woodlouse's brood, the earthworm's cocoon), juvenile, pupa (none where the species has no
/// pupa) and adult. Adults live about 3 to 12 months, and egg to adult takes 3 to 8 weeks: the
/// springtail and the firefly live shortest, the earthworm and the beetle longest.
pub(crate) const fn life_days(species: ResidentSpecies) -> [u32; 4] {
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
pub(crate) const ENERGY_HIGH_FROM: Q16 = THRIVING_FROM;
pub(crate) const ENERGY_LOW_BELOW: Q16 = FAILING_BELOW;

/// A resident whose energy stays low this long leaves: an egg or pupa fails.
pub(crate) const RETREAT_DAYS: u32 = 14;

/// An adult at high energy lays an egg every 14 days at the usual pace: its brood grows by the
/// day's tempo, and an egg is due at `BROOD_DAYS`.
pub(crate) const BROOD_DAYS: Q16 = Q16::from_int(14);

/// Arrivals bank the day's tempo while the plants feed well, up to `ARRIVAL_DAYS` for each
/// resident the terrarium holds; each arrival spends `ARRIVAL_DAYS`, so one comes every two
/// days at the usual pace.
pub(crate) const ARRIVAL_DAYS: Q16 = Q16::from_int(2);

/// A species lays or arrives only while it has fewer living members than this. The pool's
/// species at this cap always outnumber the most residents, so it never keeps a cast short.
pub(crate) const KIN_MAX: u32 = 2;

/// Plant food is the living plants' vigor summed over at least `⌈P / FOOD_SHARE⌉` of them: a
/// full terrarium feeds at its mean vigor, a sparse one less.
pub(crate) const FOOD_SHARE: u32 = 2;

/// What decay (dead plants, litter, fungus) feeds a decomposer: enough to stay, never enough
/// to breed.
pub(crate) const DETRITUS_FOOD: Q16 = Q16::HALF;

/// One monument for each full 365 days of age, at most 3. Each weathers by its own age:
/// lichened from 2 years, ancient from 5.
pub(crate) const MONUMENT_DAYS: u32 = 365;
pub(crate) const MONUMENTS_MAX: usize = 3;
pub(crate) const LICHENED_FROM_YEARS: u32 = 2;
pub(crate) const ANCIENT_FROM_YEARS: u32 = 5;

/// A commit that deletes a whole directory of at least 500 lines leaves a fossil (a sinkhole);
/// at most 5 are kept, the oldest leaving first.
pub(crate) const FOSSIL_MIN_LINES: u64 = 500;
pub(crate) const FOSSILS_MAX: usize = 5;

/// An event's rank: the lower wins arbitration.
pub(crate) const fn event_rank(kind: EventKind) -> u8 {
    match kind {
        EventKind::Earthquake => 1,
        EventKind::Sinkhole => 2,
        EventKind::Lightning => 3,
        EventKind::Flowering => 4,
        EventKind::ClearingBurn => 5,
        EventKind::Storm => 6,
        EventKind::RainAfterDrought => 7,
        EventKind::Windstorm => 8,
        EventKind::Sunburst | EventKind::Dusk => 9,
        EventKind::Clearing => 10,
    }
}

/// An event's cooldown in ticks, counted from the last time it qualified, whether it played or
/// not.
pub(crate) const fn event_cooldown(kind: EventKind) -> u32 {
    match kind {
        EventKind::Earthquake
        | EventKind::Sinkhole
        | EventKind::Lightning
        | EventKind::Flowering
        | EventKind::ClearingBurn => 24,
        EventKind::Storm => 2,
        EventKind::RainAfterDrought => 7 * 24,
        EventKind::Windstorm => 12,
        EventKind::Sunburst | EventKind::Dusk => 6,
        EventKind::Clearing => 1,
    }
}

/// How long truth remembers an event that could play: the longest absence whose strongest
/// event still plays on return.
pub(crate) const EVENT_RECALL_TICKS: u32 = 7 * 24;

/// Strength bands: the value from which an event is of middling strength, and from which it is
/// strong, in the measure each event has.
/// - earthquake: commits taken off the default chain;
pub(crate) const EARTHQUAKE_BANDS: [u32; 2] = [3, 10];
/// - sinkhole: lines in the deleted directory that names its fossil;
pub(crate) const SINKHOLE_BANDS: [u64; 2] = [2_000, 10_000];
/// - lightning: whole days the weather in effect has been failing;
pub(crate) const LIGHTNING_BANDS: [u32; 2] = [4, 7];
/// - clearing burn: the share of unused items cleared, in percent;
pub(crate) const CLEARING_BURN_BANDS: [u64; 2] = [40, 60];
/// - rain after drought: whole days without a commit;
pub(crate) const DROUGHT_BANDS: [u32; 2] = [30, 90];
/// - windstorm: the day's changed lines over the usual active day's;
pub(crate) const WINDSTORM_BANDS: [u64; 2] = [6, 12];
/// - sunburst and dusk: how far coverage moved.
pub(crate) const LIGHT_SHIFT_BANDS: [Q16; 2] = [Q16::from_ratio(5, 100), Q16::from_ratio(10, 100)];

/// Lightning strikes on the third whole day the weather in effect has failed, then on every
/// third day after it: a long failure is a few strikes, not one a day.
pub(crate) const LIGHTNING_FROM_DAYS: u32 = 3;
pub(crate) const LIGHTNING_EVERY_DAYS: u32 = 3;
/// A strike lowers its plant's vigor by this, never below `FAILING_BELOW`: lightning marks a
/// plant, it never kills one.
pub(crate) const LIGHTNING_DIP: Q16 = Q16::from_ratio(15, 100);

/// A flowering's bloom lasts a day.
pub(crate) const BLOOM_TICKS: u32 = 24;

/// A clearing burn needs at least this many unused items cleared, and this share of them, in
/// percent.
pub(crate) const CLEARING_BURN_ITEMS: u32 = 5;
pub(crate) const CLEARING_BURN_PERCENT: u64 = 20;

/// A silence this many days long is a drought. Rain after drought plays for the first commit
/// after one, counted in hours since any commit. The rain and wind baselines count it in each
/// tempo's own closed days without a sample: they leave out the days past it, and the first day
/// back restarts the average.
pub(crate) const DROUGHT_DAYS: u32 = 14;

/// A windstorm is a day whose changed lines are at least this many times the median of the
/// active days among the last `WINDSTORM_DAYS`, once `WINDSTORM_ACTIVE_DAYS` of them were
/// active.
pub(crate) const WINDSTORM_RATIO: u64 = 3;
pub(crate) const WINDSTORM_DAYS: usize = 90;
pub(crate) const WINDSTORM_ACTIVE_DAYS: usize = 7;

/// Ground marks: a settled crack or sinkhole stays until the next rain after drought or for 30
/// days; fresh soil for 14; at most 3 at once, the oldest leaving first.
pub(crate) const SETTLED_MARK_DAYS: u32 = 30;
pub(crate) const FRESH_SOIL_DAYS: u32 = 14;
pub(crate) const MARKS_MAX: usize = 3;

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

    #[test]
    fn events_follow_their_table() {
        let table: Vec<(&str, u8, u32)> = EventKind::ALL
            .iter()
            .map(|&kind| (kind.name(), event_rank(kind), event_cooldown(kind)))
            .collect();
        assert_eq!(
            table,
            [
                ("Storm", 6, 2),
                ("Clearing", 10, 1),
                ("Lightning", 3, 24),
                ("Dusk", 9, 6),
                ("Sunburst", 9, 6),
                ("ClearingBurn", 5, 24),
                ("Flowering", 4, 24),
                ("Windstorm", 8, 12),
                ("RainAfterDrought", 7, 168),
                ("Earthquake", 1, 24),
                ("Sinkhole", 2, 24),
            ]
        );
        assert_eq!(EVENT_RECALL_TICKS, 168);
        assert_eq!(
            (EARTHQUAKE_BANDS, LIGHTNING_BANDS, DROUGHT_BANDS),
            ([3, 10], [4, 7], [30, 90])
        );
        assert_eq!(
            (SINKHOLE_BANDS, CLEARING_BURN_BANDS, WINDSTORM_BANDS),
            ([2_000, 10_000], [40, 60], [6, 12])
        );
        assert_eq!(
            LIGHT_SHIFT_BANDS.map(Q16::raw),
            [Q16::from_ratio(5, 100).raw(), 6_554]
        );
        assert_eq!(
            (
                LIGHTNING_FROM_DAYS,
                LIGHTNING_EVERY_DAYS,
                LIGHTNING_DIP.raw()
            ),
            (3, 3, 9_830)
        );
        assert_eq!((BLOOM_TICKS, DROUGHT_DAYS), (24, 14));
        assert_eq!((CLEARING_BURN_ITEMS, CLEARING_BURN_PERCENT), (5, 20));
        assert_eq!(
            (WINDSTORM_RATIO, WINDSTORM_DAYS, WINDSTORM_ACTIVE_DAYS),
            (3, 90, 7)
        );
        assert_eq!((SETTLED_MARK_DAYS, FRESH_SOIL_DAYS, MARKS_MAX), (30, 14, 3));
        assert_eq!(QUIET_BELOW.raw(), 19_661);
    }
}
