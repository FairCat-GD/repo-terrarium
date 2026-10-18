//! The living strip (I-10): a scene composed on the clock, drawn only when its pixels change
//! and never more than 15 times a second. What it shows, and when that changes, is the feed's
//! business (`feed`); this module only composes.
//!
//! It reads no clock of its own; the strip passes the time in, so it can be driven in tests.

use terrarium_compose::blend::blend;
use terrarium_compose::catalog::{Art, Blend, Catalog};
use terrarium_compose::composer::{Composer, OverlayInput};
use terrarium_compose::compositor::{composite_area, damage};
use terrarium_compose::frame::{Area, Frame, NATIVE_HEIGHT, NativeImage, Viewport};
use terrarium_compose::panel::{Hover, Landmark};
use terrarium_compose::salt::InstallSalt;
use terrarium_compose::schedule::Pacer;
use terrarium_compose::tween::durations::RESTYLE_MS;
use terrarium_compose::tween::{Millis, Tween};
use terrarium_compose::variants::ArtTable;
use terrarium_compose::world::Scenery;
use terrarium_truth::scene::SceneInput;

pub struct Living {
    art: &'static Art<'static>,
    table: &'static ArtTable,
    composer: Composer,
    scenery: Scenery,
    pacer: Pacer,
    /// The moment last composed, and its pixels.
    at: Millis,
    image: NativeImage,
    /// A spare image to compose into, so a frame that changes nothing costs no allocation.
    spare: NativeImage,
    /// The frame `image` was composited from, exactly: only what a new frame changes of it
    /// is composited again. None while a handover's dissolve lies over the picture.
    shown: Option<Frame>,
    /// Where `spare` may hold other pixels than `image`; none where it holds the same.
    stale: Option<Area>,
    /// When it next wants composing, worked out once per composition or update: the loop asks
    /// on every event it handles.
    due: Option<Millis>,
    compositions: u64,
    /// A re-seed's handover: the strip as it stood, fading out over the new layout.
    handover: Option<Handover>,
}

/// The rows and columns of `area` inside `whole`.
fn span(area: Area, whole: Area) -> (std::ops::Range<u32>, std::ops::Range<usize>) {
    let clamp = |at: i32, end: i32| at.clamp(0, end).cast_unsigned();
    let rows = clamp(area.y, whole.h)..clamp(area.y.saturating_add(area.h), whole.h);
    let columns =
        clamp(area.x, whole.w) as usize..clamp(area.x.saturating_add(area.w), whole.w) as usize;
    (rows, columns)
}

/// The picture a re-seed left, and its opacity over the new one's.
struct Handover {
    from: NativeImage,
    fade: Tween,
}

