//! Every user-facing string lives here (I-8). The world never talks: no praise, shame, nagging,
//! instructions, notifications or exclamation marks. The probe is a tester's tool, so its steps
//! do give instructions, in the same plain tone.

use std::ffi::OsStr;
use std::fmt::{self, Display, Write as _};
use std::path::Path;

use terrarium_adapters::discovery::{
    COBERTURA_PATHS, Discovery, KNIP_REPORT_PATH, LCOV_PATHS, Located, Runner, Search,
};
use terrarium_adapters::git::GitError;
use terrarium_adapters::report::{
    Format, KNIP_MIN_MAJOR, KnipCounts, Outcome, Reading, Report as AdapterReport, Why,
};
use terrarium_engine::headless::HeadlessError;
use terrarium_truth::ReplayError;

use crate::cli::UsageError;
use crate::discover::Reads;
use crate::hover::NativePoint;
use crate::pinning::{AppBarPlaced, AttachError, LevelMode, OsSnapshot, PxRect};
use crate::present::PresentError;
use crate::probe::report::{Activity, EndReason, MoveSummary, ProbeEvent, Report, StepReport};
use crate::probe::script::{Phase, Step};
use crate::strip::StripError;

const PRODUCT_NAME: &str = "Repo Terrarium";
/// The binary's name, a machine identifier that never changes.
const BINARY_NAME: &str = terrarium_store::identifiers::BINARY;
/// The single source of the product version: the workspace manifest.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The `--version` output.
pub fn version_line() -> String {
    format!("{BINARY_NAME} {VERSION}")
}

/// What the sample scene's label reads: the binary's name.
pub fn sample_label() -> &'static str {
    BINARY_NAME
}

pub fn window_title() -> String {
    format!("{PRODUCT_NAME} {VERSION}")
}

pub fn usage_error(err: &UsageError) -> String {
    let problem = match err {
        UsageError::Unexpected(arg) => {
            format!("unexpected argument '{}'", arg.to_string_lossy())
        }
        UsageError::MissingProbeKind => {
            format!("--probe needs a kind: {}", PROBE_KINDS.join(" or "))
        }
        UsageError::MissingReplayRepo => "replay needs a repository path".to_owned(),
        UsageError::MissingDiscoverRepo => "discover needs a repository path".to_owned(),
        UsageError::MissingCommand => "record-tests runs the command that follows --".to_owned(),
        UsageError::MissingDump => "replay prints its result only with --dump".to_owned(),
        UsageError::MissingValue(flag) => format!("{flag} needs a value"),
        UsageError::BadTime(value) => format!(
            "'{}' is not a time in whole Unix seconds",
            value.to_string_lossy()
        ),
    };
    let probes: Vec<String> = PROBE_KINDS
        .iter()
        .map(|kind| format!("--probe {kind}"))
        .collect();
    format!(
        "{BINARY_NAME}: {problem}\nusage: {BINARY_NAME} [--version | --demo | {} | \
         --probe-readme]\n       {BINARY_NAME} replay <path> --dump [--observations <file>] \
         [--at <unix-seconds>]\n       {BINARY_NAME} discover <path>\n       {BINARY_NAME} \
         record-tests -- <command> [<argument>...]",
        probes.join(" | ")
    )
}

/// Why a headless replay printed nothing.
pub fn replay_failed(repo: &Path, err: &HeadlessError) -> String {
    let repo = repo.display();
    let problem = match err {
        HeadlessError::Git(err) => format!("{repo} could not be read as a git repository: {err}"),
        HeadlessError::Store(err) => format!("the temporary store failed: {err}"),
        HeadlessError::Import(err) => format!("the observations could not be imported: {err}"),
        HeadlessError::Replay(ReplayError::CommitsOutOfOrder { commit }) => format!(
            "the history of {repo} came out of order at commit {}",
            commit.as_str()
        ),
        HeadlessError::Replay(ReplayError::DuplicateObservation { recorded_at, kind }) => format!(
            "two {} observations share the time {recorded_at}, source and commit",
            kind.as_column()
        ),
        HeadlessError::Replay(ReplayError::BasisChanged | ReplayError::NeedsFullReplay) => {
            "the replay could not continue from its own state".to_owned()
        }
        HeadlessError::BeforeHistory => {
            format!("the time asked for is outside the history of {repo}")
        }
    };
    format!("{BINARY_NAME}: {problem}")
}

pub fn replay_observations_unreadable(path: &Path, err: &impl Display) -> String {
    format!("{BINARY_NAME}: {} could not be read: {err}", path.display())
}

pub fn replay_store_unavailable(err: &impl Display) -> String {
    format!("{BINARY_NAME}: no temporary directory for the replay's store: {err}")
}

pub fn replay_write_failed(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the dump could not be written: {err}")
}

pub fn discover_failed(repo: &Path, err: &GitError) -> String {
    format!(
        "{BINARY_NAME}: {} could not be read as a git repository: {err}",
        repo.display()
    )
}

pub fn discover_write_failed(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the report could not be written: {err}")
}

pub fn record_tests_no_directory(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the current directory could not be read, so nothing ran: {err}")
}

pub fn record_tests_no_repository(dir: &Path, err: &GitError) -> String {
    format!(
        "{BINARY_NAME}: record-tests records runs for the git repository it runs in, and {} \
         is not in one that can be read, so nothing ran: {err}",
        dir.display()
    )
}

pub fn record_tests_no_data_dir(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the data directory is unavailable, so nothing ran: {err}")
}

pub fn record_tests_not_found(program: &OsStr, err: &impl Display) -> String {
    format!(
        "{BINARY_NAME}: {}: command not found ({err})",
        program.to_string_lossy()
    )
}

pub fn record_tests_cannot_run(program: &OsStr, err: &impl Display) -> String {
    format!(
        "{BINARY_NAME}: {} could not be run: {err}",
        program.to_string_lossy()
    )
}

pub fn record_tests_not_recorded(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the run ended, but its result could not be recorded: {err}")
}

/// The width of the labels in the discovery report.
const LABEL_WIDTH: usize = 18;

