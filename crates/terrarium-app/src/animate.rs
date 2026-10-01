//! The living strip (I-10): the thriving preset, composed on the clock, drawn only when its
//! pixels change and never more than 15 times a second. Until the live terrarium reaches the
//! strip it stays by day: every hour it is told again that it is noon.
//!
//! It reads no clock of its own; the strip passes the time in, so it can be driven in tests.

use terrarium_compose::catalog::Art;
use terrarium_compose::clock::LocalClock;
use terrarium_compose::composer::{Composer, OverlayInput};
use terrarium_compose::compositor::composite_into;
use terrarium_compose::frame::Rgba640x74;
use terrarium_compose::preset;
use terrarium_compose::scale::{Damage, Padding};
use terrarium_compose::schedule::Pacer;
use terrarium_compose::tween::Millis;
use terrarium_compose::variants::ArtTable;
use terrarium_compose::world::{self, Scenery};
use terrarium_truth::scene::SceneInput;

use crate::copy;

/// How often the strip is told it is noon again, well inside the seven hours to evening.
const NOON_EVERY_MS: u32 = 3_600_000;

pub struct Living {
    art: &'static Art<'static>,
    table: ArtTable,
    composer: Composer,
    scenery: Scenery,
    scene: SceneInput,
    overlay: OverlayInput,
    pacer: Pacer,
    /// When the strip is next told it is noon.
    noon_at: Millis,
    /// The moment last composed, and its pixels.
    at: Millis,
    image: Rgba640x74,
    padding: Padding,
    /// A spare image to compose into, so a frame that changes nothing costs no allocation.
    spare: Rgba640x74,
    /// When it next wants composing, worked out once per composition: the loop asks on
    /// every event it handles.
    due: Option<Millis>,
}

impl Living {
    /// The thriving terrarium, arriving at `now`.
    ///
    /// # Panics
    /// If the embedded art is incomplete or lacks the preset, which `cargo xtask verify` rules
    /// out.
    pub fn new(now: Millis) -> Living {
        let art = Art::embedded();
        let table = ArtTable::new(&art.catalog).expect("the embedded art depicts every variant");
        let scene = preset::scene("thriving").expect("the thriving preset");
        let overlay = OverlayInput::new(
            vec![copy::sample_label().to_owned()],
            LocalClock::noon(preset::DATE),
        );
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&scene, &overlay, now);
        let scenery = composer.scenery(art, &table);
        let blank = Rgba640x74::filled([0, 0, 0, u8::MAX]);
        let mut living = Living {
            art,
            table,
            composer,
            scenery,
            scene,
            overlay,
            pacer: Pacer::default(),
            noon_at: now.plus(NOON_EVERY_MS),
            at: now,
            image: blank.clone(),
            padding: [[0; 4]; terrarium_compose::frame::NATIVE_HEIGHT as usize],
            spare: blank,
            due: None,
        };
        living.compose(now);
        living
    }

    /// When the strip should next compose: at the composer's next change, or when it is to be
    /// told it is noon, but never sooner than a fifteenth of a second after it last composed.
    pub fn due(&self) -> Option<Millis> {
        self.due
    }

    /// Brings the strip to `now`; true if its pixels changed. Every call counts against the
    /// pace, so the strip composes at most 15 times a second whatever it shows.
    pub fn advance(&mut self, now: Millis) -> bool {
        if now >= self.noon_at {
            self.overlay.clock = LocalClock::noon(preset::DATE);
            self.composer.update(&self.scene, &self.overlay, now);
            self.noon_at = now.plus(NOON_EVERY_MS);
        }
        self.compose(now)
    }

    pub fn image(&self) -> &Rgba640x74 {
        &self.image
    }

    pub fn padding(&self) -> &Padding {
        &self.padding
    }

    /// Composes `now`; true if any pixel, of the image or of the padding beside it, changed.
    fn compose(&mut self, now: Millis) -> bool {
        let mix = self.composer.palette_mix(now);
        let frame = self
            .composer
            .frame(self.art, &self.table, &self.scenery, mix, now);
        composite_into(self.art, &frame, &mut self.spare);
        let padding = world::padding(self.art, mix, frame.dim);
        self.at = now;
        self.pacer.composed(now);
        let change = self
            .composer
            .next_change(self.art, &self.table, now)
            .map_or(self.noon_at, |change| change.min(self.noon_at));
        self.due = self.pacer.wake(Some(change));
        let changed = Damage::between(&self.image, &self.padding, &self.spare, &padding).is_some();
        if changed {
            std::mem::swap(&mut self.image, &mut self.spare);
            self.padding = padding;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strip_composes_and_draws_at_most_15_times_a_second() {
        // Five minutes of the living strip, woken only when it asks to be.
        let mut living = Living::new(Millis(0));
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
    }

    #[test]
    fn between_changes_nothing_changes() {
        let mut living = Living::new(Millis(0));
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
    fn the_strip_stays_by_day_and_is_told_noon_every_hour() {
        let mut living = Living::new(Millis(0));
        for hour in 1..9_u64 {
            living.advance(Millis(hour * 3_600_000));
            assert_eq!(living.composer.palette_mix(Millis(hour * 3_600_000 + 1)), 0);
        }
        assert!(living.noon_at > Millis(8 * 3_600_000));
    }
}
