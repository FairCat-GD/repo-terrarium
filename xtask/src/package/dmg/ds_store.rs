//! Finder's `.DS_Store`: how a folder's window looks and where its icons sit, written without
//! Finder, so the DMG is made headless.
//!
//! The file is a buddy allocator (`Bud1`) holding one B-tree, `DSDB`, of records: a file name,
//! a four-letter code and a typed value. A DMG's window needs a handful of records, so the tree
//! is a single leaf, laid out as dmgbuild lays it out:
//!
//! - the header in the first 32 bytes of the allocator's space;
//! - `DSDB`'s block at 0x20 (32 bytes);
//! - the allocator's own block at 0x800 (2 KiB): the block table, the directory, the free lists;
//! - the leaf at 0x2000 (8 KiB).
//!
//! Every offset in the file counts from its fifth byte. The reader takes any tree, so the
//! package's check reads back what was written, and could read a `.DS_Store` Finder wrote.

/// A record's value. Finder's other types (`shor`, `ustr`, `comp`, `dutc`) are read, never
/// written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Long(u32),
    Bool(bool),
    Type([u8; 4]),
    Blob(Vec<u8>),
    Other([u8; 4], Vec<u8>),
}

/// One record: whose (`.` for the folder itself), what, and its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub name: String,
    pub code: [u8; 4],
    pub value: Value,
}

/// The allocator's blocks: its own, `DSDB`'s and the leaf's, each as offset | log2(size).
const ROOT_BLOCK: (u32, u32) = (0x800, 11);
const DSDB_BLOCK: (u32, u32) = (0x20, 5);
const LEAF_BLOCK: (u32, u32) = (0x2000, 13);
/// The B-tree's page size, as `DSDB` states it.
const PAGE: u32 = 0x1000;
/// The sixteen bytes after the header's offsets that Finder's files carry; dmgbuild writes
/// these.
const HEADER_TAIL: [u8; 16] = [
    0x00, 0x00, 0x10, 0x0c, 0x00, 0x00, 0x00, 0x87, 0x00, 0x00, 0x20, 0x0b, 0x00, 0x00, 0x00, 0x00,
];

/// The order Finder keeps records in: by name regardless of case, then by code.
fn order(a: &Record, b: &Record) -> std::cmp::Ordering {
    let key = |r: &Record| (r.name.to_lowercase(), r.code);
    key(a).cmp(&key(b))
}

/// `records` in the order Finder keeps them, which is the order `read` returns them in.
pub fn sorted(mut records: Vec<Record>) -> Vec<Record> {
    records.sort_by(order);
    records
}

/// The `.DS_Store` holding `records`, sorted as Finder sorts them. Too many to fit one leaf is
/// refused: a DMG's window never has that many.
pub fn write(records: &[Record]) -> Result<Vec<u8>, String> {
    let sorted = sorted(records.to_vec());
    let mut leaf = Vec::new();
    leaf.extend(0_u32.to_be_bytes());
    leaf.extend(len32(sorted.len()).to_be_bytes());
    for record in &sorted {
        leaf.extend(encode(record));
    }
    // A reader trusts the page size `DSDB` states, so a node holds no more than a page.
    if leaf.len() > usize::try_from(PAGE).unwrap_or(0) {
        return Err(format!(
            "the window's records take {} bytes, more than one leaf holds",
            leaf.len()
        ));
    }

    let mut dsdb = Vec::new();
    // The root node is block 2, at the bottom of a tree of one level, holding every record.
    for n in [2, 0, len32(sorted.len()), 1, PAGE] {
        dsdb.extend(n.to_be_bytes());
    }

    let mut root = Vec::new();
    root.extend(3_u32.to_be_bytes());
    root.extend(0_u32.to_be_bytes());
    for (offset, log2) in [ROOT_BLOCK, DSDB_BLOCK, LEAF_BLOCK] {
        root.extend((offset | log2).to_be_bytes());
    }
    root.resize(8 + 4 * 256, 0);
    root.extend(1_u32.to_be_bytes());
    root.push(4);
    root.extend(b"DSDB");
    root.extend(1_u32.to_be_bytes());
    for list in free_lists() {
        root.extend(len32(list.len()).to_be_bytes());
        for offset in list {
            root.extend(offset.to_be_bytes());
        }
    }

    let space = usize::try_from(LEAF_BLOCK.0 + (1 << LEAF_BLOCK.1)).unwrap_or(0);
    let mut file = vec![0; 4 + space];
    file[..4].copy_from_slice(&1_u32.to_be_bytes());
    let mut header = b"Bud1".to_vec();
    for n in [ROOT_BLOCK.0, 1 << ROOT_BLOCK.1, ROOT_BLOCK.0] {
        header.extend(n.to_be_bytes());
    }
    header.extend(HEADER_TAIL);
    place(&mut file, 0, &header);
    place(&mut file, DSDB_BLOCK.0, &dsdb);
    place(&mut file, ROOT_BLOCK.0, &root);
    place(&mut file, LEAF_BLOCK.0, &leaf);
    Ok(file)
}

