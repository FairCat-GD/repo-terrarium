//! `replay <path> --dump`: a headless replay, printed as truth's canonical dump.
//!
//! It opens no window. Its store lives in a temporary directory that is removed when it ends,
//! so it never reads or writes the data directory.

use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use terrarium_engine::headless::{Request, replay_dump};
use terrarium_store::DataDir;
use terrarium_truth::canonical_dump;

use crate::cli::ReplayArgs;
use crate::copy;

pub fn run(args: &ReplayArgs) -> ExitCode {
    match dump(args) {
        Ok(text) => write_out(&text),
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn dump(args: &ReplayArgs) -> Result<String, String> {
    let temp = tempfile::tempdir().map_err(|err| copy::replay_store_unavailable(&err))?;
    let mut observations = match &args.observations {
        Some(path) => {
            Some(BufReader::new(File::open(path).map_err(|err| {
                copy::replay_observations_unreadable(path, &err)
            })?))
        }
        None => None,
    };
    let dump = replay_dump(Request {
        repo: &args.repo,
        data: &DataDir::at(temp.path()),
        observations: observations.as_mut().map(|file| file as &mut dyn BufRead),
        at_secs: args.at.unwrap_or_else(now_secs),
        install_salt: [0; 16],
    })
    .map_err(|err| copy::replay_failed(&args.repo, &err))?;
    Ok(canonical_dump(&dump))
}

pub(crate) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

/// Writes the dump to stdout. A reader that stops early, as `head` does, is not a failure.
fn write_out(text: &str) -> ExitCode {
    let mut out = std::io::stdout().lock();
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if err.kind() == ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{}", copy::replay_write_failed(&err));
            ExitCode::FAILURE
        }
    }
}
