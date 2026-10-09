//! `cargo xtask golden`: the golden replay (I-13).
//!
//! The fixture arc is built in a temporary directory and replayed through the engine's headless
//! pipeline with the committed observations, through five months past its last commit. The
//! replay's canonical dump is committed at [`DUMP`], with its SHA-256 beside it at [`SHA`] in
//! `shasum` format. The dump's first line names the `SIM_VERSION` that wrote it.
//!
//! - `--check`, the default and verify's `golden` stage, replays again and compares byte for
//!   byte.
//! - `--bless` rewrites both files and prints what changed, path by path, for review. Truth's
//!   output changes only with a new `SIM_VERSION` (I-13), so it refuses a changed dump written at
//!   the version already on disk, unless `--same-version` says that version's golden has not
//!   been committed yet.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::ExitCode;

use terrarium_adapters::fixture;
use terrarium_engine::headless::{Request, replay_dump};
use terrarium_store::DataDir;
use terrarium_truth::canonical_dump;

use crate::sha256_hex;

/// The golden dump and its hash, relative to the workspace root.
pub const DUMP: &str = "tests/golden/arc.dump";
pub const SHA: &str = "tests/golden/arc.sha256";

/// The most diff lines `--bless` prints before it only counts.
const DIFF_LINES: usize = 200;

#[derive(Clone, Copy)]
pub enum Mode {
    Check,
    Bless { same_version: bool },
}

pub fn run(root: &Path, mode: Mode) -> ExitCode {
    let passed = match mode {
        Mode::Check => check(root),
        Mode::Bless { same_version } => match bless(root, same_version) {
            Ok(()) => true,
            Err(err) => {
                println!("golden: {err}");
                false
            }
        },
    };
    if passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Whether the committed golden files hold a fresh replay's dump.
pub fn check(root: &Path) -> bool {
    let fresh = match build(root) {
        Ok(fresh) => fresh,
        Err(err) => {
            println!("golden: {err}");
            return false;
        }
    };
    let dump = read(root, DUMP);
    let sha = read(root, SHA);
    let problems = problems(dump.as_deref(), sha.as_deref(), &fresh);
    for problem in &problems {
        println!("golden: {problem}");
    }
    if problems.is_empty() {
        println!(
            "golden: {DUMP} holds the arc's replay ({} lines, sha256 {})",
            fresh.lines().count(),
            sha256_hex(fresh.as_bytes())
        );
    }
    problems.is_empty()
}

/// The arc's replay, dumped.
pub fn build(root: &Path) -> Result<String, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let repo = dir.path().join("repo");
    fixture::arc(&repo).map_err(|e| e.to_string())?;
    let observations = root.join(crate::fixture::COMMITTED);
    let file = File::open(&observations)
        .map_err(|e| format!("cannot read {}: {e}", observations.display()))?;
    let mut reader = BufReader::new(file);
    let dump = replay_dump(Request {
        repo: &repo,
        data: &DataDir::at(dir.path().join("data")),
        observations: Some(&mut reader as &mut dyn BufRead),
        at_secs: fixture::ARC_REPLAY_END_SECS,
        install_salt: [0; 16],
    })
    .map_err(|e| format!("the replay failed: {e:?}"))?;
    Ok(canonical_dump(&dump))
}

fn read(root: &Path, path: &str) -> Option<String> {
    std::fs::read_to_string(root.join(path)).ok()
}

/// The hash file's text: one `shasum -a 256` line.
fn sha_line(dump: &str) -> String {
    format!("{}  arc.dump\n", sha256_hex(dump.as_bytes()))
}

/// The `SIM_VERSION` a dump's first line names.
fn header_version(dump: &str) -> Option<u32> {
    dump.lines()
        .next()?
        .strip_prefix("sim_version = ")?
        .parse()
        .ok()
}

fn version_text(dump: &str) -> String {
    header_version(dump).map_or_else(|| "unknown".to_owned(), |v| v.to_string())
}

