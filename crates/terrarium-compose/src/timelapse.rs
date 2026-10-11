//! The time-lapse: a range of days played in 30 seconds by a composer of its own,
//! whose scene runs at the time-lapse's pace ([`crate::composer::TIME_LAPSE_PACE`]), so the live strip's
//! composer is never touched and every event plays in miniature.
//!
//! Frames come 15 a second, on the GIF's own cadence of 7, 7 and 6 hundredths ([`DELAYS_CS`]),
//! so the composer's clock and the exported file keep the same time. Each frame shows one day
//! of the range ([`Sampling`]): a short range holds each day over several frames, a long one
//! skips days, and the scene a frame shows carries the event truth would play since the frame
//! before.
//!
//! A [`Reel`] composes the frames one at a time, for an encoder to take as they come; a
//! [`Stage`] is the same composer for a window that plays it on the wall clock.

use terrarium_truth::scene::SceneInput;

use crate::catalog::Art;
use crate::clock::LocalClock;
use crate::composer::{Composer, OverlayInput};
use crate::compositor::composite;
use crate::frame::NativeImage;
use crate::salt::InstallSalt;
use crate::tween::Millis;
use crate::variants::ArtTable;
use crate::world::Scenery;

/// How long a time-lapse plays, in seconds.
pub const SECONDS: u32 = 30;
/// The frames of a time-lapse: 15 a second, the manifests' ceiling.
pub const FRAMES: usize = 450;
/// Each frame's delay in hundredths of a second, in turn: three frames every 200 ms, 15 a
/// second, in the whole hundredths a GIF counts in.
pub const DELAYS_CS: [u16; 3] = [7, 7, 6];

/// When frame `i` shows, in milliseconds from the first: the sum of the delays before it.
pub fn frame_ms(i: usize) -> u64 {
    let cycle: u64 = DELAYS_CS.iter().map(|&cs| u64::from(cs) * 10).sum();
    let into: u64 = DELAYS_CS[..i % DELAYS_CS.len()]
        .iter()
        .map(|&cs| u64::from(cs) * 10)
        .sum();
    u64::try_from(i / DELAYS_CS.len()).map_or(u64::MAX, |cycles| cycles * cycle + into)
}

/// The frame showing `ms` milliseconds after the first: the last whose moment has come.
pub fn frame_at(ms: u64) -> usize {
    let cycle: u64 = DELAYS_CS.iter().map(|&cs| u64::from(cs) * 10).sum();
    let mut into = ms % cycle;
    let mut frame = 0;
    for &cs in &DELAYS_CS[..DELAYS_CS.len() - 1] {
        match into.checked_sub(u64::from(cs) * 10) {
            Some(rest) => {
                into = rest;
                frame += 1;
            }
            None => break,
        }
    }
    usize::try_from(ms / cycle).map_or(usize::MAX, |cycles| {
        cycles.saturating_mul(DELAYS_CS.len()).saturating_add(frame)
    })
}

/// Which day each frame of a time-lapse shows: the one home of the sampling, shared by the
/// replay that projects the days and the composer that plays them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sampling {
    /// The days shown, in order, each once: indices into the history's days.
    pub days: Vec<usize>,
    /// For each frame, its day's place in `days`.
    pub frames: Vec<usize>,
}

impl Sampling {
    /// `frames` frames over the days `first..=last`: the first frame shows `first`, the last
    /// shows `last`, and the frames between are spread evenly over the days between.
    ///
    /// # Panics
    /// If `frames` is zero or `last` is before `first`.
    pub fn new(first: usize, last: usize, frames: usize) -> Sampling {
        assert!(frames > 0 && last >= first, "a range of days and frames");
        let span = last - first;
        let mut days: Vec<usize> = Vec::new();
        let mut shown = Vec::with_capacity(frames);
        for i in 0..frames {
            let day = match frames - 1 {
                0 => first,
                steps => first + i * span / steps,
            };
            if days.last() != Some(&day) {
                days.push(day);
            }
            shown.push(days.len() - 1);
        }
        Sampling {
            days,
            frames: shown,
        }
    }
}

