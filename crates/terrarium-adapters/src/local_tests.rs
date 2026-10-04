//! The status file `repo-terrarium record-tests -- <cmd>` writes: one per repository,
//! `local-tests/<seed-hex>.json` in the data directory, never in the repository.
//!
//! It says how the last recorded run ended. Exit code 0 is a passing run and any other a failing
//! one, as a CI job reads it. A run a signal ended reported nothing, so it writes no file and the
//! last one stands. Keys a later version adds are ignored; a file that is not this format is
//! malformed.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::report::{Format, Loaded, Parsed, Reading, Report, Why, read_bounded, text};

/// The `format` the file declares.
pub const FORMAT: &str = "repo-terrarium-local-tests";
pub const FORMAT_VERSION: u32 = 1;

/// One recorded run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub format: String,
    pub format_version: u32,
    pub exit_code: i64,
    /// Unix seconds, UTC.
    pub started_at: i64,
    pub finished_at: i64,
}

impl Status {
    pub fn new(exit_code: i64, started_at: i64, finished_at: i64) -> Status {
        Status {
            format: FORMAT.to_owned(),
            format_version: FORMAT_VERSION,
            exit_code,
            started_at,
            finished_at,
        }
    }

    /// Writes the file whole or not at all: a temporary file beside it, then a rename, so a
    /// reader never sees half of one.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        let dir = path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir)?;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
        let mut text = serde_json::to_string(self).map_err(io::Error::other)?;
        text.push('\n');
        let written = fs::write(&temp, text).and_then(|()| fs::rename(&temp, path));
        if written.is_err() {
            let _ = fs::remove_file(&temp);
        }
        written
    }
}

/// Reads the status file at `path`. The report names it by its last two parts
/// (`local-tests/<seed-hex>.json`), never by the data directory's full path.
pub fn read(path: &Path) -> Report {
    let shown = shown(path);
    let outcome =
        read_bounded(path, &shown, Format::LocalTests.max_file_bytes()).map(|file| vec![file]);
    crate::report::finish(Path::new(""), Format::LocalTests, vec![shown], outcome)
}

fn shown(path: &Path) -> String {
    let mut parts: Vec<String> = path
        .iter()
        .rev()
        .take(2)
        .map(|part| part.to_string_lossy().into_owned())
        .collect();
    parts.reverse();
    parts.join("/")
}

pub(crate) fn parse(files: &[Loaded]) -> Parsed {
    let [file] = files else {
        return Err(Why::Missing);
    };
    let malformed = || Why::Malformed {
        path: file.path.clone(),
        line: None,
    };
    let status: Status = serde_json::from_str(&text(file)?).map_err(|_| malformed())?;
    if status.format != FORMAT || status.format_version != FORMAT_VERSION {
        return Err(malformed());
    }
    Ok((
        Reading::LocalRun {
            exit_code: status.exit_code,
        },
        Vec::new(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Outcome;

    #[test]
    fn a_written_status_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local-tests").join("ab12.json");
        Status::new(0, 100, 160).write(&path).unwrap();
        let report = read(&path);
        assert_eq!(report.files, ["local-tests/ab12.json"]);
        let known = report.known().expect("known");
        assert_eq!(known.reading, Reading::LocalRun { exit_code: 0 });
        assert_eq!(known.reading.passed(), Some(true));
        Status::new(101, 200, 260).write(&path).unwrap();
        let known = read(&path).known().cloned().expect("known");
        assert_eq!(known.reading.passed(), Some(false));
        // Only the file itself is left: the temporary one was renamed.
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn the_text_is_pinned() {
        // Stored data a later version must still read.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        Status::new(2, 10, 20).write(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\"format\":\"repo-terrarium-local-tests\",\"format_version\":1,\"exit_code\":2,\
             \"started_at\":10,\"finished_at\":20}\n"
        );
    }

    #[test]
    fn anything_else_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("local-tests")).unwrap();
        let path = dir.path().join("local-tests/s.json");
        assert_eq!(read(&path).outcome, Outcome::Unknown(Why::Missing));
        for text in [
            "",
            "{",
            "[]",
            r#"{"format":"repo-terrarium-local-tests","format_version":2,"exit_code":0,"started_at":1,"finished_at":2}"#,
            r#"{"format":"other","format_version":1,"exit_code":0,"started_at":1,"finished_at":2}"#,
            r#"{"format":"repo-terrarium-local-tests","format_version":1,"started_at":1,"finished_at":2}"#,
        ] {
            fs::write(&path, text).unwrap();
            assert!(
                matches!(read(&path).outcome, Outcome::Unknown(Why::Malformed { .. })),
                "{text}"
            );
        }
        let later = r#"{"format":"repo-terrarium-local-tests","format_version":1,"exit_code":1,"started_at":1,"finished_at":2,"note":"x"}"#;
        fs::write(&path, later).unwrap();
        assert!(read(&path).known().is_some(), "an added key is ignored");
        fs::write(&path, " ".repeat(5_000)).unwrap();
        assert_eq!(
            read(&path).outcome,
            Outcome::Unknown(Why::TooLarge("local-tests/s.json".into()))
        );
    }
}
