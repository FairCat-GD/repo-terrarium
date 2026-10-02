//! The frame scheduler over a simulated hour (I-10, N-03): the app draws only when the
//! composer's picture changes, never more than 15 times a second, and a quiet scene changes
//! only by the slow motions that keep an OLED strip from holding still, which reach every 8×8
//! block of it within the hour (N-12). The rules are written out here, never taken from the
//! crate.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use terrarium_compose::catalog::Art;
use terrarium_compose::clock::LocalClock;
use terrarium_compose::composer::{Composer, OverlayInput};
use terrarium_compose::compositor::composite;
use terrarium_compose::frame::{Draw, NATIVE_HEIGHT, Viewport};
use terrarium_compose::order::{KeyedDraw, Subject};
use terrarium_compose::preset;
use terrarium_compose::schedule::Pacer;
use terrarium_compose::tween::Millis;
use terrarium_compose::variants::ArtTable;
use terrarium_truth::PresentationState;
use terrarium_truth::scene::{Energy, LifeStage, ResidentSpecies, ResidentView, Weather};

const HOUR_MS: u64 = 3_600_000;
/// The soil's texture moves every 3 minutes; the backdrop holds still.
const SOIL_MS: u64 = 180_000;
/// Dim mode darkens the frame over 10 s.
const DIM_DOWN_MS: u32 = 10_000;
/// The blocks N-12 counts, and how many fit down the frame.
const BLOCK: u32 = 8;
const BLOCKS_DOWN: u32 = NATIVE_HEIGHT.div_ceil(BLOCK);
/// A sweep comes once in each 25-minute slot but the first, and crosses the scene in 664
/// steps: in at x = −23, a pixel at a time to 639, then out.
const SWEEP_SLOT_MS: u64 = 1_500_000;
const SWEEP_STEPS: usize = 664;
/// At most 15 frames in any second.
const MOST_PER_SECOND: usize = 15;

/// Each subject's draws, by subject.
fn by_subject(draws: &[KeyedDraw]) -> BTreeMap<Subject, Vec<Draw>> {
    let mut map: BTreeMap<Subject, Vec<Draw>> = BTreeMap::new();
    for keyed in draws {
        map.entry(keyed.key.subject).or_default().push(keyed.draw);
    }
    map
}

/// The subjects whose draws differ between two frames.
fn changed(before: &[KeyedDraw], after: &[KeyedDraw]) -> BTreeSet<Subject> {
    let (a, b) = (by_subject(before), by_subject(after));
    a.keys()
        .chain(b.keys())
        .filter(|subject| a.get(subject) != b.get(subject))
        .copied()
        .collect()
}

#[test]
fn a_quiet_hour_draws_only_its_slow_motions() {
    let art = Art::embedded();
    let table = ArtTable::new(&art.catalog).expect("the embedded art is complete");
    // A storm brings no clouds, hides the sun and moon, and sends adults into hiding; still
    // air sways nothing; one label line never cycles; and the residents hold still: an egg,
    // a pupa and a woodlouse curled up in its hide.
    let mut scene = preset::bare(PresentationState::Mature, Vec::new());
    scene.weather = Weather::Storm;
    let held = |id, species, stage| ResidentView {
        id,
        species,
        stage,
        energy: Energy::High,
    };
    scene.residents = vec![
        held(1, ResidentSpecies::Snail, LifeStage::Egg),
        held(2, ResidentSpecies::Beetle, LifeStage::Pupa),
        held(3, ResidentSpecies::Woodlouse, LifeStage::Adult),
    ];
    let overlay = OverlayInput::new(
        vec!["main - storm".to_owned()],
        LocalClock::noon(preset::DATE),
    );
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&scene, &overlay, Millis(0));
    let settled = composer.settled_at();
    let draws = |now| composer.draws(art, &table, composer.palette_mix(now), now);
    let residents = draws(settled)
        .iter()
        .filter(|keyed| matches!(keyed.key.subject, Subject::Resident { .. }))
        .count();
    assert_eq!(residents, 3, "the residents are drawn");

    let mut pacer = Pacer::default();
    let mut shown = draws(settled);
    pacer.composed(settled);
    let mut now = settled;
    let mut frames: Vec<Millis> = Vec::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    while let Some(wake) = pacer.wake(composer.next_change(art, &table, now)) {
        if wake.0 > HOUR_MS {
            break;
        }
        let next = draws(wake);
        let moved = changed(&shown, &next);
        // Each step of the sweep it wakes for shows, however faded.
        let sweep = next.iter().filter(|k| k.key.subject == Subject::Sweep);
        assert!(
            sweep.clone().all(|k| k.draw.opacity > 0),
            "an unseen sweep at {wake:?}"
        );
        // A frame is drawn only for a change, and only the slow motions change.
        assert!(
            !moved.is_empty(),
            "a frame at {wake:?} with nothing changed"
        );
        for subject in moved {
            let kind = match subject {
                Subject::Soil => "soil",
                Subject::Sweep => "sweep",
                other => panic!("{other:?} changed at {wake:?} in a quiet scene"),
            };
            *counts.entry(kind).or_default() += 1;
        }
        frames.push(wake);
        pacer.composed(wake);
        shown = next;
        now = wake;
    }

    // A drift steps at every multiple of its pace after the settled moment, to the hour.
    let steps = |every: u64| usize::try_from(HOUR_MS / every - settled.0 / every).unwrap();
    // The settled moment lies before the soil's first step.
    assert!(settled.0 < SOIL_MS);
    assert_eq!(counts.get("soil"), Some(&steps(SOIL_MS)));
    // Slots 1 and 2 begin within the hour, and the second sweep is across by 3380 s.
    let sweeps = usize::try_from(HOUR_MS / SWEEP_SLOT_MS).unwrap();
    assert_eq!(counts.get("sweep"), Some(&(sweeps * SWEEP_STEPS)));
    for (i, &frame) in frames.iter().enumerate() {
        let within = frames[i..]
            .iter()
            .take_while(|&&later| later.0 < frame.0 + 1000)
            .count();
        assert!(
            within <= MOST_PER_SECOND,
            "{within} frames in the second from {frame:?}"
        );
    }
    println!("a quiet hour: {} frames, {counts:?}", frames.len());
}

