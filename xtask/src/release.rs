//! The app's release build as the installers ship it, and the checks every shipped binary
//! passes. `package`, `size` and `probe-zip` all build through here, so they measure and ship
//! the same bytes.
//!
//! - **The app alone** (`-p terrarium-app`): building the workspace would unify xtask's
//!   features into the app's dependencies, and with them `terrarium-adapters/fixture`, which
//!   never ships.
//! - **A folder of its own,** `target/package-build`: its flags differ from a plain release
//!   build's, which the bench and the soak keep using.
//! - **No trace of the build machine:** a crash log's locations and the binary's strings
//!   name no folder of the machine that built it. rustc's paths are remapped, the Mac's C maps
//!   the cargo folder away, and the Windows C, whose `assert` would keep each file's whole path,
//!   is built without asserts. A scan of each finished binary for the build machine's folders
//!   shows it held.
//! - **The oldest macOS it runs on is 13**, set for rustc and the C alike.
//! - **The Windows exe** links the C runtime statically, so no Visual C++ redistributable is
//!   needed, and carries the icon, the version and the manifest in its resources.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use terrarium_store::identifiers::BINARY;

use crate::{macho, pe, size, sweep, windows};

/// The release build of the app alone (above).
pub const BUILD: [&str; 5] = ["build", "--release", "--locked", "-p", "terrarium-app"];

/// The oldest macOS the app runs on, as `MACOSX_DEPLOYMENT_TARGET` and `Info.plist` name it.
pub const MACOS_MINIMUM: &str = "13.0";
const MACOS_MINIMUM_PARTS: (u32, u32, u32) = (13, 0, 0);

/// What the cargo folder and the workspace are called in a remapped path.
const CARGO_HOME_AS: &str = "/cargo";
const ROOT_AS: &str = "/repo-terrarium";

/// What the installers are built for: both Macs and Windows x64 (Windows on ARM runs the x64
/// one under emulation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    MacArm,
    MacIntel,
    Windows,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::MacArm, Target::MacIntel, Target::Windows];

    pub fn triple(self) -> &'static str {
        match self {
            Target::MacArm => "aarch64-apple-darwin",
            Target::MacIntel => "x86_64-apple-darwin",
            Target::Windows => windows::TARGET,
        }
    }

    /// How a package's file name names it.
    pub fn label(self) -> &'static str {
        match self {
            Target::MacArm => "macos-arm64",
            Target::MacIntel => "macos-x64",
            Target::Windows => "windows-x64",
        }
    }

    pub fn is_mac(self) -> bool {
        matches!(self, Target::MacArm | Target::MacIntel)
    }

    /// The executable's file name on this target.
    pub fn binary_name(self) -> String {
        if self.is_mac() {
            BINARY.to_owned()
        } else {
            format!("{BINARY}.exe")
        }
    }

    pub fn from_triple(triple: &str) -> Option<Target> {
        Target::ALL
            .into_iter()
            .find(|target| target.triple() == triple)
    }

    /// What this machine builds unless told otherwise: a Mac both Macs, Windows itself.
    pub fn host_defaults() -> Vec<Target> {
        if cfg!(target_os = "macos") {
            vec![Target::MacArm, Target::MacIntel]
        } else if cfg!(windows) {
            vec![Target::Windows]
        } else {
            Vec::new()
        }
    }

    /// Whether this machine builds the target natively, rather than through xwin.
    pub fn native(self) -> bool {
        (self.is_mac() && cfg!(target_os = "macos")) || (!self.is_mac() && cfg!(windows))
    }
}

/// Where the release builds go.
pub fn build_dir(root: &Path) -> PathBuf {
    sweep::target_dir(root).join("package-build")
}

