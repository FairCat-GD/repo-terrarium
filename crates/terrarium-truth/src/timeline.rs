//! The presentation timeline: what the terrarium shows as a whole, evaluated every tick from
//! health, activity and the time since the last commit.
//!
//! Health has hysteresis: healthy from 0.55, unhealthy below 0.45, the previous class in
//! between, healthy at the start. So has activity: active from 0.35, quiet below 0.30, the
//! previous class in between (quiet after inactive), active at the start. The rules, in order of
//! precedence:
//! 1. inactive (30 days without a commit): Dormant if healthy, Dead if not;
//! 2. Recovering: some tick of the last 14 days was Struggling, Dormant or Dead, and health is
//!    at least 0.10 above its value 14 days ago;
//! 3. active: Thriving if healthy, Struggling if not;
//! 4. quiet: Mature if healthy, Struggling if not.
//!
//! Recovering reads only past ticks, so a cached state and a full replay agree. Its 14-day limit
//! follows from its window. When a spell ends because the window has passed, Recovering cannot
//! begin again while the state it fell back to holds: otherwise a slow recovery would flicker
//! between Recovering and Struggling for one hour every 14 days.

use serde::{Deserialize, Serialize};

use crate::canon::canonical_struct;
use crate::domain::{ActivityClass, HealthClass, PresentationState};
use crate::fixed::Q16;
use crate::time::{TICKS_PER_DAY, Tick};
use crate::tuning::{
    ACTIVE_FROM, CONDENSATION_DAYS, DUST_DAYS, HEALTHY_FROM, QUIET_BELOW, RECOVERY_GAIN,
    RECOVERY_WINDOW_TICKS, UNHEALTHY_BELOW,
};

/// A change of the presentation, or of either class behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub tick: Tick,
    pub state: PresentationState,
    pub health: HealthClass,
    pub activity: ActivityClass,
}

canonical_struct!(TimelineEntry {
    tick,
    state,
    health,
    activity
});

/// What one tick's evaluation reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Moment {
    pub tick: Tick,
    pub health: Q16,
    /// Health 14 days earlier; `None` before 14 days have passed.
    pub health_before: Option<Q16>,
    pub activity: Q16,
    pub inactive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationTimeline {
    /// Every change, oldest first.
    pub entries: Vec<TimelineEntry>,
    /// When the current state began.
    pub state_since: Tick,
    /// The newest tick that was Struggling, Dormant or Dead.
    pub last_bad: Option<Tick>,
    /// The state a lapsed Recovering spell fell back to, while it holds.
    pub recovery_barred: Option<PresentationState>,
}

canonical_struct!(PresentationTimeline {
    entries,
    state_since,
    last_bad,
    recovery_barred,
});

const fn is_bad(state: PresentationState) -> bool {
    matches!(
        state,
        PresentationState::Struggling | PresentationState::Dormant | PresentationState::Dead
    )
}

impl Default for PresentationTimeline {
    fn default() -> PresentationTimeline {
        PresentationTimeline {
            entries: Vec::new(),
            state_since: Tick::ZERO,
            last_bad: None,
            recovery_barred: None,
        }
    }
}

impl PresentationTimeline {
    pub fn current(&self) -> Option<&TimelineEntry> {
        self.entries.last()
    }

    /// The current state and when it began. Before the first tick, the terrarium starts
    /// thriving: the initial class is healthy and a new repository is active.
    pub fn presentation(&self) -> (PresentationState, Tick) {
        self.current()
            .map_or((PresentationState::Thriving, Tick::ZERO), |entry| {
                (entry.state, self.state_since)
            })
    }

