#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod animate;
mod cli;
mod copy;
#[cfg(feature = "dev")]
mod debug;
mod demo;
#[cfg(feature = "dev")]
mod dev;
mod discover;
#[cfg(feature = "dev")]
mod egui_host;
#[cfg(feature = "dev")]
mod egui_probe;
mod feed;
mod hotkey;
mod hover;
mod idle;
mod menu;
mod monitor;
mod output;
mod pinning;
mod present;
mod probe;
mod raise;
mod record_tests;
#[cfg(feature = "dev")]
mod repaint;
mod replay;
mod strip;

use std::process::ExitCode;

use cli::Command;
use feed::Feed;
use strip::Mode;

/// The conventional exit status for a command line the program does not accept.
const USAGE_EXIT: u8 = 2;

fn main() -> ExitCode {
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(Command::Run) => run(&Mode::Live(Feed::Thriving)),
        Ok(Command::Demo) => run(&Mode::Live(Feed::Demo)),
        Ok(Command::Probe(kind)) => run(&Mode::Probe(kind)),
        Ok(Command::Version) => {
            println!("{}", copy::version_line());
            ExitCode::SUCCESS
        }
        Ok(Command::ProbeReadme) => {
            print!("{}", copy::PROBE_README);
            ExitCode::SUCCESS
        }
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

fn run(mode: &Mode) -> ExitCode {
    match strip::run(mode) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{}", copy::window_failed(&err));
            ExitCode::FAILURE
        }
    }
}
