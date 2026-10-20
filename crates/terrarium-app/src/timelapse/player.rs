//! The time-lapse window's picture: the film played on the wall clock by a composer of its own
//! ([`Stage`]), 15 frames a second while it plays.
//!
//! The stage's clock runs only while the picture moves: while playing, and after a seek or a
//! new film until what it started has settled. Stopped and settled, nothing is composed and the
//! picture holds. A new film, the same days by night or another range, is taken by the same
//! stage, so the change tweens like any other.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::time::{Duration, Instant};

use terrarium_compose::catalog::Art;
use terrarium_compose::frame::NativeImage;
use terrarium_compose::salt::InstallSalt;
use terrarium_compose::schedule::MIN_FRAME_MS;
use terrarium_compose::timelapse::{FRAMES, Stage, frame_at};
use terrarium_compose::tween::Millis;
use terrarium_compose::variants::ArtTable;

use super::Film;

/// The last frame.
const LAST: usize = FRAMES - 1;

pub struct Player {
    salt: InstallSalt,
    film: Option<Arc<Film>>,
    stage: Option<Stage>,
    /// The stage's clock.
    clock: Millis,
    /// The wall moment the clock last moved at, while the picture moves.
    moved: Option<Instant>,
    /// The frame shown.
    frame: usize,
    /// While playing, the wall moment play began and the frame it began from.
    playing: Option<(Instant, usize)>,
    /// After a seek or a new film, the stage's moment it settles at.
    settles: Option<Millis>,
    image: Option<NativeImage>,
    /// Counts the pictures composed, so a window uploads each once.
    serial: u64,
    /// The composer panicked: the stage is gone and the picture stands as it was (I-16).
    halted: bool,
    /// Stage calls to fail before the next one runs, for the tests.
    #[cfg(test)]
    trip: u8,
}

impl Player {
    pub fn new(salt: InstallSalt) -> Player {
        Player {
            salt,
            film: None,
            stage: None,
            clock: Millis(0),
            moved: None,
            frame: 0,
            playing: None,
            settles: None,
            image: None,
            serial: 0,
            halted: false,
            #[cfg(test)]
            trip: 0,
        }
    }

    /// Whether the composer panicked, so the picture no longer moves.
    pub fn halted(&self) -> bool {
        self.halted
    }

    /// Runs `body` on the stage, unless it has panicked before; a panic in it lets the stage
    /// go, leaving the picture as it was.
    fn guarded(&mut self, body: impl FnOnce(&mut Player)) {
        if self.halted {
            return;
        }
        let ran = panic::catch_unwind(AssertUnwindSafe(|| {
            #[cfg(test)]
            if self.trip > 0 {
                self.trip -= 1;
                panic!("a composer failing in the app");
            }
            body(self);
        }));
        if ran.is_err() {
            self.halted = true;
            self.stage = None;
            self.playing = None;
            self.moved = None;
            self.settles = None;
        }
    }

    /// Plays `film` from the frame shown: the first film settled on it, a later one taken by
    /// the same composer and changing in.
    pub fn set_film(&mut self, film: Arc<Film>, now: Instant) {
        self.guarded(|player| player.set_film_guarded(film, now));
    }

    fn set_film_guarded(&mut self, film: Arc<Film>, now: Instant) {
        let showing = film.sampling.frames[self.frame];
        if self.stage.is_some() {
            self.advance_clock(now);
        }
        if let Some(stage) = &mut self.stage {
            stage.take(&film.shots, showing, self.clock);
            self.settles = Some(stage.settled_at());
            self.moved.get_or_insert(now);
        } else {
            let (stage, start) = Stage::new(
                Art::embedded(),
                ArtTable::embedded(),
                self.salt,
                film.prime.as_ref(),
                &film.shots,
                showing,
            );
            self.clock = start;
            self.settles = Some(stage.settled_at());
            self.moved = Some(now);
            self.stage = Some(stage);
        }
        self.film = Some(film);
        self.compose();
    }

    pub fn film(&self) -> Option<&Arc<Film>> {
        self.film.as_ref()
    }

    /// Plays from the frame shown, or from the first once the last is reached.
    pub fn play(&mut self, now: Instant) {
        if self.stage.is_none() {
            return;
        }
        if self.frame == LAST {
            self.seek(0, now);
        }
        self.advance_clock(now);
        self.playing = Some((now, self.frame));
        self.moved = Some(now);
    }

    /// Stops on the frame shown; what it started settles.
    pub fn stop(&mut self, now: Instant) {
        if self.playing.take().is_some() {
            self.advance_clock(now);
            self.guarded(|player| player.settles = player.stage.as_ref().map(Stage::settled_at));
        }
    }

    pub fn playing(&self) -> bool {
        self.playing.is_some()
    }

    /// Shows frame `frame`, going on playing from it if playing.
    pub fn seek(&mut self, frame: usize, now: Instant) {
        let frame = frame.min(LAST);
        self.advance_clock(now);
        self.frame = frame;
        if let Some(playing) = &mut self.playing {
            *playing = (now, frame);
        }
        self.moved = Some(now);
        self.guarded(|player| {
            player.show();
            player.settles = player.stage.as_ref().map(Stage::settled_at);
        });
    }

    /// The frame shown.
    pub fn frame(&self) -> usize {
        self.frame
    }

    /// Moves the picture on to `now`, if it moves; true if a picture was composed.
    pub fn advance(&mut self, now: Instant) -> bool {
        if self.stage.is_none() || self.moved.is_none() {
            return false;
        }
        let mut composed = false;
        self.guarded(|player| composed = player.advance_guarded(now));
        composed
    }

