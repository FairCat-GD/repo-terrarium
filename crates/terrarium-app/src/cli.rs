//! The command line, parsed by hand: a few commands do not justify a parser dependency.

use std::ffi::OsString;
use std::path::PathBuf;

use terrarium_platform::geometry::Edge;

use crate::present::Backend;

/// What the process was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Run the strip: on the repository `--repo <path>` names, connecting it, or with no
    /// arguments on the one connected last.
    Run(Option<PathBuf>),
    /// `--demo`: run the strip through the synthetic climate's states, with no repository.
    Demo,
    /// `--version`: print the version and exit.
    Version,
    /// `--probe <kind>`: run the strip with a recorder and write a report.
    Probe(ProbeKind),
    /// `--probe soak`: run the strip for a while, measuring what it costs, and write a report
    /// (N-01..N-03).
    Soak(SoakArgs),
    /// `--probe-readme`: print the Windows probe's tester instructions, which
    /// `cargo xtask probe-zip` puts in the zip beside the exe.
    ProbeReadme,
    /// `--package-copy`: print the words the installers carry, which `cargo xtask package`
    /// puts in them.
    PackageCopy,
    /// `--quit`: ask the running app to quit, and wait for it; the Windows installer's first
    /// step, and its uninstaller's.
    Quit,
    /// `replay <path> --dump`: replay a repository with no window and print truth's canonical
    /// dump; `--gif <file>`: write its whole history as a time-lapse instead.
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
    pub output: ReplayOutput,
}

/// What a replay writes.
#[derive(Debug, PartialEq, Eq)]
pub enum ReplayOutput {
    /// `--dump`: truth's canonical dump, on stdout.
    Dump,
    /// `--gif <file>`: the whole history as a time-lapse, `--night` by night and `--scale <n>`
    /// times the scene's size.
    Gif {
        path: PathBuf,
        night: bool,
        scale: u32,
    },
    /// `--mp4 <file>`: the same time-lapse as an MP4, through the person's ffmpeg, at
    /// `--scale <n>` or 4×, whichever is larger.
    Mp4 {
        path: PathBuf,
        night: bool,
        scale: u32,
    },
}

/// The largest scale `--scale` takes.
pub const MAX_SCALE: u32 = 8;

#[derive(Debug, PartialEq, Eq)]
pub struct SoakArgs {
    /// `--minutes <n>`: how long it runs; an hour when absent.
    pub minutes: u32,
    /// `--presenter <name>`: how the strip is put on screen; the default when absent.
    pub presenter: Option<Backend>,
    /// `--edge <top|bottom>`: the edge the strip runs on; the bottom when absent.
    pub edge: Option<Edge>,
    /// `--repo <path>`: the repository it runs on, connected in a data directory the soak
    /// owns; the thriving sample, with no repository, when absent.
    pub repo: Option<PathBuf>,
}

