//! The canonical text dump: truth's golden format.
//!
//! A value is written as lines of `path = value`, in ASCII, each ending in LF:
//!
//! ```text
//! presentation.state = Thriving
//! presentation.since_tick = 1200
//! plants = [1]
//! plants[0].id = 7
//! plants[0].species = Oak
//! event = None
//! ```
//!
//! - Struct fields come in declaration order, joined with `.`.
//! - A sequence is its length `[N]`, then each element at `[i]`.
//! - A map is its size `{N}`, then each entry at `[key]`, in the key type's order.
//! - An option is `None`, or its value under `.some`.
//! - A unit variant is its name; a variant with fields is its name, then its fields.
//! - Integers are decimal, `Q16` and `Q32` exact decimals, strings quoted and escaped, byte
//!   arrays lowercase hex.
//!
//! The reader accepts exactly what the writer produces, so text and value map one to one and
//! the SHA-256 of the text identifies the value. The format is part of truth's output and is
//! versioned by `SIM_VERSION`.
//!
//! It is not built on serde because a `serde::Serializer` has to name `f32` and `f64`, which
//! truth's lint wall forbids. `Canonical` has no impl for `usize`, `f32` or `f64`, so none of
//! them can reach dumped state.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use crate::fixed::{Q16, Q32};
use crate::identity::{CommitId, RepoSeed, SimVersion};
use crate::time::Tick;

/// A value with a canonical text form. Implement it with [`canonical_struct!`] or
/// [`canonical_enum!`]; a hand-written impl destructures without `..` and matches
/// exhaustively, so a new field or variant cannot be left out silently.
pub trait Canonical: Sized {
    fn write(&self, w: &mut Writer);
    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError>;
}

/// A value that can key a map in the dump. Its text has no `[`, `]`, `.`, spaces or quotes.
pub trait CanonicalKey: Ord + Sized {
    fn key(&self) -> String;
    fn from_key(text: &str) -> Option<Self>;
}

/// Why a text is not the canonical dump of the type asked for. Lines count from 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DumpError {
    /// The text is empty or does not end in LF.
    Unterminated,
    /// The text ended while `path` still needed a line.
    UnexpectedEnd { path: String },
    /// Line `line` is not the line for `path`.
    UnexpectedLine { line: usize, path: String },
    /// Line `line` holds `value`, which is not the canonical form of its type.
    BadValue { line: usize, value: String },
    /// Lines remain after the value ended, from `line` on.
    TrailingLines { line: usize },
}

/// The canonical dump of `value`.
pub fn canonical_dump<T: Canonical>(value: &T) -> String {
    let mut writer = Writer {
        text: String::new(),
        path: String::new(),
    };
    value.write(&mut writer);
    writer.text
}

/// The value whose canonical dump is `text`.
pub fn from_canonical_dump<T: Canonical>(text: &str) -> Result<T, DumpError> {
    let body = text.strip_suffix('\n').ok_or(DumpError::Unterminated)?;
    let mut reader = Reader {
        lines: body.split('\n').collect(),
        next: 0,
        path: String::new(),
    };
    let value = T::read(&mut reader)?;
    if reader.next < reader.lines.len() {
        return Err(DumpError::TrailingLines {
            line: reader.next + 1,
        });
    }
    Ok(value)
}

/// SHA-256 of the canonical dump of `value`.
pub fn canonical_sha256<T: Canonical>(value: &T) -> [u8; 32] {
    Sha256::digest(canonical_dump(value).as_bytes()).into()
}

/// Lowercase hex.
pub fn to_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// The inverse of [`to_hex`]: only lowercase hex of even length parses.
pub(crate) fn from_hex(text: &str) -> Option<Vec<u8>> {
    let (pairs, []) = text.as_bytes().as_chunks::<2>() else {
        return None;
    };
    let nibble = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    };
    pairs
        .iter()
        .map(|&[high, low]| Some((nibble(high)? << 4) | nibble(low)?))
        .collect()
}

/// Collects the lines of a dump.
pub struct Writer {
    text: String,
    path: String,
}

