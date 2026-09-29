//! A stand-in frame until the compositor draws art: sky, the soil band and the glass edges in
//! palette colours, so the strip's size and placement can be judged on screen.

use crate::frame::{GROUND_Y, NATIVE_HEIGHT, NATIVE_WIDTH, Rgba, Rgba640x74};

// Day colours from `art-assets/biome-temperate/palettes/exports/palette-temperate.json`, named
// by their logical slot.
/// Slot 26, `sky.base`.
const SKY_BASE: Rgba = [0xE5, 0xDE, 0xC1, 0xFF];
/// Slot 3, `soil.light`.
const SOIL_LIGHT: Rgba = [0x6B, 0x59, 0x40, 0xFF];
/// Slot 2, `soil.body`.
const SOIL_BODY: Rgba = [0x51, 0x43, 0x33, 0xFF];
/// The glass edge (slot 4 at opacity 150) over `sky.base`, as the reference scenes give it.
const GLASS_OVER_SKY: Rgba = [0xA5, 0xA3, 0x8D, 0xFF];

/// The glass edges run down the left and right sides only; the art has no top or bottom edge.
const GLASS_WIDTH: u32 = 2;

pub fn frame() -> Rgba640x74 {
    let mut frame = Rgba640x74::filled(SKY_BASE);
    for y in 0..NATIVE_HEIGHT {
        for x in 0..NATIVE_WIDTH {
            let glass = !(GLASS_WIDTH..NATIVE_WIDTH - GLASS_WIDTH).contains(&x);
            frame.set_pixel(x, y, if glass { edge(y) } else { interior(y) });
        }
    }
    frame
}

/// The colour each native row continues with to the monitor's edges: the glass beside the
/// sky, the soil beside the soil band.
pub fn padding() -> [Rgba; NATIVE_HEIGHT as usize] {
    let mut rows = [SKY_BASE; NATIVE_HEIGHT as usize];
    for (y, row) in (0..NATIVE_HEIGHT).zip(&mut rows) {
        *row = edge(y);
    }
    rows
}

/// A row's colour at the frame's side edges.
fn edge(y: u32) -> Rgba {
    match y {
        GROUND_Y => SOIL_LIGHT,
        y if y > GROUND_Y => SOIL_BODY,
        _ => GLASS_OVER_SKY,
    }
}

/// A row's colour between the glass edges.
fn interior(y: u32) -> Rgba {
    if y < GROUND_Y { SKY_BASE } else { edge(y) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sky_sits_between_the_glass_edges_above_the_ground_line() {
        let frame = frame();
        for y in 0..GROUND_Y {
            assert_eq!(frame.pixel(0, y), GLASS_OVER_SKY);
            assert_eq!(frame.pixel(1, y), GLASS_OVER_SKY);
            assert_eq!(frame.pixel(2, y), SKY_BASE);
            assert_eq!(frame.pixel(637, y), SKY_BASE);
            assert_eq!(frame.pixel(638, y), GLASS_OVER_SKY);
            assert_eq!(frame.pixel(639, y), GLASS_OVER_SKY);
        }
    }

    #[test]
    fn the_soil_band_spans_the_full_width_from_the_ground_line_down() {
        let frame = frame();
        assert!(frame.row(GROUND_Y).iter().all(|&p| p == SOIL_LIGHT));
        for y in GROUND_Y + 1..NATIVE_HEIGHT {
            assert!(frame.row(y).iter().all(|&p| p == SOIL_BODY));
        }
    }

    #[test]
    fn padding_continues_the_glass_and_the_soil() {
        let padding = padding();
        assert_eq!(padding[0], GLASS_OVER_SKY);
        assert_eq!(padding[65], GLASS_OVER_SKY);
        assert_eq!(padding[66], SOIL_LIGHT);
        assert_eq!(padding[73], SOIL_BODY);
    }
}