/// A soak's length when none is asked for: N-01's hour.
pub const SOAK_MINUTES: u32 = 60;
/// The longest soak: a day.
pub const MAX_SOAK_MINUTES: u32 = 24 * 60;

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
    /// `replay` without exactly one of its outputs, `--dump`, `--gif` or `--mp4`, asked for
    /// by name.
    MissingOutput,
    /// A flag of the time-lapse's, `--gif`'s or `--mp4`'s, given without either.
    GifOnly(&'static str),
    /// A `--scale` that is not a whole number from 1 to [`MAX_SCALE`].
    BadScale(OsString),
    /// A flag given without its value.
    MissingValue(&'static str),
    /// An `--at` that is not a whole number of seconds.
    BadTime(OsString),
    /// A `--minutes` that is not a whole number from 1 to [`MAX_SOAK_MINUTES`].
    BadMinutes(OsString),
    /// A `--presenter` that names no presenter.
    BadPresenter(OsString),
    /// An `--edge` that is neither `top` nor `bottom`.
    BadEdge(OsString),
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
        return Ok(Command::Run(None));
    };
    let command = if first == "--repo" {
        let repo = args
            .next()
            .filter(|repo| !repo.to_string_lossy().starts_with("--"))
            .ok_or(UsageError::MissingValue("--repo"))?;
        Command::Run(Some(PathBuf::from(repo)))
    } else if first == "--version" {
        Command::Version
    } else if first == "--demo" {
        Command::Demo
    } else if first == "--probe-readme" {
        Command::ProbeReadme
    } else if first == "--package-copy" {
        Command::PackageCopy
    } else if first == "--quit" {
        Command::Quit
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
            Some(kind) if kind == "soak" => return parse_soak(&mut args),
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

fn parse_soak(args: &mut impl Iterator<Item = OsString>) -> Result<Command, UsageError> {
    let (mut minutes, mut presenter, mut edge, mut repo) = (None, None, None, None);
    while let Some(arg) = args.next() {
        if arg == "--minutes" && minutes.is_none() {
            let value = args.next().ok_or(UsageError::MissingValue("--minutes"))?;
            let n = value
                .to_str()
                .and_then(|text| text.parse().ok())
                .filter(|n| (1..=MAX_SOAK_MINUTES).contains(n));
            minutes = Some(n.ok_or(UsageError::BadMinutes(value))?);
        } else if arg == "--presenter" && presenter.is_none() {
            let value = args.next().ok_or(UsageError::MissingValue("--presenter"))?;
            presenter = Some(match value.to_str() {
                Some("softbuffer") => Backend::Softbuffer,
                Some("wgpu") => Backend::Wgpu,
                Some("layer") => Backend::Layer,
                _ => return Err(UsageError::BadPresenter(value)),
            });
        } else if arg == "--edge" && edge.is_none() {
            let value = args.next().ok_or(UsageError::MissingValue("--edge"))?;
            edge = Some(match value.to_str() {
                Some("top") => Edge::Top,
                Some("bottom") => Edge::Bottom,
                _ => return Err(UsageError::BadEdge(value)),
            });
        } else if arg == "--repo" && repo.is_none() {
            let path = args
                .next()
                .filter(|repo| !repo.to_string_lossy().starts_with("--"))
                .ok_or(UsageError::MissingValue("--repo"))?;
            repo = Some(PathBuf::from(path));
        } else {
            return Err(UsageError::Unexpected(arg));
        }
    }
    Ok(Command::Soak(SoakArgs {
        minutes: minutes.unwrap_or(SOAK_MINUTES),
        presenter,
        edge,
        repo,
    }))
}

fn parse_replay(args: &mut impl Iterator<Item = OsString>) -> Result<Command, UsageError> {
    let repo = args
        .next()
        .filter(|repo| !repo.to_string_lossy().starts_with("--"))
        .ok_or(UsageError::MissingReplayRepo)?;
    let (mut dump, mut gif, mut mp4, mut observations, mut at) = (false, None, None, None, None);
    let (mut night, mut scale) = (false, None);
    while let Some(arg) = args.next() {
        if arg == "--dump" && !dump {
            dump = true;
        } else if arg == "--gif" && gif.is_none() {
            let path = args.next().ok_or(UsageError::MissingValue("--gif"))?;
            gif = Some(PathBuf::from(path));
        } else if arg == "--mp4" && mp4.is_none() {
            let path = args.next().ok_or(UsageError::MissingValue("--mp4"))?;
            mp4 = Some(PathBuf::from(path));
        } else if arg == "--night" && !night {
            night = true;
        } else if arg == "--scale" && scale.is_none() {
            let value = args.next().ok_or(UsageError::MissingValue("--scale"))?;
            let k = value
                .to_str()
                .and_then(|text| text.parse().ok())
                .filter(|k| (1..=MAX_SCALE).contains(k));
            scale = Some(k.ok_or(UsageError::BadScale(value))?);
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
    let output = match (dump, gif, mp4) {
        (true, None, None) => {
            if night {
                return Err(UsageError::GifOnly("--night"));
            }
            if scale.is_some() {
                return Err(UsageError::GifOnly("--scale"));
            }
            ReplayOutput::Dump
        }
        (false, Some(path), None) => ReplayOutput::Gif {
            path,
            night,
            scale: scale.unwrap_or(1),
        },
        (false, None, Some(path)) => ReplayOutput::Mp4 {
            path,
            night,
            scale: scale.unwrap_or(1),
        },
        _ => return Err(UsageError::MissingOutput),
    };
    Ok(Command::Replay(ReplayArgs {
        repo: PathBuf::from(repo),
        observations,
        at,
        output,
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
        assert_eq!(parse_strs(&[]), Ok(Command::Run(None)));
    }

    #[test]
    fn repo_connects_the_repository_it_names_and_takes_nothing_after_it() {
        assert_eq!(
            parse_strs(&["--repo", "moss"]),
            Ok(Command::Run(Some(PathBuf::from("moss"))))
        );
        assert_eq!(
            parse_strs(&["--repo"]),
            Err(UsageError::MissingValue("--repo"))
        );
        assert_eq!(
            parse_strs(&["--repo", "--demo"]),
            Err(UsageError::MissingValue("--repo"))
        );
        assert_eq!(parse_strs(&["--repo", "moss", "x"]), unexpected("x"));
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
        assert_eq!(parse_strs(&["-psn_0_12345"]), Ok(Command::Run(None)));
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
    fn the_soak_runs_an_hour_by_default_and_takes_its_four_options() {
        assert_eq!(
            parse_strs(&["--probe", "soak"]),
            Ok(Command::Soak(SoakArgs {
                minutes: SOAK_MINUTES,
                presenter: None,
                edge: None,
                repo: None,
            }))
        );
        assert_eq!(
            parse_strs(&[
                "--probe",
                "soak",
                "--repo",
                "moss",
                "--minutes",
                "10",
                "--edge",
                "top",
                "--presenter",
                "wgpu",
            ]),
            Ok(Command::Soak(SoakArgs {
                minutes: 10,
                presenter: Some(Backend::Wgpu),
                edge: Some(Edge::Top),
                repo: Some(PathBuf::from("moss")),
            }))
        );
        assert_eq!(
            parse_strs(&[
                "--probe",
                "soak",
                "--presenter",
                "softbuffer",
                "--edge",
                "bottom"
            ]),
            Ok(Command::Soak(SoakArgs {
                minutes: SOAK_MINUTES,
                presenter: Some(Backend::Softbuffer),
                edge: Some(Edge::Bottom),
                repo: None,
            }))
        );
    }

    #[test]
    fn a_soak_refuses_what_it_cannot_run() {
        let soak = |rest: &[&str]| {
            let mut args = vec!["--probe", "soak"];
            args.extend_from_slice(rest);
            parse_strs(&args)
        };
        for minutes in ["0", "1441", "an hour", "-5"] {
            assert_eq!(
                soak(&["--minutes", minutes]),
                Err(UsageError::BadMinutes(minutes.into()))
            );
        }
        assert_eq!(soak(&["--minutes", "1440"]).map(|_| ()), Ok(()));
        assert_eq!(
            soak(&["--presenter", "gl"]),
            Err(UsageError::BadPresenter("gl".into()))
        );
        assert_eq!(
            soak(&["--edge", "left"]),
            Err(UsageError::BadEdge("left".into()))
        );
        assert_eq!(
            soak(&["--minutes"]),
            Err(UsageError::MissingValue("--minutes"))
        );
        assert_eq!(soak(&["--repo"]), Err(UsageError::MissingValue("--repo")));
        assert_eq!(
            soak(&["--repo", "--edge"]),
            Err(UsageError::MissingValue("--repo"))
        );
        assert_eq!(
            soak(&["--minutes", "5", "--minutes", "6"]),
            unexpected("--minutes")
        );
        assert_eq!(soak(&["--night"]), unexpected("--night"));
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
    fn the_installers_words_and_quitting_are_commands_of_their_own() {
        assert_eq!(parse_strs(&["--package-copy"]), Ok(Command::PackageCopy));
        assert_eq!(parse_strs(&["--package-copy", "x"]), unexpected("x"));
        assert_eq!(parse_strs(&["--quit"]), Ok(Command::Quit));
        assert_eq!(parse_strs(&["--quit", "--demo"]), unexpected("--demo"));
    }

    #[test]
    fn replay_takes_a_repository_the_dump_and_two_options() {
        let replay = |observations: Option<&str>, at| {
            Ok(Command::Replay(ReplayArgs {
                repo: PathBuf::from("moss"),
                observations: observations.map(PathBuf::from),
                at,
                output: ReplayOutput::Dump,
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
            Err(UsageError::MissingOutput)
        );
        assert_eq!(
            parse_strs(&["replay", "moss", "--dump", "--gif", "fern.gif"]),
            Err(UsageError::MissingOutput)
        );
        assert_eq!(
            parse_strs(&["replay", "moss", "--dump", "--night"]),
            Err(UsageError::GifOnly("--night"))
        );
        assert_eq!(
            parse_strs(&["replay", "moss", "--dump", "--scale", "2"]),
            Err(UsageError::GifOnly("--scale"))
        );
        for bad in ["0", "9", "two", "-1"] {
            assert_eq!(
                parse_strs(&["replay", "moss", "--gif", "fern.gif", "--scale", bad]),
                Err(UsageError::BadScale(bad.into()))
            );
        }
        assert_eq!(
            parse_strs(&["replay", "moss", "--gif"]),
            Err(UsageError::MissingValue("--gif"))
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
    fn replay_writes_an_mp4_alone_by_day_or_night_at_a_scale() {
        assert_eq!(
            parse_strs(&[
                "replay", "moss", "--mp4", "fern.mp4", "--night", "--scale", "6"
            ]),
            Ok(Command::Replay(ReplayArgs {
                repo: PathBuf::from("moss"),
                observations: None,
                at: None,
                output: ReplayOutput::Mp4 {
                    path: PathBuf::from("fern.mp4"),
                    night: true,
                    scale: 6,
                },
            }))
        );
        for both in [
            &["replay", "moss", "--gif", "fern.gif", "--mp4", "fern.mp4"][..],
            &["replay", "moss", "--dump", "--mp4", "fern.mp4"][..],
        ] {
            assert_eq!(parse_strs(both), Err(UsageError::MissingOutput), "{both:?}");
        }
        assert_eq!(
            parse_strs(&["replay", "moss", "--mp4"]),
            Err(UsageError::MissingValue("--mp4"))
        );
        assert_eq!(
            parse_strs(&["replay", "moss", "--mp4", "a.mp4", "--mp4", "b.mp4"]),
            unexpected("--mp4")
        );
    }

    #[test]
    fn replay_writes_a_gif_by_day_or_night_at_a_scale() {
        let gif = |night, scale| {
            Ok(Command::Replay(ReplayArgs {
                repo: PathBuf::from("moss"),
                observations: Some(PathBuf::from("fern.jsonl")),
                at: None,
                output: ReplayOutput::Gif {
                    path: PathBuf::from("fern.gif"),
                    night,
                    scale,
                },
            }))
        };
        assert_eq!(
            parse_strs(&[
                "replay",
                "moss",
                "--gif",
                "fern.gif",
                "--observations",
                "fern.jsonl"
            ]),
            gif(false, 1)
        );
        assert_eq!(
            parse_strs(&[
                "replay",
                "moss",
                "--scale",
                "8",
                "--night",
                "--observations",
                "fern.jsonl",
                "--gif",
                "fern.gif"
            ]),
            gif(true, 8)
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