impl Living {
    /// `scene` and `overlay`, arriving at `now`, laid out by `salt` (I-2).
    ///
    /// # Panics
    /// If the embedded art is incomplete, which `cargo xtask verify` rules out.
    pub fn new(
        scene: &SceneInput,
        overlay: &OverlayInput,
        salt: InstallSalt,
        now: Millis,
    ) -> Living {
        let art = Art::embedded();
        let table = ArtTable::embedded();
        let mut composer = Composer::new(table, salt);
        composer.update(scene, overlay, now);
        let scenery = composer.scenery(art, table);
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
            shown: None,
            stale: None,
            due: None,
            compositions: 0,
            handover: None,
        };
        living.compose(now);
        living
    }

    /// A new terrarium for the same repository (re-seed): from `now` the strip shows `scene`
    /// and `overlay` laid out by the new `salt`, and the picture as it stood dissolves into it
    /// over a restyle's time, rather than cutting (I-15). The new layout stands whole from the
    /// start, as if it had always been there, and replays no event the old one played. A
    /// paused strip dissolves into the new layout standing still.
    pub fn reseed(
        &mut self,
        scene: &SceneInput,
        overlay: &OverlayInput,
        salt: InstallSalt,
        now: Millis,
    ) {
        let quiet = SceneInput {
            event: None,
            ..scene.clone()
        };
        let viewport = self.composer.viewport();
        let fresh = || {
            let mut composer = Composer::new(self.table, salt);
            composer.set_viewport(viewport);
            composer
        };
        // How long its arrivals take, found on a copy: the new composer's first update is
        // dated back by as much, so it stands settled now. It settles running; a pause takes
        // hold only at `now`.
        let running = OverlayInput {
            paused: false,
            ..overlay.clone()
        };
        let mut probe = fresh();
        probe.update(&quiet, &running, Millis(0));
        let settling = probe.settled_at().0;
        let mut composer = fresh();
        composer.update(&quiet, &running, Millis(now.0.saturating_sub(settling)));
        composer.update(&quiet, overlay, now);
        self.scenery = composer.scenery(self.art, self.table);
        self.composer = composer;
        self.handover = Some(Handover {
            from: self.image.clone(),
            fade: Tween::new(u8::MAX, 0, now, RESTYLE_MS),
        });
        self.due = self.pacer.wake(self.next_change(now));
    }

    /// Whether line `index` of the label showing at `now` has had its turn on the plate.
    pub fn label_step_done(&self, index: usize, now: Millis) -> bool {
        self.composer.label_step_done(index, now)
    }

    /// Moves the strip toward `scene` and `overlay` from `now`. The picture at `now` does not
    /// change, so nothing is composed; only the next moment it will is worked out again.
    pub fn update(&mut self, scene: &SceneInput, overlay: &OverlayInput, now: Millis) {
        self.composer.update(scene, overlay, now);
        self.due = self.pacer.wake(self.next_change(now));
    }

    /// Moves only the hover layer toward `hover` from `now`; as with [`Living::update`], the
    /// picture at `now` does not change.
    pub fn hover(&mut self, hover: &Hover, now: Millis) {
        self.composer.hover(hover, now);
        self.due = self.pacer.wake(self.next_change(now));
    }

    /// The monument or fossil at `(x, y)` in the viewport at `now`, if any.
    pub fn landmark_at(&self, x: i32, y: i32, now: Millis) -> Option<Landmark> {
        self.composer.landmark_at(self.art, self.table, x, y, now)
    }

    /// The columns it composes: the viewport's width.
    pub fn columns(&self) -> i32 {
        i32::try_from(self.composer.viewport().width()).unwrap_or(i32::MAX)
    }

    /// The embedded art's catalog, which the hover layer is laid out from.
    pub fn catalog(&self) -> &'static Catalog {
        &self.art.catalog
    }

    /// The next moment the picture changes: the composer's next change, or a handover's next
    /// step.
    fn next_change(&self, now: Millis) -> Option<Millis> {
        let fading = self
            .handover
            .as_ref()
            .and_then(|handover| handover.fade.next_step(now));
        [self.composer.next_change(self.art, self.table, now), fading]
            .into_iter()
            .flatten()
            .min()
    }

    /// When the strip should next compose: at the composer's next change, but never sooner
    /// than a fifteenth of a second after it last composed.
    pub fn due(&self) -> Option<Millis> {
        self.due
    }

    /// Brings the strip to `now` if it is due to compose; true if its pixels changed. The pace
    /// holds for every call, the OS's redraws included (I-10): one sooner composes nothing, and
    /// the picture held stands.
    pub fn advance(&mut self, now: Millis) -> bool {
        self.due.is_some_and(|due| due <= now) && self.compose(now)
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
        self.scenery = self.composer.scenery(self.art, self.table);
        // The whole picture is new at another width; the handover's is the old width's.
        self.handover = None;
        self.spare = NativeImage::new(viewport, [0, 0, 0, u8::MAX]);
        self.compose(now);
        self.spare = NativeImage::new(viewport, [0, 0, 0, u8::MAX]);
        self.stale = Some(Area {
            x: 0,
            y: 0,
            w: i32::try_from(viewport.width()).unwrap_or(i32::MAX),
            h: i32::try_from(NATIVE_HEIGHT).unwrap_or(i32::MAX),
        });
        true
    }

    /// Compositions since it was made, presented or not.
    pub fn compositions(&self) -> u64 {
        self.compositions
    }

    /// Composes `now`; true if any pixel changed. Only the area the frame changes of the one
    /// before is composited again, over a copy of the picture: the pixels are a whole
    /// composite's.
    fn compose(&mut self, now: Millis) -> bool {
        let mix = self.composer.palette_mix(now);
        let frame = self
            .composer
            .frame(self.art, self.table, &self.scenery, mix, now);
        let whole = Area {
            x: 0,
            y: 0,
            w: i32::try_from(self.spare.width()).unwrap_or(i32::MAX),
            h: i32::try_from(NATIVE_HEIGHT).unwrap_or(i32::MAX),
        };
        let area = match &self.shown {
            Some(shown) if self.handover.is_none() => damage(self.art, shown, &frame),
            _ => Some(whole),
        };
        let changed = area.is_some_and(|area| {
            // Only a frame of the same width is composited in part, over the picture held;
            // the spare already holds it but where the two last differed.
            let partial = area != whole && self.image.width() == self.spare.width();
            if partial && let Some(stale) = self.stale {
                let (rows, columns) = span(stale, whole);
                for y in rows {
                    self.spare.row_mut(y)[columns.clone()]
                        .copy_from_slice(&self.image.row(y)[columns.clone()]);
                }
            }
            let area = if partial { area } else { whole };
            composite_area(self.art, &frame, &mut self.spare, area);
            if let Some(handover) = &self.handover {
                let opacity = handover.fade.value(now);
                if opacity == 0 {
                    self.handover = None;
                } else {
                    dissolve(&mut self.spare, &handover.from, opacity);
                }
            }
            // Outside the area the two hold the same pixels.
            let changed = if partial {
                let (rows, columns) = span(area, whole);
                rows.into_iter().any(|y| {
                    self.image.row(y)[columns.clone()] != self.spare.row(y)[columns.clone()]
                })
            } else {
                self.image != self.spare
            };
            // Swapped, the spare holds the picture before, which differs only in the area.
            self.stale = changed.then_some(area);
            if changed {
                std::mem::swap(&mut self.image, &mut self.spare);
            }
            changed
        });
        self.shown = self.handover.is_none().then_some(frame);
        self.at = now;
        self.compositions += 1;
        self.pacer.composed(now);
        self.due = self.pacer.wake(self.next_change(now));
        changed
    }
}