impl Writer {
    /// Writes the line for the current path.
    pub fn leaf(&mut self, value: &str) {
        self.text.push_str(&self.path);
        self.text.push_str(" = ");
        self.text.push_str(value);
        self.text.push('\n');
    }

    /// Writes `value` under `.name`.
    pub fn field<T: Canonical>(&mut self, name: &str, value: &T) {
        let depth = push_field(&mut self.path, name);
        value.write(self);
        self.path.truncate(depth);
    }

    fn element<T: Canonical>(&mut self, key: &str, value: &T) {
        let depth = push_index(&mut self.path, key);
        value.write(self);
        self.path.truncate(depth);
    }
}

/// Reads a dump back, line by line, in the order the writer wrote it.
pub struct Reader<'a> {
    lines: Vec<&'a str>,
    next: usize,
    path: String,
}

impl<'a> Reader<'a> {
    /// The value on the current path's line.
    pub fn leaf(&mut self) -> Result<&'a str, DumpError> {
        let Some(line) = self.lines.get(self.next).copied() else {
            return Err(DumpError::UnexpectedEnd {
                path: self.path.clone(),
            });
        };
        let value = line
            .strip_prefix(self.path.as_str())
            .and_then(|rest| rest.strip_prefix(" = "))
            .ok_or_else(|| DumpError::UnexpectedLine {
                line: self.next + 1,
                path: self.path.clone(),
            })?;
        self.next += 1;
        Ok(value)
    }

    /// The value under `.name`.
    pub fn field<T: Canonical>(&mut self, name: &str) -> Result<T, DumpError> {
        let depth = push_field(&mut self.path, name);
        let value = T::read(self);
        self.path.truncate(depth);
        value
    }

    /// The error for `value`, the value just read.
    pub fn bad_value(&self, value: &str) -> DumpError {
        DumpError::BadValue {
            line: self.next,
            value: value.to_owned(),
        }
    }

    /// Reads a leaf through `parse`, accepting only the text `render` gives back.
    fn parsed<T>(
        &mut self,
        parse: impl FnOnce(&str) -> Option<T>,
        render: impl FnOnce(&T) -> String,
    ) -> Result<T, DumpError> {
        let text = self.leaf()?;
        parse(text)
            .filter(|value| render(value) == text)
            .ok_or_else(|| self.bad_value(text))
    }

    fn element<T: Canonical>(&mut self, key: &str) -> Result<T, DumpError> {
        let depth = push_index(&mut self.path, key);
        let value = T::read(self);
        self.path.truncate(depth);
        value
    }

    /// Whether the next line is exactly the current path's line with `value`.
    fn next_is(&self, value: &str) -> bool {
        self.lines.get(self.next).is_some_and(|line| {
            line.strip_prefix(self.path.as_str())
                .and_then(|rest| rest.strip_prefix(" = "))
                == Some(value)
        })
    }

    /// The key of the next map entry under the current path, as written between the brackets.
    fn next_key(&self) -> Result<&'a str, DumpError> {
        let line = self
            .lines
            .get(self.next)
            .copied()
            .ok_or_else(|| DumpError::UnexpectedEnd {
                path: self.path.clone(),
            })?;
        line.strip_prefix(self.path.as_str())
            .and_then(|rest| rest.strip_prefix('['))
            .and_then(|rest| rest.split_once(']'))
            .map(|(key, _)| key)
            .ok_or_else(|| DumpError::UnexpectedLine {
                line: self.next + 1,
                path: self.path.clone(),
            })
    }
}

fn push_field(path: &mut String, name: &str) -> usize {
    let depth = path.len();
    if !path.is_empty() {
        path.push('.');
    }
    path.push_str(name);
    depth
}

fn push_index(path: &mut String, key: &str) -> usize {
    let depth = path.len();
    path.push('[');
    path.push_str(key);
    path.push(']');
    depth
}

