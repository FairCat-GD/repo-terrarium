//! The processor time this process has used, as the OS counts it: the soak's hour average is
//! this over the wall time between two readings (N-01).

use std::time::Duration;

/// The processor time this process has used since it started, user and kernel time together,
/// across all its threads, or `None` if the OS cannot say.
pub fn time() -> Option<Duration> {
    imp::time()
}

#[cfg(target_os = "macos")]
mod imp {
    use std::mem::MaybeUninit;
    use std::time::Duration;

    // `getrusage`, not `proc_pid_rusage`: the latter's user and system times are in Mach
    // ticks on Apple silicon, the former's in microseconds everywhere.
    pub fn time() -> Option<Duration> {
        let mut usage = MaybeUninit::<libc::rusage>::zeroed();
        // SAFETY: `usage` is a writable `rusage`, valid for the whole call, which fills it for
        // this process (`RUSAGE_SELF`). It returns 0 on success.
        let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        if status != 0 {
            return None;
        }
        // SAFETY: the call succeeded, so it filled the structure, which started zeroed.
        let usage = unsafe { usage.assume_init() };
        Some(elapsed(usage.ru_utime)? + elapsed(usage.ru_stime)?)
    }

    fn elapsed(time: libc::timeval) -> Option<Duration> {
        let seconds = u64::try_from(time.tv_sec).ok()?;
        let micros = u64::try_from(time.tv_usec).ok()?;
        Some(Duration::from_secs(seconds) + Duration::from_micros(micros))
    }
}

#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    pub fn time() -> Option<Duration> {
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: takes nothing and returns this process's pseudo-handle, which needs no
        // closing.
        let process = unsafe { GetCurrentProcess() };
        // SAFETY: the four are live `FILETIME`s the call fills; the handle is this process's.
        unsafe {
            GetProcessTimes(
                process,
                &raw mut created,
                &raw mut exited,
                &raw mut kernel,
                &raw mut user,
            )
        }
        .ok()?;
        Some(ticks(kernel) + ticks(user))
    }

    /// A `FILETIME` duration counts 100-nanosecond intervals.
    fn ticks(time: FILETIME) -> Duration {
        let count = (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
        Duration::from_nanos(count.saturating_mul(100))
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::time::Instant;

    use super::*;

    #[test]
    fn the_os_counts_the_processor_time_spent() {
        let before = time().expect("the OS counts the process's processor time");
        // Spin for a quarter of a second of wall time; nearly all of it is processor time.
        let start = Instant::now();
        let mut spin = 0_u64;
        while start.elapsed() < Duration::from_millis(250) {
            spin = black_box(spin.wrapping_add(1));
        }
        let after = time().expect("the OS counts the process's processor time");
        let spent = after.saturating_sub(before);
        // A loaded machine may preempt the spin; a tenth of it is still far above the clock's
        // granularity.
        assert!(spent >= Duration::from_millis(25), "{spent:?}");
    }
}