/// One day as a time-lapse shows it: truth's scene at the day's close, the label the app
/// words for it, and the local clock it is drawn at, noon by day or midnight by night.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub scene: SceneInput,
    pub label: Vec<String>,
    pub clock: LocalClock,
}

impl Shot {
    /// The overlay a time-lapse draws this shot under: its label and clock, the default day and
    /// night, nothing dimmed or hovered, the scene at the time-lapse's pace.
    fn overlay(&self, label: &[String]) -> OverlayInput {
        OverlayInput {
            time_lapse: true,
            ..OverlayInput::new(label.to_vec(), self.clock)
        }
    }
}

/// A time-lapse's own composer, standing on a shot.
#[derive(Clone)]
pub struct Stage {
    composer: Composer,
    scenery: Scenery,
    showing: usize,
}

impl Stage {
    /// A composer of its own with the salt of the install, settled on the shot `showing` of
    /// `shots`, and the moment it stands settled. A `prime`, the day before the range, is
    /// settled first, under the first shot's label, and the first shot raised over it at that
    /// moment: so the range's first day changes in, its event plays, and the label is already
    /// its own.
    pub fn new(
        art: &Art,
        table: &ArtTable,
        salt: InstallSalt,
        prime: Option<&Shot>,
        shots: &[Shot],
        showing: usize,
    ) -> (Stage, Millis) {
        let mut composer = Composer::new(table, salt);
        let scenery = composer.scenery(art, table);
        let first = &shots[showing];
        let start = if let Some(prime) = prime {
            composer.update(&prime.scene, &prime.overlay(&first.label), Millis(0));
            let settled = composer.settled_at();
            composer.update(&first.scene, &first.overlay(&first.label), settled);
            settled
        } else {
            composer.update(&first.scene, &first.overlay(&first.label), Millis(0));
            composer.settled_at()
        };
        let stage = Stage {
            composer,
            scenery,
            showing,
        };
        (stage, start)
    }

    /// Moves to the shot `showing` of `shots` at `now`, unless it already stands there.
    pub fn show(&mut self, shots: &[Shot], showing: usize, now: Millis) {
        if showing != self.showing {
            self.take(shots, showing, now);
        }
    }

    /// Moves to the shot `showing` of new `shots` at `now`, whatever it stood on: the same
    /// days by night rather than by day, or another range's.
    pub fn take(&mut self, shots: &[Shot], showing: usize, now: Millis) {
        let shot = &shots[showing];
        self.composer
            .update(&shot.scene, &shot.overlay(&shot.label), now);
        self.showing = showing;
    }

    /// The shot the stage stands on.
    pub fn showing(&self) -> usize {
        self.showing
    }

    /// The picture at `now`, in the palette its clock gives.
    pub fn frame(&self, art: &Art, table: &ArtTable, now: Millis) -> NativeImage {
        let mix = self.composer.palette_mix(now);
        composite(
            art,
            &self.composer.frame(art, table, &self.scenery, mix, now),
        )
    }

    /// When the last change the stage was given is over.
    pub fn settled_at(&self) -> Millis {
        self.composer.settled_at()
    }
}

/// A time-lapse's frames, composed one at a time as they are asked for: never more than one
/// is held.
pub struct Reel<'a> {
    art: &'a Art<'a>,
    table: &'a ArtTable,
    shots: &'a [Shot],
    frames: &'a [usize],
    stage: Stage,
    start: Millis,
    next: usize,
}

impl<'a> Reel<'a> {
    /// The frames of `sampling` over its `shots` (one per day it shows, in its order), after
    /// an optional `prime`.
    ///
    /// # Panics
    /// If `shots` does not hold a shot for each of the sampling's days.
    pub fn new(
        art: &'a Art<'a>,
        table: &'a ArtTable,
        salt: InstallSalt,
        prime: Option<&Shot>,
        shots: &'a [Shot],
        sampling: &'a Sampling,
    ) -> Reel<'a> {
        assert_eq!(
            shots.len(),
            sampling.days.len(),
            "a shot for each day shown"
        );
        let (stage, start) = Stage::new(art, table, salt, prime, shots, sampling.frames[0]);
        Reel {
            art,
            table,
            shots,
            frames: &sampling.frames,
            stage,
            start,
            next: 0,
        }
    }

    /// How many frames the reel has.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

