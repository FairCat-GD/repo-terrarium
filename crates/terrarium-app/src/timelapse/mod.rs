//! The time-lapse: the repository's history replayed from its stored
//! observations by a replay and a composer of its own, played in 30 seconds and written as a
//! GIF, a postcard or, through the person's ffmpeg, an MP4. The live strip's engine and composer are never touched, and a paused
//! strip stays paused.
//!
//! - this module makes a [`Film`] from a history's days and writes it; `mp4` writes it through
//!   ffmpeg;
//! - `form` is the window, a model drawn with egui, and its `player`;
//! - `job` runs the reading, the replay and the writing off the loop.

pub mod form;
pub mod job;
pub mod mp4;
pub mod player;

use std::io::{self, Write};
use std::path::Path;

use terrarium_compose::catalog::Art;
use terrarium_compose::clock::LocalClock;
use terrarium_compose::export::{self, gif::GifWriter};
use terrarium_compose::salt::InstallSalt;
use terrarium_compose::scale::{StripScale, upscale_into};
use terrarium_compose::sky::LocalDate;
use terrarium_compose::timelapse::{FRAMES, Reel, Sampling, Shot, still};
use terrarium_compose::variants::ArtTable;
use terrarium_engine::timelapse::Lapse;
use terrarium_truth::{ReplayError, Tick};

use crate::copy;
use crate::files::write_beside;

/// The longest range whose label names each day; a longer one plays its days too quickly to
/// read, and names the month.
pub const DAY_LABELS_UP_TO: usize = 120;

/// A time-lapse ready to play or write: which day each frame shows, a shot for each day shown,
/// and the day before the first, which the composer settles on first.
#[derive(Clone, Debug)]
pub struct Film {
    pub sampling: Sampling,
    pub prime: Option<Shot>,
    pub shots: Vec<Shot>,
    /// Drawn at midnight rather than noon.
    pub night: bool,
}

impl Film {
    /// The days `from..=to` of `lapse`'s `closes`, by day or by night.
    ///
    /// # Panics
    /// If the range is not inside `closes`.
    pub fn new(
        lapse: &Lapse,
        closes: &[Tick],
        from: usize,
        to: usize,
        night: bool,
    ) -> Result<Film, ReplayError> {
        let sampling = Sampling::new(from, to, FRAMES);
        let before = from.checked_sub(1);
        let ticks: Vec<Tick> = before
            .into_iter()
            .chain(sampling.days.iter().copied())
            .map(|day| closes[day])
            .collect();
        let mut scenes = lapse.scenes(&ticks)?.into_iter();
        let by_day = to - from < DAY_LABELS_UP_TO;
        let mut shot = |day: usize| {
            let date = LocalDate::from_days_since_epoch(lapse.day(closes[day]));
            Shot {
                scene: scenes.next().expect("a scene for each close"),
                label: vec![copy::timelapse::date(date, by_day)],
                clock: if night {
                    LocalClock::midnight(date)
                } else {
                    LocalClock::noon(date)
                },
            }
        };
        let prime = before.map(&mut shot);
        let shots = sampling.days.iter().map(|&day| shot(day)).collect();
        Ok(Film {
            sampling,
            prime,
            shots,
            night,
        })
    }

    /// The same days at the other hour: noon by day, midnight by night.
    #[must_use]
    pub fn reclocked(&self, night: bool) -> Film {
        let clock = |shot: &Shot| Shot {
            clock: if night {
                LocalClock::midnight(shot.clock.date)
            } else {
                LocalClock::noon(shot.clock.date)
            },
            ..shot.clone()
        };
        Film {
            sampling: self.sampling.clone(),
            prime: self.prime.as_ref().map(clock),
            shots: self.shots.iter().map(clock).collect(),
            night,
        }
    }

    /// The shot frame `frame` shows.
    pub fn shot(&self, frame: usize) -> &Shot {
        &self.shots[self.sampling.frames[frame]]
    }
}

/// Writes `film` as a GIF `scale` times the scene's size to `path`, by way of a file beside it,
/// so `path` is only ever the whole GIF. `progress` hears each frame written, in percent.
pub fn write_gif(
    film: &Film,
    salt: InstallSalt,
    scale: u32,
    path: &Path,
    progress: &mut dyn FnMut(u8),
) -> io::Result<()> {
    let art = Art::embedded();
    let reel = Reel::new(
        art,
        ArtTable::embedded(),
        salt,
        film.prime.as_ref(),
        &film.shots,
        &film.sampling,
    );
    let frames = reel.len().max(1);
    write_beside(path, |file| {
        let mut gif = GifWriter::new(file, scale)?;
        for image in reel {
            gif.push(&image)?;
            progress(u8::try_from(gif.frames() * 100 / frames).unwrap_or(100));
        }
        gif.finish()?.flush()
    })
}

