//! The climate: light and soil are health, rain and wind are tempo, weather is the test signal.
//!
//! The smoothed climates step once per UTC day, when the day closes. Each close leaves a
//! [`DailyClimate`] record, and the newest record is what health, activity and the presentation
//! timeline read until the next close.

pub mod readings;
pub mod rhythm;
pub mod weather;

use serde::{Deserialize, Serialize};

use crate::canon::{canonical_enum, canonical_struct};
use crate::domain::{Climate, Reading};
use crate::fixed::{Q16, Q32};
use crate::time::Tick;
use crate::tuning::{
    ACTIVITY_DIVISOR, ACTIVITY_RAIN_WEIGHT, ACTIVITY_WIND_WEIGHT, NEUTRAL, TERRAIN_TREND_PERCENT,
    TREND_STEP, TREND_TICKS,
};

/// The climate as one day's close left it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyClimate {
    /// When the record took effect: the first tick of the new UTC day.
    pub tick: Tick,
    /// Smoothed light and soil; `None` while unknown.
    pub light: Option<Q16>,
    pub soil: Option<Q16>,
    /// Soil's components as last reported.
    pub dead_code: Option<Q16>,
    pub complexity: Option<Q16>,
    pub todo_density: Option<Q16>,
    /// Rain and wind over their baselines: 1 is the repository's usual pace.
    pub rain: Q16,
    pub wind: Q16,
    pub health: Q16,
    pub activity: Q16,
    /// The default tip's text lines.
    pub terrain: u64,
}

canonical_struct!(DailyClimate {
    tick,
    light,
    soil,
    dead_code,
    complexity,
    todo_density,
    rain,
    wind,
    health,
    activity,
    terrain,
});

/// Health is the lower of light and soil, an unknown one counting as neutral (I-3).
pub fn health(light: Option<Q16>, soil: Option<Q16>) -> Q16 {
    light.unwrap_or(NEUTRAL).min(soil.unwrap_or(NEUTRAL))
}

/// Activity: `0.5 × (0.75 × rain + 0.25 × wind)`, clamped to `[0, 1]`, so a usual day is 0.5.
pub fn activity(rain: Q32, wind: Q32) -> Q16 {
    rain.saturating_mul(Q32::from_int(ACTIVITY_RAIN_WEIGHT))
        .saturating_add(wind.saturating_mul(Q32::from_int(ACTIVITY_WIND_WEIGHT)))
        .saturating_div(Q32::from_int(ACTIVITY_DIVISOR))
        .clamp01()
        .to_q16()
}

/// Health as it stood at `tick`: the newest record in effect then, or neutral before any.
pub fn health_at(daily: &[DailyClimate], tick: Tick) -> Q16 {
    daily
        .iter()
        .rev()
        .find(|record| record.tick <= tick)
        .map_or(NEUTRAL, |record| record.health)
}

/// Which way a reading has moved over the last week.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Trend {
    Up,
    Flat,
    Down,
}

canonical_enum!(Trend { Up, Flat, Down });

/// Each reading's trend; `None` where the reading, or its value a week before, is unknown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trends {
    pub light: Option<Trend>,
    pub soil: Option<Trend>,
    pub rain: Option<Trend>,
    pub wind: Option<Trend>,
    pub health: Option<Trend>,
    pub dead_code: Option<Trend>,
    pub complexity: Option<Trend>,
    pub todo_density: Option<Trend>,
    pub terrain: Option<Trend>,
}

canonical_struct!(Trends {
    light,
    soil,
    rain,
    wind,
    health,
    dead_code,
    complexity,
    todo_density,
    terrain,
});

impl Trends {
    /// The newest record against the newest one at least a week older.
    pub fn of(daily: &[DailyClimate]) -> Trends {
        let newest = daily.last();
        let older = newest.and_then(|newest| {
            let before = newest.tick.hours().checked_sub(TREND_TICKS)?;
            daily
                .iter()
                .rev()
                .find(|record| record.tick.hours() <= before)
        });
        let between =
            |field: fn(&DailyClimate) -> Option<Q16>| trend(field(newest?), field(older?));
        Trends {
            light: between(|r| r.light),
            soil: between(|r| r.soil),
            rain: between(|r| Some(r.rain)),
            wind: between(|r| Some(r.wind)),
            health: between(|r| Some(r.health)),
            dead_code: between(|r| r.dead_code),
            complexity: between(|r| r.complexity),
            todo_density: between(|r| r.todo_density),
            terrain: terrain_trend(newest.map(|r| r.terrain), older.map(|r| r.terrain)),
        }
    }
}

/// Terrain moves when the line count changed by at least [`TERRAIN_TREND_PERCENT`] of what it
/// was; from no lines, any lines are a rise.
fn terrain_trend(now: Option<u64>, before: Option<u64>) -> Option<Trend> {
    let (now, before) = (now?, before?);
    let percent = |lines: u64| u128::from(lines) * 100;
    let step = u128::from(before) * u128::from(TERRAIN_TREND_PERCENT);
    Some(if now == before {
        Trend::Flat
    } else if now > before && percent(now - before) >= step {
        Trend::Up
    } else if now < before && percent(before - now) >= step {
        Trend::Down
    } else {
        Trend::Flat
    })
}