/// The discovery report: what was found, what each report reads as, what was not found, and one
/// line per missing adapter on how it connects. Facts only; the lines on connecting belong to
/// the settings form of the report.
pub fn discovery_report(found: &Discovery, reads: &Reads) -> String {
    let mut out = String::from("Found\n");
    let mut missing: Vec<(&str, Option<String>, Hint)> = Vec::new();
    if let Ok(branch) = &found.git {
        row(&mut out, "git", &format!("default branch {branch}"));
    }
    match &found.coverage {
        Search::Found(located) => row(
            &mut out,
            "coverage report",
            &located_text(located, reads.coverage.as_ref()),
        ),
        Search::NotFound => missing.push(("coverage report", None, Hint::Coverage)),
        Search::Refused(path) => {
            missing.push(("coverage report", Some(refused(path)), Hint::Coverage));
        }
    }
    match &found.test_report {
        Search::Found(located) => row(
            &mut out,
            "test report",
            &located_text(located, reads.test_report.as_ref()),
        ),
        Search::NotFound if found.local_tests => {}
        Search::NotFound => missing.push(("test report", None, Hint::Tests)),
        Search::Refused(path) => missing.push(("test report", Some(refused(path)), Hint::Tests)),
    }
    if found.local_tests {
        let read = reads
            .local_tests
            .as_ref()
            .map_or_else(|| "recorded".to_owned(), outcome_text);
        row(
            &mut out,
            "test runs",
            &format!("recorded by record-tests: {read}"),
        );
    }
    match &found.dead_code {
        Search::Found(located) => row(
            &mut out,
            "dead-code report",
            &located_text(located, reads.dead_code.as_ref()),
        ),
        Search::NotFound => missing.push(("dead-code report", None, Hint::DeadCode)),
        Search::Refused(path) => {
            missing.push(("dead-code report", Some(refused(path)), Hint::DeadCode));
        }
    }
    let knip: Vec<String> = found
        .knip_config
        .iter()
        .cloned()
        .chain(
            found
                .knip_installed
                .iter()
                .map(|v| format!("Knip {v} installed")),
        )
        .collect();
    if !knip.is_empty() {
        row(&mut out, "Knip", &knip.join(", "));
    }
    if found.ci_workflows.is_empty() {
        missing.push(("CI workflow", None, Hint::Ci));
    } else {
        row(&mut out, "CI workflow", &found.ci_workflows.join(", "));
    }
    if !found.runners.is_empty() {
        let runners: Vec<String> = found
            .runners
            .iter()
            .map(|r| format!("{} ({})", runner_name(r.runner), r.config))
            .collect();
        row(&mut out, "test runner", &runners.join(", "));
    }
    if missing.is_empty() {
        return out;
    }
    out.push_str("Not found\n");
    for (label, detail, _) in &missing {
        row(&mut out, label, detail.as_deref().unwrap_or(""));
    }
    out.push_str("To connect\n");
    let runner = found.runners.first().map(|r| r.runner);
    for (label, _, hint) in &missing {
        row(&mut out, label, &hint_text(*hint, runner));
    }
    out
}

fn row(out: &mut String, label: &str, text: &str) {
    let line = format!("  {label:<LABEL_WIDTH$}{text}");
    let _ = writeln!(out, "{}", line.trim_end());
}

fn refused(path: &str) -> String {
    format!("the configured path {path} is outside the repository")
}

/// A report found: where, in what format, and what it reads as.
fn located_text(located: &Located, read: Option<&AdapterReport>) -> String {
    let format = format_name(located.format);
    let place = match located.files.as_slice() {
        [only] if *only == located.pattern => format!("{only}, {format}"),
        files => format!("{}, {} {format} files", located.pattern, files.len()),
    };
    match read {
        Some(report) => format!("{place}: {}", outcome_text(report)),
        None => place,
    }
}

fn outcome_text(report: &AdapterReport) -> String {
    match &report.outcome {
        Outcome::Known(known) => reading_text(&known.reading),
        Outcome::Unknown(why) => format!("not read, {}", why_text(report.format, why)),
    }
}

fn reading_text(reading: &Reading) -> String {
    match reading {
        Reading::Coverage {
            lines_found,
            lines_hit,
        } => format!(
            "{lines_hit} of {} covered",
            plural(*lines_found, "line", "lines")
        ),
        Reading::Tests {
            tests,
            failed,
            skipped,
        } => {
            let mut text = format!("{}, ", plural(*tests, "test", "tests"));
            if *failed == 0 {
                text.push_str("none failing");
            } else {
                let _ = write!(text, "{failed} failing");
            }
            if *skipped > 0 {
                let _ = write!(text, ", {skipped} skipped");
            }
            text
        }
        Reading::LocalRun { exit_code: 0 } => "the last run passed".to_owned(),
        Reading::LocalRun { exit_code } => {
            format!("the last run failed with exit code {exit_code}")
        }
        Reading::DeadCode(KnipCounts {
            unused_exports,
            unused_files,
            ..
        }) => format!(
            "{} and {}, not part of soil yet",
            plural(*unused_exports, "unused export", "unused exports"),
            plural(
                u64::try_from(unused_files.len()).unwrap_or(u64::MAX),
                "unused file",
                "unused files"
            ),
        ),
    }
}

fn why_text(format: Format, why: &Why) -> String {
    let name = format_name(format);
    match why {
        Why::Missing => "no file is there".to_owned(),
        Why::OutsideRepository(path) => format!("{path} is outside the repository"),
        Why::NotAFile(path) => format!("{path} is not a regular file"),
        Why::TooLarge(path) => format!("{path} is larger than a report is read"),
        Why::Io { path, kind } => format!("{path} could not be read ({kind})"),
        Why::Malformed {
            path,
            line: Some(line),
        } => format!("{path} is not a complete {name} report at line {line}"),
        Why::Malformed { path, line: None } => format!("{path} is not a complete {name} report"),
        Why::Empty => match format {
            Format::Lcov | Format::Cobertura => "it holds no instrumented line".to_owned(),
            Format::JUnit => "no test ran in it".to_owned(),
            Format::Knip | Format::LocalTests => "it holds nothing".to_owned(),
        },
        Why::KnipBefore6 { version } => {
            let from = version.as_ref().map_or_else(
                || format!("a Knip before {KNIP_MIN_MAJOR}"),
                |v| format!("Knip {v}"),
            );
            format!("it is from {from}, and reports from Knip {KNIP_MIN_MAJOR} on are read")
        }
    }
}

/// `a, b or c`.
fn either(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [only] => (*only).to_owned(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn format_name(format: Format) -> &'static str {
    match format {
        Format::Lcov => "lcov",
        Format::Cobertura => "Cobertura",
        Format::JUnit => "JUnit",
        Format::Knip => "Knip",
        Format::LocalTests => "record-tests",
    }
}

fn runner_name(runner: Runner) -> &'static str {
    match runner {
        Runner::Jest => "Jest",
        Runner::Vitest => "Vitest",
        Runner::Pytest => "pytest",
        Runner::Mocha => "Mocha",
        Runner::Maven => "Maven",
        Runner::Gradle => "Gradle",
        Runner::Nextest => "cargo-nextest",
        Runner::Cargo => "Cargo",
        Runner::Go => "Go",
    }
}

/// The adapters a discovery report can say are missing.
#[derive(Clone, Copy)]
enum Hint {
    Coverage,
    Tests,
    DeadCode,
    Ci,
}

/// How a missing adapter connects. For a test report it names the detected runner's own way,
/// when it has one.
fn hint_text(hint: Hint, runner: Option<Runner>) -> String {
    match hint {
        Hint::Coverage => format!(
            "lcov is read from {}; Cobertura XML from {}.",
            either(LCOV_PATHS),
            either(COBERTURA_PATHS)
        ),
        Hint::Tests => {
            let record = format!(
                "{BINARY_NAME} record-tests -- <command> records any test command's result."
            );
            match runner.and_then(junit_fact) {
                Some(fact) => format!("{fact} Or: {record}"),
                None => record,
            }
        }
        Hint::DeadCode => format!(
            "Knip prints its JSON report to stdout, and the report is read from \
             {KNIP_REPORT_PATH}: npx knip --reporter json --no-progress > {KNIP_REPORT_PATH}"
        ),
        Hint::Ci => "Weather follows GitHub Actions runs when .github/workflows holds a workflow."
            .to_owned(),
    }
}

