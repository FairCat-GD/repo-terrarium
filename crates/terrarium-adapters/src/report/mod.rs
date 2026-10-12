//! Report files: coverage (lcov, Cobertura), test results (`JUnit` XML) and dead code (Knip
//! JSON), read from the repository's worktree and never executed.
//!
//! A report is either *known*, read in full and saying something, or *unknown*, with the reason.
//! Unknown covers a missing file, one that is not a regular file, one over its size cap, one
//! that does not parse (a file caught half-written included, which is why every parser insists
//! on the document's end), and one that parses but says nothing: no instrumented line, no test
//! that ran, a Knip report from before 6.0. An unknown report is never recorded, so the last
//! reading holds and turns stale once the default tip moves past it (I-3): stale is never
//! worse, and a file caught mid-write cannot reset a reading's average.
//!
//! Every path here is relative to the repository root, with `/` separators.

mod cobertura;
mod junit;
mod knip;
mod lcov;
mod paths;
#[cfg(test)]
mod tests;
mod xml;

use std::borrow::Cow;
use std::fs;
use std::io::{self, Read as _};
use std::path::Path;
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};
use terrarium_truth::{ObservationKind, Q16, Q32};

pub use knip::{KNIP_MIN_MAJOR, KnipCounts, installed_knip};
pub(crate) use paths::relative_parts;

/// At most this many contributors per report.
pub const MAX_CONTRIBUTORS: usize = 20;
/// At most this many files are read for one report (a surefire run writes one per class).
pub(crate) const MAX_FILES: usize = 10_000;
/// At most this many bytes, over every file of one report.
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
/// A contributor path longer than this is left out: no source file has one.
const MAX_CONTRIBUTOR_PATH: usize = 1024;

/// The report formats, each its own adapter with its own version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Format {
    Lcov,
    Cobertura,
    JUnit,
    Knip,
    /// The status file `repo-terrarium record-tests` writes.
    LocalTests,
}

impl Format {
    /// The observation kind the format feeds.
    pub const fn kind(self) -> ObservationKind {
        match self {
            Format::Lcov | Format::Cobertura => ObservationKind::Coverage,
            Format::JUnit | Format::LocalTests => ObservationKind::Weather,
            Format::Knip => ObservationKind::DeadCode,
        }
    }

    /// The `adapter_version` its observations carry. It changes when the same report would be
    /// read differently.
    pub const fn adapter_version(self) -> u32 {
        1
    }

    /// The largest file of this format that is read.
    pub const fn max_file_bytes(self) -> u64 {
        match self {
            Format::Lcov | Format::Cobertura => 64 * 1024 * 1024,
            Format::JUnit | Format::Knip => 16 * 1024 * 1024,
            Format::LocalTests => 4 * 1024,
        }
    }
}

/// One read of a report: the files it came from and what they said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub format: Format,
    /// The files read, relative to the repository root.
    pub files: Vec<String>,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Known(Known),
    Unknown(Why),
}

/// A report read in full that says something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Known {
    pub reading: Reading,
    /// The files that weigh most in the reading, at most [`MAX_CONTRIBUTORS`], heaviest first.
    pub contributors: Vec<Contributor>,
    /// SHA-256 of the report's bytes; over several files, of each file's path, length and bytes
    /// in path order.
    pub content_hash: [u8; 32],
    /// The newest modification time of the files, as Unix seconds.
    pub mtime: Option<i64>,
}

/// What a report says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reading {
    /// Lines instrumented and lines run; `lines_found` is never zero.
    Coverage { lines_found: u64, lines_hit: u64 },
    /// Test cases, those that failed or errored, and those skipped; `suite_failed` when a
    /// failure stood outside any test case. Some test ran, or something failed.
    Tests {
        tests: u64,
        failed: u64,
        skipped: u64,
        suite_failed: bool,
    },
    /// A `record-tests` run and its exit code.
    LocalRun { exit_code: i64 },
    /// What Knip found unused.
    DeadCode(KnipCounts),
}

impl Reading {
    /// The health-oriented value: line coverage, or 1 for passing and 0 for failing tests. Dead
    /// code has none here: its formula needs the number of exported items, which a Knip report
    /// does not hold; the source metrics count them
    /// ([`Measurement::dead_code`](crate::metrics::Measurement::dead_code)).
    pub fn normalized(&self) -> Option<Q16> {
        match self {
            Reading::Coverage {
                lines_found,
                lines_hit,
            } => Some(ratio(*lines_hit, *lines_found)),
            Reading::Tests { .. } | Reading::LocalRun { .. } => {
                Some(if self.passed() == Some(true) {
                    Q16::ONE
                } else {
                    Q16::ZERO
                })
            }
            Reading::DeadCode(_) => None,
        }
    }

