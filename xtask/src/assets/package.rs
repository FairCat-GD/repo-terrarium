//! The delivered package, read once into memory, and check 1: every file's SHA-256 equals
//! `PACKAGE-CONTENTS.json`.
//!
//! Two kinds of path are never read. `.DS_Store` is Finder's, and is listed with a stale hash.
//! A biome's `references/` holds appearance examples, which no test or harness reads (I-14).
//! Every other file is checked both ways: a listed file must match its hash, and a file on disk
//! must be listed.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::Deserialize;
use serde::de::IgnoredAny;

use super::{Check, Failure};
use crate::sha256_hex;

pub const CONTENTS: &str = "PACKAGE-CONTENTS.json";

/// Every file of the package except the skipped ones, by its `/`-separated path relative to
/// `art-assets/`.
#[derive(Clone, Debug)]
pub struct Package {
    files: BTreeMap<String, Vec<u8>>,
}

impl Package {
    pub fn read(root: &Path) -> io::Result<Package> {
        Ok(Package {
            files: read_tree(root, skipped)?,
        })
    }

    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// Replaces or adds a file, leaving `PACKAGE-CONTENTS.json` as it is.
    #[cfg(test)]
    pub fn put(&mut self, path: &str, bytes: Vec<u8>) {
        self.files.insert(path.to_owned(), bytes);
    }

    /// Replaces a file and records its new hash, so only the check the edit aims at fails.
    #[cfg(test)]
    pub fn replace(&mut self, path: &str, bytes: Vec<u8>) {
        let mut contents: serde_json::Value =
            serde_json::from_slice(&self.files[CONTENTS]).expect("the contents parse");
        contents["files"][path] = serde_json::Value::String(sha256_hex(&bytes));
        self.put(
            CONTENTS,
            serde_json::to_vec_pretty(&contents).expect("JSON encodes"),
        );
        self.put(path, bytes);
    }

    #[cfg(test)]
    pub fn remove(&mut self, path: &str) {
        self.files.remove(path);
    }
}

/// Whether a package path is one the pipeline never reads.
pub fn skipped(path: &str) -> bool {
    let mut parts = path.split('/');
    let first = parts.next().unwrap_or_default();
    let second = parts.next();
    path.rsplit('/').next() == Some(".DS_Store")
        || (first.starts_with("biome-") && second == Some("references"))
}

/// Every file under `root`, by `/`-separated path relative to it, except those `skip` names.
/// A skipped directory is never entered.
pub fn read_tree(root: &Path, skip: fn(&str) -> bool) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    read_dir(root, "", skip, &mut files)?;
    Ok(files)
}

fn read_dir(
    dir: &Path,
    prefix: &str,
    skip: fn(&str) -> bool,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> io::Result<()> {
    let mut entries = std::fs::read_dir(dir)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name().into_string().map_err(|name| {
            let shown = name.to_string_lossy().into_owned();
            io::Error::new(io::ErrorKind::InvalidData, format!("{shown} is not UTF-8"))
        })?;
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if skip(&path) {
            continue;
        }
        if entry.file_type()?.is_dir() {
            read_dir(&entry.path(), &path, skip, files)?;
        } else {
            files.insert(path, std::fs::read(entry.path())?);
        }
    }
    Ok(())
}

/// `PACKAGE-CONTENTS.json`: the package's own index.
#[expect(
    dead_code,
    reason = "the ignored fields only name keys, for `deny_unknown_fields`"
)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ContentsJson {
    active_ids: u32,
    manifests: u32,
    biomes: Vec<String>,
    files: BTreeMap<String, String>,
    snapshot: Option<IgnoredAny>,
    packaged_on: Option<IgnoredAny>,
    runtime_references: Option<IgnoredAny>,
}

/// What the other checks need from the package index.
#[derive(Clone, Debug)]
pub struct Contents {
    /// The biome names, as `biome-<name>/` folders.
    pub biomes: Vec<String>,
    pub active_ids: u32,
    pub manifests: u32,
}

/// Check 1. `None` when the index itself cannot be read, which leaves nothing to check the
/// rest of the package against.
pub fn check(package: &Package) -> (Option<Contents>, Vec<Failure>) {
    let fail = |message: String| Failure::new(Check::Package, message);
    let Some(bytes) = package.file(CONTENTS) else {
        return (None, vec![fail(format!("{CONTENTS} is missing"))]);
    };
    let json: ContentsJson = match serde_json::from_slice(bytes) {
        Ok(json) => json,
        Err(err) => return (None, vec![fail(format!("{CONTENTS}: {err}"))]),
    };
    let mut failures = Vec::new();
    for (path, hash) in &json.files {
        if skipped(path) {
            continue;
        }
        match package.file(path) {
            None => failures.push(fail(format!("{path} is listed in {CONTENTS} but missing"))),
            Some(bytes) if sha256_hex(bytes) != *hash => {
                failures.push(fail(format!(
                    "{path} does not match its SHA-256 in {CONTENTS}"
                )));
            }
            Some(_) => {}
        }
    }
    for path in package.paths().filter(|&path| path != CONTENTS) {
        if !json.files.contains_key(path) {
            failures.push(fail(format!("{path} is not listed in {CONTENTS}")));
        }
    }
    let contents = Contents {
        biomes: json.biomes,
        active_ids: json.active_ids,
        manifests: json.manifests,
    };
    (Some(contents), failures)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finder_files_and_reference_scenes_are_never_read() {
        assert!(skipped(".DS_Store"));
        assert!(skipped("biome-temperate/plants/.DS_Store"));
        assert!(skipped("biome-temperate/references/scene-day-3x.png"));
        assert!(skipped("biome-cave/references"));
        assert!(!skipped("biome-temperate/plants/exports/references.json"));
        assert!(!skipped("references/notes.md"));
        assert!(!skipped("START-HERE.md"));
    }

    #[test]
    fn sha256_is_lowercase_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
