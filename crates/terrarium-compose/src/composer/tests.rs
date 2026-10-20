use std::collections::BTreeSet;

use terrarium_truth::scene::Wind;

use super::*;
use crate::catalog::Layer;
use crate::preset;
use crate::tween::durations::{CLOCK_FIX_MS, LABEL_CYCLE_MS, LABEL_FADE_MS};
use terrarium_truth::scene::ResidentView;

fn table() -> ArtTable {
    ArtTable::new(&Art::embedded().catalog).unwrap()
}

fn scene(name: &str) -> SceneInput {
    preset::scene(name).unwrap()
}

fn overlay(lines: &[&str]) -> OverlayInput {
    OverlayInput::new(
        lines.iter().map(|line| (*line).to_owned()).collect(),
        LocalClock::noon(preset::DATE),
    )
}

/// The draws by day that come and go: every one but the soil's, which, like the scenery,
/// is always there.
fn visible(composer: &Composer, table: &ArtTable, now: Millis) -> Vec<Draw> {
    composer
        .draws(Art::embedded(), table, 0, now)
        .into_iter()
        .filter(|keyed| keyed.key.subject != Subject::Soil)
        .map(|keyed| keyed.draw)
        .collect()
}

fn visible_at(composer: &Composer, table: &ArtTable, mix: u8, now: Millis) -> Vec<Draw> {
    composer
        .draws(Art::embedded(), table, mix, now)
        .into_iter()
        .map(|keyed| keyed.draw)
        .collect()
}

fn of(
    composer: &Composer,
    table: &ArtTable,
    mix: u8,
    now: Millis,
    subject: Subject,
) -> Vec<KeyedDraw> {
    composer
        .draws(Art::embedded(), table, mix, now)
        .into_iter()
        .filter(|keyed| keyed.key.subject == subject)
        .collect()
}

#[test]
fn the_first_update_fades_everything_in_from_nothing() {
    let table = table();
    let mut composer = Composer::new(&table, preset::SALT);
    // No label: its fade is the quickest, and would have finished by the middle.
    composer.update(&scene("thriving"), &overlay(&[]), Millis(1000));
    assert!(visible(&composer, &table, Millis(1000)).is_empty());
    // Within the residents' quicker fade too.
    let middle = visible(&composer, &table, Millis(1600));
    let art = Art::embedded();
    assert!(!middle.is_empty());
    assert!(
        middle
            .iter()
            .all(|d| d.opacity < art.catalog.sprite(d.sprite).opacity)
    );
    let settled = composer.settled_at();
    assert_eq!(settled, Millis(1000 + u64::from(SKY_MS)));
    assert!(
        visible(&composer, &table, settled)
            .iter()
            .all(|d| d.opacity == art.catalog.sprite(d.sprite).opacity)
    );
}

#[test]
fn an_update_never_changes_the_frame_at_its_own_moment() {
    let table = table();
    let mut composer = Composer::new(&table, preset::SALT);
    let mut now = Millis(0);
    for (name, lines, after) in [
        ("thriving", &["a", "b"][..], 2500),
        ("struggling-poor-soil", &["a", "b"], 1700),
        ("dead", &["c"], 3000),
        ("recovering", &[], 1),
        ("struggling-low-light", &["d"], 700),
        ("thriving", &["d"], 9000),
        ("dormant", &["e", "f", "g"], 4100),
        ("mature", &["e"], 20_100),
    ] {
        for mix in [0, 255] {
            let before = visible_at(&composer, &table, mix, now);
            let mut copy = composer.clone();
            copy.update(&scene(name), &overlay(lines), now);
            assert_eq!(
                visible_at(&copy, &table, mix, now),
                before,
                "{name} at {mix}"
            );
        }
        composer.update(&scene(name), &overlay(lines), now);
        now = now.plus(after);
    }
}

#[test]
fn a_restyle_holds_the_old_look_until_the_new_one_has_risen() {
    let mut looks = vec![Look {
        piece: Piece::Tuft,
        opacity: Tween::at(SHOWN),
        behind: false,
    }];
    raise(
        &mut looks,
        Piece::DeadStems,
        Millis(100),
        HALF_RESTYLE,
        HALF_RESTYLE,
    );
    let [old, new] = [looks[0], looks[1]];
    let risen = 100 + u64::from(HALF_RESTYLE);
    assert_eq!(old.opacity.value(Millis(risen)), SHOWN);
    assert_eq!(new.opacity.value(Millis(risen)), SHOWN);
    assert_eq!(
        old.opacity.value(Millis(risen + u64::from(HALF_RESTYLE))),
        0
    );
    // A third look mid-way holds both older ones where they stand.
    raise(
        &mut looks,
        Piece::Tuft,
        Millis(1100),
        HALF_RESTYLE,
        HALF_RESTYLE,
    );
    let held = looks[1].opacity.value(Millis(1100));
    assert!(held > 0 && held < SHOWN);
    assert_eq!(
        looks[1]
            .opacity
            .value(Millis(1100 + u64::from(HALF_RESTYLE))),
        held
    );
    assert_eq!(
        looks[0]
            .opacity
            .value(Millis(1100 + u64::from(HALF_RESTYLE))),
        SHOWN
    );
}

/// A change that waits for a wipe to pass reads its looks as they will stand then: one
/// gone by then stays gone, whatever it shows now.
#[test]
fn a_deferred_raise_keeps_a_look_already_falling_by_then() {
    let cloud = Piece::Cloud(CloudSize::Small);
    let covered = Tween::new(SHOWN, 0, Millis(9999), 1);
    let mut looks = vec![Look {
        piece: cloud,
        opacity: covered,
        behind: true,
    }];
    raise(&mut looks, cloud, Millis(10_000), SKY_MS, HALF_RESTYLE);
    // Not raised again from where it stands now: a new look rises from nothing then.
    assert_eq!(looks.len(), 2);
    assert_eq!(looks[0].opacity, covered);
    assert_eq!(looks[1].opacity.value(Millis(10_000)), 0);
    assert_eq!(looks[1].opacity.value(Millis(5000)), 0);
    // An older look gone by then keeps its course too.
    let mut looks = vec![
        Look {
            piece: Piece::Weather(Weather::Clear),
            opacity: covered,
            behind: true,
        },
        Look {
            piece: Piece::Weather(Weather::Storm),
            opacity: Tween::at(SHOWN),
            behind: false,
        },
    ];
    raise(
        &mut looks,
        Piece::Weather(Weather::Haze),
        Millis(10_000),
        HALF_RESTYLE,
        HALF_RESTYLE,
    );
    assert_eq!(looks[0].opacity, covered);
    assert_eq!(looks[1].opacity.value(Millis(9000)), SHOWN);
}

#[test]
fn a_film_crossfades_so_two_levels_never_stack() {
    let film = |level| Piece::Film(GlassFilm::Condensation, level);
    let mut looks = vec![Look {
        piece: film(1),
        opacity: Tween::at(SHOWN),
        behind: false,
    }];
    raise(&mut looks, film(2), Millis(100), HALF_RESTYLE, HALF_RESTYLE);
    let mid = Millis(100 + u64::from(HALF_RESTYLE) / 2);
    let (old, new) = (looks[0].opacity.value(mid), looks[1].opacity.value(mid));
    // Falling while the new one rises: the two sum to about one look, never two.
    assert!(old < SHOWN && new < SHOWN);
    assert!(
        (u16::from(old) + u16::from(new)).abs_diff(255) <= 2,
        "{old} + {new}"
    );
    assert_eq!(looks[0].opacity.value(Millis(100)), SHOWN);
}

