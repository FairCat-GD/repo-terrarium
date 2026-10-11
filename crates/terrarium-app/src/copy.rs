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
use terrarium_compose::sky::LocalDate;
use terrarium_engine::headless::HeadlessError;
use terrarium_engine::{Branch, ConnectError, Problem};
use terrarium_platform::login::LoginError;
use terrarium_truth::scene::Weather;
use terrarium_truth::{ReplayError, SIM_VERSION};

use crate::cli::UsageError;
use crate::crash::Crash;
use crate::discover::Reads;
use crate::egui_host::{HostError, WindowKind};
use crate::hotkey::{Binding, Platform, Which, key_name};
use crate::live::{LabelFacts, Note, Once};
use crate::pinning::AttachError;
use crate::present::PresentError;
use crate::strip::StripError;
use global_hotkey::hotkey::Code;

mod probe;
pub use probe::*;

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
        UsageError::MissingOutput => "replay writes one result: --dump, or --gif <file>".to_owned(),
        UsageError::GifOnly(flag) => format!("{flag} goes with --gif"),
        UsageError::BadScale(value) => format!(
            "'{}' is not a scale from 1 to {}",
            value.to_string_lossy(),
            crate::cli::MAX_SCALE
        ),
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
        "{BINARY_NAME}: {problem}\nusage: {BINARY_NAME} [--repo <path> | --version | --demo | \
         {} | --probe-readme]\n       {BINARY_NAME} replay <path> (--dump | --gif <file> \
         [--night] [--scale <1-{max}>]) [--observations <file>] [--at <unix-seconds>]\n       \
         {BINARY_NAME} discover <path>\n       \
         {BINARY_NAME} record-tests -- <command> [<argument>...]",
        probes.join(" | "),
        max = crate::cli::MAX_SCALE,
    )
}

/// The strip's label on a connected repository: its name, `branch - weather`, and one note when
/// one applies, the most pressing first (`Notes::showing`). A detached `HEAD` is named by its
/// commit. `now` is the wall clock, in Unix seconds, against which a dormancy is dated.
pub fn live_label(facts: &LabelFacts, now: i64) -> Vec<String> {
    let mut lines = vec![facts.repo_name.clone()];
    if let Some((branch, weather)) = &facts.branch {
        let branch = match branch {
            Branch::Named(name) => name.as_str(),
            Branch::Detached(commit) => &commit.as_str()[..7],
        };
        lines.push(format!("{branch} - {}", weather_word(*weather)));
    }
    lines.extend(facts.note.showing().map(|note| note_line(note, now)));
    lines
}

/// One note's line, at most 18 characters.
fn note_line(note: Note, now: i64) -> String {
    match note {
        Note::Halted => "halted".to_owned(),
        Note::RepoNotFound => "repo not found".to_owned(),
        Note::Progress(percent) => format!("history {percent}%"),
        Note::Tuning => "tuning".to_owned(),
        Note::Once(Once::SimUpdated) => "sim updated".to_owned(),
        Note::Once(Once::HistoryRewritten) => "history rewritten".to_owned(),
        Note::Settling(1) => "settling 1 day".to_owned(),
        Note::Settling(days) => format!("settling {days} days"),
        Note::Dormant(since) => {
            let (then, today) = (utc_date(since), utc_date(now));
            let months = |date: LocalDate| i64::from(date.year) * 12 + i64::from(date.month);
            // Within the last eleven months the month is enough; before that, the year.
            if months(today) - months(then) <= 11 {
                format!("dormant since {}", MONTHS[usize::from(then.month - 1)])
            } else {
                format!("dormant since {}", then.year)
            }
        }
    }
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// The UTC calendar date of a moment in Unix seconds.
fn utc_date(secs: i64) -> LocalDate {
    LocalDate::from_days_since_epoch(secs.div_euclid(86_400))
}

/// The weather's word, on the label and in the debug window alike.
pub(crate) fn weather_word(weather: Weather) -> &'static str {
    match weather {
        Weather::Clear => "clear",
        Weather::Overcast => "overcast",
        Weather::Storm => "storm",
        Weather::Haze => "haze",
    }
}

/// Why the strip could not connect, and so did not open.
pub fn connect_failed(repo: Option<&Path>, err: &ConnectError) -> String {
    let problem = match (err, repo) {
        (ConnectError::Git(err), Some(repo)) => format!(
            "{} could not be connected as a git repository: {err}",
            repo.display()
        ),
        (ConnectError::Git(err), None) => {
            format!("the connected repository could not be read: {err}")
        }
        (ConnectError::Path(err), _) => format!("the path could not be made absolute: {err}"),
        (ConnectError::Store(err), _) => format!("the data directory failed: {err}"),
        (ConnectError::Salt(err), _) => format!("no random bytes for a new terrarium: {err}"),
        (ConnectError::Thread(err), _) => format!("the engine could not start: {err}"),
        (ConnectError::Halted, _) => "connecting stopped on an internal error".to_owned(),
        (ConnectError::OtherRepository, _) => other_repository().to_owned(),
    };
    format!("{BINARY_NAME}: {problem}")
}

