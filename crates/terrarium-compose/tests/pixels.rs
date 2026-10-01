//! Exact pixel snapshots of the compositor: every blend mode at several opacities, day, night
//! and a half mix, shear, text, the nine-slice and the upscaler.
//!
//! Each picture is kept twice: as a PNG to look at, and as the SHA-256 of its raw pixels. A new
//! `png` release that encodes differently fails only the PNGs; a pixel that changed fails both.
//! Create or re-bless them with `INSTA_UPDATE=always`, then look at every PNG that changed.

#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};
use terrarium_compose::catalog::{Art, Blend, Catalog, Layer, SpriteId};
use terrarium_compose::compositor::composite;
use terrarium_compose::export;
use terrarium_compose::frame::{Area, Draw, Frame, NATIVE_HEIGHT, NATIVE_WIDTH, Rgba, Shear};
use terrarium_compose::nine_slice::NineSlice;
use terrarium_compose::palette::Slot;
use terrarium_compose::sample;
use terrarium_compose::scale::{SizeStep, StripScale, upscale_into};
use terrarium_compose::text::{Face, Label, Text};
use terrarium_compose::variants::ArtTable;
use terrarium_truth::canon::to_hex;

fn art() -> &'static Art<'static> {
    Art::embedded()
}

fn catalog() -> &'static Catalog {
    &art().catalog
}

fn id(name: &str) -> SpriteId {
    catalog().id(name).expect(name)
}

fn snapshot(name: &str, width: u32, height: u32, pixels: &[Rgba]) {
    let hex = to_hex(&Sha256::digest(pixels.as_flattened()));
    insta::assert_snapshot!(
        format!("{name}-pixels"),
        format!("{width}x{height} RGBA8, SHA-256 {hex}")
    );
    insta::assert_binary_snapshot!(
        format!("{name}.png").as_str(),
        export::png(width, height, pixels)
    );
}

fn snapshot_frame(name: &str, frame: &Frame) {
    let image = composite(art(), frame);
    snapshot(name, NATIVE_WIDTH, NATIVE_HEIGHT, image.pixels());
}

fn sample_frame(mix: u8) -> Frame {
    let table = ArtTable::new(catalog()).expect("the embedded art is complete");
    sample::frame(art(), &table, "repo-terrarium", mix, 0)
}

/// A solid rectangle in one slot: the all-one-slot panel centre tile, tinted, tiled and
/// clipped to the area.
fn fill(area: Area, slot: Slot, layer: Layer, blend: Blend, opacity: u8) -> Vec<Draw> {
    let tile = id("tmp.ui.panel.c");
    let mut draws = Vec::new();
    for y in (area.y..area.y + area.h).step_by(3) {
        for x in (area.x..area.x + area.w).step_by(3) {
            draws.push(Draw {
                tint: Some(slot),
                blend,
                opacity,
                clip: Some(area),
                ..Draw::in_layer(catalog(), tile, layer, x, y)
            });
        }
    }
    draws
}

fn slot(n: usize) -> Slot {
    Slot::new(u8::try_from(n % 32).unwrap()).unwrap()
}

#[test]
fn blend_modes_at_several_opacities() {
    // 32 stripes, one per slot, under three bands: normal, multiply and add. Each band has five
    // rows of cells at opacity 0, 64, 128, 191 and 255, each cell a different source slot.
    const STRIPE: i32 = 20;
    const CELL: i32 = 4;
    let mut draws = Vec::new();
    for stripe in 0..32 {
        let x = i32::try_from(stripe).unwrap() * STRIPE;
        let area = Area {
            x,
            y: 0,
            w: STRIPE,
            h: 74,
        };
        draws.extend(fill(area, slot(stripe), Layer::Sky, Blend::Normal, 255));
    }
    for (band, blend) in [Blend::Normal, Blend::Multiply, Blend::Add]
        .into_iter()
        .enumerate()
    {
        for (row, opacity) in [0, 64, 128, 191, 255].into_iter().enumerate() {
            for stripe in 0..32 {
                let area = Area {
                    x: i32::try_from(stripe).unwrap() * STRIPE + 2,
                    y: i32::try_from(band * 24 + row * 4 + 2).unwrap(),
                    w: STRIPE - 4,
                    h: CELL,
                };
                let source = slot(stripe * 7 + band * 11 + 3);
                draws.extend(fill(area, source, Layer::EventOverlays, blend, opacity));
            }
        }
    }
    snapshot_frame(
        "blend-modes",
        &Frame {
            draws,
            ..Frame::default()
        },
    );
}

