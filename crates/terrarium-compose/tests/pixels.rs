//! Exact pixel snapshots: every blend mode at several opacities, day, night and a half mix,
//! shear, text, the nine-slice, the upscaler, each of the composer's presets and of the
//! synthetic climate's named states by day and by night, and the terrarium in motion. Between
//! them the presets and the storm show every sky state, day and night: clear, overcast, haze
//! and storm, under neutral, dusk and sunburst light.
//!
//! Each picture is kept twice: as a PNG to look at, and as the SHA-256 of its raw pixels. A new
//! `png` release that encodes differently fails only the PNGs; a pixel that changed fails both.
//! Create or re-bless them with `INSTA_UPDATE=always`, then look at every PNG that changed.

#![forbid(unsafe_code)]

use std::fmt::Write as _;

use sha2::{Digest, Sha256};
use terrarium_compose::catalog::{Art, Blend, Catalog, Layer, SpriteId};
use terrarium_compose::clock::LocalClock;
use terrarium_compose::composer::{Composer, OverlayInput};
use terrarium_compose::compositor::composite;
use terrarium_compose::export;
use terrarium_compose::frame::{Area, Draw, Frame, NATIVE_HEIGHT, Rgba, Shear, Viewport};
use terrarium_compose::nine_slice::NineSlice;
use terrarium_compose::order::{KeyedDraw, Subject};
use terrarium_compose::palette::Slot;
use terrarium_compose::scale::{SizeStep, StripScale, upscale_into};
use terrarium_compose::text::{Face, Label, Text};
use terrarium_compose::tween::Millis;
use terrarium_compose::variants::ArtTable;
use terrarium_compose::{preset, sample, synthetic};
use terrarium_truth::scene::{Weather, Wind};

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
    let hex = Sha256::digest(pixels.as_flattened())
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        });
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
    snapshot(name, image.width(), NATIVE_HEIGHT, image.pixels());
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

/// Each preset composed from nothing and settled, by day, with the preset salt.
#[test]
fn the_composer_s_presets() {
    let table = ArtTable::new(catalog()).expect("the embedded art is complete");
    for name in preset::NAMES {
        let frame = preset::frame(art(), &table, name, "repo-terrarium", 0).expect(name);
        snapshot_frame(&format!("preset-{name}"), &frame);
    }
}

/// Each preset by night: the night palette, the moon and stars, and every glow.
#[test]
fn the_composer_s_presets_by_night() {
    let table = ArtTable::new(catalog()).expect("the embedded art is complete");
    for name in preset::NAMES {
        let frame = preset::frame(art(), &table, name, "repo-terrarium", 255).expect(name);
        snapshot_frame(&format!("preset-{name}-night"), &frame);
    }
}

/// Each named state of the synthetic climate (`--demo`'s stages, the debug window's stand-in
/// for truth), composed from nothing and settled, by day and by night: all six presentation
/// states, struggling twice, from low light and from poor soil.
#[test]
fn the_synthetic_states_by_day_and_night() {
    let table = ArtTable::new(catalog()).expect("the embedded art is complete");
    for (name, knobs) in synthetic::NAMED {
        for (mix, suffix) in [(0, ""), (255, "-night")] {
            let frame = synthetic::frame(art(), &table, &knobs, "repo-terrarium", mix, false);
            snapshot_frame(&format!("synthetic-{name}{suffix}"), &frame);
        }
    }
}

/// The thriving look in the 735 columns a 2940 px screen shows at 4×, by day and night: the
/// scene 48 columns in, the backdrop, soil, glass, clouds and label to the edges.
#[test]
fn the_thriving_terrarium_in_a_wider_viewport() {
    let table = ArtTable::new(catalog()).expect("the embedded art is complete");
    let knobs = synthetic::named("thriving").unwrap();
    for (mix, suffix) in [(0, ""), (255, "-night")] {
        let frame = synthetic::frame_in(
            art(),
            &table,
            &knobs,
            "repo-terrarium",
            mix,
            false,
            Viewport::new(735),
        );
        snapshot_frame(&format!("wide-thriving{suffix}"), &frame);
    }
}

