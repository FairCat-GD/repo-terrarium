//! Dim mode after a while without input anywhere on the machine (`dim.after_minutes`). The
//! strip asks the OS how long it has been idle, only as often as the answer could change what
//! it shows.

use std::time::Duration;

/// How often a dimmed strip looks again, so input brightens it within a second.
pub const DIMMED_LOOK: Duration = Duration::from_secs(1);

/// Whether the strip is dimmed after `idle` without input, when it dims `after` that long, and
/// how long until it should look again. When the OS cannot say, the strip stays bright and asks
/// again later.
pub fn check(idle: Option<Duration>, after: Duration) -> (bool, Duration) {
    match idle {
        Some(idle) if idle >= after => (true, DIMMED_LOOK),
        Some(idle) => (false, after.saturating_sub(idle).max(DIMMED_LOOK)),
        None => (false, after),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THIRTY: Duration = Duration::from_mins(30);

    #[test]
    fn the_strip_dims_after_the_idle_time_it_is_given() {
        assert_eq!(check(Some(Duration::ZERO), THIRTY), (false, THIRTY));
        assert_eq!(
            check(Some(Duration::from_mins(29)), THIRTY),
            (false, Duration::from_mins(1))
        );
        assert_eq!(check(Some(THIRTY), THIRTY), (true, DIMMED_LOOK));
        assert_eq!(
            check(Some(Duration::from_hours(5)), THIRTY),
            (true, DIMMED_LOOK)
        );
        let five = Duration::from_mins(5);
        assert_eq!(
            check(Some(Duration::from_mins(6)), five),
            (true, DIMMED_LOOK)
        );
        assert_eq!(
            check(Some(Duration::from_mins(29)), Duration::from_mins(120)),
            (false, Duration::from_mins(91))
        );
    }

    #[test]
    fn it_never_looks_more_than_once_a_second() {
        let almost = THIRTY.saturating_sub(Duration::from_millis(10));
        assert_eq!(check(Some(almost), THIRTY), (false, DIMMED_LOOK));
    }

    #[test]
    fn an_os_that_cannot_say_leaves_the_strip_bright() {
        assert_eq!(check(None, THIRTY), (false, THIRTY));
    }
}
