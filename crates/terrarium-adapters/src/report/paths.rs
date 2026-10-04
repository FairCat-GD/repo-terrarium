//! Paths inside reports, made relative to the repository root.
//!
//! Tools write source paths relative to where they ran, or absolute. Absolute ones hold the
//! person's home directory, which must not reach a stored contributor or an export, so a path
//! that does not lead into the repository is left out of contributors. Paths are compared as
//! text with `/` separators. A relative path is relative to wherever the tool ran, which a
//! report does not say, so the worktree is asked which reading names a file that exists.

use std::path::{Path, PathBuf};

/// The parts of a repository-relative path, without `.`; `None` when it is absolute or climbs
/// out of the root with `..`.
pub(crate) fn relative_parts(path: &str) -> Option<Vec<&str>> {
    if path.is_empty() || is_absolute(path) {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then_some(parts)
}

/// `/`-rooted, `\\`-rooted (UNC), or a Windows drive.
fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with(['/', '\\'])
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

/// The repository root, and its spellings a report may use.
pub(crate) struct Context {
    root: PathBuf,
    roots: Vec<String>,
}

impl Context {
    pub(crate) fn new(root: &Path) -> Context {
        let mut roots = Vec::new();
        for candidate in [
            std::path::absolute(root).ok(),
            std::fs::canonicalize(root).ok(),
        ]
        .into_iter()
        .flatten()
        {
            let text = slashed(&candidate.to_string_lossy());
            let text = text.trim_end_matches('/').to_owned();
            if !text.is_empty() && !roots.contains(&text) {
                roots.push(text);
            }
        }
        Context {
            root: root.to_path_buf(),
            roots,
        }
    }

    /// `path` as a repository-relative path with `/`, or `None` when it leads outside the
    /// repository.
    pub(crate) fn relative(&self, path: &str) -> Option<String> {
        let path = slashed(path);
        if !is_absolute(&path) {
            return relative_parts(&path).map(|parts| parts.join("/"));
        }
        self.roots.iter().find_map(|root| {
            let rest = strip_root(&path, root)?;
            relative_parts(rest).map(|parts| parts.join("/"))
        })
    }

    /// `path` as the report at `report` (repository-relative) names it. A relative path is read
    /// against each directory holding the report, deepest first, and the first that names an
    /// existing file wins; otherwise it is read against the root. A tool usually runs in the
    /// project directory above its `coverage/`, which in a monorepo is not the root.
    pub(crate) fn locate(&self, path: &str, report: &str) -> Option<String> {
        let path = slashed(path);
        if is_absolute(&path) {
            return self.relative(&path);
        }
        let dirs = relative_parts(report).unwrap_or_default();
        for depth in (1..dirs.len()).rev() {
            let joined = format!("{}/{path}", dirs[..depth].join("/"));
            if let Some(parts) = relative_parts(&joined) {
                let candidate = parts.join("/");
                if self.root.join(&candidate).is_file() {
                    return Some(candidate);
                }
            }
        }
        self.relative(&path)
    }

    /// `path` read against `base` (a Cobertura `<source>`), as [`Context::relative`] reads it.
    pub(crate) fn relative_to(&self, base: &str, path: &str) -> Option<String> {
        if is_absolute(&slashed(path)) {
            return self.relative(path);
        }
        let base = slashed(base);
        let base = base.trim_end_matches('/');
        if base.is_empty() || base == "." {
            return self.relative(path);
        }
        self.relative(&format!("{base}/{path}"))
    }
}

/// `path` with `\` as `/`, and without the `\\?\` prefix Windows' canonical paths carry.
fn slashed(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    path.replace('\\', "/")
}

/// What follows `root/` in `path`. A drive-letter root compares without case, as Windows does.
fn strip_root<'p>(path: &'p str, root: &str) -> Option<&'p str> {
    let head = path.get(..root.len())?;
    let same = if is_absolute(root) && root.as_bytes().get(1) == Some(&b':') {
        head.eq_ignore_ascii_case(root)
    } else {
        head == root
    };
    let rest = path.get(root.len()..)?;
    (same && rest.starts_with('/')).then(|| &rest[1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_are_normalized_and_kept_inside() {
        assert_eq!(relative_parts("src/./a.ts"), Some(vec!["src", "a.ts"]));
        assert_eq!(relative_parts("src/x/../a.ts"), Some(vec!["src", "a.ts"]));
        assert_eq!(
            relative_parts(r"coverage\lcov.info"),
            Some(vec!["coverage", "lcov.info"])
        );
        for outside in [
            "../a.ts",
            "src/../../a.ts",
            "/etc/hosts",
            r"C:\a.ts",
            "c:/a",
            r"\\h\s",
            "",
            ".",
        ] {
            assert_eq!(relative_parts(outside), None, "{outside}");
        }
    }

    #[test]
    fn absolute_paths_inside_the_root_become_relative() {
        let context = Context {
            root: PathBuf::from("/home/fern/moss"),
            roots: vec!["/home/fern/moss".into(), "C:/work/moss".into()],
        };
        assert_eq!(
            context.relative("/home/fern/moss/src/a.ts").as_deref(),
            Some("src/a.ts")
        );
        assert_eq!(
            context.relative(r"c:\WORK\moss\src\a.ts").as_deref(),
            Some("src/a.ts")
        );
        assert_eq!(context.relative("src/a.ts").as_deref(), Some("src/a.ts"));
        for outside in [
            "/home/fern/mossy/a.ts",
            "/home/fern/moss",
            "/home/fern/moss/../b.ts",
            "/usr/lib/a.ts",
            "../a.ts",
        ] {
            assert_eq!(context.relative(outside), None, "{outside}");
        }
        assert_eq!(
            context
                .relative_to("/home/fern/moss", "src/a.ts")
                .as_deref(),
            Some("src/a.ts")
        );
        assert_eq!(
            context.relative_to(".", "src/a.ts").as_deref(),
            Some("src/a.ts")
        );
        assert_eq!(
            context.relative_to("src", "a.ts").as_deref(),
            Some("src/a.ts")
        );
        assert_eq!(context.relative_to("/elsewhere", "a.ts"), None);
    }

    #[test]
    fn a_relative_path_is_read_from_the_project_that_wrote_it() {
        let dir = tempfile::tempdir().unwrap();
        for file in ["packages/fern/src/a.ts", "src/b.ts"] {
            let full = dir.path().join(file);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, "").unwrap();
        }
        let context = Context::new(dir.path());
        let report = "packages/fern/coverage/lcov.info";
        assert_eq!(
            context.locate("src/a.ts", report).as_deref(),
            Some("packages/fern/src/a.ts")
        );
        // Found under no directory of the report: read from the root.
        assert_eq!(
            context.locate("src/b.ts", report).as_deref(),
            Some("src/b.ts")
        );
        assert_eq!(
            context.locate("src/c.ts", report).as_deref(),
            Some("src/c.ts")
        );
        assert_eq!(
            context.locate("src/b.ts", "coverage/lcov.info").as_deref(),
            Some("src/b.ts")
        );
        assert_eq!(context.locate("../../../x.ts", report), None);
    }
}
