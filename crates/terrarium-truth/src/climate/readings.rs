//! Light and soil: the repository's health, from the reports recorded while HEAD was at the
//! default tip.
//!
//! The newest applicable report of each kind is held as it was recorded, never decayed. Light
//! is coverage; soil is the mean of whichever of dead code, complexity and TODO density are
//! known. Each is smoothed with the 14-day EMA, once per UTC day. A report that could not tell
//! makes its reading unknown: the average is dropped, and the next known report starts it again,
//! so an unknown never drags a value toward zero (I-3).

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
    pub tick: Tick,
}

canonical_struct!(Latest { value, head, tick });

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
    /// Smoothed light and soil; `None` while unknown.
    pub light: Option<Q32>,
    pub soil: Option<Q32>,
}

canonical_struct!(Readings {
    coverage,
    dead_code,
    complexity,
    todo_density,
    light,
    soil,
});

impl Readings {
    /// Holds `latest` as the newest report of `kind`. Kinds that feed neither light nor soil
    /// are ignored.
    pub fn record(&mut self, kind: ObservationKind, latest: Latest) {
        let slot = match kind {
            ObservationKind::Coverage => &mut self.coverage,
            ObservationKind::DeadCode => &mut self.dead_code,
            ObservationKind::Complexity => &mut self.complexity,
            ObservationKind::TodoDensity => &mut self.todo_density,
            ObservationKind::Weather | ObservationKind::RefRewrite => return,
        };
        *slot = Some(latest);
    }

    /// Steps both smoothed values with the reports held at the close of a day.
    pub fn close(&mut self) {
        self.light = smooth(self.light, self.raw_light());
        self.soil = smooth(self.soil, self.raw_soil());
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

    /// Smoothed light, stale when its report is.
    pub fn light_reading(&self, tip: Option<&CommitId>) -> Reading {
        match (self.light, &self.coverage) {
            (Some(level), Some(latest)) => Reading::Known {
                value: level.to_q16(),
                stale: latest.stale(tip),
                since_tick: latest.tick,
            },
            _ => Reading::Unknown,
        }
    }

    /// Smoothed soil, stale when any known component is, as of its newest report.
    pub fn soil_reading(&self, tip: Option<&CommitId>) -> Reading {
        let Some(level) = self.soil else {
            return Reading::Unknown;
        };
        let components = self.components();
        let present = components.iter().flatten();
        Reading::Known {
            value: level.to_q16(),
            stale: present.clone().any(|c| c.value.is_some() && c.stale(tip)),
            since_tick: present.map(|c| c.tick).max().unwrap_or(Tick::ZERO),
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
}