/// Where a runner writes `JUnit` XML, for runners that can.
fn junit_fact(runner: Runner) -> Option<&'static str> {
    match runner {
        Runner::Jest => {
            Some("jest-junit writes JUnit XML to junit.xml as one of Jest's reporters.")
        }
        Runner::Vitest => {
            Some("Vitest's junit reporter writes JUnit XML to .vitest/junit/output.xml.")
        }
        Runner::Pytest => Some("pytest --junitxml=junit.xml writes JUnit XML."),
        Runner::Mocha => Some("mocha-junit-reporter writes JUnit XML to test-results.xml."),
        Runner::Maven => Some("mvn test writes JUnit XML to target/surefire-reports."),
        Runner::Gradle => Some("gradle test writes JUnit XML to build/test-results."),
        Runner::Nextest => Some(
            "cargo-nextest writes JUnit XML to target/nextest/<profile>/junit.xml when the \
             profile sets junit.path = \"junit.xml\".",
        ),
        Runner::Cargo | Runner::Go => None,
    }
}

/// The probes this build runs: the egui memory probe exists only in dev builds.
const PROBE_KINDS: &[&str] = if cfg!(feature = "dev") {
    &["pinning", "egui"]
} else {
    &["pinning"]
};

pub fn window_failed(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the strip could not open: {err}")
}

impl Display for StripError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(err) => write!(f, "the event loop failed: {err}"),
            Self::Window(err) => write!(f, "the window could not be created: {err}"),
            Self::Attach(err) => write!(f, "the window could not be pinned: {}", attach(err)),
            Self::Present(err) => write!(f, "{err}"),
            Self::NoDisplay => write!(f, "no display is connected"),
        }
    }
}

impl std::error::Error for StripError {}

#[cfg(target_os = "macos")]
fn attach(err: &AttachError) -> String {
    match err {
        AttachError::NotMainThread => "not on the main thread".to_owned(),
        AttachError::Handle(err) => format!("no window handle: {err}"),
        AttachError::NotAppKit => "the window handle is not an AppKit view".to_owned(),
        AttachError::NoWindow => "the view is not in a window".to_owned(),
    }
}

#[cfg(windows)]
fn attach(err: &AttachError) -> String {
    match err {
        AttachError::NotOwnerThread => "not on the window's own thread".to_owned(),
        AttachError::Handle(err) => format!("no window handle: {err}"),
        AttachError::NotWin32 => "the window handle is not a Win32 window".to_owned(),
        AttachError::Subclass => "the window could not be subclassed".to_owned(),
    }
}

impl Display for PresentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Surface(err) => write!(f, "the drawing surface failed: {err}"),
            Self::Size(size) => write!(
                f,
                "the drawing surface holds {} pixels, not {}",
                size.actual, size.expected
            ),
        }
    }
}

impl std::error::Error for PresentError {}

pub fn menu_quit() -> &'static str {
    "Quit"
}

pub fn menu_unavailable(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the right-click menu is unavailable: {err}")
}

pub fn hotkey_unavailable(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the raise hotkey Ctrl+Alt+T could not be registered: {err}")
}

pub fn present_failed(err: &impl Display) -> String {
    format!("{BINARY_NAME}: a frame could not be shown: {err}")
}

/// Windows refused the `AppBar`, so the strip is a plain window above the taskbar and nothing
/// stops maximised windows from covering it.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn edge_not_reserved() -> String {
    format!("{BINARY_NAME}: the screen edge could not be reserved; the strip is a plain window")
}

// The pinning probe.

/// The key that ends input in the terminal the probe runs in.
#[cfg(windows)]
const END_OF_INPUT: &str = "Ctrl-Z then Enter";
#[cfg(not(windows))]
const END_OF_INPUT: &str = "Control-D";

/// One step of the probe's walk-through.
pub struct StepText {
    pub key: &'static str,
    pub text: &'static str,
}

#[cfg(target_os = "macos")]
pub const PROBE_LAUNCH: StepText = StepText {
    key: "launch",
    text: "The strip should now lie along the bottom edge of the display marked with * above, \
           sky over a band of soil, without any app having lost focus.",
};

#[cfg(windows)]
pub const PROBE_LAUNCH: StepText = StepText {
    key: "launch",
    text: "The strip should now lie along the bottom of the display marked with * above, just \
           above the taskbar if the taskbar is on that display: sky over a band of soil. No \
           app should have lost focus. If this run follows a taskkill, first note whether a \
           maximised window stopped short of the bottom before this run started.",
};

/// Run once at each level: phase A at desktop-icon + 1, phase B at always-on-bottom.
#[cfg(target_os = "macos")]
pub const PROBE_LEVEL_STEPS: &[StepText] = &[
    StepText {
        key: "hover-clicks",
        text: "Click a window of another app so that app is frontmost. Move the pointer onto \
               the strip and off again, left-click the strip, then right-click it and dismiss \
               the menu with Esc. Note whether the other app lost focus.",
    },
    StepText {
        key: "mission-control",
        text: "Open Mission Control and close it. Then use Show Desktop (spread thumb and three \
               fingers on the trackpad, or press F11) and undo it. Note where the strip was \
               each time.",
    },
    StepText {
        key: "stage-manager",
        text: "Turn Stage Manager on from Control Center and look at the strip, then turn Stage \
               Manager off again.",
    },
    StepText {
        key: "wallpaper-click",
        text: "In System Settings > Desktop & Dock, set \"Click wallpaper to reveal desktop\" to \
               Always. Click empty wallpaper, then click the strip. Put the setting back as it \
               was.",
    },
    StepText {
        key: "hide-others",
        text: "In another app, press Option-Command-H (Hide Others). Note whether the strip \
               stayed, then bring the hidden apps back.",
    },
];

/// Run once, at the default level.
#[cfg(target_os = "macos")]
pub const PROBE_ONCE_STEPS: &[StepText] = &[
    StepText {
        key: "new-space",
        text: "Open Mission Control, add a Space with the + button, and switch to it. Note \
               whether the strip is there, then switch back and remove that Space.",
    },
    StepText {
        key: "zoomed-window",
        text: "On the strip's display, zoom an ordinary window to fill the screen (Option-click \
               its green button). Note what covers what, then restore the window.",
    },
    StepText {
        key: "fullscreen-other-display",
        text: "With a second display: put an app in full screen on the display without the \
               strip and note whether the strip stays. With one display, press Enter.",
    },
    StepText {
        key: "dock",
        text: "With the Dock at the bottom of the strip's display, move the pointer to the very \
               bottom edge. Note whether the Dock appears over the soil band.",
    },
    StepText {
        key: "hotkey",
        text: "Press Control-Option-T. The strip should come above your windows for 10 seconds, \
               then go back below them. Note whether any app lost focus.",
    },
    StepText {
        key: "cmd-tab-dock",
        text: "Press Command-Tab, then look at the Dock. Note whether Repo Terrarium appears in \
               either.",
    },
    StepText {
        key: "idle-cpu",
        text: "Open Activity Monitor and find repo-terrarium. Keep the pointer off the strip for \
               30 seconds and note its % CPU. The report counts the frames drawn during this \
               step; a still strip draws none.",
    },
    StepText {
        key: "resolution",
        text: "In System Settings > Displays, change the resolution of the strip's display, then \
               change it back. Note whether the strip refitted both times.",
    },
    StepText {
        key: "sleep-wake",
        text: "Optional: put the Mac to sleep, wake it, and note whether the strip is in place. \
               To skip, press Enter.",
    },
    StepText {
        key: "external-display",
        text: "If an external display is connected, note which display the strip chose. \
               Otherwise press Enter.",
    },
    StepText {
        key: "separate-spaces",
        text: "The header shows this run's \"Displays have separate Spaces\" setting. The other \
               setting takes effect only after logging out, so it needs a second run of this \
               probe, which can be ended early with Control-D.",
    },
    StepText {
        key: "preferred-level",
        text: "Which level worked better: desktop-icon + 1 (phases A and C) or always-on-bottom \
               (phase B)? Type the choice and the reason.",
    },
];