    pub fn evaluate(&mut self, moment: &Moment) {
        let previous = self.current().copied();
        let health = if moment.health >= HEALTHY_FROM {
            HealthClass::Healthy
        } else if moment.health < UNHEALTHY_BELOW {
            HealthClass::Unhealthy
        } else {
            previous.map_or(HealthClass::Healthy, |entry| entry.health)
        };
        let activity = if moment.inactive {
            ActivityClass::Inactive
        } else if moment.activity >= ACTIVE_FROM {
            ActivityClass::Active
        } else if moment.activity < QUIET_BELOW {
            ActivityClass::Quiet
        } else {
            match previous.map(|entry| entry.activity) {
                Some(ActivityClass::Inactive | ActivityClass::Quiet) => ActivityClass::Quiet,
                Some(ActivityClass::Active) | None => ActivityClass::Active,
            }
        };
        let healthy = health == HealthClass::Healthy;
        let previous_state = previous.map(|entry| entry.state);
        let in_window = self
            .last_bad
            .is_some_and(|bad| moment.tick.hours_since(bad) <= RECOVERY_WINDOW_TICKS);
        let gained = moment
            .health_before
            .is_some_and(|before| moment.health >= before.saturating_add(RECOVERY_GAIN));
        let barred = self.recovery_barred.is_some() && self.recovery_barred == previous_state;
        let state = match activity {
            ActivityClass::Inactive if healthy => PresentationState::Dormant,
            ActivityClass::Inactive => PresentationState::Dead,
            _ if in_window && gained && !barred => PresentationState::Recovering,
            _ if !healthy => PresentationState::Struggling,
            ActivityClass::Active => PresentationState::Thriving,
            ActivityClass::Quiet => PresentationState::Mature,
        };
        if previous_state == Some(PresentationState::Recovering)
            && state != PresentationState::Recovering
            && !in_window
        {
            self.recovery_barred = Some(state);
        } else if self.recovery_barred.is_some_and(|barred| barred != state) {
            self.recovery_barred = None;
        }
        if is_bad(state) {
            self.last_bad = Some(moment.tick);
        }
        if previous_state != Some(state) {
            self.state_since = moment.tick;
        }
        let entry = TimelineEntry {
            tick: moment.tick,
            state,
            health,
            activity,
        };
        let changed = previous.is_none_or(|previous| {
            (previous.state, previous.health, previous.activity) != (state, health, activity)
        });
        if changed {
            self.entries.push(entry);
        }
    }

    /// Condensation on the glass: levels 1, 2 and 3 from the first dormant tick, after 30 days
    /// and after 90.
    pub fn condensation(&self, now: Tick) -> u8 {
        self.level(now, PresentationState::Dormant, &CONDENSATION_DAYS)
    }

    /// Dust on the glass: levels 1, 2 and 3 after 365, 540 and 730 days dead.
    pub fn dust(&self, now: Tick) -> u8 {
        self.level(now, PresentationState::Dead, &DUST_DAYS)
    }

