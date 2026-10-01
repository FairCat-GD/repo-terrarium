//! The sky's own rules: the moon of the local date, the clouds a weather brings, which bodies a
//! weather lets through, and how the clouds drift.
//!
//! The weather band is the top 13 rows, tiled from x = 0; below it the sky is flat `sky.base`.
//! The sun, the moon and the stars stand inside the band's rows and the clouds drift below
//! them, so no two kinds of sky piece ever overlap, and the depth order's y-then-x rule is the
//! right one within the sky layer too.

use terrarium_truth::scene::Weather;

/// The rows the weather band covers, from the top.
pub const BAND_HEIGHT: i32 = 13;
/// The rows the sun, the moon and the stars may use: inside the band, above the clouds.
pub const BODY_ROWS: std::ops::RangeInclusive<i32> = 1..=11;
/// The top row of the highest cloud path, just below the band. Each further path is
/// [`CLOUD_PATH_STEP`] rows lower, so no two paths share a top row.
pub const CLOUD_TOP: i32 = BAND_HEIGHT + 1;
pub const CLOUD_PATH_STEP: i32 = 4;

/// A calendar date on the local clock. Day and night, and the moon, are the composer's, never
/// truth's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl LocalDate {
    pub const fn new(year: i32, month: u8, day: u8) -> LocalDate {
        LocalDate { year, month, day }
    }

    /// Days since 1970-01-01 in the proleptic Gregorian calendar (Howard Hinnant's
    /// `days_from_civil`).
    pub fn days_since_epoch(self) -> i64 {
        let (month, day) = (i64::from(self.month), i64::from(self.day));
        let year = i64::from(self.year) - i64::from(month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year.rem_euclid(400);
        let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }
}

/// The eight phases of the moon, each centred on its principal moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MoonPhase {
    New,
    WaxingCrescent,
    FirstQuarter,
    WaxingGibbous,
    Full,
    WaningGibbous,
    LastQuarter,
    WaningCrescent,
}

impl MoonPhase {
    const ALL: [MoonPhase; 8] = [
        MoonPhase::New,
        MoonPhase::WaxingCrescent,
        MoonPhase::FirstQuarter,
        MoonPhase::WaxingGibbous,
        MoonPhase::Full,
        MoonPhase::WaningGibbous,
        MoonPhase::LastQuarter,
        MoonPhase::WaningCrescent,
    ];

    /// How the four delivered moons show this phase; `None` at new moon, which shows nothing.
    /// The gibbous phases show the full disc, and the waning crescent is the waxing one
    /// mirrored, since the art has one crescent, lit on the right.
    pub const fn look(self) -> Option<MoonLook> {
        let (sprite, mirrored) = match self {
            MoonPhase::New => return None,
            MoonPhase::WaxingCrescent => (3, false),
            MoonPhase::FirstQuarter => (2, false),
            MoonPhase::WaxingGibbous | MoonPhase::Full | MoonPhase::WaningGibbous => (1, false),
            MoonPhase::LastQuarter => (4, false),
            MoonPhase::WaningCrescent => (3, true),
        };
        Some(MoonLook { sprite, mirrored })
    }
}

/// A moon as drawn: which of `tmp.sky.moon.phase.1` to `.4`, and whether it is mirrored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MoonLook {
    /// 1 to 4, as the art numbers them.
    pub sprite: u8,
    pub mirrored: bool,
}

const DAY_MS: i64 = 86_400_000;
/// The mean synodic month, 29.530588853 days.
const SYNODIC_MS: i64 = 2_551_442_877;
/// A new moon to count from: 2000-01-06 18:14 UTC.
const NEW_MOON_MS: i64 = 10_962 * DAY_MS + (18 * 60 + 14) * 60_000;

/// The phase on `date`, at local noon, from the mean synodic month in integers. The mean phase
/// strays from the true one by up to about 14 hours, a small part of an octant's 3.7 days.
pub fn moon_phase(date: LocalDate) -> MoonPhase {
    let noon = date.days_since_epoch() * DAY_MS + DAY_MS / 2;
    let age = (noon - NEW_MOON_MS).rem_euclid(SYNODIC_MS);
    let octant = (age * 8 + SYNODIC_MS / 2) / SYNODIC_MS % 8;
    MoonPhase::ALL[usize::try_from(octant).unwrap_or(0)]
}

/// The cloud sizes the art draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CloudSize {
    Small,
    Medium,
    Large,
}

/// The most clouds any weather brings.
pub const MAX_CLOUDS: usize = 4;

/// The clouds a weather brings, by place: cloud `i` of every weather drifts along the same
/// path, so a change of weather restyles a cloud where it stands. A storm's sky is its band.
pub const fn clouds(weather: Weather) -> &'static [CloudSize] {
    use CloudSize::{Large, Medium, Small};
    match weather {
        Weather::Clear => &[Small, Medium, Small],
        Weather::Haze => &[Large, Medium],
        Weather::Overcast => &[Medium, Large, Small, Large],
        Weather::Storm => &[],
    }
}

/// Whether the weather lets the sun and the moon through: not under overcast or storm.
pub const fn shows_sun_and_moon(weather: Weather) -> bool {
    matches!(weather, Weather::Clear | Weather::Haze)
}

