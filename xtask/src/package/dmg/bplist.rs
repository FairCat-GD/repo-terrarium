//! Binary property lists (`bplist00`), as Finder keeps a window's settings in a `.DS_Store`:
//! one dictionary of strings, numbers, booleans and data. Keys are written sorted, so the same
//! dictionary is always the same bytes.

use std::collections::BTreeMap;

/// A value in the dictionary.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(u32),
    Real(f64),
    Str(String),
    Data(Vec<u8>),
}

/// The dictionary as a binary property list. Strings outside ASCII are written as UTF-16.
pub fn write(dict: &BTreeMap<&str, Value>) -> Vec<u8> {
    // Object 0 is the dictionary, then each key, then each value.
    let count = 1 + 2 * dict.len();
    let reference = |index: usize| -> Vec<u8> {
        if count <= 0x100 {
            vec![u8::try_from(index).unwrap_or(u8::MAX)]
        } else {
            u16::try_from(index)
                .unwrap_or(u16::MAX)
                .to_be_bytes()
                .to_vec()
        }
    };
    let mut objects: Vec<Vec<u8>> = Vec::with_capacity(count);
    let mut top = marker(0xD, dict.len());
    for index in 0..dict.len() {
        top.extend(reference(1 + index));
    }
    for index in 0..dict.len() {
        top.extend(reference(1 + dict.len() + index));
    }
    objects.push(top);
    for key in dict.keys() {
        objects.push(string(key));
    }
    for value in dict.values() {
        objects.push(match value {
            Value::Bool(false) => vec![0x08],
            Value::Bool(true) => vec![0x09],
            Value::Int(n) => {
                let mut out = vec![0x12];
                out.extend(n.to_be_bytes());
                out
            }
            Value::Real(r) => {
                let mut out = vec![0x23];
                out.extend(r.to_be_bytes());
                out
            }
            Value::Str(s) => string(s),
            Value::Data(bytes) => {
                let mut out = marker(0x4, bytes.len());
                out.extend(bytes);
                out
            }
        });
    }
    let mut out = b"bplist00".to_vec();
    let mut offsets = Vec::with_capacity(count);
    for object in &objects {
        offsets.push(out.len());
        out.extend(object);
    }
    let table = out.len();
    let offset_size: u8 = if table <= 0xFFFF { 2 } else { 4 };
    for offset in offsets {
        if offset_size == 2 {
            out.extend(u16::try_from(offset).unwrap_or(u16::MAX).to_be_bytes());
        } else {
            out.extend(u32::try_from(offset).unwrap_or(u32::MAX).to_be_bytes());
        }
    }
    out.extend([0; 6]);
    out.push(offset_size);
    out.push(u8::try_from(reference(0).len()).unwrap_or(u8::MAX));
    out.extend(u64::try_from(count).unwrap_or(u64::MAX).to_be_bytes());
    out.extend(0_u64.to_be_bytes());
    out.extend(u64::try_from(table).unwrap_or(u64::MAX).to_be_bytes());
    out
}

/// A string object: ASCII as bytes, anything else as UTF-16.
fn string(s: &str) -> Vec<u8> {
    if s.is_ascii() {
        let mut out = marker(0x5, s.len());
        out.extend(s.as_bytes());
        out
    } else {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut out = marker(0x6, units.len());
        out.extend(units.iter().flat_map(|unit| unit.to_be_bytes()));
        out
    }
}

/// An object's type and length: the length in the marker's low nibble, or after it as an
/// integer object when it is 15 or more.
fn marker(kind: u8, length: usize) -> Vec<u8> {
    if length < 15 {
        vec![(kind << 4) | u8::try_from(length).unwrap_or(0)]
    } else if let Ok(short) = u16::try_from(length) {
        let mut out = vec![(kind << 4) | 0xF, 0x11];
        out.extend(short.to_be_bytes());
        out
    } else {
        let mut out = vec![(kind << 4) | 0xF, 0x12];
        out.extend(u32::try_from(length).unwrap_or(u32::MAX).to_be_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_dictionary_is_the_bytes_the_format_names() {
        let dict = BTreeMap::from([("a", Value::Bool(true)), ("n", Value::Int(2))]);
        let out = write(&dict);
        let mut want = b"bplist00".to_vec();
        // The dictionary: two pairs, keys 1 and 2, values 3 and 4.
        want.extend([0xD2, 1, 2, 3, 4]);
        want.extend([0x51, b'a', 0x51, b'n', 0x09, 0x12, 0, 0, 0, 2]);
        // Offsets of the five objects, then the trailer.
        want.extend([0, 8, 0, 13, 0, 15, 0, 17, 0, 18]);
        want.extend([0, 0, 0, 0, 0, 0, 2, 1]);
        want.extend(5_u64.to_be_bytes());
        want.extend(0_u64.to_be_bytes());
        want.extend(23_u64.to_be_bytes());
        assert_eq!(out, want);
    }

    #[test]
    fn long_strings_and_data_carry_their_length_after_the_marker() {
        assert_eq!(marker(0x4, 14), [0x4E]);
        assert_eq!(marker(0x4, 15), [0x4F, 0x11, 0, 15]);
        assert_eq!(marker(0x5, 70_000), [0x5F, 0x12, 0, 1, 0x11, 0x70]);
        assert_eq!(string("é"), [0x61, 0x00, 0xE9]);
    }

    /// macOS's own reader takes what this writes, every kind of value as it was.
    #[cfg(target_os = "macos")]
    #[test]
    fn plutil_reads_it_back() {
        use std::io::Write as _;
        use std::process::{Command, Stdio};
        let dict = BTreeMap::from([
            ("arrangeBy", Value::Str("none".to_owned())),
            ("backgroundImageAlias", Value::Data(vec![7; 300])),
            ("iconSize", Value::Real(128.0)),
            ("labelOnBottom", Value::Bool(true)),
            ("name", Value::Str("Terrário".to_owned())),
            ("viewOptionsVersion", Value::Int(1)),
        ]);
        let mut child = Command::new("plutil")
            .args(["-convert", "xml1", "-o", "-", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&write(&dict))
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        // plutil's spacing and line breaks are its own; only what it read is checked.
        let xml: String = String::from_utf8(out.stdout)
            .unwrap()
            .split_whitespace()
            .collect();
        let base64 = "BwcH".repeat(100);
        for part in [
            "<key>arrangeBy</key><string>none</string>",
            "<key>labelOnBottom</key><true/>",
            "<key>name</key><string>Terrário</string>",
            "<key>viewOptionsVersion</key><integer>1</integer>",
            &format!("<key>backgroundImageAlias</key><data>{base64}</data>"),
        ] {
            assert!(xml.contains(part), "{part} is not in\n{xml}");
        }
        assert!(
            xml.contains("<key>iconSize</key><real>128</real>")
                || xml.contains("<key>iconSize</key><real>128.0</real>")
        );
    }
}