/// `Canonical` for a struct with named fields. The list must name every field once: `write`
/// destructures without `..` and `read` builds a struct literal, so a missing, extra or
/// misspelt field is a compile error.
macro_rules! canonical_struct {
    ($ty:ident { $($field:ident),+ $(,)? }) => {
        impl $crate::canon::Canonical for $ty {
            fn write(&self, w: &mut $crate::canon::Writer) {
                let $ty { $($field),+ } = self;
                $( w.field(stringify!($field), $field); )+
            }

            fn read(
                r: &mut $crate::canon::Reader<'_>,
            ) -> Result<Self, $crate::canon::DumpError> {
                Ok($ty { $( $field: r.field(stringify!($field))?, )+ })
            }
        }
    };
}

/// `Canonical` and `CanonicalKey` for an enum of unit variants, plus `ALL` and `name()`. The
/// list must name every variant once, in declaration order: `name()` matches exhaustively, so a
/// missing or unknown variant is a compile error, and a test checks `ALL` against the postcard
/// encoding, which follows declaration order.
macro_rules! canonical_enum {
    ($ty:ident { $($variant:ident),+ $(,)? }) => {
        impl $ty {
            /// Every variant, in declaration order.
            pub const ALL: &'static [$ty] = &[$($ty::$variant),+];

            /// The variant's name, as the canonical dump writes it.
            pub const fn name(self) -> &'static str {
                match self {
                    $( $ty::$variant => stringify!($variant), )+
                }
            }

            /// The variant named `name`.
            pub fn from_name(name: &str) -> Option<$ty> {
                $ty::ALL.iter().copied().find(|variant| variant.name() == name)
            }
        }

        impl $crate::canon::Canonical for $ty {
            fn write(&self, w: &mut $crate::canon::Writer) {
                w.leaf(self.name());
            }

            fn read(
                r: &mut $crate::canon::Reader<'_>,
            ) -> Result<Self, $crate::canon::DumpError> {
                let text = r.leaf()?;
                $ty::from_name(text).ok_or_else(|| r.bad_value(text))
            }
        }

        impl $crate::canon::CanonicalKey for $ty {
            fn key(&self) -> String {
                self.name().to_owned()
            }

            fn from_key(text: &str) -> Option<Self> {
                $ty::from_name(text)
            }
        }
    };
}

pub(crate) use canonical_enum;
pub(crate) use canonical_struct;

/// `Canonical` and `CanonicalKey` for an integer type, in plain decimal.
macro_rules! canonical_integer {
    ($($ty:ty),+) => {$(
        impl Canonical for $ty {
            fn write(&self, w: &mut Writer) {
                w.leaf(&self.to_string());
            }

            fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
                r.parsed(|text| text.parse().ok(), ToString::to_string)
            }
        }

        impl CanonicalKey for $ty {
            fn key(&self) -> String {
                self.to_string()
            }

            fn from_key(text: &str) -> Option<Self> {
                text.parse().ok().filter(|value: &$ty| value.to_string() == text)
            }
        }
    )+};
}

canonical_integer!(u8, u16, u32, u64, i8, i16, i32, i64);

impl Canonical for bool {
    fn write(&self, w: &mut Writer) {
        w.leaf(if *self { "true" } else { "false" });
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        match r.leaf()? {
            "true" => Ok(true),
            "false" => Ok(false),
            other => Err(r.bad_value(other)),
        }
    }
}

impl Canonical for String {
    fn write(&self, w: &mut Writer) {
        w.leaf(&quote(self));
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        r.parsed(unquote, |value| quote(value))
    }
}

/// `"` + the text + `"`. Printable ASCII stays as it is, except `"` and `\`, which are escaped
/// with `\`; every other character is `\u{…}` with its code point in lowercase hex.
fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for c in text.chars() {
        match c {
            '"' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            ' '..='~' => quoted.push(c),
            _ => {
                let _ = write!(quoted, "\\u{{{:x}}}", u32::from(c));
            }
        }
    }
    quoted.push('"');
    quoted
}

fn unquote(text: &str) -> Option<String> {
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    let mut value = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            value.push(c);
            continue;
        }
        match chars.next()? {
            escaped @ ('"' | '\\') => value.push(escaped),
            'u' => {
                let rest = chars.as_str().strip_prefix('{')?;
                let (hex, after) = rest.split_once('}')?;
                value.push(char::from_u32(u32::from_str_radix(hex, 16).ok()?)?);
                chars = after.chars();
            }
            _ => return None,
        }
    }
    Some(value)
}

