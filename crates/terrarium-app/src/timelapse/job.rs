//! The time-lapse's work off the loop: reading the history, replaying a range of it, and
//! writing an export, each on a thread of its own that answers through the event loop.
//!
//! A [`Session`] lives while the window is open, and answers through the callback it was given
//! (the strip forwards it to the event loop). Its answers carry its generation, so one from a
//! window since closed is dropped; a film carries the request it answers, so one an earlier
//! request outran is dropped too. A panic on one of its threads ends that job alone:
//! the window says so, and the strip runs on.

use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use terrarium_compose::salt::InstallSalt;
use terrarium_compose::timelapse::Shot;
use terrarium_engine::timelapse::Lapse;
use terrarium_store::{DataDir, Terrarium};
use terrarium_truth::Tick;

use terrarium_adapters::exec::{Tool, find_ffmpeg};

use super::mp4::write_mp4;
use super::{Film, write_gif, write_postcard};
use crate::copy;

/// Where a session's answers go, with the generation of the session that sends them.
pub type Answer = Arc<dyn Fn(u64, Msg) + Send + Sync>;

/// What a time-lapse job says.
pub enum Msg {
    /// The person's ffmpeg, if one was found: the MP4 is offered only then.
    Ffmpeg(Option<Tool>),
    /// How far the history's read has come, in percent.
    Reading(u8),
    /// The history read, or why not, worded.
    Read(Result<Arc<Lapse>, String>),
    /// The film asked for by request `asked`, or why not, worded.
    Filmed {
        asked: u64,
        film: Result<Arc<Film>, String>,
    },
    /// How far a GIF's or an MP4's writing has come, in percent.
    Writing(u8),
    /// An export written to its path, or why not, worded.
    Written(Result<PathBuf, String>),
}

/// The time-lapse window's work.
pub struct Session {
    pub generation: u64,
    answer: Answer,
    salt: InstallSalt,
    /// Stops the history's read once the window has closed.
    cancel: Arc<AtomicBool>,
    lapse: Option<(Arc<Lapse>, Arc<[Tick]>)>,
    /// The latest film asked for.
    asked: u64,
}

impl Session {
    /// Starts reading `terrarium`'s history through now, from its repository and the store
    /// in `data`, after looking for the person's ffmpeg, never inside the repository.
    pub fn start(
        generation: u64,
        answer: &Answer,
        data: DataDir,
        terrarium: Terrarium,
        at_secs: i64,
    ) -> Session {
        let session = Session::new(generation, answer, InstallSalt::new(terrarium.install_salt));
        let cancel = Arc::clone(&session.cancel);
        let progress = Arc::clone(answer);
        let halted = |why| Msg::Read(Err(why));
        spawn(
            answer,
            generation,
            "terrarium-time-lapse",
            halted,
            move || {
                let ffmpeg = find_ffmpeg(Some(Path::new(&terrarium.repo.path)));
                progress(generation, Msg::Ffmpeg(ffmpeg));
                let mut heard = |percent| progress(generation, Msg::Reading(percent));
                let read = Lapse::for_terrarium(&data, &terrarium, at_secs, &mut heard, &cancel);
                Msg::Read(
                    read.map(Arc::new)
                        .map_err(|err| copy::timelapse::failed(&err)),
                )
            },
        );
        session
    }

    /// A session reading nothing yet, answering through `answer`.
    pub(super) fn new(generation: u64, answer: &Answer, salt: InstallSalt) -> Session {
        Session {
            generation,
            answer: Arc::clone(answer),
            salt,
            cancel: Arc::new(AtomicBool::new(false)),
            lapse: None,
            asked: 0,
        }
    }