    fn advance_guarded(&mut self, now: Instant) -> bool {
        self.advance_clock(now);
        if let Some((began, from)) = self.playing {
            let elapsed = now.saturating_duration_since(began);
            let frame = from.saturating_add(frame_at(millis(elapsed)));
            self.frame = frame.min(LAST);
            self.show();
            if frame >= LAST {
                self.playing = None;
                self.settles = self.stage.as_ref().map(Stage::settled_at);
            }
        }
        self.compose();
        if self.playing.is_none() && self.settles.is_none_or(|settles| self.clock >= settles) {
            self.moved = None;
            self.settles = None;
        }
        true
    }

    /// How long until the picture next moves, while it does.
    pub fn next_in(&self) -> Option<Duration> {
        self.moved.map(|_| Duration::from_millis(MIN_FRAME_MS))
    }

    /// The picture, and its serial.
    pub fn image(&self) -> Option<(&NativeImage, u64)> {
        self.image.as_ref().map(|image| (image, self.serial))
    }

    fn advance_clock(&mut self, now: Instant) {
        if let Some(moved) = self.moved.replace(now) {
            let elapsed = millis(now.saturating_duration_since(moved));
            self.clock = Millis(self.clock.0.saturating_add(elapsed));
        }
    }

    fn show(&mut self) {
        if let (Some(stage), Some(film)) = (&mut self.stage, &self.film) {
            stage.show(&film.shots, film.sampling.frames[self.frame], self.clock);
        }
    }

    fn compose(&mut self) {
        if let Some(stage) = &self.stage {
            self.image = Some(stage.frame(Art::embedded(), ArtTable::embedded(), self.clock));
            self.serial += 1;
        }
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use terrarium_compose::clock::LocalClock;
    use terrarium_compose::preset;
    use terrarium_compose::timelapse::{Sampling, Shot};

    use super::*;

    fn film(names: &[&str]) -> Arc<Film> {
        let shots: Vec<Shot> = names
            .iter()
            .map(|name| Shot {
                scene: preset::scene(name).unwrap(),
                label: vec![(*name).to_owned()],
                clock: LocalClock::noon(preset::DATE),
            })
            .collect();
        Arc::new(Film {
            sampling: Sampling::new(0, shots.len() - 1, FRAMES),
            prime: None,
            shots,
            night: false,
        })
    }

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn playing_follows_the_wall_clock_at_fifteen_frames_a_second_and_stops_at_the_end() {
        let start = Instant::now();
        let mut player = Player::new(preset::SALT);
        player.set_film(film(&["thriving", "dead"]), start);
        assert!(player.image().is_some());
        // A new film settles first, then the picture holds.
        let mut ms = 0;
        while player.next_in().is_some() && ms < 900 {
            ms += 67;
            assert!(player.advance(at(start, ms)));
        }
        assert_eq!(player.next_in(), None);
        assert!(!player.advance(at(start, 950)));
        player.play(at(start, 1000));
        assert!(player.playing());
        assert!(player.advance(at(start, 1000 + 210)));
        assert_eq!(player.frame(), 3);
        assert!(player.advance(at(start, 1000 + 15_000)));
        assert_eq!(player.frame(), 225);
        player.advance(at(start, 1000 + 31_000));
        assert_eq!(player.frame(), LAST);
        assert!(!player.playing());
        // Played to the end, play starts over.
        player.play(at(start, 40_000));
        assert_eq!(player.frame(), 0);
    }

    #[test]
    fn a_composer_panic_stops_the_picture_where_it_stood() {
        let start = Instant::now();
        let mut player = Player::new(preset::SALT);
        player.set_film(film(&["thriving", "dead"]), start);
        let (_, before) = player.image().unwrap();
        let held = player.image().unwrap().0.clone();
        player.trip = 1;
        assert!(!player.advance(at(start, 67)));
        assert!(player.halted());
        assert_eq!(player.next_in(), None);
        assert_eq!(player.image(), Some((&held, before)));
        // Nothing asks the composer again.
        player.play(at(start, 200));
        player.seek(LAST, at(start, 300));
        player.set_film(film(&["dead"]), at(start, 400));
        assert!(!player.advance(at(start, 500)));
        assert_eq!(player.image(), Some((&held, before)));
    }

    #[test]
    fn a_seek_or_a_new_film_moves_the_picture_until_it_settles_and_a_stop_holds_the_frame() {
        let start = Instant::now();
        let mut player = Player::new(preset::SALT);
        player.set_film(film(&["thriving", "dead"]), start);
        let mut ms = 0;
        while player.next_in().is_some() {
            ms += 67;
            player.advance(at(start, ms));
        }
        let (_, before) = player.image().unwrap();
        player.seek(LAST, at(start, ms));
        assert_eq!(player.frame(), LAST);
        assert!(player.next_in().is_some());
        while player.next_in().is_some() {
            ms += 67;
            player.advance(at(start, ms));
        }
        assert!(player.image().unwrap().1 > before);
        // The same days by night: taken by the same stage, so it changes in.
        let night = Arc::new(film(&["thriving", "dead"]).reclocked(true));
        assert!(
            night
                .shots
                .iter()
                .all(|shot| shot.clock == LocalClock::midnight(preset::DATE))
        );
        player.set_film(night, at(start, ms));
        assert_eq!(player.frame(), LAST);
        assert!(player.next_in().is_some());
        player.play(at(start, ms));
        player.advance(at(start, ms + 1000));
        player.stop(at(start, ms + 1000));
        let frame = player.frame();
        assert!(!player.playing());
        player.advance(at(start, ms + 5000));
        assert_eq!(player.frame(), frame);
    }
}
