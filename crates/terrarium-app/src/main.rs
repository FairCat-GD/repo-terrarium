#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod cli;
mod copy;
mod window;

use std::process::ExitCode;

use cli::Command;

/// The conventional exit status for a command line the program does not accept.
const USAGE_EXIT: u8 = 2;

fn main() -> ExitCode {
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(Command::Run) => match window::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("{}", copy::window_failed(&err));
                ExitCode::FAILURE
            }
        },
        Ok(Command::Version) => {
            println!("{}", copy::version_line());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{}", copy::usage_error(&err));
            ExitCode::from(USAGE_EXIT)
        }
    }
}