#[test]
fn every_piece_restyles_by_its_art() {
    // Art that is not opaque and normal crossfades; the rest covers.
    let table = table();
    let catalog = &Art::embedded().catalog;
    let plant = Subject::Plant { id: 1, cell: 2 };
    let pieces = [
        (Piece::Tuft, Subject::Fill { cell: 0 }),
        (Piece::DressingTuft, Subject::Dressing(0)),
        (Piece::Weed(WeedForm::Patch), Subject::Weed { cell: 3 }),
        (Piece::Litter(LitterSet::Amber), Subject::Litter(0)),
        (Piece::Fungus, Subject::Fungus(0)),
        (Piece::FungusGlow, Subject::Fungus(0)),
        (
            Piece::Stone(MonumentAge::Ancient),
            Subject::Monument { id: 1, spot: 0 },
        ),
        (
            Piece::Mark {
                age: MonumentAge::Young,
            },
            Subject::Monument { id: 1, spot: 0 },
        ),
        (
            Piece::LichenGlow(MonumentAge::Lichened),
            Subject::Monument { id: 1, spot: 0 },
        ),
        (
            Piece::Fossil(FossilForm::Bone),
            Subject::Fossil { id: 1, spot: 0 },
        ),
        (Piece::Weather(Weather::Overcast), Subject::Weather),
        (Piece::Cloud(CloudSize::Large), Subject::Cloud(0)),
        (Piece::Sun, Subject::Sun),
        (
            Piece::Moon(MoonLook {
                sprite: 2,
                mirrored: false,
            }),
            Subject::Moon,
        ),
        (Piece::Star, Subject::Star(0)),
        (Piece::Light(Light::Dusk), Subject::Light),
        (Piece::Light(Light::Sunburst), Subject::Light),
        (
            Piece::Film(GlassFilm::Dust, 2),
            Subject::Film(GlassFilm::Dust),
        ),
        (
            Piece::PlantGlow {
                species: PlantSpecies::Oak,
                stage: GrowthStage::Mature,
            },
            plant,
        ),
        (
            Piece::Foliage {
                species: PlantSpecies::Oak,
                stage: GrowthStage::Mature,
                state: PlantState::Pale,
            },
            plant,
        ),
    ];
    for (piece, subject) in pieces {
        let sprite = piece.sprite(&table, preset::SALT, subject, 1).expect("art");
        let art = catalog.sprite(sprite);
        let translucent = art.blend != crate::catalog::Blend::Normal || art.opacity < 255;
        let expected = if translucent {
            Restyle::Crossfade
        } else {
            Restyle::Cover
        };
        assert_eq!(piece.restyle(), expected, "{piece:?}");
    }
}

#[test]
fn a_pale_turn_fades_the_foliage_and_never_the_stem() {
    let table = table();
    let mut composer = Composer::new(&table, preset::SALT);
    let thriving = scene("mature");
    composer.update(&thriving, &overlay(&[]), Millis(0));
    let settled = composer.settled_at();
    let mut pale = thriving.clone();
    for plant in &mut pale.plants {
        plant.state = PlantState::Pale;
    }
    composer.update(&pale, &overlay(&[]), settled);
    let mid = settled.plus(HALF_RESTYLE / 2);
    let stems: Vec<Depicted> = composer
        .depicted(Art::embedded(), &table, mid)
        .into_iter()
        .filter(|d| matches!(d.piece, Piece::Stem { .. }))
        .collect();
    assert!(!stems.is_empty());
    assert!(stems.iter().all(|d| d.opacity == SHOWN));
}

#[test]
fn finished_looks_are_pruned_and_live_ones_kept() {
    let table = table();
    let mut composer = Composer::new(&table, preset::SALT);
    let plants = |composer: &Composer| {
        composer
            .entries
            .keys()
            .filter(|s| matches!(s, Subject::Plant { .. }))
            .count()
    };
    composer.update(&scene("thriving"), &overlay(&["a"]), Millis(0));
    let full = plants(&composer);
    let mut empty = scene("thriving");
    empty.plants.clear();
    composer.update(&empty, &overlay(&["a"]), composer.settled_at());
    // The plants are leaving; none is pruned until it has gone.
    assert_eq!(plants(&composer), full);
    composer.update(&empty, &overlay(&["a"]), composer.settled_at().plus(1));
    assert_eq!(plants(&composer), 0);
    assert!(composer.entries.values().all(Entry::shown));
}

#[test]
fn the_tuft_variant_is_the_salt_s_choice_and_never_changes() {
    let table = table();
    let tufts = table.tufts();
    let chosen: Vec<SpriteId> = (0..30)
        .filter_map(|cell| Piece::Tuft.sprite(&table, preset::SALT, Subject::Fill { cell }, 1))
        .collect();
    assert!(chosen.iter().all(|id| tufts.contains(id)));
    assert!(chosen.iter().any(|&id| id != chosen[0]));
    assert_eq!(
        Some(chosen[4]),
        Piece::Tuft.sprite(&table, preset::SALT, Subject::Fill { cell: 4 }, 1)
    );
}

#[test]
fn night_only_art_is_gone_by_day_and_day_only_art_by_night() {
    let table = table();
    let art = Art::embedded();
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&scene("mature"), &overlay(&["a"]), Millis(0));
    let settled = composer.settled_at();
    let flagged = |mix: u8, flag: Flags| {
        composer
            .draws(art, &table, mix, settled)
            .iter()
            .filter(|keyed| art.catalog.sprite(keyed.draw.sprite).flags.contains(flag))
            .count()
    };
    assert_eq!(flagged(0, Flags::NIGHT_ONLY), 0);
    assert!(flagged(255, Flags::NIGHT_ONLY) > 0);
    assert!(flagged(0, Flags::DAY_ONLY) > 0);
    assert_eq!(flagged(255, Flags::DAY_ONLY), 0);
    // Halfway, a glow shows at half its own opacity.
    let glow = table.world().plant_glow;
    let half = composer.draws(art, &table, 128, settled);
    let glows: Vec<u8> = half
        .iter()
        .filter(|keyed| keyed.draw.sprite == glow)
        .map(|keyed| keyed.draw.opacity)
        .collect();
    assert!(!glows.is_empty());
    assert!(
        glows
            .iter()
            .all(|&o| o == scale(art.catalog.sprite(glow).opacity, 128))
    );
}

#[test]
fn a_plant_glows_only_while_young_or_mature_and_thriving() {
    for &stage in GrowthStage::ALL {
        for &state in PlantState::ALL {
            let glows = stage != GrowthStage::Seedling && state == PlantState::Thriving;
            assert_eq!(shows_glow(stage, state), glows, "{stage:?} {state:?}");
        }
    }
}

