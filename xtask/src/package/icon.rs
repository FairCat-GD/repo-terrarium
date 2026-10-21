//! The app's icon: the PNGs in `packaging/icon/`, checked against the shape both
//! platforms show, and made into the Mac's `.icns` and the Windows icon's images by whole-number
//! scaling only (I-11). `packaging/icon/README.md` says what to deliver.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;

use png::{BitDepth, ColorType, Decoder, Transformations};

/// Where the icon's sources are, from the workspace root.
pub const DIR: &str = "packaging/icon";

/// One source image: its side and its RGBA pixels, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub side: u32,
    pub rgba: Vec<u8>,
}

/// The icon's sources: one drawn at each size; the 48 is optional.
#[derive(Debug)]
pub struct Icon {
    pub s16: Image,
    /// Windows' small icon at 150% scaling.
    pub s24: Option<Image>,
    pub s32: Image,
    pub s48: Option<Image>,
    pub s64: Image,
}

/// The rounded square every source fills: its margin and its corners' radius, by side.
fn shape(side: u32) -> Option<(f64, f64)> {
    match side {
        16 => Some((1.0, 3.0)),
        24 => Some((2.0, 4.0)),
        32 => Some((3.0, 6.0)),
        48 => Some((5.0, 8.0)),
        64 => Some((7.0, 11.0)),
        _ => None,
    }
}

/// Reads and checks the sources in `root`'s `packaging/icon/`.
pub fn load(root: &Path) -> Result<Icon, String> {
    let dir = root.join(DIR);
    let read = |side: u32| -> Result<Option<Image>, String> {
        let path = dir.join(format!("icon-{side}.png"));
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        let image = decode(&bytes).map_err(|err| format!("{}: {err}", path.display()))?;
        check(&image, side).map_err(|err| format!("{}: {err}", path.display()))?;
        Ok(Some(image))
    };
    let required = |side: u32| -> Result<Image, String> {
        read(side)?.ok_or_else(|| format!("{}/icon-{side}.png is missing", dir.display()))
    };
    Ok(Icon {
        s16: required(16)?,
        s24: read(24)?,
        s32: required(32)?,
        s48: read(48)?,
        s64: required(64)?,
    })
}

/// A square PNG as an icon's source.
fn decode(bytes: &[u8]) -> Result<Image, String> {
    let (width, height, rgba) = decode_rgba(bytes)?;
    if width != height {
        return Err(format!("is {width}×{height}, not square"));
    }
    Ok(Image { side: width, rgba })
}

/// A PNG of any colour type as its width, height and RGBA pixels: palettes and transparency
/// are expanded, and 16-bit samples kept to their high byte, as an editor's 16-bit export of
/// 8-bit art holds them.
pub fn decode_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(
        Transformations::EXPAND | Transformations::ALPHA | Transformations::STRIP_16,
    );
    let mut reader = decoder.read_info().map_err(|err| err.to_string())?;
    let mut pixels = vec![0; reader.output_buffer_size().ok_or("is too large")?];
    let frame = reader
        .next_frame(&mut pixels)
        .map_err(|err| err.to_string())?;
    let pixels = &pixels[..frame.buffer_size()];
    let rgba = match (frame.color_type, frame.bit_depth) {
        (ColorType::Rgba, BitDepth::Eight) => pixels.to_vec(),
        (ColorType::GrayscaleAlpha, BitDepth::Eight) => pixels
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[grey, alpha]| [grey, grey, grey, alpha])
            .collect(),
        (color, depth) => return Err(format!("is {depth:?} {color:?}, not 8-bit colour")),
    };
    Ok((frame.width, frame.height, rgba))
}

