//! The alias Finder follows to a DMG window's background picture: an Alias Manager record,
//! version 2, as dmgbuild writes it. Finder finds the picture by the volume's name and creation
//! date, then by the file's catalog id, then by its path.
//!
//! The picture sits in the volume's root, so its folder is the root (catalog id 2) and the
//! record carries no path of folder ids. The volume's path is where a person's Mac mounts it,
//! `/Volumes/<name>`, never where the build mounted it.

/// What the alias names, read from the mounted volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub volume: String,
    /// The volume's creation date, in seconds since 1904 (HFS+'s epoch).
    pub volume_created: u32,
    pub folder_id: u32,
    pub file: String,
    pub file_id: u32,
    pub file_created: u32,
}

/// Seconds between 1904-01-01 and 1970-01-01, both UTC.
pub const HFS_EPOCH_OFFSET: u64 = 2_082_844_800;

/// The record. Names longer than its fixed fields hold, or outside ASCII, are refused: the
/// product's name and its picture's are neither.
pub fn record(target: &Target) -> Result<Vec<u8>, String> {
    for (name, longest) in [(&target.volume, 27), (&target.file, 63)] {
        if !name.is_ascii() || name.len() > longest {
            return Err(format!(
                "{name:?} does not fit an alias: ASCII, {longest} bytes at most"
            ));
        }
    }
    let mut out = Vec::new();
    out.extend([0; 4]); // the creator's own data
    out.extend([0; 2]); // the record's length, set below
    out.extend(2_u16.to_be_bytes()); // version 2
    out.extend(0_u16.to_be_bytes()); // a file
    out.extend(pascal(&target.volume, 28));
    out.extend(target.volume_created.to_be_bytes());
    out.extend(b"H+");
    out.extend(0_u16.to_be_bytes()); // a fixed disk
    out.extend(target.folder_id.to_be_bytes());
    out.extend(pascal(&target.file, 64));
    out.extend(target.file_id.to_be_bytes());
    out.extend(target.file_created.to_be_bytes());
    out.extend([0; 8]); // creator and type codes
    out.extend((-1_i16).to_be_bytes()); // levels from the volume to the file, unknown
    out.extend((-1_i16).to_be_bytes());
    out.extend([0; 4]); // volume attributes
    out.extend([0; 2]); // file system id
    out.extend([0; 10]);
    let volume_path = format!("/Volumes/{}", target.volume);
    let file_path = format!("/{}", target.file);
    for (tag, value) in [
        (0, target.volume.as_bytes().to_vec()),
        (16, precise(target.volume_created)),
        (17, precise(target.file_created)),
        (2, format!("{}:{}", target.volume, target.file).into_bytes()),
        (14, unicode(&target.file)),
        (15, unicode(&target.volume)),
        (18, file_path.into_bytes()),
        (19, volume_path.into_bytes()),
    ] {
        out.extend(i16::to_be_bytes(tag));
        out.extend(len16(value.len()).to_be_bytes());
        out.extend(&value);
        if value.len() % 2 == 1 {
            out.push(0);
        }
    }
    out.extend((-1_i16).to_be_bytes());
    out.extend(0_u16.to_be_bytes());
    let length = len16(out.len()).to_be_bytes();
    out[4..6].copy_from_slice(&length);
    Ok(out)
}

/// A Pascal string padded to `width` bytes, its length byte included.
fn pascal(s: &str, width: usize) -> Vec<u8> {
    let mut out = vec![u8::try_from(s.len()).unwrap_or(u8::MAX)];
    out.extend(s.as_bytes());
    out.resize(width, 0);
    out
}

/// A date as the record's extras keep it: seconds as 16.16 fixed point, in 64 bits.
fn precise(seconds: u32) -> Vec<u8> {
    (u64::from(seconds) << 16).to_be_bytes().to_vec()
}

/// A name as UTF-16, after its length in code units.
fn unicode(s: &str) -> Vec<u8> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut out = len16(units.len()).to_be_bytes().to_vec();
    out.extend(units.iter().flat_map(|unit| unit.to_be_bytes()));
    out
}

fn len16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> Target {
        Target {
            volume: "Repo Terrarium 0.31.1".to_owned(),
            volume_created: 0xE6FE_707C,
            folder_id: 2,
            file: ".background.tiff".to_owned(),
            file_id: 19,
            file_created: 0xE6FE_707D,
        }
    }

    #[test]
    fn the_header_holds_the_volume_and_the_file() {
        let out = record(&target()).unwrap();
        assert_eq!(usize::from(u16::from_be_bytes([out[4], out[5]])), out.len());
        assert_eq!(out[6..10], [0, 2, 0, 0]);
        assert_eq!(out[10], 21);
        assert_eq!(&out[11..32], b"Repo Terrarium 0.31.1");
        assert!(out[32..38].iter().all(|&b| b == 0));
        assert_eq!(out[38..42], 0xE6FE_707C_u32.to_be_bytes());
        assert_eq!(&out[42..44], b"H+");
        assert_eq!(out[46..50], 2_u32.to_be_bytes());
        assert_eq!(out[50], 16);
        assert_eq!(&out[51..67], b".background.tiff");
        assert_eq!(out[114..118], 19_u32.to_be_bytes());
        assert_eq!(out[118..122], 0xE6FE_707D_u32.to_be_bytes());
        assert_eq!(out[130..134], [0xFF; 4]);
    }

    /// The extras, read back as Finder reads them: each tag, its length, its value padded to an
    /// even length, then the end.
    #[test]
    fn the_extras_name_the_volume_as_it_is_mounted_and_end_with_the_end_tag() {
        let out = record(&target()).unwrap();
        let mut p = 150;
        let mut tags = Vec::new();
        loop {
            let tag = i16::from_be_bytes([out[p], out[p + 1]]);
            let length = usize::from(u16::from_be_bytes([out[p + 2], out[p + 3]]));
            p += 4;
            if tag == -1 {
                assert_eq!(length, 0);
                break;
            }
            tags.push((tag, out[p..p + length].to_vec()));
            p += length + length % 2;
        }
        assert_eq!(p, out.len());
        let value = |tag: i16| tags.iter().find(|(t, _)| *t == tag).unwrap().1.clone();
        assert_eq!(
            tags.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
            [0, 16, 17, 2, 14, 15, 18, 19]
        );
        assert_eq!(value(0), b"Repo Terrarium 0.31.1");
        assert_eq!(value(16), [0, 0, 0xE6, 0xFE, 0x70, 0x7C, 0, 0]);
        assert_eq!(value(2), b"Repo Terrarium 0.31.1:.background.tiff");
        assert_eq!(value(14)[..4], [0, 16, 0, b'.']);
        assert_eq!(value(18), b"/.background.tiff");
        assert_eq!(value(19), b"/Volumes/Repo Terrarium 0.31.1");
    }

    #[test]
    fn a_name_its_fields_cannot_hold_is_refused() {
        let mut long = target();
        long.volume = "A volume name of twenty-eight".to_owned();
        assert!(record(&long).is_err());
        let mut accented = target();
        accented.volume = "Terrário".to_owned();
        assert!(record(&accented).is_err());
    }
}