#[test]
fn a_whole_weather_look_covers_the_old_one_in_every_column() {
    let table = table();
    for viewport in [Viewport::SCENE, Viewport::new(735)] {
        let mut composer = Composer::new(&table, preset::SALT);
        composer.set_viewport(viewport);
        let mut clear = preset::bare(PresentationState::Thriving, Vec::new());
        composer.update(&clear, &overlay(&[]), Millis(0));
        clear.weather = Weather::Overcast;
        let settled = composer.settled_at();
        composer.update(&clear, &overlay(&[]), settled);
        let mid = settled.plus(HALF_RESTYLE / 2);
        let looks = of(&composer, &table, 0, mid, Subject::Weather);
        // One draw a look, the new one on top, each across every column of the viewport.
        assert_eq!(
            looks.iter().map(|keyed| keyed.key.look).collect::<Vec<_>>(),
            [0, 1]
        );
        let width = i32::try_from(viewport.width()).unwrap();
        for keyed in looks {
            let w = i32::from(Art::embedded().catalog.sprite(keyed.draw.sprite).rect.w);
            assert!(
                keyed.draw.x <= 0 && keyed.draw.x + w >= width,
                "{viewport:?}"
            );
        }
    }
}

#[test]
fn marks_paint_over_their_stone_at_the_prose_points() {
    let table = table();
    let mut composer = Composer::new(&table, preset::SALT);
    let mut input = preset::bare(PresentationState::Mature, Vec::new());
    input.monuments = vec![terrarium_truth::scene::MonumentView {
        id: 7,
        age: MonumentAge::Ancient,
        marks: 3,
    }];
    composer.update(&input, &overlay(&[]), Millis(0));
    let spot = composer.monument_spots()[&7];
    let draws = of(
        &composer,
        &table,
        255,
        composer.settled_at(),
        Subject::Monument { id: 7, spot },
    );
    let parts: Vec<u8> = draws.iter().map(|keyed| keyed.key.part).collect();
    assert_eq!(parts, [0, 1, 2, 3, 4], "stone, three marks, then the glow");
    let stone = draws[0].draw;
    let points = &table.world().mark_points;
    for (keyed, point) in draws[1..4].iter().zip(points) {
        assert_eq!(keyed.draw.layer, Layer::Monuments);
        assert_eq!(
            (keyed.draw.x - stone.x, keyed.draw.y - stone.y),
            (i32::from(point.x), i32::from(point.y))
        );
    }
}

#[test]
fn a_landmark_is_found_where_it_is_drawn_and_nowhere_beside_it() {
    let table = table();
    let art = Art::embedded();
    let mut composer = Composer::new(&table, preset::SALT);
    composer.set_viewport(Viewport::new(700));
    let mut input = preset::bare(PresentationState::Mature, Vec::new());
    input.monuments = vec![terrarium_truth::scene::MonumentView {
        id: 7,
        age: MonumentAge::Young,
        marks: 0,
    }];
    input.fossils = vec![terrarium_truth::scene::FossilView {
        id: 9,
        form: FossilForm::Bone,
    }];
    composer.update(&input, &overlay(&[]), Millis(0));
    let now = composer.settled_at();
    let drawn = |subject: Subject| -> Area {
        let draw = of(&composer, &table, 0, now, subject)[0].draw;
        let rect = art.catalog.sprite(draw.sprite).rect;
        Area {
            x: draw.x,
            y: draw.y,
            w: i32::from(rect.w),
            h: i32::from(rect.h),
        }
    };
    let stone = drawn(Subject::Monument {
        id: 7,
        spot: composer.monument_spots()[&7],
    });
    let fossil = drawn(Subject::Fossil {
        id: 9,
        spot: composer.fossil_spots()[&9],
    });
    let at = |composer: &Composer, x, y| composer.landmark_at(art, &table, x, y, now);
    for (area, landmark) in [
        (stone, Landmark::Monument(7)),
        (fossil, Landmark::Fossil(9)),
    ] {
        let c = &composer;
        assert_eq!(at(c, area.x, area.y), Some(landmark));
        let corner = (area.x + area.w - 1, area.y + area.h - 1);
        assert_eq!(at(c, corner.0, corner.1), Some(landmark));
        assert_ne!(at(c, area.x - 1, area.y), Some(landmark));
        assert_ne!(at(c, area.x + area.w, corner.1), Some(landmark));
    }
    // Leaving, it is no longer pointed at.
    input.monuments.clear();
    composer.update(&input, &overlay(&[]), now);
    assert_eq!(at(&composer, stone.x, stone.y), None);
}

#[test]
fn the_label_cycles_its_lines_and_crossfades_between_them() {
    let look = LabelLook {
        lines: vec!["one".into(), "two".into(), "three".into()],
        since: Millis(1000),
        opacity: Tween::at(SHOWN),
        arrived: vec![Millis(1000); 3],
    };
    let cycle = u64::from(LABEL_CYCLE_MS);
    assert_eq!(look.showing(Millis(1000)), [(0, SHOWN)]);
    assert_eq!(look.showing(Millis(1000 + cycle - 1)), [(0, SHOWN)]);
    // The step to the next line: the old one still whole, the new one at nothing.
    assert_eq!(look.showing(Millis(1000 + cycle)), [(0, SHOWN), (1, 0)]);
    let mid = look.showing(Millis(1000 + cycle + u64::from(LABEL_FADE_MS) / 2));
    assert_eq!((mid[0].0, mid[1].0), (0, 1));
    assert_eq!(u16::from(mid[0].1) + u16::from(mid[1].1), 255);
    assert_eq!(
        look.showing(Millis(1000 + cycle + u64::from(LABEL_FADE_MS))),
        [(1, SHOWN)]
    );
    // After the last line, the first again.
    assert_eq!(
        look.showing(Millis(1000 + 3 * cycle + u64::from(LABEL_FADE_MS))),
        [(0, SHOWN)]
    );
    let single = LabelLook {
        lines: vec!["only".into()],
        ..look.clone()
    };
    assert_eq!(single.showing(Millis(1000 + 5 * cycle)), [(0, SHOWN)]);
}

#[test]
fn a_label_line_s_turn_ends_when_the_cycle_moves_on_from_it() {
    let table = table();
    let mut composer = Composer::new(&table, preset::SALT);
    let input = preset::bare(PresentationState::Thriving, Vec::new());
    let cycle = u64::from(LABEL_CYCLE_MS);
    composer.update(
        &input,
        &overlay(&["moss", "main - clear", "sim updated"]),
        Millis(0),
    );
    assert!(!composer.label_step_done(2, Millis(3 * cycle - 1)));
    assert!(composer.label_step_done(2, Millis(3 * cycle)));
    assert!(composer.label_step_done(0, Millis(cycle)));
    assert!(
        !composer.label_step_done(3, Millis(10 * cycle)),
        "no such line"
    );
    // A new set starts again.
    composer.update(
        &input,
        &overlay(&["moss", "sim updated"]),
        Millis(3 * cycle),
    );
    assert!(!composer.label_step_done(1, Millis(4 * cycle)));
    assert!(composer.label_step_done(1, Millis(5 * cycle)));
    // One line never moves on.
    composer.update(&input, &overlay(&["moss"]), Millis(5 * cycle));
    assert!(!composer.label_step_done(0, Millis(50 * cycle)));
}

/// Two composers on one bare scene, labelled `lines` from the start: one to change, one to
/// compare with.
fn label_pair(table: &ArtTable, lines: &[&str]) -> (Composer, Composer) {
    let input = preset::bare(PresentationState::Thriving, Vec::new());
    let make = || {
        let mut composer = Composer::new(table, preset::SALT);
        composer.update(&input, &overlay(lines), Millis(0));
        composer
    };
    (make(), make())
}

