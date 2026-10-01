//! The local wall clock: today's date and the time of day where the user is, for day and night.
//! The standard library knows only UTC, so the time zone comes from the OS.

/// A moment on the local wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalNow {
    pub year: i32,
    /// 1 to 12.
    pub month: u8,
    /// 1 to 31.
    pub day: u8,
    /// Milliseconds since local midnight, below 86,400,000.
    pub ms_of_day: u32,
}

impl LocalNow {
    /// The moment these fields name, or `None` when one is out of range. A leap second reads
    /// as the second before it, so the day never runs long.
    pub fn from_fields(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        millis: u32,
    ) -> Option<LocalNow> {
        if !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || hour > 23
            || minute > 59
            || second > 60
            || millis > 999
        {
            return None;
        }
        let seconds = (hour * 60 + minute) * 60 + second.min(59);
        Some(LocalNow {
            year,
            month: u8::try_from(month).ok()?,
            day: u8::try_from(day).ok()?,
            ms_of_day: seconds * 1000 + millis,
        })
    }
}

/// The local date and time now, or `None` if the OS cannot say.
pub fn local_now() -> Option<LocalNow> {
    imp::local_now()
}

#[cfg(target_os = "macos")]
mod imp {
    use std::mem::MaybeUninit;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::LocalNow;

    pub fn local_now() -> Option<LocalNow> {
        let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
        let seconds = libc::time_t::try_from(since_epoch.as_secs()).ok()?;
        let mut fields = MaybeUninit::<libc::tm>::zeroed();
        // SAFETY: `seconds` is a live `time_t` and `fields` a writable `tm`, both valid for the
        // whole call. `localtime_r` is the thread-safe form: it writes only into `fields` and
        // returns null on failure.
        let filled = unsafe { libc::localtime_r(&raw const seconds, fields.as_mut_ptr()) };
        if filled.is_null() {
            return None;
        }
        // SAFETY: `localtime_r` returned non-null, so it filled every field; the structure
        // started zeroed, so even a field it skipped holds a valid value.
        let tm = unsafe { fields.assume_init() };
        let field = |value: libc::c_int| u32::try_from(value).ok();
        LocalNow::from_fields(
            tm.tm_year.checked_add(1900)?,
            field(tm.tm_mon)? + 1,
            field(tm.tm_mday)?,
            field(tm.tm_hour)?,
            field(tm.tm_min)?,
            field(tm.tm_sec)?,
            since_epoch.subsec_millis(),
        )
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::System::SystemInformation::GetLocalTime;

    use super::LocalNow;

    pub fn local_now() -> Option<LocalNow> {
        // SAFETY: `GetLocalTime` takes nothing and returns the time by value; it cannot fail.
        let time = unsafe { GetLocalTime() };
        LocalNow::from_fields(
            i32::from(time.wYear),
            u32::from(time.wMonth),
            u32::from(time.wDay),
            u32::from(time.wHour),
            u32::from(time.wMinute),
            u32::from(time.wSecond),
            u32::from(time.wMilliseconds),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_become_a_date_and_milliseconds_since_midnight() {
        assert_eq!(
            LocalNow::from_fields(2026, 10, 1, 19, 30, 15, 250),
            Some(LocalNow {
                year: 2026,
                month: 10,
                day: 1,
                ms_of_day: ((19 * 60 + 30) * 60 + 15) * 1000 + 250,
            })
        );
        let last = LocalNow::from_fields(2026, 12, 31, 23, 59, 59, 999).unwrap();
        assert_eq!(last.ms_of_day, 86_399_999);
    }

    #[test]
    fn a_leap_second_stays_inside_the_day() {
        let leap = LocalNow::from_fields(2016, 12, 31, 23, 59, 60, 999).unwrap();
        assert_eq!(leap.ms_of_day, 86_399_999);
    }

    #[test]
    fn out_of_range_fields_are_refused() {
        for (month, day, hour, minute, second, millis) in [
            (0, 1, 0, 0, 0, 0),
            (13, 1, 0, 0, 0, 0),
            (1, 0, 0, 0, 0, 0),
            (1, 32, 0, 0, 0, 0),
            (1, 1, 24, 0, 0, 0),
            (1, 1, 0, 60, 0, 0),
            (1, 1, 0, 0, 61, 0),
            (1, 1, 0, 0, 0, 1000),
        ] {
            assert_eq!(
                LocalNow::from_fields(2026, month, day, hour, minute, second, millis),
                None
            );
        }
    }

    #[test]
    fn the_os_names_a_moment_today() {
        let now = local_now().expect("the OS knows the local time");
        assert!(now.year >= 2026, "{now:?}");
        assert!(now.ms_of_day < 86_400_000);
    }
}