impl<T: Canonical> Canonical for Vec<T> {
    fn write(&self, w: &mut Writer) {
        w.leaf(&format!("[{}]", self.len()));
        for (index, item) in self.iter().enumerate() {
            w.element(&index.to_string(), item);
        }
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        let length: usize = r.parsed(
            |text| text.strip_prefix('[')?.strip_suffix(']')?.parse().ok(),
            |length| format!("[{length}]"),
        )?;
        // Every element needs at least one line, so a forged length cannot outgrow the text.
        let mut items = Vec::new();
        for index in 0..length {
            items.push(r.element(&index.to_string())?);
        }
        Ok(items)
    }
}

impl<T: Canonical> Canonical for Option<T> {
    fn write(&self, w: &mut Writer) {
        match self {
            None => w.leaf("None"),
            Some(value) => w.field("some", value),
        }
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        if r.next_is("None") {
            r.leaf()?;
            Ok(None)
        } else {
            r.field("some").map(Some)
        }
    }
}

impl<K: CanonicalKey, V: Canonical> Canonical for BTreeMap<K, V> {
    fn write(&self, w: &mut Writer) {
        w.leaf(&format!("{{{}}}", self.len()));
        for (key, value) in self {
            w.element(&key.key(), value);
        }
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        let size: usize = r.parsed(
            |text| text.strip_prefix('{')?.strip_suffix('}')?.parse().ok(),
            |size| format!("{{{size}}}"),
        )?;
        let mut map = BTreeMap::new();
        for _ in 0..size {
            let text = r.next_key()?;
            let key = K::from_key(text)
                .filter(|key| key.key() == text)
                // Keys must arrive in order, so the text of a map is unique.
                .filter(|key| map.last_key_value().is_none_or(|(last, _)| last < key))
                .ok_or_else(|| DumpError::BadValue {
                    line: r.next + 1,
                    value: text.to_owned(),
                })?;
            let value = r.element(text)?;
            map.insert(key, value);
        }
        Ok(map)
    }
}

impl Canonical for Q16 {
    fn write(&self, w: &mut Writer) {
        w.leaf(&render_fixed(i128::from(self.raw()), Q16::FRAC_BITS));
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        r.parsed(
            |text| {
                let raw = parse_fixed(text, Q16::FRAC_BITS)?;
                i32::try_from(raw).ok().map(Q16::from_raw)
            },
            |value| render_fixed(i128::from(value.raw()), Q16::FRAC_BITS),
        )
    }
}

impl Canonical for Q32 {
    fn write(&self, w: &mut Writer) {
        w.leaf(&render_fixed(i128::from(self.raw()), Q32::FRAC_BITS));
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        r.parsed(
            |text| {
                let raw = parse_fixed(text, Q32::FRAC_BITS)?;
                i64::try_from(raw).ok().map(Q32::from_raw)
            },
            |value| render_fixed(i128::from(value.raw()), Q32::FRAC_BITS),
        )
    }
}

/// The exact decimal of `raw / 2^frac_bits`: `2⁻ᵏ = 5ᵏ / 10ᵏ`, so the fraction has at most
/// `frac_bits` digits. Trailing zeros are dropped, and a whole number has no point.
fn render_fixed(raw: i128, frac_bits: u32) -> String {
    let magnitude = raw.unsigned_abs();
    let whole = magnitude >> frac_bits;
    let fraction = magnitude & ((1 << frac_bits) - 1);
    let mut text = String::new();
    if raw < 0 {
        text.push('-');
    }
    let _ = write!(text, "{whole}");
    if fraction != 0 {
        let digits = (fraction * 5u128.pow(frac_bits)).to_string();
        let padding = frac_width(frac_bits).saturating_sub(digits.len());
        text.push('.');
        text.extend(std::iter::repeat_n('0', padding));
        text.push_str(digits.trim_end_matches('0'));
    }
    text
}

