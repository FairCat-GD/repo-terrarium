//! The time-lapse as a GIF: the same frames always make the same bytes.
//!
//! The first frame is the whole picture, and each frame after it the rectangle that changed,
//! drawn over the one before. A rectangle's colours are counted exactly. Up to 256 of them make
//! its own palette as they are. Past that, a median cut of the counted colours picks 256, and
//! each colour takes the nearest of them, with no dithering, in integers throughout. Frames
//! wait 7, 7 and 6 hundredths of a second in turn ([`DELAYS_CS`]), and the file repeats
//! forever.
//!
//! Colours are chosen at 1× and their indices repeated for the scale, so a larger GIF costs no
//! more choosing. Only the frame before is kept: frames are written as they come.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::{self, Write};

use ::gif::{DisposalMethod, Encoder, EncodingError, Frame, Repeat};

use crate::frame::{NATIVE_HEIGHT, NATIVE_WIDTH, NativeImage};
use crate::scale::{Damage, StripScale};
use crate::timelapse::DELAYS_CS;

/// The most colours a frame's palette holds.
const COLOURS: usize = 256;

type Colour = [u8; 3];

/// A GIF of the scene, written a frame at a time.
pub struct GifWriter<W: Write> {
    encoder: Encoder<W>,
    scale: u32,
    previous: Option<NativeImage>,
    frames: usize,
}

impl<W: Write> GifWriter<W> {
    /// A GIF of the scene at `scale` times its size, repeating forever, written to `writer`.
    pub fn new(writer: W, scale: u32) -> io::Result<GifWriter<W>> {
        let size = StripScale::exact(scale);
        let width = u16::try_from(size.width).map_err(io::Error::other)?;
        let height = u16::try_from(size.height).map_err(io::Error::other)?;
        let mut encoder = Encoder::new(writer, width, height, &[]).map_err(failed)?;
        encoder.set_repeat(Repeat::Infinite).map_err(failed)?;
        Ok(GifWriter {
            encoder,
            scale: size.scale,
            previous: None,
            frames: 0,
        })
    }

    /// Writes the next frame, `image`, as the rectangle that changed since the one before (a
    /// single pixel if none did, so it still takes its time).
    ///
    /// # Panics
    /// If `image` is not the scene's width.
    pub fn push(&mut self, image: &NativeImage) -> io::Result<()> {
        assert_eq!(image.width(), NATIVE_WIDTH, "a frame of the scene");
        let area = match &self.previous {
            None => Some(Area {
                x: 0,
                y: 0,
                w: NATIVE_WIDTH,
                h: NATIVE_HEIGHT,
            }),
            Some(previous) => Damage::between(previous, image).map(|damage| Area {
                x: damage.x,
                y: damage.y,
                w: damage.w,
                h: damage.h,
            }),
        }
        .unwrap_or(Area {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
        });
        let (palette, indices) = indexed(image, area);
        let k = self.scale;
        let coordinate = |native: u32| u16::try_from(native * k).map_err(io::Error::other);
        let frame = Frame {
            delay: DELAYS_CS[self.frames % DELAYS_CS.len()],
            dispose: DisposalMethod::Keep,
            left: coordinate(area.x)?,
            top: coordinate(area.y)?,
            width: coordinate(area.w)?,
            height: coordinate(area.h)?,
            palette: Some(palette.into_iter().flatten().collect()),
            buffer: Cow::Owned(replicate(&indices, area.w, k)),
            ..Frame::default()
        };
        self.encoder.write_frame(&frame).map_err(failed)?;
        self.previous = Some(image.clone());
        self.frames += 1;
        Ok(())
    }

    /// How many frames have been written.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Ends the file, and hands back the writer.
    pub fn finish(self) -> io::Result<W> {
        self.encoder.into_inner().map_err(failed)
    }
}

fn failed(err: EncodingError) -> io::Error {
    match err {
        EncodingError::Io(err) => err,
        other => io::Error::other(other),
    }
}

/// A rectangle of native pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Area {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// The colours of `area` of `image` as a palette of at most 256, and each pixel's index into
/// it, row by row.
fn indexed(image: &NativeImage, area: Area) -> (Vec<Colour>, Vec<u8>) {
    let pixels = || {
        (area.y..area.y + area.h).flat_map(move |y| {
            image.row(y)[area.x as usize..(area.x + area.w) as usize]
                .iter()
                .map(|&[r, g, b, _]| [r, g, b])
        })
    };
    let mut counts: BTreeMap<Colour, u64> = BTreeMap::new();
    for colour in pixels() {
        *counts.entry(colour).or_default() += 1;
    }
    let palette = if counts.len() <= COLOURS {
        counts.keys().copied().collect()
    } else {
        median_cut(&counts)
    };
    let index: BTreeMap<Colour, u8> = counts
        .keys()
        .map(|&colour| (colour, nearest(&palette, colour)))
        .collect();
    let indices = pixels().map(|colour| index[&colour]).collect();
    (palette, indices)
}