/// Whether the weather lets the stars through: only a clear sky.
pub const fn shows_stars(weather: Weather) -> bool {
    matches!(weather, Weather::Clear)
}

/// The far left a cloud reaches before it wraps: wider than the widest cloud, so a cloud
/// wraps only while it is wholly outside the frame.
const CLOUD_MARGIN: i32 = 32;
/// The positions a cloud's left edge cycles through: `-CLOUD_MARGIN..=640`.
const CLOUD_SPAN: i64 = 640 + 32 + 1;

/// One cloud's path: where it starts, its row, and how long it takes to move a pixel right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drift {
    pub start: i32,
    pub y: i32,
    pub ms_per_px: u32,
}

impl Drift {
    /// The cloud's left edge at `now`.
    pub fn x(self, now: crate::tween::Millis) -> i32 {
        let moved = i64::try_from(now.0 / u64::from(self.ms_per_px.max(1))).unwrap_or(0);
        let at = (i64::from(self.start) + moved).rem_euclid(CLOUD_SPAN);
        i32::try_from(at).unwrap_or(0) - CLOUD_MARGIN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tween::Millis;

    #[test]
    fn days_count_from_the_epoch_both_ways() {
        assert_eq!(LocalDate::new(1970, 1, 1).days_since_epoch(), 0);
        assert_eq!(LocalDate::new(2000, 1, 1).days_since_epoch(), 10_957);
        assert_eq!(LocalDate::new(2000, 3, 1).days_since_epoch(), 11_017);
        assert_eq!(LocalDate::new(1969, 12, 31).days_since_epoch(), -1);
        assert_eq!(LocalDate::new(2024, 2, 29).days_since_epoch(), 19_782);
    }

    #[test]
    fn the_moon_matches_known_phases() {
        // Eclipses fall on exact new and full moons; the quarters are from almanacs.
        for (date, phase) in [
            (LocalDate::new(2024, 4, 8), MoonPhase::New),
            (LocalDate::new(2024, 4, 15), MoonPhase::FirstQuarter),
            (LocalDate::new(2024, 4, 23), MoonPhase::Full),
            (LocalDate::new(2024, 5, 1), MoonPhase::LastQuarter),
            (LocalDate::new(2025, 3, 14), MoonPhase::Full),
            (LocalDate::new(2025, 3, 29), MoonPhase::New),
            (LocalDate::new(2026, 3, 3), MoonPhase::Full),
            (LocalDate::new(2026, 3, 19), MoonPhase::New),
        ] {
            assert_eq!(moon_phase(date), phase, "{date:?}");
        }
    }

    #[test]
    fn a_month_of_days_walks_every_phase_in_order() {
        // From one new moon (2026-03-19) to the next, a day at a time.
        let days = (19..=31)
            .map(|day| LocalDate::new(2026, 3, day))
            .chain((1..=18).map(|day| LocalDate::new(2026, 4, day)));
        let mut seen: Vec<MoonPhase> = Vec::new();
        for date in days {
            let phase = moon_phase(date);
            if seen.last() != Some(&phase) {
                seen.push(phase);
            }
        }
        let mut expected = MoonPhase::ALL.to_vec();
        expected.push(MoonPhase::New);
        assert_eq!(seen, expected);
    }

    #[test]
    fn the_four_moons_show_eight_phases() {
        let looks: Vec<Option<MoonLook>> = MoonPhase::ALL.iter().map(|p| p.look()).collect();
        assert_eq!(looks[0], None);
        assert_eq!(
            MoonPhase::WaningCrescent.look(),
            Some(MoonLook {
                sprite: 3,
                mirrored: true
            })
        );
        assert_eq!(MoonPhase::WaxingGibbous.look(), MoonPhase::Full.look());
        let sprites: std::collections::BTreeSet<u8> =
            looks.iter().flatten().map(|look| look.sprite).collect();
        assert_eq!(sprites, [1, 2, 3, 4].into());
    }

    #[test]
    fn a_cloud_wraps_only_wholly_outside_the_frame() {
        let drift = Drift {
            start: 0,
            y: 20,
            ms_per_px: 3000,
        };
        let mut last = drift.x(Millis(0));
        assert_eq!(last, -CLOUD_MARGIN);
        for step in 1..=2 * CLOUD_SPAN {
            let x = drift.x(Millis(u64::try_from(step).unwrap() * 3000));
            if x < last {
                // The jump: from past the right edge to wholly left of the frame.
                assert!(last >= 640 && x + 30 <= 0, "{last} → {x}");
            } else {
                assert_eq!(x, last + 1);
            }
            last = x;
        }
        assert_eq!(drift.x(Millis(2999)), -CLOUD_MARGIN);
    }

    #[test]
    fn weather_decides_which_bodies_and_clouds_show() {
        assert!(shows_sun_and_moon(Weather::Haze) && !shows_stars(Weather::Haze));
        assert!(!shows_sun_and_moon(Weather::Overcast) && !shows_sun_and_moon(Weather::Storm));
        assert!(clouds(Weather::Storm).is_empty());
        assert!(Weather::ALL.iter().all(|&w| clouds(w).len() <= MAX_CLOUDS));
    }
}