/// Whether `image` is `side` square and fills the rounded square: every pixel whose centre is
/// outside it transparent, every one more than a pixel inside it opaque. The pixels between
/// are the outline's play, either way.
pub fn check(image: &Image, side: u32) -> Result<(), String> {
    let (margin, radius) = shape(side).ok_or_else(|| format!("{side} px is not an icon size"))?;
    if image.side != side {
        return Err(format!("is {} px square, not {side}", image.side));
    }
    let side_f = f64::from(side);
    let half = (side_f - 2.0 * margin) / 2.0;
    let centre = side_f / 2.0;
    for y in 0..side {
        for x in 0..side {
            let alpha = image.rgba[usize::try_from((y * side + x) * 4 + 3).unwrap_or(0)];
            let distance = rounded_square_distance(
                f64::from(x) + 0.5 - centre,
                f64::from(y) + 0.5 - centre,
                half,
                radius,
            );
            if distance > 0.0 && alpha != 0 {
                return Err(format!(
                    "the pixel at {x},{y} is outside the rounded square but not transparent"
                ));
            }
            if distance < -1.0 && alpha != 255 {
                return Err(format!(
                    "the pixel at {x},{y} is inside the rounded square but not opaque"
                ));
            }
        }
    }
    Ok(())
}

/// The signed distance from a point to a rounded square centred on the origin, `half` its
/// half-side and `radius` its corners': negative inside.
fn rounded_square_distance(x: f64, y: f64, half: f64, radius: f64) -> f64 {
    let qx = x.abs() - (half - radius);
    let qy = y.abs() - (half - radius);
    let outside = qx.max(0.0).hypot(qy.max(0.0));
    let inside = qx.max(qy).min(0.0);
    outside + inside - radius
}

/// `image` `factor` times its size, each pixel a square of `factor` (nearest sampling).
pub fn scale(image: &Image, factor: u32) -> Image {
    let side = image.side * factor;
    let mut rgba = Vec::with_capacity(usize::try_from(side * side * 4).unwrap_or(0));
    for y in 0..side {
        for x in 0..side {
            let at = usize::try_from(((y / factor) * image.side + x / factor) * 4).unwrap_or(0);
            rgba.extend_from_slice(&image.rgba[at..at + 4]);
        }
    }
    Image { side, rgba }
}

/// `image` as an 8-bit RGBA PNG, the same bytes for the same pixels.
pub fn png(image: &Image) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.side, image.side);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    encoder.set_filter(png::Filter::Adaptive);
    let mut writer = encoder
        .write_header()
        .expect("a header of a valid size encodes into memory");
    writer
        .write_image_data(&image.rgba)
        .expect("pixels of the declared size encode into memory");
    writer
        .finish()
        .expect("a complete image finishes into memory");
    out
}

/// Every image a platform's icon holds, by its side in pixels, as PNG.
pub type Sizes = BTreeMap<u32, Vec<u8>>;

/// The Mac's sizes, each from the source drawn nearest it: 16 and 32 as drawn, 64 the 64 as
/// drawn, and 128 to 1024 the 64 at ×2 to ×16.
pub fn mac_sizes(icon: &Icon) -> Sizes {
    let mut sizes = Sizes::new();
    sizes.insert(16, png(&icon.s16));
    sizes.insert(32, png(&icon.s32));
    sizes.insert(64, png(&icon.s64));
    for factor in [2, 4, 8, 16] {
        sizes.insert(64 * factor, png(&scale(&icon.s64, factor)));
    }
    sizes
}

/// Windows' sizes: 16, 32 and 64 as drawn, 24 when it is drawn, 48 drawn or the 16 at ×3, and
/// 256 the 64 at ×4.
pub fn windows_sizes(icon: &Icon) -> Sizes {
    let mut sizes = Sizes::new();
    sizes.insert(16, png(&icon.s16));
    if let Some(s24) = &icon.s24 {
        sizes.insert(24, png(s24));
    }
    sizes.insert(32, png(&icon.s32));
    let s48 = icon.s48.clone().unwrap_or_else(|| scale(&icon.s16, 3));
    sizes.insert(48, png(&s48));
    sizes.insert(64, png(&icon.s64));
    sizes.insert(256, png(&scale(&icon.s64, 4)));
    sizes
}

