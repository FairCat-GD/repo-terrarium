//! Letting a command `record-tests` runs answer Ctrl-C itself.
//!
//! Ctrl-C reaches every process attached to the terminal: the wrapper and the command it runs.
//! Left with the default action, the wrapper would end at once and hand the prompt back while
//! the command was still winding down and printing. After [`defer_to_child`], the wrapper
//! notices the interrupt and does nothing, and keeps waiting; the command reacts as it always
//! does, and its exit becomes the wrapper's. The command does not inherit this: a caught
//! signal is reset to its default when a program starts (Unix), and a console handler belongs
//! to the process that installed it (Windows).

/// Makes Ctrl-C (and Ctrl-\ or Ctrl-Break) leave this process running. Call it just before
/// starting the command whose interrupt it is.
pub fn defer_to_child() {
    imp::defer_to_child();
}

#[cfg(target_os = "macos")]
mod imp {
    /// Does nothing, so the signal neither ends the process nor stays ignored in the command.
    extern "C" fn noticed(_signal: libc::c_int) {}

    pub fn defer_to_child() {
        let handler: extern "C" fn(libc::c_int) = noticed;
        for signal in [libc::SIGINT, libc::SIGQUIT] {
            // SAFETY: `noticed` is a plain function that lives as long as the process and does
            // nothing, which is async-signal-safe. macOS's `signal` restarts interrupted system
            // calls, and the wait for the command retries on `EINTR` anyway.
            unsafe { libc::signal(signal, handler as libc::sighandler_t) };
        }
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
    use windows::core::BOOL;

    /// TRUE for Ctrl-C and Ctrl-Break, so they do not end this process; a closed console,
    /// logoff or shutdown passes on.
    extern "system" fn noticed(control: u32) -> BOOL {
        BOOL::from(control == CTRL_C_EVENT || control == CTRL_BREAK_EVENT)
    }

    pub fn defer_to_child() {
        // SAFETY: the handler is a plain function that lives as long as the process and reads
        // only its argument. Unlike `SetConsoleCtrlHandler(None, TRUE)`, a handler is not
        // inherited, so the command still gets Ctrl-C.
        let _ = unsafe { SetConsoleCtrlHandler(Some(noticed), true) };
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    pub fn defer_to_child() {}
}
