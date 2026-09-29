//! The command line. Parsed by hand while it has two commands.

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

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, UsageError> {
    let mut args = args.into_iter();
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
    fn an_argument_after_version_is_refused() {
        assert_eq!(parse_strs(&["--version", "extra"]), unexpected("extra"));
    }
}