/// `from` drawn over `onto` at `opacity`, pixel by pixel, with the one blend rule.
fn dissolve(onto: &mut NativeImage, from: &NativeImage, opacity: u8) {
    for y in 0..NATIVE_HEIGHT {
        for (out, old) in onto.row_mut(y).iter_mut().zip(from.row(y)) {
            // Red, green and blue; every pixel of the strip is opaque.
            for (channel, &was) in out.iter_mut().zip(old).take(3) {
                *channel = blend(*channel, was, opacity, Blend::Normal);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use terrarium_compose::clock::LocalClock;
    use terrarium_compose::preset;

    use super::*;
    use crate::copy;

    const HOUR_MS: u64 = 3_600_000;

    fn thriving_at(clock: LocalClock) -> Living {
        let overlay = OverlayInput::new(vec![copy::sample_label().to_owned()], clock);
        Living::new(
            &preset::scene("thriving").unwrap(),
            &overlay,
            preset::SALT,
            Millis(0),
        )
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
            frames += usize::from(living.compose(due));
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
    fn each_picture_is_a_whole_composite_of_its_frame() {
        // Four minutes of the thriving strip, composited only where each frame changed: at
        // the scene's width, then at 735 columns, then through a re-seed's dissolve.
        let mut living = thriving_at(LocalClock::noon(preset::DATE));
        let mut checked = 0;
        let mut reseeded = false;
        while let Some(due) = living.due().filter(|due| due.0 <= 240_000) {
            if due.0 >= 80_000 && living.columns() == 640 {
                living.fit(Viewport::new(735), due);
            }
            if due.0 >= 160_000 && !reseeded {
                reseeded = true;
                let overlay = OverlayInput::new(
                    vec![copy::sample_label().to_owned()],
                    LocalClock::noon(preset::DATE),
                );
                let scene = preset::scene("thriving").unwrap();
                living.reseed(&scene, &overlay, InstallSalt::new([9; 16]), due);
            }
            living.compose(due);
            if living.handover.is_none() {
                let mix = living.composer.palette_mix(due);
                let frame =
                    living
                        .composer
                        .frame(living.art, living.table, &living.scenery, mix, due);
                let whole = terrarium_compose::compositor::composite(living.art, &frame);
                assert!(living.image() == &whole, "at {due:?}");
            }
            checked += 1;
        }
        assert!(checked > 1000, "{checked} compositions");
    }

    #[test]
    fn the_pace_holds_whoever_asks_for_a_frame() {
        let mut living = thriving_at(LocalClock::noon(preset::DATE));
        let due = living.due().expect("the strip lives");
        let compositions = living.compositions();
        // Asked sooner, as by the OS's own redraw, it composes nothing.
        assert!(!living.advance(Millis(due.0 - 1)));
        assert_eq!(living.compositions(), compositions);
        living.advance(due);
        assert_eq!(living.compositions(), compositions + 1);
    }

    #[test]
    fn between_changes_nothing_changes() {
        let mut living = thriving_at(LocalClock::noon(preset::DATE));
        for _ in 0..200 {
            let due = living.due().expect("the strip lives");
            let change = living
                .composer
                .next_change(living.art, living.table, living.at)
                .expect("the strip lives");
            if change.0 > living.at.0 + 1 {
                let mid = Millis(living.at.0 + (change.0 - living.at.0) / 2);
                assert!(
                    !living.compose(mid),
                    "a change at {mid:?} before {change:?}"
                );
            }
            living.compose(due);
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
        living.compose(now);
        let before = living.image().clone();
        let night = OverlayInput {
            dim: true,
            ..OverlayInput::new(
                vec![copy::sample_label().to_owned()],
                LocalClock::midnight(preset::DATE),
            )
        };
        living.update(&preset::scene("dead").unwrap(), &night, now);
        assert!(!living.compose(now), "the update cut the picture");
        assert_eq!(living.image(), &before);
        assert!(living.due().is_some_and(|due| due > now));
    }

    #[test]
    fn a_reseed_dissolves_into_the_new_layout_and_never_cuts() {
        let mut living = thriving_at(LocalClock::noon(preset::DATE));
        let now = Millis(120_000);
        living.compose(now);
        let before = living.image().clone();
        let overlay = OverlayInput::new(
            vec![copy::sample_label().to_owned()],
            LocalClock::noon(preset::DATE),
        );
        let salt = InstallSalt::new([7; 16]);
        living.reseed(&preset::scene("thriving").unwrap(), &overlay, salt, now);
        assert!(
            living.composer.settled_at() <= now,
            "the new layout stands whole from the start"
        );
        assert!(!living.compose(now), "the re-seed cut the picture");
        assert_eq!(living.image(), &before);
        // It dissolves at the strip's pace, and the old picture is gone by its end.
        let end = now.plus(RESTYLE_MS);
        let mut wakes = Vec::new();
        let mut mid = None;
        while let Some(due) = living.due().filter(|&due| due <= end) {
            wakes.push(due);
            living.compose(due);
            if mid.is_none() && due >= now.plus(RESTYLE_MS / 2) {
                mid = Some(living.image().clone());
            }
        }
        assert!(
            wakes.len() <= 15 * 6 + 1,
            "{} wakes in six seconds",
            wakes.len()
        );
        living.compose(end);
        assert!(living.handover.is_none(), "the handover ended");
        let mix = living.composer.palette_mix(end);
        let frame = living
            .composer
            .frame(living.art, living.table, &living.scenery, mix, end);
        let mut alone = NativeImage::new(living.composer.viewport(), [0, 0, 0, u8::MAX]);
        terrarium_compose::compositor::composite_into(living.art, &frame, &mut alone);
        assert_eq!(living.image(), &alone, "nothing of the old layout stays");
        assert_ne!(&alone, &before, "a new layout");
        let mid = mid.expect("a frame mid-way");
        assert!(mid != before && mid != alone, "mid-way, a blend of the two");
    }

    #[test]
    fn a_paused_strip_asks_for_no_frame_once_its_label_has_crossed() {
        let mut living = thriving_at(LocalClock::noon(preset::DATE));
        let scene = preset::scene("thriving").unwrap();
        let overlay = |line: &str, paused| OverlayInput {
            paused,
            ..OverlayInput::new(vec![line.to_owned()], LocalClock::noon(preset::DATE))
        };
        // A pause and a resume first, so the scene's clock stands behind the strip's.
        living.update(&scene, &overlay(copy::label_paused(), true), Millis(30_000));
        living.update(
            &scene,
            &overlay(copy::sample_label(), false),
            Millis(60_000),
        );
        let now = Millis(90_000);
        living.compose(now);
        living.update(&scene, &overlay(copy::label_paused(), true), now);
        assert!(!living.compose(now), "the pause cut the picture");
        // The label crosses to its one line, then nothing moves.
        let mut last = now;
        while let Some(due) = living.due() {
            assert!(due.0 < now.0 + 60_000, "still asking at {due:?}");
            living.compose(due);
            last = due;
        }
        let held = living.image().clone();
        assert!(last > now);
        assert!(
            !living.compose(Millis(now.0 + 600_000)),
            "ten minutes on, nothing changed"
        );
        assert_eq!(living.image(), &held);
    }

    #[test]
    fn a_reseed_while_paused_dissolves_into_the_new_layout_standing_still() {
        let mut living = thriving_at(LocalClock::noon(preset::DATE));
        let now = Millis(120_000);
        let paused = OverlayInput {
            paused: true,
            ..OverlayInput::new(
                vec![copy::label_paused().to_owned()],
                LocalClock::noon(preset::DATE),
            )
        };
        let scene = preset::scene("thriving").unwrap();
        living.update(&scene, &paused, now);
        living.compose(now);
        let salt = InstallSalt::new([7; 16]);
        living.reseed(&scene, &paused, salt, now);
        assert!(!living.compose(now), "the re-seed cut the picture");
        // Once the old picture has dissolved, the new layout stands whole and still.
        let end = now.plus(RESTYLE_MS);
        living.compose(end);
        assert!(living.handover.is_none());
        let settled = living.image().clone();
        let mut whole = Composer::new(living.table, salt);
        whole.set_viewport(living.composer.viewport());
        let running = OverlayInput {
            paused: false,
            ..paused.clone()
        };
        whole.update(&scene, &running, Millis(0));
        assert_eq!(
            living.composer.depicted(end),
            whole.depicted(whole.settled_at()),
            "the new layout stands whole"
        );
        assert!(!living.compose(Millis(end.0 + 300_000)), "it stands still");
        assert_eq!(living.image(), &settled);
    }

    #[test]
    fn a_woken_clock_eases_the_palette_and_never_cuts() {
        // The composer carried noon on, but the machine slept until midnight: the next update
        // brings the true clock, and the palette eases there over the clock fix, not at once.
        let mut living = thriving_at(LocalClock::noon(preset::DATE));
        let now = Millis(60_000);
        living.compose(now);
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