fn relabel(composer: &mut Composer, lines: &[&str], now: Millis) {
    let input = preset::bare(PresentationState::Thriving, Vec::new());
    composer.update(&input, &overlay(lines), now);
}

fn label_draws(composer: &Composer, table: &ArtTable, now: Millis) -> Vec<Draw> {
    of(composer, table, 0, now, Subject::Label)
        .iter()
        .map(|keyed| keyed.draw)
        .collect()
}

#[test]
fn a_changed_hidden_line_swaps_without_moving_anything() {
    let table = table();
    let (mut changed, same) = label_pair(&table, &["moss", "settling 3 days"]);
    let at = Millis(5_000);
    relabel(&mut changed, &["moss", "settling 2 days"], at);
    for ms in [0, 750, 1_500, 10_000] {
        let now = at.plus(ms);
        assert_eq!(
            label_draws(&changed, &table, now),
            label_draws(&same, &table, now),
            "{now:?}"
        );
    }
    // Its turn shows the new text.
    let turn = Millis(u64::from(LABEL_CYCLE_MS + LABEL_FADE_MS));
    assert_ne!(
        label_draws(&changed, &table, turn),
        label_draws(&same, &table, turn)
    );
}

#[test]
fn an_unchanged_visible_line_never_dims_through_a_set_change() {
    let table = table();
    let (mut changed, same) = label_pair(&table, &["moss"]);
    let at = same.settled_at();
    relabel(&mut changed, &["moss", "history 10%"], at);
    for ms in [0, 750, 1_500] {
        let now = at.plus(ms);
        let draws = label_draws(&changed, &table, now);
        assert_eq!(draws, label_draws(&same, &table, now), "{now:?}");
        assert!(draws.iter().all(|draw| draw.opacity == SHOWN));
    }
    // The note has its turn a cycle later.
    let turn = at.plus(LABEL_CYCLE_MS + LABEL_FADE_MS);
    assert_ne!(
        label_draws(&changed, &table, turn),
        label_draws(&same, &table, turn)
    );
}

#[test]
fn a_changed_visible_line_alone_crossfades_and_the_cycle_keeps_its_phase() {
    let table = table();
    let (mut changed, same) = label_pair(&table, &["moss", "main - clear"]);
    let at = Millis(5_000);
    relabel(&mut changed, &["fern", "main - clear"], at);
    assert_eq!(
        label_draws(&changed, &table, at),
        label_draws(&same, &table, at)
    );
    let mid = label_draws(&changed, &table, at.plus(LABEL_FADE_MS / 2));
    assert!(mid.iter().any(|draw| draw.opacity < SHOWN));
    // The next line comes at the cycle's own moment, not a cycle after the change.
    let next = Millis(u64::from(LABEL_CYCLE_MS + LABEL_FADE_MS));
    assert_eq!(
        label_draws(&changed, &table, next),
        label_draws(&same, &table, next)
    );
}

#[test]
fn a_once_line_swapped_in_silently_still_has_its_turn() {
    let table = table();
    let (mut composer, _) = label_pair(&table, &["moss", "settling 3 days"]);
    let cycle = u64::from(LABEL_CYCLE_MS);
    // At 45 s the first line shows whole; the second swaps in unseen.
    relabel(&mut composer, &["moss", "sim updated"], Millis(45_000));
    assert!(!composer.label_step_done(1, Millis(4 * cycle - 1)));
    assert!(composer.label_step_done(1, Millis(4 * cycle)));
    assert!(composer.label_step_done(0, Millis(4 * cycle)));
}

#[test]
fn a_frame_relabelled_is_the_frame_the_composer_draws_with_that_label() {
    let table = table();
    let art = Art::embedded();
    let settled = |line: &str, clock: LocalClock, dim: bool| {
        let mut composer = Composer::new(&table, preset::SALT);
        let overlay = OverlayInput {
            dim,
            ..OverlayInput::new(vec![line.to_owned()], clock)
        };
        composer.update(&scene("thriving"), &overlay, Millis(0));
        let at = composer
            .settled_at()
            .plus(crate::tween::durations::DIM_DOWN_MS);
        let scenery = composer.scenery(art, &table);
        let mix = composer.palette_mix(at);
        composer.frame(art, &table, &scenery, mix, at)
    };
    for (clock, dim) in [
        (LocalClock::noon(preset::DATE), false),
        (LocalClock::midnight(preset::DATE), false),
        (LocalClock::noon(preset::DATE), true),
    ] {
        let moss = settled("moss", clock, dim);
        let halted = settled("halted", clock, dim);
        let relabelled = crate::text::relabelled(&art.catalog, &moss, "halted").unwrap();
        assert_eq!(
            crate::compositor::composite(art, &relabelled),
            crate::compositor::composite(art, &halted),
            "dim {dim}"
        );
        assert_ne!(
            crate::compositor::composite(art, &moss),
            crate::compositor::composite(art, &halted)
        );
    }
}

#[test]
fn new_label_lines_crossfade_from_the_old_on_the_plate() {
    let table = table();
    let art = Art::embedded();
    let mut composer = Composer::new(&table, preset::SALT);
    let input = preset::bare(PresentationState::Thriving, Vec::new());
    composer.update(&input, &overlay(&["main - clear"]), Millis(0));
    let settled = composer.settled_at();
    let text = |composer: &Composer, now| of(composer, &table, 0, now, Subject::Label);
    let before = text(&composer, settled);
    assert_eq!(before.len(), "main - clear".len());
    let plate = world::label_area(&art.catalog).unwrap();
    for keyed in &before {
        let draw = keyed.draw;
        let rect = art.catalog.sprite(draw.sprite).rect;
        assert!(draw.x >= plate.x && draw.x + i32::from(rect.w) <= plate.x + plate.w);
        assert!(draw.y >= plate.y && draw.y + i32::from(rect.h) <= plate.y + plate.h);
        assert_eq!(draw.layer, Layer::Label);
    }
    composer.update(&input, &overlay(&["feature/a-very-long-branch"]), settled);
    let draws = |keyed: &[KeyedDraw]| keyed.iter().map(|k| k.draw).collect::<Vec<_>>();
    assert_eq!(draws(&text(&composer, settled)), draws(&before));
    let mid = text(&composer, settled.plus(LABEL_FADE_MS / 2));
    assert!(mid.iter().any(|keyed| keyed.key.look == 0));
    assert!(mid.iter().any(|keyed| keyed.key.look == 1));
    let after = text(&composer, settled.plus(LABEL_FADE_MS));
    // Fitted to the plate: 17 characters and a tilde.
    assert_eq!(after.len(), 18);
    assert!(
        after
            .iter()
            .all(|keyed| keyed.key.look == 1 && keyed.draw.opacity == SHOWN)
    );
}

