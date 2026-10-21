//! The DMG: opened, it shows a window of its own picture, the app on the left and a link to
//! Applications on the right, to drag one onto the other. The volume is named with the version
//! and shows the app's icon.
//!
//! Finder keeps such a window in the volume's `.DS_Store`, and the picture in a hidden TIFF it
//! finds through an alias. Both are written here rather than by asking Finder to arrange the
//! window, as dmgbuild does: nothing opens, so the build is the same on this Mac, on the
//! release workflow's runner and in a harness.
//!
//! 1. The app, the link, the picture and the volume's icon are staged in a folder.
//! 2. `hdiutil` makes a writable image of it and mounts it out of Finder's sight, in a
//!    temporary folder of the system's: mounted inside a folder something watches, such as a
//!    checkout in Documents, the volume gets a `.fseventsd` log written as it is closed.
//! 3. The alias needs the mounted picture's catalog id and the volume's creation date, so the
//!    `.DS_Store` is written there, and the volume marked as having its own icon.
//! 4. The image is compressed into the DMG, which is mounted again, read-only, and checked.
//!
//! `hdiutil` stays the tool: macOS 26 and later print that `diskutil image` replaces some of its
//! verbs, which the release workflow's macOS 15 lacks.

mod alias;
pub mod background;
mod bplist;
mod ds_store;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use background::Picture;
use bplist::Value as Plist;
use ds_store::{Record, Value};

use super::mac::run;

/// The window's size in points, the picture's at 1×.
pub const WINDOW: (u32, u32) = (660, 400);
/// Where the window opens, as Finder keeps its bounds.
const WINDOW_AT: (u32, u32) = (400, 520);
/// The icons' size in points, and their centres in the window, as the picture is drawn for.
const ICON_SIZE: f64 = 80.0;
const APP_AT: (u32, u32) = (180, 200);
const LINK_AT: (u32, u32) = (480, 200);

/// The hidden files in the volume's root beside the app.
const PICTURE: &str = ".background.tiff";
const VOLUME_ICON: &str = ".VolumeIcon.icns";
const DS_STORE: &str = ".DS_Store";
/// The link to `/Applications`.
pub const LINK: &str = "Applications";
/// The volume root's Finder information with its custom-icon flag set, as `xattr` takes it.
const FINDER_INFO: &str = "0000000000000000040000000000000000000000000000000000000000000000";
/// Room the writable image keeps beyond what it is made from, for the `.DS_Store`.
const ROOM: u64 = 16 << 20;

/// Puts the picture and the volume's icon into `staging`, beside the app and the link.
pub fn stage(staging: &Path, picture: &Picture, icns: &[u8]) -> Result<(), String> {
    background::tiff(picture, &staging.join(PICTURE))?;
    let icon = staging.join(VOLUME_ICON);
    fs::write(&icon, icns).map_err(|err| format!("{}: {err}", icon.display()))
}

/// Makes `dmg` of `staging`, its volume named `volume`, its window set as above; `app` is the
/// app's folder name. `work` holds the writable image while it is made.
pub fn image(
    staging: &Path,
    volume: &str,
    app: &str,
    dmg: &Path,
    work: &Path,
) -> Result<(), String> {
    let writable = work.join("writable.dmg");
    let _ = fs::remove_file(&writable);
    let _ = fs::remove_file(dmg);
    let kilobytes = (folder_size(staging)? + ROOM) / 1024;
    hdiutil(&[
        "create".as_ref(),
        "-quiet".as_ref(),
        "-srcfolder".as_ref(),
        staging.as_os_str(),
        "-volname".as_ref(),
        volume.as_ref(),
        "-fs".as_ref(),
        "HFS+".as_ref(),
        "-format".as_ref(),
        "UDRW".as_ref(),
        "-size".as_ref(),
        format!("{kilobytes}k").as_ref(),
        writable.as_os_str(),
    ])?;
    let mounted = Mounted::attach(&writable, true)?;
    let mount = mounted.path();
    let records = window(app, &picture_facts(mount, volume)?)?;
    let store = mount.join(DS_STORE);
    fs::write(&store, ds_store::write(&records)?)
        .map_err(|err| format!("{}: {err}", store.display()))?;
    run(
        Command::new("xattr")
            .args(["-wx", "com.apple.FinderInfo", FINDER_INFO])
            .arg(mount),
        "xattr -wx com.apple.FinderInfo",
    )?;
    mounted.detach()?;
    hdiutil(&[
        "convert".as_ref(),
        "-quiet".as_ref(),
        writable.as_os_str(),
        "-format".as_ref(),
        "UDZO".as_ref(),
        "-o".as_ref(),
        dmg.as_os_str(),
    ])?;
    fs::remove_file(&writable).map_err(|err| format!("{}: {err}", writable.display()))
}

