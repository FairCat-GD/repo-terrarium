//! `replay <path> --dump`: a headless replay, printed as truth's canonical dump.
//!
//! It opens no window. Its store lives in a temporary directory that is removed when it ends,
//! so it never reads or writes the data directory.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::process::ExitCode;

use terrarium_engine::headless::{Request, replay_dump};
use terrarium_store::DataDir;
use terrarium_truth::canonical_dump;

use crate::cli::ReplayArgs;
use crate::{copy, now_secs, output};

pub fn run(args: &ReplayArgs) -> ExitCode {
    match dump(args) {
        Ok(text) => output::print(&text, copy::replay_write_failed),
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