/// The one sky state no preset holds: the mature terrarium under a storm, by day and night.
#[test]
fn a_storm_sky_by_day_and_night() {
    let table = ArtTable::new(catalog()).expect("the embedded art is complete");
    let mut input = preset::scene("mature").unwrap();
    input.weather = Weather::Storm;
    let overlay = preset::overlay("mature", "main - storm").unwrap();
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&input, &overlay, Millis(0));
    let scenery = composer.scenery(art(), &table);
    for (mix, when) in [(0, "day"), (255, "night")] {
        let frame = composer.frame(art(), &table, &scenery, mix, composer.settled_at());
        snapshot_frame(&format!("sky-storm-{when}"), &frame);
    }
}

/// Frames stacked top to bottom into one picture, a key-frame sheet.
fn snapshot_sheet(name: &str, frames: &[Frame]) {
    let images: Vec<_> = frames.iter().map(|frame| composite(art(), frame)).collect();
    let width = images[0].width();
    let pixels: Vec<Rgba> = images
        .iter()
        .flat_map(|image| image.pixels().iter().copied())
        .collect();
    let height = NATIVE_HEIGHT * u32::try_from(images.len()).unwrap();
    snapshot(name, width, height, &pixels);
}

/// Each event in the sky, played over the thriving terrarium as truth would bring it, in key
/// frames from its update: the storm front crossing a quarter, a half and three quarters of the
/// way and across; the clearing likewise; lightning's two frames and after; dusk and sunburst
/// half and wholly in; rain fading in, falling and fading out; a windstorm's leaves on their
/// way. By day and by night, and the storm from the right by day.
#[test]
fn sky_events_in_key_frames_by_day_and_night() {
    use terrarium_truth::Tick;
    use terrarium_truth::scene::{EventKind, Side};

    let table = ArtTable::new(catalog()).expect("the embedded art is complete");
    // A storm front crosses 640 columns and its curtain's 24, a column a frame.
    let across = (640 + 24) * 67;
    let cases: [(EventKind, Side, Weather, Vec<u64>); 8] = [
        (
            EventKind::Storm,
            Side::Left,
            Weather::Clear,
            vec![0, across / 4, across / 2, 3 * across / 4, across],
        ),
        (
            EventKind::Storm,
            Side::Right,
            Weather::Clear,
            vec![across / 4, across / 2, 3 * across / 4],
        ),
        (
            EventKind::Clearing,
            Side::Left,
            Weather::Storm,
            vec![0, across / 4, across / 2, 3 * across / 4, across],
        ),
        (
            EventKind::Lightning,
            Side::Left,
            Weather::Storm,
            vec![0, 1, 167, 333],
        ),
        (
            EventKind::Dusk,
            Side::Left,
            Weather::Clear,
            vec![0, 600, 1200],
        ),
        (
            EventKind::Sunburst,
            Side::Left,
            Weather::Clear,
            vec![0, 600, 1200],
        ),
        (
            EventKind::RainAfterDrought,
            Side::Left,
            Weather::Clear,
            vec![1500, 3000, 16_000, 28_500, 30_000],
        ),
        (
            EventKind::Windstorm,
            Side::Left,
            Weather::Clear,
            vec![2000, 5000, 9000, 14_000, 20_000],
        ),
    ];
    for (kind, side, before, moments) in cases {
        let nights: &[bool] = if side == Side::Right {
            &[false]
        } else {
            &[false, true]
        };
        for &night in nights {
            let clock = if night {
                LocalClock::midnight(preset::DATE)
            } else {
                LocalClock::noon(preset::DATE)
            };
            let overlay = OverlayInput::new(vec!["repo-terrarium".into()], clock);
            let mut scene = preset::scene("thriving").unwrap();
            scene.weather = before;
            let mut composer = Composer::new(&table, preset::SALT);
            composer.update(&scene, &overlay, Millis(0));
            let begin = composer.settled_at();
            synthetic::set_event(&mut scene, kind, Tick::new(1));
            if let Some(event) = &mut scene.event {
                event.side = side;
            }
            composer.update(&scene, &overlay, begin);
            let scenery = composer.scenery(art(), &table);
            let mix = if night { 255 } else { 0 };
            let frames: Vec<Frame> = moments
                .iter()
                .map(|&ms| composer.frame(art(), &table, &scenery, mix, Millis(begin.0 + ms)))
                .collect();
            let name = format!("{kind:?}").to_ascii_lowercase();
            let side = if side == Side::Right { "-right" } else { "" };
            let when = if night { "night" } else { "day" };
            snapshot_sheet(&format!("event-{name}{side}-{when}"), &frames);
        }
    }
}

