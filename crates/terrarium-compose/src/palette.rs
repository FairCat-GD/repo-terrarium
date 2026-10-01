//! Palette slots and the colours a frame paints them in.
//!
//! The atlas holds slot indices, never colours. Each frame first looks every slot up in the
//! day and night rows and blends the two by its `palette_mix`, so night swaps the slots before
//! anything is composited, and nothing is ever quantized back into the palette.

use crate::catalog::{PALETTE_SLOTS, Palette, Rgb};
use crate::compositor::mix;

/// A logical palette slot, 0..=31. Its atlas byte is the slot + 1, because 0 is transparent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Slot(u8);

impl Slot {
    /// `soil.deep`: glyph ink on the stone label.
    pub const SOIL_DEEP: Slot = Slot(0);
    /// `stone.shadow`: the glass edge's outer column.
    pub const STONE_SHADOW: Slot = Slot(4);
    /// `stone.light`: glyph ink on dark panels.
    pub const STONE_LIGHT: Slot = Slot(6);
    /// `sky.base`: the sky wherever no sky tile is drawn.
    pub const SKY_BASE: Slot = Slot(26);

    pub fn new(slot: u8) -> Option<Slot> {
        (usize::from(slot) < PALETTE_SLOTS).then_some(Slot(slot))
    }

    /// The slot that an atlas byte holds; 0 is transparent and holds none.
    pub fn from_atlas(index: u8) -> Option<Slot> {
        index.checked_sub(1).and_then(Slot::new)
    }

    pub fn get(self) -> u8 {
        self.0
    }

    pub fn index(self) -> usize {
        usize::from(self.0)
    }
}

/// Every slot's colour at one palette mix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lut([Rgb; PALETTE_SLOTS]);

impl Lut {
    /// Day at `mix` 0, night at 255, and each slot's two colours blended in gamma-encoded sRGB
    /// between them, with the compositor's rounding.
    pub fn new(palette: &Palette, mix_by: u8) -> Lut {
        Lut(std::array::from_fn(|slot| {
            let (day, night) = (palette.day[slot], palette.night[slot]);
            std::array::from_fn(|channel| mix(day[channel], night[channel], mix_by))
        }))
    }

    pub fn colour(&self, slot: Slot) -> Rgb {
        self.0[slot.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Art;

    #[test]
    fn slots_stop_at_the_palette_and_atlas_bytes_are_slot_plus_one() {
        assert_eq!(Slot::new(31).map(Slot::get), Some(31));
        assert_eq!(Slot::new(32), None);
        assert_eq!(Slot::from_atlas(0), None);
        assert_eq!(Slot::from_atlas(1), Some(Slot::SOIL_DEEP));
        assert_eq!(Slot::from_atlas(27), Some(Slot::SKY_BASE));
        assert_eq!(Slot::from_atlas(33), None);
    }

    #[test]
    fn mix_0_is_day_and_255_is_night() {
        let palette = &Art::embedded().catalog.palette;
        let day = Lut::new(palette, 0);
        let night = Lut::new(palette, 255);
        for slot in (0..32).filter_map(Slot::new) {
            assert_eq!(day.colour(slot), palette.day[slot.index()]);
            assert_eq!(night.colour(slot), palette.night[slot.index()]);
        }
    }

    #[test]
    fn a_mix_between_blends_each_channel_and_rounds_to_nearest() {
        let palette = &Art::embedded().catalog.palette;
        // sky.base is #E5DEC1 by day and #4D6472 by night (palette-temperate.json).
        assert_eq!(palette.day[26], [0xE5, 0xDE, 0xC1]);
        assert_eq!(palette.night[26], [0x4D, 0x64, 0x72]);
        // (229 × 127 + 77 × 128) / 255 = 152.7, and so on.
        assert_eq!(
            Lut::new(palette, 128).colour(Slot::SKY_BASE),
            [153, 161, 153]
        );
    }
}
