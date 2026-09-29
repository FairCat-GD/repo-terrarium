//! The command line, parsed by hand: two commands do not justify a parser dependency.

use std::ffi::OsString;

/// What the process was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// No arguments: open the window.
    Run,
    /// `--version`: print the version and exit.
    Version,
}

/// An argument the command line does not accept.
#[derive(Debug, PartialEq, Eq)]
pub struct UsageError {
    pub unexpected: OsString,
}

/// macOS `LaunchServices` can append a process serial number (`-psn_0_12345`) when it launches
/// an app bundle. It is not the user's argument, so it never makes the app refuse to start.
const PROCESS_SERIAL_NUMBER: &str = "-psn_";

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, UsageError> {
    let mut args = args.into_iter().filter(|arg| {
        !arg.to_str()
            .is_some_and(|arg| arg.starts_with(PROCESS_SERIAL_NUMBER))
    });
    let Some(first) = args.next() else {
        return Ok(Command::Run);
    };
    if first != "--version" {
        return Err(UsageError { unexpected: first });
    }
    match args.next() {
        None => Ok(Command::Version),
        Some(extra) => Err(UsageError { unexpected: extra }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<Command, UsageError> {
        parse(args.iter().map(OsString::from))
    }

    fn unexpected(arg: &str) -> Result<Command, UsageError> {
        Err(UsageError {
            unexpected: arg.into(),
        })
    }

    #[test]
    fn no_arguments_runs_the_app() {
        assert_eq!(parse_strs(&[]), Ok(Command::Run));
    }

    #[test]
    fn version_flag_prints_the_version() {
        assert_eq!(parse_strs(&["--version"]), Ok(Command::Version));
    }

    #[test]
    fn an_unknown_argument_is_refused() {
        assert_eq!(parse_strs(&["--bogus"]), unexpected("--bogus"));
        assert_eq!(parse_strs(&["-V"]), unexpected("-V"));
    }

    #[test]
    fn a_launcher_process_serial_number_is_ignored() {
        assert_eq!(parse_strs(&["-psn_0_12345"]), Ok(Command::Run));
        assert_eq!(
            parse_strs(&["-psn_0_12345", "--version"]),
            Ok(Command::Version)
        );
    }

    #[test]
    fn an_argument_after_version_is_refused() {
        assert_eq!(parse_strs(&["--version", "extra"]), unexpected("extra"));
    }
}