#[test]
fn the_sample_by_day() {
    snapshot_frame("sample-day", &sample_frame(0));
}

#[test]
fn the_sample_at_a_half_mix() {
    snapshot_frame("sample-half", &sample_frame(128));
}

#[test]
fn the_sample_by_night() {
    snapshot_frame("sample-night", &sample_frame(255));
}

#[test]
fn foliage_sheared_about_its_pivot() {
    // The oak's mature foliage at slopes from a pixel per four rows left to one per row
    // right, then flipped, then flipped, sheared and clipped to its upper half.
    let foliage = id("tmp.plant.oak.foliage.mature.thriving");
    let pivot = catalog()
        .sprite(foliage)
        .shear_pivot
        .expect("a shear pivot")
        .y;
    let base = |x| Draw::anchored(catalog(), foliage, x, 60);
    let mut draws: Vec<Draw> = [-64, 0, 64, 128, 256]
        .into_iter()
        .zip([40, 120, 200, 280, 360])
        .map(|(slope, x)| Draw {
            shear: Shear {
                pivot_y: pivot,
                slope,
            },
            ..base(x)
        })
        .collect();
    draws.push(base(450).mirrored(catalog()));
    let turned = Draw {
        shear: Shear {
            pivot_y: pivot,
            slope: 96,
        },
        ..base(550).mirrored(catalog())
    };
    draws.push(Draw {
        clip: Some(Area {
            x: turned.x,
            y: 0,
            w: 70,
            h: turned.y + 16,
        }),
        ..turned
    });
    snapshot_frame(
        "shear",
        &Frame {
            draws,
            ..Frame::default()
        },
    );
}

#[test]
fn text_in_both_faces_and_both_inks() {
    let body = Text::new(catalog(), Face::Body).unwrap();
    let headings = Text::new(catalog(), Face::Headings).unwrap();
    let label = Label::new(catalog()).unwrap();
    let mut draws = label.draws("feature/a-very-long-branch-name", 10, 2);
    draws.extend(label.draws("main - clear", 110, 2));
    let panel = NineSlice::new(catalog()).unwrap();
    draws.extend(panel.draws(
        Area {
            x: 4,
            y: 14,
            w: 632,
            h: 56,
        },
        Layer::SoilPanel,
    ));
    let lines = [
        (
            body,
            "The quick brown fox jumps over 13 lazy dogs: {}[]<>|~`^_",
            Slot::STONE_LIGHT,
        ),
        (
            body,
            "Fallbacks: caf\u{e9} \u{b7} na\u{ef}ve \u{2603} \u{1f331}",
            Slot::STONE_LIGHT,
        ),
        (
            headings,
            "Headings: lowercase draws in UPPERCASE 0123456789",
            Slot::STONE_LIGHT,
        ),
        (body, "soil.deep ink on a dark panel", Slot::SOIL_DEEP),
    ];
    for (row, (face, line, ink)) in lines.into_iter().enumerate() {
        let y = 20 + i32::try_from(row).unwrap() * 11;
        draws.extend(face.draws(line, 10, y, ink, Layer::SoilPanel));
    }
    snapshot_frame(
        "text",
        &Frame {
            draws,
            ..Frame::default()
        },
    );
}

#[test]
fn nine_slice_panels_at_and_past_the_minimum() {
    let panel = NineSlice::new(catalog()).unwrap();
    let mut draws = Vec::new();
    for (x, w, h) in [
        (10, 9, 9),
        (30, 10, 10),
        (50, 11, 13),
        (80, 40, 30),
        (140, 4, 4),
    ] {
        draws.extend(panel.draws(Area { x, y: 10, w, h }, Layer::SoilPanel));
    }
    snapshot_frame(
        "nine-slice",
        &Frame {
            draws,
            ..Frame::default()
        },
    );
}

#[test]
fn the_sample_upscaled_2x_with_odd_padding() {
    let image = composite(art(), &sample_frame(0));
    let padding = sample::padding(art(), 0, 0);
    // 1301 is 2 × 640 plus 21: 10 columns of padding on the left and 11 on the right.
    let scale = StripScale::for_physical_width(1301, SizeStep::Full);
    assert_eq!((scale.scale, scale.image_x), (2, 10));
    let mut pixels = vec![[0; 4]; scale.pixel_count()];
    upscale_into(&image, &scale, &padding, |colour| colour, &mut pixels).unwrap();
    snapshot("upscale-2x", scale.width, scale.height, &pixels);
}