/// Writes `shot`, settled, as a PNG `scale` times the scene's size to `path`, by way of a file
/// beside it.
pub fn write_postcard(shot: &Shot, salt: InstallSalt, scale: u32, path: &Path) -> io::Result<()> {
    let art = Art::embedded();
    let image = still(art, ArtTable::embedded(), salt, shot);
    let size = StripScale::exact(scale);
    let mut pixels = vec![[0; 4]; size.pixel_count()];
    upscale_into(&image, &size, |rgba| rgba, &mut pixels).expect("a buffer of the postcard's size");
    let bytes = export::png(size.width, size.height, &pixels);
    write_beside(path, |file| {
        file.write_all(&bytes)?;
        file.flush()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use sha2::{Digest, Sha256};
    use terrarium_adapters::fixture;
    use terrarium_compose::export::read_png;
    use terrarium_compose::preset;
    use terrarium_compose::timelapse::DELAYS_CS;
    use terrarium_engine::headless::{Request, lapse};
    use terrarium_store::DataDir;
    use terrarium_truth::canon::to_hex;
    use terrarium_truth::scene::EventKind;

    use super::*;
    use crate::files::beside;

    /// The fixture arc, read as `replay` reads it.
    pub(crate) fn arc() -> (tempfile::TempDir, Lapse) {
        let dir = tempfile::tempdir().unwrap();
        fixture::arc(&dir.path().join("repo")).unwrap();
        let observations = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/arc/observations.jsonl"),
        )
        .unwrap();
        let read = lapse(Request {
            repo: &dir.path().join("repo"),
            data: &DataDir::at(dir.path().join("data")),
            observations: Some(&mut observations.as_slice()),
            at_secs: fixture::ARC_REPLAY_END_SECS,
            install_salt: [0; 16],
        })
        .unwrap();
        (dir, read)
    }

    /// The GIF golden: two months of the arc, from a flowering through a storm
    /// and the lightning and sinkhole after it, played in 30 seconds by day at 1×, are the
    /// same bytes on every run and every machine.
    #[test]
    fn two_months_of_the_arc_make_the_same_gif_every_time() {
        let (dir, lapse) = arc();
        let closes = lapse.closes();
        let film = Film::new(&lapse, &closes, 255, 315, false).unwrap();
        assert!(film.prime.is_some());
        assert_eq!(film.shots.len(), 61);
        assert_eq!(film.shots[0].label, ["19 sep 2024"]);
        assert_eq!(
            film.shots[0].clock,
            LocalClock::noon(LocalDate::new(2024, 9, 19))
        );
        let played: Vec<EventKind> = film
            .shots
            .iter()
            .filter_map(|shot| shot.scene.event.map(|event| event.kind))
            .collect();
        for kind in [EventKind::Flowering, EventKind::Storm, EventKind::Sinkhole] {
            assert!(played.contains(&kind), "{kind:?} in {played:?}");
        }
        let path = dir.path().join("arc.gif");
        let mut heard = Vec::new();
        write_gif(&film, InstallSalt::new([0; 16]), 1, &path, &mut |percent| {
            heard.push(percent);
        })
        .unwrap();
        assert_eq!(heard.last(), Some(&100));
        assert!(!beside(&path).exists());
        let bytes = std::fs::read(&path).unwrap();
        let mut decoder = ::gif::DecodeOptions::new()
            .read_info(bytes.as_slice())
            .unwrap();
        assert_eq!(decoder.repeat(), ::gif::Repeat::Infinite);
        assert_eq!((decoder.width(), decoder.height()), (640, 74));
        let mut delays = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            delays.push(frame.delay);
        }
        assert_eq!(delays.len(), FRAMES);
        assert!(
            delays
                .iter()
                .enumerate()
                .all(|(i, &delay)| delay == DELAYS_CS[i % 3])
        );
        insta::assert_snapshot!(
            "arc-two-months-gif",
            format!(
                "{} bytes, SHA-256 {}",
                bytes.len(),
                to_hex(&Sha256::digest(&bytes))
            )
        );
    }

    #[test]
    fn a_postcard_is_its_shot_settled_at_its_scale() {
        let dir = tempfile::tempdir().unwrap();
        let shot = Shot {
            scene: preset::scene("thriving").unwrap(),
            label: vec!["14 mar 2024".to_owned()],
            clock: LocalClock::noon(preset::DATE),
        };
        let path = dir.path().join("fern.png");
        write_postcard(&shot, preset::SALT, 2, &path).unwrap();
        let (width, height, pixels) = read_png(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!((width, height), (1280, 148));
        let settled = still(Art::embedded(), ArtTable::embedded(), preset::SALT, &shot);
        let size = StripScale::exact(2);
        let mut expected = vec![[0; 4]; size.pixel_count()];
        upscale_into(&settled, &size, |rgba| rgba, &mut expected).unwrap();
        assert_eq!(pixels, expected);
    }
}
