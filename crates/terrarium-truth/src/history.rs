//! The rules that turn git history into [`CommitFacts`](crate::CommitFacts).
//!
//! The git adapter applies them, but they decide what truth computes, so they live here and are
//! compiled under [`SIM_VERSION`](crate::SIM_VERSION): changing any of them is a truth change.
//! Paths are bytes, because a git path need not be UTF-8.

/// Directories whose files never count toward terrain, matched as any directory component of a
/// path.
pub const TERRAIN_EXCLUDED_DIRS: &[&[u8]] = &[
    b"node_modules",
    b"vendor",
    b"dist",
    b"build",
    b"target",
    b".git",
    b"third_party",
];

/// Lockfiles whose names never count, at any depth. `*.lock` and `*.min.*` are matched by
/// [`counts_toward_terrain`] itself.
pub const TERRAIN_EXCLUDED_FILES: &[&[u8]] = &[
    b"package-lock.json",
    b"pnpm-lock.yaml",
    b"yarn.lock",
    b"Cargo.lock",
];

/// A file is a rename of another when at least this share of it is unchanged.
pub const RENAME_SIMILARITY_PERCENT: u8 = 50;

/// The most deletions and additions compared pairwise for fuzzy renames in one commit, as git's
/// default `diff.renameLimit`. Beyond it only exact renames are found, as in git.
pub const RENAME_LIMIT: usize = 1000;

/// How far into a blob a NUL byte marks it as binary, as git sniffs.
pub const BINARY_SNIFF_BYTES: usize = 8000;

/// A blob larger than this is binary whatever it holds, as git's `core.bigFileThreshold`
/// default.
pub const LARGEST_TEXT_BLOB_BYTES: u64 = 512 * 1024 * 1024;

/// Commit and tag times earlier than this (1970-01-01, UTC) are read as it, so a malformed date
/// cannot pull tick 0 decades into the past.
pub const EARLIEST_COMMIT_SECS: i64 = 0;

/// Whether the file at `path` (slash-separated, relative to the repository root) counts toward
/// terrain and line statistics. Binary content is judged separately, by [`looks_binary`].
pub fn counts_toward_terrain(path: &[u8]) -> bool {
    let mut components = path.split(|&b| b == b'/');
    let Some(name) = components.next_back() else {
        return false;
    };
    if components.any(|dir| TERRAIN_EXCLUDED_DIRS.contains(&dir)) {
        return false;
    }
    let minified = name.windows(5).any(|w| w == b".min.");
    let lockfile = name.ends_with(b".lock") || TERRAIN_EXCLUDED_FILES.contains(&name);
    !(name.is_empty() || minified || lockfile)
}

/// Whether a blob of `size` bytes, starting with `prefix`, is binary: too large, or with a NUL
/// byte in its first [`BINARY_SNIFF_BYTES`]. Attributes and diff drivers are never consulted:
/// HEAD's change as HEAD moves, and the others are install-local.
pub fn looks_binary(size: u64, prefix: &[u8]) -> bool {
    size > LARGEST_TEXT_BLOB_BYTES || prefix.iter().take(BINARY_SNIFF_BYTES).any(|&b| b == 0)
}

/// The lines of a text blob: every `\n`, plus a last line without one.
pub fn count_lines(bytes: &[u8]) -> u64 {
    let newlines = bytes
        .iter()
        .fold(0_u64, |n, &b| n.saturating_add(u64::from(b == b'\n')));
    let unterminated = bytes.last().is_some_and(|&b| b != b'\n');
    newlines.saturating_add(u64::from(unterminated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excluded_directories_match_at_any_depth() {
        for path in [
            "vendor/lib.rs",
            "web/node_modules/leaf/index.js",
            "crates/moss/target/debug/out.txt",
            "dist/app.js",
            "build/notes.md",
            "third_party/fern/fern.c",
            ".git/config",
        ] {
            assert!(!counts_toward_terrain(path.as_bytes()), "{path}");
        }
        // A file named like an excluded directory still counts; only directories exclude.
        for path in [
            "src/build.rs",
            "src/vendor",
            "docs/target.md",
            "builder/x.rs",
        ] {
            assert!(counts_toward_terrain(path.as_bytes()), "{path}");
        }
    }

    #[test]
    fn lockfiles_and_minified_files_never_count() {
        for path in [
            "Cargo.lock",
            "web/yarn.lock",
            "package-lock.json",
            "apps/site/pnpm-lock.yaml",
            "Gemfile.lock",
            "static/app.min.js",
            "static/style.min.css",
        ] {
            assert!(!counts_toward_terrain(path.as_bytes()), "{path}");
        }
        for path in [
            "src/meadow.rs",
            "lock.rs",
            "admin.js",
            "README.md",
            "Cargo.toml",
        ] {
            assert!(counts_toward_terrain(path.as_bytes()), "{path}");
        }
        assert!(!counts_toward_terrain(b""));
        assert!(!counts_toward_terrain(b"src/"));
        // Paths need not be UTF-8.
        assert!(counts_toward_terrain(b"src/\xffpond.rs"));
    }

    #[test]
    fn binary_is_a_nul_in_the_sniffed_prefix_or_a_giant_blob() {
        assert!(!looks_binary(5, b"moss\n"));
        assert!(looks_binary(5, b"mo\0s\n"));
        let mut late_nul = vec![b'a'; BINARY_SNIFF_BYTES];
        late_nul.push(0);
        assert!(!looks_binary(
            u64::try_from(late_nul.len()).unwrap(),
            &late_nul
        ));
        late_nul[BINARY_SNIFF_BYTES - 1] = 0;
        assert!(looks_binary(
            u64::try_from(late_nul.len()).unwrap(),
            &late_nul
        ));
        assert!(!looks_binary(LARGEST_TEXT_BLOB_BYTES, b"text"));
        assert!(looks_binary(LARGEST_TEXT_BLOB_BYTES + 1, b"text"));
    }

    #[test]
    fn lines_count_terminators_and_a_last_unterminated_line() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"\n"), 1);
        assert_eq!(count_lines(b"fern"), 1);
        assert_eq!(count_lines(b"fern\nmoss\n"), 2);
        assert_eq!(count_lines(b"fern\nmoss"), 2);
        assert_eq!(count_lines(b"fern\r\nmoss\r\n"), 2);
    }
}
