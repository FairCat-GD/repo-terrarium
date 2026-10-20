//! `repo-terrarium record-tests -- <command>`: runs the person's test command and records how
//! it ended, as local weather.
//!
//! The command runs directly, never through a shell, in the current directory, with the
//! terminal and environment it was typed in. Its exit code is this process's exit code, so a
//! script or editor that runs the tests through it sees no difference. The result goes into the
//! data directory (`local-tests/<seed-hex>.json`), never into the repository.
//!
//! Before anything runs, the repository around the current directory and the data directory
//! must both be found; if not, nothing runs, and the exit code is 125. A command that cannot be
//! found exits 127 and one that cannot be started 126, as `env` and `timeout` do. A run a signal
//! ended reported no result, so nothing is written, and the exit code is 128 plus the signal.

use terrarium_adapters::exec::{self, ExecError, Finished};
use terrarium_adapters::git::GitRepo;
use terrarium_adapters::local_tests::Status;
use terrarium_store::DataDir;

use crate::cli::RecordTestsArgs;
use crate::copy;
use terrarium_engine::wall_secs;

/// This command's own failure: the tests never ran.
const NOT_RUN: i32 = 125;
const CANNOT_RUN: i32 = 126;
const NOT_FOUND: i32 = 127;
/// A shell's exit status for a process a signal ended is 128 plus the signal.
const SIGNALLED: i32 = 128;

/// Runs the command and returns the exit code to leave with.
pub fn run(args: &RecordTestsArgs) -> i32 {
    let status_file = match locate() {
        Ok(path) => path,
        Err(message) => {
            eprintln!("{message}");
            return NOT_RUN;
        }
    };
    let started_at = wall_secs();
    terrarium_platform::interrupt::defer_to_child();
    match exec::run_user_command(&args.program, &args.args) {
        Ok(Finished::Code(code)) => {
            let status = Status::new(i64::from(code), started_at, wall_secs());
            if let Err(err) = status.write(&status_file) {
                eprintln!("{}", copy::record_tests_not_recorded(&err));
            }
            code
        }
        Ok(Finished::Signal(signal)) => SIGNALLED.saturating_add(signal),
        Err(ExecError::NotFound(err)) => {
            eprintln!("{}", copy::record_tests_not_found(&args.program, &err));
            NOT_FOUND
        }
        Err(ExecError::CannotRun(err)) => {
            eprintln!("{}", copy::record_tests_cannot_run(&args.program, &err));
            CANNOT_RUN
        }
    }
}

/// Where this repository's status file goes, or what stops it from being written.
fn locate() -> Result<std::path::PathBuf, String> {
    let here = std::env::current_dir().map_err(|err| copy::record_tests_no_directory(&err))?;
    let git =
        GitRepo::discover(&here).map_err(|err| copy::record_tests_no_repository(&here, &err))?;
    let (_, identity) = git
        .default_identity()
        .map_err(|err| copy::record_tests_no_repository(&here, &err))?;
    let seed = identity.seed;
    let data = DataDir::resolve().map_err(|err| copy::record_tests_no_data_dir(&err))?;
    Ok(data.local_tests(&seed))
}
