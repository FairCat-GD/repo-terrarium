//! The opt-in crash log (`crashlog.opt_in`). While the box is checked, a panic on any thread, a
//! truth panic the engine catches among them (I-16), leaves a report in the data directory's
//! `logs/` for the person to attach to an issue. While it is unchecked nothing is written, and
//! nothing is ever sent anywhere.
//!
//! The hook only formats and writes. The folder is pruned at start-up and when the box is
//! checked, never inside a panic.

use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::panic::{self, PanicHookInfo};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use terrarium_compose::sky::LocalDate;

use crate::copy;

/// How many reports the folder keeps: the newest.
pub const KEEP: usize = 10;
const PREFIX: &str = "crash-";
const SUFFIX: &str = ".log";
const SECONDS_PER_DAY: i64 = 86_400;

static ON: AtomicBool = AtomicBool::new(false);
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// What a report says of one panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crash {
    pub thread: String,
    pub location: String,
    pub message: String,
    pub backtrace: String,
}

/// Whether a panic from now on is written.
pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

/// Installs the hook into `dir`, writing while `on`, before the hook already installed, which
/// still runs after it (the default prints to stderr). Only the first call installs it.
pub fn install(dir: PathBuf, on: bool) {
    set_enabled(on);
    if DIR.set(dir).is_err() {
        return;
    }
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if ON.load(Ordering::Relaxed)
            && let Some(dir) = DIR.get()
        {
            let _ = write(dir, &crash(info), crate::now_secs(), std::process::id());
        }
        previous(info);
    }));
}

/// The report of the panic `info` describes, on the thread it happened on.
fn crash(info: &PanicHookInfo<'_>) -> Crash {
    let thread = std::thread::current();
    Crash {
        thread: thread.name().unwrap_or_default().to_owned(),
        location: info.location().map(ToString::to_string).unwrap_or_default(),
        message: info.payload_as_str().unwrap_or_default().to_owned(),
        backtrace: std::backtrace::Backtrace::force_capture().to_string(),
    }
}

/// Writes `crash` into `dir` as a new file named for `unix_secs` and `pid`, creating the folder;
/// a name already taken gets a number after it. Returns the file written.
pub fn write(dir: &Path, crash: &Crash, unix_secs: i64, pid: u32) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stem = stem(unix_secs, pid);
    let text = copy::crash_report(crash);
    let mut last = None;
    for n in 0..10 {
        let name = match n {
            0 => format!("{stem}{SUFFIX}"),
            n => format!("{stem}-{n}{SUFFIX}"),
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(text.as_bytes())?;
                return Ok(path);
            }
            Err(err) => last = Some(err),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::from(io::ErrorKind::AlreadyExists)))
}

/// A report's file name before its suffix: the UTC moment, so names sort by time, and the
/// process.
fn stem(unix_secs: i64, pid: u32) -> String {
    let date = LocalDate::from_days_since_epoch(unix_secs.div_euclid(SECONDS_PER_DAY));
    let second = unix_secs.rem_euclid(SECONDS_PER_DAY);
    format!(
        "{PREFIX}{:04}{:02}{:02}T{:02}{:02}{:02}Z-{pid}",
        date.year,
        date.month,
        date.day,
        second / 3_600,
        second / 60 % 60,
        second % 60
    )
}

/// Removes all but the newest `keep` reports in `dir`. Anything else there stays.
pub fn prune(dir: &Path, keep: usize) -> io::Result<()> {
    let mut reports: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(PREFIX) && name.ends_with(SUFFIX))
            })
            .collect(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    reports.sort();
    let excess = reports.len().saturating_sub(keep);
    for path in reports.into_iter().take(excess) {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crash() -> Crash {
        Crash {
            thread: "terrarium-engine".into(),
            location: "crates/terrarium-truth/src/sim.rs:12:5".into(),
            message: "attempt to add with overflow".into(),
            backtrace: "0: terrarium_truth::sim::step".into(),
        }
    }

    #[test]
    fn a_report_is_named_by_its_utc_moment_so_names_sort_by_time() {
        // 2026-10-11 12:34:56 UTC.
        assert_eq!(stem(1_791_722_096, 42), "crash-20261011T123456Z-42");
        assert_eq!(stem(0, 7), "crash-19700101T000000Z-7");
        let earlier = stem(1_791_722_096, 900);
        let later = stem(1_791_722_097, 1);
        assert!(earlier < later);
    }

    #[test]
    fn a_report_holds_every_fact_and_never_overwrites_another() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        let first = write(&logs, &crash(), 1_791_722_096, 42).unwrap();
        let second = write(&logs, &crash(), 1_791_722_096, 42).unwrap();
        assert_ne!(first, second);
        let text = fs::read_to_string(&first).unwrap();
        for fact in [
            env!("CARGO_PKG_VERSION"),
            "terrarium-engine",
            "crates/terrarium-truth/src/sim.rs:12:5",
            "attempt to add with overflow",
            "terrarium_truth::sim::step",
            std::env::consts::OS,
        ] {
            assert!(text.contains(fact), "{fact} missing from {text}");
        }
    }

    #[test]
    fn pruning_keeps_the_newest_reports_and_nothing_else_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        for second in 0..13 {
            write(dir.path(), &crash(), 1_791_722_000 + second, 1).unwrap();
        }
        fs::write(dir.path().join("notes.txt"), "kept").unwrap();
        prune(dir.path(), KEEP).unwrap();
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names.len(), KEEP + 1);
        assert_eq!(names.last().map(String::as_str), Some("notes.txt"));
        assert_eq!(names[0], format!("{}{SUFFIX}", stem(1_791_722_003, 1)));
        // A folder not made yet has nothing to prune.
        prune(&dir.path().join("none"), KEEP).unwrap();
    }
}