/// Mounts `dmg` read-only and checks what a person who opens it gets; a line for each check,
/// and the first failure ends it.
pub fn check(dmg: &Path, volume: &str, app: &str) -> Result<Vec<String>, String> {
    let mounted = Mounted::attach(dmg, false)?;
    let lines = check_mounted(mounted.path(), volume, app);
    mounted.detach()?;
    lines
}

fn check_mounted(mount: &Path, volume: &str, app: &str) -> Result<Vec<String>, String> {
    let mut lines = vec![format!("{volume}:")];

    let named = output(
        Command::new("diskutil").arg("info").arg(mount),
        "diskutil info",
    )?;
    let shown = named
        .lines()
        .find_map(|line| line.trim().strip_prefix("Volume Name:"))
        .map(str::trim);
    if shown != Some(volume) {
        return Err(format!("the volume is named {shown:?}, not {volume:?}"));
    }

    let mut held: Vec<String> = fs::read_dir(mount)
        .map_err(|err| format!("{}: {err}", mount.display()))?
        .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()
        .map_err(|err| format!("{}: {err}", mount.display()))?;
    held.sort();
    let mut wanted = vec![DS_STORE, PICTURE, VOLUME_ICON, LINK, app];
    wanted.sort_unstable();
    if held != wanted {
        return Err(format!("the volume holds {held:?}, not {wanted:?}"));
    }
    let link = fs::read_link(mount.join(LINK)).map_err(|err| format!("{LINK}: {err}"))?;
    if link != Path::new("/Applications") {
        return Err(format!("{LINK} links to {}", link.display()));
    }
    lines.push(format!(
        "  holds {app} and a link to /Applications, with its window's files hidden"
    ));

    run(
        Command::new("codesign")
            .args(["--verify", "--strict", "--verbose=1"])
            .arg(mount.join(app)),
        "codesign --verify",
    )?;
    lines.push("  the app's seal holds".to_owned());

    let tiff = output(
        Command::new("tiffutil")
            .arg("-info")
            .arg(mount.join(PICTURE)),
        "tiffutil -info",
    )?;
    let sizes: Vec<&str> = tiff
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Image Width: "))
        .collect();
    let (width, height) = WINDOW;
    let both = [
        format!("{width} Image Length: {height}"),
        format!("{} Image Length: {}", width * 2, height * 2),
    ];
    if sizes != both {
        return Err(format!(
            "the picture holds {sizes:?}, not one at 1× and one at 2× ({both:?})"
        ));
    }
    lines.push(format!("  its picture at {width}×{height} and at 2×"));

    let info = output(
        Command::new("xattr")
            .args(["-px", "com.apple.FinderInfo"])
            .arg(mount),
        "xattr -px com.apple.FinderInfo",
    )?;
    let info: String = info.split_whitespace().collect();
    if !info.eq_ignore_ascii_case(FINDER_INFO) {
        return Err(format!(
            "the volume's Finder information is {info}, not {FINDER_INFO} (its own icon)"
        ));
    }
    lines.push("  the app's icon on the volume".to_owned());

    let store = fs::read(mount.join(DS_STORE)).map_err(|err| format!("{DS_STORE}: {err}"))?;
    let found = crate::release::machine_traces(&store)?;
    if !found.is_empty() {
        return Err(format!(
            "the {DS_STORE} names the build machine's folders: {}",
            found.join(", ")
        ));
    }
    let read = ds_store::read(&store)?;
    if read != ds_store::sorted(window(app, &picture_facts(mount, volume)?)?) {
        return Err(format!(
            "the {DS_STORE} does not hold the window, its icons' places and its picture's alias \
             as this volume has them"
        ));
    }
    lines.push(format!(
        "  its window: {width}×{height}, {ICON_SIZE}-point icons at {APP_AT:?} and {LINK_AT:?}, \
         no toolbar, the picture found by its alias"
    ));
    Ok(lines)
}