/// Builds the app for `target` as it ships; the binary's path. The Windows exe links `res`, its
/// resources. A `.res` is named by its contents, so a change to it is a change to the flags,
/// and the exe is linked again.
pub fn build(root: &Path, target: Target, res: Option<&Path>) -> Result<PathBuf, String> {
    let machine = Machine::here()?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let build_dir = build_dir(root);
    let mut command = Command::new(&cargo);
    command
        .args(BUILD)
        .args(["--target", target.triple()])
        .arg("--target-dir")
        .arg(&build_dir)
        .current_dir(root);
    let mut flags = remap_flags(root, &machine.cargo_home);
    let c_flags = c_flags(target, &machine.cargo_home)?;
    if target.is_mac() {
        command.env("MACOSX_DEPLOYMENT_TARGET", MACOS_MINIMUM);
        command.env(c_flags_var(target), c_flags);
    } else {
        let res = res.ok_or("the Windows build needs its resources")?;
        flags.push(OsString::from("-Ctarget-feature=+crt-static"));
        let mut link = OsString::from("-Clink-arg=");
        link.push(res);
        flags.push(link);
        if target.native() {
            command.env(c_flags_var(target), c_flags);
        } else {
            let cross = windows::cross_build(root)?;
            flags.extend(cross.rustflags);
            command.env("CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER", "rust-lld");
            for (key, mut value) in cross.env {
                if key == c_flags_var(target) {
                    value.push(" ");
                    value.push(&c_flags);
                }
                command.env(key, value);
            }
        }
    }
    // With `--target`, these reach only the target's artifacts, never the build scripts.
    command.env("CARGO_ENCODED_RUSTFLAGS", flags.join(OsStr::new("\u{1f}")));
    let built = command.status();
    if !built.is_ok_and(|status| status.success()) {
        return Err(format!("the {} build failed", target.triple()));
    }
    let binary = build_dir
        .join(target.triple())
        .join("release")
        .join(target.binary_name());
    if binary.is_file() {
        Ok(binary)
    } else {
        Err(format!("no binary at {}", binary.display()))
    }
}

/// The folders whose names must not reach a shipped binary: the build machine's home, its cargo
/// folder, and the workspace.
struct Machine {
    home: PathBuf,
    cargo_home: PathBuf,
}

impl Machine {
    fn here() -> Result<Machine, String> {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .ok_or("the build machine's home folder is not set")?;
        // As cargo names it to rustc: never canonical, which on Windows would add a `\\?\`
        // that no path rustc sees begins with.
        let cargo_home =
            std::env::var_os("CARGO_HOME").map_or_else(|| home.join(".cargo"), PathBuf::from);
        Ok(Machine { home, cargo_home })
    }
}

/// rustc's paths, remapped: a dependency's file in a panic's location reads `/cargo/…` and the
/// workspace's own `/repo-terrarium/…`, wherever they were built.
fn remap_flags(root: &Path, cargo_home: &Path) -> Vec<OsString> {
    [(cargo_home, CARGO_HOME_AS), (root, ROOT_AS)]
        .into_iter()
        .map(|(from, to)| {
            let mut flag = OsString::from("--remap-path-prefix=");
            flag.push(from);
            flag.push("=");
            flag.push(to);
            flag
        })
        .collect()
}

/// The variable cc-rs reads the target's C flags from.
fn c_flags_var(target: Target) -> &'static str {
    match target {
        Target::MacArm => "CFLAGS_aarch64_apple_darwin",
        Target::MacIntel => "CFLAGS_x86_64_apple_darwin",
        Target::Windows => "CFLAGS_x86_64_pc_windows_msvc",
    }
}

/// The target's C flags. clang maps the cargo folder away from `__FILE__`. The Windows C is
/// built without asserts, the C runtime's `assert` keeping each file's whole path, and so is
/// the C a native build compiles with cl, which has no such map. cc-rs splits these on
/// whitespace, so a cargo folder with any is refused rather than quoted.
fn c_flags(target: Target, cargo_home: &Path) -> Result<OsString, String> {
    if cargo_home.to_string_lossy().contains(char::is_whitespace) {
        return Err(format!(
            "{} holds whitespace, which cc-rs would split; set CARGO_HOME to a folder without",
            cargo_home.display()
        ));
    }
    let mut flags = OsString::new();
    if target == Target::Windows {
        flags.push("-DNDEBUG");
        if target.native() {
            return Ok(flags);
        }
        flags.push(" ");
    }
    flags.push("-ffile-prefix-map=");
    flags.push(cargo_home);
    flags.push("=");
    flags.push(CARGO_HOME_AS);
    Ok(flags)
}

/// Checks the binary `target`'s build made, with a line for each; the first failure ends it.
pub fn check(target: Target, binary: &Path) -> Result<Vec<String>, String> {
    let bytes = fs::read(binary).map_err(|err| format!("{}: {err}", binary.display()))?;
    let mut lines = Vec::new();
    let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let (within, said) = size::verdict(length);
    lines.push(format!("{} {said}", binary.display()));
    if !within {
        return Err(lines.join("\n"));
    }
    let machine = Machine::here()?;
    let found = traces(
        &bytes,
        &[&machine.home, &machine.cargo_home],
        target == Target::Windows,
    );
    if !found.is_empty() {
        return Err(format!(
            "{} names the build machine's folders: {}",
            binary.display(),
            found.join(", ")
        ));
    }
    lines.push("  no build machine's folder in it".to_owned());
    if target.is_mac() {
        let minimum = macho::minimum_macos(&bytes)
            .ok_or_else(|| format!("{} is not a Mach-O file this check reads", binary.display()))?;
        if minimum != MACOS_MINIMUM_PARTS {
            return Err(format!(
                "{} runs on macOS {}.{}.{} and later, not {MACOS_MINIMUM}",
                binary.display(),
                minimum.0,
                minimum.1,
                minimum.2
            ));
        }
        lines.push(format!("  runs on macOS {MACOS_MINIMUM} and later"));
    } else {
        lines.extend(check_exe(binary, &bytes)?);
    }
    Ok(lines)
}