    /// The flag that stops the history's read, which the session's end raises.
    #[cfg(test)]
    pub(super) fn cancel(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Takes the history read.
    pub fn read(&mut self, lapse: Arc<Lapse>) {
        let closes = lapse.closes().into();
        self.lapse = Some((lapse, closes));
    }

    /// The UTC day each close ends, as days since 1970-01-01.
    pub fn days(&self) -> Vec<i64> {
        self.lapse
            .as_ref()
            .map(|(lapse, closes)| closes.iter().map(|&close| lapse.day(close)).collect())
            .unwrap_or_default()
    }

    /// Asks for the film of the days `from..=to`, by day or by night; an earlier request's
    /// answer is dropped from now on.
    pub fn film(&mut self, from: usize, to: usize, night: bool) {
        let Some((lapse, closes)) = self.lapse.clone() else {
            return;
        };
        self.asked += 1;
        let asked = self.asked;
        let halted = move |why| Msg::Filmed {
            asked,
            film: Err(why),
        };
        spawn(
            &self.answer,
            self.generation,
            "terrarium-time-lapse",
            halted,
            move || {
                let film = Film::new(&lapse, &closes, from, to, night)
                    .map(Arc::new)
                    .map_err(|err| copy::timelapse::replay_failed(&err));
                Msg::Filmed { asked, film }
            },
        );
    }

    /// Whether a film is the latest asked for.
    pub fn is_latest(&self, asked: u64) -> bool {
        asked == self.asked
    }

    /// Writes `film` as a GIF at `scale` to `path`.
    pub fn write_gif(&self, film: Arc<Film>, scale: u32, path: PathBuf) {
        let (generation, salt) = (self.generation, self.salt);
        let progress = Arc::clone(&self.answer);
        let halted = |why| Msg::Written(Err(why));
        spawn(
            &self.answer,
            generation,
            "terrarium-time-lapse-gif",
            halted,
            move || {
                let mut heard = in_steps(progress, generation);
                Msg::Written(
                    write_gif(&film, salt, scale, &path, &mut heard)
                        .map(|()| path)
                        .map_err(|err| copy::timelapse::not_saved(&err)),
                )
            },
        );
    }

    /// Writes `film` as an MP4 at `scale`, or 4× if larger, to `path` through `ffmpeg`.
    pub fn write_mp4(&self, film: Arc<Film>, scale: u32, ffmpeg: Tool, path: PathBuf) {
        let (generation, salt) = (self.generation, self.salt);
        let progress = Arc::clone(&self.answer);
        let halted = |why| Msg::Written(Err(why));
        spawn(
            &self.answer,
            generation,
            "terrarium-time-lapse-mp4",
            halted,
            move || {
                let mut heard = in_steps(progress, generation);
                Msg::Written(
                    write_mp4(&film, salt, scale, &ffmpeg, &path, &mut heard)
                        .map(|()| path)
                        .map_err(|err| copy::timelapse::not_saved(&copy::ffmpeg_problem(&err))),
                )
            },
        );
    }

    /// Writes `shot`, settled, as a PNG at `scale` to `path`.
    pub fn write_postcard(&self, shot: Shot, scale: u32, path: PathBuf) {
        let salt = self.salt;
        let halted = |why| Msg::Written(Err(why));
        spawn(
            &self.answer,
            self.generation,
            "terrarium-time-lapse-png",
            halted,
            move || {
                Msg::Written(
                    write_postcard(&shot, salt, scale, &path)
                        .map(|()| path)
                        .map_err(|err| copy::timelapse::not_saved(&err)),
                )
            },
        );
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// A writing's progress, sent as it passes each 5%.
fn in_steps(answer: Answer, generation: u64) -> impl FnMut(u8) {
    let mut last = None;
    move |percent: u8| {
        let step = percent / 5;
        if last != Some(step) {
            last = Some(step);
            answer(generation, Msg::Writing(percent));
        }
    }
}

/// Runs `job` on a named thread and sends what it says; a panic sends `halted` with the
/// words for it.
fn spawn(
    answer: &Answer,
    generation: u64,
    name: &str,
    halted: impl FnOnce(String) -> Msg + Send + 'static,
    job: impl FnOnce() -> Msg + Send + 'static,
) {
    let answer = Arc::clone(answer);
    let _ = thread::Builder::new().name(name.into()).spawn(move || {
        let msg = panic::catch_unwind(AssertUnwindSafe(job))
            .unwrap_or_else(|_| halted(copy::timelapse::HALTED.to_owned()));
        answer(generation, msg);
    });
}