/// The `.DS_Store`'s records for the volume's root: the window, its view and its picture, and
/// where the app and the link sit. The values are those dmgbuild writes.
fn window(app: &str, picture: &alias::Target) -> Result<Vec<Record>, String> {
    let (width, height) = WINDOW;
    let (x, y) = WINDOW_AT;
    let browser = bplist::write(&BTreeMap::from([
        ("ContainerShowSidebar", Plist::Bool(false)),
        ("PreviewPaneVisibility", Plist::Bool(false)),
        ("ShowPathbar", Plist::Bool(false)),
        ("ShowSidebar", Plist::Bool(false)),
        ("ShowStatusBar", Plist::Bool(false)),
        ("ShowTabView", Plist::Bool(false)),
        ("ShowToolbar", Plist::Bool(false)),
        ("SidebarWidth", Plist::Int(180)),
        (
            "WindowBounds",
            Plist::Str(format!("{{{{{x}, {y}}}, {{{width}, {height}}}}}")),
        ),
    ]));
    let view = bplist::write(&BTreeMap::from([
        ("arrangeBy", Plist::Str("none".to_owned())),
        ("backgroundColorBlue", Plist::Real(1.0)),
        ("backgroundColorGreen", Plist::Real(1.0)),
        ("backgroundColorRed", Plist::Real(1.0)),
        ("backgroundImageAlias", Plist::Data(alias::record(picture)?)),
        // A picture, rather than the default or a colour.
        ("backgroundType", Plist::Int(2)),
        ("gridOffsetX", Plist::Real(0.0)),
        ("gridOffsetY", Plist::Real(0.0)),
        ("gridSpacing", Plist::Real(100.0)),
        ("iconSize", Plist::Real(ICON_SIZE)),
        ("labelOnBottom", Plist::Bool(true)),
        ("scrollPositionX", Plist::Real(0.0)),
        ("scrollPositionY", Plist::Real(0.0)),
        ("showIconPreview", Plist::Bool(false)),
        ("showItemInfo", Plist::Bool(false)),
        ("textSize", Plist::Real(12.0)),
        ("viewOptionsVersion", Plist::Int(1)),
    ]));
    let record = |name: &str, code: &[u8; 4], value: Value| Record {
        name: name.to_owned(),
        code: *code,
        value,
    };
    Ok(vec![
        record(".", b"bwsp", Value::Blob(browser)),
        record(".", b"icvl", Value::Type(*b"icnv")),
        record(".", b"icvp", Value::Blob(view)),
        record(".", b"vSrn", Value::Long(1)),
        record(LINK, b"Iloc", Value::Blob(icon_at(LINK_AT))),
        record(app, b"Iloc", Value::Blob(icon_at(APP_AT))),
    ])
}

/// An icon's place: its centre, then the bytes Finder writes after it.
fn icon_at((x, y): (u32, u32)) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    out.extend(x.to_be_bytes());
    out.extend(y.to_be_bytes());
    out.extend([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00]);
    out
}

