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

/// The bytes the files under `dir` hold, without following symlinks; 0 when it is missing.
pub fn size(dir: &Path) -> u64 {
    let mut total = 0_u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
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
    let bytes = size(&target);
    if !over_budget(bytes) {
        return;
    }
    let tenths = bytes.saturating_mul(10) >> 30;
    println!(
        "==> sweep: {} holds {}.{} GiB, over the {} GiB budget: cargo clean",
        target.display(),
        tenths / 10,
        tenths % 10,
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
        assert_eq!(size(dir.path()), 1_024);
        assert_eq!(size(&dir.path().join("absent")), 0);
    }

    #[test]
    fn only_a_directory_over_the_budget_is_cleaned() {
        assert_eq!(BUDGET, 30 * 1024 * 1024 * 1024);
        assert!(!over_budget(0));
        assert!(!over_budget(BUDGET));
        assert!(over_budget(BUDGET + 1));
    }
}