#[test]
fn a_lively_hour_draws_only_changes_and_never_more_than_15_a_second() {
    let art = Art::embedded();
    let table = ArtTable::new(&art.catalog).expect("the embedded art is complete");
    // The thriving preset as the strip shows it: residents about, a breeze, clouds, by day.
    let scene = preset::scene("thriving").expect("a preset");
    let overlay = preset::overlay("thriving", "repo-terrarium").expect("a preset");
    let mut composer = Composer::new(&table, preset::SALT);
    composer.update(&scene, &overlay, Millis(0));
    let draws = |now| composer.draws(art, &table, composer.palette_mix(now), now);
    let mut pacer = Pacer::default();
    let mut shown = draws(Millis(0));
    let mut now = Millis(0);
    let mut frames: Vec<Millis> = Vec::new();
    while let Some(wake) = pacer.wake(composer.next_change(art, &table, now)) {
        if wake.0 > HOUR_MS {
            break;
        }
        let next = draws(wake);
        assert!(
            !changed(&shown, &next).is_empty(),
            "a frame at {wake:?} with nothing changed"
        );
        frames.push(wake);
        pacer.composed(wake);
        shown = next;
        now = wake;
    }
    for (i, &frame) in frames.iter().enumerate() {
        let within = frames[i..]
            .iter()
            .take_while(|&&later| later.0 < frame.0 + 1000)
            .count();
        assert!(
            within <= MOST_PER_SECOND,
            "{within} frames in the second from {frame:?}"
        );
    }
    println!("a lively hour: {} frames", frames.len());
}

#[test]
fn no_region_holds_still_for_an_hour() {
    let art = Art::embedded();
    let table = ArtTable::new(&art.catalog).expect("the embedded art is complete");
    // The quietest scene the strip shows: a dead terrarium under a storm, so no plant,
    // resident, cloud, sun, moon or star moves, and the backdrop holds still. The label has
    // the two lines the app always gives it, so it cycles.
    let mut scene = preset::bare(PresentationState::Dead, Vec::new());
    scene.weather = Weather::Storm;
    let lines = vec!["repo-terrarium".to_owned(), "main - storm".to_owned()];
    // The scene alone, and the 735 columns a 2940 px screen shows at 4×.
    let views = [Viewport::SCENE, Viewport::new(735)];
    let cases = views.into_iter().flat_map(|viewport| {
        [
            LocalClock::noon(preset::DATE),
            LocalClock::midnight(preset::DATE),
        ]
        .into_iter()
        .flat_map(move |clock| [false, true].map(|dim| (viewport, clock, dim)))
    });
    for (viewport, clock, dim) in cases {
        let overlay = OverlayInput {
            dim,
            ..OverlayInput::new(lines.clone(), clock)
        };
        let mut composer = Composer::new(&table, preset::SALT);
        composer.set_viewport(viewport);
        composer.update(&scene, &overlay, Millis(0));
        let scenery = composer.scenery(art, &table);
        let picture = |now: Millis| {
            let mix = composer.palette_mix(now);
            composite(art, &composer.frame(art, &table, &scenery, mix, now))
        };
        // From once the scene, and dim mode, have settled.
        let start = composer.settled_at().plus(DIM_DOWN_MS);
        let across = viewport.width().div_ceil(BLOCK);
        let mut changed = vec![vec![false; across as usize]; BLOCKS_DOWN as usize];
        let mut shown = picture(start);
        let mut pacer = Pacer::default();
        pacer.composed(start);
        let mut now = start;
        while let Some(wake) = pacer.wake(composer.next_change(art, &table, now)) {
            if wake.0 > start.0 + HOUR_MS || changed.iter().flatten().all(|&c| c) {
                break;
            }
            let next = picture(wake);
            for y in 0..NATIVE_HEIGHT {
                let pairs = shown.row(y).iter().zip(next.row(y));
                for (x, _) in (0..viewport.width())
                    .zip(pairs)
                    .filter(|(_, (a, b))| a != b)
                {
                    changed[(y / BLOCK) as usize][(x / BLOCK) as usize] = true;
                }
            }
            pacer.composed(wake);
            shown = next;
            now = wake;
        }
        let still: Vec<(u32, u32)> = (0..BLOCKS_DOWN)
            .flat_map(|row| (0..across).map(move |column| (column, row)))
            .filter(|&(column, row)| !changed[row as usize][column as usize])
            .collect();
        assert!(
            still.is_empty(),
            "{} blocks (column, row) hold still for an hour in {viewport:?} at {clock:?}, dim {dim}: {still:?}",
            still.len()
        );
    }
}