/// What the picture's alias names, read from the volume mounted at `mount`.
fn picture_facts(mount: &Path, volume: &str) -> Result<alias::Target, String> {
    let root = fs::metadata(mount).map_err(|err| format!("{}: {err}", mount.display()))?;
    let path = mount.join(PICTURE);
    let file = fs::metadata(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(alias::Target {
        volume: volume.to_owned(),
        volume_created: hfs_date(root.created())?,
        folder_id: catalog_id(&root)?,
        file: PICTURE.to_owned(),
        file_id: catalog_id(&file)?,
        file_created: hfs_date(file.created())?,
    })
}

/// A creation date in seconds since 1904, HFS+'s epoch, as the alias keeps it.
fn hfs_date(created: std::io::Result<SystemTime>) -> Result<u32, String> {
    let created = created.map_err(|err| format!("a creation date: {err}"))?;
    let unix = created
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_secs();
    u32::try_from(unix + alias::HFS_EPOCH_OFFSET).map_err(|err| err.to_string())
}

/// A file's catalog id: its inode number on HFS+.
#[cfg(unix)]
fn catalog_id(meta: &fs::Metadata) -> Result<u32, String> {
    use std::os::unix::fs::MetadataExt as _;
    u32::try_from(meta.ino()).map_err(|err| err.to_string())
}

#[cfg(not(unix))]
fn catalog_id(_: &fs::Metadata) -> Result<u32, String> {
    Err("a DMG is made on a Mac".to_owned())
}

/// The bytes the files under `folder` hold.
fn folder_size(folder: &Path) -> Result<u64, String> {
    let mut total = 0;
    for entry in fs::read_dir(folder).map_err(|err| format!("{}: {err}", folder.display()))? {
        let entry = entry.map_err(|err| format!("{}: {err}", folder.display()))?;
        let kind = entry.file_type().map_err(|err| err.to_string())?;
        if kind.is_dir() {
            total += folder_size(&entry.path())?;
        } else if kind.is_file() {
            total += entry.metadata().map_err(|err| err.to_string())?.len();
        }
    }
    Ok(total)
}

/// A volume mounted out of Finder's sight in a temporary folder, detached again however the
/// work around it ends.
struct Mounted {
    at: tempfile::TempDir,
    attached: bool,
}

impl Mounted {
    fn attach(image: &Path, writable: bool) -> Result<Mounted, String> {
        let at = tempfile::Builder::new()
            .prefix("repo-terrarium-dmg-")
            .tempdir()
            .map_err(|err| format!("a mount point: {err}"))?;
        hdiutil(&[
            "attach".as_ref(),
            "-quiet".as_ref(),
            "-nobrowse".as_ref(),
            "-noautoopen".as_ref(),
            if writable { "-readwrite" } else { "-readonly" }.as_ref(),
            "-mountpoint".as_ref(),
            at.path().as_os_str(),
            image.as_os_str(),
        ])?;
        Ok(Mounted { at, attached: true })
    }

    fn path(&self) -> &Path {
        self.at.path()
    }

    fn detach(mut self) -> Result<(), String> {
        self.attached = false;
        hdiutil(&[
            "detach".as_ref(),
            "-quiet".as_ref(),
            self.at.path().as_os_str(),
        ])
    }
}

impl Drop for Mounted {
    fn drop(&mut self) {
        if self.attached {
            let _ = Command::new("hdiutil")
                .args(["detach", "-quiet", "-force"])
                .arg(self.at.path())
                .output();
        }
    }
}

/// Runs `hdiutil` with `args`. It can find a device busy for a moment, so it is tried three
/// times.
fn hdiutil(args: &[&OsStr]) -> Result<(), String> {
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
        let out = Command::new("hdiutil")
            .args(args)
            .output()
            .map_err(|err| format!("hdiutil: {err}"))?;
        if out.status.success() {
            return Ok(());
        }
        last = String::from_utf8_lossy(&out.stderr).into_owned();
    }
    let verb = args
        .first()
        .map_or_else(String::new, |a| a.to_string_lossy().into_owned());
    Err(format!("hdiutil {verb} failed: {last}"))
}

/// What `command` prints, when it succeeds.
fn output(command: &mut Command, what: &str) -> Result<String, String> {
    let out = command.output().map_err(|err| format!("{what}: {err}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture() -> alias::Target {
        alias::Target {
            volume: "Repo Terrarium 0.31.1".to_owned(),
            volume_created: 0xE6FE_707C,
            folder_id: 2,
            file: PICTURE.to_owned(),
            file_id: 19,
            file_created: 0xE6FE_707D,
        }
    }

    #[test]
    fn an_icons_place_is_its_centre_then_the_bytes_finder_writes() {
        assert_eq!(
            icon_at(APP_AT),
            [
                0, 0, 0, 0xB4, 0, 0, 0, 0xC8, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0
            ]
        );
        assert_eq!(icon_at(LINK_AT)[..8], [0, 0, 1, 0xE0, 0, 0, 0, 0xC8]);
    }

    #[test]
    fn the_window_reads_back_as_it_was_written() {
        let records = window("Repo Terrarium.app", &picture()).unwrap();
        let read = ds_store::read(&ds_store::write(&records).unwrap()).unwrap();
        let names: Vec<(&str, &[u8; 4])> =
            read.iter().map(|r| (r.name.as_str(), &r.code)).collect();
        assert_eq!(
            names,
            [
                (".", b"bwsp"),
                (".", b"icvl"),
                (".", b"icvp"),
                (".", b"vSrn"),
                ("Applications", b"Iloc"),
                ("Repo Terrarium.app", b"Iloc"),
            ]
        );
        assert_eq!(read, ds_store::sorted(records));
    }

    #[test]
    fn the_view_holds_the_alias_to_the_picture() {
        let records = window("Repo Terrarium.app", &picture()).unwrap();
        let Value::Blob(view) = &records[2].value else {
            panic!("the view is a blob");
        };
        let alias = alias::record(&picture()).unwrap();
        assert!(view.windows(alias.len()).any(|w| w == alias.as_slice()));
        let Value::Blob(browser) = &records[0].value else {
            panic!("the browser is a blob");
        };
        let bounds = b"{{400, 520}, {660, 400}}";
        assert!(browser.windows(bounds.len()).any(|w| w == bounds));
    }
}
