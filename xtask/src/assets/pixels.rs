//! Checks 3 and 4: the pixels.
//!
//! Every PNG is read as the raw indices it stores, never expanded to colour
//! (`Transformations::IDENTITY`), because the runtime atlas keeps those indices.
//!
//! - Check 3: every rect lies inside its atlas, and the atlas holds exactly the pixels of the
//!   individual PNG. Art with its own colours has no rect. A backdrop covers the whole frame,
//!   or a wider one with the 640-wide scene at its centre, for wider viewports to crop.
//! - Check 4, the palette validator: every shared index is at most 32, and every shared file's
//!   `PLTE` and `tRNS` are the biome palette: index 0 the only transparent entry, 1..=32 the
//!   day colours. Art with its own colours carries its manifest's day list instead, and uses no
//!   index past it. The palette PNG matches the palette JSON, and the slots all art needs fit
//!   an atlas byte.
//!
//! Each image comes back as runtime atlas bytes: shared art keeps its PNG indices, and art with
//! its own colours is mapped onto the slots its day and night pairs get (see [`Slots`]).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use png::{BitDepth, ColorType, Decoder, Transformations};
use terrarium_compose::catalog::{Flags, MAX_SLOTS, Palette, Point, Rect, Rgb, SHARED_SLOTS};
use terrarium_compose::frame::{NATIVE_HEIGHT, NATIVE_WIDTH};

use super::manifest::{Biome, LocalDraft, SpriteDraft};
use super::package::Package;
use super::{Check, Failure};

/// Atlas bytes, row-major: 0 is transparent and every other byte is slot + 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indexed {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Indexed {
    /// The pixels under `rect`, or `None` if it does not fit.
    pub fn region(&self, rect: Rect) -> Option<Vec<u8>> {
        let (x, y, w, h) = (
            u32::from(rect.x),
            u32::from(rect.y),
            u32::from(rect.w),
            u32::from(rect.h),
        );
        if x + w > self.width || y + h > self.height {
            return None;
        }
        let stride = usize::try_from(self.width).ok()?;
        let (x, w) = (usize::try_from(x).ok()?, usize::try_from(w).ok()?);
        let rows = usize::try_from(y).ok()?..usize::try_from(y + h).ok()?;
        Some(
            rows.flat_map(|row| &self.pixels[row * stride + x..row * stride + x + w])
                .copied()
                .collect(),
        )
    }
}

/// An indexed PNG with the palette chunks it carries.
struct Decoded {
    image: Indexed,
    plte: Vec<u8>,
    trns: Vec<u8>,
}

fn decode_indexed(bytes: &[u8]) -> Result<Decoded, String> {
    let mut decoder = Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(|err| err.to_string())?;
    let info = reader.info();
    if info.color_type != ColorType::Indexed || info.bit_depth != BitDepth::Eight || info.interlaced
    {
        return Err(format!(
            "is {:?} {:?}{}, not 8-bit indexed",
            info.bit_depth,
            info.color_type,
            if info.interlaced { " interlaced" } else { "" }
        ));
    }
    let plte = info.palette.as_deref().unwrap_or_default().to_vec();
    let trns = info.trns.as_deref().unwrap_or_default().to_vec();
    let mut pixels = vec![0; reader.output_buffer_size().ok_or("is too large")?];
    let frame = reader
        .next_frame(&mut pixels)
        .map_err(|err| err.to_string())?;
    pixels.truncate(frame.buffer_size());
    Ok(Decoded {
        image: Indexed {
            width: frame.width,
            height: frame.height,
            pixels,
        },
        plte,
        trns,
    })
}

/// The palette PNG: RGBA, day in row 0 and night in row 1.
fn decode_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(|err| err.to_string())?;
    let info = reader.info();
    if info.color_type != ColorType::Rgba || info.bit_depth != BitDepth::Eight {
        return Err(format!(
            "is {:?} {:?}, not 8-bit RGBA",
            info.bit_depth, info.color_type
        ));
    }
    let mut pixels = vec![0; reader.output_buffer_size().ok_or("is too large")?];
    let frame = reader
        .next_frame(&mut pixels)
        .map_err(|err| err.to_string())?;
    pixels.truncate(frame.buffer_size());
    Ok((frame.width, frame.height, pixels))
}

/// The runtime palette as it grows: the shared slots, then one slot for each day and night pair
/// that art with its own colours uses and no slot already holds.
pub struct Slots {
    palette: Palette,
}

