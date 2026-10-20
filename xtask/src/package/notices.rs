//! The notices that ship beside the app (`LICENSES.txt`): its own licences and its art's, then
//! every crate the app is built from for the target, with the licence texts each crate carries.
//! MIT, Apache-2.0 and the fonts' licences all ask for their notice to travel with a copy
//! (`deny.toml` names the fonts'). A text many crates share is written once, under all of
//! their names.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use cargo_metadata::MetadataCommand;
use terrarium_store::identifiers::BINARY;

use crate::release::Target;

/// The app's own licence files, from the workspace root.
const OWN: [&str; 3] = ["LICENSE-MIT", "LICENSE-APACHE", "LICENSE-ART.md"];

/// One crate the app is built from.
#[derive(Debug, PartialEq, Eq)]
pub struct Crate {
    pub name: String,
    pub version: String,
    /// Its manifest's licence expression.
    pub licence: String,
    /// Its licence files: each one's name, as the crate keeps it, and its text.
    pub files: Vec<(String, String)>,
}

/// `LICENSES.txt` for `target`.
pub fn notices(root: &Path, target: Target) -> Result<String, String> {
    let own = OWN
        .iter()
        .map(|name| {
            let path = root.join(name);
            fs::read_to_string(&path)
                .map(|text| ((*name).to_owned(), text))
                .map_err(|err| format!("{}: {err}", path.display()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(assemble(
        env!("CARGO_PKG_VERSION"),
        &own,
        &crates(root, target)?,
    ))
}

/// The crates the app links for `target`, from the registry: `cargo tree` names them, for the
/// app alone, its normal dependencies only, and `cargo metadata` says where each one is.
fn crates(root: &Path, target: Target) -> Result<Vec<Crate>, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let tree = Command::new(&cargo)
        .args([
            "tree",
            "--locked",
            "-p",
            "terrarium-app",
            "-e",
            "normal",
            "--prefix",
            "none",
            "-f",
            "{p}",
            "--target",
            target.triple(),
        ])
        .current_dir(root)
        .output()
        .map_err(|err| format!("cargo tree: {err}"))?;
    if !tree.status.success() {
        return Err("cargo tree failed".to_owned());
    }
    let linked: BTreeSet<(String, String)> = String::from_utf8_lossy(&tree.stdout)
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let name = words.next()?;
            let version = words.next()?.strip_prefix('v')?;
            Some((name.to_owned(), version.to_owned()))
        })
        .collect();
    let metadata = MetadataCommand::new()
        .current_dir(root)
        .other_options(vec![
            "--locked".to_owned(),
            "--filter-platform".to_owned(),
            target.triple().to_owned(),
        ])
        .exec()
        .map_err(|err| format!("cargo metadata failed: {err}"))?;
    let mut crates = Vec::new();
    for package in metadata.packages {
        let key = (package.name.to_string(), package.version.to_string());
        // The workspace's own crates are the app, under its own licence above.
        if package.source.is_none() || !linked.contains(&key) {
            continue;
        }
        let dir = package
            .manifest_path
            .parent()
            .ok_or("a manifest without a folder")?
            .as_std_path();
        crates.push(Crate {
            name: key.0,
            version: key.1,
            licence: package.license.unwrap_or_default(),
            files: licence_files(dir)?,
        });
    }
    crates.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    Ok(crates)
}

/// A crate's licence files: those at its top whose names say so, and every text beside a font
/// in a folder of its own, which is the font's licence.
fn licence_files(dir: &Path) -> Result<Vec<(String, String)>, String> {
    let mut files = Vec::new();
    let mut folders = Vec::new();
    for entry in fs::read_dir(dir).map_err(|err| format!("{}: {err}", dir.display()))? {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            folders.push((name, path));
        } else if is_licence(&name) {
            files.push((name, path));
        }
    }
    for (folder, path) in folders {
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        let names: Vec<String> = entries
            .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
            .collect();
        let fonts = names.iter().any(|name| {
            Path::new(name).extension().is_some_and(|extension| {
                extension.eq_ignore_ascii_case("ttf") || extension.eq_ignore_ascii_case("otf")
            })
        });
        for name in names {
            if fonts && name.to_ascii_lowercase().ends_with(".txt") {
                files.push((format!("{folder}/{name}"), path.join(&name)));
            }
        }
    }
    files.sort();
    files
        .into_iter()
        .map(|(name, path)| {
            fs::read(&path)
                .map(|bytes| (name, String::from_utf8_lossy(&bytes).into_owned()))
                .map_err(|err| format!("{}: {err}", path.display()))
        })
        .collect()
}