/// The Windows exe's imports and resources.
///
/// - comctl32 is imported by ordinal only: the `comctl32.dll` Windows loads for an app without
///   a Common Controls 6 manifest may export its subclass functions by ordinal only, and a name
///   it lacks stops the exe before it starts.
/// - Nothing comes from the C runtime's DLLs: it is linked in.
/// - One manifest, id 1, with the console policy, a group icon with id 1, and the version.
pub fn check_exe(path: &Path, bytes: &[u8]) -> Result<Vec<String>, String> {
    let unreadable = || format!("{} is not a PE32+ file this check can read", path.display());
    let named = pe::named_imports(bytes, "comctl32.dll").ok_or_else(unreadable)?;
    if !named.is_empty() {
        return Err(format!(
            "the exe imports these from comctl32.dll by name, not by ordinal: {}",
            named.join(", ")
        ));
    }
    let runtime: Vec<String> = pe::imported_dlls(bytes)
        .ok_or_else(unreadable)?
        .into_iter()
        .filter(|dll| {
            let dll = dll.to_ascii_lowercase();
            dll.starts_with("vcruntime") || dll.starts_with("api-ms-win-crt-")
        })
        .collect();
    if !runtime.is_empty() {
        return Err(format!(
            "the exe needs the C runtime's DLLs: {}",
            runtime.join(", ")
        ));
    }
    let resources = pe::resources(bytes).ok_or_else(unreadable)?;
    let of = |kind: u16| resources.iter().filter(move |r| r.kind == kind);
    let manifests: Vec<_> = of(RT_MANIFEST).collect();
    let manifest_ok = manifests.len() == 1
        && manifests[0].id == 1
        && String::from_utf8_lossy(&manifests[0].data).contains("consoleAllocationPolicy");
    if !manifest_ok {
        return Err(format!(
            "the exe should hold one manifest, id 1, with the console policy; it holds {}",
            manifests.len()
        ));
    }
    if !of(RT_GROUP_ICON).any(|r| r.id == 1) || of(RT_VERSION).next().is_none() {
        return Err("the exe lacks its icon or its version".to_owned());
    }
    Ok(vec![
        "  comctl32 by ordinal, the C runtime linked in".to_owned(),
        "  its manifest, icon and version in its resources".to_owned(),
    ])
}

/// The resource types the checks read.
pub const RT_GROUP_ICON: u16 = 14;
pub const RT_VERSION: u16 = 16;
pub const RT_MANIFEST: u16 = 24;