/// Why a folder could not be connected, relinked, or a terrarium changed, as its window says
/// it.
pub fn connect_problem(err: &ConnectError) -> String {
    match err {
        ConnectError::Git(err) => {
            format!("This folder is not a git repository that can be read: {err}.")
        }
        ConnectError::Path(err) => format!("This folder's path could not be read: {err}."),
        ConnectError::Store(err) => format!("The data directory failed: {err}."),
        ConnectError::Salt(err) => format!("No random bytes for a new terrarium: {err}."),
        ConnectError::Thread(err) => format!("The engine could not start: {err}."),
        ConnectError::Halted => "Connecting stopped on an internal error.".to_owned(),
        ConnectError::OtherRepository => format!("{}.", capitalized(other_repository())),
    }
}

fn other_repository() -> &'static str {
    "that folder holds another repository"
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

pub fn data_dir_unavailable(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the data directory is unavailable: {err}")
}

/// A refresh that failed, which the next may not repeat.
pub fn engine_problem(problem: &Problem) -> String {
    let problem = match problem {
        Problem::Git(err) => format!("reading the repository failed: {err}"),
        Problem::Store(err) => format!("the data directory failed: {err}"),
        Problem::Replay(err) => format!("the replay could not run: {err:?}"),
        Problem::Missing(what) => format!("a refresh went on without its {what}"),
    };
    format!("{BINARY_NAME}: {problem}")
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

pub fn replay_no_day(repo: &Path) -> String {
    format!(
        "{BINARY_NAME}: no day of the history of {} has ended yet",
        repo.display()
    )
}

pub fn replay_gif_failed(path: &Path, err: &impl Display) -> String {
    format!(
        "{BINARY_NAME}: the GIF could not be written to {}: {err}",
        path.display()
    )
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

pub fn print_failed(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the text could not be written: {err}")
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

/// Where light, soil and weather are kept, for the discovery report.
pub const DEVICE_LOSS: &str = "Light, soil and weather are recorded on this machine only. On a \
    new machine or after a reinstall, the git history replays, and they read as unknown before \
    that day.";

/// Whether the discovery report says how each missing adapter connects: the command line and
/// the settings say it; the report shown once at connect says only what was found and not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hints {
    Shown,
    Left,
}

/// The discovery report: what was found, what each report reads as, what was not found, one
/// line per missing adapter on how it connects when `hints` asks, and where light, soil and
/// weather are kept. Facts only.
pub fn discovery_report(found: &Discovery, reads: &Reads, hints: Hints) -> String {
    let mut out = report_rows(found, reads, hints);
    let _ = writeln!(out, "\n{DEVICE_LOSS}");
    out
}

fn report_rows(found: &Discovery, reads: &Reads, hints: Hints) -> String {
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
    if hints == Hints::Left {
        return out;
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
        files => format!(
            "{}, {}",
            located.pattern,
            plural(
                u64::try_from(files.len()).unwrap_or(u64::MAX),
                &format!("{format} file"),
                &format!("{format} files")
            )
        ),
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
            suite_failed,
        } => {
            let mut text = format!("{}, ", plural(*tests, "test", "tests"));
            if *failed == 0 && *suite_failed {
                text.push_str("none failing, but a suite failed outside its tests");
            } else if *failed == 0 {
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
            "lcov is read from {}; Cobertura XML from {}. Of several there, the newest is read.",
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
            Self::Connect(_, err) => write!(f, "{}", connect_problem(err)),
            Self::NoConnectWindow => {
                write!(
                    f,
                    "no repository is connected, and its window could not open"
                )
            }
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

pub fn menu_settings() -> &'static str {
    "Settings"
}

pub fn menu_time_lapse() -> &'static str {
    "Time-lapse"
}

pub fn menu_pause() -> &'static str {
    "Pause"
}

pub fn menu_resume() -> &'static str {
    "Resume"
}

/// The label's one line while the scene is paused.
pub fn label_paused() -> &'static str {
    "paused"
}

