//! Paths inside reports, made relative to the repository root.
//!
//! Tools write source paths relative to where they ran, or absolute. Absolute ones hold the
//! person's home directory, which must not reach a stored contributor or an export, so a path
//! that does not lead into the repository is left out of contributors. Paths are compared as
//! text with `/` separators; nothing here touches the file system except finding the root's real
//! path once.

use std::path::Path;

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

/// The repository root's spellings a report may use.
pub(crate) struct Context {
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
        Context { roots }
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
}