/// The Mac's `.icns`: each entry's type names its size, @2x types the doubled ones, each holding
/// a PNG.
pub fn icns(sizes: &Sizes) -> Vec<u8> {
    const ENTRIES: [(&[u8; 4], u32); 10] = [
        (b"icp4", 16),
        (b"ic11", 32),
        (b"icp5", 32),
        (b"ic12", 64),
        (b"ic07", 128),
        (b"ic13", 256),
        (b"ic08", 256),
        (b"ic14", 512),
        (b"ic09", 512),
        (b"ic10", 1024),
    ];
    let mut body = Vec::new();
    for (kind, side) in ENTRIES {
        let Some(png) = sizes.get(&side) else {
            continue;
        };
        body.extend_from_slice(kind);
        body.extend(be32(png.len() + 8));
        body.extend(png);
    }
    let mut out = b"icns".to_vec();
    out.extend(be32(body.len() + 8));
    out.extend(body);
    out
}

/// A Windows `.ico` file holding `sizes`, each as PNG, which Windows reads at every size.
pub fn ico(sizes: &Sizes) -> Vec<u8> {
    let count = sizes.len();
    let mut out = Vec::new();
    out.extend(0_u16.to_le_bytes());
    out.extend(1_u16.to_le_bytes());
    out.extend(le16(count));
    let mut offset = 6 + 16 * count;
    for (side, png) in sizes {
        out.extend(directory_entry(*side, png.len()));
        out.extend(le32(offset));
        offset += png.len();
    }
    for png in sizes.values() {
        out.extend(png);
    }
    out
}

/// The 12 bytes an icon directory's entry starts with, in a `.ico` file and in a group icon
/// alike: the side (0 for 256), no palette, one plane, 32 bits, and the image's length.
pub fn directory_entry(side: u32, length: usize) -> Vec<u8> {
    let side = u8::try_from(side).unwrap_or(0);
    let mut out = vec![side, side, 0, 0];
    out.extend(1_u16.to_le_bytes());
    out.extend(32_u16.to_le_bytes());
    out.extend(le32(length));
    out
}

fn be32(n: usize) -> [u8; 4] {
    u32::try_from(n).unwrap_or(u32::MAX).to_be_bytes()
}

fn le32(n: usize) -> [u8; 4] {
    u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes()
}

