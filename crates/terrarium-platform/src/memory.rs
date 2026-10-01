//! The process's memory, as the OS counts it: for the debug window's readout and the egui
//! memory probe (N-02).

/// Bytes of memory the process holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    /// Resident in physical memory: `ps`'s RSS on macOS, the working set on Windows.
    pub resident: u64,
    /// What the process is charged for, compressed and swapped pages included: Activity
    /// Monitor's "Memory" on macOS. Windows has no single equivalent.
    pub footprint: Option<u64>,
}

/// This process's memory now, or `None` if the OS cannot say.
pub fn usage() -> Option<Memory> {
    imp::usage()
}

#[cfg(target_os = "macos")]
mod imp {
    use std::mem::MaybeUninit;

    use super::Memory;

    pub fn usage() -> Option<Memory> {
        let pid = libc::c_int::try_from(std::process::id()).ok()?;
        let mut info = MaybeUninit::<libc::rusage_info_v2>::zeroed();
        // SAFETY: `info` is a writable `rusage_info_v2`, the structure the V2 flavour fills,
        // valid for the whole call; the pid is this process's own. It returns 0 on success.
        let status =
            unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, info.as_mut_ptr().cast()) };
        if status != 0 {
            return None;
        }
        // SAFETY: the call succeeded, so it filled the structure, which started zeroed.
        let info = unsafe { info.assume_init() };
        Some(Memory {
            resident: info.ri_resident_size,
            footprint: Some(info.ri_phys_footprint),
        })
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    use super::Memory;

    pub fn usage() -> Option<Memory> {
        let size = u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: size,
            ..Default::default()
        };
        // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no closing, and
        // `counters` is a live structure of the size passed, which the call fills.
        unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &raw mut counters, size) }.ok()?;
        Some(Memory {
            resident: u64::try_from(counters.WorkingSetSize).ok()?,
            footprint: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_os_counts_this_process_s_memory() {
        let memory = usage().expect("the OS counts the process's memory");
        // A running test binary holds at least a megabyte, and far less than a terabyte.
        assert!((1 << 20..1 << 40).contains(&memory.resident), "{memory:?}");
    }
}