/// A hotkey's keys as the platform's keyboards name them: Control-Option-T on a Mac, in
/// Apple's order, and Ctrl+Alt+T on Windows.
pub fn hotkey(binding: Binding, platform: Platform) -> String {
    let mods = binding.mods;
    let names = settings::modifier_names(platform);
    let mut parts: Vec<&str> = [mods.control, mods.option, mods.shift, mods.command]
        .into_iter()
        .zip(names)
        .filter(|(on, _)| *on)
        .map(|(_, name)| name)
        .collect();
    let key = key_word(binding.key);
    parts.push(&key);
    let between = match platform {
        Platform::Mac => "-",
        Platform::Windows => "+",
    };
    parts.join(between)
}

/// A key as the words name it: the space bar by its name, any other by its own legend.
pub fn key_word(key: Code) -> String {
    match key {
        Code::Space => "Space".to_owned(),
        key => key_name(key).to_owned(),
    }
}

/// A crash log's report: what ran, where it failed and how it got there.
pub fn crash_report(crash: &Crash) -> String {
    format!(
        "{BINARY_NAME} {VERSION} crash report\n\
         simulation version: {}\n\
         system: {} {}\n\
         thread: {}\n\
         at: {}\n\
         message: {}\n\n\
         backtrace:\n{}\n",
        SIM_VERSION.0,
        std::env::consts::OS,
        std::env::consts::ARCH,
        crash.thread,
        crash.location,
        crash.message,
        crash.backtrace
    )
}
/// An egui window's title.
pub fn egui_window_title(kind: WindowKind) -> String {
    match kind {
        WindowKind::Connect => PRODUCT_NAME.to_owned(),
        WindowKind::Settings => format!("{PRODUCT_NAME}: settings"),
        WindowKind::TimeLapse => format!("{PRODUCT_NAME}: time-lapse"),
        #[cfg(feature = "dev")]
        WindowKind::Debug => format!("{PRODUCT_NAME} {VERSION}: debug"),
    }
}

pub fn host_failed(err: &HostError) -> String {
    let why = match err {
        HostError::Window(err) => format!("the window could not be created: {err}"),
        HostError::Gpu(err) => format!("no graphics device: {err}"),
    };
    format!("{BINARY_NAME}: a window could not open: {why}")
}

/// The connect window.
pub mod connect {
    pub const HEADING: &str = "Connect a repository";
    pub const INTRO: &str = "The terrarium grows from one local git repository. Only its git \
        history is read; nothing in it runs.";
    pub const CHOOSE: &str = "Choose folder…";
    pub const NONE_CHOSEN: &str = "No folder chosen";
    pub const OPEN_AT_LOGIN: &str = "Open at login";
    pub const CONNECT: &str = "Connect";
    pub const READING: &str = "Reading the repository…";
    pub const DONE: &str = "Done";

    pub fn connected(name: &str) -> String {
        format!("{name} is connected")
    }
}

/// The settings window: its tabs, and in each what is set and what it does.
pub mod settings {
    use std::fmt::Display;

    use terrarium_platform::login::{LoginError, LoginStatus};

    use super::{PRODUCT_NAME, VERSION};
    use crate::hotkey::{Platform, Refusal};
    use crate::prefs::AdapterKind;

    pub const REPOSITORY: &str = "Repository";
    pub const STRIP: &str = "Strip";
    pub const ADAPTERS: &str = "Adapters";
    pub const HOTKEYS: &str = "Hotkeys";
    pub const TIME: &str = "Time";
    pub const GENERAL: &str = "General";
    pub const NAME: &str = "Name";
    pub const FOLDER: &str = "Folder";
    pub const DEFAULT_BRANCH: &str = "Default branch";
    pub const NOT_FOUND: &str = "Not found at this folder.";
    pub const LOCATE: &str = "Locate…";
    pub const RELINK: &str = "Relink";
    pub const NEW_TERRARIUM: &str = "New terrarium…";
    pub const RESEED_STATEMENT: &str = "A new terrarium lays the same history out anew: \
        plants, residents and stones take new places. Its history, and what it shows, stay as \
        they are.";
    pub const RESEED: &str = "New terrarium";
    pub const DISCONNECT_BUTTON: &str = "Disconnect…";
    pub const DELETE_OBSERVATIONS: &str = "Delete its recorded observations";
    pub const DELETE_STATEMENT: &str = "Light, soil and weather history exist only in these \
        observations, so they would be lost. The git history still replays.";
    pub const DISCONNECT: &str = "Disconnect";
    pub const CANCEL: &str = "Cancel";
    pub const WORKING: &str = "Working…";

    pub fn chosen(path: &str) -> String {
        format!("Chosen: {path}")
    }

    pub fn disconnect_statement(name: &str) -> String {
        format!(
            "Disconnecting stops showing {name}. Its git history replays whenever it is \
             connected again."
        )
    }

    /// A change that could not be kept.
    pub fn not_saved(err: &impl Display) -> String {
        format!("The setting could not be saved: {err}")
    }

