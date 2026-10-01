//! How long the machine has gone without input, anywhere: dim mode starts after a while of it.
//! Neither OS asks the user for permission to tell.

use std::time::Duration;

/// The time since the last keyboard, mouse or other input on this machine, or `None` if the
/// OS cannot say.
pub fn since_input() -> Option<Duration> {
    imp::since_input()
}

#[cfg(target_os = "macos")]
mod imp {
    use std::time::Duration;

    use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGEventType};

    /// `kCGAnyInputEventType`: every kind of input event.
    const ANY_INPUT: CGEventType = CGEventType(u32::MAX);

    pub fn since_input() -> Option<Duration> {
        // The combined session state covers the whole login session, not only this app's
        // events, and needs no Input Monitoring permission.
        let seconds = CGEventSource::seconds_since_last_event_type(
            CGEventSourceStateID::CombinedSessionState,
            ANY_INPUT,
        );
        Duration::try_from_secs_f64(seconds).ok()
    }
}

#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    pub fn since_input() -> Option<Duration> {
        let mut last = LASTINPUTINFO {
            cbSize: u32::try_from(std::mem::size_of::<LASTINPUTINFO>()).ok()?,
            dwTime: 0,
        };
        // SAFETY: `last` is a live `LASTINPUTINFO` whose `cbSize` is set, as the call requires;
        // it writes only `dwTime`.
        if !unsafe { GetLastInputInfo(&raw mut last) }.as_bool() {
            return None;
        }
        // SAFETY: `GetTickCount` takes nothing and cannot fail.
        let now = unsafe { GetTickCount() };
        // Both are milliseconds since boot in 32 bits, wrapping every 49.7 days.
        Some(Duration::from_millis(u64::from(
            now.wrapping_sub(last.dwTime),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_os_says_how_long_input_has_been_idle() {
        let idle = since_input().expect("the OS knows the idle time");
        // Uptime bounds it; a year is far beyond any test machine's.
        assert!(idle < Duration::from_hours(366 * 24), "{idle:?}");
    }
}
