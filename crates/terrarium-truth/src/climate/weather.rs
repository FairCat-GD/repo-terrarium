//! The weather: test outcomes from CI runs and local runs.
//!
//! Each source keeps its newest run. The source in effect is the first of current-branch CI,
//! default-branch CI and local results whose newest run could tell pass from fail; with none,
//! the sky is haze. A source whose newest run could not tell is passed over, so unreachable CI
//! falls back to the last local result. The current branch is HEAD's branch as the newest
//! observation of any kind recorded it.
//!
//! Only a source's own outcome flipping, while that source was and stays in effect, is a
//! transition, which events may answer. Any other change of what is in effect, a branch switch above all, is a
//! switch: the sky tweens and nothing fires.

use serde::{Deserialize, Serialize};

use crate::canon::{Canonical, DumpError, Reader, Writer, canonical_struct};
use crate::domain::{Reading, WeatherReading, WeatherSource};
use crate::fixed::Q16;
use crate::time::Tick;

/// One source's runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceWeather {
    pub source: WeatherSource,
    /// The newest run: `Some(true)` passed, `Some(false)` failed, `None` could not tell.
    pub latest: Option<bool>,
    /// The newest run that could tell.
    pub known: Option<bool>,
    /// When `known` took its value.
    pub since: Tick,
    /// When `known` last flipped between pass and fail.
    pub flipped: Option<Tick>,
}

canonical_struct!(SourceWeather {
    source,
    latest,
    known,
    since,
    flipped
});

/// The source in effect and its outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InEffect {
    pub source: WeatherSource,
    pub pass: bool,
}

canonical_struct!(InEffect { source, pass });

/// A change of the weather in effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeatherChange {
    /// The source in effect flipped its own outcome.
    Transition { source: WeatherSource, pass: bool },
    /// Something else changed what is in effect; `to` is `None` for haze.
    Switch { to: Option<WeatherSource> },
}