    /// Whether the tests passed: a run with no failure, or an exit code of 0 (any other code
    /// fails, as it fails a CI job).
    pub fn passed(&self) -> Option<bool> {
        match self {
            Reading::Tests {
                failed,
                suite_failed,
                ..
            } => Some(*failed == 0 && !*suite_failed),
            Reading::LocalRun { exit_code } => Some(*exit_code == 0),
            Reading::Coverage { .. } | Reading::DeadCode(_) => None,
        }
    }
}

/// `num / den` in `[0, 1]`, rounded, for counts of any size.
pub(crate) fn ratio(num: u64, den: u64) -> Q16 {
    let (mut num, mut den) = (num.min(den), den);
    while den > i64::MAX.unsigned_abs() {
        num >>= 1;
        den >>= 1;
    }
    match (i64::try_from(num), i64::try_from(den)) {
        (Ok(num), Ok(den)) if den > 0 => Q32::from_ratio(num, den).to_q16().clamp01(),
        _ => Q16::ZERO,
    }
}

/// A file that weighs in a reading, with its weight: uncovered lines, failing tests, unused
/// items.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Contributor {
    pub path: String,
    pub value: i64,
}

/// Why a report says nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    /// No file matched.
    Missing,
    /// A configured path that is absolute or leaves the repository.
    OutsideRepository(String),
    /// The path is a directory, a device, a pipe or a broken link.
    NotAFile(String),
    /// More bytes or files than the format reads.
    TooLarge(String),
    /// Reading failed.
    Io { path: String, kind: io::ErrorKind },
    /// The file does not parse as its format, or ends before its document does; `line` is
    /// 1-based when known.
    Malformed { path: String, line: Option<u64> },
    /// It parses, but measured nothing: no instrumented line, or no test that ran.
    Empty,
    /// A Knip report from before 6.0, whose format this does not read. `version` is the
    /// installed Knip's, when known.
    KnipBefore6 { version: Option<String> },
}

impl Report {
    /// Reads the report of `format` made of `files`, relative to `root`.
    pub fn read(root: &Path, format: Format, files: &[String]) -> Report {
        finish(root, format, files.to_vec(), load(root, format, files))
    }

    pub fn known(&self) -> Option<&Known> {
        match &self.outcome {
            Outcome::Known(known) => Some(known),
            Outcome::Unknown(_) => None,
        }
    }
}

/// One file's bytes.
pub(crate) struct Loaded {
    pub(crate) path: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) mtime: Option<i64>,
}

fn load(root: &Path, format: Format, files: &[String]) -> Result<Vec<Loaded>, Why> {
    if files.is_empty() {
        return Err(Why::Missing);
    }
    if files.len() > MAX_FILES {
        return Err(Why::TooLarge(files[0].clone()));
    }
    let mut sorted: Vec<&String> = files.iter().collect();
    sorted.sort();
    sorted.dedup();
    let mut total = 0_u64;
    let mut loaded = Vec::with_capacity(sorted.len());
    for path in sorted {
        if paths::relative_parts(path).is_none() {
            return Err(Why::OutsideRepository(path.clone()));
        }
        let file = read_bounded(&root.join(path), path, format.max_file_bytes())?;
        total = total.saturating_add(u64::try_from(file.bytes.len()).unwrap_or(u64::MAX));
        if total > MAX_TOTAL_BYTES {
            return Err(Why::TooLarge(path.clone()));
        }
        loaded.push(file);
    }
    Ok(loaded)
}

/// Reads a regular file of at most `limit` bytes. `shown` names it in the reason.
pub(crate) fn read_bounded(full: &Path, shown: &str, limit: u64) -> Result<Loaded, Why> {
    let io_err = |err: io::Error| Why::Io {
        path: shown.to_owned(),
        kind: err.kind(),
    };
    // Following links, so a link to a regular file reads; a pipe or a device would block or
    // never end, so only regular files are opened.
    let meta = match fs::metadata(full) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Err(Why::Missing),
        Err(err) => return Err(io_err(err)),
    };
    if !meta.is_file() {
        return Err(Why::NotAFile(shown.to_owned()));
    }
    if meta.len() > limit {
        return Err(Why::TooLarge(shown.to_owned()));
    }
    let mut bytes = Vec::new();
    fs::File::open(full)
        .and_then(|file| file.take(limit.saturating_add(1)).read_to_end(&mut bytes))
        .map_err(io_err)?;
    // The file grew between the size check and the read.
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(Why::TooLarge(shown.to_owned()));
    }
    let mtime = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|since| i64::try_from(since.as_secs()).ok());
    Ok(Loaded {
        path: shown.to_owned(),
        bytes,
        mtime,
    })
}

/// SHA-256 of one file's bytes, or of every file's path, length and bytes, in path order.
pub(crate) fn content_hash(files: &[Loaded]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    if let [only] = files {
        hasher.update(&only.bytes);
    } else {
        for file in files {
            hasher.update(file.path.as_bytes());
            hasher.update([0]);
            hasher.update(
                u64::try_from(file.bytes.len())
                    .unwrap_or(u64::MAX)
                    .to_le_bytes(),
            );
            hasher.update(&file.bytes);
        }
    }
    hasher.finalize().into()
}