    // The strip.
    pub const MONITOR: &str = "Display";
    pub const AUTOMATIC: &str = "Automatic";
    pub const AUTOMATIC_FACT: &str =
        "Automatic puts the strip on a display other than the main one, when there is one.";
    pub const MISSING_MONITOR: &str = "The display chosen earlier, not connected now";
    pub const EDGE: &str = "Edge";
    pub const BOTTOM: &str = "Bottom";
    pub const TOP: &str = "Top";
    pub const SIZE: &str = "Size";
    pub const LARGEST: &str = "Largest that fits";
    pub const ONE_SMALLER: &str = "One step smaller";

    /// A display in the list: its name and size, and whether it is the main one.
    pub fn monitor(name: &str, width: u32, height: u32, primary: bool) -> String {
        let main = if primary { ", main" } else { "" };
        format!("{name} ({width}×{height}{main})")
    }

    /// A size's choice, with the strip's scale at it on the display shown, when known.
    pub fn size(label: &str, scale: Option<u32>) -> String {
        match scale {
            Some(scale) => format!("{label} ({scale}×)"),
            None => label.to_owned(),
        }
    }

    // The adapters.
    pub const ADAPTERS_FACT: &str = "These apply to the connected repository. A report path \
        is relative to its folder; with none set, the places listed in the report are read.";
    pub const READ: &str = "Read";
    pub const REPORT_PATH: &str = "Report path";
    pub const NO_PATH: &str = "the usual places";
    pub const APPLY: &str = "Apply";
    pub const CLEAR: &str = "Clear";
    pub const OUTSIDE: &str = "That path is not inside the repository.";
    pub const LOOKING: &str = "Looking at the repository…";
    pub const KNIP_RUN: &str = "Run Knip from this repository";
    pub const KNIP_LATER: &str = "Running Knip comes with a later version.";
    pub const CI: &str = "GitHub Actions";
    pub const CI_REPOSITORY: &str = "Repository";
    pub const CI_TOKEN: &str = "Access token";
    pub const CI_LATER: &str = "Weather from GitHub Actions comes with a later version.";

    pub fn adapter(kind: AdapterKind) -> &'static str {
        match kind {
            AdapterKind::Coverage => "Coverage report",
            AdapterKind::Tests => "Test results",
            AdapterKind::DeadCode => "Dead-code report (Knip)",
        }
    }

    // The hotkeys.
    pub const RAISE: &str = "Raise the strip";
    pub const TIME_LAPSE: &str = "Open the time-lapse";
    pub const KEY: &str = "Key";
    pub const DEFAULT: &str = "Default";

    /// The modifiers' names, as the platform's keyboards print them, in Apple's order.
    pub fn modifier_names(platform: Platform) -> [&'static str; 4] {
        match platform {
            Platform::Mac => ["Control", "Option", "Shift", "Command"],
            Platform::Windows => ["Ctrl", "Alt", "Shift", "Win"],
        }
    }

    pub fn now_set(keys: &str) -> String {
        format!("Now: {keys}")
    }

    pub fn refusal(refusal: Refusal, platform: Platform) -> &'static str {
        match (refusal, platform) {
            (Refusal::TooFewModifiers, _) => "A hotkey takes two modifier keys or more.",
            (Refusal::OptionShift, Platform::Mac) => {
                "macOS refuses hotkeys of Option and Shift alone."
            }
            (Refusal::OptionShift, Platform::Windows) => {
                "Alt and Shift alone type characters on many keyboards."
            }
            (Refusal::Taken, _) => "The other hotkey has these keys.",
        }
    }

    pub fn not_registered(err: &impl Display) -> String {
        format!("These keys could not be registered: {err}")
    }

    // Day, night and dim mode.
    pub const DAY_STARTS: &str = "Day begins";
    pub const NIGHT_STARTS: &str = "Night begins";
    pub const SAME_TIMES: &str = "Day and night begin at different times.";
    pub const CROSSFADE_FACT: &str = "The sky crosses from one to the other over 45 minutes.";
    pub const DIM_AFTER: &str = "Dim after";
    pub const DIM_FACT: &str = "With no input anywhere on this computer for that long, the \
        strip dims and its motion slows.";

    /// An hour or a minute, as a clock shows it.
    pub fn two_digits(value: u16) -> String {
        format!("{value:02}")
    }

    pub fn minutes(minutes: u32) -> String {
        format!("{minutes} minutes")
    }

    // General.
    pub const OPEN_AT_LOGIN: &str = "Open at login";
    pub const CRASH_LOG: &str = "Write a crash log";

    /// What opening at login waits on, when the OS says it does not apply yet.
    pub fn login_status(status: LoginStatus) -> Option<&'static str> {
        match status {
            LoginStatus::RequiresApproval => {
                Some("Waiting for approval in System Settings, under Login Items.")
            }
            LoginStatus::Unavailable => {
                Some("Open at login applies to the installed app; this build is not one.")
            }
            LoginStatus::Enabled | LoginStatus::Disabled => None,
        }
    }

    pub fn login_problem(err: &LoginError) -> String {
        match err {
            LoginError::NotBundled => {
                "Open at login applies to the installed app; this build is not one.".to_owned()
            }
            LoginError::Os(reason) => format!("Open at login could not be set: {reason}"),
        }
    }

    pub fn crash_log_fact(folder: &str) -> String {
        format!(
            "A crash writes a report into {folder}, to attach to an issue. Nothing is sent \
             anywhere."
        )
    }

    pub fn version() -> String {
        format!("{PRODUCT_NAME} {VERSION}")
    }
}