impl Canonical for WeatherChange {
    fn write(&self, w: &mut Writer) {
        match self {
            WeatherChange::Transition { source, pass } => {
                w.leaf("Transition");
                w.field("source", source);
                w.field("pass", pass);
            }
            WeatherChange::Switch { to } => {
                w.leaf("Switch");
                w.field("to", to);
            }
        }
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        match r.leaf()? {
            "Transition" => Ok(WeatherChange::Transition {
                source: r.field("source")?,
                pass: r.field("pass")?,
            }),
            "Switch" => Ok(WeatherChange::Switch { to: r.field("to")? }),
            other => Err(r.bad_value(other)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeAt {
    pub tick: Tick,
    pub change: WeatherChange,
}

canonical_struct!(ChangeAt { tick, change });

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherState {
    /// The sources the weather can be chosen from, ordered by source: the local tests, and the
    /// CI of HEAD's branch and of the default one. Another branch's CI is let go of at the next
    /// evaluation, so the list stays three long however many branches HEAD visits. A list
    /// rather than a map: a branch name may hold characters a map key in the canonical dump
    /// cannot.
    pub sources: Vec<SourceWeather>,
    pub current_branch: Option<String>,
    /// When the observation that named the current branch was recorded.
    pub branch_recorded_at: Option<i64>,
    pub in_effect: Option<InEffect>,
    /// The newest change of what is in effect.
    pub last_change: Option<ChangeAt>,
}

canonical_struct!(WeatherState {
    sources,
    current_branch,
    branch_recorded_at,
    in_effect,
    last_change
});

impl WeatherState {
    /// Notes where HEAD was when an observation was recorded, unless a newer one has.
    pub fn note_head(&mut self, branch: Option<&str>, recorded_at: i64) {
        if self
            .branch_recorded_at
            .is_some_and(|newest| newest > recorded_at)
        {
            return;
        }
        self.current_branch = branch.map(str::to_owned);
        self.branch_recorded_at = Some(recorded_at);
    }

    /// Records a run of `source` at `tick`.
    pub fn record(&mut self, source: &WeatherSource, pass: Option<bool>, tick: Tick) {
        let index = match self.sources.binary_search_by(|s| s.source.cmp(source)) {
            Ok(index) => index,
            Err(index) => {
                self.sources.insert(
                    index,
                    SourceWeather {
                        source: source.clone(),
                        latest: None,
                        known: None,
                        since: tick,
                        flipped: None,
                    },
                );
                index
            }
        };
        let Some(entry) = self.sources.get_mut(index) else {
            return;
        };
        entry.latest = pass;
        if let Some(pass) = pass
            && entry.known != Some(pass)
        {
            if entry.known.is_some() {
                entry.flipped = Some(tick);
            }
            entry.known = Some(pass);
            entry.since = tick;
        }
    }

    fn source(&self, source: &WeatherSource) -> Option<&SourceWeather> {
        self.sources
            .binary_search_by(|s| s.source.cmp(source))
            .ok()
            .and_then(|index| self.sources.get(index))
    }

    /// Lets go of every source [`WeatherState::choose`] cannot read, chooses the source in
    /// effect at `tick` and records how it changed. A branch HEAD comes back to starts from its
    /// next run, as one never seen.
    pub fn evaluate(&mut self, tick: Tick, default_branch: &str) {
        let current = self.current_branch.as_deref();
        self.sources.retain(|kept| match &kept.source {
            WeatherSource::Local => true,
            WeatherSource::Ci { branch } => branch == default_branch || Some(&**branch) == current,
        });
        let chosen = self.choose(default_branch);
        if chosen == self.in_effect {
            return;
        }
        let same_source = |now: &InEffect| {
            self.in_effect
                .as_ref()
                .is_some_and(|before| before.source == now.source)
        };
        let change = match &chosen {
            Some(now)
                if same_source(now)
                    && self.source(&now.source).and_then(|s| s.flipped) == Some(tick) =>
            {
                WeatherChange::Transition {
                    source: now.source.clone(),
                    pass: now.pass,
                }
            }
            _ => WeatherChange::Switch {
                to: chosen.as_ref().map(|now| now.source.clone()),
            },
        };
        self.in_effect = chosen;
        self.last_change = Some(ChangeAt { tick, change });
    }

    fn choose(&self, default_branch: &str) -> Option<InEffect> {
        let ci = |branch: &str| WeatherSource::Ci {
            branch: branch.to_owned(),
        };
        let current = self.current_branch.as_deref().map(ci);
        let precedence = [
            current,
            Some(ci(default_branch)),
            Some(WeatherSource::Local),
        ];
        precedence.into_iter().flatten().find_map(|source| {
            let pass = self.source(&source)?.latest?;
            Some(InEffect { source, pass })
        })
    }

    /// The weather in effect, as a reading: 1 for a pass and 0 for a failure, since the run
    /// that began the current outcome.
    pub fn reading(&self) -> WeatherReading {
        match &self.in_effect {
            Some(now) => WeatherReading {
                state: Reading::Known {
                    value: if now.pass { Q16::ONE } else { Q16::ZERO },
                    stale: false,
                    since_tick: self.source(&now.source).map_or(Tick::ZERO, |s| s.since),
                },
                source: Some(now.source.clone()),
            },
            None => WeatherReading {
                state: Reading::Unknown,
                source: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ci(branch: &str) -> WeatherSource {
        WeatherSource::Ci {
            branch: branch.to_owned(),
        }
    }

    fn at(hours: u32) -> Tick {
        Tick::new(hours)
    }

    #[test]
    fn a_branch_switch_is_never_a_transition() {
        let mut weather = WeatherState::default();
        weather.note_head(Some("main"), 0);
        weather.record(&ci("main"), Some(true), at(1));
        weather.evaluate(at(1), "main");
        weather.note_head(Some("feature/fern"), 0);
        weather.record(&ci("feature/fern"), Some(false), at(2));
        weather.evaluate(at(2), "main");
        assert_eq!(
            weather.last_change,
            Some(ChangeAt {
                tick: at(2),
                change: WeatherChange::Switch {
                    to: Some(ci("feature/fern"))
                }
            })
        );
        weather.note_head(Some("main"), 0);
        weather.evaluate(at(3), "main");
        assert_eq!(
            weather.last_change.clone().map(|c| c.change),
            Some(WeatherChange::Switch {
                to: Some(ci("main"))
            })
        );
        // Main's own run flipping while main is in effect is a transition.
        weather.record(&ci("main"), Some(false), at(4));
        weather.evaluate(at(4), "main");
        assert_eq!(
            weather.last_change.clone().map(|c| c.change),
            Some(WeatherChange::Transition {
                source: ci("main"),
                pass: false
            })
        );
    }

    #[test]
    fn a_flip_out_of_effect_is_not_a_transition_when_it_comes_into_effect() {
        let mut weather = WeatherState::default();
        weather.note_head(Some("feature/fern"), 0);
        weather.record(&ci("feature/fern"), Some(true), at(1));
        weather.record(&ci("main"), Some(true), at(1));
        weather.evaluate(at(1), "main");
        weather.record(&ci("main"), Some(false), at(2));
        weather.evaluate(at(2), "main");
        assert_eq!(weather.last_change.as_ref().map(|c| c.tick), Some(at(1)));
        weather.note_head(Some("main"), 0);
        weather.evaluate(at(3), "main");
        assert_eq!(
            weather.last_change.clone().map(|c| c.change),
            Some(WeatherChange::Switch {
                to: Some(ci("main"))
            })
        );
    }

    #[test]
    fn an_unknown_run_passes_the_source_over() {
        let mut weather = WeatherState::default();
        weather.note_head(Some("main"), 0);
        weather.record(&WeatherSource::Local, Some(true), at(1));
        weather.record(&ci("main"), Some(false), at(1));
        weather.evaluate(at(1), "main");
        assert_eq!(weather.reading().source, Some(ci("main")));
        weather.record(&ci("main"), None, at(2));
        weather.evaluate(at(2), "main");
        let reading = weather.reading();
        assert_eq!(reading.source, Some(WeatherSource::Local));
        assert_eq!(reading.state.known_value(), Some(Q16::ONE));
        // Failing again after the gap flips nothing: main's last known run failed too.
        weather.record(&ci("main"), Some(false), at(3));
        weather.evaluate(at(3), "main");
        assert_eq!(
            weather.last_change.clone().map(|c| c.change),
            Some(WeatherChange::Switch {
                to: Some(ci("main"))
            })
        );
        assert_eq!(
            weather.reading().state,
            Reading::Known {
                value: Q16::ZERO,
                stale: false,
                since_tick: at(1),
            }
        );
    }

    #[test]
    fn an_older_observation_does_not_move_the_current_branch() {
        let mut weather = WeatherState::default();
        weather.note_head(Some("feature/fern"), 10);
        weather.note_head(Some("main"), 5);
        assert_eq!(weather.current_branch.as_deref(), Some("feature/fern"));
        weather.note_head(None, 10);
        assert_eq!(weather.current_branch, None);
    }

    #[test]
    fn no_source_that_can_tell_is_haze() {
        let mut weather = WeatherState::default();
        assert_eq!(weather.reading().source, None);
        weather.record(&WeatherSource::Local, None, at(0));
        weather.evaluate(at(0), "main");
        assert_eq!(
            weather.reading(),
            WeatherReading {
                state: Reading::Unknown,
                source: None
            }
        );
        assert_eq!(weather.last_change, None);
    }

    #[test]
    fn only_the_three_sources_choose_reads_are_kept() {
        let mut weather = WeatherState::default();
        weather.record(&WeatherSource::Local, Some(true), at(1));
        for (hour, branch) in (2..).zip(["feature/fern", "experiment/moss", "fix/lichen"]) {
            weather.note_head(Some(branch), i64::from(hour));
            weather.record(&ci(branch), Some(false), at(hour));
            weather.record(&ci("main"), Some(true), at(hour));
            weather.evaluate(at(hour), "main");
        }
        let kept: Vec<&WeatherSource> = weather.sources.iter().map(|s| &s.source).collect();
        assert_eq!(
            kept,
            [&ci("fix/lichen"), &ci("main"), &WeatherSource::Local]
        );
        // Detached, only the default branch's and the local source remain.
        weather.note_head(None, 10);
        weather.evaluate(at(10), "main");
        let kept: Vec<&WeatherSource> = weather.sources.iter().map(|s| &s.source).collect();
        assert_eq!(kept, [&ci("main"), &WeatherSource::Local]);
    }

    #[test]
    fn returning_to_a_pruned_branch_is_a_switch_once_its_run_is_recorded() {
        let mut weather = WeatherState::default();
        weather.note_head(Some("feature/fern"), 0);
        weather.record(&ci("feature/fern"), Some(false), at(1));
        weather.record(&ci("main"), Some(true), at(1));
        weather.evaluate(at(1), "main");
        weather.note_head(Some("main"), 2);
        weather.evaluate(at(2), "main");
        // Back on the feature branch: its failure is gone with it, so main's pass is in effect
        // until the branch's next run.
        weather.note_head(Some("feature/fern"), 3);
        weather.evaluate(at(3), "main");
        assert_eq!(weather.reading().source, Some(ci("main")));
        // Its next run, a pass, comes into effect as a switch, never as a clearing.
        weather.record(&ci("feature/fern"), Some(true), at(4));
        weather.evaluate(at(4), "main");
        assert_eq!(
            weather.last_change,
            Some(ChangeAt {
                tick: at(4),
                change: WeatherChange::Switch {
                    to: Some(ci("feature/fern"))
                }
            })
        );
        assert_eq!(
            weather.reading().state,
            Reading::Known {
                value: Q16::ONE,
                stale: false,
                since_tick: at(4),
            }
        );
    }

    #[test]
    fn a_flip_in_the_tick_its_source_comes_into_effect_is_a_switch() {
        let mut weather = WeatherState::default();
        weather.note_head(Some("feature/fern"), 0);
        weather.record(&ci("feature/fern"), Some(true), at(1));
        weather.record(&ci("main"), Some(true), at(1));
        weather.evaluate(at(1), "main");
        // In one tick HEAD moves to main and main's run flips: main comes into effect.
        weather.note_head(Some("main"), 10);
        weather.record(&ci("main"), Some(false), at(2));
        weather.evaluate(at(2), "main");
        assert_eq!(
            weather.last_change.clone().map(|c| c.change),
            Some(WeatherChange::Switch {
                to: Some(ci("main"))
            })
        );
        // Main's next flip, while it stays in effect, is a transition.
        weather.record(&ci("main"), Some(true), at(3));
        weather.evaluate(at(3), "main");
        assert_eq!(
            weather.last_change.clone().map(|c| c.change),
            Some(WeatherChange::Transition {
                source: ci("main"),
                pass: true
            })
        );
    }
}
