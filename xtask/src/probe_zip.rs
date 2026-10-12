//! `cargo xtask probe-zip`: the Windows probe build for testers. There is no CI, so it is
//! cross-built on the Mac for the target `verify` lints and the release ships.
//!
//! - The MSVC C runtime and the Windows SDK libraries come from `xwin`, splatted once into a
//!   cache outside the repository. Microsoft's license for them is accepted by a person, when
//!   they run the one-time command this prints; this task never accepts it.
//! - Linking uses the `rust-lld` the toolchain ships, so no LLVM install is needed.
//! - The C a dependency compiles for the target (bundled SQLite) is compiled as `windows` says.
//! - The C runtime is linked statically, so testers need no Visual C++ redistributable.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use crate::sweep;
use crate::windows::{self, TARGET, XWIN_DIR_ENV, XWIN_VERSION, xwin_dir};

/// The release build of the app alone. Building the workspace would unify xtask's features
/// into the app's dependencies, and with them `terrarium-adapters/fixture`, which never ships.
pub const BUILD: [&str; 5] = ["build", "--release", "--locked", "-p", "terrarium-app"];

pub fn run(root: &Path) -> ExitCode {
    let Some(xwin_dir) = xwin_dir() else {
        println!("probe-zip: HOME is not set, and neither is {XWIN_DIR_ENV}");
        return ExitCode::FAILURE;
    };
    if !has_xwin() {
        println!("probe-zip needs xwin {XWIN_VERSION}: cargo install --locked xwin@{XWIN_VERSION}");
        return ExitCode::FAILURE;
    }
    if !lib_dirs(&xwin_dir).iter().all(|dir| dir.is_dir()) {
        println!(
            "{}",
            windows::splat_needed(
                &xwin_dir,
                "probe-zip needs the Windows CRT and SDK libraries"
            )
        );
        return ExitCode::FAILURE;
    }

    let Some(lld) = windows::toolchain_lld(root) else {
        println!("probe-zip: the toolchain's rust-lld was not found");
        return ExitCode::FAILURE;
    };
    let c_env = match windows::c_build_env(&xwin_dir, &lld, std::env::var_os("DYLD_LIBRARY_PATH")) {
        Ok(env) => env,
        Err(err) => {
            println!("probe-zip: {err}");
            return ExitCode::FAILURE;
        }
    };

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let build_dir = sweep::target_dir(root).join("probe-build");
    let built = Command::new(&cargo)
        .args(BUILD)
        .args(["--target", TARGET])
        .arg("--target-dir")
        .arg(&build_dir)
        .env("CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER", "rust-lld")
        // With `--target`, these reach only the Windows artifacts, never the build scripts.
        .env("CARGO_ENCODED_RUSTFLAGS", encoded_rustflags(&xwin_dir))
        .envs(c_env)
        .current_dir(root)
        .status();
    if !built.is_ok_and(|status| status.success()) {
        println!("probe-zip: the Windows build failed");
        return ExitCode::FAILURE;
    }
    let exe = build_dir.join(TARGET).join("release/repo-terrarium.exe");
    if let Err(err) = check_imports(&exe) {
        println!("probe-zip: {err}");
        return ExitCode::FAILURE;
    }

    // The tester's instructions are app copy, so the app prints them.
    let readme = Command::new(&cargo)
        .args(["run", "--quiet", "--locked", "-p", "terrarium-app", "--"])
        .arg("--probe-readme")
        .current_dir(root)
        .output();
    let readme = match readme {
        Ok(out) if out.status.success() && !out.stdout.is_empty() => {
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
        _ => {
            println!("probe-zip: repo-terrarium --probe-readme failed");
            return ExitCode::FAILURE;
        }
    };

    match package(root, &build_dir, &readme) {
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

/// `xwin --version` prints `xwin 0.10.0`.
fn has_xwin() -> bool {
    let Ok(out) = Command::new("xwin")
        .arg("--version")
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    let printed = String::from_utf8_lossy(&out.stdout);
    out.status.success() && printed.split_whitespace().eq(["xwin", XWIN_VERSION])
}

/// The CRT's and the SDK's import libraries, in `xwin splat`'s default layout.
fn lib_dirs(xwin_dir: &Path) -> [PathBuf; 3] {
    [
        xwin_dir.join("crt/lib/x86_64"),
        xwin_dir.join("sdk/lib/um/x86_64"),
        xwin_dir.join("sdk/lib/ucrt/x86_64"),
    ]
}

/// The Windows target's rustflags, separated by 0x1f as `CARGO_ENCODED_RUSTFLAGS` expects, so a
/// path with spaces stays one argument.
fn encoded_rustflags(xwin_dir: &Path) -> OsString {
    let mut flags: Vec<OsString> = [
        "-Clinker-flavor=lld-link",
        "-Ctarget-feature=+crt-static",
        // The static C runtime names debug databases that are not shipped; the linker's
        // warning about each one is noise.
        "-Clink-arg=/IGNORE:4099",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    for dir in lib_dirs(xwin_dir) {
        let mut flag = OsString::from("-Lnative=");
        flag.push(dir);
        flags.push(flag);
    }
    flags.join(OsStr::new("\u{1f}"))
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

/// The exe must import comctl32 by ordinal only. The `comctl32.dll` Windows loads for an app
/// without a Common Controls 6 manifest may export its subclass functions by ordinal only, and a
/// name it lacks stops the exe before it starts, which no test here would see.
fn check_imports(exe: &Path) -> Result<(), String> {
    let bytes = fs::read(exe).map_err(|err| format!("{}: {err}", exe.display()))?;
    let named = named_imports(&bytes, "comctl32.dll")
        .ok_or_else(|| format!("{} is not a PE32+ file this check can read", exe.display()))?;
    if named.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the exe imports these from comctl32.dll by name, not by ordinal: {}",
            named.join(", ")
        ))
    }
}

/// The functions a PE32+ image imports by name from `dll` (ordinal imports are left out), or
/// `None` if the image cannot be read as one.
fn named_imports(image: &[u8], dll: &str) -> Option<Vec<String>> {
    let u16_at = |at: usize| Some(u16::from_le_bytes(image.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(image.get(at..at + 4)?.try_into().ok()?));
    let u64_at = |at: usize| Some(u64::from_le_bytes(image.get(at..at + 8)?.try_into().ok()?));
    let usize_at = |at: usize| u32_at(at).and_then(|v| usize::try_from(v).ok());
    let text_at = |at: usize| {
        let rest = image.get(at..)?;
        let end = rest.iter().position(|&b| b == 0)?;
        std::str::from_utf8(&rest[..end]).ok()
    };

    let pe = usize_at(0x3c)?;
    if image.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let sections = usize::from(u16_at(pe + 6)?);
    let optional = pe + 24;
    if u16_at(optional)? != 0x20b {
        return None;
    }
    let section_table = optional + usize::from(u16_at(pe + 20)?);
    // The import directory is the second data directory.
    let imports = usize_at(optional + 112 + 8)?;
    let file_offset = |rva: usize| {
        (0..sections).find_map(|index| {
            let header = section_table + index * 40;
            let size = usize_at(header + 8)?.max(usize_at(header + 16)?);
            let start = usize_at(header + 12)?;
            let raw = usize_at(header + 20)?;
            (start..start + size)
                .contains(&rva)
                .then(|| rva - start + raw)
        })
    };

    let mut descriptor = file_offset(imports)?;
    loop {
        let name = usize_at(descriptor + 12)?;
        if name == 0 {
            return Some(Vec::new());
        }
        if text_at(file_offset(name)?)?.eq_ignore_ascii_case(dll) {
            let lookup = match usize_at(descriptor)? {
                0 => usize_at(descriptor + 16)?,
                table => table,
            };
            let mut entry = file_offset(lookup)?;
            let mut named = Vec::new();
            loop {
                let value = u64_at(entry)?;
                if value == 0 {
                    return Some(named);
                }
                // The top bit marks an import by ordinal; otherwise the low 31 bits point to
                // a two-byte hint followed by the name.
                if value >> 63 == 0 {
                    let hint = usize::try_from(value & 0x7fff_ffff).ok()?;
                    named.push(text_at(file_offset(hint)? + 2)?.to_owned());
                }
                entry += 8;
            }
        }
        descriptor += 20;
    }
}

/// Zips the exe, the README and the launcher into `target/probe/`.
fn package(root: &Path, build_dir: &Path, readme: &str) -> Result<PathBuf, String> {
    let exe = build_dir.join(TARGET).join("release/repo-terrarium.exe");
    if !exe.is_file() {
        return Err(format!("no exe at {}", exe.display()));
    }
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
    ];
    for (path, text) in &files {
        let written = match text {
            Some(text) => fs::write(path, text),
            None => fs::copy(&exe, path).map(|_| ()),
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
    #[test]
    fn the_release_build_selects_the_app_alone() {
        assert_eq!(super::BUILD[3..], ["-p", "terrarium-app"]);
        assert!(!super::BUILD.contains(&"--workspace"));
    }

    use super::*;

    /// A minimal PE32+ image: one section at RVA 0x1000 (file offset 0x200) holding an import
    /// directory for each `(dll, entries)`, where an entry is a name or an ordinal.
    fn image(dlls: &[(&str, &[Result<&str, u16>])]) -> Vec<u8> {
        const RVA: usize = 0x1000;
        const RAW: usize = 0x200;
        fn put32(buf: &mut [u8], at: usize, value: usize) {
            buf[at..at + 4].copy_from_slice(&u32::try_from(value).unwrap().to_le_bytes());
        }
        let mut data = vec![0_u8; (dlls.len() + 1) * 20];
        for (index, (dll, entries)) in dlls.iter().enumerate() {
            let name_rva = RVA + data.len();
            data.extend(dll.bytes().chain([0]));
            let table_at = data.len();
            data.resize(table_at + (entries.len() + 1) * 8, 0);
            for (slot, entry) in entries.iter().enumerate() {
                let value = match entry {
                    Ok(name) => {
                        let hint_rva = RVA + data.len();
                        data.extend([0, 0].into_iter().chain(name.bytes()).chain([0]));
                        u64::try_from(hint_rva).unwrap()
                    }
                    Err(ordinal) => (1 << 63) | u64::from(*ordinal),
                };
                let at = table_at + slot * 8;
                data[at..at + 8].copy_from_slice(&value.to_le_bytes());
            }
            put32(&mut data, index * 20, RVA + table_at);
            put32(&mut data, index * 20 + 12, name_rva);
        }
        let mut image = vec![0_u8; RAW];
        image[0x3c] = 0x40;
        image[0x40..0x44].copy_from_slice(b"PE\0\0");
        image[0x46] = 1; // one section
        image[0x54] = 0xf0; // the optional header's size
        image[0x58..0x5a].copy_from_slice(&0x20b_u16.to_le_bytes());
        put32(&mut image, 0x58 + 112 + 8, RVA);
        let section = 0x58 + 0xf0;
        put32(&mut image, section + 8, data.len());
        put32(&mut image, section + 12, RVA);
        put32(&mut image, section + 16, data.len());
        put32(&mut image, section + 20, RAW);
        image.extend(data);
        image
    }

    #[test]
    fn ordinal_imports_from_comctl32_pass_and_named_ones_are_listed() {
        let ordinals = image(&[
            ("KERNEL32.dll", &[Ok("ExitProcess")]),
            ("comctl32.dll", &[Err(410), Err(412), Err(413)]),
        ]);
        assert_eq!(named_imports(&ordinals, "comctl32.dll"), Some(Vec::new()));
        let named = image(&[("COMCTL32.dll", &[Err(410), Ok("SetWindowSubclass")])]);
        assert_eq!(
            named_imports(&named, "comctl32.dll"),
            Some(vec!["SetWindowSubclass".to_owned()])
        );
        assert_eq!(named_imports(b"MZ", "comctl32.dll"), None);
    }

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
    fn the_rustflags_link_the_crt_statically_against_the_splatted_libraries() {
        let flags = encoded_rustflags(Path::new("/a dir/xwin"));
        let flags: Vec<_> = flags.to_str().unwrap().split('\u{1f}').collect();
        assert_eq!(
            flags,
            [
                "-Clinker-flavor=lld-link",
                "-Ctarget-feature=+crt-static",
                "-Clink-arg=/IGNORE:4099",
                "-Lnative=/a dir/xwin/crt/lib/x86_64",
                "-Lnative=/a dir/xwin/sdk/lib/um/x86_64",
                "-Lnative=/a dir/xwin/sdk/lib/ucrt/x86_64",
            ]
        );
    }
}
