//! Light and soil: the repository's health, from the reports recorded while HEAD was at the
//! default tip.
//!
//! The newest applicable report of each kind is held as it was recorded, never decayed; a report
//! applied late never replaces a newer one. Light is coverage; soil is the mean of whichever of
//! dead code, complexity and TODO density are known. Each is smoothed with the 14-day EMA, once
//! per UTC day, and remembers the reports it last took, which date it and decide whether it is
//! stale. A report that could not tell makes its reading unknown at the next close: the average
//! is dropped, and the next known report starts it again, so an unknown never drags a value
//! toward zero (I-3).

use serde::{Deserialize, Serialize};

use crate::canon::canonical_struct;
use crate::domain::{ObservationKind, Reading};
use crate::fixed::{Q16, Q32};
use crate::identity::CommitId;
use crate::time::Tick;
use crate::tuning::SMOOTHING_ALPHA;

/// The newest applicable report of one kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Latest {
    /// What it reported; `None` when it could not tell.
    pub value: Option<Q16>,
    /// HEAD when it was recorded, which was the default tip.
    pub head: CommitId,
    /// The tick it was recorded in.
    pub tick: Tick,
    /// Unix seconds, UTC: which of two reports is newer.
    pub recorded_at: i64,
}

canonical_struct!(Latest {
    value,
    head,
    tick,
    recorded_at
});

impl Latest {
    /// Stale once the default tip has moved past the report's commit.
    fn stale(&self, tip: Option<&CommitId>) -> bool {
        tip != Some(&self.head)
    }

    /// The report itself, unsmoothed, as the soil panel's causes show it.
    pub fn reading(latest: Option<&Latest>, tip: Option<&CommitId>) -> Reading {
        match latest {
            Some(
                latest @ Latest {
                    value: Some(value), ..
                },
            ) => Reading::Known {
                value: *value,
                stale: latest.stale(tip),
                since_tick: latest.tick,
            },
            _ => Reading::Unknown,
        }
    }
}

/// The reports behind light and soil, and their smoothed values.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Readings {
    pub coverage: Option<Latest>,
    pub dead_code: Option<Latest>,
    pub complexity: Option<Latest>,
    pub todo_density: Option<Latest>,
    /// The value of the coverage report the newest one replaced: the light of a change.
    pub coverage_before: Option<Q16>,
    /// Smoothed light and soil; `None` while unknown.
    pub light: Option<Q32>,
    pub soil: Option<Q32>,
    /// The reports the last close smoothed in: the coverage report, and each known soil
    /// component's.
    pub light_from: Option<Latest>,
    pub soil_from: Vec<Latest>,
}

canonical_struct!(Readings {
    coverage,
    dead_code,
    complexity,
    todo_density,
    coverage_before,
    light,
    soil,
    light_from,
    soil_from,
});

impl Readings {
    /// Holds `latest` as the newest report of `kind`, unless a newer one is held, and says
    /// whether it took it. Kinds that feed neither light nor soil are ignored.
    pub fn record(&mut self, kind: ObservationKind, latest: Latest) -> bool {
        let slot = match kind {
            ObservationKind::Coverage => &mut self.coverage,
            ObservationKind::DeadCode => &mut self.dead_code,
            ObservationKind::Complexity => &mut self.complexity,
            ObservationKind::TodoDensity => &mut self.todo_density,
            ObservationKind::Weather | ObservationKind::RefRewrite => return false,
        };
        if slot
            .as_ref()
            .is_some_and(|held| held.recorded_at > latest.recorded_at)
        {
            return false;
        }
        let before = slot.replace(latest).and_then(|held| held.value);
        if kind == ObservationKind::Coverage {
            self.coverage_before = before;
        }
        true
    }

    /// Steps both smoothed values with the reports held at the close of a day.
    pub fn close(&mut self) {
        let light = self.raw_light();
        self.light = smooth(self.light, light);
        self.light_from = light.and(self.coverage.clone());
        self.soil = smooth(self.soil, self.raw_soil());
        self.soil_from = self
            .components()
            .into_iter()
            .flatten()
            .filter(|c| c.value.is_some())
            .cloned()
            .collect();
    }

    /// How the newest coverage report moved from the one before it, while that report is
    /// recent (recorded no earlier than `since`).
    pub fn coverage_change(&self, since: Tick) -> Option<Q16> {
        let latest = self.coverage.as_ref().filter(|c| c.tick >= since)?;
        Some(latest.value?.saturating_sub(self.coverage_before?))
    }

    pub fn raw_light(&self) -> Option<Q16> {
        self.coverage.as_ref().and_then(|latest| latest.value)
    }

    /// The mean of the soil components that are known, or `None` if none is.
    pub fn raw_soil(&self) -> Option<Q16> {
        let mut sum = Q16::ZERO;
        let mut known = 0;
        for value in self
            .components()
            .into_iter()
            .flatten()
            .filter_map(|c| c.value)
        {
            sum = sum.saturating_add(value);
            known += 1;
        }
        (known > 0).then(|| sum.saturating_div(Q16::from_int(known)))
    }

    fn components(&self) -> [Option<&Latest>; 3] {
        [
            self.dead_code.as_ref(),
            self.complexity.as_ref(),
            self.todo_density.as_ref(),
        ]
    }

    /// Smoothed light, dated and judged stale by the report it last took.
    pub fn light_reading(&self, tip: Option<&CommitId>) -> Reading {
        match (self.light, &self.light_from) {
            (Some(level), Some(latest)) => Reading::Known {
                value: level.to_q16(),
                stale: latest.stale(tip),
                since_tick: latest.tick,
            },
            _ => Reading::Unknown,
        }
    }