/// What is wrong with the committed files, against a fresh dump. Empty when they hold it.
fn problems(dump: Option<&str>, sha: Option<&str>, fresh: &str) -> Vec<String> {
    let fix = "run `cargo xtask golden --bless`";
    let mut problems = Vec::new();
    match dump {
        None => problems.push(format!("{DUMP} is missing; {fix}")),
        Some(old) if old != fresh && header_version(old) == header_version(fresh) => {
            problems.push(format!(
                "{DUMP} differs from a fresh replay, both at SIM_VERSION {}: truth's output \
                 changed without a SIM_VERSION bump (I-13)",
                version_text(fresh)
            ));
        }
        Some(old) if old != fresh => problems.push(format!(
            "SIM_VERSION is {} but {DUMP} was written at {}; {fix}",
            version_text(fresh),
            version_text(old)
        )),
        Some(_) => {}
    }
    match (sha, dump) {
        (None, _) => problems.push(format!("{SHA} is missing; {fix}")),
        (Some(sha), Some(dump)) if sha != sha_line(dump) => {
            problems.push(format!("{SHA} does not hold the SHA-256 of {DUMP}; {fix}"));
        }
        _ => {}
    }
    problems
}

/// Why `--bless` must not write `fresh` over `old`, if it must not.
fn refusal(old: Option<&str>, fresh: &str, same_version: bool) -> Option<String> {
    let old = old?;
    let unbumped = old != fresh && header_version(old) == header_version(fresh);
    (unbumped && !same_version).then(|| {
        format!(
            "truth's output changed but SIM_VERSION is still {}: bump it (I-13), or pass \
             --same-version if version {} has no committed golden yet",
            version_text(fresh),
            version_text(fresh)
        )
    })
}

fn bless(root: &Path, same_version: bool) -> Result<(), String> {
    let fresh = build(root)?;
    let old = read(root, DUMP);
    if let Some(why) = refusal(old.as_deref(), &fresh, same_version) {
        return Err(why);
    }
    let path = root.join(DUMP);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, &fresh).map_err(|e| e.to_string())?;
    let sha = sha_line(&fresh);
    if read(root, SHA).as_deref() != Some(sha.as_str()) {
        std::fs::write(root.join(SHA), &sha).map_err(|e| e.to_string())?;
        println!("golden: rewrote {SHA}");
    }
    match old {
        Some(old) if old == fresh => println!("golden: {DUMP} is unchanged"),
        Some(old) => {
            println!(
                "golden: rewrote {DUMP} at SIM_VERSION {}, was {}:",
                version_text(&fresh),
                version_text(&old)
            );
            print!("{}", diff(&old, &fresh));
        }
        None => println!(
            "golden: wrote {DUMP} at SIM_VERSION {} ({} lines)",
            version_text(&fresh),
            fresh.lines().count()
        ),
    }
    Ok(())
}

/// A dump's lines as (path, value).
fn pairs(text: &str) -> Vec<(&str, &str)> {
    text.lines()
        .map(|line| line.split_once(" = ").unwrap_or((line, "")))
        .collect()
}

