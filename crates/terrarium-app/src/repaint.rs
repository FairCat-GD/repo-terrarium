//! When each egui window next wants painting. egui names a delay after every pass, and may ask
//! between passes; the loop waits for the earliest of them and for nothing once a window has
//! none, so an open window that shows nothing new keeps the loop asleep (I-10).

use std::time::{Duration, Instant};

/// The next repaint each window asked for: at most one per window, the earliest.
#[derive(Debug)]
pub struct RepaintSchedule<K> {
    due: Vec<(K, Instant)>,
}

impl<K> Default for RepaintSchedule<K> {
    fn default() -> Self {
        RepaintSchedule { due: Vec::new() }
    }
}

impl<K: Copy + PartialEq> RepaintSchedule<K> {
    /// `key` wants painting at `at`; an earlier request it already has stands.
    pub fn request(&mut self, key: K, at: Instant) {
        match self.due.iter_mut().find(|(k, _)| *k == key) {
            Some((_, due)) => *due = (*due).min(at),
            None => self.due.push((key, at)),
        }
    }

    /// The windows due by `now`, each taken off the schedule: a deadline met is never waited
    /// for again, so a past one cannot keep the loop spinning.
    pub fn take_due(&mut self, now: Instant) -> Vec<K> {
        let mut taken = Vec::new();
        self.due.retain(|&(key, at)| {
            let due = at <= now;
            if due {
                taken.push(key);
            }
            !due
        });
        taken
    }

    /// `key` is painting now, or has closed: what it asked for is answered.
    pub fn forget(&mut self, key: K) {
        self.due.retain(|(k, _)| *k != key);
    }

    /// The earliest repaint anything wants.
    pub fn next(&self) -> Option<Instant> {
        self.due.iter().map(|&(_, at)| at).min()
    }
}

/// The moment a repaint `delay` after `now` falls due; egui's "never" is `Duration::MAX`.
pub fn after(now: Instant, delay: Duration) -> Option<Instant> {
    now.checked_add(delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn the_earliest_request_for_a_window_wins() {
        let base = Instant::now();
        let mut schedule = RepaintSchedule::default();
        schedule.request(1, ms(base, 500));
        schedule.request(1, ms(base, 200));
        schedule.request(1, ms(base, 900));
        schedule.request(2, ms(base, 300));
        assert_eq!(schedule.next(), Some(ms(base, 200)));
        assert_eq!(schedule.take_due(ms(base, 250)), [1]);
        assert_eq!(schedule.next(), Some(ms(base, 300)));
    }

    #[test]
    fn a_met_deadline_is_taken_off_and_never_waited_for_again() {
        let base = Instant::now();
        let mut schedule = RepaintSchedule::default();
        schedule.request(7, ms(base, 100));
        assert!(schedule.take_due(ms(base, 50)).is_empty());
        assert_eq!(schedule.take_due(ms(base, 100)), [7]);
        assert!(schedule.take_due(ms(base, 5_000)).is_empty());
        assert_eq!(schedule.next(), None);
    }

    #[test]
    fn a_closed_window_is_forgotten() {
        let base = Instant::now();
        let mut schedule = RepaintSchedule::default();
        schedule.request(1, ms(base, 100));
        schedule.request(2, ms(base, 200));
        schedule.forget(1);
        assert_eq!(schedule.next(), Some(ms(base, 200)));
        schedule.forget(2);
        assert_eq!(schedule.next(), None);
    }

    #[test]
    fn egui_s_never_is_no_deadline() {
        let base = Instant::now();
        assert_eq!(after(base, Duration::MAX), None);
        assert_eq!(after(base, Duration::from_millis(500)), Some(ms(base, 500)));
    }
}