/// Each of `folders` that `bytes` holds, as a path in either slash, in UTF-8 or UTF-16 (the
/// Windows C runtime's `assert` keeps its file names wide); in any case of its letters when
/// `any_case`, as Windows reads paths.
fn traces(bytes: &[u8], folders: &[&Path], any_case: bool) -> Vec<String> {
    let folded;
    let bytes = if any_case {
        folded = bytes.to_ascii_lowercase();
        folded.as_slice()
    } else {
        bytes
    };
    let mut found = Vec::new();
    for folder in folders {
        let text = folder.to_string_lossy();
        let text = if any_case {
            text.to_ascii_lowercase()
        } else {
            text.into_owned()
        };
        let mut forms = vec![text.clone()];
        for form in [text.replace('\\', "/"), text.replace('/', "\\")] {
            if !forms.contains(&form) {
                forms.push(form);
            }
        }
        for form in forms {
            let wide: Vec<u8> = form.encode_utf16().flat_map(u16::to_le_bytes).collect();
            for (encoding, needle) in [("UTF-8", form.as_bytes()), ("UTF-16", wide.as_slice())] {
                if contains(bytes, needle) {
                    found.push(format!("{form} ({encoding})"));
                }
            }
        }
    }
    found
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_build_selects_the_app_alone() {
        assert_eq!(BUILD[3..], ["-p", "terrarium-app"]);
        assert!(!BUILD.contains(&"--workspace"));
    }

    #[test]
    fn each_target_has_its_triple_label_and_binary() {
        assert_eq!(
            Target::ALL.map(Target::triple),
            [
                "aarch64-apple-darwin",
                "x86_64-apple-darwin",
                "x86_64-pc-windows-msvc"
            ]
        );
        assert_eq!(
            Target::ALL.map(Target::label),
            ["macos-arm64", "macos-x64", "windows-x64"]
        );
        assert_eq!(Target::MacIntel.binary_name(), "repo-terrarium");
        assert_eq!(Target::Windows.binary_name(), "repo-terrarium.exe");
        assert_eq!(
            Target::from_triple("x86_64-apple-darwin"),
            Some(Target::MacIntel)
        );
        assert_eq!(Target::from_triple("aarch64-pc-windows-msvc"), None);
    }

    #[test]
    fn the_paths_are_remapped_to_names_that_say_nothing_of_the_machine() {
        let flags = remap_flags(Path::new("/w s/repo"), Path::new("/home/x/.cargo"));
        assert_eq!(
            flags,
            [
                "--remap-path-prefix=/home/x/.cargo=/cargo",
                "--remap-path-prefix=/w s/repo=/repo-terrarium"
            ]
        );
    }

    #[test]
    fn the_c_maps_the_cargo_folder_away_and_the_windows_c_has_no_asserts() {
        let home = Path::new("/home/x/.cargo");
        assert_eq!(
            c_flags(Target::MacArm, home).unwrap(),
            "-ffile-prefix-map=/home/x/.cargo=/cargo"
        );
        let windows = c_flags(Target::Windows, home).unwrap();
        let windows = windows.to_str().unwrap();
        assert!(windows.starts_with("-DNDEBUG"), "{windows}");
        assert!(c_flags(Target::MacArm, Path::new("/a b/.cargo")).is_err());
    }

    #[test]
    fn a_folder_is_found_in_either_encoding_and_either_slash() {
        let folder = Path::new("/Users/someone/.cargo");
        let wide: Vec<u8> = "\\Users\\someone\\.cargo\\registry"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let mut bytes = b"head /Users/someone/.cargo/registry tail".to_vec();
        assert_eq!(traces(&bytes, &[folder], false).len(), 1);
        bytes.extend(&wide);
        assert_eq!(
            traces(&bytes, &[folder], false),
            [
                "/Users/someone/.cargo (UTF-8)",
                "\\Users\\someone\\.cargo (UTF-16)"
            ]
        );
        assert!(traces(b"/cargo/registry/src", &[folder], false).is_empty());
        // Windows reads paths in any case, and so does its scan.
        let windows = Path::new(r"C:\Users\Someone");
        let other = b"C:\\USERS\\someone\\.cargo";
        assert!(traces(other, &[windows], false).is_empty());
        assert_eq!(
            traces(other, &[windows], true),
            [r"c:\users\someone (UTF-8)"]
        );
    }

    #[test]
    fn the_exe_check_wants_its_resources_and_no_runtime_dlls() {
        let path = Path::new("app.exe");
        let manifest: &[u8] = b"<consoleAllocationPolicy>detached</consoleAllocationPolicy>";
        let good = pe::tests::image_with_resources(&[
            (3, 1, b"png"),
            (RT_GROUP_ICON, 1, b"group"),
            (RT_VERSION, 1, b"version"),
            (RT_MANIFEST, 1, manifest),
        ]);
        assert!(
            check_exe(path, &good).is_ok(),
            "{:?}",
            check_exe(path, &good)
        );
        for missing in [
            pe::tests::image_with_resources(&[(RT_GROUP_ICON, 1, b"g"), (RT_VERSION, 1, b"v")]),
            pe::tests::image_with_resources(&[(RT_MANIFEST, 1, manifest), (RT_VERSION, 1, b"v")]),
            pe::tests::image_with_resources(&[
                (RT_GROUP_ICON, 1, b"g"),
                (RT_VERSION, 1, b"v"),
                (RT_MANIFEST, 1, manifest),
                (RT_MANIFEST, 2, manifest),
            ]),
        ] {
            assert!(check_exe(path, &missing).is_err());
        }
        let runtime = pe::tests::image(&[("VCRUNTIME140.dll", &[Ok("memcpy")])]);
        assert!(
            check_exe(path, &runtime)
                .unwrap_err()
                .contains("VCRUNTIME140.dll")
        );
        let named = pe::tests::image(&[("comctl32.dll", &[Ok("SetWindowSubclass")])]);
        assert!(check_exe(path, &named).unwrap_err().contains("by name"));
    }
}
