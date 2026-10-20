//! The Windows exe's resources as a `.res` file, which the linker (`link.exe` on Windows,
//! `rust-lld` on the Mac) takes beside the objects, so no resource compiler is needed:
//!
//! - **the icon:** Explorer's, the Start menu's and the installed apps' list's;
//! - **the version details:** Windows names the process in Task Manager by its description;
//! - **the manifest** (`packaging/windows/repo-terrarium.manifest`): the console policy, so
//!   Windows 11 24H2 and later opens no console for a launch from Explorer, the Start menu or
//!   the login item, while a terminal's console is still used.
//!
//! The layout is the one `rc.exe` writes: an empty first entry, then each resource, each
//! padded to four bytes.

use std::path::Path;

use super::icon::{Sizes, directory_entry};
use crate::release::{RT_GROUP_ICON, RT_MANIFEST, RT_VERSION};

const RT_ICON: u16 = 3;
/// English (United States), and the Unicode code page: the version table's key and its
/// translation.
const LANGUAGE: u16 = 0x0409;
const CODE_PAGE: u16 = 0x04b0;
/// The memory flags `rc.exe` gives each kind: moveable, pure, and for an icon's images
/// discardable.
const ICON_FLAGS: u16 = 0x1010;
const GROUP_FLAGS: u16 = 0x1030;
const DATA_FLAGS: u16 = 0x0030;

/// Where the manifest is, from the workspace root.
pub const MANIFEST: &str = "packaging/windows/repo-terrarium.manifest";

/// What the version details say.
pub struct Details<'a> {
    pub product: &'a str,
    pub publisher: &'a str,
    /// The copyright line: the licences.
    pub licence: &'a str,
    /// The executable's name, without `.exe`.
    pub binary: &'a str,
    /// `major.minor.patch`.
    pub version: &'a str,
}

/// The manifest, read from the workspace.
pub fn manifest(root: &Path) -> Result<Vec<u8>, String> {
    let path = root.join(MANIFEST);
    std::fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))
}

