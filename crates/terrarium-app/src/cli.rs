//! The command line, parsed by hand: a few commands do not justify a parser dependency.

use std::ffi::OsString;
use std::path::PathBuf;

/// What the process was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// No arguments: run the strip.
    Run,
    /// `--demo`: run the strip through the synthetic climate's states, with no repository.
    Demo,
    /// `--version`: print the version and exit.
    Version,
    /// `--probe <kind>`: run the strip with a recorder and write a report.
    Probe(ProbeKind),
    /// `--probe-readme`: print the Windows probe's tester instructions, which
    /// `cargo xtask probe-zip` puts in the zip beside the exe.
    ProbeReadme,
    /// `replay <path> --dump`: replay a repository with no window and print truth's canonical
    /// dump.
    Replay(ReplayArgs),
    /// `discover <path>`: print what the repository has for each adapter.
    Discover(PathBuf),
    /// `record-tests -- <command>`: run a test command and record how it ended.
    RecordTests(RecordTestsArgs),
}

#[derive(Debug, PartialEq, Eq)]
pub struct RecordTestsArgs {
    pub program: OsString,
    /// Passed to the program exactly as given.
    pub args: Vec<OsString>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ReplayArgs {
    pub repo: PathBuf,
    /// `--observations <file>`: an observations export to replay with.
    pub observations: Option<PathBuf>,
    /// `--at <unix-seconds>`: the moment to replay through; now when absent.
    pub at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    /// Window level, Spaces, hover, the hotkey and activation.
    Pinning,
    /// The memory an egui window costs, opened and closed again (dev builds).
    #[cfg(feature = "dev")]
    Egui,
}

/// A command line the program does not accept.
#[derive(Debug, PartialEq, Eq)]
pub enum UsageError {
    Unexpected(OsString),
    MissingProbeKind,
    MissingReplayRepo,
    MissingDiscoverRepo,
    /// `record-tests` without `--` and a command after it.
    MissingCommand,
    /// `replay` has one output so far, the dump, and is asked for it by name.
    MissingDump,
    /// A flag given without its value.
    MissingValue(&'static str),
    /// An `--at` that is not a whole number of seconds.
    BadTime(OsString),
}

/// macOS `LaunchServices` can append a process serial number (`-psn_0_12345`) when it launches
/// an app bundle. It is not the user's argument, so it never makes the app refuse to start. After
/// `--`, every argument is the user's command's, and passes as given.
const PROCESS_SERIAL_NUMBER: &str = "-psn_";

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, UsageError> {
    let all: Vec<OsString> = args.into_iter().collect();
    let separator = all.iter().position(|arg| arg == "--").unwrap_or(all.len());
    let (own, theirs) = all.split_at(separator);
    let mut args = own
        .iter()
        .filter(|arg| {
            !arg.to_str()
                .is_some_and(|arg| arg.starts_with(PROCESS_SERIAL_NUMBER))
        })
        .chain(theirs)
        .cloned();
    let Some(first) = args.next() else {
        return Ok(Command::Run);
    };
    let command = if first == "--version" {
        Command::Version
    } else if first == "--demo" {
        Command::Demo
    } else if first == "--probe-readme" {
        Command::ProbeReadme
    } else if first == "replay" {
        return parse_replay(&mut args);
    } else if first == "discover" {
        let repo = args
            .next()
            .filter(|repo| !repo.to_string_lossy().starts_with("--"))
            .ok_or(UsageError::MissingDiscoverRepo)?;
        Command::Discover(PathBuf::from(repo))
    } else if first == "record-tests" {
        if args.next().is_none_or(|separator| separator != "--") {
            return Err(UsageError::MissingCommand);
        }
        let program = args.next().ok_or(UsageError::MissingCommand)?;
        return Ok(Command::RecordTests(RecordTestsArgs {
            program,
            args: args.collect(),
        }));
    } else if first == "--probe" {
        match args.next() {
            None => return Err(UsageError::MissingProbeKind),
            Some(kind) if kind == "pinning" => Command::Probe(ProbeKind::Pinning),
            #[cfg(feature = "dev")]
            Some(kind) if kind == "egui" => Command::Probe(ProbeKind::Egui),
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

fn parse_replay(args: &mut impl Iterator<Item = OsString>) -> Result<Command, UsageError> {
    let repo = args
        .next()
        .filter(|repo| !repo.to_string_lossy().starts_with("--"))
        .ok_or(UsageError::MissingReplayRepo)?;
    let (mut dump, mut observations, mut at) = (false, None, None);
    while let Some(arg) = args.next() {
        if arg == "--dump" && !dump {
            dump = true;
        } else if arg == "--observations" && observations.is_none() {
            let path = args
                .next()
                .ok_or(UsageError::MissingValue("--observations"))?;
            observations = Some(PathBuf::from(path));
        } else if arg == "--at" && at.is_none() {
            let value = args.next().ok_or(UsageError::MissingValue("--at"))?;
            let secs = value.to_str().and_then(|text| text.parse().ok());
            at = Some(secs.ok_or(UsageError::BadTime(value))?);
        } else {
            return Err(UsageError::Unexpected(arg));
        }
    }
    if !dump {
        return Err(UsageError::MissingDump);
    }
    Ok(Command::Replay(ReplayArgs {
        repo: PathBuf::from(repo),
        observations,
        at,
    }))
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
    fn the_demo_runs_the_strip_on_its_own_and_takes_nothing_after_it() {
        assert_eq!(parse_strs(&["--demo"]), Ok(Command::Demo));
        assert_eq!(parse_strs(&["--demo", "x"]), unexpected("x"));
    }

    #[test]
    fn the_egui_probe_is_a_dev_build_s_alone() {
        let parsed = parse_strs(&["--probe", "egui"]);
        #[cfg(feature = "dev")]
        assert_eq!(parsed, Ok(Command::Probe(ProbeKind::Egui)));
        #[cfg(not(feature = "dev"))]
        assert_eq!(parsed, unexpected("egui"));
    }

    #[test]
    fn the_probe_readme_is_a_command_of_its_own() {
        assert_eq!(parse_strs(&["--probe-readme"]), Ok(Command::ProbeReadme));
        assert_eq!(parse_strs(&["--probe-readme", "x"]), unexpected("x"));
    }

    #[test]
    fn replay_takes_a_repository_the_dump_and_two_options() {
        let replay = |observations: Option<&str>, at| {
            Ok(Command::Replay(ReplayArgs {
                repo: PathBuf::from("moss"),
                observations: observations.map(PathBuf::from),
                at,
            }))
        };
        assert_eq!(
            parse_strs(&["replay", "moss", "--dump"]),
            replay(None, None)
        );
        assert_eq!(
            parse_strs(&[
                "replay",
                "moss",
                "--at",
                "1700000000",
                "--dump",
                "--observations",
                "fern.jsonl"
            ]),
            replay(Some("fern.jsonl"), Some(1_700_000_000))
        );
        assert_eq!(
            parse_strs(&["replay", "moss", "--at", "-5", "--dump"]),
            replay(None, Some(-5))
        );
    }

    #[test]
    fn replay_refuses_what_it_does_not_take() {
        assert_eq!(parse_strs(&["replay"]), Err(UsageError::MissingReplayRepo));
        assert_eq!(
            parse_strs(&["replay", "--dump"]),
            Err(UsageError::MissingReplayRepo)
        );
        assert_eq!(
            parse_strs(&["replay", "moss"]),
            Err(UsageError::MissingDump)
        );
        assert_eq!(
            parse_strs(&["replay", "moss", "--dump", "--at"]),
            Err(UsageError::MissingValue("--at"))
        );
        assert_eq!(
            parse_strs(&["replay", "moss", "--dump", "--observations"]),
            Err(UsageError::MissingValue("--observations"))
        );
        assert_eq!(
            parse_strs(&["replay", "moss", "--dump", "--at", "noon"]),
            Err(UsageError::BadTime("noon".into()))
        );
        assert_eq!(
            parse_strs(&["replay", "moss", "--dump", "--dump"]),
            unexpected("--dump")
        );
        assert_eq!(
            parse_strs(&["replay", "moss", "--dump", "x"]),
            unexpected("x")
        );
    }

    #[test]
    fn discover_takes_one_repository() {
        assert_eq!(
            parse_strs(&["discover", "moss"]),
            Ok(Command::Discover(PathBuf::from("moss")))
        );
        assert_eq!(
            parse_strs(&["discover"]),
            Err(UsageError::MissingDiscoverRepo)
        );
        assert_eq!(
            parse_strs(&["discover", "--dump"]),
            Err(UsageError::MissingDiscoverRepo)
        );
        assert_eq!(
            parse_strs(&["discover", "moss", "fern"]),
            unexpected("fern")
        );
    }

    #[test]
    fn record_tests_passes_everything_after_the_separator_as_given() {
        assert_eq!(
            parse_strs(&[
                "record-tests",
                "--",
                "cargo",
                "test",
                "--",
                "-psn_kept",
                "--version"
            ]),
            Ok(Command::RecordTests(RecordTestsArgs {
                program: "cargo".into(),
                args: ["test", "--", "-psn_kept", "--version"]
                    .map(OsString::from)
                    .to_vec(),
            }))
        );
        assert_eq!(
            parse_strs(&["-psn_0_1", "record-tests", "--", "npm"]),
            Ok(Command::RecordTests(RecordTestsArgs {
                program: "npm".into(),
                args: Vec::new(),
            }))
        );
        for refused in [
            &["record-tests"][..],
            &["record-tests", "--"],
            &["record-tests", "npm", "test"],
        ] {
            assert_eq!(
                parse_strs(refused),
                Err(UsageError::MissingCommand),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn a_probe_needs_a_known_kind_and_nothing_after_it() {
        assert_eq!(parse_strs(&["--probe"]), Err(UsageError::MissingProbeKind));
        assert_eq!(parse_strs(&["--probe", "fps"]), unexpected("fps"));
        assert_eq!(parse_strs(&["--probe", "pinning", "x"]), unexpected("x"));
    }
}
