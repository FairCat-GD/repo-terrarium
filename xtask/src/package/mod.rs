//! `cargo xtask package`: the installers, into `target/package/`.
//!
//! - **On a Mac:** the DMG for Apple silicon (`dmg`); with `--windows`, the Windows installer
//!   too, cross-built through xwin (`windows`), with Homebrew's makensis.
//! - **On Windows:** the Windows x64 installer.
//! - `--target <triple>` makes one alone.
//!
//! Each binary is built and checked as `release` says, its notices gathered, its icon made from
//! `packaging/icon/`, and its words read from the app's copy module. The release workflow
//! (`.github/workflows/release.yml`) runs this same command on each platform.

mod dmg;
pub mod icon;
mod mac;
mod notices;
mod plist;
mod res;
mod windows;
pub mod words;

pub use windows::installer_name;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use terrarium_store::identifiers::BINARY;

use crate::release::{self, Target};
use crate::sweep;

/// The workspace version every package carries.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where the installers go, and the work folder beside them.
pub fn out_dir(root: &Path) -> PathBuf {
    sweep::target_dir(root).join("package")
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let targets = match targets(args) {
        Ok(targets) => targets,
        Err(err) => {
            println!("package: {err}");
            return ExitCode::from(2);
        }
    };
    match package(root, &targets) {
        Ok(made) => {
            for path in made {
                println!("package: {}", path.display());
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("package: {err}");
            ExitCode::FAILURE
        }
    }
}

/// What the arguments ask for: this machine's own targets, those and Windows (`--windows`), or
/// one (`--target <triple>`).
fn targets(args: &[String]) -> Result<Vec<Target>, String> {
    let targets = match args {
        [] => Target::host_defaults(),
        [flag] if flag == "--windows" => {
            let mut targets = Target::host_defaults();
            if !targets.contains(&Target::Windows) {
                targets.push(Target::Windows);
            }
            targets
        }
        [flag, triple] if flag == "--target" => {
            vec![Target::from_triple(triple).ok_or_else(|| format!("{triple} is not packaged"))?]
        }
        _ => return Err("usage: cargo xtask package [--windows | --target <triple>]".to_owned()),
    };
    if targets.is_empty() {
        return Err("packages are made on a Mac or on Windows".to_owned());
    }
    Ok(targets)
}

fn package(root: &Path, targets: &[Target]) -> Result<Vec<PathBuf>, String> {
    let out_dir = out_dir(root);
    let work = out_dir.join("work");
    fs::create_dir_all(&work).map_err(|err| format!("{}: {err}", work.display()))?;
    let words = words::read(root)?;
    let icon = icon::load(root)?;
    let icns = icon::icns(&icon::mac_sizes(&icon));
    let windows_sizes = icon::windows_sizes(&icon);
    let mut made = Vec::new();
    for &target in targets {
        let notices = notices::notices(root, target)?;
        if target.is_mac() {
            let picture = dmg::background::load(root)?;
            let binary = build_checked(root, target, None)?;
            let contents = mac::Contents {
                words: &words,
                version: VERSION,
                icns: &icns,
                notices: &notices,
                picture: &picture,
            };
            made.push(mac::package(target, &binary, &contents, &out_dir)?);
        } else {
            let res = windows::res_file(root, &words, &windows_sizes, VERSION, &work)?;
            let exe = build_checked(root, target, Some(&res))?;
            let licences = work.join("LICENSES-windows.txt");
            let ico = work.join("app.ico");
            for (path, bytes) in [
                (&licences, notices.as_bytes().to_vec()),
                (&ico, icon::ico(&windows_sizes)),
            ] {
                fs::write(path, bytes).map_err(|err| format!("{}: {err}", path.display()))?;
            }
            let installer = out_dir.join(windows::installer_name(VERSION));
            let inputs = windows::Inputs {
                exe: &exe,
                licences: &licences,
                icon: &ico,
                out: &installer,
            };
            windows::installer(root, &words, VERSION, &inputs)?;
            made.push(installer);
        }
    }
    Ok(made)
}

/// Builds `target` and checks what it built, printing each check.
fn build_checked(root: &Path, target: Target, res: Option<&Path>) -> Result<PathBuf, String> {
    let binary = release::build(root, target, res)?;
    for line in release::check(target, &binary)? {
        println!("{line}");
    }
    println!("{}", runs(target, &binary)?);
    Ok(binary)
}

/// Runs the binary's `--version` where this machine can: a Mac its own, Windows its own. A
/// binary this machine cannot run is said so, and passes.
fn runs(target: Target, binary: &Path) -> Result<String, String> {
    if !target.native() {
        return Ok("  not run here: built for another system".to_owned());
    }
    let out = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|err| format!("{} --version: {err}", binary.display()))?;
    let said = String::from_utf8_lossy(&out.stdout);
    let wanted = format!("{BINARY} {VERSION}");
    if out.status.success() && said.trim() == wanted {
        return Ok(format!("  runs here: {wanted}"));
    }
    Err(format!(
        "{} --version said {:?}, not {wanted:?}",
        binary.display(),
        said.trim()
    ))
}

/// The Windows exe as the installer ships it, built and checked: `size` measures it and
/// `probe-zip` zips it.
pub fn windows_exe(root: &Path) -> Result<PathBuf, String> {
    let work = out_dir(root).join("work");
    let words = words::read(root)?;
    let sizes = icon::windows_sizes(&icon::load(root)?);
    let res = windows::res_file(root, &words, &sizes, VERSION, &work)?;
    build_checked(root, Target::Windows, Some(&res))
}

/// Builds and checks `target`, the Windows exe with its resources.
pub fn binary(root: &Path, target: Target) -> Result<PathBuf, String> {
    if target.is_mac() {
        build_checked(root, target, None)
    } else {
        windows_exe(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arguments_name_this_machines_targets_windows_too_or_one() {
        let args =
            |args: &[&str]| targets(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>());
        assert_eq!(
            args(&["--target", "aarch64-apple-darwin"]),
            Ok(vec![Target::Mac])
        );
        assert!(args(&["--target", "x86_64-apple-darwin"]).is_err());
        assert!(args(&["--target", "aarch64-pc-windows-msvc"]).is_err());
        assert!(args(&["--everything"]).is_err());
        if cfg!(target_os = "macos") {
            assert_eq!(args(&[]), Ok(vec![Target::Mac]));
            assert_eq!(args(&["--windows"]), Ok(vec![Target::Mac, Target::Windows]));
        }
    }
}
