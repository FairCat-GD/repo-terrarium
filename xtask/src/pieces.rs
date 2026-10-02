//! The gallery's `pieces/`: the background art on its own, for art review. Each
//! piece is composed as the strip composes it, from the thriving look with every other draw left
//! out, and written at 3× by day and by night, cropped to the piece:
//!
//! - `sky-<weather>-*.png`: each weather's backdrop above the ground line.
//! - `soil-*.png`: the soil band and its top edge, with the four rows of backdrop above it.
//! - `glass-*.png`: the glass edges and corners, the frame's two ends side by side.
//! - `label-*.png`: the label plate and its text.
//! - `wide-*.png`: the empty terrarium in the 735 columns a 2940 px screen shows at 4×, and
//!   `widest-day.png` in all 840 the art covers: the backdrop, soil and glass to the edges.
//! - `empty-day.png`, `-night.png` and `-dim.png`: every background piece together.
//! - `contract.csv`: each piece's sprites, with what the composer holds them to: size, anchor,
//!   place, layer, opacity by day and night, the palette slots they use, and how they drift.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use terrarium_compose::catalog::{Art, Blend, Catalog, Flags, Layer, SHARED_SLOTS, SpriteId};
use terrarium_compose::compositor::composite;
use terrarium_compose::drift::{
    SOIL_MS_PER_PX, SOIL_PERIOD, SWEEP_JITTER_MS, SWEEP_SLOT_MS, SWEEP_STEP_MS, SWEEP_STEP_PX,
};
use terrarium_compose::export;
use terrarium_compose::frame::{Area, Frame, GROUND_Y, NATIVE_HEIGHT, NATIVE_WIDTH, Rgba};
use terrarium_compose::frame::{Draw, NativeImage, Viewport, WIDEST};
use terrarium_compose::synthetic::{self, Knobs};
use terrarium_compose::text::Label;
use terrarium_compose::variants::ArtTable;
use terrarium_compose::world;
use terrarium_truth::scene::Weather;

use crate::gallery::Files;
use crate::render::large_png;

/// Each native pixel becomes a block this many pixels wide, as in the rest of the gallery.
const SCALE: u32 = 3;
/// The native columns between the two ends of the glass, and their colour.
const GAP: u32 = 4;
const BACKDROP: Rgba = [24, 24, 24, u8::MAX];
/// The rows of backdrop shown above the soil, and the columns and rows shown around the plate.
const SKY_ABOVE_SOIL: i32 = 4;
const MARGIN: i32 = 2;
/// How much of each end of the frame the glass picture shows.
const GLASS_END: i32 = 32;
/// The columns a 2940 px screen shows at 4×.
const WIDE: u32 = 735;

const WIDTH: i32 = NATIVE_WIDTH.cast_signed();
const HEIGHT: i32 = NATIVE_HEIGHT.cast_signed();
const GROUND: i32 = GROUND_Y.cast_signed();

const WEATHERS: [Weather; 4] = [
    Weather::Clear,
    Weather::Overcast,
    Weather::Storm,
    Weather::Haze,
];
const SOIL: [&str; 4] = [
    "tmp.soil.tile.01",
    "tmp.soil.tile.02",
    "tmp.soil.edge.01",
    "tmp.soil.edge.02",
];
const GLASS_EDGES: [&str; 2] = ["tmp.glass.edge.left", "tmp.glass.edge.right"];

/// The background's sprites, by piece.
struct Pieces {
    /// Each weather's backdrop.
    backdrops: Vec<(Weather, SpriteId)>,
    soil: Vec<SpriteId>,
    /// The edges, then the corners.
    glass: Vec<SpriteId>,
    sweep: SpriteId,
    plate: SpriteId,
}