#[test]
fn the_palette_follows_the_clock_and_names_each_of_its_steps() {
    let table = table();
    let mut composer = Composer::new(&table, preset::SALT);
    let input = preset::bare(PresentationState::Thriving, Vec::new());
    let evening = OverlayInput::new(Vec::new(), LocalClock::new(preset::DATE, 19 * 3_600_000));
    composer.update(&input, &evening, Millis(0));
    assert_eq!(composer.palette_mix(Millis(0)), 0);
    // Carried forward on the composer's clock: night 45 minutes into the evening.
    assert_eq!(composer.palette_mix(Millis(45 * 60_000)), u8::MAX);
    let mut now = Millis(0);
    let mut steps = 0;
    while let Some(next) = composer.next_change(Art::embedded(), &table, now) {
        if next > Millis(46 * 60_000) {
            break;
        }
        // The scene fading in names its own changes too; the palette holds between them.
        let mix = composer.palette_mix(now);
        assert_eq!(composer.palette_mix(Millis(next.0 - 1)), mix, "{next:?}");
        steps += usize::from(composer.palette_mix(next) != mix);
        now = next;
    }
    assert_eq!(steps, 255, "every step of the mix is named");
}

#[test]
fn a_clock_that_disagrees_eases_the_palette_over() {
    let table = table();
    let mut composer = Composer::new(&table, preset::SALT);
    let input = preset::bare(PresentationState::Thriving, Vec::new());
    let at = |ms| OverlayInput::new(Vec::new(), LocalClock::new(preset::DATE, ms));
    composer.update(&input, &at(12 * 3_600_000), Millis(0));
    // Woken at midnight: the palette eases to night rather than jumping.
    let woke = Millis(5000);
    composer.update(&input, &at(0), woke);
    assert_eq!(composer.palette_mix(woke), 0);
    let mut last = 0;
    for ms in (0..=CLOCK_FIX_MS).step_by(10) {
        let mix = composer.palette_mix(woke.plus(ms));
        assert!(mix >= last && mix - last <= 2, "{ms}: {last} → {mix}");
        last = mix;
    }
    assert_eq!(last, u8::MAX);
    assert_eq!(composer.settled_at(), woke.plus(CLOCK_FIX_MS));
}

#[test]
fn dim_mode_settles_in_slowly_and_lifts_quickly() {
    let table = table();
    let mut composer = Composer::new(&table, preset::SALT);
    let input = preset::bare(PresentationState::Thriving, Vec::new());
    let noon = OverlayInput::new(Vec::new(), LocalClock::noon(preset::DATE));
    let dimmed = OverlayInput {
        dim: true,
        ..noon.clone()
    };
    composer.update(&input, &dimmed, Millis(0));
    assert_eq!(composer.dim(Millis(0)), 0);
    assert_eq!(composer.dim(Millis(u64::from(DIM_DOWN_MS))), DIM);
    let scenery = composer.scenery(Art::embedded(), &table);
    let frame = composer.frame(Art::embedded(), &table, &scenery, 0, Millis(20_000));
    assert_eq!(frame.dim, DIM);
    composer.update(&input, &noon, Millis(20_000));
    assert_eq!(composer.dim(Millis(20_000)), DIM);
    assert_eq!(composer.dim(Millis(20_000 + u64::from(DIM_UP_MS))), 0);
}

/// The plants whose foliage leans at `now`, and every leaning draw's part.
fn leaning(composer: &Composer, table: &ArtTable, now: Millis) -> (Vec<Subject>, Vec<u8>) {
    let draws = composer.draws(Art::embedded(), table, 0, now);
    let leaning: Vec<&KeyedDraw> = draws
        .iter()
        .filter(|keyed| keyed.draw.shear != Shear::NONE)
        .collect();
    let mut subjects: Vec<Subject> = leaning.iter().map(|keyed| keyed.key.subject).collect();
    subjects.dedup();
    (
        subjects,
        leaning.iter().map(|keyed| keyed.key.part).collect(),
    )
}

#[test]
fn a_gust_turns_ahead_is_named_however_long_the_frame_holds_before_it() {
    use crate::motion::TURN_MS;
    // One swaying fern under a storm: no clouds, sun or residents, and one label line, so
    // after a gust the frame holds for turns, until the plant's next gust.
    let art = Art::embedded();
    let table = table();
    let fern = PlantView {
        id: 1,
        species: PlantSpecies::Fern,
        stage: GrowthStage::Young,
        state: PlantState::Thriving,
        bloom: false,
        scorched: false,
        order: 0,
    };
    let mut scene = preset::bare(PresentationState::Thriving, vec![fern]);
    scene.weather = Weather::Storm;
    scene.wind = Wind::Breezy;
    for dim in [false, true] {
        let overlay = OverlayInput {
            dim,
            ..overlay(&["main"])
        };
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&scene, &overlay, Millis(0));
        let scenery = composer.scenery(art, &table);
        let frame = |now: Millis| {
            let draws = composer.frame(art, &table, &scenery, composer.palette_mix(now), now);
            draws.draws
        };
        let mut moment = composer.settled_at();
        let mut longest = 0;
        // Past the longest wait between one cell's gusts, in either mode.
        while moment.0 < turn_start(MOTION_AHEAD + 2).0 {
            let next = composer.next_change(art, &table, moment).unwrap();
            let held = frame(moment);
            // A gust holds each pose at least 300 ms, so 100 ms steps see every one.
            for inside in (moment.0 + 1..next.0).step_by(100) {
                assert!(
                    frame(Millis(inside)) == held,
                    "changed at {inside} before {next:?}"
                );
            }
            longest = longest.max(next.0 - moment.0);
            moment = next;
        }
        assert!(
            longest > 2 * TURN_MS,
            "the frame never held for long: {longest} ms"
        );
    }
}

#[test]
fn wind_sways_at_most_six_plants_their_foliage_and_blooms_only() {
    let table = table();
    let mut thriving = scene("thriving");
    for (wind, most) in [(Wind::Breezy, 1), (Wind::Gale, 2)] {
        thriving.wind = wind;
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&thriving, &overlay(&[]), Millis(0));
        assert!(
            leaning(&composer, &table, Millis(20_000)).0.is_empty(),
            "the first turn is still"
        );
        let mut seen = BTreeSet::new();
        // Every cell gusts once in five turns.
        for ms in (24_000..144_000).step_by(100) {
            let (subjects, parts) = leaning(&composer, &table, Millis(ms));
            assert!(subjects.len() <= sway::MOST_SWAYING);
            assert!(
                parts.iter().all(|&part| part == 1 || part == 2),
                "{parts:?}"
            );
            for keyed in composer.draws(Art::embedded(), &table, 0, Millis(ms)) {
                let top = keyed.draw.shear.offset(0);
                if keyed.key.part == 1 {
                    assert!(top.abs() <= most, "{top}");
                }
            }
            seen.extend(subjects);
        }
        let living: BTreeSet<Subject> = composer
            .placement()
            .plants
            .iter()
            .filter(|(_, placed)| sways(placed.view.stage, placed.view.state))
            .map(|(&id, placed)| Subject::Plant {
                id,
                cell: placed.cell,
            })
            .collect();
        assert!(living.len() > sway::MOST_SWAYING);
        assert_eq!(seen, living, "every living plant takes its turn");
    }
    thriving.wind = Wind::Calm;
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&thriving, &overlay(&[]), Millis(0));
    for ms in (24_000..72_000).step_by(100) {
        assert!(leaning(&composer, &table, Millis(ms)).0.is_empty());
    }
}