fn trend(now: Option<Q16>, before: Option<Q16>) -> Option<Trend> {
    let change = now?.saturating_sub(before?);
    Some(if change >= TREND_STEP {
        Trend::Up
    } else if change <= Q16::ZERO.saturating_sub(TREND_STEP) {
        Trend::Down
    } else {
        Trend::Flat
    })
}

/// Soil's causes as last reported, unsmoothed, for the soil panel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoilCauses {
    pub dead_code: Reading,
    pub complexity: Reading,
    pub todo_density: Reading,
}

canonical_struct!(SoilCauses {
    dead_code,
    complexity,
    todo_density
});

/// The climate as the soil panel and the label read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClimateReport {
    pub climate: Climate,
    pub health: Q16,
    pub activity: Q16,
    /// Days until rain and wind are read against the repository's own baseline; `None` once
    /// they are.
    pub settling_days_left: Option<u32>,
    pub trends: Trends,
    pub causes: SoilCauses,
}

canonical_struct!(ClimateReport {
    climate,
    health,
    activity,
    settling_days_left,
    trends,
    causes,
});

#[cfg(test)]
mod tests {
    use super::*;

    fn record(tick: u32, light: Option<Q16>, health: Q16) -> DailyClimate {
        DailyClimate {
            tick: Tick::new(tick),
            light,
            soil: None,
            dead_code: None,
            complexity: None,
            todo_density: None,
            rain: Q16::ONE,
            wind: Q16::ONE,
            health,
            activity: Q16::HALF,
            terrain: 0,
        }
    }

    #[test]
    fn a_usual_day_is_exactly_half_active() {
        assert_eq!(activity(Q32::ONE, Q32::ONE), Q16::HALF);
        assert_eq!(activity(Q32::ZERO, Q32::ZERO), Q16::ZERO);
        assert_eq!(activity(Q32::from_int(4), Q32::ZERO), Q16::ONE);
        assert_eq!(activity(Q32::ONE, Q32::ZERO), Q16::from_ratio(3, 8));
    }

    #[test]
    fn unknown_health_inputs_count_as_neutral() {
        assert_eq!(health(None, None), NEUTRAL);
        let low = Q16::from_ratio(1, 5);
        assert_eq!(health(Some(low), None), low);
        assert_eq!(health(None, Some(Q16::ONE)), NEUTRAL);
    }

    #[test]
    fn health_at_reads_the_record_in_effect() {
        let daily = [
            record(24, None, Q16::from_ratio(1, 4)),
            record(48, None, Q16::from_ratio(3, 4)),
        ];
        assert_eq!(health_at(&daily, Tick::new(23)), NEUTRAL);
        assert_eq!(health_at(&daily, Tick::new(24)), Q16::from_ratio(1, 4));
        assert_eq!(health_at(&daily, Tick::new(47)), Q16::from_ratio(1, 4));
        assert_eq!(health_at(&daily, Tick::new(90)), Q16::from_ratio(3, 4));
    }

    #[test]
    fn a_trend_compares_with_a_week_before() {
        let step = TREND_STEP;
        let base = Q16::HALF;
        let daily = [
            record(24, Some(base), base),
            record(48, Some(base.saturating_add(step)), base),
            record(24 + 168, Some(base.saturating_add(step)), base),
        ];
        let trends = Trends::of(&daily);
        assert_eq!(trends.light, Some(Trend::Up));
        assert_eq!(trends.health, Some(Trend::Flat));
        assert_eq!(trends.soil, None);
        // Less than a week of records has no trend.
        assert_eq!(Trends::of(&daily[..2]).light, None);
        let falling = [
            record(24, Some(base), base),
            record(24 + 168, Some(base.saturating_sub(step)), base),
        ];
        assert_eq!(Trends::of(&falling).light, Some(Trend::Down));
        let small = [
            record(24, Some(base), base),
            record(24 + 168, Some(base.saturating_sub(Q16::EPSILON)), base),
        ];
        assert_eq!(Trends::of(&small).light, Some(Trend::Flat));
    }

    #[test]
    fn terrain_moves_by_two_percent_of_its_lines() {
        let week = |before: u64, now: u64| {
            let mut old = record(24, None, NEUTRAL);
            old.terrain = before;
            let mut new = record(24 + 168, None, NEUTRAL);
            new.terrain = now;
            Trends::of(&[old, new]).terrain
        };
        assert_eq!(week(1000, 1020), Some(Trend::Up));
        assert_eq!(week(1000, 1019), Some(Trend::Flat));
        assert_eq!(week(1000, 980), Some(Trend::Down));
        assert_eq!(week(1000, 981), Some(Trend::Flat));
        assert_eq!(week(0, 0), Some(Trend::Flat));
        assert_eq!(week(0, 1), Some(Trend::Up));
        assert_eq!(week(u64::MAX, 0), Some(Trend::Down));
        assert_eq!(week(0, u64::MAX), Some(Trend::Up));
        // Less than a week of records has no trend.
        assert_eq!(Trends::of(&[record(24, None, NEUTRAL)]).terrain, None);
    }
}