impl Slots {
    pub fn new(shared: &Palette) -> Slots {
        Slots {
            palette: shared.clone(),
        }
    }

    /// The slot holding this pair, added if none does yet; `None` once the palette is full.
    pub fn slot(&mut self, day: Rgb, night: Rgb) -> Option<u8> {
        let mut held = self.palette.day.iter().zip(&self.palette.night);
        if let Some(slot) = held.position(|pair| pair == (&day, &night)) {
            return u8::try_from(slot).ok();
        }
        if self.palette.slots() == MAX_SLOTS {
            return None;
        }
        self.palette.day.push(day);
        self.palette.night.push(night);
        u8::try_from(self.palette.slots() - 1).ok()
    }

    pub fn palette(self) -> Palette {
        self.palette
    }
}

/// Decodes and checks every atlas and sprite. Returns each sprite's atlas bytes, by ID, for
/// those that decoded, and the runtime palette. Art with its own colours takes its slots in ID
/// order, and within a sprite in the order of its PNG indices, so the palette is the same on
/// every run.
pub fn check(
    package: &Package,
    biome: &Biome,
) -> (BTreeMap<String, Indexed>, Palette, Vec<Failure>) {
    let mut failures = Vec::new();
    check_palette_png(package, biome, &mut failures);

    let mut atlases = Vec::new();
    for atlas in &biome.atlases {
        let decoded = load(package, &atlas.path, &biome.palette, &mut failures);
        if let Some(image) = &decoded
            && (image.width, image.height) != (atlas.width, atlas.height)
        {
            failures.push(Failure::new(
                Check::Pixels,
                format!(
                    "{} is {}×{}, but its manifest says {}×{}",
                    atlas.path, image.width, image.height, atlas.width, atlas.height
                ),
            ));
        }
        atlases.push(decoded);
    }

    let mut slots = Slots::new(&biome.palette);
    let mut images = BTreeMap::new();
    for sprite in &biome.sprites {
        let fail =
            |message: String| Failure::new(Check::Pixels, format!("{}: {message}", sprite.id));
        let image = match &sprite.local {
            None => load(package, &sprite.file, &biome.palette, &mut failures),
            Some(local) => load_local(package, sprite, local, &mut failures),
        };
        let Some(image) = image else {
            continue;
        };
        let size = (u32::from(sprite.size.w), u32::from(sprite.size.h));
        if (image.width, image.height) != size {
            failures.push(fail(format!(
                "{} is {}×{}, but its size is {}×{}",
                sprite.file, image.width, image.height, size.0, size.1
            )));
            continue;
        }
        if sprite.flags.contains(Flags::BACKDROP) {
            // The scene stands in the middle, so a viewport crops it the same on either side.
            let scene = sprite.scene.unwrap_or(Rect {
                x: 0,
                y: 0,
                w: sprite.size.w,
                h: sprite.size.h,
            });
            let extra = size.0.saturating_sub(NATIVE_WIDTH);
            let covers = size.1 == NATIVE_HEIGHT
                && u32::from(scene.w) == NATIVE_WIDTH
                && extra.is_multiple_of(2)
                && u32::from(scene.x) == extra / 2
                && sprite.at == Some(Point::default())
                && !image.pixels.contains(&0);
            if !covers {
                failures.push(fail(format!(
                    "a backdrop covers the {NATIVE_WIDTH}×{NATIVE_HEIGHT} frame from (0, 0), or a wider one centred on it, with no transparent pixel"
                )));
            }
        }
        if let (Some(rect), Some(atlas)) = (sprite.rect, &atlases[sprite.atlas]) {
            match atlas.region(rect) {
                None => failures.push(fail(format!(
                    "its rect lies outside {}",
                    biome.atlases[sprite.atlas].path
                ))),
                Some(region) if region != image.pixels => failures.push(fail(format!(
                    "{} differs from its rect in {}",
                    sprite.file, biome.atlases[sprite.atlas].path
                ))),
                Some(_) => {}
            }
        }
        let image = match &sprite.local {
            None => Some(image),
            Some(local) => own_slots(&image, local, &mut slots),
        };
        let Some(image) = image else {
            failures.push(Failure::new(
                Check::Palette,
                format!(
                    "{}: its colours take the palette past {MAX_SLOTS} slots",
                    sprite.id
                ),
            ));
            continue;
        };
        images.insert(sprite.id.clone(), image);
    }
    (images, slots.palette(), failures)
}