#[test]
fn a_wind_change_waits_for_the_next_turn_and_dim_quarters_the_gusts() {
    let table = table();
    let mut scene = scene("thriving");
    scene.wind = Wind::Gale;
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&scene, &overlay(&[]), Millis(0));
    // Calm arrives mid-turn: the gusts of the running turn play out.
    let moving = (24_000..48_000).find(|&ms| !leaning(&composer, &table, Millis(ms)).0.is_empty());
    let moving = Millis(moving.expect("a gust in the first turn that moves"));
    let calm = SceneInput {
        wind: Wind::Calm,
        ..scene.clone()
    };
    composer.update(&calm, &overlay(&[]), moving);
    assert!(!leaning(&composer, &table, moving).0.is_empty());
    assert!((48_000..96_000).all(|ms| leaning(&composer, &table, Millis(ms)).0.is_empty()));
    // Dimmed, only every fourth turn gusts.
    let mut composer = Composer::new(&table, preset::SALT);
    let dimmed = OverlayInput {
        dim: true,
        ..overlay(&[])
    };
    composer.update(&scene, &dimmed, Millis(0));
    for w in 1..9_u64 {
        let gusting = (0..24_000).step_by(100).any(|ms| {
            !leaning(&composer, &table, Millis(w * 24_000 + ms))
                .0
                .is_empty()
        });
        assert_eq!(gusting, w % 4 == 0, "turn {w}");
    }
}

use terrarium_truth::scene::Energy;

fn resident(id: u32, species: ResidentSpecies, stage: LifeStage) -> ResidentView {
    ResidentView {
        id,
        species,
        stage,
        energy: Energy::High,
    }
}

/// What a resident's draw shows: its art's life stage and routine, and where it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    stage: String,
    routine: String,
    x: i32,
}

impl Seen {
    fn name(&self) -> String {
        format!("{}.{}", self.stage, self.routine)
    }
}

/// Each resident's draws at `now`, by id, in draw order: the top look last.
fn creatures(
    composer: &Composer,
    table: &ArtTable,
    mix: u8,
    now: Millis,
) -> BTreeMap<u32, Vec<Seen>> {
    let catalog = &Art::embedded().catalog;
    let mut creatures: BTreeMap<u32, Vec<Seen>> = BTreeMap::new();
    for keyed in composer.draws(Art::embedded(), table, mix, now) {
        if let Subject::Resident { id, .. } = keyed.key.subject
            && keyed.key.part == 0
        {
            let keys = &catalog.sprite(keyed.draw.sprite).keys;
            creatures.entry(id).or_default().push(Seen {
                stage: keys.life_stage.clone().unwrap_or_default(),
                routine: keys.state.clone().unwrap_or_default(),
                x: keyed.key.x,
            });
        }
    }
    creatures
}

/// The residents walking, flying, hopping or signalling at `now`, and of those, in the air
/// or signalling.
fn moving(composer: &Composer, table: &ArtTable, mix: u8, now: Millis) -> (usize, usize) {
    let creatures = creatures(composer, table, mix, now);
    let doing = |names: &[&str]| {
        creatures
            .values()
            .filter(|looks| {
                looks
                    .iter()
                    .any(|look| names.contains(&look.routine.as_str()))
            })
            .count()
    };
    (
        doing(&["move", "takeoff", "flight", "land", "hop", "signal"]),
        doing(&["takeoff", "flight", "land", "hop", "signal"]),
    )
}

/// The top look of each resident at every step of `range`, each change of it once.
fn story(
    composer: &Composer,
    table: &ArtTable,
    mix: u8,
    range: std::ops::Range<u64>,
) -> BTreeMap<u32, Vec<String>> {
    let mut seen: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for ms in range.step_by(50) {
        for (id, looks) in creatures(composer, table, mix, Millis(ms)) {
            let Some(top) = looks.last().map(Seen::name) else {
                continue;
            };
            let names = seen.entry(id).or_default();
            if names.last() != Some(&top) {
                names.push(top);
            }
        }
    }
    seen
}

fn six() -> Vec<ResidentView> {
    use ResidentSpecies::{Beetle, Firefly, Harvestman, Ladybird, Snail, Springtail};
    [Snail, Ladybird, Beetle, Firefly, Springtail, Harvestman]
        .into_iter()
        .zip(1..)
        .map(|(species, id)| resident(id, species, LifeStage::Adult))
        .collect()
}

#[test]
fn at_most_three_residents_move_and_one_flies_and_each_rests_at_home() {
    let table = table();
    for (clock, mix) in [
        (LocalClock::noon(preset::DATE), 0),
        (LocalClock::midnight(preset::DATE), 255),
    ] {
        let mut scene = scene("thriving");
        scene.residents = six();
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&scene, &OverlayInput::new(Vec::new(), clock), Millis(0));
        let (mut most, mut flew, mut walked) = (0, false, BTreeSet::new());
        for ms in (24_000..240_000).step_by(83) {
            let (moving, flying) = moving(&composer, &table, mix, Millis(ms));
            assert!(moving <= resident::MOST_MOVING, "{moving} moving at {ms}");
            assert!(flying <= resident::MOST_FLYING, "{flying} flying at {ms}");
            most = most.max(moving);
            flew |= flying > 0;
            for (id, looks) in creatures(&composer, &table, mix, Millis(ms)) {
                if looks.iter().any(|look| look.routine == "move") {
                    walked.insert(id);
                }
            }
        }
        assert_eq!(most, resident::MOST_MOVING, "three move together at times");
        assert!(walked.len() >= 3, "{walked:?}");
        // By night the firefly takes to the air or signals; by day nothing flies but a
        // springtail's hop.
        if mix == 255 {
            assert!(flew);
        }
        // At every turn's start each resident stands at home.
        let homes = creatures(&composer, &table, mix, Millis(24_000));
        for w in 2..10 {
            let at = creatures(&composer, &table, mix, Millis(w * 24_000));
            for (id, looks) in &at {
                assert_eq!(
                    looks.last().map(|look| look.x),
                    homes[id].last().map(|look| look.x),
                    "{id} in turn {w}"
                );
            }
        }
    }
}

#[test]
fn a_change_of_energy_alone_settles_with_the_tweens() {
    let table = table();
    let mut scene = scene("thriving");
    scene.residents = six();
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&scene, &overlay(&[]), Millis(0));
    let settled = composer.settled_at();
    for resident in &mut scene.residents {
        resident.energy = Energy::Low;
    }
    composer.update(&scene, &overlay(&[]), Millis(30_000));
    assert_eq!(composer.settled_at(), settled, "nothing new to settle");
    // A step of stage plays at the next turn, and settling waits for it.
    scene.residents[0].stage = LifeStage::Dying;
    composer.update(&scene, &overlay(&[]), Millis(31_000));
    assert!(composer.settled_at() > Millis(48_000));
}

