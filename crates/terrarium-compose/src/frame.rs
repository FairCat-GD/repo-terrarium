//! The native frame. Every pixel is composed at 640×74 and only ever scaled by an integer
//! (I-11).

use std::fmt;

pub const NATIVE_WIDTH: u32 = 640;
pub const NATIVE_HEIGHT: u32 = 74;
/// The ground line. The soil band is this row and the seven below it.
pub const GROUND_Y: u32 = 66;

/// One pixel: red, green, blue, alpha.
pub type Rgba = [u8; 4];

/// A native frame in RGBA8, row-major, top row first.
#[derive(Clone, PartialEq, Eq)]
pub struct Rgba640x74 {
    pixels: Box<[Rgba]>,
}

impl Rgba640x74 {
    pub fn filled(colour: Rgba) -> Self {
        Self {
            pixels: vec![colour; (NATIVE_WIDTH * NATIVE_HEIGHT) as usize].into_boxed_slice(),
        }
    }

    /// # Panics
    /// If `(x, y)` lies outside the frame.
    pub fn pixel(&self, x: u32, y: u32) -> Rgba {
        self.pixels[index(x, y)]
    }

    /// # Panics
    /// If `(x, y)` lies outside the frame.
    pub fn set_pixel(&mut self, x: u32, y: u32, colour: Rgba) {
        self.pixels[index(x, y)] = colour;
    }

    /// # Panics
    /// If `y` lies outside the frame.
    pub fn row(&self, y: u32) -> &[Rgba] {
        let start = index(0, y);
        &self.pixels[start..start + NATIVE_WIDTH as usize]
    }
}

fn index(x: u32, y: u32) -> usize {
    assert!(
        x < NATIVE_WIDTH && y < NATIVE_HEIGHT,
        "({x}, {y}) is outside the native frame"
    );
    (y * NATIVE_WIDTH + x) as usize
}

impl fmt::Debug for Rgba640x74 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Rgba640x74 {{ .. }}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_are_row_major() {
        let mut frame = Rgba640x74::filled([0, 0, 0, 255]);
        frame.set_pixel(639, 73, [1, 2, 3, 255]);
        frame.set_pixel(5, 1, [9, 9, 9, 255]);
        assert_eq!(frame.pixel(639, 73), [1, 2, 3, 255]);
        assert_eq!(frame.row(1)[5], [9, 9, 9, 255]);
        assert_eq!(frame.row(73).len(), 640);
    }

    #[test]
    #[should_panic(expected = "outside the native frame")]
    fn a_pixel_outside_the_frame_panics() {
        let _ = Rgba640x74::filled([0; 4]).pixel(640, 0);
    }
}