/// Art with its own colours: its PNG, checked against its day list. Returns its PNG indices, or
/// `None` after any problem, since an index past its lists has no colours to map.
fn load_local(
    package: &Package,
    sprite: &SpriteDraft,
    local: &LocalDraft,
    failures: &mut Vec<Failure>,
) -> Option<Indexed> {
    let before = failures.len();
    let image = load_with(package, &sprite.file, failures, |decoded| {
        own_problems(decoded, &local.day)
    })?;
    (failures.len() == before).then_some(image)
}

/// Maps PNG indices onto runtime slots, one per day and night pair, taken in index order; `None`
/// once the palette is full.
fn own_slots(image: &Indexed, local: &LocalDraft, slots: &mut Slots) -> Option<Indexed> {
    let used: BTreeSet<u8> = image.pixels.iter().copied().filter(|&i| i != 0).collect();
    let mut bytes = [0_u8; 256];
    for index in used {
        let colour = usize::from(index) - 1;
        bytes[usize::from(index)] = slots.slot(local.day[colour], local.night[colour])? + 1;
    }
    Some(Indexed {
        width: image.width,
        height: image.height,
        pixels: image
            .pixels
            .iter()
            .map(|&i| bytes[usize::from(i)])
            .collect(),
    })
}

/// Decodes one indexed PNG and runs the palette validator on it.
fn load(
    package: &Package,
    path: &str,
    palette: &Palette,
    failures: &mut Vec<Failure>,
) -> Option<Indexed> {
    load_with(package, path, failures, |decoded| {
        palette_problems(decoded, palette)
    })
}

/// Decodes one indexed PNG and reports what `problems` finds in it.
fn load_with(
    package: &Package,
    path: &str,
    failures: &mut Vec<Failure>,
    problems: impl FnOnce(&Decoded) -> Vec<String>,
) -> Option<Indexed> {
    let Some(bytes) = package.file(path) else {
        failures.push(Failure::new(Check::Pixels, format!("{path} is missing")));
        return None;
    };
    let decoded = match decode_indexed(bytes) {
        Ok(decoded) => decoded,
        Err(err) => {
            failures.push(Failure::new(Check::Pixels, format!("{path} {err}")));
            return None;
        }
    };
    failures.extend(
        problems(&decoded)
            .into_iter()
            .map(|problem| Failure::new(Check::Palette, format!("{path}: {problem}"))),
    );
    Some(decoded.image)
}

/// Shared art against the biome palette.
fn palette_problems(decoded: &Decoded, palette: &Palette) -> Vec<String> {
    let colours = palette.day.get(..SHARED_SLOTS).unwrap_or_default();
    let mut problems = index_problems(decoded, SHARED_SLOTS);
    if !plte_holds(decoded, colours) || decoded.plte.len() / 3 != SHARED_SLOTS + 1 {
        problems.push(format!(
            "its PLTE is not index 0 plus the {SHARED_SLOTS} day colours"
        ));
    }
    problems.extend(alpha_problem(decoded));
    problems
}

/// Art with its own colours against its manifest's list for this file.
fn own_problems(decoded: &Decoded, colours: &[Rgb]) -> Vec<String> {
    let mut problems = index_problems(decoded, colours.len());
    if !plte_holds(decoded, colours) {
        problems.push(format!(
            "its PLTE is not index 0 plus the {} colours its manifest lists",
            colours.len()
        ));
    }
    problems.extend(alpha_problem(decoded));
    problems
}

/// Whether PLTE entries 1.. are `colours`, in order.
fn plte_holds(decoded: &Decoded, colours: &[Rgb]) -> bool {
    decoded.plte.len() / 3 > colours.len()
        && colours
            .iter()
            .enumerate()
            .all(|(i, colour)| decoded.plte[3 * (i + 1)..3 * (i + 2)] == *colour)
}

fn index_problems(decoded: &Decoded, colours: usize) -> Vec<String> {
    let mut problems = Vec::new();
    let max = u8::try_from(colours).unwrap_or(u8::MAX);
    let over: Vec<usize> = decoded
        .image
        .pixels
        .iter()
        .enumerate()
        .filter(|(_, i)| **i > max)
        .map(|(n, _)| n)
        .collect();
    if let Some(&first) = over.first() {
        let width = usize::try_from(decoded.image.width).unwrap_or(1).max(1);
        problems.push(format!(
            "{} pixels use an index above {max}, the first {} at ({}, {})",
            over.len(),
            decoded.image.pixels[first],
            first % width,
            first / width
        ));
    }
    problems
}

