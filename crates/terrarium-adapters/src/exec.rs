//! The one place a process is started (I-5). `cargo xtask verify`'s graph stage refuses
//! `std::process::Command` in any other crate source.
//!
//! [`run_user_command`] runs the command a person typed after `repo-terrarium record-tests --`.
//! It is theirs, named explicitly on their own command line, so it runs as their shell would run
//! it: their environment, their working directory, their terminal. It is executed directly,
//! never through a shell, so its arguments reach it exactly as given. Nothing found in a
//! repository is ever run from here without the user's per-adapter opt-in.

use std::ffi::{OsStr, OsString};
use std::io;
use std::process::{Command, ExitStatus};

/// How a command ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finished {
    /// It exited with this code.
    Code(i32),
    /// A signal ended it (Unix), so it never reported a result.
    Signal(i32),
}

/// Why a command did not start.
#[derive(Debug)]
pub enum ExecError {
    /// No program by that name was found.
    NotFound(io::Error),
    /// The program exists but could not be started (permissions, format, …).
    CannotRun(io::Error),
}

/// Runs `program` with `args`, inheriting stdin, stdout, stderr, the environment and the
/// working directory, and waits for it to end.
pub fn run_user_command(program: &OsStr, args: &[OsString]) -> Result<Finished, ExecError> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|err| match err.kind() {
            io::ErrorKind::NotFound => ExecError::NotFound(err),
            _ => ExecError::CannotRun(err),
        })?;
    Ok(finished(status))
}

#[cfg(unix)]
fn finished(status: ExitStatus) -> Finished {
    use std::os::unix::process::ExitStatusExt as _;
    match (status.code(), status.signal()) {
        (Some(code), _) => Finished::Code(code),
        (None, Some(signal)) => Finished::Signal(signal),
        // A stopped or continued child is not an ended one; `status()` never returns either.
        (None, None) => Finished::Signal(0),
    }
}

#[cfg(not(unix))]
fn finished(status: ExitStatus) -> Finished {
    // Windows always has an exit code.
    Finished::Code(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_program_is_not_found() {
        let missing = OsStr::new("repo-terrarium-test-no-such-program");
        assert!(matches!(
            run_user_command(missing, &[]),
            Err(ExecError::NotFound(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn the_exit_code_comes_back() {
        // `sh` is the command here, typed by the test, as a person types theirs.
        let code = run_user_command(OsStr::new("sh"), &["-c".into(), "exit 3".into()]).unwrap();
        assert_eq!(code, Finished::Code(3));
        let killed =
            run_user_command(OsStr::new("sh"), &["-c".into(), "kill -TERM $$".into()]).unwrap();
        assert_eq!(killed, Finished::Signal(15));
    }
}