/// Windows has one level, the topmost `AppBar`, so every step runs once. The last one ends the
/// process the way a crash does, so it must stay last.
#[cfg(windows)]
pub const PROBE_ONCE_STEPS: &[StepText] = &[
    StepText {
        key: "hover-clicks",
        text: "Click a window of another app so that app has focus. Move the pointer onto the \
               strip and off again, left-click the strip, then right-click it and dismiss the \
               menu with Esc. Note whether the other app lost focus after the left-click, and \
               after the right-click.",
    },
    StepText {
        key: "maximize",
        text: "On the strip's display, maximise an ordinary window. Note whether it stops at \
               the top of the strip or covers it, then restore the window.",
    },
    StepText {
        key: "alt-tab-taskbar",
        text: "Hold Alt and press Tab. Note whether Repo Terrarium is among the windows shown, \
               then release. Then look along the taskbar and note whether it has a Repo \
               Terrarium button.",
    },
    StepText {
        key: "win-d",
        text: "Press Win+D to show the desktop. Note whether the strip stays visible, then press \
               Win+D again to bring the windows back.",
    },
    StepText {
        key: "fullscreen",
        text: "On the strip's display, play a video full screen, or press F11 in a browser. \
               Note whether the strip covers the bottom of it, then leave full screen. This \
               build keeps the strip on top on purpose; the report records what Windows says.",
    },
    StepText {
        key: "display-scale",
        text: "In Settings > System > Display, change the scale of the strip's display (for \
               example from 100% to 125%), then change it back. Note whether the strip refitted \
               both times.",
    },
    StepText {
        key: "hotkey",
        text: "Press Ctrl+Alt+T. The strip is already above other windows, so nothing should \
               change. Note whether any app lost focus.",
    },
    StepText {
        key: "idle-cpu",
        text: "Open Task Manager and find repo-terrarium. Keep the pointer off the strip for 30 \
               seconds and note its CPU. The report counts the frames drawn during this step; a \
               still strip draws none.",
    },
    StepText {
        key: "second-display",
        text: "If a second display is connected, note which display the strip chose. \
               Otherwise press Enter.",
    },
    StepText {
        key: "taskkill",
        text: "Last step. Open a second terminal and run: taskkill /f /im repo-terrarium.exe \
               It ends this probe the way a crash would, so this report stops here. Then \
               maximise a window on the strip's display and see whether it stops short of the \
               bottom, and start the probe again as the README describes.",
    },
];

pub fn level_name(mode: LevelMode) -> &'static str {
    match mode {
        #[cfg(target_os = "macos")]
        LevelMode::DesktopIconPlusOne => "desktop-icon + 1",
        #[cfg(target_os = "macos")]
        LevelMode::AlwaysOnBottom => "always-on-bottom",
        #[cfg(windows)]
        LevelMode::TopmostAppBar => "topmost AppBar",
    }
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Launch => "launch",
        #[cfg(target_os = "macos")]
        Phase::A => "phase A",
        #[cfg(target_os = "macos")]
        Phase::B => "phase B",
        Phase::C => "phase C",
    }
}

pub fn probe_intro(json: &Path, text: &Path) -> String {
    format!(
        "{BINARY_NAME} {VERSION}: pinning probe\n\
         Each step describes something to try with the strip. Press Enter when it is done; to \
         record a note, type it first. {END_OF_INPUT} ends the probe early. The report is \
         rewritten after every step:\n  {}\n  {}",
        json.display(),
        text.display()
    )
}

pub fn probe_header(report: &Report) -> String {
    let mut out = format!("\nOS: {}", report.os);
    // A macOS setting; Windows has no Spaces, so it has no line.
    if report.separate_spaces.is_some() {
        let _ = write!(
            out,
            "\nDisplays have separate Spaces: {}",
            yes_no(report.separate_spaces)
        );
    }
    out.push_str("\nDisplays:");
    for monitor in &report.monitors {
        let chosen = if report.chosen.as_ref() == Some(&monitor.id) {
            "*"
        } else {
            " "
        };
        let primary = if monitor.primary { ", primary" } else { "" };
        let _ = write!(
            out,
            "\n {chosen} {} [{}]: {}x{} px, scale {}{primary}; {}",
            monitor.name,
            monitor.id.0,
            monitor.physical_width,
            monitor.physical_height,
            monitor.scale,
            monitor.detail
        );
    }
    if let Some(strip) = &report.strip {
        let _ = write!(
            out,
            "\nStrip: {}x scale, {}x{} px, image from x = {} px",
            strip.scale, strip.width, strip.height, strip.image_x
        );
    }
    out
}

pub fn probe_step(index: usize, total: usize, step: &Step) -> String {
    format!(
        "\n[{} of {}] {}, {}: {}\n{}\n(Enter when done; type a note first to record one.)",
        index + 1,
        total,
        phase_name(step.phase),
        level_name(step.level),
        step.id,
        step.text
    )
}

pub fn probe_echo(ms: u64, event: &ProbeEvent) -> String {
    format!("  {ms:>8} ms  {}", probe_event(event))
}

