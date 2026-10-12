//! When the live pipeline refreshes (N-06), as a pure function of what happened and two clocks:
//! the worker's monotonic time, for waits measured in milliseconds, and the wall clock, for the
//! hour truth runs on.
//!
//! - **A watcher event** waits until events have stopped for [`DEBOUNCE`]: one burst of writes
//!   (a commit, a rebase, a test run writing its reports) is one refresh. A burst that never
//!   stops is refreshed every [`SETTLE_AT_MOST`] all the same.
//! - **A follow-up** reads the reports again once a report still being written has been still
//!   long enough, or once a new watch is armed.
//! - **The hour.** Truth's tick is the UTC hour, so whenever the wall clock's hour is not the one
//!   last refreshed at, one catch-up runs. That is the hourly tick, and it is also what a wake
//!   from sleep or any jump of the clock looks like; since the tick is read from the clock and
//!   never counted, no tick is applied twice. The worker looks at least once a minute.
//! - **A wake** from the platform, or a request, refreshes at once.
//! - **A job** that came back (a CI poll's answer with something to record, a Knip run's
//!   report) refreshes at once, without reading the reports. A refresh due at once for a
//!   stronger reason is never weakened by one: a request still reads the reports and the
//!   settings.
//! - **Repository not found:** nothing but a retry once a minute.

use std::time::Duration;

/// How long the watcher must be quiet before a refresh, and how long a report must have been
/// still before it is read.
pub const DEBOUNCE: Duration = Duration::from_millis(500);
/// The longest a refresh waits for a stream of events to stop.
const SETTLE_AT_MOST: Duration = Duration::from_secs(5);
/// The longest the worker sleeps before it looks at the wall clock again.
const LOOK_EVERY: Duration = Duration::from_secs(60);
/// How often a repository that was not found is looked for.
const RETRY_EVERY: Duration = Duration::from_secs(60);

const SECONDS_PER_HOUR: i64 = 3_600;

/// Why a refresh runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// The first, at connect.
    Connect,
    /// A watcher batch.
    Watch,
    /// A report that was still being written, or a new watch.
    FollowUp,
    /// The wall clock's hour moved: the hourly tick, a wake, a jump.
    Hour,
    /// The platform said the machine woke.
    Wake,
    /// Asked for.
    Request,
    /// The repository that was not found is looked for again.
    Retry,
    /// A CI poll or a Knip run came back with something to record.
    Job,
}

impl Cause {
    /// Which of two refreshes due at once is the one run: a connect, then a request (which
    /// reads the reports and the settings), then a wake, then a job's.
    fn rank(self) -> u8 {
        match self {
            Cause::Connect => 4,
            Cause::Request => 3,
            Cause::Wake => 2,
            Cause::Job => 1,
            Cause::Watch | Cause::FollowUp | Cause::Hour | Cause::Retry => 0,
        }
    }
}

/// One refresh to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refresh {
    pub cause: Cause,
    /// Whether the reports are read. The hour and a wake read git and replay only: a report is
    /// read only behind the watcher's debounce or a stillness check.
    pub reports: bool,
}

/// What the scheduler is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The watcher saw something change: the reports, or only git.
    Touched {
        reports: bool,
    },
    Wake,
    Request,
    /// A job came back with something to record.
    Job,
}

/// Whether the repository can be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    Found,
    NotFound,
}

#[derive(Clone, Copy, Debug)]
struct Settle {
    first: Duration,
    due: Duration,
    reports: bool,
}

#[derive(Clone, Debug)]
pub struct Schedule {
    /// The UTC hour of the last refresh, read from the wall clock.
    hour: i64,
    /// A pending watcher refresh: when its first event came, when it is due, and whether it
    /// touched the reports.
    settle: Option<Settle>,
    follow_up: Option<Duration>,
    now: Option<Cause>,
    reach: Reach,
    retry_at: Duration,
}

/// The UTC hour holding `wall`.
fn hour_of(wall: i64) -> i64 {
    wall.div_euclid(SECONDS_PER_HOUR)
}

impl Schedule {
    /// A schedule whose first refresh is the connect's, at once.
    pub fn new(wall: i64) -> Schedule {
        Schedule {
            hour: hour_of(wall),
            settle: None,
            follow_up: None,
            now: Some(Cause::Connect),
            reach: Reach::Found,
            retry_at: Duration::ZERO,
        }
    }

    pub fn on(&mut self, event: Event, mono: Duration) {
        if self.reach == Reach::NotFound {
            // The watcher is gone; a wake or a request only looks for the repository sooner,
            // and a job has nothing to record into.
            if matches!(event, Event::Wake | Event::Request) {
                self.retry_at = mono;
            }
            return;
        }
        match event {
            Event::Touched { reports } => {
                let first = self.settle.map_or(mono, |settle| settle.first);
                self.settle = Some(Settle {
                    first,
                    due: (mono + DEBOUNCE).min(first + SETTLE_AT_MOST),
                    reports: reports || self.settle.is_some_and(|settle| settle.reports),
                });
            }
            Event::Wake => self.at_once(Cause::Wake),
            Event::Request => self.at_once(Cause::Request),
            Event::Job => self.at_once(Cause::Job),
        }
    }