impl Iterator for Reel<'_> {
    type Item = NativeImage;

    fn next(&mut self) -> Option<NativeImage> {
        let i = self.next;
        let &showing = self.frames.get(i)?;
        let now = Millis(self.start.0 + frame_ms(i));
        self.stage.show(self.shots, showing, now);
        self.next += 1;
        Some(self.stage.frame(self.art, self.table, now))
    }
}

/// One shot as a still: a composer of its own, settled on it. The postcard.
pub fn still(art: &Art, table: &ArtTable, salt: InstallSalt, shot: &Shot) -> NativeImage {
    let (stage, settled) = Stage::new(art, table, salt, None, std::slice::from_ref(shot), 0);
    stage.frame(art, table, settled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset;

    fn table() -> ArtTable {
        ArtTable::new(&Art::embedded().catalog).unwrap()
    }

    fn shot(name: &str, label: &str) -> Shot {
        Shot {
            scene: preset::scene(name).unwrap(),
            label: vec![label.to_owned()],
            clock: LocalClock::noon(preset::DATE),
        }
    }

    #[test]
    fn frames_keep_the_gif_s_cadence_of_fifteen_a_second() {
        let times: Vec<u64> = (0..7).map(frame_ms).collect();
        assert_eq!(times, [0, 70, 140, 200, 270, 340, 400]);
        assert_eq!(frame_ms(FRAMES), u64::from(SECONDS) * 1000);
        for i in 0..FRAMES {
            assert_eq!(frame_at(frame_ms(i)), i);
            assert_eq!(frame_at(frame_ms(i + 1) - 1), i);
        }
    }

    #[test]
    fn the_sampling_reaches_both_ends_and_holds_or_skips_days_evenly() {
        let long = Sampling::new(100, 961, FRAMES);
        assert_eq!(long.frames.len(), FRAMES);
        assert_eq!(long.days.first(), Some(&100));
        assert_eq!(long.days.last(), Some(&961));
        assert_eq!(long.days.len(), FRAMES);
        assert!(
            long.days
                .windows(2)
                .all(|pair| (1..=2).contains(&(pair[1] - pair[0])))
        );
        let short = Sampling::new(10, 14, FRAMES);
        assert_eq!(short.days, [10, 11, 12, 13, 14]);
        assert_eq!(short.frames[0], 0);
        assert_eq!(short.frames[FRAMES - 1], 4);
        // Each day holds for as many frames as the next, give or take one.
        let held: Vec<usize> = (0..5)
            .map(|day| short.frames.iter().filter(|&&f| f == day).count())
            .collect();
        assert!(held[..4].iter().all(|&n| n == 112 || n == 113), "{held:?}");
        assert_eq!(held[4], 1);
        assert_eq!(Sampling::new(7, 7, 3).days, [7]);
    }

    #[test]
    fn a_reel_is_the_same_frames_every_time_and_starts_settled_on_its_prime() {
        let table = table();
        let art = Art::embedded();
        let shots = [
            shot("thriving", "1 mar 2024"),
            shot("struggling-poor-soil", "2 mar 2024"),
        ];
        let prime = shot("dead", "29 feb 2024");
        let sampling = Sampling {
            days: vec![0, 1],
            frames: vec![0, 0, 0, 1, 1, 1, 1, 1],
        };
        let reel = |prime| -> Vec<NativeImage> {
            Reel::new(art, &table, preset::SALT, prime, &shots, &sampling).collect()
        };
        let frames = reel(Some(&prime));
        assert_eq!(frames.len(), 8);
        assert_eq!(frames, reel(Some(&prime)));
        // The first frame is the prime, settled under the first day's label; the day changes
        // in from there.
        let (stage, start) = Stage::new(
            art,
            &table,
            preset::SALT,
            None,
            &[Shot {
                label: shots[0].label.clone(),
                ..prime.clone()
            }],
            0,
        );
        assert_eq!(frames[0], stage.frame(art, &table, start));
        assert_ne!(frames[2], frames[0]);
        // Without a prime, the first frame is the first day, settled.
        assert_eq!(reel(None)[0], still(art, &table, preset::SALT, &shots[0]));
    }
}