fn probe_event(event: &ProbeEvent) -> String {
    match event {
        ProbeEvent::Placed {
            monitor,
            strip,
            frame,
        } => {
            format!(
                "placed on [{}]: {}x scale, {}x{} px; frame at ({}, {}), {} x {} {}",
                monitor.0,
                strip.scale,
                strip.width,
                strip.height,
                frame.x,
                frame.y,
                frame.width,
                frame.height,
                frame.unit
            ) + &frame.appbar.as_ref().map_or_else(String::new, appbar_text)
        }
        ProbeEvent::Level { mode, value } => {
            format!("level {}: {}", level_name(*mode), level_value(*value))
        }
        ProbeEvent::Raised { value } => format!("raised: {}", level_value(*value)),
        ProbeEvent::Restored { mode, value } => {
            format!("restored to {}: {}", level_name(*mode), level_value(*value))
        }
        ProbeEvent::PointerEntered(activity) => {
            format!("pointer entered; {}", activity_text(activity))
        }
        ProbeEvent::PointerLeft(activity) => format!("pointer left; {}", activity_text(activity)),
        ProbeEvent::Click {
            button,
            point,
            activity,
        } => format!(
            "{button} click at {}; {}",
            point.map_or_else(|| "an unknown point".to_owned(), point_text),
            activity_text(activity)
        ),
        ProbeEvent::Focused { focused: true } => "the strip window became key".to_owned(),
        ProbeEvent::Focused { focused: false } => "the strip window is not key".to_owned(),
        ProbeEvent::AppActive { at, active } => {
            format!("at {at}, this app is {}", active_text(*active))
        }
        ProbeEvent::System { what } => format!("system: {what}"),
        ProbeEvent::ScaleFactor { factor } => format!("scale factor now {factor}"),
        ProbeEvent::Resized { width, height } => format!("resized to {width}x{height} px"),
        ProbeEvent::Occluded { occluded: true } => "fully covered".to_owned(),
        ProbeEvent::Occluded { occluded: false } => "visible again".to_owned(),
        ProbeEvent::HotkeyUnavailable { error } => format!("no raise hotkey: {error}"),
        ProbeEvent::Os(snapshot) => os_text(snapshot),
    }
}

pub fn rect_text(rect: &PxRect) -> String {
    format!(
        "({}, {})-({}, {})",
        rect.left, rect.top, rect.right, rect.bottom
    )
}

fn maybe_rect(rect: Option<&PxRect>) -> String {
    rect.map_or_else(|| "unknown".to_owned(), rect_text)
}

fn appbar_text(appbar: &AppBarPlaced) -> String {
    let mut out = format!(
        "\n            {}: monitor {}, work area {} before and {} after; proposed {}, queried {}, \
         granted {}, window {}",
        if appbar.reserved {
            "edge reserved"
        } else {
            "edge not reserved, a plain window"
        },
        rect_text(&appbar.monitor),
        rect_text(&appbar.work_before),
        maybe_rect(appbar.work_after.as_ref()),
        rect_text(&appbar.proposal),
        maybe_rect(appbar.queried.as_ref()),
        rect_text(&appbar.set),
        maybe_rect(appbar.window.as_ref()),
    );
    if appbar.unchanged {
        out.push_str("; unchanged, nothing sent");
    }
    if appbar.size_mismatch {
        out.push_str("; the granted band is not the strip's size");
    }
    if let Some(stale) = &appbar.stale {
        let _ = write!(
            out,
            "\n            old reservation: {} ({} taskbars explain a work area of {}){}",
            if stale.suspected { "suspected" } else { "none" },
            stale.taskbars,
            rect_text(&stale.expected),
            if stale.other_instance {
                "; another strip is running, so nothing was cleared".to_owned()
            } else if stale.cleared {
                format!(
                    "; cleared, work area now {}",
                    maybe_rect(stale.work_after.as_ref())
                )
            } else {
                String::new()
            }
        );
    }
    out
}

fn os_text(snapshot: &OsSnapshot) -> String {
    let flag = |on: bool, name: &str| format!("{}{name}", if on { "" } else { "not " });
    let mut out = format!(
        "window: {}, {}, {}, {}, {}; work area {}",
        flag(snapshot.tool_window, "tool window"),
        flag(snapshot.no_activate, "no-activate"),
        flag(snapshot.app_window, "app window"),
        flag(snapshot.topmost, "topmost"),
        flag(snapshot.visible, "visible"),
        maybe_rect(snapshot.work_area.as_ref())
    );
    if !snapshot.messages.is_empty() {
        out.push_str("\n            messages (subclass/hook):");
        for count in &snapshot.messages {
            let _ = write!(out, " {} {}/{}", count.name, count.subclass, count.hook);
        }
    }
    out
}

fn level_value(value: Option<isize>) -> String {
    value.map_or_else(
        || "window level unknown".to_owned(),
        |v| format!("window level {v}"),
    )
}

fn active_text(active: Option<bool>) -> &'static str {
    match active {
        Some(true) => "active",
        Some(false) => "inactive",
        None => "of unknown activity",
    }
}

fn activity_text(activity: &Activity) -> String {
    format!(
        "this app {}, frontmost: {}",
        active_text(activity.app_active),
        activity.frontmost.as_deref().unwrap_or("unknown")
    )
}

fn point_text(point: NativePoint) -> String {
    match point {
        NativePoint::Scene { x, y } => format!("native ({x}, {y})"),
        NativePoint::Padding => "beside the scene".to_owned(),
    }
}

fn yes_no(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

pub fn probe_written(json: &Path, text: &Path) -> String {
    format!(
        "\nThe report is written:\n  {}\n  {}",
        json.display(),
        text.display()
    )
}

pub fn probe_write_failed(dir: &Path, err: &impl Display) -> String {
    format!(
        "{BINARY_NAME}: the probe report could not be written to {}: {err}",
        dir.display()
    )
}

fn end_reason(reason: EndReason) -> &'static str {
    match reason {
        EndReason::Completed => "every step answered",
        EndReason::Quit => "quit from the menu",
        EndReason::InputClosed => "input closed",
        EndReason::WindowClosed => "window closed",
        EndReason::LaunchFailed => "the strip could not open",
    }
}

/// The report for people: the header, then each step with its note, counts and events.
pub fn probe_report_text(report: &Report) -> String {
    let mut out = format!(
        "{BINARY_NAME} {} pinning probe, started at Unix time {}\n",
        report.version, report.started_unix
    );
    out.push_str(probe_header(report).trim_start());
    let _ = write!(
        out,
        "\nEnded: {}\n",
        report.ended.map_or("still running", end_reason)
    );
    for step in &report.steps {
        out.push_str(&step_text(step));
    }
    out
}

fn step_text(step: &StepReport) -> String {
    let mut out = format!(
        "\n{} ({})\n  note: {}\n  frames drawn: {}\n  pointer moves: {}\n",
        step.id,
        level_name(step.level),
        if step.note.is_empty() {
            "(none)"
        } else {
            &step.note
        },
        step.frames,
        moves_text(&step.moves)
    );
    for timed in &step.events {
        let _ = writeln!(out, "{}", probe_echo(timed.ms, &timed.event));
    }
    if step.events_dropped > 0 {
        let _ = writeln!(out, "  and {} more events", step.events_dropped);
    }
    out
}

fn moves_text(moves: &MoveSummary) -> String {
    let total = moves.while_active + moves.while_inactive + moves.while_unknown;
    if total == 0 {
        return "none".to_owned();
    }
    format!(
        "{total} ({} while active, {} while inactive, {} unknown), from {} to {}",
        moves.while_active,
        moves.while_inactive,
        moves.while_unknown,
        moves.first.map_or_else(String::new, point_text),
        moves.last.map_or_else(String::new, point_text)
    )
}

/// The Windows probe's instructions for testers, which `cargo xtask probe-zip` puts in the zip
/// as `README.txt` (`repo-terrarium --probe-readme` prints it). Plain text, wrapped at 80.
pub const PROBE_README: &str = "Repo Terrarium: the Windows pinning probe
==========================================