fn le16(n: usize) -> [u8; 2] {
    u16::try_from(n).unwrap_or(u16::MAX).to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source that fills its rounded square with one colour.
    pub fn filled(side: u32) -> Image {
        let (margin, radius) = shape(side).unwrap();
        let side_f = f64::from(side);
        let half = (side_f - 2.0 * margin) / 2.0;
        let mut rgba = Vec::new();
        for y in 0..side {
            for x in 0..side {
                let d = rounded_square_distance(
                    f64::from(x) + 0.5 - side_f / 2.0,
                    f64::from(y) + 0.5 - side_f / 2.0,
                    half,
                    radius,
                );
                rgba.extend(if d < -0.5 { [40, 90, 50, 255] } else { [0; 4] });
            }
        }
        Image { side, rgba }
    }

    fn set_alpha(image: &mut Image, x: u32, y: u32, alpha: u8) {
        let at = usize::try_from((y * image.side + x) * 4 + 3).unwrap();
        image.rgba[at] = alpha;
    }

    #[test]
    fn a_source_must_fill_its_rounded_square_and_nothing_outside() {
        for side in [16, 32, 48, 64] {
            assert_eq!(check(&filled(side), side), Ok(()), "{side}");
        }
        assert!(check(&filled(32), 64).is_err());
        let mut corner = filled(64);
        // The corner outside the curve.
        set_alpha(&mut corner, 8, 8, 255);
        assert!(check(&corner, 64).unwrap_err().contains("outside"));
        let mut margin = filled(64);
        set_alpha(&mut margin, 6, 32, 255);
        assert!(check(&margin, 64).unwrap_err().contains("outside"));
        let mut hole = filled(64);
        set_alpha(&mut hole, 32, 32, 128);
        assert!(check(&hole, 64).unwrap_err().contains("not opaque"));
        // The outline's play: the square's first column may stay transparent.
        let mut edge = filled(64);
        set_alpha(&mut edge, 7, 32, 0);
        assert_eq!(check(&edge, 64), Ok(()));
    }

    #[test]
    fn scaling_repeats_each_pixel_as_a_square() {
        let image = Image {
            side: 2,
            rgba: [
                [1, 2, 3, 4],
                [5, 6, 7, 8],
                [9, 10, 11, 12],
                [13, 14, 15, 16],
            ]
            .concat(),
        };
        let big = scale(&image, 2);
        assert_eq!(big.side, 4);
        let pixel = |x: usize, y: usize| big.rgba[(y * 4 + x) * 4];
        assert_eq!(
            [pixel(0, 0), pixel(1, 1), pixel(2, 0), pixel(3, 1)],
            [1, 1, 5, 5]
        );
        assert_eq!([pixel(0, 2), pixel(3, 3)], [9, 13]);
    }

    #[test]
    fn a_png_reads_back_as_its_pixels() {
        let image = filled(16);
        assert_eq!(decode(&png(&image)), Ok(image));
    }

    #[test]
    fn a_16_bit_png_reads_as_its_8_bit_pixels() {
        let image = filled(16);
        let wide: Vec<u8> = image.rgba.iter().flat_map(|&v| [v, v]).collect();
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, 16, 16);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Sixteen);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&wide).unwrap();
        writer.finish().unwrap();
        assert_eq!(decode(&out), Ok(image));
    }

    fn icon() -> Icon {
        Icon {
            s16: filled(16),
            s32: filled(32),
            s24: None,
            s48: None,
            s64: filled(64),
        }
    }

    #[test]
    fn the_mac_icon_holds_every_size_from_16_to_1024() {
        let sizes = mac_sizes(&icon());
        assert_eq!(
            sizes.keys().copied().collect::<Vec<_>>(),
            [16, 32, 64, 128, 256, 512, 1024]
        );
        let icns = icns(&sizes);
        assert_eq!(&icns[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes(icns[4..8].try_into().unwrap()),
            u32::try_from(icns.len()).unwrap()
        );
        // Each entry's length covers its header, so walking them ends at the file's end.
        let mut at = 8;
        let mut kinds = Vec::new();
        while at < icns.len() {
            kinds.push(String::from_utf8(icns[at..at + 4].to_vec()).unwrap());
            at += usize::try_from(u32::from_be_bytes(icns[at + 4..at + 8].try_into().unwrap()))
                .unwrap();
        }
        assert_eq!(at, icns.len());
        assert_eq!(
            kinds,
            [
                "icp4", "ic11", "icp5", "ic12", "ic07", "ic13", "ic08", "ic14", "ic09", "ic10"
            ]
        );
    }

    #[test]
    fn the_windows_icon_holds_five_sizes_and_48_falls_back_to_the_16_tripled() {
        let sizes = windows_sizes(&icon());
        assert_eq!(
            sizes.keys().copied().collect::<Vec<_>>(),
            [16, 32, 48, 64, 256]
        );
        assert_eq!(decode(&sizes[&48]).unwrap(), scale(&filled(16), 3));
        let ico = ico(&sizes);
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 5, 0]);
        // The 256 is written as 0, and each entry points at its PNG.
        let entry = |n: usize| &ico[6 + n * 16..6 + (n + 1) * 16];
        assert_eq!(entry(4)[0], 0);
        let offset =
            usize::try_from(u32::from_le_bytes(entry(0)[12..16].try_into().unwrap())).unwrap();
        assert_eq!(&ico[offset..offset + 8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn a_drawn_24_joins_the_windows_icon() {
        let mut with = icon();
        with.s24 = Some(filled(24));
        let sizes = windows_sizes(&with);
        assert_eq!(
            sizes.keys().copied().collect::<Vec<_>>(),
            [16, 24, 32, 48, 64, 256]
        );
        assert_eq!(decode(&sizes[&24]), Ok(filled(24)));
        assert_eq!(&ico(&sizes)[4..6], &[6, 0]);
    }
}
