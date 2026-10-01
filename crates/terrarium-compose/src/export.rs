//! Pictures written to files: the one PNG encoder for exports, `cargo xtask render` and the
//! snapshot tests.

use crate::frame::Rgba;

/// `pixels` (row-major, `width × height`) as an 8-bit RGB PNG. Every composed pixel is
/// opaque, so alpha is dropped. Compression and filtering are fixed, so the same pixels
/// always encode to the same bytes with a given `png` release.
///
/// # Panics
/// If `pixels` does not hold `width × height` pixels.
pub fn png(width: u32, height: u32, pixels: &[Rgba]) -> Vec<u8> {
    assert_eq!(
        pixels.len(),
        width as usize * height as usize,
        "a {width}×{height} image"
    );
    let rgb: Vec<u8> = pixels.iter().flat_map(|&[r, g, b, _]| [r, g, b]).collect();
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Balanced);
    encoder.set_filter(png::Filter::Adaptive);
    let mut writer = encoder
        .write_header()
        .expect("a header of a valid size encodes into memory");
    writer
        .write_image_data(&rgb)
        .expect("pixels of the declared size encode into memory");
    writer
        .finish()
        .expect("a complete image finishes into memory");
    out
}

/// Reads back what [`png`] writes: an 8-bit RGB PNG, as its size and opaque pixels. `None`
/// for anything else.
pub fn read_png(bytes: &[u8]) -> Option<(u32, u32, Vec<Rgba>)> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    if (info.color_type, info.bit_depth) != (png::ColorType::Rgb, png::BitDepth::Eight) {
        return None;
    }
    let (rgb, _) = buffer.get(..info.buffer_size())?.as_chunks::<3>();
    let pixels = rgb.iter().map(|&[r, g, b]| [r, g, b, 255]).collect();
    Some((info.width, info.height, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_round_trips_through_its_png() {
        let pixels: Vec<Rgba> = (0..6_u8).map(|i| [i, 2 * i, 3 * i, 255]).collect();
        let bytes = png(3, 2, &pixels);
        assert_eq!(read_png(&bytes), Some((3, 2, pixels.clone())));
        assert_eq!(png(3, 2, &pixels), bytes, "the same pixels, the same bytes");
    }

    #[test]
    fn anything_but_its_own_format_reads_as_none() {
        assert_eq!(read_png(b"not a png"), None);
        let mut gray = Vec::new();
        let mut encoder = png::Encoder::new(&mut gray, 1, 1);
        encoder.set_color(png::ColorType::Grayscale);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[7])
            .unwrap();
        assert_eq!(read_png(&gray), None);
    }

    #[test]
    #[should_panic(expected = "a 4×2 image")]
    fn the_wrong_number_of_pixels_is_refused() {
        let _ = png(4, 2, &[[0; 4]; 6]);
    }
}
