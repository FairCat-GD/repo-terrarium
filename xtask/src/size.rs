//! `cargo xtask size` (N-08): the release binary, built as it ships, stays under 25 MB with the
//! tier-one grammars in it. A fat-LTO build takes minutes, so it is run like `bench`, when a
//! change could move the size (a dependency, a grammar, the art), not as a `verify` stage. It
//! measures the binary this machine builds.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitCode};

use crate::{probe_zip, sweep};

/// N-08's "< 25 MB", read in decimal megabytes, the stricter reading.
const BUDGET_BYTES: u64 = 25_000_000;

pub fn run(root: &Path) -> ExitCode {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let built = Command::new(&cargo)
        .args(probe_zip::BUILD)
        .current_dir(root)
        .status();
    if !built.is_ok_and(|status| status.success()) {
        println!("size: the release build failed");
        return ExitCode::FAILURE;
    }
    let binary = sweep::target_dir(root)
        .join("release")
        .join(format!("repo-terrarium{}", std::env::consts::EXE_SUFFIX));
    let bytes = match std::fs::metadata(&binary) {
        Ok(metadata) => metadata.len(),
        Err(err) => {
            println!("size: {}: {err}", binary.display());
            return ExitCode::FAILURE;
        }
    };
    let (within, said) = verdict(bytes);
    println!("size: {} {said}", binary.display());
    if within {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Whether `bytes` is within the budget, and the line that says so.
fn verdict(bytes: u64) -> (bool, String) {
    let within = bytes < BUDGET_BYTES;
    let megabytes = bytes / 10_000;
    let said = format!(
        "is {} bytes ({}.{:02} MB), {} the 25 MB budget (N-08): {}",
        grouped(bytes),
        megabytes / 100,
        megabytes % 100,
        if within { "under" } else { "over" },
        if within { "ok" } else { "FAILED" }
    );
    (within, said)
}

/// `14590560` as `14,590,560`.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_is_under_25_million_bytes() {
        assert_eq!(
            verdict(14_590_560),
            (
                true,
                "is 14,590,560 bytes (14.59 MB), under the 25 MB budget (N-08): ok".to_owned()
            )
        );
        assert!(verdict(24_999_999).0);
        assert_eq!(
            verdict(25_000_000),
            (
                false,
                "is 25,000,000 bytes (25.00 MB), over the 25 MB budget (N-08): FAILED".to_owned()
            )
        );
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
    }
}