/// The soil panel and the hover labels: the only place numbers and paths are shown. Every line
/// states a fact.
pub mod panel {
    use terrarium_truth::Q16;

    use super::utc_date;

    pub const CLIMATE: &str = "climate";
    pub const LIGHT: &str = "light";
    pub const SOIL: &str = "soil";
    pub const RAIN: &str = "rain";
    pub const WIND: &str = "wind";
    pub const WEATHER: &str = "weather";
    pub const TERRAIN: &str = "terrain";
    pub const AGE: &str = "age";
    pub const DEAD_CODE: &str = "dead code";
    pub const COMPLEXITY: &str = "complexity";
    pub const TODO_DENSITY: &str = "todo density";
    /// The titles of the pages of files.
    pub const UNCOVERED_LINES: &str = "uncovered lines";
    pub const FAILING_TESTS: &str = "failing tests";

    pub const UNKNOWN: &str = "unknown";
    pub const PASS: &str = "pass";
    pub const FAIL: &str = "fail";

    /// A page with nothing to list says why, in one line.
    pub const NO_COVERAGE_REPORT: &str = "no coverage report found";
    pub const COVERAGE_NOT_READ: &str = "no coverage read yet";
    pub const NO_UNCOVERED_LINES: &str = "no uncovered lines";
    pub const NO_TEST_REPORT: &str = "no test report found";
    pub const TESTS_NOT_READ: &str = "no test run read yet";
    pub const NO_FAILING_TESTS: &str = "no failing tests";
    pub const FAILING_UNNAMED: &str = "the failing tests are not named";
    pub const NOT_MEASURED: &str = "not measured yet";
    pub const COVERAGE_OFF: &str = "coverage is off in settings";
    pub const TESTS_OFF: &str = "tests are off in settings";
    pub const NO_FILES: &str = "no files named";

    /// A reading's value to two decimals: `0.78`, or `1.20` for rain at a fifth over its usual
    /// pace.
    pub fn value(value: Q16) -> String {
        let hundredths = (i64::from(value.raw()) * 100 + (1 << 15)).div_euclid(1 << 16);
        let sign = if hundredths < 0 { "-" } else { "" };
        let hundredths = hundredths.abs();
        format!("{sign}{}.{:02}", hundredths / 100, hundredths % 100)
    }

    /// Lines of code: `912 lines`, `48k lines`, `1.2m lines`.
    pub fn lines(lines: u64) -> String {
        match lines {
            1 => "1 line".to_owned(),
            0..1000 => format!("{lines} lines"),
            1000..1_000_000 => format!("{}k lines", lines / 1000),
            _ => {
                let tenths = lines / 100_000;
                format!("{}.{}m lines", tenths / 10, tenths % 10)
            }
        }
    }

    /// How long since the first commit, `days` whole days: `12 days`, `8 months`, `3 years`.
    pub fn age(days: i64) -> String {
        let (n, unit) = match days.max(0) {
            days @ 0..61 => (days, "day"),
            days @ 61..730 => (days / 30, "month"),
            days => (days / 365, "year"),
        };
        if n == 1 {
            format!("1 {unit}")
        } else {
            format!("{n} {unit}s")
        }
    }

    /// The reading taken on another branch than the default, shown beside light.
    pub fn on_branch(branch: &str) -> String {
        format!("on {branch}")
    }

    /// A file's weight: its uncovered lines, failing tests, unused items.
    pub fn count(value: i64) -> String {
        value.to_string()
    }

    /// The year a moment fell in, in UTC: a monument's label.
    pub fn year(secs: i64) -> String {
        utc_date(secs).year.to_string()
    }

    /// A fossil's label: the directory it was, and the year it was deleted.
    pub fn fossil(path: Option<&str>, secs: i64) -> String {
        match path {
            Some(path) => format!("{path} - {}", year(secs)),
            None => year(secs),
        }
    }
}

