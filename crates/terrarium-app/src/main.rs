#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod animate;
mod cli;
mod connect;
mod copy;
#[cfg(feature = "dev")]
mod debug;
mod demo;
#[cfg(feature = "dev")]
mod dev;
mod discover;
mod egui_host;
#[cfg(feature = "dev")]
mod egui_probe;
mod feed;
mod hotkey;
mod hover;
mod idle;
mod instance;
mod lifecycle;
mod live;
mod login;
mod menu;
mod monitor;
mod output;
mod pinning;
mod present;
mod probe;
mod raise;
mod record_tests;
mod repaint;
mod replay;
#[cfg(feature = "dev")]
mod sandbox;
mod settings;
mod shell;
mod strip;

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use cli::Command;
use feed::Feed;
use strip::Mode;

/// The conventional exit status for a command line the program does not accept.
const USAGE_EXIT: u8 = 2;

fn main() -> ExitCode {
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(Command::Run(repo)) => live::run(repo),
        Ok(Command::Demo) => run(&Mode::Live(Feed::Demo), None),
        Ok(Command::Probe(kind)) => run(&Mode::Probe(kind), None),
        Ok(Command::Version) => {
            output::print(&format!("{}\n", copy::version_line()), copy::print_failed)
        }
        // Byte for byte: `cargo xtask probe-zip` writes this into the zip as `README.txt`.
        Ok(Command::ProbeReadme) => output::print(copy::PROBE_README, copy::print_failed),
        Ok(Command::Replay(args)) => replay::run(&args),
        Ok(Command::Discover(repo)) => discover::run(&repo),
        // The command's own exit code, whole: `ExitCode` holds only 0 to 255, and Windows codes
        // are wider. Nothing is left to drop: messages went to the unbuffered stderr.
        Ok(Command::RecordTests(args)) => std::process::exit(record_tests::run(&args)),
        Err(err) => {
            eprintln!("{}", copy::usage_error(&err));
            ExitCode::from(USAGE_EXIT)
        }
    }
}

/// Unix seconds now: 0 before 1970, which only a clock set far back gives.
pub(crate) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

fn run(mode: &Mode, launch: Option<live::Launch>) -> ExitCode {
    match strip::run(mode, launch) {
        Ok(()) => ExitCode::SUCCESS,
        Err(strip::StripError::Connect(repo, err)) => {
            eprintln!("{}", copy::connect_failed(repo.as_deref(), &err));
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("{}", copy::window_failed(&err));
            ExitCode::FAILURE
        }
    }
}
