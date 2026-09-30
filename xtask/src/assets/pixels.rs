//! Checks 3 and 4: the pixels.
//!
//! Every PNG is read as the raw indices it stores, never expanded to colour
//! (`Transformations::IDENTITY`), because the runtime atlas keeps those indices.
//!
//! - Check 3: every rect lies inside its atlas, and the atlas holds exactly the pixels of the
//!   individual PNG.
//! - Check 4, the palette validator: every index is at most 32, and every file's `PLTE` and
//!   `tRNS` are the biome palette: index 0 the only transparent entry, 1..=32 the day colours.
//!   The palette PNG matches the palette JSON.

use std::collections::BTreeMap;
use std::io::Cursor;

use png::{BitDepth, ColorType, Decoder, Transformations};
use terrarium_compose::catalog::{PALETTE_SLOTS, Palette, Rect};

use super::manifest::Biome;
use super::package::Package;
use super::{Check, Failure};

/// Raw PNG indices, row-major: 0 is transparent and 1..=32 is slot + 1.
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

/// Decodes and checks every atlas and sprite; returns each sprite's pixels, by ID, for those
/// that decoded.
pub fn check(package: &Package, biome: &Biome) -> (BTreeMap<String, Indexed>, Vec<Failure>) {
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

    let mut images = BTreeMap::new();
    for sprite in &biome.sprites {
        let Some(image) = load(package, &sprite.file, &biome.palette, &mut failures) else {
            continue;
        };
        let fail =
            |message: String| Failure::new(Check::Pixels, format!("{}: {message}", sprite.id));
        let size = (u32::from(sprite.rect.w), u32::from(sprite.rect.h));
        if (image.width, image.height) != size {
            failures.push(fail(format!(
                "{} is {}×{}, but its rect is {}×{}",
                sprite.file, image.width, image.height, size.0, size.1
            )));
        } else if let Some(atlas) = &atlases[sprite.atlas] {
            match atlas.region(sprite.rect) {
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
        images.insert(sprite.id.clone(), image);
    }
    (images, failures)
}

/// Decodes one indexed PNG and runs the palette validator on it.
fn load(
    package: &Package,
    path: &str,
    palette: &Palette,
    failures: &mut Vec<Failure>,
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
        palette_problems(&decoded, palette)
            .into_iter()
            .map(|problem| Failure::new(Check::Palette, format!("{path}: {problem}"))),
    );
    Some(decoded.image)
}

fn palette_problems(decoded: &Decoded, palette: &Palette) -> Vec<String> {
    let mut problems = Vec::new();
    let max = u8::try_from(PALETTE_SLOTS).expect("32 fits in a byte");
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
    let entries = decoded.plte.len() / 3;
    let day_ok = entries == PALETTE_SLOTS + 1
        && (0..PALETTE_SLOTS)
            .all(|slot| decoded.plte[3 * (slot + 1)..3 * (slot + 2)] == palette.day[slot]);
    if !day_ok {
        problems.push(format!(
            "its PLTE is not index 0 plus the {PALETTE_SLOTS} day colours"
        ));
    }
    let binary_alpha =
        decoded.trns.first() == Some(&0) && decoded.trns[1..].iter().all(|&a| a == 255);
    if !binary_alpha {
        problems.push("its tRNS does not make index 0, and only index 0, transparent".into());
    }
    problems
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
    if usize::try_from(width) != Ok(PALETTE_SLOTS) || height != 2 {
        return failures.push(fail(format!("is {width}×{height}, not {PALETTE_SLOTS}×2")));
    }
    let rows = [
        &palette_rows(&biome.palette.day),
        &palette_rows(&biome.palette.night),
    ];
    for (row, expected) in pixels.chunks(PALETTE_SLOTS * 4).zip(rows) {
        if row != expected.as_slice() {
            failures.push(fail(
                "does not match the palette JSON (row 0 day, row 1 night)".into(),
            ));
            return;
        }
    }
}

fn palette_rows(colours: &[[u8; 3]; PALETTE_SLOTS]) -> Vec<u8> {
    colours
        .iter()
        .flat_map(|&[r, g, b]| [r, g, b, 255])
        .collect()
}

#[cfg(test)]
pub mod testing {
    //! Re-encodes an indexed PNG after an edit, keeping or replacing its palette chunks.

    use super::*;

    pub fn edit(bytes: &[u8], edit: impl FnOnce(&mut Indexed, &mut Vec<u8>)) -> Vec<u8> {
        let mut decoded = decode_indexed(bytes).expect("a delivered PNG decodes");
        edit(&mut decoded.image, &mut decoded.plte);
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