/// A second launch found the app running, and asked it to show its settings.
pub fn shown_in_running() -> String {
    format!("{BINARY_NAME}: already running; its settings are shown")
}

/// A second launch with `--repo` found the app running: it connects nothing.
pub fn repo_while_running() -> String {
    format!("{BINARY_NAME}: already running; --repo connects only when it starts the app")
}

pub fn instance_unavailable(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the data directory's lock could not be taken: {err}")
}

pub fn request_failed(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the running app could not be asked to show its settings: {err}")
}

/// A second launch's requests are not watched, so only a restart shows the settings.
pub fn requests_unwatched(err: &impl Display) -> String {
    format!("{BINARY_NAME}: a second launch cannot reach this one: {err}")
}

/// "Open at login" was checked but not set.
pub fn login_not_set(err: &LoginError) -> String {
    match err {
        LoginError::NotBundled => format!(
            "{BINARY_NAME}: open at login applies to the installed app; this build is not one"
        ),
        LoginError::Os(reason) => {
            format!("{BINARY_NAME}: open at login could not be set: {reason}")
        }
    }
}

/// Connecting from the window went on without its report.
pub fn report_unavailable() -> &'static str {
    "The report could not be made."
}

pub fn menu_unavailable(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the right-click menu is unavailable: {err}")
}

pub fn hotkey_unavailable(which: Which, binding: Binding, err: &impl Display) -> String {
    let name = match which {
        Which::Raise => "raise",
        Which::TimeLapse => "time-lapse",
    };
    format!(
        "{BINARY_NAME}: the {name} hotkey {} could not be registered: {err}",
        hotkey(binding, Platform::THIS)
    )
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

#[cfg(feature = "dev")]
pub fn menu_debug() -> &'static str {
    "Debug window"
}

/// The time-lapse window's words, and the date its label shows.
pub mod timelapse {
    use std::fmt::Display;
    use std::path::Path;

    use terrarium_compose::sky::LocalDate;
    use terrarium_engine::timelapse::LapseError;
    use terrarium_truth::ReplayError;

    use super::MONTHS;

    pub const FROM: &str = "From";
    pub const TO: &str = "To";
    pub const PLAY: &str = "Play";
    pub const STOP: &str = "Stop";
    pub const BY_DAY: &str = "By day";
    pub const BY_NIGHT: &str = "By night";
    pub const SCALE: &str = "Scale";
    pub const SAVE_GIF: &str = "Save GIF…";
    pub const SAVE_POSTCARD: &str = "Save postcard…";
    pub const GIF: &str = "GIF";
    pub const PNG: &str = "PNG";
    pub const PREPARING: &str = "Preparing the days";
    pub const NO_DAY_YET: &str = "No day of this history has ended yet.";
    pub const ANOTHER_REPOSITORY: &str =
        "The folder holds another repository than this terrarium's.";
    pub const OUTSIDE_HISTORY: &str = "This moment is outside the repository's history.";
    pub const HALTED: &str = "The time-lapse stopped on an error.";

    /// The history being read, and how far.
    pub fn reading(percent: Option<u8>) -> String {
        match percent {
            Some(percent) => format!("Reading the history: {percent}%"),
            None => "Reading the history".to_owned(),
        }
    }

    /// Why the history could not be read; nothing, when its window closed first.
    pub fn failed(err: &LapseError) -> String {
        match err {
            LapseError::NotThisRepository => ANOTHER_REPOSITORY.to_owned(),
            LapseError::BeforeHistory => OUTSIDE_HISTORY.to_owned(),
            LapseError::Git(err) => format!("The history could not be read: {err}"),
            LapseError::Store(err) => format!("The observations could not be read: {err}"),
        }
    }

    pub fn replay_failed(err: &ReplayError) -> String {
        format!("The replay could not run: {err:?}")
    }

    /// A day in the label: its date, or, in a time-lapse longer than its days can be read
    /// in, its month.
    pub fn date(date: LocalDate, by_day: bool) -> String {
        let month = MONTHS[usize::from(date.month - 1)];
        if by_day {
            format!("{} {month} {}", date.day, date.year)
        } else {
            format!("{month} {}", date.year)
        }
    }

    /// A day as the window's sliders name it.
    pub fn day(date: LocalDate) -> String {
        self::date(date, true)
    }

    pub fn scale(k: u32) -> String {
        format!("{k}×")
    }

    pub fn writing(percent: u8) -> String {
        format!("Writing the GIF: {percent}%")
    }

    pub fn saved(path: &Path) -> String {
        format!(
            "Saved {}",
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
        )
    }

    pub fn not_saved(err: &impl Display) -> String {
        format!("Not saved: {err}")
    }

