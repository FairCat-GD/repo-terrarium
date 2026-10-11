//! `cargo xtask hero`: the fixture arc's whole history as the time-lapse the app writes, by day
//! and by night at 2×, for the launch GIF, written to `target/hero/`.
//!
//! The arc is built in a temporary directory as `golden` builds it, and the app writes each GIF
//! with `repo-terrarium replay --gif`, since the date on its label is the app's copy:
//!
//! - `hero-day.gif` and `hero-night.gif`: 450 frames, 30 seconds, repeating;
//! - `SHA256SUMS`: both files' hashes. The same build always writes the same bytes.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode};

use terrarium_adapters::fixture;

use crate::gallery::{self, Files};
use crate::sweep;

/// The scale the hero is written at.
const SCALE: &str = "2";

pub fn run(root: &Path) -> ExitCode {
    let dir = sweep::target_dir(root).join("hero");
    match render(root).and_then(|files| gallery::write(&dir, &files).map(|()| files)) {
        Ok(files) => {
            for (name, bytes) in &files {
                println!("hero: {name}, {} bytes", bytes.len());
            }
            println!("hero: wrote {} files to {}", files.len(), dir.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("hero: {err}");
            ExitCode::FAILURE
        }
    }
}

fn render(root: &Path) -> Result<Files, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let repo = dir.path().join("repo");
    fixture::arc(&repo).map_err(|e| e.to_string())?;
    let observations = root.join(crate::fixture::COMMITTED);
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut files = Files::new();
    for (name, night) in [("hero-day.gif", false), ("hero-night.gif", true)] {
        let out = dir.path().join(name);
        let mut command = Command::new(&cargo);
        command
            .args([
                "run",
                "--quiet",
                "--locked",
                "-p",
                "terrarium-app",
                "--",
                "replay",
            ])
            .arg(&repo)
            .arg("--observations")
            .arg(&observations)
            .args(["--at", &fixture::ARC_REPLAY_END_SECS.to_string()])
            .arg("--gif")
            .arg(&out)
            .args(["--scale", SCALE])
            .current_dir(root);
        if night {
            command.arg("--night");
        }
        let status = command.status().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("repo-terrarium replay --gif failed for {name}"));
        }
        let bytes = std::fs::read(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        files.push((name.to_owned(), bytes));
    }
    let mut sums = String::new();
    for (name, bytes) in &files {
        let _ = writeln!(sums, "{}  {name}", crate::sha256_hex(bytes));
    }
    files.push(("SHA256SUMS".to_owned(), sums.into_bytes()));
    Ok(files)
}
