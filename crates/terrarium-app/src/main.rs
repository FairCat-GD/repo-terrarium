#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod animate;
mod cli;
mod connect;
mod copy;
mod crash;
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
mod files;
mod gpu;
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
mod picture;
mod pinning;
mod portability;
mod prefs;
mod present;
mod probe;
mod raise;
mod record_tests;
mod redraw;
mod repaint;
mod replay;
#[cfg(feature = "dev")]
mod sandbox;
mod settings;
mod shell;
mod soil;
mod strip;
mod timelapse;

use std::process::ExitCode;

use cli::Command;
use feed::Feed;
use strip::Mode;

/// The conventional exit status for a command line the program does not accept.
const USAGE_EXIT: u8 = 2;

fn main() -> ExitCode {
    match cli::parse(std::env::args_os().skip(1)) {
        // The strip lets go of a console Windows opened for this launch alone (Explorer, the
        // Start menu, the login item); one shared with a terminal stays.
        Ok(Command::Run(repo)) => {
            terrarium_platform::console::release_if_alone();
            live::run(repo)
        }
        Ok(Command::Demo) => {
            terrarium_platform::console::release_if_alone();
            run(&Mode::Live(Feed::Demo), None)
        }
        Ok(Command::Probe(kind)) => run(&Mode::Probe(kind), None),
        Ok(Command::Soak(args)) => probe::soak::run(args),
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

pub(crate) fn run(mode: &Mode, launch: Option<live::Launch>) -> ExitCode {
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
