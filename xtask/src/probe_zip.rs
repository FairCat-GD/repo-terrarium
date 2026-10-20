//! `cargo xtask probe-zip`: the Windows probe build for testers: the exe the installer carries,
//! cross-built on the Mac (`release`, `windows`), zipped with the tester's instructions and the
//! launchers that run each probe.
//!
//! - The MSVC C runtime and the Windows SDK libraries come from `xwin`, splatted once into a
//!   cache outside the repository. Microsoft's license for them is accepted by a person, when
//!   they run the one-time command this prints; this task never accepts it.
//! - The C runtime is linked statically, so testers need no Visual C++ redistributable.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::{package, sweep};

pub fn run(root: &Path) -> ExitCode {
    let exe = match package::windows_exe(root) {
        Ok(exe) => exe,
        Err(err) => {
            println!("probe-zip: {err}");
            return ExitCode::FAILURE;
        }
    };
    // The tester's instructions are app copy, so the app prints them.
    let readme = match package::words::app_prints(root, "--probe-readme") {
        Ok(readme) => readme,
        Err(err) => {
            println!("probe-zip: {err}");
            return ExitCode::FAILURE;
        }
    };

    match package_zip(root, &exe, &readme) {
        Ok(zip) => {
            println!("probe-zip: {}", zip.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("probe-zip: {err}");
            ExitCode::FAILURE
        }
    }
}

fn zip_name(version: &str) -> String {
    format!("repo-terrarium-{version}-probe-windows-x64.zip")
}

/// Double-clicked from the unzipped folder: it runs the probe there, so the reports land beside
/// it, and keeps the window open at the end. cmd wants CRLF line endings.
fn run_probe_cmd() -> String {
    [
        "@echo off",
        "cd /d \"%~dp0\"",
        "repo-terrarium.exe --probe pinning",
        "pause",
    ]
    .map(|line| format!("{line}\r\n"))
    .concat()
}

/// The soak, an hour of the strip on the thriving sample measured, with `presenter` (the
/// default when none), so a tester can measure both. It writes its reports beside it.
fn run_soak_cmd(presenter: Option<&str>) -> String {
    let soak = match presenter {
        Some(presenter) => format!("repo-terrarium.exe --probe soak --presenter {presenter}"),
        None => "repo-terrarium.exe --probe soak".to_owned(),
    };
    ["@echo off", "cd /d \"%~dp0\"", soak.as_str(), "pause"]
        .map(|line| format!("{line}\r\n"))
        .concat()
}

/// The probe's connect step: it starts the app as Explorer would, in a console of its own (which
/// the app lets go of), with its files in a `data` folder beside it, so nothing is written
/// outside the probe's folder. `start` returns at once, so this window closes by itself.
fn run_connect_cmd() -> String {
    [
        "@echo off",
        "cd /d \"%~dp0\"",
        "set \"TERRARIUM_DATA_DIR=%~dp0data\"",
        "start \"\" repo-terrarium.exe",
    ]
    .map(|line| format!("{line}\r\n"))
    .concat()
}

/// Zips the exe, the README and the launchers into `target/probe/`.
fn package_zip(root: &Path, exe: &Path, readme: &str) -> Result<PathBuf, String> {
    let out_dir = sweep::target_dir(root).join("probe");
    let staging = out_dir.join("staging");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|err| format!("{}: {err}", staging.display()))?;
    let files = [
        (staging.join("repo-terrarium.exe"), None),
        (
            staging.join("README.txt"),
            Some(readme.replace('\n', "\r\n")),
        ),
        (staging.join("run-probe.cmd"), Some(run_probe_cmd())),
        (staging.join("run-connect.cmd"), Some(run_connect_cmd())),
        (staging.join("run-soak.cmd"), Some(run_soak_cmd(None))),
        (
            staging.join("run-soak-gpu.cmd"),
            Some(run_soak_cmd(Some("wgpu"))),
        ),
    ];
    for (path, text) in &files {
        let written = match text {
            Some(text) => fs::write(path, text),
            None => fs::copy(exe, path).map(|_| ()),
        };
        written.map_err(|err| format!("{}: {err}", path.display()))?;
    }

    let zip = out_dir.join(zip_name(env!("CARGO_PKG_VERSION")));
    // `zip` adds to an existing archive, so an old one is removed first.
    let _ = fs::remove_file(&zip);
    let zipped = Command::new("zip")
        .args(["-j", "-X", "-q"])
        .arg(&zip)
        .args(files.iter().map(|(path, _)| path))
        .status();
    if !zipped.is_ok_and(|status| status.success()) {
        return Err("zip failed".to_owned());
    }
    let _ = fs::remove_dir_all(&staging);
    Ok(zip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zip_is_named_by_version() {
        assert_eq!(
            zip_name("0.3.0"),
            "repo-terrarium-0.3.0-probe-windows-x64.zip"
        );
    }

    #[test]
    fn the_launcher_has_crlf_lines_and_runs_the_probe_from_its_folder() {
        let cmd = run_probe_cmd();
        assert!(
            cmd.split_inclusive('\n').all(|line| line.ends_with("\r\n")),
            "{cmd:?}"
        );
        assert!(cmd.contains("cd /d \"%~dp0\"\r\nrepo-terrarium.exe --probe pinning\r\n"));
    }

    #[test]
    fn the_soak_launchers_measure_the_default_presenter_and_the_gpu_s() {
        for (cmd, line) in [
            (run_soak_cmd(None), "repo-terrarium.exe --probe soak\r\n"),
            (
                run_soak_cmd(Some("wgpu")),
                "repo-terrarium.exe --probe soak --presenter wgpu\r\n",
            ),
        ] {
            assert!(
                cmd.split_inclusive('\n').all(|line| line.ends_with("\r\n")),
                "{cmd:?}"
            );
            assert!(
                cmd.contains(&format!("cd /d \"%~dp0\"\r\n{line}")),
                "{cmd:?}"
            );
            assert!(cmd.ends_with("pause\r\n"));
        }
    }

    #[test]
    fn the_connect_launcher_starts_the_app_alone_with_its_data_beside_it() {
        let cmd = run_connect_cmd();
        assert!(
            cmd.split_inclusive('\n').all(|line| line.ends_with("\r\n")),
            "{cmd:?}"
        );
        assert!(cmd.contains("set \"TERRARIUM_DATA_DIR=%~dp0data\"\r\n"));
        // `start` gives the app a console of its own, as a double-click in Explorer does.
        assert!(cmd.ends_with("start \"\" repo-terrarium.exe\r\n"));
        assert!(!cmd.contains("--probe"));
    }
}