/// 256 colours for more: the counted colours are split, box by box, until there are 256
/// boxes, and each box gives the mean of its pixels. The box split is the one whose colours
/// spread widest along one channel (then the one with the most pixels, then the first), at
/// the colour by which half its pixels lie on that channel's lower side.
fn median_cut(counts: &BTreeMap<Colour, u64>) -> Vec<Colour> {
    let mut boxes: Vec<Vec<(Colour, u64)>> =
        vec![counts.iter().map(|(&colour, &n)| (colour, n)).collect()];
    while boxes.len() < COLOURS {
        let Some((at, channel)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, colours)| colours.len() > 1)
            .map(|(at, colours)| {
                let (channel, spread) = widest(colours);
                (at, channel, spread, weight(colours))
            })
            .max_by(|a, b| a.2.cmp(&b.2).then(a.3.cmp(&b.3)).then(b.0.cmp(&a.0)))
            .map(|(at, channel, ..)| (at, channel))
        else {
            break;
        };
        let mut colours = std::mem::take(&mut boxes[at]);
        colours.sort_unstable_by_key(|&(colour, _)| (colour[channel], colour));
        let total = weight(&colours);
        let mut seen = 0;
        let cut = colours
            .iter()
            .position(|&(_, n)| {
                seen += n;
                seen * 2 >= total
            })
            .map_or(1, |at| at + 1)
            .clamp(1, colours.len() - 1);
        let upper = colours.split_off(cut);
        boxes[at] = colours;
        boxes.push(upper);
    }
    boxes.iter().map(|colours| mean(colours)).collect()
}

/// The channel along which `colours` spread widest, the first of equals, and how wide.
fn widest(colours: &[(Colour, u64)]) -> (usize, u8) {
    (0..3)
        .map(|channel| {
            let values = colours.iter().map(|(colour, _)| colour[channel]);
            let spread = values.clone().max().unwrap_or(0) - values.min().unwrap_or(0);
            (channel, spread)
        })
        .fold(
            (0, 0),
            |best, this| if this.1 > best.1 { this } else { best },
        )
}

fn weight(colours: &[(Colour, u64)]) -> u64 {
    colours.iter().map(|&(_, n)| n).sum()
}

/// The mean colour of `colours`' pixels, each channel rounded half up.
fn mean(colours: &[(Colour, u64)]) -> Colour {
    let total = weight(colours).max(1);
    [0, 1, 2].map(|channel| {
        let sum: u64 = colours
            .iter()
            .map(|&(colour, n)| u64::from(colour[channel]) * n)
            .sum();
        u8::try_from((sum + total / 2) / total).unwrap_or(u8::MAX)
    })
}

/// The index of the palette's colour nearest `colour` by squared distance, the first of equals.
fn nearest(palette: &[Colour], colour: Colour) -> u8 {
    let distance = |entry: &Colour| -> u32 {
        (0..3)
            .map(|c| {
                let d = u32::from(entry[c].abs_diff(colour[c]));
                d * d
            })
            .sum()
    };
    palette
        .iter()
        .enumerate()
        .min_by_key(|&(_, entry)| distance(entry))
        .and_then(|(at, _)| u8::try_from(at).ok())
        .unwrap_or(0)
}

