//! The Windows target, built on the Mac: what its C needs. Bundled SQLite and the tree-sitter
//! grammars compile C in their build scripts, and the Mac has no Windows headers of its own, so
//! each task that builds the target says how that C is handled, in one place:
//!
//! - **a lint** (`verify`'s two `clippy (windows)` stages) never links: SQLite's C is skipped
//!   through its build script's switch, and the grammars' C, which has no such switch, is
//!   compiled as a real build compiles it, so the lint also shows that it compiles for Windows;
//! - **a real build** (`probe-zip`) compiles all of it with the Mac's clang against the `xwin`
//!   splat's headers and archives it with the toolchain's `rust-lld` acting as `lib.exe`.
//!
//! The splat is made once, by a person, since making it accepts Microsoft's license.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const TARGET: &str = "x86_64-pc-windows-msvc";

/// The `xwin` version the splat layout here and the flags in `probe_zip` are written for.
pub const XWIN_VERSION: &str = "0.10.0";
/// Where the splat lives unless `TERRARIUM_XWIN_DIR` says otherwise.
pub const XWIN_DIR_ENV: &str = "TERRARIUM_XWIN_DIR";

/// For a lint: this undocumented switch sends libsqlite3-sys's build script down its linked
/// path, which compiles nothing and uses the same bundled bindings. The Rust a lint checks is
/// unchanged.
const LINT_ENV: &[(&str, &str)] = &[("LIBSQLITE3_SYS_USE_PKG_CONFIG", "1")];

/// The splat directory: `TERRARIUM_XWIN_DIR`, else a cache under the user's home.
pub fn xwin_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(XWIN_DIR_ENV) {
        return Some(PathBuf::from(dir));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(home.join("Library/Caches/repo-terrarium/xwin"))
}

/// What a person runs once to make the splat at `xwin_dir`, and why it is theirs to run.
pub fn splat_needed(xwin_dir: &Path, what: &str) -> String {
    format!(
        "{what} in {}.\n\
         Getting them downloads Microsoft's packages and accepts Microsoft's license for them, so \
         it is a person's decision. With xwin {XWIN_VERSION} installed (cargo install --locked \
         xwin@{XWIN_VERSION}), the one-time command is:\n  \
         xwin --accept-license --cache-dir {} splat --output {}",
        xwin_dir.display(),
        xwin_dir.with_extension("cache").display(),
        xwin_dir.display()
    )
}

/// For a lint, on this machine: SQLite's switch, and the environment that compiles the
/// grammars' C against the splat's headers.
pub fn lint_env(root: &Path) -> Result<Vec<(&'static str, OsString)>, String> {
    let xwin_dir =
        xwin_dir().ok_or_else(|| format!("HOME is not set, and neither is {XWIN_DIR_ENV}"))?;
    let lld = toolchain_lld(root).ok_or("the toolchain's rust-lld was not found")?;
    lint_env_with(&xwin_dir, &lld, std::env::var_os("DYLD_LIBRARY_PATH"))
}

/// [`lint_env`] with the splat, `rust-lld` and the loader path given. The headers must be
/// there; a lint never links, so the libraries need not be.
fn lint_env_with(
    xwin_dir: &Path,
    lld: &Lld,
    existing_dyld: Option<OsString>,
) -> Result<Vec<(&'static str, OsString)>, String> {
    if !include_dirs(xwin_dir).iter().all(|dir| dir.is_dir()) {
        return Err(splat_needed(
            xwin_dir,
            "the Windows target's C needs the CRT and SDK headers",
        ));
    }
    let mut env: Vec<(&'static str, OsString)> = LINT_ENV
        .iter()
        .map(|(key, value)| (*key, OsString::from(value)))
        .collect();
    env.extend(c_build_env(xwin_dir, lld, existing_dyld)?);
    Ok(env)
}

/// The CRT's and the SDK's headers, in `xwin splat`'s default layout.
fn include_dirs(xwin_dir: &Path) -> [PathBuf; 4] {
    [
        xwin_dir.join("crt/include"),
        xwin_dir.join("sdk/include/ucrt"),
        xwin_dir.join("sdk/include/um"),
        xwin_dir.join("sdk/include/shared"),
    ]
}

/// The toolchain's `rust-lld`, and the directory of the LLVM library it loads.
pub struct Lld {
    pub program: PathBuf,
    pub library_dir: PathBuf,
}

/// Asks the workspace's own `rustc` (pinned by `rust-toolchain.toml`) where its `rust-lld` is.
pub fn toolchain_lld(root: &Path) -> Option<Lld> {
    let rustc = |args: &[&str]| -> Option<String> {
        let out = Command::new("rustc")
            .args(args)
            .current_dir(root)
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let sysroot = PathBuf::from(rustc(&["--print", "sysroot"])?.trim());
    let version = rustc(&["-vV"])?;
    let host = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))?;
    let program = sysroot
        .join("lib/rustlib")
        .join(host.trim())
        .join("bin/rust-lld");
    program.is_file().then(|| Lld {
        program,
        library_dir: sysroot.join("lib"),
    })
}

