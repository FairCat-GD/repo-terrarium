//! `cargo xtask bench`: the gated release-build budgets. Each bench is a `harness = false`
//! target that prints its measurement and exits non-zero over its budget. Cargo's bench
//! profile inherits the release profile, so the timings are those of a shipped build.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitCode};

/// (package, bench target, what it holds to its budget).
const BENCHES: &[(&str, &str, &str)] = &[(
    "terrarium-compose",
    "catalog",
    "loading the embedded art, under 20 ms",
)];

pub fn run(root: &Path) -> ExitCode {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut failed = Vec::new();
    for (package, bench, budget) in BENCHES {
        println!("\n==> {package} {bench}: {budget}");
        let passed = Command::new(&cargo)
            .args(["bench", "--locked", "-p", package, "--bench", bench])
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
