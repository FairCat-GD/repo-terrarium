//! A strip picture in an egui window: uploaded once per picture, and shown pixel for pixel at a
//! whole scale (I-11), as the time-lapse window and the connect window show theirs.

use egui::{ColorImage, TextureHandle, TextureOptions, Vec2};
use terrarium_compose::frame::{NATIVE_HEIGHT, NativeImage};

/// The picture as uploaded to the window's egui context, and which picture it is. A window
/// opened again has a new context, which needs it uploaded anew.
pub struct Texture {
    ctx: egui::Context,
    handle: TextureHandle,
    serial: u64,
}

/// The largest whole scale, at most `up_to` and at least 1, at which `width` native columns fit
/// `room` physical pixels.
pub fn whole_scale(width: u32, room: f32, up_to: u32) -> u32 {
    let fits = |k: u32| u16::try_from(width * k).is_ok_and(|wide| f32::from(wide) <= room);
    (1..=up_to).rev().find(|&k| fits(k)).unwrap_or(1)
}

/// The size in points of a picture `width` columns wide: the largest whole scale of it whose
/// physical width fits the room the window has, at most `up_to`, at least 1×.
pub fn size(ui: &egui::Ui, width: u32, up_to: u32) -> Vec2 {
    let ppp = ui.ctx().pixels_per_point();
    let k = whole_scale(width, ui.available_width() * ppp, up_to);
    let points = |native: u32| u16::try_from(native * k).map_or(f32::MAX, f32::from) / ppp;
    Vec2::new(points(width), points(NATIVE_HEIGHT))
}

/// The picture in `ctx`, uploaded once per picture and anew for a new context.
pub fn upload<'a>(
    texture: &'a mut Option<Texture>,
    ctx: &egui::Context,
    image: &NativeImage,
    serial: u64,
) -> &'a TextureHandle {
    let colours = || {
        let bytes: Vec<u8> = image.pixels().iter().flatten().copied().collect();
        ColorImage::from_rgba_unmultiplied([image.width() as usize, NATIVE_HEIGHT as usize], &bytes)
    };
    match texture {
        Some(shown) if shown.ctx == *ctx => {
            if shown.serial != serial {
                shown.handle.set(colours(), TextureOptions::NEAREST);
                shown.serial = serial;
            }
        }
        _ => {
            *texture = Some(Texture {
                ctx: ctx.clone(),
                handle: ctx.load_texture("picture", colours(), TextureOptions::NEAREST),
                serial,
            });
        }
    }
    &texture.as_ref().expect("uploaded").handle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_shown_at_the_largest_whole_scale_that_fits() {
        assert_eq!(whole_scale(640, 1_328.0, 4), 2);
        assert_eq!(whole_scale(640, 1_279.0, 4), 1);
        assert_eq!(whole_scale(640, 2_560.0, 3), 3, "never past the most asked");
        assert_eq!(whole_scale(640, 300.0, 4), 1, "never below 1");
    }
}