    /// The GIF's name, as the save dialog offers it.
    pub fn gif_name(repo: &str) -> String {
        format!("{repo}-time-lapse.gif")
    }

    /// The postcard's name, as the save dialog offers it.
    pub fn postcard_name(repo: &str, date: LocalDate) -> String {
        format!(
            "{repo}-{:04}-{:02}-{:02}.png",
            date.year, date.month, date.day
        )
    }
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
    use crate::egui_probe::{Phase, Sample};

    pub const SCENE: &str = "Scene";
    pub const LIVE: &str = "The connected repository, as the engine sends it";
    pub const THRIVING: &str = "Thriving, the sample";
    pub const DEMO: &str = "The demo cycle, from its start";
    pub const SYNTHETIC: &str = "Synthetic climate";
    pub const SIMULATION: &str = "Simulation";
    pub const SIMULATION_NOTE: &str = "The real sim, a day at a time, over a history these \
        sliders make. Rain and wind are measured against the usual pace, so one held high \
        settles back to usual. Haze is a test run that tells nothing. Soil rising fast clears \
        the dead code: a clearing burn.";
    pub const SPEED: &str = "Days a second (0 holds)";
    pub const TAG_RELEASE: &str = "Tag a release";
    pub const DELETE_DIRECTORY: &str = "Delete a directory";
    pub const REWRITE_HISTORY: &str = "Rewrite the default branch";
    pub const START_OVER: &str = "Start over";
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
    pub const EVENT_TRIGGERS: &str = "Each press plays the event once, with what it brings \
        to the sky and the ground.";
    pub const CLEAR_EVENT: &str = "None";
    pub const CLOCK: &str = "Clock";
    pub const FIXED_TIME: &str = "A fixed time of day";
    pub const DIM: &str = "Dim";
    pub const DIM_IDLE: &str = "after idle time";
    pub const DIM_ON: &str = "on";
    pub const DIM_OFF: &str = "off";
    pub const READOUT: &str = "Readout";

    pub fn preset(name: &str) -> String {
        format!("Preset: {name}")
    }