#[test]
fn a_newcomer_is_drawn_though_one_leaving_still_fades() {
    let table = table();
    let mut scene = scene("thriving");
    scene.residents = six();
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&scene, &overlay(&[]), Millis(0));
    // One leaves as another arrives: six in the scene, a seventh still fading.
    scene.residents.pop();
    scene
        .residents
        .push(resident(9, ResidentSpecies::Woodlouse, LifeStage::Adult));
    composer.update(&scene, &overlay(&[]), Millis(30_000));
    let shown: BTreeSet<u32> = composer
        .depicted(Art::embedded(), &table, Millis(30_600))
        .iter()
        .filter_map(|d| match d.subject {
            Subject::Resident { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    assert!(shown.contains(&9), "the newcomer fades in: {shown:?}");
    assert!(shown.contains(&6), "the one leaving fades out");
}

#[test]
fn a_resident_faded_out_wakes_the_strip_no_more() {
    let table = table();
    let art = Art::embedded();
    let mut scene = scene("thriving");
    scene.residents = vec![resident(1, ResidentSpecies::Snail, LifeStage::Adult)];
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&scene, &overlay(&[]), Millis(0));
    scene.residents.clear();
    composer.update(&scene, &overlay(&[]), Millis(30_000));
    // Once its fade is over, every moment named changes what is drawn.
    let mut now = Millis(30_000 + u64::from(resident::TRANSITION_MS));
    for _ in 0..60 {
        let Some(next) = composer.next_change(art, &table, now) else {
            break;
        };
        assert_ne!(
            visible_at(&composer, &table, 0, Millis(next.0 - 1)),
            visible_at(&composer, &table, 0, next),
            "{next:?} changes nothing"
        );
        now = next;
    }
}

#[test]
fn of_many_fireflies_and_springtails_one_at_most_takes_to_the_air() {
    let table = table();
    let mut scene = scene("thriving");
    scene.residents = [
        ResidentSpecies::Firefly,
        ResidentSpecies::Firefly,
        ResidentSpecies::Springtail,
        ResidentSpecies::Firefly,
    ]
    .into_iter()
    .zip(1..)
    .map(|(species, id)| resident(id, species, LifeStage::Adult))
    .collect();
    let night = OverlayInput::new(Vec::new(), LocalClock::midnight(preset::DATE));
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&scene, &night, Millis(0));
    let mut flew = 0;
    for ms in (24_000..480_000).step_by(83) {
        let (_, flying) = moving(&composer, &table, 255, Millis(ms));
        assert!(
            flying <= resident::MOST_FLYING,
            "{flying} in the air at {ms}"
        );
        flew += flying;
    }
    assert!(flew > 0);
}

#[test]
fn a_resident_hatches_ecloses_and_dies_through_its_own_art() {
    let table = table();
    let mut scene = scene("thriving");
    scene.residents = vec![
        resident(1, ResidentSpecies::Snail, LifeStage::Egg),
        resident(2, ResidentSpecies::Ladybird, LifeStage::Pupa),
        resident(3, ResidentSpecies::Beetle, LifeStage::Adult),
    ];
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&scene, &overlay(&[]), Millis(0));
    let at = |composer: &Composer, ms| creatures(composer, &table, 0, Millis(ms));
    assert_eq!(at(&composer, 30_000)[&1][0].name(), "egg.egg");
    assert_eq!(at(&composer, 30_000)[&2][0].name(), "pupa.pupa");
    // The next update steps each one on; the step plays at the next turn.
    scene.residents = vec![
        resident(1, ResidentSpecies::Snail, LifeStage::Juvenile),
        resident(2, ResidentSpecies::Ladybird, LifeStage::Adult),
        resident(3, ResidentSpecies::Beetle, LifeStage::Dying),
    ];
    composer.update(&scene, &overlay(&[]), Millis(30_000));
    assert_eq!(
        at(&composer, 47_000)[&1][0].name(),
        "egg.egg",
        "the running turn holds"
    );
    // The snail feeds at a plant's foot: hatching hands it to its idle, and the feed rises
    // over that.
    let seen = story(&composer, &table, 0, 48_000..72_000);
    assert_eq!(
        seen[&1],
        ["egg.egg", "egg.hatch", "juvenile.idle", "juvenile.feed"].map(String::from)
    );
    assert_eq!(
        seen[&2][..2],
        ["pupa.pupa", "pupa.eclose"].map(String::from)
    );
    assert!(seen[&2][2].starts_with("adult."), "{:?}", seen[&2]);
    assert_eq!(seen[&3].last().map(String::as_str), Some("adult.death"));
    let at_hatch = (48_000..72_000)
        .step_by(50)
        .flat_map(|ms| at(&composer, ms).remove(&1).unwrap_or_default())
        .map(|look| look.stage);
    assert!(
        at_hatch.into_iter().all(|stage| stage != "adult"),
        "juveniles never use adult art"
    );
    // Death holds its last frame until truth removes the resident.
    let held = at(&composer, 200_000)[&3].clone();
    assert_eq!(held, at(&composer, 300_000)[&3]);
    assert!(
        composer.settled_at() >= Millis(48_000),
        "the steps are part of settling"
    );
}

#[test]
fn a_storm_sends_adults_into_hiding_and_juveniles_still() {
    let table = table();
    let mut scene = scene("thriving");
    scene.residents = vec![
        resident(1, ResidentSpecies::Snail, LifeStage::Adult),
        resident(2, ResidentSpecies::Woodlouse, LifeStage::Adult),
        resident(3, ResidentSpecies::Ladybird, LifeStage::Adult),
        resident(4, ResidentSpecies::Beetle, LifeStage::Juvenile),
    ];
    let night = OverlayInput::new(Vec::new(), LocalClock::midnight(preset::DATE));
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&scene, &night, Millis(0));
    scene.weather = Weather::Storm;
    composer.update(&scene, &night, Millis(30_000));
    let seen = story(&composer, &table, 255, 48_000..96_000);
    // The snail feeds at a plant's foot, and draws in from there.
    assert_eq!(
        seen[&1][1..],
        ["adult.withdraw", "adult.hide"].map(String::from)
    );
    assert_eq!(
        seen[&2][1..],
        ["adult.curl", "adult.hide"].map(String::from)
    );
    // The ladybird is about at midnight too: it stops feeding and hides.
    assert_eq!(seen[&3], ["adult.feed", "adult.hide"].map(String::from));
    // The young beetle stops feeding and shelters on its idle's first frame, never moving.
    assert_eq!(seen[&4].last().map(String::as_str), Some("juvenile.idle"));
    assert!(!seen[&4].contains(&"juvenile.move".to_owned()));
    let still = creatures(&composer, &table, 255, Millis(80_000))[&4].clone();
    assert_eq!(still, creatures(&composer, &table, 255, Millis(90_000))[&4]);
}

#[test]
fn a_firefly_asleep_by_day_covers_its_sleep_with_its_hide_in_a_storm() {
    let table = table();
    let mut scene = scene("thriving");
    scene.residents = vec![resident(5, ResidentSpecies::Firefly, LifeStage::Adult)];
    let noon = OverlayInput::new(Vec::new(), LocalClock::noon(preset::DATE));
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&scene, &noon, Millis(0));
    scene.weather = Weather::Storm;
    composer.update(&scene, &noon, Millis(30_000));
    let seen = story(&composer, &table, 0, 48_000..96_000);
    assert_eq!(seen[&5], ["adult.sleep", "adult.hide"].map(String::from));
}