Thank you for testing. Repo Terrarium is a thin pixel-art strip that sits along
the bottom edge of a monitor. On Windows it reserves that edge, the way the
taskbar does, so maximised windows stop above it. Nobody on this project has a
Windows machine, so this probe is how we find out whether that works.

It takes about ten minutes. It changes nothing on your machine: there is no
installer, and it writes only its report files, into this folder.


Before you start
----------------

1. Extract the zip first (right-click it, then Extract All). Running it from
   inside the zip does not work.
2. Windows may say \"Windows protected your PC\", because the app is not signed.
   Choose \"More info\", then \"Run anyway\".
3. In this build the strip stays on top of full-screen video on its display.
   That is expected; the probe measures it.


Run 1: the walk-through
-----------------------

Double-click run-probe.cmd. A console window opens and the strip appears.

The console describes one thing to try at a time. Try it, then come back to the
console: type a short note if something is worth saying, and press Enter to go
on. Enter alone is fine too.

The last step asks you to end the probe from a second terminal with:

    taskkill /f /im repo-terrarium.exe

That ends it the way a crash would, on purpose. Then maximise a window on the
same display and look whether it stops short of the bottom, as if the strip
were still there.


Run 2 and run 3: two short checks
---------------------------------

Run 2. Double-click run-probe.cmd again. At the first step, type a note saying
whether the maximised window stopped short of the bottom before this run, then
look at the strip and press Enter. Now press Ctrl-C in the console. The probe
ends at once, and the console asks \"Terminate batch job (Y/N)?\": type Y and
press Enter.

Run 3. Double-click run-probe.cmd once more. At the first step, type a note
saying whether a maximised window still stopped short of the bottom after run 2,
then press Enter. End this run with Ctrl-Z and then Enter.


Sending the results
-------------------

Each run writes two files into this folder, named
repo-terrarium-probe-pinning-<numbers>.json and .txt. Please send all of the
repo-terrarium-probe- files back. They hold the display sizes, what Windows
reported about the strip, the names of window classes (never window titles),
and your notes. Nothing else.

If the strip ever gets stuck on screen, or the bottom of the screen stays
reserved after the probe has ended, signing out and back in clears it.
";

#[cfg(feature = "dev")]
pub fn menu_debug() -> &'static str {
    "Debug window"
}

/// The words of the debug window and the egui memory probe (dev builds only).
#[cfg(feature = "dev")]
pub mod debug {
    use std::fmt::Write as _;
    use std::path::Path;

    use terrarium_truth::PresentationState;
    use terrarium_truth::scene::{EventKind, Light, Weather};

    use super::{BINARY_NAME, PRODUCT_NAME, VERSION};
    use crate::debug::Readout;
    use crate::egui_host::{HostError, WindowKind};
    use crate::egui_probe::{Phase, Sample};

    pub const SCENE: &str = "Scene";
    pub const THRIVING: &str = "Thriving, as a plain launch shows it";
    pub const DEMO: &str = "The demo cycle, from its start";
    pub const SYNTHETIC: &str = "Synthetic climate";
    pub const CLIMATE: &str = "Climate";
    pub const HEALTH: &str = "Health (light and soil)";
    pub const ACTIVITY: &str = "Activity (rain and wind)";
    pub const LIGHT: &str = "Light";
    pub const SOIL: &str = "Soil";
    pub const RAIN: &str = "Rain, % of baseline";
    pub const WIND: &str = "Wind, % of baseline";
    pub const WEATHER: &str = "Weather";
    pub const LIGHT_SHIFT: &str = "Light shift";
    pub const IDLE_DAYS: &str = "Days since a commit";
    pub const AGE: &str = "Age in years";
    pub const RECOVERING: &str = "Recovering";
    pub const EVENT: &str = "Event";
    pub const EVENTS_NOT_DRAWN: &str =
        "The scene carries the event; its choreography is not drawn yet.";
    pub const CLEAR_EVENT: &str = "None";
    pub const CLOCK: &str = "Clock";
    pub const FIXED_TIME: &str = "A fixed time of day";
    pub const DIM: &str = "Dim";
    pub const DIM_IDLE: &str = "after idle time";
    pub const DIM_ON: &str = "on";
    pub const DIM_OFF: &str = "off";
    pub const READOUT: &str = "Readout";

    pub fn window_title(kind: WindowKind) -> String {
        match kind {
            WindowKind::Debug => format!("{PRODUCT_NAME} {VERSION}: debug"),
        }
    }

    pub fn preset(name: &str) -> String {
        format!("Preset: {name}")
    }