/// The `.res` holding the icon's `sizes`, the version `details` and the `manifest`.
pub fn res(sizes: &Sizes, details: &Details, manifest: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = entry(0, 0, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend(0_u16.to_le_bytes());
    group.extend(1_u16.to_le_bytes());
    group.extend(word(sizes.len())?.to_le_bytes());
    for (index, (side, png)) in sizes.iter().enumerate() {
        let id = word(index + 1)?;
        out.extend(entry(RT_ICON, id, ICON_FLAGS, LANGUAGE, png));
        group.extend(directory_entry(*side, png.len()));
        group.extend(id.to_le_bytes());
    }
    out.extend(entry(RT_GROUP_ICON, 1, GROUP_FLAGS, LANGUAGE, &group));
    out.extend(entry(
        RT_VERSION,
        1,
        DATA_FLAGS,
        LANGUAGE,
        &version_info(details)?,
    ));
    out.extend(entry(RT_MANIFEST, 1, DATA_FLAGS, LANGUAGE, manifest));
    Ok(out)
}

fn word(n: usize) -> Result<u16, String> {
    u16::try_from(n).map_err(|_| format!("{n} does not fit a resource's 16 bits"))
}

/// One resource: its 32-byte header (the data's length, the header's, the type and the name as
/// numbers, the flags and the language), its data, and the padding to four bytes.
fn entry(kind: u16, id: u16, flags: u16, language: u16, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + data.len() + 3);
    out.extend(u32::try_from(data.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend(32_u32.to_le_bytes());
    out.extend(0xffff_u16.to_le_bytes());
    out.extend(kind.to_le_bytes());
    out.extend(0xffff_u16.to_le_bytes());
    out.extend(id.to_le_bytes());
    out.extend(0_u32.to_le_bytes());
    out.extend(flags.to_le_bytes());
    out.extend(language.to_le_bytes());
    out.extend(0_u32.to_le_bytes());
    out.extend(0_u32.to_le_bytes());
    out.extend(data);
    pad(&mut out);
    out
}

fn pad(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// `text` in UTF-16, with its terminating zero.
fn wide(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// One node of the version details: its length, its value's length, whether it is text, its
/// key, then its value and its children, each starting on four bytes. Its length leaves out
/// the padding after it.
fn node(
    key: &str,
    text: bool,
    value_length: usize,
    value: &[u8],
    children: &[Vec<u8>],
) -> Result<Vec<u8>, String> {
    let mut out = vec![0, 0];
    out.extend(word(value_length)?.to_le_bytes());
    out.extend(u16::from(text).to_le_bytes());
    out.extend(wide(key));
    pad(&mut out);
    out.extend(value);
    for child in children {
        pad(&mut out);
        out.extend(child);
    }
    let length = word(out.len())?.to_le_bytes();
    out[..2].copy_from_slice(&length);
    Ok(out)
}

/// The `VS_VERSIONINFO` the details make: the fixed numbers, the English strings, and the
/// translation that names their table.
fn version_info(details: &Details) -> Result<Vec<u8>, String> {
    let numbers: Vec<u16> = details
        .version
        .split('.')
        .map(str::parse::<u16>)
        .collect::<Result<_, _>>()
        .map_err(|_| format!("{} is not a version of numbers", details.version))?;
    let [major, minor, patch] = numbers[..] else {
        return Err(format!("{} is not major.minor.patch", details.version));
    };
    let high = u32::from(major) << 16 | u32::from(minor);
    let low = u32::from(patch) << 16;
    let mut fixed = Vec::with_capacity(52);
    for value in [
        0xfeef_04bd, // the signature
        0x0001_0000, // the structure's version
        high,
        low,
        high,
        low,
        0x3f,        // the flags that mean something
        0,           // none set
        0x0004_0004, // NT, Win32
        1,           // an application
        0,
        0,
        0,
    ] {
        fixed.extend(u32::to_le_bytes(value));
    }
    let original = format!("{}.exe", details.binary);
    let strings = [
        ("CompanyName", details.publisher),
        ("FileDescription", details.product),
        ("FileVersion", details.version),
        ("InternalName", details.binary),
        ("LegalCopyright", details.licence),
        ("OriginalFilename", original.as_str()),
        ("ProductName", details.product),
        ("ProductVersion", details.version),
    ]
    .iter()
    .map(|(key, value)| {
        node(
            key,
            true,
            value.encode_utf16().count() + 1,
            &wide(value),
            &[],
        )
    })
    .collect::<Result<Vec<_>, _>>()?;
    let table = node(
        &format!("{LANGUAGE:04X}{CODE_PAGE:04X}"),
        true,
        0,
        &[],
        &strings,
    )?;
    let string_info = node("StringFileInfo", true, 0, &[], &[table])?;
    let translation = [LANGUAGE.to_le_bytes(), CODE_PAGE.to_le_bytes()].concat();
    let var = node("Translation", false, 4, &translation, &[])?;
    let var_info = node("VarFileInfo", true, 0, &[], &[var])?;
    node(
        "VS_VERSION_INFO",
        false,
        fixed.len(),
        &fixed,
        &[string_info, var_info],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each entry of a `.res`: its type, its name, its language and its data.
    fn entries(res: &[u8]) -> Vec<(u16, u16, u16, Vec<u8>)> {
        let u16_at = |at: usize| u16::from_le_bytes(res[at..at + 2].try_into().unwrap());
        let u32_at =
            |at: usize| usize::try_from(u32::from_le_bytes(res[at..at + 4].try_into().unwrap()));
        let mut at = 0;
        let mut out = Vec::new();
        while at < res.len() {
            let size = u32_at(at).unwrap();
            assert_eq!(u32_at(at + 4).unwrap(), 32);
            assert_eq!((u16_at(at + 8), u16_at(at + 12)), (0xffff, 0xffff));
            out.push((
                u16_at(at + 10),
                u16_at(at + 14),
                u16_at(at + 22),
                res[at + 32..at + 32 + size].to_vec(),
            ));
            at += (32 + size).next_multiple_of(4);
        }
        assert_eq!(at, res.len());
        out
    }

    fn details() -> Details<'static> {
        Details {
            product: "Moss Box",
            publisher: "Example Garden",
            licence: "Code MIT",
            binary: "moss-box",
            version: "0.31.2",
        }
    }

    #[test]
    fn the_res_holds_the_icons_their_group_the_version_and_the_manifest() {
        let mut sizes = Sizes::new();
        sizes.insert(16, b"\x89PNG sixteen".to_vec());
        sizes.insert(256, b"\x89PNG two hundred and fifty-six".to_vec());
        let res = res(&sizes, &details(), b"<assembly/>").unwrap();
        let found = entries(&res);
        // rc.exe's empty first entry.
        assert_eq!(found[0], (0, 0, 0, Vec::new()));
        assert_eq!(&res[..32], &{
            let mut empty = [0_u8; 32];
            empty[4] = 32;
            empty[8..10].copy_from_slice(&[0xff, 0xff]);
            empty[12..14].copy_from_slice(&[0xff, 0xff]);
            empty
        });
        let kinds: Vec<(u16, u16)> = found[1..].iter().map(|e| (e.0, e.1)).collect();
        assert_eq!(kinds, [(3, 1), (3, 2), (14, 1), (16, 1), (24, 1)]);
        assert!(found[1..].iter().all(|e| e.2 == 0x0409));
        assert_eq!(found[2].3, b"\x89PNG two hundred and fifty-six");
        // The group: two entries, the 256 written as 0, each naming its image's id.
        let group = &found[3].3;
        assert_eq!(&group[..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(group.len(), 6 + 2 * 14);
        assert_eq!(group[6 + 14], 0);
        assert_eq!(&group[6 + 14 + 12..6 + 14 + 14], &[2, 0]);
        assert_eq!(found[5].3, b"<assembly/>");
    }

    /// The node at `at` in a version table: its length, value length, text flag and key, and
    /// where its value starts.
    fn read_node(table: &[u8], at: usize) -> (usize, usize, u16, String, usize) {
        let u16_at = |at: usize| u16::from_le_bytes(table[at..at + 2].try_into().unwrap());
        let mut key = Vec::new();
        let mut end = at + 6;
        while u16_at(end) != 0 {
            key.push(u16_at(end));
            end += 2;
        }
        (
            usize::from(u16_at(at)),
            usize::from(u16_at(at + 2)),
            u16_at(at + 4),
            String::from_utf16(&key).unwrap(),
            (end + 2).next_multiple_of(4),
        )
    }

    #[test]
    fn the_version_details_nest_as_windows_reads_them() {
        let info = version_info(&details()).unwrap();
        let (length, value_length, text, key, value) = read_node(&info, 0);
        assert_eq!((length, value_length, text), (info.len(), 52, 0));
        assert_eq!(key, "VS_VERSION_INFO");
        let fixed = |n: usize| {
            u32::from_le_bytes(info[value + n * 4..value + n * 4 + 4].try_into().unwrap())
        };
        assert_eq!(fixed(0), 0xfeef_04bd);
        assert_eq!((fixed(2), fixed(3)), (31, 2 << 16));
        // StringFileInfo, then its table, then the strings.
        let (string_info_length, _, _, key, table) = read_node(&info, value + 52);
        assert_eq!(key, "StringFileInfo");
        let (_, _, _, key, first) = read_node(&info, table);
        assert_eq!(key, "040904B0");
        let mut at = first;
        let mut strings = Vec::new();
        while at < value + 52 + string_info_length {
            let (length, value_length, text, key, value) = read_node(&info, at);
            assert_eq!(text, 1);
            let units: Vec<u16> = info[value..value + value_length * 2]
                .chunks(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            strings.push((key, String::from_utf16(&units[..units.len() - 1]).unwrap()));
            at = (at + length).next_multiple_of(4);
        }
        assert_eq!(
            strings,
            [
                ("CompanyName", "Example Garden"),
                ("FileDescription", "Moss Box"),
                ("FileVersion", "0.31.2"),
                ("InternalName", "moss-box"),
                ("LegalCopyright", "Code MIT"),
                ("OriginalFilename", "moss-box.exe"),
                ("ProductName", "Moss Box"),
                ("ProductVersion", "0.31.2"),
            ]
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
        );
        // VarFileInfo's translation names the table.
        let (_, _, _, key, var) = read_node(&info, at);
        assert_eq!(key, "VarFileInfo");
        let (_, value_length, text, key, value) = read_node(&info, var);
        assert_eq!((key.as_str(), value_length, text), ("Translation", 4, 0));
        assert_eq!(&info[value..value + 4], &[0x09, 0x04, 0xb0, 0x04]);
        assert!(
            version_info(&Details {
                version: "0.31",
                ..details()
            })
            .is_err()
        );
    }
}
