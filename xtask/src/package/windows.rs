//! The Windows exe's resources and its installer (`packaging/windows/installer.nsi`), which
//! NSIS's `makensis` compiles: on Windows from NSIS's own folder, on the Mac from Homebrew's
//! `makensis`. The script holds no words or names of its own; each is passed as a define.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use terrarium_store::identifiers::{
    BINARY, WINDOWS_INSTALL_DIR, WINDOWS_RUN_VALUE, WINDOWS_UNINSTALL_KEY,
};

use super::icon::Sizes;
use super::res::{self, Details};
use super::words::Words;
use crate::sha256_hex;

/// Where the installer's script is, from the workspace root.
pub const SCRIPT: &str = "packaging/windows/installer.nsi";

/// Writes the exe's resources into `work`, named by their contents (a change to them is then a
/// change to the build's flags, and the exe is linked again); the file's path.
pub fn res_file(
    root: &Path,
    words: &Words,
    sizes: &Sizes,
    version: &str,
    work: &Path,
) -> Result<PathBuf, String> {
    let details = Details {
        product: &words.product,
        publisher: &words.publisher,
        licence: &words.licence,
        binary: BINARY,
        version,
    };
    let bytes = res::res(sizes, &details, &res::manifest(root)?)?;
    fs::create_dir_all(work).map_err(|err| format!("{}: {err}", work.display()))?;
    let path = work.join(format!("{BINARY}-{}.res", &sha256_hex(&bytes)[..16]));
    fs::write(&path, bytes).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(path)
}

/// The files the installer is made from, and where it goes.
pub struct Inputs<'a> {
    pub exe: &'a Path,
    pub licences: &'a Path,
    pub icon: &'a Path,
    pub out: &'a Path,
}

/// The installer's file name for `version`.
pub fn installer_name(version: &str) -> String {
    format!("{BINARY}-{version}-windows-x64-setup.exe")
}

/// Compiles the installer script with makensis.
pub fn installer(root: &Path, words: &Words, version: &str, inputs: &Inputs) -> Result<(), String> {
    let makensis = makensis().ok_or(
        "the installer needs NSIS's makensis: on a Mac, brew install makensis; on Windows, NSIS 3",
    )?;
    let mut command = Command::new(&makensis);
    // Off Windows, makensis converts text through the C library and aborts without a UTF-8
    // character type.
    if !cfg!(windows) {
        command.env("LC_CTYPE", "UTF-8");
    }
    let out = command
        .args(["-V2", "-WX"])
        .args(defines(words, version, inputs))
        .arg(root.join(SCRIPT))
        .output()
        .map_err(|err| format!("{}: {err}", makensis.display()))?;
    if out.status.success() && inputs.out.is_file() {
        Ok(())
    } else {
        Err(format!(
            "makensis failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

/// Each value the script reads, as makensis's `-D` takes it.
fn defines(words: &Words, version: &str, inputs: &Inputs) -> Vec<String> {
    let path = |path: &Path| path.display().to_string();
    [
        ("PRODUCT", words.product.clone()),
        ("PUBLISHER", words.publisher.clone()),
        ("LICENCE", words.licence.clone()),
        ("STILL_RUNNING", words.still_running.clone()),
        ("VERSION", version.to_owned()),
        ("BINARY", BINARY.to_owned()),
        ("INSTALL_DIR", WINDOWS_INSTALL_DIR.to_owned()),
        ("UNINSTALL_KEY", WINDOWS_UNINSTALL_KEY.to_owned()),
        ("RUN_VALUE", WINDOWS_RUN_VALUE.to_owned()),
        ("EXE", path(inputs.exe)),
        ("LICENSES", path(inputs.licences)),
        ("ICON", path(inputs.icon)),
        ("OUTFILE", path(inputs.out)),
    ]
    .into_iter()
    .map(|(name, value)| format!("-D{name}={value}"))
    .collect()
}

/// NSIS's compiler: in NSIS's own folder on Windows, else on `PATH` or in Homebrew's folder.
fn makensis() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = ["ProgramFiles(x86)", "ProgramFiles"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|dir| PathBuf::from(dir).join("NSIS").join("makensis.exe"))
        .collect();
    if let Some(path) = std::env::var_os("PATH") {
        let name = if cfg!(windows) {
            "makensis.exe"
        } else {
            "makensis"
        };
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(name)));
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/makensis"));
    candidates.push(PathBuf::from("/usr/local/bin/makensis"));
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::words::tests::words;

    #[test]
    fn the_script_gets_the_copy_modules_words_and_the_pinned_names() {
        let inputs = Inputs {
            exe: Path::new("/w/repo-terrarium.exe"),
            licences: Path::new("/w/LICENSES.txt"),
            icon: Path::new("/w/app.ico"),
            out: Path::new("/o/setup.exe"),
        };
        let defines = defines(&words(), "0.31.0", &inputs);
        assert_eq!(
            defines,
            [
                "-DPRODUCT=Moss Box",
                "-DPUBLISHER=Example Garden",
                "-DLICENCE=Code MIT; art CC0",
                "-DSTILL_RUNNING=Moss Box is still running.",
                "-DVERSION=0.31.0",
                "-DBINARY=repo-terrarium",
                "-DINSTALL_DIR=repo-terrarium",
                "-DUNINSTALL_KEY=io.github.faircat-gd.repo-terrarium",
                "-DRUN_VALUE=io.github.faircat-gd.repo-terrarium",
                "-DEXE=/w/repo-terrarium.exe",
                "-DLICENSES=/w/LICENSES.txt",
                "-DICON=/w/app.ico",
                "-DOUTFILE=/o/setup.exe",
            ]
        );
        assert_eq!(
            installer_name("0.31.0"),
            "repo-terrarium-0.31.0-windows-x64-setup.exe"
        );
    }

    #[test]
    fn the_script_is_plain_ascii_as_makensis_reads_it_everywhere() {
        let script = include_str!("../../../packaging/windows/installer.nsi");
        assert!(script.is_ascii());
    }

    #[test]
    fn the_script_reads_every_value_it_is_given() {
        let script = include_str!("../../../packaging/windows/installer.nsi");
        let inputs = Inputs {
            exe: Path::new("e"),
            licences: Path::new("l"),
            icon: Path::new("i"),
            out: Path::new("o"),
        };
        for define in defines(&words(), "0.31.0", &inputs) {
            let name = define
                .trim_start_matches("-D")
                .split('=')
                .next()
                .unwrap()
                .to_owned();
            assert!(script.contains(&format!("${{{name}}}")), "{name}");
        }
    }
}
