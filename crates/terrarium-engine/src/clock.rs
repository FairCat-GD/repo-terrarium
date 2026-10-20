//! The wall clock the live pipeline reads (truth ends at the hour holding now). It is a trait
//! so tests can set the time, jump it forwards and back, and cross hours on demand.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync + 'static {
    /// Now, as Unix seconds, UTC.
    fn wall_secs(&self) -> i64;
}

/// The operating system's clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn wall_secs(&self) -> i64 {
        wall_secs()
    }
}

/// The operating system's clock, as Unix seconds, UTC: negative before 1970, which only a clock
/// set far back gives. The one reading of the wall clock every crate above truth shares.
pub fn wall_secs() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_secs()).map_or(i64::MIN, |s| -s),
    }
}

/// A clock that says what it is told, shared between the test that sets it and the engine that
/// reads it.
#[derive(Clone, Debug)]
pub struct ManualClock(Arc<AtomicI64>);

impl ManualClock {
    pub fn at(secs: i64) -> ManualClock {
        ManualClock(Arc::new(AtomicI64::new(secs)))
    }

    pub fn set(&self, secs: i64) {
        self.0.store(secs, Ordering::SeqCst);
    }

    pub fn advance(&self, secs: i64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn wall_secs(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manual_clock_is_shared() {
        let clock = ManualClock::at(100);
        let read = clock.clone();
        clock.advance(3_600);
        assert_eq!(read.wall_secs(), 3_700);
        clock.set(5);
        assert_eq!(read.wall_secs(), 5);
    }

    #[test]
    fn the_system_clock_is_after_2024() {
        assert!(SystemClock.wall_secs() > 1_704_067_200);
    }
}
