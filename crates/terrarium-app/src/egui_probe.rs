//! `--probe egui` (dev builds): what an egui window costs, and whether closing it gives the
//! cost back. The living strip runs; after it settles, the debug window opens and closes five
//! times on timers, through the same path as the menu. Each phase is sampled: the process's
//! memory, how often the loop woke, and which app had focus. The report is written to the
//! current directory and the strip exits.
//!
//! Run it on a release build (`cargo run --release -p terrarium-app --features dev -- --probe
//! egui`): a debug build's wgpu validation would swell the numbers.

use std::time::{Duration, Instant};

use terrarium_platform::memory::Memory;

/// How long the strip runs before the first sample, and how long each phase is held.
pub const SETTLE: Duration = Duration::from_secs(15);
pub const HOLD: Duration = Duration::from_secs(15);
pub const CYCLES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Before any window has opened.
    Baseline,
    /// The debug window open, in this cycle.
    Open(usize),
    /// The debug window closed again, in this cycle.
    Closed(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Sample(Phase),
    Open,
    Close,
    Finish,
}

/// The probe's steps, each with the wait before it.
pub fn steps() -> Vec<(Duration, Step)> {
    let mut steps = vec![(SETTLE, Step::Sample(Phase::Baseline))];
    for cycle in 1..=CYCLES {
        steps.extend([
            (Duration::ZERO, Step::Open),
            (HOLD, Step::Sample(Phase::Open(cycle))),
            (Duration::ZERO, Step::Close),
            (HOLD, Step::Sample(Phase::Closed(cycle))),
        ]);
    }
    steps.push((Duration::ZERO, Step::Finish));
    steps
}

/// One phase, measured.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub phase: Phase,
    /// Seconds since the probe started.
    pub at: u64,
    pub memory: Option<Memory>,
    /// How often the loop woke over the phase, per minute.
    pub wakes_per_minute: u64,
    pub app_active: Option<bool>,
    pub frontmost: Option<String>,
}

pub struct EguiProbe {
    steps: Vec<(Duration, Step)>,
    next: usize,
    started: Instant,
    due: Instant,
    /// The loop's wakes when the last sample was taken, and when.
    wakes: u64,
    wakes_at: Instant,
    samples: Vec<Sample>,
}

impl EguiProbe {
    pub fn start(now: Instant) -> EguiProbe {
        let steps = steps();
        let due = now + steps[0].0;
        EguiProbe {
            steps,
            next: 0,
            started: now,
            due,
            wakes: 0,
            wakes_at: now,
            samples: Vec::new(),
        }
    }

    /// When the next step falls due; `None` once the probe has finished.
    pub fn due(&self) -> Option<Instant> {
        (self.next < self.steps.len()).then_some(self.due)
    }

    /// The step due by `now`, if one is, and the next one scheduled after it.
    pub fn take(&mut self, now: Instant) -> Option<Step> {
        if self.due()? > now {
            return None;
        }
        let (_, step) = self.steps[self.next];
        self.next += 1;
        if let Some(&(wait, _)) = self.steps.get(self.next) {
            self.due = now + wait;
        }
        Some(step)
    }

    /// Records `phase` at `now`, the loop having woken `wakes` times since launch.
    pub fn sample(
        &mut self,
        phase: Phase,
        now: Instant,
        wakes: u64,
        memory: Option<Memory>,
        app_active: Option<bool>,
        frontmost: Option<String>,
    ) {
        let millis = now
            .saturating_duration_since(self.wakes_at)
            .as_millis()
            .max(1);
        let woke = u128::from(wakes.saturating_sub(self.wakes));
        self.samples.push(Sample {
            phase,
            at: now.saturating_duration_since(self.started).as_secs(),
            memory,
            wakes_per_minute: u64::try_from(woke * 60_000 / millis).unwrap_or(u64::MAX),
            app_active,
            frontmost,
        });
        self.wakes = wakes;
        self.wakes_at = now;
    }

    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_settles_then_opens_and_closes_five_times() {
        let steps = steps();
        assert_eq!(steps.len(), 1 + CYCLES * 4 + 1);
        assert_eq!(steps[0], (SETTLE, Step::Sample(Phase::Baseline)));
        let actions: Vec<Step> = steps
            .iter()
            .map(|&(_, step)| step)
            .filter(|step| matches!(step, Step::Open | Step::Close))
            .collect();
        assert_eq!(actions.len(), CYCLES * 2);
        assert!(
            actions
                .chunks(2)
                .all(|pair| pair == [Step::Open, Step::Close])
        );
        assert_eq!(steps.last(), Some(&(Duration::ZERO, Step::Finish)));
        let total: Duration = steps.iter().map(|&(wait, _)| wait).sum();
        assert_eq!(total, Duration::from_secs(165));
    }

    #[test]
    fn steps_come_only_when_due_and_the_probe_then_ends() {
        let base = Instant::now();
        let mut probe = EguiProbe::start(base);
        assert_eq!(probe.take(base), None);
        let mut now = base;
        let mut taken = Vec::new();
        while let Some(due) = probe.due() {
            now = due;
            taken.extend(probe.take(now));
        }
        assert_eq!(taken.len(), steps().len());
        assert_eq!(now.duration_since(base), Duration::from_secs(165));
        assert_eq!(probe.take(now + HOLD), None);
    }

    #[test]
    fn a_sample_counts_the_wakes_of_its_own_phase() {
        let base = Instant::now();
        let mut probe = EguiProbe::start(base);
        probe.sample(Phase::Baseline, base + SETTLE, 30, None, None, None);
        probe.sample(
            Phase::Open(1),
            base + SETTLE + HOLD,
            330,
            None,
            Some(true),
            None,
        );
        assert_eq!(probe.samples()[0].wakes_per_minute, 120);
        assert_eq!(probe.samples()[1].wakes_per_minute, 1200);
        assert_eq!(probe.samples()[1].at, 30);
    }
}
