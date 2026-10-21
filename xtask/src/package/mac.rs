//! The Mac's app bundle and its DMG.
//!
//! - **The bundle:** `Contents/Info.plist`, the binary in `MacOS/`, and the icon and the
//!   notices in `Resources/`.
//! - **An ad-hoc signature** over the whole bundle (`codesign --sign -`). Apple silicon runs
//!   nothing without at least that, and a downloaded app whose bundle is not sealed is called
//!   damaged rather than offered Open Anyway. It names no developer, so this is still the
//!   unsigned build ships; a Developer ID replaces it once there is one.
//! - **The DMG** (`dmg`): the app beside a link to `/Applications`, in a window of its own
//!   picture, compressed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use terrarium_store::identifiers::{BINARY, BUNDLE_ID};

use super::dmg::{self, background::Picture};
use super::plist::{ICON_FILE, info_plist};
use super::words::Words;
use crate::release::Target;

/// What goes in the bundle beside the binary.
pub struct Contents<'a> {
    pub words: &'a Words,
    pub version: &'a str,
    pub icns: &'a [u8],
    pub notices: &'a str,
    /// The DMG window's picture.
    pub picture: &'a Picture,
}

/// Bundles, signs and images `binary` for `target` into `out_dir`; the DMG's path.
pub fn package(
    target: Target,
    binary: &Path,
    contents: &Contents,
    out_dir: &Path,
) -> Result<PathBuf, String> {
    let work = out_dir.join("work");
    let staging = work.join(target.label());
    let _ = fs::remove_dir_all(&staging);
    let app_name = format!("{}.app", contents.words.product);
    let app = staging.join(&app_name);
    bundle(&app, binary, contents)?;
    sign(&app)?;
    symlink("/Applications", &staging.join(dmg::LINK))?;
    dmg::stage(&staging, contents.picture, contents.icns)?;
    let image = out_dir.join(format!(
        "{BINARY}-{}-{}.dmg",
        contents.version,
        target.label()
    ));
    // The volume is named with the version, so two DMGs open at once are told apart.
    let volume = format!("{} {}", contents.words.product, contents.version);
    let made = dmg::image(&staging, &volume, &app_name, &image, &work);
    let _ = fs::remove_dir_all(&staging);
    made?;
    for line in dmg::check(&image, &volume, &app_name)? {
        println!("{line}");
    }
    Ok(image)
}

fn bundle(app: &Path, binary: &Path, contents: &Contents) -> Result<(), String> {
    let macos = app.join("Contents/MacOS");
    let resources = app.join("Contents/Resources");
    for dir in [&macos, &resources] {
        fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    }
    let write = |path: PathBuf, bytes: &[u8]| {
        fs::write(&path, bytes).map_err(|err| format!("{}: {err}", path.display()))
    };
    let plist = app.join("Contents/Info.plist");
    write(
        plist.clone(),
        info_plist(contents.words, contents.version).as_bytes(),
    )?;
    write(resources.join(format!("{ICON_FILE}.icns")), contents.icns)?;
    write(resources.join("LICENSES.txt"), contents.notices.as_bytes())?;
    // `fs::copy` keeps the binary's permission to run.
    fs::copy(binary, macos.join(BINARY)).map_err(|err| format!("{}: {err}", binary.display()))?;
    run(
        Command::new("plutil").arg("-lint").arg(&plist),
        "plutil -lint",
    )?;
    Ok(())
}

/// Signs the finished bundle ad hoc, and checks the signature seals it under the bundle id.
fn sign(app: &Path) -> Result<(), String> {
    run(
        Command::new("codesign")
            .args(["--force", "--sign", "-", "--timestamp=none"])
            .arg(app),
        "codesign",
    )?;
    run(
        Command::new("codesign")
            .args(["--verify", "--strict", "--verbose=1"])
            .arg(app),
        "codesign --verify",
    )?;
    let shown = Command::new("codesign")
        .args(["--display", "--verbose=2"])
        .arg(app)
        .stdout(Stdio::null())
        .output()
        .map_err(|err| format!("codesign --display: {err}"))?;
    // codesign describes the signature on stderr.
    let shown = String::from_utf8_lossy(&shown.stderr);
    if !shown.contains(&format!("Identifier={BUNDLE_ID}")) || !shown.contains("Signature=adhoc") {
        return Err(format!("the signature is not the bundle's own:\n{shown}"));
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(to: &str, link: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(to, link).map_err(|err| format!("{}: {err}", link.display()))
}

#[cfg(not(unix))]
fn symlink(_: &str, link: &Path) -> Result<(), String> {
    Err(format!("{} is made on a Mac", link.display()))
}

/// Runs `command`; what it printed, when it succeeds.
pub(super) fn run(command: &mut Command, what: &str) -> Result<String, String> {
    let out = command.output().map_err(|err| format!("{what}: {err}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{what} failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}
