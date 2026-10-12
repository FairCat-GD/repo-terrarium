//! Knip's JSON report (dead code), as `knip --reporter json` prints it. Knip writes the report
//! to stdout only; it is read from the file a person redirects it into.
//!
//! Knip 6 is the minimum. Its report is `{"issues": [{"file": …, "<type>": [{"name": …}, …]},
//! …]}`, every issue type an array; Knip 5's had a root `files` key and keyed `enumMembers` and
//! `classMembers` by their parent, and is refused rather than misread. The report carries no
//! version, so when the repository has Knip installed, its version is checked too. Keys Knip adds
//! later are ignored.
//!
//! The report lists only what is unused, never how much is exported, so it gives the counts the
//! dead-code formula needs on one side only; the source metrics count the other, which is why
//! the unused exports are also kept per file.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};

use super::{Contributor, Loaded, Parsed, Reading, Why, read_bounded, text};

/// The oldest Knip major version whose report this reads.
pub const KNIP_MIN_MAJOR: u64 = 6;
/// Where an installed Knip says its version, relative to the repository root.
const INSTALLED_MANIFEST: &str = "node_modules/knip/package.json";
const MANIFEST_MAX_BYTES: u64 = 1024 * 1024;

/// Issue types that are exported symbols nothing imports.
const EXPORT_TYPES: [&str; 4] = ["exports", "types", "nsExports", "nsTypes"];
/// Issue types that are members of an exported enum or namespace.
const MEMBER_TYPES: [&str; 2] = ["enumMembers", "namespaceMembers"];
const DEPENDENCY_TYPES: [&str; 3] = [
    "dependencies",
    "devDependencies",
    "optionalPeerDependencies",
];

/// What a Knip report counts as unused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KnipCounts {
    /// Exports and exported types nothing imports.
    pub unused_exports: u64,
    /// The same, by the file that exports them, relative to the repository root: what the
    /// source metrics weigh against the exports they count in that file.
    pub unused_exports_per_file: BTreeMap<String, u64>,
    /// Members of exported enums and namespaces nothing uses.
    pub unused_members: u64,
    /// Files nothing imports, relative to the repository root.
    pub unused_files: Vec<String>,
    /// Dependencies in `package.json` nothing uses.
    pub unused_dependencies: u64,
    /// The installed Knip's version, when the repository has one.
    pub version: Option<String>,
}

/// The version of the Knip installed in the repository's `node_modules`, if it says.
pub fn installed_knip(root: &Path) -> Option<String> {
    let manifest = read_bounded(
        &root.join(INSTALLED_MANIFEST),
        INSTALLED_MANIFEST,
        MANIFEST_MAX_BYTES,
    )
    .ok()?;
    let value: Value = serde_json::from_str(&text(&manifest).ok()?).ok()?;
    value.get("version")?.as_str().map(str::to_owned)
}

/// The major version of a semver text, if it starts with one.
pub(crate) fn major(version: &str) -> Option<u64> {
    let digits: String = version
        .trim_start_matches('v')
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

pub(crate) fn parse(root: &Path, files: &[Loaded]) -> Parsed {
    let version = installed_knip(root);
    if let Some(installed) = &version
        && major(installed).is_some_and(|m| m < KNIP_MIN_MAJOR)
    {
        return Err(Why::KnipBefore6 { version });
    }
    let mut counts = KnipCounts {
        version: version.clone(),
        ..KnipCounts::default()
    };
    let mut contributors = Vec::new();
    for file in files {
        read_file(file, version.as_ref(), &mut counts, &mut contributors)?;
    }
    counts.unused_files.sort();
    counts.unused_files.dedup();
    Ok((Reading::DeadCode(counts), contributors))
}

fn read_file(
    file: &Loaded,
    version: Option<&String>,
    counts: &mut KnipCounts,
    contributors: &mut Vec<Contributor>,
) -> Result<(), Why> {
    let malformed = |line: Option<u64>| Why::Malformed {
        path: file.path.clone(),
        line,
    };
    let text = text(file)?;
    let root: Value = serde_json::from_str(&text)
        .map_err(|err| malformed(u64::try_from(err.line()).ok().filter(|&l| l > 0)))?;
    let root = root.as_object().ok_or_else(|| malformed(None))?;
    if root.contains_key("files") {
        return Err(Why::KnipBefore6 {
            version: version.cloned(),
        });
    }
    let issues = root
        .get("issues")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed(None))?;
    for entry in issues {
        let entry = entry.as_object().ok_or_else(|| malformed(None))?;
        let path = entry
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed(None))?;
        if entry.contains_key("classMembers")
            || MEMBER_TYPES
                .iter()
                .any(|key| entry.get(*key).is_some_and(Value::is_object))
        {
            return Err(Why::KnipBefore6 {
                version: version.cloned(),
            });
        }
        let exports = issue_count(entry, &EXPORT_TYPES).ok_or_else(|| malformed(None))?;
        let members = issue_count(entry, &MEMBER_TYPES).ok_or_else(|| malformed(None))?;
        let dependencies = issue_count(entry, &DEPENDENCY_TYPES).ok_or_else(|| malformed(None))?;
        let unused_files = names(entry, "files").ok_or_else(|| malformed(None))?;
        counts.unused_exports = counts.unused_exports.saturating_add(exports);
        counts.unused_members = counts.unused_members.saturating_add(members);
        counts.unused_dependencies = counts.unused_dependencies.saturating_add(dependencies);
        let weight = exports
            .saturating_add(members)
            .saturating_add(u64::try_from(unused_files.len()).unwrap_or(u64::MAX));
        counts.unused_files.extend(unused_files);
        if let Some(path) = super::paths::relative_parts(path) {
            let path = path.join("/");
            if exports > 0 {
                let held = counts
                    .unused_exports_per_file
                    .entry(path.clone())
                    .or_default();
                *held = held.saturating_add(exports);
            }
            contributors.push(Contributor {
                path,
                value: i64::try_from(weight).unwrap_or(i64::MAX),
            });
        }
    }
    Ok(())
}

/// The number of issues of `types` in one entry; `None` when one is not an array.
fn issue_count(entry: &Map<String, Value>, types: &[&str]) -> Option<u64> {
    types
        .iter()
        .try_fold(0_u64, |sum, key| match entry.get(*key) {
            None => Some(sum),
            Some(Value::Array(items)) => {
                Some(sum.saturating_add(u64::try_from(items.len()).unwrap_or(u64::MAX)))
            }
            Some(_) => None,
        })
}

/// The `name`s of one issue type that are repository-relative paths; `None` when the type is
/// not an array of named items.
fn names(entry: &Map<String, Value>, key: &str) -> Option<Vec<String>> {
    let Some(items) = entry.get(key) else {
        return Some(Vec::new());
    };
    let mut paths = Vec::new();
    for item in items.as_array()? {
        let name = item.get("name")?.as_str()?;
        if let Some(parts) = super::paths::relative_parts(name) {
            paths.push(parts.join("/"));
        }
    }
    Some(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_versions_read_from_semver_text() {
        assert_eq!(major("6.39.0"), Some(6));
        assert_eq!(major("v5.88.0"), Some(5));
        assert_eq!(major("10.0.0-beta.1"), Some(10));
        assert_eq!(major("next"), None);
    }
}
