//! The console the app was started with. On Windows the app is a console program, so the
//! commands a person types in a terminal print there and are waited for. Started from Explorer,
//! the Start menu or the login item, Windows gives it a console window of its own, which would
//! sit beside the strip and end the app when closed; the strip lets go of that one.

/// Lets go of the console when this process is the only one attached to it: Windows opened it
/// for this launch, and no terminal is waiting on it. A console shared with a terminal is kept,
/// so its Ctrl-C still ends the strip. Whether it let go; always `false` off Windows.
pub fn release_if_alone() -> bool {
    imp::release_if_alone()
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{
        FreeConsole, GetConsoleProcessList, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        SetStdHandle,
    };

    pub fn release_if_alone() -> bool {
        let mut attached = [0_u32; 2];
        // SAFETY: the buffer is writable for its whole length, which the wrapper passes; the
        // count returned can be larger than the buffer, and nothing past it is written.
        let count = unsafe { GetConsoleProcessList(&mut attached) };
        if count != 1 {
            return false;
        }
        // SAFETY: detaches this process from its console; nothing here holds the console.
        if unsafe { FreeConsole() }.is_err() {
            return false;
        }
        // The standard handles named the console. Null, Rust's standard streams take writes as
        // done and print nothing, where the closed console's handles could fail them.
        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: sets one of this process's standard handles; a null handle is allowed.
            let _ = unsafe { SetStdHandle(which, HANDLE::default()) };
        }
        true
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn release_if_alone() -> bool {
        false
    }
}
