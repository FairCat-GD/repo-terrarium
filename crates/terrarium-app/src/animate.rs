//! The living strip (I-10): a scene composed on the clock, drawn only when its pixels change
//! and never more than 15 times a second. What it shows, and when that changes, is the feed's
//! business (`feed`); this module only composes.
//!
//! It reads no clock of its own; the strip passes the time in, so it can be driven in tests.

use terrarium_compose::catalog::Art;
use terrarium_compose::composer::{Composer, OverlayInput};
use terrarium_compose::compositor::composite_into;
use terrarium_compose::frame::{NativeImage, Viewport};
use terrarium_compose::preset;
use terrarium_compose::scale::Damage;
use terrarium_compose::schedule::Pacer;
use terrarium_compose::tween::Millis;
use terrarium_compose::variants::ArtTable;
use terrarium_compose::world::Scenery;
use terrarium_truth::scene::SceneInput;

pub struct Living {
    art: &'static Art<'static>,
    table: ArtTable,
    composer: Composer,
    scenery: Scenery,
    pacer: Pacer,
    /// The moment last composed, and its pixels.
    at: Millis,
    image: NativeImage,
    /// A spare image to compose into, so a frame that changes nothing costs no allocation.
    spare: NativeImage,
    /// When it next wants composing, worked out once per composition or update: the loop asks
    /// on every event it handles.
    due: Option<Millis>,
    compositions: u64,
}

impl Living {
    /// `scene` and `overlay`, arriving at `now`.
    ///
    /// # Panics
    /// If the embedded art is incomplete, which `cargo xtask verify` rules out.
    pub fn new(scene: &SceneInput, overlay: &OverlayInput, now: Millis) -> Living {
        let art = Art::embedded();
        let table = ArtTable::new(&art.catalog).expect("the embedded art depicts every variant");
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(scene, overlay, now);
        let scenery = composer.scenery(art, &table);
        let blank = NativeImage::filled([0, 0, 0, u8::MAX]);
        let mut living = Living {
            art,
            table,
            composer,
            scenery,
            pacer: Pacer::default(),
            at: now,
            image: blank.clone(),
            spare: blank,
            due: None,
            compositions: 0,
        };
        living.compose(now);
        living
    }

    /// Moves the strip toward `scene` and `overlay` from `now`. The picture at `now` does not
    /// change, so nothing is composed; only the next moment it will is worked out again.
    pub fn update(&mut self, scene: &SceneInput, overlay: &OverlayInput, now: Millis) {
        self.composer.update(scene, overlay, now);
        self.due = self
            .pacer
            .wake(self.composer.next_change(self.art, &self.table, now));
    }

    /// When the strip should next compose: at the composer's next change, but never sooner
    /// than a fifteenth of a second after it last composed.
    pub fn due(&self) -> Option<Millis> {
        self.due
    }

    /// Brings the strip to `now`; true if its pixels changed. Every call counts against the
    /// pace, so the strip composes at most 15 times a second whatever it shows.
    pub fn advance(&mut self, now: Millis) -> bool {
        self.compose(now)
    }

    pub fn image(&self) -> &NativeImage {
        &self.image
    }

    /// Composes for `viewport` from now on, at once at `now` if it is another; true then, since
    /// the strip's width changed and its whole picture is new.
    pub fn fit(&mut self, viewport: Viewport, now: Millis) -> bool {
        if self.composer.viewport() == viewport {
            return false;
        }
        self.composer.set_viewport(viewport);
        self.scenery = self.composer.scenery(self.art, &self.table);
        self.spare = NativeImage::new(viewport, [0, 0, 0, u8::MAX]);
        self.compose(now);
        self.spare = NativeImage::new(viewport, [0, 0, 0, u8::MAX]);
        true
    }

    /// Compositions since it was made, presented or not.
    #[cfg_attr(not(feature = "dev"), allow(dead_code))]
    pub fn compositions(&self) -> u64 {
        self.compositions
    }

