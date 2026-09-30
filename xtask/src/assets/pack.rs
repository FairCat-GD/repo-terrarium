//! The runtime atlas: every sprite's pixels, packed deterministically.
//!
//! - Sprites with identical pixels (same size, same indices) share one place.
//! - The distinct images are ordered by the smallest ID that uses them, and laid into shelves:
//!   left to right, with a new shelf when the next image would overflow the width.
//! - The width is 1024, or 2048 when 1024 would need more than 2048 rows. Anything larger
//!   fails.
//!
//! Every input keeps its own rect, so an ID never loses its entry to a twin.

use std::collections::BTreeMap;

use terrarium_compose::catalog::{MAX_ATLAS_SIDE, Rect, Size};

use super::pixels::Indexed;

const WIDTHS: [u16; 2] = [1024, MAX_ATLAS_SIDE];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packed {
    pub size: Size,
    /// Row-major raw PNG indices; unused space is 0, transparent.
    pub atlas: Vec<u8>,
    /// One per input, in input order.
    pub rects: Vec<Rect>,
    /// How many distinct images the inputs share.
    pub unique: usize,
}

/// Packs `images`, which the caller gives in ID order.
pub fn pack(images: &[&Indexed]) -> Result<Packed, String> {
    // Each distinct image once, in the order its first user appears.
    let mut first_use: BTreeMap<(u32, u32, &[u8]), usize> = BTreeMap::new();
    let mut unique: Vec<&Indexed> = Vec::new();
    let slots: Vec<usize> = images
        .iter()
        .map(|image| {
            *first_use
                .entry((image.width, image.height, &image.pixels))
                .or_insert_with(|| {
                    unique.push(image);
                    unique.len() - 1
                })
        })
        .collect();

    for width in WIDTHS {
        if let Some((height, places)) = shelves(&unique, width) {
            let size = Size {
                w: width,
                h: height,
            };
            let mut atlas = vec![0; usize::from(width) * usize::from(height)];
            for (image, place) in unique.iter().zip(&places) {
                blit(&mut atlas, width, image, *place);
            }
            let rects = slots.iter().map(|&slot| places[slot]).collect();
            return Ok(Packed {
                size,
                atlas,
                rects,
                unique: unique.len(),
            });
        }
    }
    Err(format!(
        "{} distinct images do not fit a {MAX_ATLAS_SIDE}×{MAX_ATLAS_SIDE} atlas",
        unique.len()
    ))
}

/// Shelf placement at `width`, or `None` if an image is wider or the shelves run past the
/// maximum height.
fn shelves(images: &[&Indexed], width: u16) -> Option<(u16, Vec<Rect>)> {
    let (mut x, mut y, mut shelf) = (0_u32, 0_u32, 0_u32);
    let mut places = Vec::with_capacity(images.len());
    for image in images {
        if image.width > u32::from(width) {
            return None;
        }
        if x + image.width > u32::from(width) {
            (x, y, shelf) = (0, y + shelf, 0);
        }
        places.push(Rect {
            x: u16::try_from(x).ok()?,
            y: u16::try_from(y).ok()?,
            w: u16::try_from(image.width).ok()?,
            h: u16::try_from(image.height).ok()?,
        });
        x += image.width;
        shelf = shelf.max(image.height);
    }
    let height = u16::try_from(y + shelf)
        .ok()
        .filter(|&h| h <= MAX_ATLAS_SIDE)?;
    Some((height, places))
}

fn blit(atlas: &mut [u8], width: u16, image: &Indexed, place: Rect) {
    let (stride, w) = (usize::from(width), usize::from(place.w));
    for (row, pixels) in image.pixels.chunks(w).enumerate() {
        let start = (usize::from(place.y) + row) * stride + usize::from(place.x);
        atlas[start..start + w].copy_from_slice(pixels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, fill: u8) -> Indexed {
        let len = usize::try_from(width * height).unwrap();
        Indexed {
            width,
            height,
            pixels: vec![fill; len],
        }
    }

    fn cut(packed: &Packed, rect: Rect) -> Vec<u8> {
        let full = Indexed {
            width: u32::from(packed.size.w),
            height: u32::from(packed.size.h),
            pixels: packed.atlas.clone(),
        };
        full.region(rect).unwrap()
    }

    #[test]
    fn every_input_keeps_its_own_rect_and_twins_share_pixels() {
        let (a, b, twin) = (image(3, 2, 1), image(2, 2, 2), image(3, 2, 1));
        let packed = pack(&[&a, &b, &twin]).unwrap();
        assert_eq!(packed.rects.len(), 3);
        assert_eq!(packed.unique, 2);
        assert_eq!(packed.rects[0], packed.rects[2]);
        assert_eq!(cut(&packed, packed.rects[0]), a.pixels);
        assert_eq!(cut(&packed, packed.rects[1]), b.pixels);
    }

    #[test]
    fn shelves_fill_left_to_right_and_never_overlap() {
        let images: Vec<Indexed> = (1..=40)
            .map(|n| image(90 + n, 10 + n % 7, u8::try_from(n % 32 + 1).unwrap()))
            .collect();
        let refs: Vec<&Indexed> = images.iter().collect();
        let packed = pack(&refs).unwrap();
        assert_eq!(packed.size.w, 1024);
        for (i, a) in packed.rects.iter().enumerate() {
            assert!(u32::from(a.x) + u32::from(a.w) <= 1024 && a.y + a.h <= packed.size.h);
            assert_eq!(cut(&packed, *a), images[i].pixels);
            for b in &packed.rects[i + 1..] {
                let apart =
                    a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn the_same_inputs_pack_to_the_same_bytes() {
        let images: Vec<Indexed> = (1..=12)
            .map(|n| image(n * 7, n, u8::try_from(n).unwrap()))
            .collect();
        let refs: Vec<&Indexed> = images.iter().collect();
        assert_eq!(pack(&refs), pack(&refs));
    }

    #[test]
    fn a_tall_set_widens_to_2048_and_a_huge_one_fails() {
        let tall: Vec<Indexed> = (0..3).map(|n| image(1000, 1000, n + 1)).collect();
        let refs: Vec<&Indexed> = tall.iter().collect();
        assert_eq!(pack(&refs).unwrap().size, Size { w: 2048, h: 2000 });

        let huge: Vec<Indexed> = (0..5).map(|n| image(1000, 1000, n + 1)).collect();
        let refs: Vec<&Indexed> = huge.iter().collect();
        assert!(pack(&refs).is_err());
        assert!(pack(&[&image(2049, 1, 1)]).is_err());
    }
}
