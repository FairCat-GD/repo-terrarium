//! Dim mode after a while without input anywhere on the machine. The strip asks the OS how long
//! it has been idle, only as often as the answer could change what it shows.

use std::time::Duration;

/// How long without input before the strip dims (the default).
pub const DIM_AFTER: Duration = Duration::from_mins(30);
/// How often a dimmed strip looks again, so input brightens it within a second.
pub const DIMMED_LOOK: Duration = Duration::from_secs(1);

/// Whether the strip is dimmed after `idle` without input, and how long until it should look
/// again. When the OS cannot say, the strip stays bright and asks again later.
pub fn check(idle: Option<Duration>) -> (bool, Duration) {
    match idle {
        Some(idle) if idle >= DIM_AFTER => (true, DIMMED_LOOK),
        Some(idle) => (false, DIM_AFTER.saturating_sub(idle).max(DIMMED_LOOK)),
        None => (false, DIM_AFTER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strip_dims_after_thirty_idle_minutes() {
        assert_eq!(check(Some(Duration::ZERO)), (false, DIM_AFTER));
        assert_eq!(
            check(Some(Duration::from_mins(29))),
            (false, Duration::from_mins(1))
        );
        assert_eq!(check(Some(DIM_AFTER)), (true, DIMMED_LOOK));
        assert_eq!(check(Some(Duration::from_hours(5))), (true, DIMMED_LOOK));
    }

    #[test]
    fn it_never_looks_more_than_once_a_second() {
        let almost = DIM_AFTER.saturating_sub(Duration::from_millis(10));
        assert_eq!(check(Some(almost)), (false, DIMMED_LOOK));
    }

    #[test]
    fn an_os_that_cannot_say_leaves_the_strip_bright() {
        assert_eq!(check(None), (false, DIM_AFTER));
    }
}