/// The raw value of an exact decimal with at most `frac_bits` fraction digits. Anything not
/// exactly representable is rejected rather than rounded.
fn parse_fixed(text: &str, frac_bits: u32) -> Option<i128> {
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let digits_only = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || !digits_only(whole) || !digits_only(fraction) {
        return None;
    }
    if fraction.len() > frac_width(frac_bits) {
        return None;
    }
    let whole: i128 = whole.parse().ok()?;
    let mut raw = whole.checked_mul(1 << frac_bits)?;
    if !fraction.is_empty() {
        // digits / 10ⁿ × 2ᵏ = digits / 5ⁿ × 2ᵏ⁻ⁿ, which stays within 10ᵏ for k ≤ 32.
        let places = u32::try_from(fraction.len()).ok()?;
        let fives = 5i128.checked_pow(places)?;
        let digits: i128 = fraction.parse().ok()?;
        if digits % fives != 0 {
            return None;
        }
        raw = raw.checked_add((digits / fives) << (frac_bits - places))?;
    }
    Some(if negative { -raw } else { raw })
}

/// The most fraction digits a `frac_bits` fixed-point value needs.
fn frac_width(frac_bits: u32) -> usize {
    usize::try_from(frac_bits).unwrap_or(usize::MAX)
}

impl Canonical for Tick {
    fn write(&self, w: &mut Writer) {
        self.hours().write(w);
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        u32::read(r).map(Tick::new)
    }
}

impl Canonical for SimVersion {
    fn write(&self, w: &mut Writer) {
        self.0.write(w);
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        u32::read(r).map(SimVersion)
    }
}

impl Canonical for RepoSeed {
    fn write(&self, w: &mut Writer) {
        w.leaf(&self.to_hex());
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        r.parsed(RepoSeed::from_hex, RepoSeed::to_hex)
    }
}