/// The report of `format` from what was loaded.
pub(crate) fn finish(
    root: &Path,
    format: Format,
    files: Vec<String>,
    loaded: Result<Vec<Loaded>, Why>,
) -> Report {
    let outcome = match loaded {
        Ok(loaded) => parse(root, format, &loaded),
        Err(why) => Outcome::Unknown(why),
    };
    Report {
        format,
        files,
        outcome,
    }
}

fn parse(root: &Path, format: Format, files: &[Loaded]) -> Outcome {
    let context = paths::Context::new(root);
    let parsed = match format {
        Format::Lcov => lcov::parse(&context, files),
        Format::Cobertura => cobertura::parse(&context, files),
        Format::JUnit => junit::parse(&context, files),
        Format::Knip => knip::parse(root, files),
        Format::LocalTests => crate::local_tests::parse(files),
    };
    match parsed {
        Ok((reading, contributors)) => Outcome::Known(Known {
            reading,
            contributors: top(contributors),
            content_hash: content_hash(files),
            mtime: files.iter().filter_map(|file| file.mtime).max(),
        }),
        Err(why) => Outcome::Unknown(why),
    }
}

/// What a parser returns: the reading and every contributor, before the cap.
pub(crate) type Parsed = Result<(Reading, Vec<Contributor>), Why>;

/// A coverage report's lines found and hit, summed over its files (each file's `found` and
/// `hit`, with `hit` held to `found`), and each file the repository holds as a contributor
/// weighed by its lines not run.
pub(crate) fn coverage_totals<'a>(
    files: impl IntoIterator<Item = (u64, u64, Option<&'a String>)>,
) -> (u64, u64, Vec<Contributor>) {
    let mut found = 0_u64;
    let mut hit = 0_u64;
    let mut contributors = Vec::new();
    for (f, h, path) in files {
        let h = h.min(f);
        found = found.saturating_add(f);
        hit = hit.saturating_add(h);
        if let Some(path) = path {
            contributors.push(Contributor {
                path: path.clone(),
                value: i64::try_from(f - h).unwrap_or(i64::MAX),
            });
        }
    }
    (found, hit, contributors)
}

/// The heaviest [`MAX_CONTRIBUTORS`], by weight and then path; nothing weightless, and no path
/// too long to be a source file.
pub(crate) fn top(mut contributors: Vec<Contributor>) -> Vec<Contributor> {
    contributors.retain(|c| c.value > 0 && c.path.len() <= MAX_CONTRIBUTOR_PATH);
    contributors.sort_by(|a, b| b.value.cmp(&a.value).then_with(|| a.path.cmp(&b.path)));
    contributors.truncate(MAX_CONTRIBUTORS);
    contributors
}

/// A file's text: UTF-8, with or without a byte-order mark, or UTF-16 with one (what Windows
/// PowerShell 5's `>` writes). Anything else is [`Why::Malformed`].
pub(crate) fn text(file: &Loaded) -> Result<Cow<'_, str>, Why> {
    let malformed = |line| Why::Malformed {
        path: file.path.clone(),
        line,
    };
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let (pairs, odd) = rest.as_chunks::<2>();
        if !odd.is_empty() {
            return Err(malformed(None));
        }
        char::decode_utf16(pairs.iter().map(|pair| unit(*pair)))
            .collect::<Result<String, _>>()
            .map(Cow::Owned)
            .map_err(|_| malformed(None))
    };
    if let Some(rest) = file.bytes.strip_prefix(&[0xff, 0xfe]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = file.bytes.strip_prefix(&[0xfe, 0xff]) {
        return utf16(rest, u16::from_be_bytes);
    }
    let bytes = file
        .bytes
        .strip_prefix(&[0xef, 0xbb, 0xbf])
        .unwrap_or(&file.bytes);
    std::str::from_utf8(bytes)
        .map(Cow::Borrowed)
        .map_err(|err| malformed(Some(line_at(bytes, err.valid_up_to()))))
}

/// The 1-based line holding byte `offset`.
pub(crate) fn line_at(bytes: &[u8], offset: usize) -> u64 {
    let newlines = bytes[..offset.min(bytes.len())]
        .split(|&b| b == b'\n')
        .count();
    u64::try_from(newlines).unwrap_or(u64::MAX)
}

/// A non-negative count in a report attribute or field: digits only, saturating.
pub(crate) fn count(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(text.bytes().fold(0_u64, |n, digit| {
        n.saturating_mul(10).saturating_add(u64::from(digit - b'0'))
    }))
}

/// A signed execution count (some tools write `-1` for "not executable"): its sign and size.
/// Only whether it is above zero matters.
pub(crate) fn hits(text: &str) -> Option<bool> {
    let text = text.trim();
    match text.strip_prefix('-') {
        Some(rest) => count(rest).map(|_| false),
        None => count(text.strip_prefix('+').unwrap_or(text)).map(|n| n > 0),
    }
}