/// Whether a crate's top-level file is a licence or a notice, by its name.
fn is_licence(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "LICENSE",
        "LICENCE",
        "COPYING",
        "NOTICE",
        "UNLICENSE",
        "COPYRIGHT",
    ]
    .iter()
    .any(|start| upper.starts_with(start))
}

/// A licence text as compared and written: Unix line ends, no trailing spaces, one final line
/// end.
fn normalised(text: &str) -> String {
    let mut out: String = text
        .replace("\r\n", "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    out = out.trim_matches('\n').to_owned();
    out.push('\n');
    out
}

/// The whole file: the app's version, its own licences, the crates one to a line, and each
/// distinct licence text once, under the crates that carry it.
pub fn assemble(version: &str, own: &[(String, String)], crates: &[Crate]) -> String {
    let rule = |title: &str| format!("\n======== {title} ========\n\n");
    let mut out = format!("{BINARY} {version}\n");
    for (name, text) in own {
        out.push_str(&rule(name));
        out.push_str(&normalised(text));
    }
    out.push_str(&rule("crates"));
    for krate in crates {
        let licence = if krate.licence.is_empty() {
            "(no licence named)"
        } else {
            &krate.licence
        };
        let _ = writeln!(out, "{} {}: {licence}", krate.name, krate.version);
    }
    let mut texts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for krate in crates {
        for (file, text) in &krate.files {
            texts
                .entry(normalised(text))
                .or_default()
                .push(format!("{} {} {file}", krate.name, krate.version));
        }
    }
    let mut texts: Vec<(Vec<String>, String)> = texts
        .into_iter()
        .map(|(text, owners)| (owners, text))
        .collect();
    texts.sort();
    for (owners, text) in texts {
        out.push_str(&rule(&owners.join(", ")));
        out.push_str(&text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn krate(name: &str, files: &[(&str, &str)]) -> Crate {
        Crate {
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            licence: "MIT".to_owned(),
            files: files
                .iter()
                .map(|(file, text)| ((*file).to_owned(), (*text).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn a_text_many_crates_share_is_written_once_under_their_names() {
        let own = [("LICENSE-MIT".to_owned(), "the app's own\r\n".to_owned())];
        let crates = [
            krate("alder", &[("LICENSE-MIT", "MIT text  \n")]),
            krate("birch", &[("LICENSE", "MIT text\r\n\r\n")]),
            krate("cedar", &[("COPYING", "another text")]),
            krate("damson", &[]),
        ];
        let out = assemble("0.31.0", &own, &crates);
        assert!(out.starts_with("repo-terrarium 0.31.0\n"));
        assert!(out.contains("\n======== LICENSE-MIT ========\n\nthe app's own\n"));
        assert!(
            out.contains(
                "alder 1.0.0: MIT\nbirch 1.0.0: MIT\ncedar 1.0.0: MIT\ndamson 1.0.0: MIT\n"
            )
        );
        assert_eq!(out.matches("MIT text").count(), 1, "{out}");
        assert!(out.contains(
            "======== alder 1.0.0 LICENSE-MIT, birch 1.0.0 LICENSE ========\n\nMIT text\n"
        ));
        assert!(out.contains("======== cedar 1.0.0 COPYING ========\n\nanother text\n"));
    }

    #[test]
    fn a_crates_licences_are_its_named_files_and_its_fonts_texts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (path, text) in [
            ("LICENSE-APACHE", "apache"),
            ("license.md", "mit"),
            ("README.md", "readme"),
            ("src/lib.rs", "code"),
            ("src/notes.txt", "notes"),
            ("fonts/Font.ttf", "glyphs"),
            ("fonts/OFL.txt", "ofl"),
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let files = licence_files(root).unwrap();
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["LICENSE-APACHE", "fonts/OFL.txt", "license.md"]);
        assert_eq!(files[1].1, "ofl");
    }
}