/// The allocator's free blocks, one list per size: what the header and the three blocks leave
/// of its 2 GiB space. They are the buddies of each used block and of its ancestors, those that
/// overlap no used block.
fn free_lists() -> Vec<Vec<u32>> {
    let used = [(0, 5), DSDB_BLOCK, ROOT_BLOCK, LEAF_BLOCK];
    let overlaps = |offset: u32, log2: u32| {
        let end = u64::from(offset) + (1_u64 << log2);
        used.iter()
            .any(|&(o, l)| u64::from(o) < end && u64::from(offset) < u64::from(o) + (1_u64 << l))
    };
    let mut lists = vec![Vec::new(); 32];
    for &(offset, log2) in &used {
        for level in log2..31 {
            let buddy = (offset & !((1 << level) - 1)) ^ (1 << level);
            let list: &mut Vec<u32> = &mut lists[usize::try_from(level).unwrap_or(0)];
            if !overlaps(buddy, level) && !list.contains(&buddy) {
                list.push(buddy);
            }
        }
    }
    for list in &mut lists {
        list.sort_unstable();
    }
    lists
}

fn place(file: &mut [u8], offset: u32, bytes: &[u8]) {
    let at = 4 + usize::try_from(offset).unwrap_or(0);
    file[at..at + bytes.len()].copy_from_slice(bytes);
}

fn len32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn encode(record: &Record) -> Vec<u8> {
    let units: Vec<u16> = record.name.encode_utf16().collect();
    let mut out = len32(units.len()).to_be_bytes().to_vec();
    out.extend(units.iter().flat_map(|unit| unit.to_be_bytes()));
    out.extend(record.code);
    match &record.value {
        Value::Long(n) => {
            out.extend(b"long");
            out.extend(n.to_be_bytes());
        }
        Value::Bool(b) => {
            out.extend(b"bool");
            out.push(u8::from(*b));
        }
        Value::Type(t) => {
            out.extend(b"type");
            out.extend(t);
        }
        Value::Blob(bytes) => {
            out.extend(b"blob");
            out.extend(len32(bytes.len()).to_be_bytes());
            out.extend(bytes);
        }
        Value::Other(kind, bytes) => {
            out.extend(kind);
            out.extend(bytes);
        }
    }
    out
}