    /// A refresh at once for `cause`, unless one is due at once for a stronger reason.
    fn at_once(&mut self, cause: Cause) {
        if self.now.is_none_or(|now| now.rank() < cause.rank()) {
            self.now = Some(cause);
        }
    }

    /// Reads the reports again after `delay`, unless a follow-up is due sooner.
    pub fn follow_up(&mut self, mono: Duration, delay: Duration) {
        let at = mono + delay;
        self.follow_up = Some(self.follow_up.map_or(at, |due| due.min(at)));
    }

    /// Records whether the last refresh reached the repository.
    pub fn reached(&mut self, reach: Reach, mono: Duration) {
        if reach == Reach::NotFound {
            self.settle = None;
            self.follow_up = None;
            self.now = None;
            self.retry_at = mono + RETRY_EVERY;
        }
        self.reach = reach;
    }

    /// The refresh to run now, if one is due; taking it clears what it covers.
    pub fn take(&mut self, mono: Duration, wall: i64) -> Option<Refresh> {
        let hour = hour_of(wall);
        if self.reach == Reach::NotFound {
            if mono < self.retry_at {
                return None;
            }
            self.retry_at = mono + RETRY_EVERY;
            self.hour = hour;
            return Some(Refresh {
                cause: Cause::Retry,
                reports: true,
            });
        }
        let settled = self.settle.is_some_and(|settle| mono >= settle.due);
        let followed = self.follow_up.is_some_and(|at| mono >= at);
        let cause = self
            .now
            .or_else(|| settled.then_some(Cause::Watch))
            .or_else(|| followed.then_some(Cause::FollowUp))
            .or_else(|| (hour != self.hour).then_some(Cause::Hour))?;
        // A refresh covers everything pending, a batch still settling included: the reports it
        // reads are read only once they have been still long enough, whatever the cause.
        let reports = matches!(cause, Cause::Connect | Cause::Request)
            || self.settle.is_some_and(|settle| settle.reports)
            || self.follow_up.is_some();
        self.settle = None;
        self.follow_up = None;
        self.now = None;
        self.hour = hour;
        Some(Refresh { cause, reports })
    }