fn alpha_problem(decoded: &Decoded) -> Option<String> {
    let binary_alpha =
        decoded.trns.first() == Some(&0) && decoded.trns[1..].iter().all(|&a| a == 255);
    (!binary_alpha).then(|| "its tRNS does not make index 0, and only index 0, transparent".into())
}

fn check_palette_png(package: &Package, biome: &Biome, failures: &mut Vec<Failure>) {
    let path = &biome.palette_png;
    let fail = |message: String| Failure::new(Check::Palette, format!("{path} {message}"));
    let Some(bytes) = package.file(path) else {
        failures.push(fail("is missing".into()));
        return;
    };
    let (width, height, pixels) = match decode_rgba(bytes) {
        Ok(image) => image,
        Err(err) => return failures.push(fail(err)),
    };
    if usize::try_from(width) != Ok(SHARED_SLOTS) || height != 2 {
        return failures.push(fail(format!("is {width}×{height}, not {SHARED_SLOTS}×2")));
    }
    let rows = [
        &palette_rows(&biome.palette.day),
        &palette_rows(&biome.palette.night),
    ];
    for (row, expected) in pixels.chunks(SHARED_SLOTS * 4).zip(rows) {
        if row != expected.as_slice() {
            failures.push(fail(
                "does not match the palette JSON (row 0 day, row 1 night)".into(),
            ));
            return;
        }
    }
}

fn palette_rows(colours: &[Rgb]) -> Vec<u8> {
    colours
        .iter()
        .flat_map(|&[r, g, b]| [r, g, b, 255])
        .collect()
}

#[cfg(test)]
pub mod testing {
    //! Re-encodes an indexed PNG after an edit to its pixels or palette chunks.

    use super::*;

    /// `edit` gets the pixels, the `PLTE` bytes and the `tRNS` alphas.
    pub fn edit(
        bytes: &[u8],
        edit: impl FnOnce(&mut Indexed, &mut Vec<u8>, &mut Vec<u8>),
    ) -> Vec<u8> {
        let mut decoded = decode_indexed(bytes).expect("a delivered PNG decodes");
        edit(&mut decoded.image, &mut decoded.plte, &mut decoded.trns);
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, decoded.image.width, decoded.image.height);
        encoder.set_color(ColorType::Indexed);
        encoder.set_depth(BitDepth::Eight);
        encoder.set_palette(decoded.plte);
        encoder.set_trns(decoded.trns);
        let mut writer = encoder.write_header().expect("the header encodes");
        writer
            .write_image_data(&decoded.image.pixels)
            .expect("the pixels encode");
        writer.finish().expect("the PNG finishes");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_takes_the_slot_that_holds_it_and_the_palette_stops_at_255() {
        let shared = Palette {
            day: vec![[1, 1, 1], [2, 2, 2]],
            night: vec![[9, 9, 9], [8, 8, 8]],
        };
        let mut slots = Slots::new(&shared);
        assert_eq!(slots.slot([2, 2, 2], [8, 8, 8]), Some(1), "a shared pair");
        assert_eq!(
            slots.slot([2, 2, 2], [7, 7, 7]),
            Some(2),
            "a new night is a new pair"
        );
        assert_eq!(slots.slot([2, 2, 2], [7, 7, 7]), Some(2), "each pair once");
        for n in 3..MAX_SLOTS {
            let n = u8::try_from(n).unwrap();
            assert_eq!(slots.slot([n, 0, 0], [0, 0, 0]), Some(n));
        }
        assert_eq!(slots.slot([0, 0, 1], [0, 0, 0]), None, "the 256th slot");
        assert_eq!(
            slots.slot([2, 2, 2], [7, 7, 7]),
            Some(2),
            "a full palette still finds"
        );
        assert_eq!(slots.palette().slots(), MAX_SLOTS);
    }

    #[test]
    fn a_region_is_cut_row_by_row_and_refused_past_the_edge() {
        let image = Indexed {
            width: 3,
            height: 2,
            pixels: vec![1, 2, 3, 4, 5, 6],
        };
        assert_eq!(
            image.region(Rect {
                x: 1,
                y: 0,
                w: 2,
                h: 2
            }),
            Some(vec![2, 3, 5, 6])
        );
        assert_eq!(
            image.region(Rect {
                x: 2,
                y: 0,
                w: 2,
                h: 1
            }),
            None
        );
        assert_eq!(
            image.region(Rect {
                x: 0,
                y: 1,
                w: 1,
                h: 2
            }),
            None
        );
    }
}