impl Pieces {
    fn new(catalog: &Catalog, table: &ArtTable) -> Result<Pieces, String> {
        let id = |sprite: &str| {
            catalog
                .id(sprite)
                .ok_or_else(|| format!("no sprite {sprite}"))
        };
        let backdrops = WEATHERS
            .iter()
            .map(|&weather| {
                let backdrop = table.weather(weather).first().copied();
                backdrop
                    .map(|backdrop| (weather, backdrop))
                    .ok_or_else(|| format!("no backdrop for {}", name(weather)))
            })
            .collect::<Result<_, _>>()?;
        let soil = SOIL
            .iter()
            .map(|sprite| id(sprite))
            .collect::<Result<_, _>>()?;
        let mut glass: Vec<SpriteId> = GLASS_EDGES
            .iter()
            .map(|sprite| id(sprite))
            .collect::<Result<_, _>>()?;
        glass.extend(table.world().corners);
        Ok(Pieces {
            backdrops,
            soil,
            glass,
            sweep: table.world().sweep,
            plate: id(Label::PLATE)?,
        })
    }

    /// Whether `draw` is the label: its plate, or the text inked on it.
    fn label(&self, draw: &Draw) -> bool {
        draw.sprite == self.plate || (draw.layer == Layer::Label && draw.tint.is_some())
    }

    /// Whether `draw` belongs to a backdrop, the soil, the glass or the label.
    fn background(&self, draw: &Draw) -> bool {
        self.backdrops
            .iter()
            .any(|&(_, backdrop)| backdrop == draw.sprite)
            || self.soil.contains(&draw.sprite)
            || self.glass.contains(&draw.sprite)
            || self.label(draw)
    }
}

/// Every file of `pieces/`, its path under the gallery's directory.
pub fn render(art: &Art, table: &ArtTable, label: &str) -> Result<Files, String> {
    let pieces = Pieces::new(&art.catalog, table)?;
    let thriving = synthetic::named("thriving").ok_or("no thriving look")?;
    let sky = Area {
        x: 0,
        y: 0,
        w: WIDTH,
        h: GROUND,
    };
    let soil = Area {
        x: 0,
        y: GROUND - SKY_ABOVE_SOIL,
        w: WIDTH,
        h: HEIGHT - GROUND + SKY_ABOVE_SOIL,
    };
    let glass = [
        Area {
            x: 0,
            y: 0,
            w: GLASS_END,
            h: HEIGHT,
        },
        Area {
            x: WIDTH - GLASS_END,
            y: 0,
            w: GLASS_END,
            h: HEIGHT,
        },
    ];
    let plate = world::label_area(&art.catalog).ok_or("no label plate")?;
    let plate = around(plate, MARGIN);

    let mut files = Files::new();
    for (suffix, mix) in [("day", 0), ("night", u8::MAX)] {
        // The thriving look's own weather is one of the skies; its frame serves the rest.
        let frame = synthetic::frame(art, table, &thriving, label, mix, false);
        for &(weather, backdrop) in &pieces.backdrops {
            let other;
            let sky_frame = if weather == thriving.weather {
                &frame
            } else {
                let knobs = Knobs {
                    weather,
                    ..thriving
                };
                other = synthetic::frame(art, table, &knobs, label, mix, false);
                &other
            };
            let image = composite(art, &only(sky_frame, &|d| d.sprite == backdrop));
            files.push((
                format!("pieces/sky-{}-{suffix}.png", name(weather)),
                crop(&image, &[sky]),
            ));
        }
        let soil_image = composite(art, &only(&frame, &|d| pieces.soil.contains(&d.sprite)));
        files.push((
            format!("pieces/soil-{suffix}.png"),
            crop(&soil_image, &[soil]),
        ));
        let glass_image = composite(art, &only(&frame, &|d| pieces.glass.contains(&d.sprite)));
        files.push((
            format!("pieces/glass-{suffix}.png"),
            crop(&glass_image, &glass),
        ));
        let label_image = composite(art, &only(&frame, &|d| pieces.label(d)));
        files.push((
            format!("pieces/label-{suffix}.png"),
            crop(&label_image, &[plate]),
        ));
        files.push((
            format!("pieces/wide-{suffix}.png"),
            empty_in(art, table, &pieces, &thriving, label, mix, WIDE)?,
        ));
    }
    files.push((
        "pieces/widest-day.png".to_owned(),
        empty_in(art, table, &pieces, &thriving, label, 0, WIDEST)?,
    ));
    for (suffix, mix, dim) in [
        ("day", 0, false),
        ("night", u8::MAX, false),
        ("dim", 0, true),
    ] {
        let frame = synthetic::frame(art, table, &thriving, label, mix, dim);
        let empty = only(&frame, &|d| pieces.background(d));
        files.push((
            format!("pieces/empty-{suffix}.png"),
            large_png(&composite(art, &empty))?,
        ));
    }
    files.push(("pieces/contract.csv".to_owned(), contract(art, &pieces)));
    Ok(files)
}