/// Every record in a `.DS_Store`, in the tree's order.
pub fn read(file: &[u8]) -> Result<Vec<Record>, String> {
    let bad = |what: &str| format!("the .DS_Store's {what} does not read");
    if u32_at(file, 0) != Some(1) || file.get(4..8) != Some(b"Bud1") {
        return Err(bad("header"));
    }
    let space = &file[4..];
    let at = |n: u32| usize::try_from(n).unwrap_or(usize::MAX);
    let (root_offset, root_size) = match (u32_at(space, 4), u32_at(space, 8)) {
        (Some(offset), Some(size)) => (at(offset), at(size)),
        _ => return Err(bad("header")),
    };
    let root = space
        .get(root_offset..root_offset.saturating_add(root_size))
        .ok_or_else(|| bad("allocator"))?;
    let blocks = at(u32_at(root, 0).ok_or_else(|| bad("allocator"))?);
    let addresses: Vec<u32> = (0..blocks)
        .map(|index| u32_at(root, 8 + 4 * index))
        .collect::<Option<_>>()
        .ok_or_else(|| bad("block table"))?;
    let mut p = 8 + 4 * blocks.div_ceil(256) * 256;
    let directories = u32_at(root, p).ok_or_else(|| bad("directory"))?;
    p += 4;
    let mut dsdb = None;
    for _ in 0..directories {
        let length = usize::from(*root.get(p).ok_or_else(|| bad("directory"))?);
        let name = root
            .get(p + 1..p + 1 + length)
            .ok_or_else(|| bad("directory"))?;
        let id = u32_at(root, p + 1 + length).ok_or_else(|| bad("directory"))?;
        p += 1 + length + 4;
        if name == b"DSDB" {
            dsdb = Some(id);
        }
    }
    let tree = Tree { space, addresses };
    let dsdb = tree.block(dsdb.ok_or_else(|| bad("DSDB"))?)?;
    let root_node = u32_at(dsdb, 0).ok_or_else(|| bad("DSDB"))?;
    let mut records = Vec::new();
    tree.walk(root_node, 0, &mut records)?;
    Ok(records)
}

/// The allocator's space and its block table.
struct Tree<'a> {
    space: &'a [u8],
    addresses: Vec<u32>,
}

impl Tree<'_> {
    fn block(&self, id: u32) -> Result<&[u8], String> {
        let bad = || "a .DS_Store block does not read".to_owned();
        let address = *self
            .addresses
            .get(usize::try_from(id).map_err(|_| bad())?)
            .ok_or_else(bad)?;
        let offset = usize::try_from(address & !0x1F).map_err(|_| bad())?;
        let size = 1_usize << (address & 0x1F);
        self.space
            .get(offset..offset.saturating_add(size))
            .ok_or_else(bad)
    }

    /// The node's records in order: an internal node's each after its child's, and its last
    /// child's after all of them. A tree deeper than any Finder writes is refused.
    fn walk(&self, node: u32, depth: u32, records: &mut Vec<Record>) -> Result<(), String> {
        let bad = || "a .DS_Store node does not read".to_owned();
        if depth > 16 {
            return Err(bad());
        }
        let bytes = self.block(node)?;
        let last = u32_at(bytes, 0).ok_or_else(bad)?;
        let count = u32_at(bytes, 4).ok_or_else(bad)?;
        let mut p = 8;
        for _ in 0..count {
            if last != 0 {
                self.walk(u32_at(bytes, p).ok_or_else(bad)?, depth + 1, records)?;
                p += 4;
            }
            let (record, next) = decode(bytes, p)?;
            records.push(record);
            p = next;
        }
        if last != 0 {
            self.walk(last, depth + 1, records)?;
        }
        Ok(())
    }
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at.checked_add(4)?)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_be_bytes)
}