/// The thriving terrarium, living, from its first update at `clock`, in `wind`, perhaps dimmed.
fn living(clock: LocalClock, wind: Wind, dim: bool) -> (Composer, ArtTable) {
    let table = ArtTable::new(catalog()).expect("the embedded art is complete");
    let mut input = preset::scene("thriving").unwrap();
    input.wind = wind;
    let overlay = OverlayInput {
        dim,
        ..OverlayInput::new(vec!["repo-terrarium".into()], clock)
    };
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&input, &overlay, Millis(0));
    (composer, table)
}

/// The first moment, in 167 ms steps from the first turn that moves, at which `wanted` holds
/// of the draws: within the first 33 minutes, by which the first sweep has crossed.
fn first(
    composer: &Composer,
    table: &ArtTable,
    mix: u8,
    wanted: impl Fn(&[KeyedDraw]) -> bool,
) -> Millis {
    (24_000..2_000_000)
        .step_by(167)
        .map(Millis)
        .find(|&now| wanted(&composer.draws(art(), table, mix, now)))
        .expect("the moment comes")
}

fn routine(keyed: &KeyedDraw) -> Option<&'static str> {
    catalog().sprite(keyed.draw.sprite).keys.state.as_deref()
}

/// Motion, each at the first moment it shows: a resident walking while a gale leans the
/// plants; a firefly's lantern by night; the reflection crossing the glass; and dim mode.
#[test]
fn the_terrarium_in_motion() {
    let (composer, table) = living(LocalClock::noon(preset::DATE), Wind::Gale, false);
    let scenery = composer.scenery(art(), &table);
    let walking = first(&composer, &table, 0, |draws| {
        draws.iter().any(|k| routine(k) == Some("move"))
            && draws.iter().any(|k| k.draw.shear.offset(0) == 2)
    });
    snapshot_frame(
        "motion-day",
        &composer.frame(art(), &table, &scenery, 0, walking),
    );
    let sweeping = first(&composer, &table, 0, |draws| {
        draws
            .iter()
            .any(|k| k.key.subject == Subject::Sweep && (300..340).contains(&k.draw.x))
    });
    snapshot_frame(
        "sweep",
        &composer.frame(art(), &table, &scenery, 0, sweeping),
    );

    let (composer, table) = living(LocalClock::midnight(preset::DATE), Wind::Breezy, false);
    let glowing = first(&composer, &table, 255, |draws| {
        draws
            .iter()
            .any(|k| routine(k) == Some("signal-glow") && k.draw.opacity == 96)
    });
    snapshot_frame(
        "motion-night",
        &composer.frame(art(), &table, &scenery, 255, glowing),
    );

    let (composer, table) = living(LocalClock::noon(preset::DATE), Wind::Breezy, true);
    let frame = composer.frame(art(), &table, &scenery, 0, composer.settled_at());
    assert_eq!(frame.dim, 153);
    snapshot_frame("dimmed", &frame);
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
fn the_sample_upscaled_2x_in_a_wider_viewport() {
    // 1301 at 2× shows 651 columns, the scene 6 in, and the last column cut to one pixel.
    let scale = StripScale::for_physical_width(1301, SizeStep::Full);
    assert_eq!((scale.scale, scale.columns, scale.image_x), (2, 651, 0));
    let table = ArtTable::new(&art().catalog).unwrap();
    let frame = sample::frame_in(art(), &table, "repo-terrarium", 0, 0, scale.viewport());
    let image = composite(art(), &frame);
    let mut pixels = vec![[0; 4]; scale.pixel_count()];
    upscale_into(&image, &scale, |colour| colour, &mut pixels).unwrap();
    snapshot("upscale-2x", scale.width, scale.height, &pixels);
}