/// What changed between two dumps, line by line. Every line of a canonical dump has a path of
/// its own, so lines pair up by path: `-` for a value gone, `+` for a value new.
fn diff(old: &str, new: &str) -> String {
    let (old, new) = (pairs(old), pairs(new));
    let old_values: BTreeMap<&str, &str> = old.iter().copied().collect();
    let new_values: BTreeMap<&str, &str> = new.iter().copied().collect();
    let mut lines = Vec::new();
    let (mut changed, mut added, mut removed) = (0, 0, 0);
    for &(path, value) in &new {
        match old_values.get(path) {
            Some(&before) if before == value => {}
            Some(before) => {
                changed += 1;
                lines.push(format!("- {path} = {before}"));
                lines.push(format!("+ {path} = {value}"));
            }
            None => {
                added += 1;
                lines.push(format!("+ {path} = {value}"));
            }
        }
    }
    for &(path, value) in &old {
        if !new_values.contains_key(path) {
            removed += 1;
            lines.push(format!("- {path} = {value}"));
        }
    }
    let mut text = String::new();
    for line in lines.iter().take(DIFF_LINES) {
        text.push_str(line);
        text.push('\n');
    }
    if lines.len() > DIFF_LINES {
        let _ = writeln!(text, "… {} more lines", lines.len() - DIFF_LINES);
    }
    let _ = writeln!(
        text,
        "{changed} values changed, {added} added, {removed} removed"
    );
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = "sim_version = 1\nplants = [1]\nplants[0].id = 7\n";

    fn with_version(version: u32, rest: &str) -> String {
        format!("sim_version = {version}\n{rest}")
    }

    #[test]
    fn the_header_names_the_version() {
        assert_eq!(header_version(V1), Some(1));
        assert_eq!(header_version("sim_version = 12\n"), Some(12));
        assert_eq!(header_version("plants = [1]\n"), None);
        assert_eq!(header_version(""), None);
    }

    #[test]
    fn committed_files_that_hold_the_fresh_dump_pass() {
        assert!(problems(Some(V1), Some(&sha_line(V1)), V1).is_empty());
    }

    #[test]
    fn each_mismatch_says_what_it_is() {
        let unbumped = with_version(1, "plants = [0]\n");
        let bumped = with_version(2, "plants = [0]\n");
        let sha = sha_line(V1);
        let only = |problems: Vec<String>| {
            assert_eq!(problems.len(), 1, "{problems:?}");
            problems.into_iter().next().unwrap_or_default()
        };
        assert!(
            only(problems(Some(V1), Some(&sha), &unbumped)).contains("without a SIM_VERSION bump")
        );
        assert!(only(problems(Some(V1), Some(&sha), &bumped)).contains("SIM_VERSION is 2 but"));
        assert!(only(problems(None, Some(&sha), V1)).contains("arc.dump is missing"));
        assert!(only(problems(Some(V1), None, V1)).contains("arc.sha256 is missing"));
        let edited = sha.replacen('0', "1", 1).replacen('a', "b", 1);
        assert_ne!(edited, sha);
        assert!(only(problems(Some(V1), Some(&edited), V1)).contains("does not hold the SHA-256"));
    }

    #[test]
    fn bless_refuses_a_change_at_the_same_version() {
        let unbumped = with_version(1, "plants = [0]\n");
        let bumped = with_version(2, "plants = [0]\n");
        assert!(refusal(Some(V1), &unbumped, false).is_some());
        assert_eq!(refusal(Some(V1), &unbumped, true), None);
        assert_eq!(refusal(Some(V1), &bumped, false), None);
        assert_eq!(refusal(Some(V1), V1, false), None);
        assert_eq!(refusal(None, &unbumped, false), None);
    }

    #[test]
    fn the_diff_pairs_lines_by_path() {
        let old = "sim_version = 1\na = 1\nb = 2\nc = 3\n";
        let new = "sim_version = 2\na = 1\nb = 5\nd = 4\n";
        assert_eq!(
            diff(old, new),
            "- sim_version = 1\n+ sim_version = 2\n- b = 2\n+ b = 5\n+ d = 4\n- c = 3\n\
             2 values changed, 1 added, 1 removed\n"
        );
        assert_eq!(diff(old, old), "0 values changed, 0 added, 0 removed\n");
    }

    #[test]
    fn a_long_diff_is_cut_and_counted() {
        let lines = |value| -> String {
            (0..300).fold(String::new(), |mut text, n| {
                let _ = writeln!(text, "x[{n}] = {value}");
                text
            })
        };
        let (old, new) = (lines(0), lines(1));
        let text = diff(&old, &new);
        assert_eq!(text.lines().count(), DIFF_LINES + 2);
        assert!(text.contains("… 400 more lines\n"));
        assert!(text.ends_with("300 values changed, 0 added, 0 removed\n"));
    }
}