/// For a real build: the environment that compiles the target's C. cc-rs reads the `CFLAGS`,
/// `AR` and `ARFLAGS` variables suffixed with the target for that target only. It names the
/// archiver by exact path, so a path with spaces stays one program, but splits `CFLAGS` on
/// whitespace, so a header directory with whitespace is refused rather than quoted (quoting
/// would need `CC_SHELL_ESCAPED_FLAGS`, which every target's flags would then obey).
/// `rust-lld` run directly needs its LLVM library on `DYLD_LIBRARY_PATH`; it goes in front of
/// whatever `existing_dyld` held.
pub fn c_build_env(
    xwin_dir: &Path,
    lld: &Lld,
    existing_dyld: Option<OsString>,
) -> Result<Vec<(&'static str, OsString)>, String> {
    let mut c_flags = OsString::new();
    for dir in include_dirs(xwin_dir) {
        if dir.to_string_lossy().contains(char::is_whitespace) {
            return Err(format!(
                "{} holds whitespace, which cc-rs would split; move the splat (TERRARIUM_XWIN_DIR)",
                dir.display()
            ));
        }
        if !c_flags.is_empty() {
            c_flags.push(" ");
        }
        c_flags.push("-isystem ");
        c_flags.push(&dir);
    }
    let mut dyld = lld.library_dir.clone().into_os_string();
    if let Some(existing) = existing_dyld.filter(|existing| !existing.is_empty()) {
        dyld.push(":");
        dyld.push(existing);
    }
    Ok(vec![
        ("CFLAGS_x86_64_pc_windows_msvc", c_flags),
        (
            "AR_x86_64_pc_windows_msvc",
            lld.program.clone().into_os_string(),
        ),
        (
            "ARFLAGS_x86_64_pc_windows_msvc",
            OsString::from("-flavor link /lib"),
        ),
        ("DYLD_LIBRARY_PATH", dyld),
    ])
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// The libsqlite3-sys release both settings were checked against. The test below fails when
    /// the lock moves past it, so an upgrade re-checks the lint's switch and the real build
    /// together.
    const CHECKED_LIBSQLITE3_SYS: &str = "0.38.2";

    fn lld() -> Lld {
        Lld {
            program: PathBuf::from("/tool chain/bin/rust-lld"),
            library_dir: PathBuf::from("/tool chain/lib"),
        }
    }

    #[test]
    fn a_real_build_compiles_the_targets_c_against_the_splat_and_archives_with_lld() {
        let env = c_build_env(Path::new("/cache/xwin"), &lld(), Some("/opt/lib".into())).unwrap();
        let env: Vec<(&str, &str)> = env
            .iter()
            .map(|(key, value)| (*key, value.to_str().unwrap()))
            .collect();
        assert_eq!(
            env,
            [
                (
                    "CFLAGS_x86_64_pc_windows_msvc",
                    "-isystem /cache/xwin/crt/include -isystem /cache/xwin/sdk/include/ucrt \
                     -isystem /cache/xwin/sdk/include/um -isystem /cache/xwin/sdk/include/shared"
                ),
                ("AR_x86_64_pc_windows_msvc", "/tool chain/bin/rust-lld"),
                ("ARFLAGS_x86_64_pc_windows_msvc", "-flavor link /lib"),
                ("DYLD_LIBRARY_PATH", "/tool chain/lib:/opt/lib"),
            ]
        );
        let alone = c_build_env(Path::new("/cache/xwin"), &lld(), None).unwrap();
        assert_eq!(alone[3].1, "/tool chain/lib");
    }

    #[test]
    fn a_lint_compiles_the_grammars_c_against_the_splat_and_skips_sqlites() {
        let dir = tempfile::tempdir().unwrap();
        let xwin = dir.path().join("xwin");
        let err = lint_env_with(&xwin, &lld(), None).unwrap_err();
        assert!(err.contains("xwin --accept-license"), "{err}");
        assert!(err.contains(&xwin.display().to_string()), "{err}");
        for include in include_dirs(&xwin) {
            fs::create_dir_all(include).unwrap();
        }
        let env = lint_env_with(&xwin, &lld(), None).unwrap();
        let keys: Vec<&str> = env.iter().map(|(key, _)| *key).collect();
        assert_eq!(
            keys,
            [
                "LIBSQLITE3_SYS_USE_PKG_CONFIG",
                "CFLAGS_x86_64_pc_windows_msvc",
                "AR_x86_64_pc_windows_msvc",
                "ARFLAGS_x86_64_pc_windows_msvc",
                "DYLD_LIBRARY_PATH",
            ]
        );
    }

    #[test]
    fn a_splat_path_with_whitespace_is_refused() {
        let err = c_build_env(Path::new("/a dir/xwin"), &lld(), None).unwrap_err();
        assert!(err.contains("/a dir/xwin/crt/include"), "{err}");
    }

    #[test]
    fn the_sqlite_settings_were_checked_against_the_locked_libsqlite3_sys() {
        let lock: toml::Table =
            toml::from_str(include_str!("../../Cargo.lock")).expect("the lock parses");
        let versions: Vec<&str> = lock["package"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["name"].as_str() == Some("libsqlite3-sys"))
            .map(|p| p["version"].as_str().unwrap())
            .collect();
        assert_eq!(
            versions,
            [CHECKED_LIBSQLITE3_SYS],
            "libsqlite3-sys moved: re-check LINT_ENV and run `cargo xtask probe-zip`"
        );
    }
}
