//! The process's memory, as the OS counts it: for the debug window's readout, the egui memory
//! probe and the soak (N-02). Each OS's fair measure differs: the footprint on macOS, private
//! bytes and the working set on Windows.

/// Bytes of memory the process holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    /// Resident in physical memory: `ps`'s RSS on macOS, the working set on Windows.
    pub resident: u64,
    /// The most ever resident: Windows' peak working set. macOS keeps no such count.
    pub resident_peak: Option<u64>,
    /// What the process is charged for, compressed and swapped pages included: Activity
    /// Monitor's "Memory" on macOS. Windows has no single equivalent.
    pub footprint: Option<u64>,
    /// The most it was ever charged, as macOS counts it over the process's life.
    pub footprint_peak: Option<u64>,
    /// Memory committed to this process alone: Windows' private bytes.
    pub private: Option<u64>,
    /// The most ever committed to it alone: Windows' peak private bytes.
    pub private_peak: Option<u64>,
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
        let mut info = MaybeUninit::<libc::rusage_info_v4>::zeroed();
        // SAFETY: `info` is a writable `rusage_info_v4`, the structure the V4 flavour fills,
        // valid for the whole call; the pid is this process's own. It returns 0 on success.
        let status =
            unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, info.as_mut_ptr().cast()) };
        if status != 0 {
            return None;
        }
        // SAFETY: the call succeeded, so it filled the structure, which started zeroed.
        let info = unsafe { info.assume_init() };
        Some(Memory {
            resident: info.ri_resident_size,
            resident_peak: None,
            footprint: Some(info.ri_phys_footprint),
            footprint_peak: Some(info.ri_lifetime_max_phys_footprint),
            private: None,
            private_peak: None,
        })
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::GetCurrentProcess;

    use super::Memory;

    pub fn usage() -> Option<Memory> {
        let size = u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>()).ok()?;
        let mut counters = PROCESS_MEMORY_COUNTERS_EX {
            cb: size,
            ..Default::default()
        };
        // SAFETY: takes nothing and returns this process's pseudo-handle, which needs no
        // closing.
        let process = unsafe { GetCurrentProcess() };
        // SAFETY: `counters` is a live extended structure of the size passed in `cb`, which
        // tells the call it may fill the extended fields; it starts with the plain one.
        unsafe { GetProcessMemoryInfo(process, (&raw mut counters).cast(), size) }.ok()?;
        let bytes = |count: usize| u64::try_from(count).ok();
        Some(Memory {
            resident: bytes(counters.WorkingSetSize)?,
            resident_peak: bytes(counters.PeakWorkingSetSize),
            footprint: None,
            footprint_peak: None,
            private: bytes(counters.PrivateUsage),
            private_peak: bytes(counters.PeakPagefileUsage),
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
        let plausible = 1 << 20..1 << 40;
        assert!(plausible.contains(&memory.resident), "{memory:?}");
        // Each OS's fair measure is there, and a peak is never below its present value.
        let (now, peak) = if cfg!(target_os = "macos") {
            (memory.footprint, memory.footprint_peak)
        } else {
            (memory.private, memory.private_peak)
        };
        let (now, peak) = (now.expect("charged"), peak.expect("its peak"));
        assert!(plausible.contains(&now), "{memory:?}");
        assert!(peak >= now, "{memory:?}");
    }
}