    /// Smoothed soil, stale when any component it last took is, as of the newest of them.
    pub fn soil_reading(&self, tip: Option<&CommitId>) -> Reading {
        let (Some(level), Some(since_tick)) =
            (self.soil, self.soil_from.iter().map(|c| c.tick).max())
        else {
            return Reading::Unknown;
        };
        Reading::Known {
            value: level.to_q16(),
            stale: self.soil_from.iter().any(|c| c.stale(tip)),
            since_tick,
        }
    }
}

/// One step of the EMA, started by the first known sample and dropped by an unknown one.
fn smooth(level: Option<Q32>, sample: Option<Q16>) -> Option<Q32> {
    let sample = sample?.to_q32();
    Some(level.map_or(sample, |level| level.ema(sample, SMOOTHING_ALPHA)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(digit: char) -> CommitId {
        CommitId::from_hex(&digit.to_string().repeat(40)).expect("an id")
    }

    fn latest(value: Option<Q16>, digit: char, tick: u32) -> Latest {
        Latest {
            value,
            head: head(digit),
            tick: Tick::new(tick),
            recorded_at: i64::from(tick),
        }
    }

    #[test]
    fn a_steady_reading_settles_exactly_on_its_value() {
        // In Q16 the average would stop a few steps short of 0.55 and never read as healthy.
        let mut readings = Readings::default();
        readings.record(
            ObservationKind::Coverage,
            latest(Some(Q16::from_ratio(30, 100)), 'a', 0),
        );
        readings.close();
        readings.record(
            ObservationKind::Coverage,
            latest(Some(Q16::from_ratio(55, 100)), 'a', 1),
        );
        for _ in 0..400 {
            readings.close();
        }
        assert_eq!(
            readings.light_reading(Some(&head('a'))).known_value(),
            Some(Q16::from_ratio(55, 100))
        );
    }

    #[test]
    fn an_unknown_report_makes_the_reading_unknown_until_a_known_one() {
        let mut readings = Readings::default();
        readings.record(ObservationKind::Coverage, latest(Some(Q16::ONE), 'a', 0));
        readings.close();
        assert_eq!(readings.light_reading(None).known_value(), Some(Q16::ONE));
        readings.record(ObservationKind::Coverage, latest(None, 'a', 1));
        readings.close();
        assert_eq!(readings.light_reading(None), Reading::Unknown);
        readings.record(ObservationKind::Coverage, latest(Some(Q16::HALF), 'a', 2));
        readings.close();
        // The average starts again from the new value, not from the old one or from zero.
        assert_eq!(readings.light_reading(None).known_value(), Some(Q16::HALF));
    }

    #[test]
    fn soil_is_the_mean_of_the_known_components() {
        let mut readings = Readings::default();
        assert_eq!(readings.raw_soil(), None);
        readings.record(ObservationKind::DeadCode, latest(Some(Q16::ONE), 'a', 3));
        readings.record(ObservationKind::Complexity, latest(None, 'a', 4));
        readings.record(
            ObservationKind::TodoDensity,
            latest(Some(Q16::HALF), 'b', 5),
        );
        assert_eq!(readings.raw_soil(), Some(Q16::from_ratio(3, 4)));
        readings.close();
        let reading = readings.soil_reading(Some(&head('a')));
        assert_eq!(
            reading,
            Reading::Known {
                value: Q16::from_ratio(3, 4),
                // TODO density was recorded at another commit.
                stale: true,
                since_tick: Tick::new(5),
            }
        );
        assert_eq!(
            Latest::reading(readings.complexity.as_ref(), None),
            Reading::Unknown
        );
    }

    #[test]
    fn a_reading_is_dated_by_the_report_it_last_took() {
        let mut readings = Readings::default();
        readings.record(ObservationKind::Coverage, latest(Some(Q16::ONE), 'a', 3));
        readings.record(ObservationKind::DeadCode, latest(Some(Q16::ONE), 'a', 3));
        readings.close();
        // Until the next close, newer reports change neither the value nor its date.
        readings.record(ObservationKind::Coverage, latest(None, 'b', 9));
        readings.record(ObservationKind::Complexity, latest(None, 'b', 9));
        let known = |since| Reading::Known {
            value: Q16::ONE,
            stale: false,
            since_tick: Tick::new(since),
        };
        assert_eq!(readings.light_reading(Some(&head('a'))), known(3));
        assert_eq!(readings.soil_reading(Some(&head('a'))), known(3));
        readings.close();
        assert_eq!(readings.light_reading(Some(&head('a'))), Reading::Unknown);
        // The unknown complexity report does not date soil.
        assert_eq!(readings.soil_reading(Some(&head('a'))), known(3));
    }

    #[test]
    fn an_older_report_never_replaces_a_newer_one() {
        let mut readings = Readings::default();
        readings.record(ObservationKind::Coverage, latest(Some(Q16::HALF), 'a', 5));
        readings.record(ObservationKind::Coverage, latest(Some(Q16::ONE), 'b', 4));
        assert_eq!(readings.raw_light(), Some(Q16::HALF));
        readings.record(ObservationKind::Coverage, latest(Some(Q16::ONE), 'c', 6));
        assert_eq!(readings.coverage_change(Tick::new(6)), Some(Q16::HALF));
        assert_eq!(readings.coverage_change(Tick::new(7)), None);
    }
}
