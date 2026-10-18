//! `cargo xtask bench`: the gated release-build budgets. Each bench is a `harness = false`
//! target that prints its measurement and exits non-zero over its budget. Cargo's bench
//! profile inherits the release profile, so the timings are those of a shipped build.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitCode};

/// (package, bench target, features it needs, what it holds to its budget).
const BENCHES: &[(&str, &str, &[&str], &str)] = &[
    (
        "terrarium-compose",
        "catalog",
        &[],
        "loading the embedded art, under 20 ms",
    ),
    (
        "terrarium-compose",
        "compositor",
        &[],
        "compositing a busy frame, median under 1 ms",
    ),
    (
        "terrarium-adapters",
        "history",
        &["fixture"],
        "reading a synthetic 10,000-commit history, under 10 s",
    ),
    (
        "terrarium-adapters",
        "metrics",
        &["fixture"],
        "measuring 100,000 lines of source, under 1 s, and one changed file, under 50 ms",
    ),
    (
        "terrarium-engine",
        "replay",
        &[],
        "reading and replaying a synthetic 10,000-commit history, events included, under 30 s",
    ),
    (
        "terrarium-engine",
        "live",
        &[],
        "the same through the live pipeline: connecting under 30 s, again from the cache \
         under 250 ms and its refresh under 5 s, a commit's refresh under 1 s, a tag's under 2 s",
    ),
];

pub fn run(root: &Path) -> ExitCode {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut failed = Vec::new();
    for (package, bench, features, budget) in BENCHES {
        println!("\n==> {package} {bench}: {budget}");
        let mut args = vec!["bench", "--locked", "-p", package, "--bench", bench];
        let features = features.join(",");
        if !features.is_empty() {
            args.extend(["--features", &features]);
        }
        let passed = Command::new(&cargo)
            .args(&args)
            .current_dir(root)
            .status()
            .is_ok_and(|status| status.success());
        if !passed {
            failed.push(*bench);
        }
    }
    if failed.is_empty() {
        println!("\nbench: all {} within budget", BENCHES.len());
        ExitCode::SUCCESS
    } else {
        println!("\nbench: FAILED at {}", failed.join(", "));
        ExitCode::FAILURE
    }
}
