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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_round_trips_through_its_png() {
        let pixels: Vec<Rgba> = (0..6_u8).map(|i| [i, 2 * i, 3 * i, 255]).collect();
        let bytes = png(3, 2, &pixels);
        let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buffer).unwrap();
        assert_eq!((info.width, info.height), (3, 2));
        assert_eq!(info.color_type, png::ColorType::Rgb);
        let (rgb, _) = buffer[..info.buffer_size()].as_chunks::<3>();
        let read: Vec<Rgba> = rgb.iter().map(|&[r, g, b]| [r, g, b, 255]).collect();
        assert_eq!(read, pixels);
        assert_eq!(png(3, 2, &pixels), bytes, "the same pixels, the same bytes");
    }

    #[test]
    #[should_panic(expected = "a 4×2 image")]
    fn the_wrong_number_of_pixels_is_refused() {
        let _ = png(4, 2, &[[0; 4]; 6]);
    }
}