/// `indices`, rows `width` long, each index repeated `k` times along its row and each row `k`
/// times over.
fn replicate(indices: &[u8], width: u32, k: u32) -> Vec<u8> {
    if k == 1 {
        return indices.to_vec();
    }
    let k = k as usize;
    let mut out = Vec::with_capacity(indices.len() * k * k);
    for row in indices.chunks(width as usize) {
        let wide: Vec<u8> = row
            .iter()
            .flat_map(|&index| std::iter::repeat_n(index, k))
            .collect();
        for _ in 0..k {
            out.extend_from_slice(&wide);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Rgba;
    use crate::scale::upscale_into;

    /// A GIF read back: each frame as the whole picture with its delay, the repeat, and the
    /// size.
    struct Decoded {
        frames: Vec<(u16, Vec<Rgba>)>,
        repeat: ::gif::Repeat,
        size: (u16, u16),
    }

    fn decode(bytes: &[u8]) -> Decoded {
        let mut options = ::gif::DecodeOptions::new();
        options.set_color_output(::gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(bytes).unwrap();
        let size = (decoder.width(), decoder.height());
        let mut canvas = vec![[0, 0, 0, 0]; usize::from(size.0) * usize::from(size.1)];
        let mut frames = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            let (left, top) = (usize::from(frame.left), usize::from(frame.top));
            let width = usize::from(frame.width);
            for (i, pixel) in frame.buffer.chunks(4).enumerate() {
                let (x, y) = (left + i % width, top + i / width);
                canvas[y * usize::from(size.0) + x] = [pixel[0], pixel[1], pixel[2], pixel[3]];
            }
            frames.push((frame.delay, canvas.clone()));
        }
        Decoded {
            frames,
            repeat: decoder.repeat(),
            size,
        }
    }

    fn write(images: &[NativeImage], scale: u32) -> Vec<u8> {
        let mut writer = GifWriter::new(Vec::new(), scale).unwrap();
        for image in images {
            writer.push(image).unwrap();
        }
        writer.finish().unwrap()
    }

    /// A picture of `colours` colours, in bands down the columns.
    fn bands(colours: u32, shift: u8) -> NativeImage {
        let mut image = NativeImage::filled([0, 0, 0, 255]);
        for x in 0..NATIVE_WIDTH {
            let band = x * colours / NATIVE_WIDTH;
            let [_, a, b, c] = band.to_be_bytes();
            for y in 0..NATIVE_HEIGHT {
                image.set_pixel(x, y, [a.wrapping_add(shift), b.wrapping_mul(7), c, 255]);
            }
        }
        image
    }

    #[test]
    fn up_to_256_colours_come_back_exactly_with_the_delays_and_the_repeat() {
        let images = [bands(256, 0), bands(256, 0), bands(200, 3), bands(2, 9)];
        let bytes = write(&images, 1);
        let Decoded {
            frames,
            repeat,
            size,
        } = decode(&bytes);
        assert_eq!(repeat, ::gif::Repeat::Infinite);
        assert_eq!(size, (640, 74));
        let delays: Vec<u16> = frames.iter().map(|(delay, _)| *delay).collect();
        assert_eq!(delays, [7, 7, 6, 7]);
        for ((_, pixels), image) in frames.iter().zip(&images) {
            assert_eq!(pixels.as_slice(), image.pixels());
        }
        assert_eq!(bytes, write(&images, 1), "the same bytes every time");
    }

    #[test]
    fn a_frame_is_the_rectangle_that_changed_or_one_pixel() {
        let first = bands(4, 0);
        let mut second = first.clone();
        second.set_pixel(30, 40, [1, 2, 3, 255]);
        second.set_pixel(35, 41, [1, 2, 3, 255]);
        let bytes = write(&[first.clone(), second.clone(), second], 2);
        let mut decoder = ::gif::DecodeOptions::new()
            .read_info(bytes.as_slice())
            .unwrap();
        let mut rects = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            rects.push((frame.left, frame.top, frame.width, frame.height));
        }
        assert_eq!(rects, [(0, 0, 1280, 148), (60, 80, 12, 4), (0, 0, 2, 2)]);
    }

    #[test]
    fn a_scaled_gif_is_the_scaled_picture() {
        let image = bands(40, 1);
        let Decoded { frames, size, .. } = decode(&write(std::slice::from_ref(&image), 3));
        assert_eq!(size, (1920, 222));
        let scale = StripScale::exact(3);
        let mut expected = vec![[0; 4]; scale.pixel_count()];
        upscale_into(&image, &scale, |rgba| rgba, &mut expected).unwrap();
        assert_eq!(frames[0].1, expected);
    }

    #[test]
    fn past_256_colours_each_takes_the_nearest_of_256_and_the_bytes_hold() {
        // 640 columns of distinct colours.
        let mut image = NativeImage::filled([0, 0, 0, 255]);
        for x in 0..NATIVE_WIDTH {
            let [_, _, high, low] = x.to_be_bytes();
            for y in 0..NATIVE_HEIGHT {
                image.set_pixel(x, y, [low, high.wrapping_mul(80), low / 2, 255]);
            }
        }
        let bytes = write(std::slice::from_ref(&image), 1);
        assert_eq!(bytes, write(std::slice::from_ref(&image), 1));
        let mut decoder = ::gif::DecodeOptions::new()
            .read_info(bytes.as_slice())
            .unwrap();
        let frame = decoder.read_next_frame().unwrap().unwrap();
        let palette: Vec<Colour> = frame
            .palette
            .as_ref()
            .unwrap()
            .chunks(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        assert_eq!(palette.len(), 256);
        for (i, &index) in frame.buffer.iter().enumerate() {
            let x = u32::try_from(i).unwrap() % NATIVE_WIDTH;
            let [r, g, b, _] = image.pixel(x, 0);
            assert_eq!(index, nearest(&palette, [r, g, b]), "column {x}");
        }
    }

    #[test]
    fn a_median_cut_splits_the_widest_box_at_its_weighted_middle() {
        // Two grey ramps, one heavy: the cut lands where half the pixels lie.
        let mut counts = BTreeMap::new();
        for v in 0..=255_u8 {
            counts.insert([v, v, v], if v < 64 { 10 } else { 1 });
        }
        counts.insert([0, 255, 0], 1);
        let palette = median_cut(&counts);
        assert_eq!(palette.len(), 256);
        assert_eq!(palette, median_cut(&counts));
        assert_eq!(widest(&[([0, 9, 9], 1), ([9, 0, 9], 1)]), (0, 9));
        assert_eq!(mean(&[([0, 0, 0], 1), ([3, 4, 5], 1)]), [2, 2, 3]);
        assert_eq!(
            nearest(&[[0, 0, 0], [10, 10, 10], [10, 10, 10]], [9, 9, 9]),
            1
        );
    }
}
