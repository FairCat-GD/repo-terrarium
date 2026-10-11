//! The time-lapse's work off the loop: reading the history, replaying a range of it, and
//! writing an export, each on a thread of its own that answers through the event loop.
//!
//! A [`Session`] lives while the window is open. Its answers carry its generation, so one
//! from a window since closed is dropped; a film carries the request it answers, so one an
//! earlier request outran is dropped too. A panic on one of its threads ends that job alone:
//! the window says so, and the strip runs on.

use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use terrarium_compose::salt::InstallSalt;
use terrarium_compose::timelapse::Shot;
use terrarium_engine::timelapse::Lapse;
use terrarium_store::{DataDir, Terrarium};
use terrarium_truth::Tick;
use winit::event_loop::EventLoopProxy;

use super::{Film, write_gif, write_postcard};
use crate::copy;
use crate::strip::UserEvent;

/// What a time-lapse job says.
pub enum Msg {
    /// How far the history's read has come, in percent.
    Reading(u8),
    /// The history read, or why not, worded.
    Read(Result<Arc<Lapse>, String>),
    /// The film asked for by request `asked`, or why not, worded.
    Filmed {
        asked: u64,
        film: Result<Arc<Film>, String>,
    },
    /// How far the GIF's writing has come, in percent.
    Writing(u8),
    /// An export written to its path, or why not, worded.
    Written(Result<PathBuf, String>),
}

/// The time-lapse window's work.
pub struct Session {
    pub generation: u64,
    proxy: EventLoopProxy<UserEvent>,
    salt: InstallSalt,
    /// Stops the history's read once the window has closed.
    cancel: Arc<AtomicBool>,
    lapse: Option<(Arc<Lapse>, Arc<[Tick]>)>,
    /// The latest film asked for.
    asked: u64,
}

impl Session {
    /// Starts reading `terrarium`'s history through now, from its repository and the store
    /// in `data`.
    pub fn start(
        generation: u64,
        proxy: &EventLoopProxy<UserEvent>,
        data: DataDir,
        terrarium: Terrarium,
        at_secs: i64,
    ) -> Session {
        let cancel = Arc::new(AtomicBool::new(false));
        let session = Session {
            generation,
            proxy: proxy.clone(),
            salt: InstallSalt::new(terrarium.install_salt),
            cancel: Arc::clone(&cancel),
            lapse: None,
            asked: 0,
        };
        let progress = proxy.clone();
        let halted = |why| Msg::Read(Err(why));
        spawn(
            proxy,
            generation,
            "terrarium-time-lapse",
            halted,
            move || {
                let mut heard = |percent| {
                    let _ = progress
                        .send_event(UserEvent::TimeLapse(generation, Msg::Reading(percent)));
                };
                let read = Lapse::for_terrarium(&data, &terrarium, at_secs, &mut heard, &cancel);
                Msg::Read(
                    read.map(Arc::new)
                        .map_err(|err| copy::timelapse::failed(&err)),
                )
            },
        );
        session
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
            &self.proxy,
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
        let progress = self.proxy.clone();
        let halted = |why| Msg::Written(Err(why));
        spawn(
            &self.proxy,
            generation,
            "terrarium-time-lapse-gif",
            halted,
            move || {
                let mut last = None;
                let mut heard = |percent: u8| {
                    let step = percent / 5;
                    if last != Some(step) {
                        last = Some(step);
                        let _ = progress
                            .send_event(UserEvent::TimeLapse(generation, Msg::Writing(percent)));
                    }
                };
                Msg::Written(
                    write_gif(&film, salt, scale, &path, &mut heard)
                        .map(|()| path)
                        .map_err(|err| copy::timelapse::not_saved(&err)),
                )
            },
        );
    }

    /// Writes `shot`, settled, as a PNG at `scale` to `path`.
    pub fn write_postcard(&self, shot: Shot, scale: u32, path: PathBuf) {
        let salt = self.salt;
        let halted = |why| Msg::Written(Err(why));
        spawn(
            &self.proxy,
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

/// Runs `job` on a named thread and sends what it says; a panic sends `halted` with the
/// words for it.
fn spawn(
    proxy: &EventLoopProxy<UserEvent>,
    generation: u64,
    name: &str,
    halted: impl FnOnce(String) -> Msg + Send + 'static,
    job: impl FnOnce() -> Msg + Send + 'static,
) {
    let proxy = proxy.clone();
    let _ = thread::Builder::new().name(name.into()).spawn(move || {
        let msg = panic::catch_unwind(AssertUnwindSafe(job))
            .unwrap_or_else(|_| halted(copy::timelapse::HALTED.to_owned()));
        let _ = proxy.send_event(UserEvent::TimeLapse(generation, msg));
    });
}
