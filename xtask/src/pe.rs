//! Reading a built Windows exe, which no test here can run: what it imports, and the resources
//! the linker put in it. A PE32+ image only; anything else reads as `None`.

/// The parts of a PE32+ image these checks read: its bytes, and where its sections map.
struct Image<'a> {
    bytes: &'a [u8],
    /// Each section's virtual address, virtual size and raw offset.
    sections: Vec<(usize, usize, usize)>,
    /// The data directories' virtual addresses and sizes, in order.
    directories: Vec<(usize, usize)>,
}

/// The import directory's place among the data directories, and the resource directory's.
const IMPORTS: usize = 1;
const RESOURCES: usize = 2;

impl<'a> Image<'a> {
    fn read(bytes: &'a [u8]) -> Option<Image<'a>> {
        let image = Image {
            bytes,
            sections: Vec::new(),
            directories: Vec::new(),
        };
        let pe = image.usize_at(0x3c)?;
        if bytes.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let count = usize::from(image.u16_at(pe + 6)?);
        let optional = pe + 24;
        if image.u16_at(optional)? != 0x20b {
            return None;
        }
        let directory_count = image.usize_at(optional + 108)?.min(16);
        let directories = (0..directory_count)
            .map(|index| {
                let at = optional + 112 + index * 8;
                Some((image.usize_at(at)?, image.usize_at(at + 4)?))
            })
            .collect::<Option<Vec<_>>>()?;
        let table = optional + usize::from(image.u16_at(pe + 20)?);
        let sections = (0..count)
            .map(|index| {
                let header = table + index * 40;
                let size = image
                    .usize_at(header + 8)?
                    .max(image.usize_at(header + 16)?);
                Some((
                    image.usize_at(header + 12)?,
                    size,
                    image.usize_at(header + 20)?,
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Image {
            sections,
            directories,
            ..image
        })
    }

    fn u16_at(&self, at: usize) -> Option<u16> {
        Some(u16::from_le_bytes(
            self.bytes.get(at..at + 2)?.try_into().ok()?,
        ))
    }

    fn u32_at(&self, at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(
            self.bytes.get(at..at + 4)?.try_into().ok()?,
        ))
    }

    fn u64_at(&self, at: usize) -> Option<u64> {
        Some(u64::from_le_bytes(
            self.bytes.get(at..at + 8)?.try_into().ok()?,
        ))
    }

    fn usize_at(&self, at: usize) -> Option<usize> {
        self.u32_at(at).and_then(|v| usize::try_from(v).ok())
    }

    fn text_at(&self, at: usize) -> Option<&'a str> {
        let rest = self.bytes.get(at..)?;
        let end = rest.iter().position(|&b| b == 0)?;
        std::str::from_utf8(&rest[..end]).ok()
    }

    /// The file offset of a virtual address.
    fn offset(&self, rva: usize) -> Option<usize> {
        self.sections.iter().find_map(|&(start, size, raw)| {
            (start..start + size)
                .contains(&rva)
                .then(|| rva - start + raw)
        })
    }

    fn directory(&self, index: usize) -> Option<(usize, usize)> {
        self.directories
            .get(index)
            .copied()
            .filter(|&(rva, _)| rva != 0)
    }
}

/// The functions a PE32+ image imports by name from `dll` (ordinal imports are left out), or
/// `None` if the image cannot be read as one.
pub fn named_imports(bytes: &[u8], dll: &str) -> Option<Vec<String>> {
    let image = Image::read(bytes)?;
    let Some((imports, _)) = image.directory(IMPORTS) else {
        return Some(Vec::new());
    };
    let mut descriptor = image.offset(imports)?;
    loop {
        let name = image.usize_at(descriptor + 12)?;
        if name == 0 {
            return Some(Vec::new());
        }
        if image
            .text_at(image.offset(name)?)?
            .eq_ignore_ascii_case(dll)
        {
            let lookup = match image.usize_at(descriptor)? {
                0 => image.usize_at(descriptor + 16)?,
                table => table,
            };
            let mut entry = image.offset(lookup)?;
            let mut named = Vec::new();
            loop {
                let value = image.u64_at(entry)?;
                if value == 0 {
                    return Some(named);
                }
                // The top bit marks an import by ordinal; otherwise the low 31 bits point to
                // a two-byte hint followed by the name.
                if value >> 63 == 0 {
                    let hint = usize::try_from(value & 0x7fff_ffff).ok()?;
                    named.push(image.text_at(image.offset(hint)? + 2)?.to_owned());
                }
                entry += 8;
            }
        }
        descriptor += 20;
    }
}

/// The DLLs a PE32+ image imports from, by the names it gives them.
pub fn imported_dlls(bytes: &[u8]) -> Option<Vec<String>> {
    let image = Image::read(bytes)?;
    let Some((imports, _)) = image.directory(IMPORTS) else {
        return Some(Vec::new());
    };
    let mut descriptor = image.offset(imports)?;
    let mut dlls = Vec::new();
    loop {
        let name = image.usize_at(descriptor + 12)?;
        if name == 0 {
            return Some(dlls);
        }
        dlls.push(image.text_at(image.offset(name)?)?.to_owned());
        descriptor += 20;
    }
}

/// One resource the image holds: its type and its name, each an id, and its bytes.
#[derive(Debug, PartialEq, Eq)]
pub struct Resource {
    pub kind: u16,
    pub id: u16,
    pub data: Vec<u8>,
}

/// The resources with numeric types and ids a PE32+ image holds, every language of each, in
/// the directory's order. Named types and names are left out: the app's resources have none.
pub fn resources(bytes: &[u8]) -> Option<Vec<Resource>> {
    let image = Image::read(bytes)?;
    let Some((rva, _)) = image.directory(RESOURCES) else {
        return Some(Vec::new());
    };
    let base = image.offset(rva)?;
    // A directory's entries: each an id (or a name, with the top bit set) and an offset from
    // the resource section's start, to a subdirectory when its top bit is set.
    let entries = |at: usize| -> Option<Vec<(u32, u32)>> {
        let named = usize::from(image.u16_at(at + 12)?);
        let ids = usize::from(image.u16_at(at + 14)?);
        (0..named + ids)
            .map(|index| {
                let entry = at + 16 + index * 8;
                Some((image.u32_at(entry)?, image.u32_at(entry + 4)?))
            })
            .collect()
    };
    let subdirectory = |offset: u32| -> Option<usize> {
        let within = usize::try_from(offset & 0x7fff_ffff).ok()?;
        (offset >> 31 == 1).then_some(base + within)
    };
    let id = |name: u32| -> Option<u16> {
        if name >> 31 == 1 {
            None
        } else {
            u16::try_from(name).ok()
        }
    };
    let mut found = Vec::new();
    for (kind, kinds) in entries(base)? {
        let (Some(kind), Some(names)) = (id(kind), subdirectory(kinds)) else {
            continue;
        };
        for (name, languages) in entries(names)? {
            let (Some(name), Some(languages)) = (id(name), subdirectory(languages)) else {
                continue;
            };
            for (_, leaf) in entries(languages)? {
                let leaf = base + usize::try_from(leaf & 0x7fff_ffff).ok()?;
                let start = image.offset(image.usize_at(leaf)?)?;
                let size = image.usize_at(leaf + 4)?;
                found.push(Resource {
                    kind,
                    id: name,
                    data: image.bytes.get(start..start + size)?.to_vec(),
                });
            }
        }
    }
    Some(found)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn put32(buf: &mut [u8], at: usize, value: usize) {
        buf[at..at + 4].copy_from_slice(&u32::try_from(value).unwrap().to_le_bytes());
    }

    /// A minimal PE32+ image: one section at RVA 0x1000 (file offset 0x200) holding `data`,
    /// with the import directory at `imports` and the resource directory at `resources`, each
    /// an offset into `data`.
    pub fn image_with(data: &[u8], imports: Option<usize>, resources: Option<usize>) -> Vec<u8> {
        const RVA: usize = 0x1000;
        const RAW: usize = 0x200;
        let mut image = vec![0_u8; RAW];
        image[0x3c] = 0x40;
        image[0x40..0x44].copy_from_slice(b"PE\0\0");
        image[0x46] = 1; // one section
        image[0x54] = 0xf0; // the optional header's size
        image[0x58..0x5a].copy_from_slice(&0x20b_u16.to_le_bytes());
        put32(&mut image, 0x58 + 108, 16);
        if let Some(at) = imports {
            put32(&mut image, 0x58 + 112 + 8, RVA + at);
        }
        if let Some(at) = resources {
            put32(&mut image, 0x58 + 112 + 16, RVA + at);
        }
        let section = 0x58 + 0xf0;
        put32(&mut image, section + 8, data.len());
        put32(&mut image, section + 12, RVA);
        put32(&mut image, section + 16, data.len());
        put32(&mut image, section + 20, RAW);
        image.extend(data);
        image
    }

    /// An image importing, from each `(dll, entries)`, an entry by name or by ordinal.
    pub fn image(dlls: &[(&str, &[Result<&str, u16>])]) -> Vec<u8> {
        const RVA: usize = 0x1000;
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
        image_with(&data, Some(0), None)
    }

    /// An image whose resource directory holds `items`, each `(type, id, bytes)`, in English.
    pub fn image_with_resources(items: &[(u16, u16, &[u8])]) -> Vec<u8> {
        const RVA: usize = 0x1000;
        fn directory(entries: &[(u32, u32)]) -> Vec<u8> {
            let mut out = vec![0_u8; 16];
            out[14..16].copy_from_slice(&u16::try_from(entries.len()).unwrap().to_le_bytes());
            for (name, offset) in entries {
                out.extend(name.to_le_bytes());
                out.extend(offset.to_le_bytes());
            }
            out
        }
        // Laid out as a linker does: the type directory, then each type's names, then each
        // name's languages, then the leaves, then the data.
        let mut kinds: Vec<u16> = items.iter().map(|item| item.0).collect();
        kinds.dedup();
        let level = |entries: usize| 16 + entries * 8;
        let mut data = Vec::new();
        let type_size = level(kinds.len());
        let names_size: usize = kinds
            .iter()
            .map(|kind| level(items.iter().filter(|item| item.0 == *kind).count()))
            .sum();
        let languages_at = type_size + names_size;
        let leaves_at = languages_at + items.len() * level(1);
        let payload_at = leaves_at + items.len() * 16;
        let flag = |offset: usize| u32::try_from(offset).unwrap() | 1 << 31;
        let mut name_at = type_size;
        let mut type_entries = Vec::new();
        let mut name_dirs = Vec::new();
        let mut index = 0;
        for kind in &kinds {
            type_entries.push((u32::from(*kind), flag(name_at)));
            let mine: Vec<_> = items.iter().filter(|item| item.0 == *kind).collect();
            let entries: Vec<(u32, u32)> = mine
                .iter()
                .enumerate()
                .map(|(n, item)| {
                    (
                        u32::from(item.1),
                        flag(languages_at + (index + n) * level(1)),
                    )
                })
                .collect();
            index += mine.len();
            name_at += level(entries.len());
            name_dirs.push(directory(&entries));
        }
        data.extend(directory(&type_entries));
        for dir in name_dirs {
            data.extend(dir);
        }
        for n in 0..items.len() {
            data.extend(directory(&[(
                0x409,
                u32::try_from(leaves_at + n * 16).unwrap(),
            )]));
        }
        let mut payload: Vec<u8> = Vec::new();
        for (_, _, bytes) in items {
            let at = payload_at + payload.len();
            let mut leaf = vec![0_u8; 16];
            put32(&mut leaf, 0, RVA + at);
            put32(&mut leaf, 4, bytes.len());
            data.extend(leaf);
            payload.extend(*bytes);
        }
        data.extend(payload);
        image_with(&data, None, Some(0))
    }

    #[test]
    fn ordinal_imports_from_comctl32_pass_and_named_ones_are_listed() {
        let ordinals = image(&[
            ("KERNEL32.dll", &[Ok("ExitProcess")]),
            ("comctl32.dll", &[Err(410), Err(412), Err(413)]),
        ]);
        assert_eq!(named_imports(&ordinals, "comctl32.dll"), Some(Vec::new()));
        assert_eq!(
            imported_dlls(&ordinals),
            Some(vec!["KERNEL32.dll".to_owned(), "comctl32.dll".to_owned()])
        );
        let named = image(&[("COMCTL32.dll", &[Err(410), Ok("SetWindowSubclass")])]);
        assert_eq!(
            named_imports(&named, "comctl32.dll"),
            Some(vec!["SetWindowSubclass".to_owned()])
        );
        assert_eq!(named_imports(b"MZ", "comctl32.dll"), None);
    }

    #[test]
    fn the_resources_are_read_with_their_types_ids_and_bytes() {
        let exe = image_with_resources(&[
            (3, 1, b"icon one"),
            (3, 2, b"icon two"),
            (14, 1, b"group"),
            (24, 1, b"<assembly/>"),
        ]);
        assert_eq!(
            resources(&exe),
            Some(vec![
                Resource {
                    kind: 3,
                    id: 1,
                    data: b"icon one".to_vec()
                },
                Resource {
                    kind: 3,
                    id: 2,
                    data: b"icon two".to_vec()
                },
                Resource {
                    kind: 14,
                    id: 1,
                    data: b"group".to_vec()
                },
                Resource {
                    kind: 24,
                    id: 1,
                    data: b"<assembly/>".to_vec()
                },
            ])
        );
        assert_eq!(resources(&image(&[])), Some(Vec::new()));
    }
}