impl Canonical for CommitId {
    fn write(&self, w: &mut Writer) {
        w.leaf(self.as_str());
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        r.parsed(CommitId::from_hex, |id| id.as_str().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::scene::PlantSpecies;
    use crate::test_support::proptest_config;

    /// A struct wrapper, so values dump under a field name as they do in real state.
    #[derive(Debug, PartialEq)]
    struct Holder<T> {
        value: T,
    }

    impl<T: Canonical> Canonical for Holder<T> {
        fn write(&self, w: &mut Writer) {
            let Holder { value } = self;
            w.field("value", value);
        }

        fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
            Ok(Holder {
                value: r.field("value")?,
            })
        }
    }

    fn dump<T: Canonical>(value: T) -> String {
        canonical_dump(&Holder { value })
    }

    fn parse<T: Canonical>(text: &str) -> Result<T, DumpError> {
        from_canonical_dump::<Holder<T>>(text).map(|holder| holder.value)
    }

    fn round_trips<T: Canonical + std::fmt::Debug + PartialEq + Clone>(value: &T) -> bool {
        parse::<T>(&dump(value.clone())) == Ok(value.clone())
    }

    #[test]
    fn fixed_point_is_exact_decimal() {
        let q16 = |raw| dump(Q16::from_raw(raw));
        assert_eq!(q16(0), "value = 0\n");
        assert_eq!(q16(1 << 16), "value = 1\n");
        assert_eq!(q16(1 << 15), "value = 0.5\n");
        assert_eq!(q16(-(5 << 14)), "value = -1.25\n");
        assert_eq!(q16(1), "value = 0.0000152587890625\n");
        assert_eq!(q16(i32::MIN), "value = -32768\n");
        assert_eq!(q16(i32::MAX), "value = 32767.9999847412109375\n");
        assert_eq!(
            dump(Q32::EPSILON),
            "value = 0.00000000023283064365386962890625\n"
        );
        assert_eq!(dump(Q32::MIN), "value = -2147483648\n");
    }

    #[test]
    fn only_the_canonical_form_parses() {
        for text in [
            "-0", "1.50", "01", "+1", "1.", ".5", "0.00001", "1e3", " 1", "1 ", "32768", "",
        ] {
            assert!(
                parse::<Q16>(&format!("value = {text}\n")).is_err(),
                "{text:?} parsed"
            );
        }
        for text in ["+1", "01", "-0", "1_000", "4294967296"] {
            assert!(parse::<u32>(&format!("value = {text}\n")).is_err());
        }
        assert!(parse::<bool>("value = True\n").is_err());
        assert!(parse::<String>("value = \"\\n\"\n").is_err());
        assert!(parse::<String>("value = \"\\u{41}\"\n").is_err());
        assert!(parse::<PlantSpecies>("value = oak\n").is_err());
    }

    #[test]
    fn strings_escape_everything_but_printable_ascii() {
        let text = "a\"b\\c\nd\u{e9}";
        assert_eq!(
            dump(text.to_owned()),
            "value = \"a\\\"b\\\\c\\u{a}d\\u{e9}\"\n"
        );
        assert!(round_trips(&text.to_owned()));
    }

    #[test]
    fn collections_have_their_shapes() {
        assert_eq!(dump(Vec::<u8>::new()), "value = [0]\n");
        assert_eq!(
            dump(vec![Some(3_u8), None]),
            "value = [2]\nvalue[0].some = 3\nvalue[1] = None\n"
        );
        let map = BTreeMap::from([(10_u32, true), (2, false)]);
        assert_eq!(
            dump(map.clone()),
            "value = {2}\nvalue[2] = false\nvalue[10] = true\n"
        );
        assert!(round_trips(&map));
        let species = BTreeMap::from([(PlantSpecies::Reed, 1_u8), (PlantSpecies::Oak, 2)]);
        assert_eq!(
            dump(species.clone()),
            "value = {2}\nvalue[Oak] = 2\nvalue[Reed] = 1\n"
        );
        assert!(round_trips(&species));
        assert!(round_trips(&Some(Some(1_u64))));
        assert!(round_trips(&Some(None::<u64>)));
    }

    #[test]
    fn malformed_dumps_are_refused() {
        assert_eq!(parse::<u8>("value = 1"), Err(DumpError::Unterminated));
        assert_eq!(parse::<u8>(""), Err(DumpError::Unterminated));
        assert_eq!(
            parse::<u8>("value = 1\nvalue = 2\n"),
            Err(DumpError::TrailingLines { line: 2 })
        );
        assert_eq!(
            parse::<u8>("other = 1\n"),
            Err(DumpError::UnexpectedLine {
                line: 1,
                path: "value".to_owned()
            })
        );
        assert_eq!(
            parse::<Vec<u8>>("value = [2]\nvalue[0] = 1\n"),
            Err(DumpError::UnexpectedEnd {
                path: "value[1]".to_owned()
            })
        );
        assert!(parse::<Vec<u8>>("value = [1]\nvalue[1] = 1\n").is_err());
        // Out-of-order and duplicate keys would give one map two texts.
        assert!(parse::<BTreeMap<u32, u8>>("value = {2}\nvalue[10] = 1\nvalue[2] = 1\n").is_err());
        assert!(parse::<BTreeMap<u32, u8>>("value = {2}\nvalue[2] = 1\nvalue[2] = 1\n").is_err());
        assert!(parse::<BTreeMap<u32, u8>>("value = {1}\nvalue[02] = 1\n").is_err());
    }

    #[test]
    fn the_hash_is_sha256_of_the_text() {
        // SHA-256 of "value = 1\n", from `shasum -a 256`.
        assert_eq!(
            to_hex(&canonical_sha256(&Holder { value: 1_u8 })),
            "585c93666fcb046b7b264d3fa73202aa2a38254ae82a4b3ba19e873c2d5a9886"
        );
    }

    proptest! {
        #![proptest_config(proptest_config())]

        #[test]
        fn scalars_round_trip(a: i32, b: i64, c: u64, d: bool, e: String) {
            prop_assert!(round_trips(&Q16::from_raw(a)));
            prop_assert!(round_trips(&Q32::from_raw(b)));
            prop_assert!(round_trips(&c));
            prop_assert!(round_trips(&b));
            prop_assert!(round_trips(&d));
            prop_assert!(round_trips(&e));
        }

        #[test]
        fn hex_round_trips(bytes: Vec<u8>) {
            prop_assert_eq!(from_hex(&to_hex(&bytes)), Some(bytes));
        }
    }
}
