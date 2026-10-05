//! Keeps the build directory from filling the disk. Every configuration `verify` builds (default
//! and all features, release, the Windows target) keeps its own incremental caches and stale
//! dependency artifacts, and cargo never deletes them: left alone, `target/` reached 87 GB.
//! `verify` therefore starts by measuring it, and cleans it once it is over [`BUDGET`]; the
//! stages then rebuild what they need.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The most `target/` may hold before `verify` cleans it: room for every configuration `verify`
/// and the harnesses build, with a working day's incremental caches.
pub const BUDGET: u64 = 30 << 30;

/// The build directory: `CARGO_TARGET_DIR` when set, else `target/` under the workspace root.
pub fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from)
}

/// The bytes the files under `dir` hold, without following symlinks; 0 when it is missing. The
/// walk stops once the total passes `limit`, so a directory over budget is not measured to the
/// last file.
pub fn size(dir: &Path, limit: u64) -> u64 {
    let mut total = 0_u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        if total > limit {
            break;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&path) {
                stack.extend(entries.flatten().map(|entry| entry.path()));
            }
        } else {
            total = total.saturating_add(meta.len());
        }
    }
    total
}

/// Whether a build directory of `bytes` must be cleaned.
pub const fn over_budget(bytes: u64) -> bool {
    bytes > BUDGET
}

/// Runs `cargo clean` when the build directory is over [`BUDGET`]. A failure is reported and
/// never stops `verify`: the stages still run on whatever is left.
pub fn before_verify(root: &Path, cargo: &OsString) {
    let target = target_dir(root);
    let bytes = size(&target, BUDGET);
    if !over_budget(bytes) {
        return;
    }
    println!(
        "==> sweep: {} holds over the {} GiB budget: cargo clean",
        target.display(),
        BUDGET >> 30
    );
    let cleaned = Command::new(cargo)
        .arg("clean")
        .current_dir(root)
        .status()
        .is_ok_and(|status| status.success());
    if !cleaned {
        println!("--> sweep: cargo clean failed; run it by hand");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_size_counts_every_file_below_and_a_missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("debug/incremental/a");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("one"), [0_u8; 1_000]).unwrap();
        std::fs::write(dir.path().join("debug/two"), [0_u8; 24]).unwrap();
        assert_eq!(size(dir.path(), u64::MAX), 1_024);
        assert_eq!(size(&dir.path().join("absent"), u64::MAX), 0);
        // Past the limit, the walk stops: what it counted is over it, and short of the whole.
        let many = dir.path().join("deps");
        std::fs::create_dir_all(&many).unwrap();
        for n in 0..10 {
            std::fs::write(many.join(format!("{n}")), [0_u8; 100]).unwrap();
        }
        assert_eq!(size(&many, u64::MAX), 1_000);
        let partial = size(&many, 150);
        assert!(partial > 150 && partial < 1_000, "{partial}");
    }

    #[test]
    fn only_a_directory_over_the_budget_is_cleaned() {
        assert_eq!(BUDGET, 30 * 1024 * 1024 * 1024);
        assert!(!over_budget(0));
        assert!(!over_budget(BUDGET));
        assert!(over_budget(BUDGET + 1));
    }
}