/// One record at `p`, and where the next begins.
fn decode(bytes: &[u8], p: usize) -> Result<(Record, usize), String> {
    let bad = || "a .DS_Store record does not read".to_owned();
    let take = |from: usize, n: usize| {
        bytes
            .get(from..from.checked_add(n).ok_or_else(bad)?)
            .ok_or_else(bad)
    };
    let length_at = |from: usize| -> Result<usize, String> {
        u32_at(bytes, from)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(bad)
    };
    let units = length_at(p)?;
    let name: Vec<u16> = take(p + 4, 2 * units)?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_be_bytes(pair))
        .collect();
    let name = String::from_utf16(&name).map_err(|_| bad())?;
    let mut q = p + 4 + 2 * units;
    let code: [u8; 4] = take(q, 4)?.try_into().map_err(|_| bad())?;
    let kind: [u8; 4] = take(q + 4, 4)?.try_into().map_err(|_| bad())?;
    q += 8;
    let (value, length) = match &kind {
        b"long" => (Value::Long(u32_at(bytes, q).ok_or_else(bad)?), 4),
        b"bool" => (Value::Bool(take(q, 1)?[0] != 0), 1),
        b"type" => (Value::Type(take(q, 4)?.try_into().map_err(|_| bad())?), 4),
        b"blob" => {
            let n = length_at(q)?;
            (Value::Blob(take(q + 4, n)?.to_vec()), 4 + n)
        }
        b"shor" => (Value::Other(kind, take(q, 4)?.to_vec()), 4),
        b"comp" | b"dutc" => (Value::Other(kind, take(q, 8)?.to_vec()), 8),
        b"ustr" => {
            let n = length_at(q)?;
            (Value::Other(kind, take(q, 4 + 2 * n)?.to_vec()), 4 + 2 * n)
        }
        _ => return Err(bad()),
    };
    Ok((Record { name, code, value }, q + length))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records() -> Vec<Record> {
        let record = |name: &str, code: &[u8; 4], value| Record {
            name: name.to_owned(),
            code: *code,
            value,
        };
        vec![
            record("Zebra.app", b"Iloc", Value::Blob(vec![1; 16])),
            record(".", b"vSrn", Value::Long(1)),
            record("applications", b"Iloc", Value::Blob(vec![2; 16])),
            record(".", b"icvl", Value::Type(*b"icnv")),
            record(".", b"bwsp", Value::Blob(vec![3; 5])),
            record("é", b"dilc", Value::Bool(true)),
        ]
    }

    /// The layout dmgbuild writes, which Finder reads: the header, the three blocks, and the
    /// allocator's free lists.
    #[test]
    fn the_file_is_laid_out_as_dmgbuild_lays_it_out() {
        let file = write(&records()).unwrap();
        assert_eq!(file.len(), 4 + 0x4000);
        assert_eq!(file[..8], [0, 0, 0, 1, b'B', b'u', b'd', b'1']);
        assert_eq!(file[8..20], [0, 0, 8, 0, 0, 0, 8, 0, 0, 0, 8, 0]);
        let root = &file[4 + 0x800..4 + 0x1000];
        assert_eq!(
            root[..20],
            [
                0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 8, 0x0B, 0, 0, 0, 0x25, 0, 0, 0x20, 0x0D
            ]
        );
        let free = |log2: u32| [6, 7, 8, 9, 10, 12].contains(&log2) || (14..31).contains(&log2);
        let want: Vec<Vec<u32>> = (0..32)
            .map(|log2| {
                if free(log2) {
                    vec![1 << log2]
                } else {
                    Vec::new()
                }
            })
            .collect();
        assert_eq!(free_lists(), want);
    }

    #[test]
    fn records_come_back_in_finders_order() {
        let read = read(&write(&records()).unwrap()).unwrap();
        let order: Vec<(&str, &[u8; 4])> =
            read.iter().map(|r| (r.name.as_str(), &r.code)).collect();
        assert_eq!(
            order,
            [
                (".", b"bwsp"),
                (".", b"icvl"),
                (".", b"vSrn"),
                ("applications", b"Iloc"),
                ("Zebra.app", b"Iloc"),
                ("é", b"dilc"),
            ]
        );
        assert_eq!(read, sorted(records()));
    }

    #[test]
    fn each_type_is_encoded_as_finder_reads_it() {
        let encoded = |value| {
            encode(&Record {
                name: ".".to_owned(),
                code: *b"test",
                value,
            })[10..]
                .to_vec()
        };
        assert_eq!(encoded(Value::Long(1)), b"long\0\0\0\x01");
        assert_eq!(encoded(Value::Bool(true)), b"bool\x01");
        assert_eq!(encoded(Value::Type(*b"icnv")), b"typeicnv");
        assert_eq!(encoded(Value::Blob(vec![9, 9])), b"blob\0\0\0\x02\x09\x09");
        let name = encode(&Record {
            name: "Aé".to_owned(),
            code: *b"Iloc",
            value: Value::Long(0),
        });
        assert_eq!(name[..8], [0, 0, 0, 2, 0, b'A', 0, 0xE9]);
    }

    #[test]
    fn a_window_bigger_than_a_page_or_a_damaged_file_is_refused() {
        let big = vec![Record {
            name: ".".to_owned(),
            code: *b"icvp",
            value: Value::Blob(vec![0; 5000]),
        }];
        assert!(write(&big).is_err());
        let mut file = write(&records()).unwrap();
        assert!(read(&file[..100]).is_err());
        file[4] = b'X';
        assert!(read(&file).is_err());
    }
}