    pub fn weather(weather: Weather) -> &'static str {
        super::weather_word(weather)
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
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the slider holds whole minutes of one day"
        )]
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
        if let Some(day) = readout.sim_day {
            lines.push(format!("simulated day: {day}"));
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

    /// The egui memory report's file, in the directory the probe ran from.
    pub fn egui_memory_file(started_unix: i64) -> String {
        format!("egui-memory-{started_unix}.txt")
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
    use crate::probe::report::{EndReason, Report, StepReport};

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
    fn the_panel_s_values_read_plainly_at_their_edges() {
        use terrarium_truth::Q16;
        let value = |num, den| panel::value(Q16::from_ratio(num, den));
        assert_eq!(value(0, 1), "0.00");
        assert_eq!(value(1, 1), "1.00");
        assert_eq!(value(78, 100), "0.78");
        assert_eq!(value(6, 5), "1.20");
        assert_eq!(value(1, 200), "0.01", "rounded to the nearest hundredth");
        assert_eq!(value(-1, 4), "-0.25");
        assert_eq!(panel::lines(0), "0 lines");
        assert_eq!(panel::lines(1), "1 line");
        assert_eq!(panel::lines(999), "999 lines");
        assert_eq!(panel::lines(1000), "1k lines");
        assert_eq!(panel::lines(48_210), "48k lines");
        assert_eq!(panel::lines(999_999), "999k lines");
        assert_eq!(panel::lines(1_000_000), "1.0m lines");
        assert_eq!(panel::lines(1_250_000), "1.2m lines");
        assert_eq!(panel::age(0), "0 days");
        assert_eq!(panel::age(1), "1 day");
        assert_eq!(panel::age(60), "60 days");
        assert_eq!(panel::age(61), "2 months");
        assert_eq!(panel::age(729), "24 months");
        assert_eq!(panel::age(730), "2 years");
        assert_eq!(panel::age(-5), "0 days", "a clock behind the root");
        // 2023-04-03, 12:00 UTC.
        assert_eq!(panel::year(1_680_523_200), "2023");
        assert_eq!(
            panel::fossil(Some("src/old"), 1_680_523_200),
            "src/old - 2023"
        );
        assert_eq!(panel::fossil(None, 1_680_523_200), "2023");
    }

    #[test]
    fn a_hotkey_is_worded_as_each_platform_names_its_keys() {
        let raise = Which::Raise.default_binding();
        assert_eq!(hotkey(raise, Platform::Mac), "Control-Option-T");
        assert_eq!(hotkey(raise, Platform::Windows), "Ctrl+Alt+T");
        let every = Binding::parse("ctrl+alt+shift+super+Space").unwrap();
        assert_eq!(
            hotkey(every, Platform::Mac),
            "Control-Option-Shift-Command-Space"
        );
        assert_eq!(hotkey(every, Platform::Windows), "Ctrl+Alt+Shift+Win+Space");
        assert_eq!(
            settings::size(settings::LARGEST, Some(4)),
            "Largest that fits (4×)"
        );
        assert_eq!(
            settings::size(settings::ONE_SMALLER, None),
            "One step smaller"
        );
    }

    #[test]
    fn copy_has_no_exclamation_marks() {
        let mut literals = string_literals(include_str!("copy.rs"));
        literals.extend(string_literals(include_str!("copy/probe.rs")));
        assert!(
            literals.iter().any(|l| l == "Repo Terrarium"),
            "the scan found {literals:?}"
        );
        assert!(
            literals.iter().any(|l| l.contains("pinning probe")),
            "the scan missed the probe's words"
        );
        for literal in literals {
            assert!(!literal.contains('!'), "copy must not exclaim: {literal:?}");
        }
    }

    /// What the world never says (I-8): praise, shame, nagging, instructions or notifications,
    /// as the words that carry them.
    const BANNED: &[&str] = &[
        "great",
        "awesome",
        "amazing",
        "congrat",
        "well done",
        "good job",
        "keep it up",
        "proud",
        "shame",
        "sadly",
        "unfortunately",
        "oops",
        "don't forget",
        "remember to",
        "reminder",
        "you should",
        "you need",
        "please",
        "notification",
        "alert",
    ];

    /// The words that ship, above this test module, so the list above is not among them. The
    /// probe's words are a tester's instructions, so `copy/probe.rs` is left out.
    #[test]
    fn copy_has_no_banned_words() {
        let source = include_str!("copy.rs");
        let (shipped, _) = source
            .split_once("\n#[cfg(test)]\nmod tests {")
            .expect("the test module follows the words");
        let literals = string_literals(shipped);
        assert!(literals.iter().any(|l| l == "Repo Terrarium"));
        for literal in literals {
            let lower = literal.to_lowercase();
            for word in BANNED {
                assert!(
                    !lower.contains(word),
                    "copy must not say {word:?}: {literal:?}"
                );
            }
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

        use super::super::{DEVICE_LOSS, Hints, discovery_report, report_rows};
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
                        suite_failed: false,
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
                report_rows(&discovery, &reads, Hints::Shown),
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
        fn the_report_says_where_light_soil_and_weather_are_kept_and_hints_only_when_asked() {
            let everything = Discovery {
                coverage: found(Format::Lcov, "coverage/lcov.info", &["coverage/lcov.info"]),
                local_tests: true,
                dead_code: found(Format::Knip, "knip-report.json", &["knip-report.json"]),
                ci_workflows: vec![".github/workflows/ci.yml".to_owned()],
                ..nothing()
            };
            let little = nothing();
            for discovery in [&everything, &little] {
                for hints in [Hints::Shown, Hints::Left] {
                    let text = discovery_report(discovery, &no_reads(), hints);
                    assert!(text.ends_with(&format!("\n{DEVICE_LOSS}\n")), "{text}");
                    assert_eq!(
                        text.contains("To connect"),
                        hints == Hints::Shown && discovery == &little
                    );
                }
            }
            assert!(discovery_report(&little, &no_reads(), Hints::Left).contains("Not found"));
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
            let text = report_rows(&discovery, &no_reads(), Hints::Shown);
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
                 or cobertura.xml. Of several there, the newest is read.\n"
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
            assert!(!report_rows(&recorded, &no_reads(), Hints::Shown).contains("  test report"));
        }

        #[test]
        fn one_file_of_many_and_a_suite_failure_read_plainly() {
            let discovery = Discovery {
                test_report: found(
                    Format::JUnit,
                    "target/surefire-reports/TEST-*.xml",
                    &["target/surefire-reports/TEST-moss.FernTest.xml"],
                ),
                ..nothing()
            };
            let reads = Reads {
                test_report: Some(known(
                    Format::JUnit,
                    &["target/surefire-reports/TEST-moss.FernTest.xml"],
                    Reading::Tests {
                        tests: 1,
                        failed: 0,
                        skipped: 0,
                        suite_failed: true,
                    },
                )),
                ..no_reads()
            };
            assert!(report_rows(&discovery, &reads, Hints::Shown).contains(
                "  test report       target/surefire-reports/TEST-*.xml, 1 JUnit file: 1 test, \
                 none failing, but a suite failed outside its tests\n"
            ));
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
                let text = report_rows(&discovery, &reads, Hints::Shown);
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
                report_rows(&knip, &old, Hints::Shown).contains(
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
            assert!(report_rows(&knip, &read, Hints::Shown).contains(
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
