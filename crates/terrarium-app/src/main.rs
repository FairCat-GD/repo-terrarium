#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod cli;
mod copy;
mod hotkey;
mod hover;
mod menu;
mod monitor;
mod pinning;
mod present;
mod probe;
mod raise;
mod strip;

use std::process::ExitCode;

use cli::{Command, ProbeKind};

/// The conventional exit status for a command line the program does not accept.
const USAGE_EXIT: u8 = 2;

fn main() -> ExitCode {
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(Command::Run) => run(None),
        Ok(Command::Probe(kind)) => run(Some(kind)),
        Ok(Command::Version) => {
            println!("{}", copy::version_line());
            ExitCode::SUCCESS
        }
        Ok(Command::ProbeReadme) => {
            print!("{}", copy::PROBE_README);
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{}", copy::usage_error(&err));
            ExitCode::from(USAGE_EXIT)
        }
    }
}

fn run(probe: Option<ProbeKind>) -> ExitCode {
    match strip::run(probe) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{}", copy::window_failed(&err));
            ExitCode::FAILURE
        }
    }
}
