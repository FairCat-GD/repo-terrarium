//! The raise hotkey's timer: the strip comes above other windows for a while, then goes back.

use std::time::{Duration, Instant};

/// How long a raise lasts.
pub const RAISE_FOR: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
pub struct RaiseTimer {
    until: Option<Instant>,
}

impl RaiseTimer {
    /// Starts a raise, or extends the one in progress.
    pub fn press(&mut self, now: Instant) {
        self.until = Some(now + RAISE_FOR);
    }

    /// When the loop must wake to restore the level. `None` while nothing is raised, so the
    /// loop sleeps until an event arrives.
    pub fn deadline(&self) -> Option<Instant> {
        self.until
    }

    /// True exactly once, at or after the deadline.
    pub fn restore_due(&mut self, now: Instant) -> bool {
        match self.until {
            Some(until) if now >= until => {
                self.until = None;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_raised_means_no_deadline() {
        let mut timer = RaiseTimer::default();
        assert_eq!(timer.deadline(), None);
        assert!(!timer.restore_due(Instant::now()));
    }

    #[test]
    fn a_raise_restores_once_after_ten_seconds() {
        let start = Instant::now();
        let mut timer = RaiseTimer::default();
        timer.press(start);
        assert_eq!(timer.deadline(), Some(start + RAISE_FOR));
        assert!(!timer.restore_due(start + Duration::from_millis(9_999)));
        assert!(timer.restore_due(start + RAISE_FOR));
        assert!(!timer.restore_due(start + RAISE_FOR * 2));
        assert_eq!(timer.deadline(), None);
    }

    #[test]
    fn pressing_again_extends_the_raise() {
        let start = Instant::now();
        let mut timer = RaiseTimer::default();
        timer.press(start);
        timer.press(start + Duration::from_secs(6));
        assert!(!timer.restore_due(start + RAISE_FOR));
        assert!(timer.restore_due(start + Duration::from_secs(16)));
    }
}