    fn level(&self, now: Tick, state: PresentationState, days: &[u32; 3]) -> u8 {
        if self.current().map(|entry| entry.state) != Some(state) {
            return 0;
        }
        let spent = now.hours_since(self.state_since) / TICKS_PER_DAY;
        let reached = days.iter().filter(|&&day| spent >= day).count();
        u8::try_from(reached).unwrap_or(u8::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::INACTIVE_TICKS;

    fn q(percent: i32) -> Q16 {
        Q16::from_ratio(percent, 100)
    }

    fn moment(tick: u32, health: Q16, activity: Q16) -> Moment {
        Moment {
            tick: Tick::new(tick),
            health,
            health_before: None,
            activity,
            inactive: false,
        }
    }

    fn state(timeline: &PresentationTimeline) -> PresentationState {
        timeline.presentation().0
    }

    #[test]
    fn health_holds_its_class_inside_the_band() {
        let mut timeline = PresentationTimeline::default();
        timeline.evaluate(&moment(0, q(50), q(50)));
        assert_eq!(state(&timeline), PresentationState::Thriving);
        timeline.evaluate(&moment(1, q(44), q(50)));
        assert_eq!(state(&timeline), PresentationState::Struggling);
        timeline.evaluate(&moment(2, q(54), q(50)));
        assert_eq!(state(&timeline), PresentationState::Struggling);
        timeline.evaluate(&moment(3, q(55), q(50)));
        assert_eq!(state(&timeline), PresentationState::Thriving);
        timeline.evaluate(&moment(4, q(46), q(20)));
        assert_eq!(state(&timeline), PresentationState::Mature);
        assert_eq!(timeline.entries.len(), 4);
    }

    #[test]
    fn inactivity_is_dormant_when_healthy_and_dead_when_not() {
        let mut timeline = PresentationTimeline::default();
        let inactive = |tick, health| Moment {
            inactive: true,
            ..moment(tick, health, Q16::ZERO)
        };
        timeline.evaluate(&inactive(INACTIVE_TICKS, q(60)));
        assert_eq!(state(&timeline), PresentationState::Dormant);
        timeline.evaluate(&inactive(INACTIVE_TICKS + 1, q(40)));
        assert_eq!(state(&timeline), PresentationState::Dead);
        assert_eq!(timeline.state_since, Tick::new(INACTIVE_TICKS + 1));
    }

    /// Fourteen days of Struggling at health 0.30, then `health_now` for `hours`, evaluated
    /// hourly. Returns each change of state.
    fn recover(health_now: Q16, hours: u32) -> Vec<(u32, PresentationState)> {
        let mut timeline = PresentationTimeline::default();
        let low = q(30);
        for tick in 0..RECOVERY_WINDOW_TICKS {
            timeline.evaluate(&moment(tick, low, q(50)));
        }
        for tick in RECOVERY_WINDOW_TICKS..RECOVERY_WINDOW_TICKS + hours {
            timeline.evaluate(&Moment {
                health_before: Some(low),
                ..moment(tick, health_now, q(50))
            });
        }
        timeline
            .entries
            .iter()
            .map(|entry| (entry.tick.hours(), entry.state))
            .collect()
    }

    #[test]
    fn recovering_needs_a_gain_and_lasts_at_most_14_days() {
        let start = RECOVERY_WINDOW_TICKS;
        assert_eq!(
            recover(q(39), 1000),
            [(0, PresentationState::Struggling)],
            "a gain under 0.10 is not recovery"
        );
        // Healthy again: Recovering for 14 days after the last Struggling tick, then Thriving.
        assert_eq!(
            recover(q(60), 1000),
            [
                (0, PresentationState::Struggling),
                (start, PresentationState::Recovering),
                (
                    start - 1 + RECOVERY_WINDOW_TICKS + 1,
                    PresentationState::Thriving
                ),
            ]
        );
    }

    #[test]
    fn a_lapsed_recovery_does_not_flicker_back() {
        let start = RECOVERY_WINDOW_TICKS;
        // Still unhealthy after its 14 days: it falls back to Struggling and stays there.
        assert_eq!(
            recover(q(41), 2000),
            [
                (0, PresentationState::Struggling),
                (start, PresentationState::Recovering),
                (start + RECOVERY_WINDOW_TICKS, PresentationState::Struggling),
            ]
        );
    }

    #[test]
    fn the_bar_lifts_when_the_state_changes() {
        let mut timeline = PresentationTimeline {
            recovery_barred: Some(PresentationState::Struggling),
            ..PresentationTimeline::default()
        };
        timeline.evaluate(&moment(0, q(30), q(50)));
        assert_eq!(
            timeline.recovery_barred,
            Some(PresentationState::Struggling)
        );
        timeline.evaluate(&moment(1, q(60), q(50)));
        assert_eq!(state(&timeline), PresentationState::Thriving);
        assert_eq!(timeline.recovery_barred, None);
    }

    #[test]
    fn glass_levels_follow_days_in_the_state() {
        let mut timeline = PresentationTimeline::default();
        let day = TICKS_PER_DAY;
        timeline.evaluate(&Moment {
            inactive: true,
            ..moment(0, q(60), Q16::ZERO)
        });
        assert_eq!(timeline.condensation(Tick::ZERO), 1);
        assert_eq!(timeline.condensation(Tick::new(30 * day - 1)), 1);
        assert_eq!(timeline.condensation(Tick::new(30 * day)), 2);
        assert_eq!(timeline.condensation(Tick::new(90 * day)), 3);
        assert_eq!(timeline.dust(Tick::new(900 * day)), 0);
        timeline.evaluate(&Moment {
            inactive: true,
            ..moment(10, q(40), Q16::ZERO)
        });
        assert_eq!(timeline.condensation(Tick::new(10)), 0);
        assert_eq!(timeline.dust(Tick::new(10 + 365 * day - 1)), 0);
        assert_eq!(timeline.dust(Tick::new(10 + 365 * day)), 1);
        assert_eq!(timeline.dust(Tick::new(10 + 540 * day)), 2);
        assert_eq!(timeline.dust(Tick::new(10 + 730 * day)), 3);
    }

    #[test]
    fn activity_holds_its_class_between_its_thresholds() {
        use ActivityClass::{Active, Quiet};
        let mut timeline = PresentationTimeline::default();
        let healthy = q(80);
        let classes: Vec<ActivityClass> = [40, 32, 29, 32, 34, 35]
            .into_iter()
            .zip(0..)
            .map(|(activity, tick)| {
                timeline.evaluate(&moment(tick, healthy, q(activity)));
                timeline
                    .current()
                    .map(|entry| entry.activity)
                    .expect("evaluated")
            })
            .collect();
        assert_eq!(classes, [Active, Active, Quiet, Quiet, Quiet, Active]);
        // With no class before, in between is active; after inactive, quiet.
        let mut fresh = PresentationTimeline::default();
        fresh.evaluate(&moment(0, healthy, q(32)));
        assert_eq!(fresh.current().map(|e| e.activity), Some(Active));
        let mut waking = PresentationTimeline::default();
        waking.evaluate(&Moment {
            inactive: true,
            ..moment(0, healthy, q(32))
        });
        waking.evaluate(&moment(1, healthy, q(32)));
        assert_eq!(waking.current().map(|e| e.activity), Some(Quiet));
    }
}
