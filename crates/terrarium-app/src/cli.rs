//! The command line, parsed by hand: a few commands do not justify a parser dependency.

use std::ffi::OsString;

/// What the process was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// No arguments: run the strip.
    Run,
    /// `--version`: print the version and exit.
    Version,
    /// `--probe <kind>`: run the strip with a recorder and write a report.
    Probe(ProbeKind),
    /// `--probe-readme`: print the Windows probe's tester instructions, which
    /// `cargo xtask probe-zip` puts in the zip beside the exe.
    ProbeReadme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    /// Window level, Spaces, hover, the hotkey and activation.
    Pinning,
}

/// A command line the program does not accept.
#[derive(Debug, PartialEq, Eq)]
pub enum UsageError {
    Unexpected(OsString),
    MissingProbeKind,
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
    let command = if first == "--version" {
        Command::Version
    } else if first == "--probe-readme" {
        Command::ProbeReadme
    } else if first == "--probe" {
        match args.next() {
            None => return Err(UsageError::MissingProbeKind),
            Some(kind) if kind == "pinning" => Command::Probe(ProbeKind::Pinning),
            Some(kind) => return Err(UsageError::Unexpected(kind)),
        }
    } else {
        return Err(UsageError::Unexpected(first));
    };
    match args.next() {
        None => Ok(command),
        Some(extra) => Err(UsageError::Unexpected(extra)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<Command, UsageError> {
        parse(args.iter().map(OsString::from))
    }

    fn unexpected(arg: &str) -> Result<Command, UsageError> {
        Err(UsageError::Unexpected(arg.into()))
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

    #[test]
    fn the_pinning_probe_is_accepted() {
        assert_eq!(
            parse_strs(&["--probe", "pinning"]),
            Ok(Command::Probe(ProbeKind::Pinning))
        );
    }

    #[test]
    fn the_probe_readme_is_a_command_of_its_own() {
        assert_eq!(parse_strs(&["--probe-readme"]), Ok(Command::ProbeReadme));
        assert_eq!(parse_strs(&["--probe-readme", "x"]), unexpected("x"));
    }

    #[test]
    fn a_probe_needs_a_known_kind_and_nothing_after_it() {
        assert_eq!(parse_strs(&["--probe"]), Err(UsageError::MissingProbeKind));
        assert_eq!(parse_strs(&["--probe", "fps"]), unexpected("fps"));
        assert_eq!(parse_strs(&["--probe", "pinning", "x"]), unexpected("x"));
    }
}
