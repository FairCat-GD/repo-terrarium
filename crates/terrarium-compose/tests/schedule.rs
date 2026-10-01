//! The frame scheduler over a simulated hour (I-10, N-03): the app draws only when the
//! composer's picture changes, never more than 15 times a second, and a quiet scene changes
//! only by the slow motions that keep an OLED strip from holding still. The rules are written
//! out here, never taken from the crate.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use terrarium_compose::catalog::Art;
use terrarium_compose::clock::LocalClock;
use terrarium_compose::composer::{Composer, OverlayInput};
use terrarium_compose::frame::Draw;
use terrarium_compose::order::{KeyedDraw, Subject};
use terrarium_compose::preset;
use terrarium_compose::schedule::Pacer;
use terrarium_compose::tween::Millis;
use terrarium_compose::variants::ArtTable;
use terrarium_truth::PresentationState;
use terrarium_truth::scene::{Energy, LifeStage, ResidentSpecies, ResidentView, Weather};

const HOUR_MS: u64 = 3_600_000;
/// The sky band moves every 12 s, the soil's texture every 3 minutes.
const BAND_MS: u64 = 12_000;
const SOIL_MS: u64 = 180_000;
/// A sweep comes once in each 330 s slot but the first, and crosses in 332 steps: in at
/// x = −22, two pixels at a time to 638, then out.
const SWEEP_SLOT_MS: u64 = 330_000;
const SWEEP_STEPS: usize = 332;
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
        // A frame is drawn only for a change, and only the slow motions change.
        assert!(
            !moved.is_empty(),
            "a frame at {wake:?} with nothing changed"
        );
        for subject in moved {
            let kind = match subject {
                Subject::Weather => "band",
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
    // The settled moment lies before the first step of each drift.
    assert!(settled.0 < BAND_MS);
    assert_eq!(counts.get("band"), Some(&steps(BAND_MS)));
    assert_eq!(counts.get("soil"), Some(&steps(SOIL_MS)));
    // Slots 1 to 10 begin within the hour, and the last sweep is across by 3423 s.
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