    pub fn weather(weather: Weather) -> &'static str {
        match weather {
            Weather::Clear => "clear",
            Weather::Overcast => "overcast",
            Weather::Storm => "storm",
            Weather::Haze => "haze",
        }
    }

    pub fn light(light: Light) -> &'static str {
        match light {
            Light::Neutral => "neutral",
            Light::Dusk => "dusk",
            Light::Sunburst => "sunburst",
        }
    }

    pub fn event(kind: EventKind) -> &'static str {
        match kind {
            EventKind::Storm => "storm",
            EventKind::Clearing => "clearing",
            EventKind::Lightning => "lightning",
            EventKind::Dusk => "dusk",
            EventKind::Sunburst => "sunburst",
            EventKind::ClearingBurn => "clearing burn",
            EventKind::Flowering => "flowering",
            EventKind::Windstorm => "windstorm",
            EventKind::RainAfterDrought => "rain after drought",
            EventKind::Earthquake => "earthquake",
            EventKind::Sinkhole => "sinkhole",
        }
    }

    pub fn state(state: PresentationState) -> &'static str {
        match state {
            PresentationState::Thriving => "thriving",
            PresentationState::Struggling => "struggling",
            PresentationState::Mature => "mature",
            PresentationState::Dormant => "dormant",
            PresentationState::Dead => "dead",
            PresentationState::Recovering => "recovering",
        }
    }

    pub fn derived(health: u8, activity: u8, shown: PresentationState) -> String {
        format!("health {health}, activity {activity}: {}", state(shown))
    }

    /// `minutes` after midnight, as `HH:MM`.
    pub fn time_of_day(minutes: f64) -> String {
        // The slider holds whole minutes of one day.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let minutes = minutes.max(0.0) as u32;
        format!("{:02}:{:02}", minutes / 60, minutes % 60)
    }

    fn megabytes(bytes: u64) -> String {
        format!("{}.{} MB", bytes >> 20, ((bytes & 0xF_FFFF) * 10) >> 20)
    }

    pub fn readout(readout: &Readout) -> Vec<String> {
        let mut lines = vec![
            format!("state: {}", state(readout.state)),
            format!(
                "local time {}{}",
                time_of_day(f64::from(readout.clock.ms_of_day / 60_000)),
                if readout.dimmed { ", dimmed" } else { "" }
            ),
            format!("frames presented: {}", readout.frames),
            format!("frames in the last second: {}", readout.fps),
            format!("compositions: {}", readout.compositions),
        ];
        if let Some(stage) = readout.stage {
            lines.push(format!("demo stage: {stage}"));
        }
        lines.push(match readout.memory {
            Some(memory) => match memory.footprint {
                Some(footprint) => format!(
                    "memory: {} resident, {} footprint",
                    megabytes(memory.resident),
                    megabytes(footprint)
                ),
                None => format!("memory: {} resident", megabytes(memory.resident)),
            },
            None => "memory: not known".to_owned(),
        });
        lines
    }

    pub fn host_failed(err: &HostError) -> String {
        let why = match err {
            HostError::Window(err) => format!("the window could not be created: {err}"),
            HostError::Gpu(err) => format!("no graphics device: {err}"),
        };
        format!("{BINARY_NAME}: the debug window could not open: {why}")
    }

    fn phase(phase: Phase) -> String {
        match phase {
            Phase::Baseline => "baseline, before any window".to_owned(),
            Phase::Open(cycle) => format!("cycle {cycle}, window open"),
            Phase::Closed(cycle) => format!("cycle {cycle}, window closed"),
        }
    }

    /// The egui memory probe's report: one line per phase.
    pub fn egui_probe_report(samples: &[Sample]) -> String {
        let mut text = format!(
            "{PRODUCT_NAME} {VERSION}: the egui memory probe\n\
             Each phase is held 15 s. Memory is resident size and footprint; wakes are the \
             event loop's, per minute.\n\n"
        );
        for sample in samples {
            let memory = sample.memory.map_or_else(
                || "memory not known".to_owned(),
                |memory| {
                    format!(
                        "{} resident, {} footprint",
                        megabytes(memory.resident),
                        memory.footprint.map_or_else(|| "no".to_owned(), megabytes)
                    )
                },
            );
            let _ = writeln!(
                text,
                "{:>4} s  {:<28} {memory}, {} wakes a minute, app active {}, frontmost {}",
                sample.at,
                phase(sample.phase),
                sample.wakes_per_minute,
                sample
                    .app_active
                    .map_or("not known", |active| if active { "yes" } else { "no" }),
                sample.frontmost.as_deref().unwrap_or("not known"),
            );
        }
        text
    }

    pub fn egui_probe_written(path: &Path) -> String {
        format!("The egui memory report is written: {}", path.display())
    }

    pub fn egui_probe_write_failed(path: &Path, err: &impl std::fmt::Display) -> String {
        format!(
            "{BINARY_NAME}: the egui memory report could not be written to {}: {err}",
            path.display()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pinning::DEFAULT_LEVEL;

    /// Every string literal in a source file, tests included, so a string anywhere in this module
    /// is checked; a list of rendered strings would miss one nobody added to it.
    fn string_literals(source: &str) -> Vec<String> {
        let mut literals = Vec::new();
        let mut chars = source.chars();
        while let Some(c) = chars.next() {
            match c {
                // A comment runs to the end of its line.
                '/' if chars.clone().next() == Some('/') => {
                    chars.by_ref().find(|&c| c == '\n');
                }
                // A char literal (`'"'`, `'\\''`) is skipped; a lifetime (`'static`) is not one.
                '\'' => {
                    let mut ahead = chars.clone();
                    match (ahead.next(), ahead.next()) {
                        (Some('\\'), _) => {
                            chars.nth(1);
                            chars.by_ref().find(|&c| c == '\'');
                        }
                        (Some(_), Some('\'')) => {
                            chars.nth(1);
                        }
                        _ => {}
                    }
                }
                '"' => {
                    let mut literal = String::new();
                    while let Some(c) = chars.next() {
                        match c {
                            '\\' => literal.extend(chars.next()),
                            '"' => break,
                            _ => literal.push(c),
                        }
                    }
                    literals.push(literal);
                }
                _ => {}
            }
        }
        literals
    }

    #[test]
    fn copy_has_no_exclamation_marks() {
        let literals = string_literals(include_str!("copy.rs"));
        assert!(
            literals.iter().any(|l| l == "Repo Terrarium"),
            "the scan found {literals:?}"
        );
        for literal in literals {
            assert!(!literal.contains('!'), "copy must not exclaim: {literal:?}");
        }
    }

    #[test]
    fn the_binary_is_built_under_its_pinned_name() {
        // The manifest's `[[bin]] name` and the pinned identifier must agree.
        assert_eq!(env!("CARGO_BIN_NAME"), BINARY_NAME);
    }

    #[test]
    fn the_scan_finds_literals_and_skips_comments() {
        let source = "// a comment: \"skipped\"\nconst A: &'static str = \"one\";\nfn f() { g(\"x \\\"y\\\" {}\", 1) }\nconst Q: [char; 2] = ['\"', '\\''];";
        assert_eq!(string_literals(source), ["one", "x \"y\" {}"]);
    }

    #[test]
    fn the_text_report_holds_every_step_note() {
        let mut step = StepReport::new("b.stage-manager", DEFAULT_LEVEL);
        step.note = "strip stayed put".to_owned();
        step.frames = 3;
        let report = Report {
            probe: "pinning",
            version: "0.0.0",
            os: "Example OS".to_owned(),
            started_unix: 7,
            separate_spaces: Some(false),
            monitors: Vec::new(),
            chosen: None,
            strip: None,
            steps: vec![step, StepReport::new("c.dock", DEFAULT_LEVEL)],
            ended: Some(EndReason::Quit),
        };
        let text = probe_report_text(&report);
        let level = level_name(DEFAULT_LEVEL);
        assert!(
            text.contains(&format!("b.stage-manager ({level})")),
            "{text}"
        );
        assert!(text.contains("note: strip stayed put"), "{text}");
        assert!(text.contains("frames drawn: 3"), "{text}");
        assert!(
            text.contains(&format!("c.dock ({level})\n  note: (none)")),
            "{text}"
        );
        assert!(text.contains("Ended: quit from the menu"), "{text}");
    }

    #[test]
    fn the_probe_readme_names_the_launcher_the_reports_and_the_steps_it_asks_for() {
        for needle in [
            "run-probe.cmd",
            "repo-terrarium-probe-",
            "taskkill /f /im repo-terrarium.exe",
            "Ctrl-C",
            "Terminate batch job",
            "Ctrl-Z",
        ] {
            assert!(PROBE_README.contains(needle), "{needle}");
        }
        assert!(PROBE_README.lines().all(|line| line.chars().count() <= 80));
    }

    mod discovery {
        use terrarium_adapters::discovery::{Discovery, Located, Runner, RunnerFound, Search};
        use terrarium_adapters::report::{
            Format, KnipCounts, Known, Outcome, Reading, Report, Why,
        };

        use super::super::discovery_report;
        use crate::discover::Reads;

        fn known(format: Format, files: &[&str], reading: Reading) -> Report {
            Report {
                format,
                files: files.iter().map(|f| (*f).to_owned()).collect(),
                outcome: Outcome::Known(Known {
                    reading,
                    contributors: Vec::new(),
                    content_hash: [0; 32],
                    mtime: None,
                }),
            }
        }

        fn found(format: Format, pattern: &str, files: &[&str]) -> Search {
            Search::Found(Located {
                format,
                pattern: pattern.to_owned(),
                files: files.iter().map(|f| (*f).to_owned()).collect(),
            })
        }

        fn nothing() -> Discovery {
            Discovery {
                git: Ok("main".to_owned()),
                coverage: Search::NotFound,
                test_report: Search::NotFound,
                local_tests: false,
                dead_code: Search::NotFound,
                knip_config: None,
                knip_installed: None,
                ci_workflows: Vec::new(),
                runners: Vec::new(),
            }
        }

        fn no_reads() -> Reads {
            Reads {
                coverage: None,
                test_report: None,
                local_tests: None,
                dead_code: None,
            }
        }

        #[test]
        fn a_report_states_what_was_found_and_how_the_rest_connects() {
            let surefire = [
                "target/surefire-reports/TEST-moss.FernTest.xml",
                "target/surefire-reports/TEST-moss.ReedTest.xml",
            ];
            let discovery = Discovery {
                coverage: found(Format::Lcov, "coverage/lcov.info", &["coverage/lcov.info"]),
                test_report: found(
                    Format::JUnit,
                    "target/surefire-reports/TEST-*.xml",
                    &surefire,
                ),
                local_tests: true,
                ci_workflows: vec![".github/workflows/ci.yml".to_owned()],
                runners: vec![RunnerFound {
                    runner: Runner::Maven,
                    config: "pom.xml".to_owned(),
                }],
                ..nothing()
            };
            let reads = Reads {
                coverage: Some(known(
                    Format::Lcov,
                    &["coverage/lcov.info"],
                    Reading::Coverage {
                        lines_found: 1530,
                        lines_hit: 1204,
                    },
                )),
                test_report: Some(known(
                    Format::JUnit,
                    &surefire,
                    Reading::Tests {
                        tests: 120,
                        failed: 3,
                        skipped: 1,
                    },
                )),
                local_tests: Some(known(
                    Format::LocalTests,
                    &["local-tests/ab.json"],
                    Reading::LocalRun { exit_code: 101 },
                )),
                dead_code: None,
            };
            assert_eq!(
                discovery_report(&discovery, &reads),
                "Found\n\
                 \x20 git               default branch main\n\
                 \x20 coverage report   coverage/lcov.info, lcov: 1204 of 1530 lines covered\n\
                 \x20 test report       target/surefire-reports/TEST-*.xml, 2 JUnit files: 120 \
                 tests, 3 failing, 1 skipped\n\
                 \x20 test runs         recorded by record-tests: the last run failed with exit \
                 code 101\n\
                 \x20 CI workflow       .github/workflows/ci.yml\n\
                 \x20 test runner       Maven (pom.xml)\n\
                 Not found\n\
                 \x20 dead-code report\n\
                 To connect\n\
                 \x20 dead-code report  Knip prints its JSON report to stdout, and the report is \
                 read from knip-report.json: npx knip --reporter json --no-progress > \
                 knip-report.json\n"
            );
        }

        #[test]
        fn what_is_missing_gets_one_line_each_and_the_runner_s_own_way() {
            let discovery = Discovery {
                runners: vec![RunnerFound {
                    runner: Runner::Jest,
                    config: "jest.config.ts".to_owned(),
                }],
                knip_config: Some("knip.json".to_owned()),
                ..nothing()
            };
            let text = discovery_report(&discovery, &no_reads());
            let (found, rest) = text.split_once("Not found\n").unwrap();
            assert!(found.contains("  Knip              knip.json\n"), "{found}");
            let (missing, connect) = rest.split_once("To connect\n").unwrap();
            assert_eq!(
                missing,
                "  coverage report\n  test report\n  dead-code report\n  CI workflow\n"
            );
            assert_eq!(connect.lines().count(), 4, "{connect}");
            assert!(connect.contains(
                "  coverage report   lcov is read from coverage/lcov.info, lcov.info or \
                 coverage.lcov; Cobertura XML from coverage/cobertura-coverage.xml, coverage.xml \
                 or cobertura.xml.\n"
            ));
            assert!(connect.contains(
                "  test report       jest-junit writes JUnit XML to junit.xml as one of Jest's \
                 reporters. Or: repo-terrarium record-tests -- <command> records any test \
                 command's result.\n"
            ));
            assert!(connect.contains("Knip prints its JSON report to stdout"));
            // A run recorded by record-tests is a test report.
            let recorded = Discovery {
                local_tests: true,
                ..nothing()
            };
            assert!(!discovery_report(&recorded, &no_reads()).contains("  test report"));
        }

        #[test]
        fn a_report_that_cannot_be_used_says_why() {
            let cases = [
                (Why::Missing, "no file is there"),
                (
                    Why::OutsideRepository("../x".into()),
                    "../x is outside the repository",
                ),
                (
                    Why::NotAFile("junit.xml".into()),
                    "junit.xml is not a regular file",
                ),
                (
                    Why::TooLarge("junit.xml".into()),
                    "junit.xml is larger than a report is read",
                ),
                (
                    Why::Io {
                        path: "junit.xml".into(),
                        kind: std::io::ErrorKind::PermissionDenied,
                    },
                    "junit.xml could not be read (permission denied)",
                ),
                (
                    Why::Malformed {
                        path: "junit.xml".into(),
                        line: Some(7),
                    },
                    "junit.xml is not a complete JUnit report at line 7",
                ),
                (Why::Empty, "no test ran in it"),
            ];
            for (why, expected) in cases {
                let discovery = Discovery {
                    test_report: found(Format::JUnit, "junit.xml", &["junit.xml"]),
                    ..nothing()
                };
                let reads = Reads {
                    test_report: Some(Report {
                        format: Format::JUnit,
                        files: vec!["junit.xml".into()],
                        outcome: Outcome::Unknown(why),
                    }),
                    ..no_reads()
                };
                let text = discovery_report(&discovery, &reads);
                assert!(
                    text.contains(&format!("junit.xml, JUnit: not read, {expected}\n")),
                    "{text}"
                );
            }
            let knip = Discovery {
                dead_code: found(Format::Knip, "knip-report.json", &["knip-report.json"]),
                ..nothing()
            };
            let old = Reads {
                dead_code: Some(Report {
                    format: Format::Knip,
                    files: vec!["knip-report.json".into()],
                    outcome: Outcome::Unknown(Why::KnipBefore6 {
                        version: Some("5.88.0".into()),
                    }),
                }),
                ..no_reads()
            };
            assert!(
                discovery_report(&knip, &old).contains(
                    "not read, it is from Knip 5.88.0, and reports from Knip 6 on are read"
                )
            );
            let read = Reads {
                dead_code: Some(known(
                    Format::Knip,
                    &["knip-report.json"],
                    Reading::DeadCode(KnipCounts {
                        unused_exports: 1,
                        unused_files: vec!["src/a.ts".into(), "src/b.ts".into()],
                        ..KnipCounts::default()
                    }),
                )),
                ..no_reads()
            };
            assert!(discovery_report(&knip, &read).contains(
                "knip-report.json, Knip: 1 unused export and 2 unused files, not part of soil yet"
            ));
        }
    }

    #[test]
    fn the_version_line_and_title_carry_the_workspace_version() {
        assert_eq!(
            version_line(),
            format!("repo-terrarium {}", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            window_title(),
            format!("Repo Terrarium {}", env!("CARGO_PKG_VERSION"))
        );
    }
}