#[test]
fn a_firefly_signals_by_night_with_its_lantern_and_never_by_day() {
    let table = table();
    let catalog = &Art::embedded().catalog;
    let mut scene = scene("thriving");
    scene.residents = vec![resident(7, ResidentSpecies::Firefly, LifeStage::Adult)];
    let lanterns = |clock: LocalClock, mix: u8| {
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&scene, &OverlayInput::new(Vec::new(), clock), Millis(0));
        (24_000..600_000)
            .step_by(100)
            .filter(|&ms| {
                composer
                    .draws(Art::embedded(), &table, mix, Millis(ms))
                    .iter()
                    .any(|keyed| {
                        keyed.key.part == 1
                            && catalog.sprite(keyed.draw.sprite).keys.state.as_deref()
                                == Some("signal-glow")
                    })
            })
            .count()
    };
    assert!(lanterns(LocalClock::midnight(preset::DATE), 255) > 0);
    assert_eq!(lanterns(LocalClock::noon(preset::DATE), 255), 0);
}

#[test]
fn the_moon_shows_its_phase_and_new_moon_shows_none() {
    let table = table();
    let mut composer = Composer::new(&table, preset::SALT);
    let input = preset::bare(PresentationState::Thriving, Vec::new());
    let at = |composer: &Composer| of(composer, &table, 255, composer.settled_at(), Subject::Moon);
    let date = |date| OverlayInput::new(Vec::new(), LocalClock::noon(date));
    composer.update(&input, &date(sky::LocalDate::new(2026, 3, 19)), Millis(0));
    assert!(at(&composer).is_empty());
    composer.update(
        &input,
        &date(sky::LocalDate::new(2026, 3, 15)),
        composer.settled_at(),
    );
    let waning = at(&composer);
    assert_eq!(waning.len(), 1);
    assert!(waning[0].draw.flip_x);
    assert_eq!(
        Some(waning[0].draw.sprite),
        Art::embedded().catalog.id("tmp.sky.moon.phase.3")
    );
}

#[test]
fn the_scene_clock_runs_at_its_rate_and_maps_back_to_the_first_moment_it_reaches() {
    let mut clock = SceneClock::default();
    assert_eq!(clock.scene(Millis(700)), Millis(700));
    assert_eq!(clock.real(Millis(700)), Some(Millis(700)));
    // A time-lapse from 1000: the scene's moment there stays, and runs eight times faster.
    clock.set(false, true, Millis(1000));
    assert_eq!(clock.scene(Millis(1000)), Millis(1000));
    assert_eq!(clock.scene(Millis(1001)), Millis(1008));
    assert_eq!(clock.real(Millis(1008)), Some(Millis(1001)));
    // A scene's moment between two of the composer's is first reached at the later one.
    assert_eq!(clock.real(Millis(1009)), Some(Millis(1002)));
    assert_eq!(clock.scene(Millis(1002)), Millis(1016));
    // Before its anchor the clock runs at the composer's pace.
    assert_eq!(clock.scene(Millis(400)), Millis(400));
    assert_eq!(clock.real(Millis(400)), Some(Millis(400)));
    // Setting the same rate again moves nothing.
    let before = clock;
    clock.set(false, true, Millis(5000));
    assert_eq!(clock, before);
    // A pause holds the scene and reaches no moment; a resume carries on from there.
    clock.set(true, true, Millis(1100));
    assert_eq!(clock.scene(Millis(9000)), Millis(1800));
    assert_eq!(clock.real(Millis(1800)), None);
    clock.set(false, true, Millis(2100));
    assert_eq!(clock.scene(Millis(2100)), Millis(1800));
    assert_eq!(clock.scene(Millis(2110)), Millis(1880));
    assert_eq!(clock.real(Millis(1880)), Some(Millis(2110)));
    // Nothing overflows at the end of time.
    assert_eq!(clock.scene(Millis(u64::MAX)), Millis(u64::MAX));
    let last = clock.real(Millis(u64::MAX)).unwrap();
    assert_eq!(clock.scene(last), Millis(u64::MAX));
}

#[test]
fn a_pause_at_the_composer_s_pace_maps_as_it_always_has() {
    let mut clock = SceneClock::default();
    clock.set(true, false, Millis(1000));
    assert_eq!(clock.scene(Millis(1500)), Millis(1000));
    assert_eq!(clock.scene(Millis(600)), Millis(600));
    clock.set(false, false, Millis(3000));
    // Held 2 s: the scene is 2 s behind, before the resume as well as after it.
    for ms in [0, 1500, 2500, 3000, 7000] {
        assert_eq!(
            clock.scene(Millis(ms)),
            Millis(ms.saturating_sub(2000)),
            "{ms}"
        );
    }
    for ms in [0, 500, 1000, 5000] {
        assert_eq!(clock.real(Millis(ms)), Some(Millis(ms + 2000)), "{ms}");
    }
}

#[test]
fn a_time_lapse_draws_what_the_composer_draws_at_eight_times_the_moment() {
    let table = table();
    let art = Art::embedded();
    let lapse = OverlayInput {
        time_lapse: true,
        ..overlay(&[])
    };
    let mut fast = Composer::new(&table, preset::SALT);
    let mut slow = Composer::new(&table, preset::SALT);
    fast.update(&scene("thriving"), &lapse, Millis(1000));
    slow.update(&scene("thriving"), &overlay(&[]), Millis(1000));
    let pace = u64::from(TIME_LAPSE_PACE);
    let scaled = |ms: u64| Millis(1000 + (ms - 1000) * pace);
    fast.update(&scene("struggling-poor-soil"), &lapse, Millis(1500));
    slow.update(&scene("struggling-poor-soil"), &overlay(&[]), scaled(1500));
    for ms in [1000, 1001, 1250, 1500, 1501, 1600, 1875, 2000, 2400] {
        assert_eq!(
            fast.draws(art, &table, 0, Millis(ms)),
            slow.draws(art, &table, 0, scaled(ms)),
            "{ms}"
        );
        assert_eq!(fast.palette_mix(Millis(ms)), slow.palette_mix(scaled(ms)));
    }
    // It settles when the scene reaches the moment the slow one settles.
    let settled = slow.settled_at();
    assert!(settled > scaled(1500));
    assert_eq!(
        fast.settled_at(),
        Millis(1000 + (settled.0 - 1000).div_ceil(pace))
    );
    // Each change it names is the first moment the scene reaches the slow one's.
    let mut now = Millis(1500);
    for _ in 0..50 {
        let Some(next) = fast.next_change(art, &table, now) else {
            break;
        };
        assert!(next > now);
        let scene = slow.next_change(art, &table, scaled(now.0)).unwrap();
        assert_eq!(next, Millis(1000 + (scene.0 - 1000).div_ceil(pace)));
        now = next;
    }
}

#[test]
fn a_time_lapse_label_crossfades_at_its_pace() {
    let table = table();
    let mut composer = Composer::new(&table, preset::SALT);
    let input = preset::bare(PresentationState::Thriving, Vec::new());
    let lapse = |line: &str| OverlayInput {
        time_lapse: true,
        ..overlay(&[line])
    };
    composer.update(&input, &lapse("mar 2024"), Millis(0));
    let settled = composer.settled_at();
    composer.update(&input, &lapse("apr 2024"), settled);
    let fade = LABEL_FADE_MS.div_ceil(TIME_LAPSE_PACE);
    assert_eq!(fade, 188);
    let looks = |now: Millis| {
        of(&composer, &table, 0, now, Subject::Label)
            .iter()
            .map(|keyed| keyed.key.look)
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(looks(settled.plus(fade / 2)), BTreeSet::from([0, 1]));
    assert_eq!(looks(settled.plus(fade)), BTreeSet::from([1]));
    assert_eq!(composer.settled_at(), settled.plus(fade));
}