/// `knobs`' background pieces alone, in `columns`, at 3×.
fn empty_in(
    art: &Art,
    table: &ArtTable,
    pieces: &Pieces,
    knobs: &Knobs,
    label: &str,
    mix: u8,
    columns: u32,
) -> Result<Vec<u8>, String> {
    let viewport = Viewport::new(columns);
    let frame = synthetic::frame_in(art, table, knobs, label, mix, false, viewport);
    large_png(&composite(art, &only(&frame, &|d| pieces.background(d))))
}

/// `frame` with only the draws `keep` keeps.
fn only(frame: &Frame, keep: &dyn Fn(&Draw) -> bool) -> Frame {
    Frame {
        draws: frame.draws.iter().copied().filter(|d| keep(d)).collect(),
        ..*frame
    }
}

/// `area` grown by `margin` on every side, kept inside the frame.
fn around(area: Area, margin: i32) -> Area {
    let (x, y) = ((area.x - margin).max(0), (area.y - margin).max(0));
    let right = (area.x + area.w + margin).min(WIDTH);
    let bottom = (area.y + area.h + margin).min(HEIGHT);
    Area {
        x,
        y,
        w: right - x,
        h: bottom - y,
    }
}

/// The weather's name in a file name.
fn name(weather: Weather) -> &'static str {
    match weather {
        Weather::Clear => "clear",
        Weather::Overcast => "overcast",
        Weather::Storm => "storm",
        Weather::Haze => "haze",
    }
}

/// `areas` of `image` side by side, [`GAP`] apart, each native pixel a block [`SCALE`] wide.
fn crop(image: &NativeImage, areas: &[Area]) -> Vec<u8> {
    let size = |n: i32| u32::try_from(n).unwrap_or(0);
    let gaps = u32::try_from(areas.len().saturating_sub(1)).unwrap_or(0);
    let width = areas.iter().map(|a| size(a.w)).sum::<u32>() + gaps * GAP;
    let height = areas.iter().map(|a| size(a.h)).max().unwrap_or(0);
    let stride = width * SCALE;
    let block = usize::try_from(SCALE).unwrap_or(1);
    let mut pixels = vec![BACKDROP; usize::try_from(stride * height * SCALE).unwrap_or(0)];
    let mut left = 0;
    for area in areas {
        for y in 0..size(area.h) {
            for x in 0..size(area.w) {
                let colour = image.pixel(size(area.x) + x, size(area.y) + y);
                for dy in 0..SCALE {
                    let row = (y * SCALE + dy) * stride + (left + x) * SCALE;
                    let at = usize::try_from(row).unwrap_or(0);
                    pixels[at..at + block].fill(colour);
                }
            }
        }
        left += size(area.w) + GAP;
    }
    export::png(stride, height * SCALE, &pixels)
}