    /// How long the worker may sleep before something is due: never past the next hour, and
    /// never more than [`LOOK_EVERY`].
    pub fn wait(&self, mono: Duration, wall: i64) -> Duration {
        if self.now.is_some() {
            return Duration::ZERO;
        }
        let to_hour = Duration::from_secs(
            u64::try_from(SECONDS_PER_HOUR - wall.rem_euclid(SECONDS_PER_HOUR)).unwrap_or(0),
        );
        let mut wait = LOOK_EVERY.min(to_hour);
        let mut until = |at: Duration| wait = wait.min(at.saturating_sub(mono));
        match self.reach {
            Reach::NotFound => until(self.retry_at),
            Reach::Found => {
                if let Some(settle) = self.settle {
                    until(settle.due);
                }
                if let Some(at) = self.follow_up {
                    until(at);
                }
            }
        }
        wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600;
    /// 10:00 UTC on a day in 2024.
    const T: i64 = 1_704_708_000;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn connected() -> Schedule {
        let mut s = Schedule::new(T);
        assert_eq!(
            s.take(ms(0), T),
            Some(Refresh {
                cause: Cause::Connect,
                reports: true
            })
        );
        s
    }

    #[test]
    fn a_burst_of_events_is_one_refresh_once_it_stops() {
        let mut s = connected();
        assert_eq!(s.take(ms(1), T), None);
        s.on(Event::Touched { reports: false }, ms(1_000));
        s.on(Event::Touched { reports: true }, ms(1_300));
        s.on(Event::Touched { reports: false }, ms(1_600));
        assert_eq!(s.wait(ms(1_600), T), DEBOUNCE);
        assert_eq!(s.take(ms(2_099), T), None);
        assert_eq!(
            s.take(ms(2_100), T),
            Some(Refresh {
                cause: Cause::Watch,
                reports: true
            })
        );
        assert_eq!(s.take(ms(2_101), T), None);
    }

    #[test]
    fn events_that_never_stop_still_refresh() {
        let mut s = connected();
        let mut at = ms(0);
        while at < SETTLE_AT_MOST {
            s.on(Event::Touched { reports: false }, at);
            assert_eq!(s.take(at, T), None);
            at += ms(100);
        }
        s.on(Event::Touched { reports: false }, at);
        assert_eq!(s.take(at, T).map(|r| r.cause), Some(Cause::Watch));
    }

    #[test]
    fn a_git_only_batch_reads_no_reports_and_a_follow_up_does() {
        let mut s = connected();
        s.on(Event::Touched { reports: false }, ms(0));
        assert_eq!(
            s.take(DEBOUNCE, T),
            Some(Refresh {
                cause: Cause::Watch,
                reports: false
            })
        );
        s.follow_up(ms(10_000), ms(500));
        s.follow_up(ms(10_000), ms(900));
        assert_eq!(s.wait(ms(10_000), T), ms(500));
        assert_eq!(s.take(ms(10_499), T), None);
        assert_eq!(
            s.take(ms(10_500), T),
            Some(Refresh {
                cause: Cause::FollowUp,
                reports: true
            })
        );
    }

    #[test]
    fn every_new_hour_catches_up_once_however_the_clock_got_there() {
        let mut s = connected();
        // The hourly tick.
        assert_eq!(s.wait(ms(0), T + HOUR - 10), Duration::from_secs(10));
        assert_eq!(s.take(ms(0), T + HOUR - 1), None);
        let tick = s.take(ms(0), T + HOUR).unwrap();
        assert_eq!(tick.cause, Cause::Hour);
        assert!(!tick.reports);
        assert_eq!(s.take(ms(1), T + HOUR + 5), None);
        // A wake five hours later, which the monotonic clock never saw: one catch-up.
        assert_eq!(s.take(ms(2), T + 6 * HOUR).unwrap().cause, Cause::Hour);
        assert_eq!(s.take(ms(3), T + 6 * HOUR + 1), None);
        // A clock set back an hour moves the hour too; the pipeline never rewinds.
        assert_eq!(s.take(ms(4), T + 5 * HOUR).unwrap().cause, Cause::Hour);
        assert_eq!(s.take(ms(5), T + 5 * HOUR), None);
    }

    #[test]
    fn the_worker_looks_at_least_once_a_minute() {
        let s = connected();
        assert_eq!(s.wait(ms(0), T), LOOK_EVERY);
        assert_eq!(s.wait(ms(0), T + HOUR - 1), Duration::from_secs(1));
    }

    #[test]
    fn a_wake_or_a_request_refreshes_at_once_and_covers_a_settling_batch() {
        let mut s = connected();
        s.on(Event::Touched { reports: false }, ms(0));
        s.on(Event::Wake, ms(10));
        assert_eq!(s.wait(ms(10), T), Duration::ZERO);
        assert_eq!(
            s.take(ms(10), T),
            Some(Refresh {
                cause: Cause::Wake,
                reports: false
            })
        );
        assert_eq!(s.take(DEBOUNCE + ms(10), T), None);
        s.on(Event::Request, ms(20));
        assert_eq!(
            s.take(ms(20), T),
            Some(Refresh {
                cause: Cause::Request,
                reports: true
            })
        );
    }

    #[test]
    fn a_missing_repository_is_only_looked_for_once_a_minute() {
        let mut s = connected();
        s.on(Event::Touched { reports: true }, ms(0));
        s.reached(Reach::NotFound, ms(0));
        s.on(Event::Touched { reports: true }, ms(10));
        assert_eq!(s.take(DEBOUNCE + ms(10), T + HOUR), None);
        assert_eq!(s.wait(ms(10), T), RETRY_EVERY.saturating_sub(ms(10)));
        let retry = s.take(RETRY_EVERY, T + HOUR).unwrap();
        assert_eq!(retry.cause, Cause::Retry);
        assert!(retry.reports);
        assert_eq!(s.take(RETRY_EVERY + ms(1), T + HOUR), None);
        // A wake looks at once.
        s.on(Event::Wake, RETRY_EVERY + ms(2));
        assert_eq!(
            s.take(RETRY_EVERY + ms(2), T + HOUR).unwrap().cause,
            Cause::Retry
        );
        s.reached(Reach::Found, RETRY_EVERY + ms(3));
        assert_eq!(s.take(RETRY_EVERY + ms(4), T + HOUR), None);
    }

    #[test]
    fn a_refresh_due_at_once_is_never_weakened() {
        let mut s = connected();
        s.on(Event::Request, ms(10));
        s.on(Event::Job, ms(11));
        s.on(Event::Wake, ms(12));
        assert_eq!(
            s.take(ms(13), T),
            Some(Refresh {
                cause: Cause::Request,
                reports: true
            }),
            "the request still reads the reports and the settings"
        );
        s.on(Event::Job, ms(20));
        assert_eq!(
            s.take(ms(21), T),
            Some(Refresh {
                cause: Cause::Job,
                reports: false
            })
        );
        s.on(Event::Job, ms(30));
        s.on(Event::Wake, ms(31));
        assert_eq!(s.take(ms(32), T).map(|r| r.cause), Some(Cause::Wake));
        assert_eq!(s.wait(ms(32), T), Duration::from_secs(60));
    }

    #[test]
    fn a_job_while_the_repository_is_missing_is_dropped() {
        let mut s = connected();
        s.reached(Reach::NotFound, ms(10));
        s.on(Event::Job, ms(20));
        assert_eq!(s.take(ms(21), T), None);
    }
}