    /// Composes `now`; true if any pixel changed.
    fn compose(&mut self, now: Millis) -> bool {
        let mix = self.composer.palette_mix(now);
        let frame = self
            .composer
            .frame(self.art, &self.table, &self.scenery, mix, now);
        composite_into(self.art, &frame, &mut self.spare);
        self.at = now;
        self.compositions += 1;
        self.pacer.composed(now);
        self.due = self
            .pacer
            .wake(self.composer.next_change(self.art, &self.table, now));
        let changed = Damage::between(&self.image, &self.spare).is_some();
        if changed {
            std::mem::swap(&mut self.image, &mut self.spare);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use terrarium_compose::clock::LocalClock;

    use super::*;
    use crate::copy;

    const HOUR_MS: u64 = 3_600_000;

    fn thriving_at(clock: LocalClock) -> Living {
        let overlay = OverlayInput::new(vec![copy::sample_label().to_owned()], clock);
        Living::new(&preset::scene("thriving").unwrap(), &overlay, Millis(0))
    }

    #[test]
    fn the_strip_composes_and_draws_at_most_15_times_a_second() {
        // Five minutes of the living strip, woken only when it asks to be.
        let mut living = thriving_at(LocalClock::noon(preset::DATE));
        let mut wakes: Vec<Millis> = Vec::new();
        let mut frames = 0;
        while let Some(due) = living.due() {
            if due.0 > 300_000 {
                break;
            }
            wakes.push(due);
            frames += usize::from(living.advance(due));
        }
        for (i, &wake) in wakes.iter().enumerate() {
            let within = wakes[i..]
                .iter()
                .take_while(|&&later| later.0 < wake.0 + 1000)
                .count();
            assert!(within <= 15, "{within} wakes in the second from {wake:?}");
        }
        assert!(frames > 300, "it lives: {frames} frames");
        assert_eq!(
            living.compositions(),
            1 + u64::try_from(wakes.len()).unwrap()
        );
    }

    #[test]
    fn between_changes_nothing_changes() {
        let mut living = thriving_at(LocalClock::noon(preset::DATE));
        for _ in 0..200 {
            let due = living.due().expect("the strip lives");
            let change = living
                .composer
                .next_change(living.art, &living.table, living.at)
                .expect("the strip lives");
            if change.0 > living.at.0 + 1 {
                let mid = Millis(living.at.0 + (change.0 - living.at.0) / 2);
                assert!(
                    !living.advance(mid),
                    "a change at {mid:?} before {change:?}"
                );
            }
            living.advance(due);
        }
    }

    #[test]
    fn the_strip_follows_the_clock_it_is_given_into_night() {
        // Told it is 18:00, the strip carries the clock on by itself: by 20:00 it is night.
        let evening = LocalClock::new(preset::DATE, 18 * 3_600_000);
        let living = thriving_at(evening);
        assert_eq!(living.composer.palette_mix(Millis(0)), 0);
        assert_eq!(living.composer.palette_mix(Millis(2 * HOUR_MS)), 255);
    }

    #[test]
    fn an_update_never_changes_the_picture_at_its_own_moment() {
        let mut living = thriving_at(LocalClock::noon(preset::DATE));
        let now = Millis(90_000);
        living.advance(now);
        let before = living.image().clone();
        let night = OverlayInput {
            dim: true,
            ..OverlayInput::new(
                vec![copy::sample_label().to_owned()],
                LocalClock::midnight(preset::DATE),
            )
        };
        living.update(&preset::scene("dead").unwrap(), &night, now);
        assert!(!living.advance(now), "the update cut the picture");
        assert_eq!(living.image(), &before);
        assert!(living.due().is_some_and(|due| due > now));
    }

    #[test]
    fn a_woken_clock_eases_the_palette_and_never_cuts() {
        // The composer carried noon on, but the machine slept until midnight: the next update
        // brings the true clock, and the palette eases there over the clock fix, not at once.
        let mut living = thriving_at(LocalClock::noon(preset::DATE));
        let now = Millis(60_000);
        living.advance(now);
        assert_eq!(living.composer.palette_mix(now), 0);
        let overlay = OverlayInput::new(
            vec![copy::sample_label().to_owned()],
            LocalClock::midnight(preset::DATE),
        );
        living.update(&preset::scene("thriving").unwrap(), &overlay, now);
        assert_eq!(living.composer.palette_mix(now), 0);
        assert!(living.composer.palette_mix(Millis(now.0 + 3_000)) > 0);
        assert_eq!(living.composer.palette_mix(Millis(now.0 + 6_000)), 255);
    }
}