/// One row per background sprite: what the composer draws it at and needs of it.
fn contract(art: &Art, pieces: &Pieces) -> Vec<u8> {
    let mut rows: Vec<(String, SpriteId, String)> = Vec::new();
    for &(weather, backdrop) in &pieces.backdrops {
        let drift = "none: the whole scene holds still".to_owned();
        rows.push((format!("sky-{}", name(weather)), backdrop, drift));
    }
    for &tile in &pieces.soil {
        let drift = format!(
            "1 px / {} right; the pattern repeats every {SOIL_PERIOD} px",
            duration(SOIL_MS_PER_PX)
        );
        rows.push(("soil".to_owned(), tile, drift));
    }
    for &piece in &pieces.glass {
        rows.push(("glass".to_owned(), piece, "none".to_owned()));
    }
    let sweep = format!(
        "{SWEEP_STEP_PX} px / {} right; one pass every {} to {}",
        duration(SWEEP_STEP_MS),
        duration(SWEEP_SLOT_MS - SWEEP_JITTER_MS),
        duration(SWEEP_SLOT_MS + SWEEP_JITTER_MS)
    );
    rows.push(("glass".to_owned(), pieces.sweep, sweep));
    rows.push(("label".to_owned(), pieces.plate, "none".to_owned()));

    let mut csv = String::from(
        "piece,id,size,anchor,place,layer,opacity,night_opacity,blend,tile_x,palette,slots,aseprite_indices,drift\n",
    );
    for (piece, id, drift) in rows {
        let sprite = art.catalog.sprite(id);
        let slots: BTreeSet<u8> = art
            .rows(id)
            .flat_map(|row| row.iter().copied())
            .filter(|&index| index > 0)
            .collect();
        // Art with its own colours takes slots past the shared ones, and its PNG indices are
        // its own, not slot + 1.
        let own = slots.iter().any(|&byte| usize::from(byte) > SHARED_SLOTS);
        let list = |offset: u8| {
            slots
                .iter()
                .map(|index| (index - 1 + offset).to_string())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let place = sprite
            .at
            .map_or_else(|| "-".to_owned(), |at| format!("{} {}", at.x, at.y));
        let night_opacity = sprite
            .night_opacity
            .map_or_else(|| "-".to_owned(), |n| n.to_string());
        let layer = sprite
            .layer
            .map_or_else(|| "-".to_owned(), |l| format!("{l:?}").to_ascii_lowercase());
        let blend = match sprite.blend {
            Blend::Normal => "normal",
            Blend::Multiply => "multiply",
            Blend::Add => "add",
        };
        let _ = writeln!(
            csv,
            "{piece},{},{}x{},{} {},{place},{layer},{},{night_opacity},{blend},{},{},{},{},{drift}",
            sprite.id,
            sprite.rect.w,
            sprite.rect.h,
            sprite.anchor.x,
            sprite.anchor.y,
            sprite.opacity,
            if sprite.flags.contains(Flags::TILE_X) {
                "yes"
            } else {
                "no"
            },
            if own { "own" } else { "shared" },
            list(0),
            if own { "-".to_owned() } else { list(1) },
        );
    }
    csv.into_bytes()
}

/// `ms` in the largest whole unit: `12 s`, `3 min`, `100 ms`.
fn duration(ms: u64) -> String {
    if ms.is_multiple_of(60_000) {
        format!("{} min", ms / 60_000)
    } else if ms.is_multiple_of(1000) {
        format!("{} s", ms / 1000)
    } else {
        format!("{ms} ms")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered() -> (Files, &'static Art<'static>, ArtTable) {
        let art = Art::embedded();
        let table = ArtTable::new(&art.catalog).unwrap();
        (render(art, &table, "repo-terrarium").unwrap(), art, table)
    }

    #[test]
    fn a_crop_scales_each_area_and_sets_them_apart() {
        let mut image = NativeImage::filled([10, 20, 30, 255]);
        image.set_pixel(0, 5, [200, 0, 0, 255]);
        image.set_pixel(608, 5, [0, 200, 0, 255]);
        let areas = [
            Area {
                x: 0,
                y: 0,
                w: 32,
                h: 74,
            },
            Area {
                x: 608,
                y: 0,
                w: 32,
                h: 74,
            },
        ];
        let (w, h, pixels) = export::read_png(&crop(&image, &areas)).unwrap();
        assert_eq!((w, h), ((32 + GAP + 32) * SCALE, 74 * SCALE));
        let at = |x: u32, y: u32| pixels[usize::try_from(y * w + x).unwrap()];
        // Native (0, 5) fills the block at (0, 15); the second area starts past the gap.
        for (dx, dy) in [(0, 0), (2, 2)] {
            assert_eq!(at(dx, 15 + dy), [200, 0, 0, 255]);
            assert_eq!(at((32 + GAP) * SCALE + dx, 15 + dy), [0, 200, 0, 255]);
        }
        assert_eq!(at(3, 15), [10, 20, 30, 255]);
        assert_eq!(at(32 * SCALE, 15), BACKDROP);
    }

    #[test]
    fn every_sprite_a_piece_draws_has_a_contract_row() {
        let (files, art, table) = rendered();
        let csv = &files
            .iter()
            .find(|(n, _)| n == "pieces/contract.csv")
            .unwrap()
            .1;
        let csv = String::from_utf8_lossy(csv);
        let ids: BTreeSet<&str> = csv
            .lines()
            .skip(1)
            .map(|line| line.split(',').nth(1).unwrap())
            .collect();
        let pieces = Pieces::new(&art.catalog, &table).unwrap();
        let thriving = synthetic::named("thriving").unwrap();
        for weather in WEATHERS {
            let knobs = Knobs {
                weather,
                ..thriving
            };
            let frame = synthetic::frame(art, &table, &knobs, "repo-terrarium", 0, false);
            for draw in frame.draws.iter().filter(|d| pieces.background(d)) {
                // The label's glyphs are tinted masks, the font's and not the plate's.
                if draw.tint.is_none() {
                    let id = &art.catalog.sprite(draw.sprite).id;
                    assert!(ids.contains(id.as_str()), "{id} has no row");
                }
            }
        }
        assert!(ids.contains(art.catalog.sprite(pieces.sweep).id.as_str()));
        assert_eq!(ids.len(), csv.lines().count() - 1, "a sprite has two rows");
        let columns = csv.lines().next().unwrap().split(',').count();
        assert!(csv.lines().all(|line| line.split(',').count() == columns));
    }

    #[test]
    fn the_wide_picture_holds_the_scene_s_backdrop_in_its_middle() {
        let (files, art, _) = rendered();
        let picture = |name: &str| {
            let png = &files.iter().find(|(n, _)| n == name).unwrap().1;
            export::read_png(png).unwrap()
        };
        let (w, h, wide) = picture("pieces/wide-day.png");
        assert_eq!((w, h), (3 * WIDE, 3 * NATIVE_HEIGHT));
        let (scene_w, _, scene) = picture("pieces/empty-day.png");
        assert_eq!(scene_w, 3 * NATIVE_WIDTH);
        let (widest_w, _, _) = picture("pieces/widest-day.png");
        assert_eq!(widest_w, 3 * WIDEST);
        // Above the soil, clear of the glass and the label, the scene's columns show the same
        // backdrop, 48 columns in.
        let inset = u32::try_from(Viewport::new(WIDE).inset()).unwrap();
        for y in (0..3 * u32::try_from(GROUND).unwrap()).step_by(3) {
            for x in (3 * 100..3 * 540).step_by(3) {
                let at = |pixels: &[[u8; 4]], width: u32, x: u32| {
                    pixels[usize::try_from(y * width + x).unwrap()]
                };
                assert_eq!(
                    at(&wide, w, x + 3 * inset),
                    at(&scene, scene_w, x),
                    "({x}, {y})"
                );
            }
        }
        let _ = art;
    }

    #[test]
    fn a_margin_stays_inside_the_frame() {
        let inside = around(
            Area {
                x: 9,
                y: 2,
                w: 90,
                h: 9,
            },
            2,
        );
        assert_eq!((inside.x, inside.y, inside.w, inside.h), (7, 0, 94, 13));
        let corner = around(
            Area {
                x: 1,
                y: 1,
                w: 4,
                h: 4,
            },
            3,
        );
        assert_eq!((corner.x, corner.y, corner.w, corner.h), (0, 0, 8, 8));
        let edge = around(
            Area {
                x: 630,
                y: 70,
                w: 10,
                h: 4,
            },
            3,
        );
        assert_eq!((edge.x, edge.y, edge.w, edge.h), (627, 67, 13, 7));
    }
}
