//! The terrarium's data, exported from the settings window: its observations as JSON Lines,
//! which another install imports, and its climate as JSON. Each is written on a thread of its
//! own, by way of a file beside its path, and answers the event loop. A panic ends that export
//! alone: the window says so, and the strip runs on.
//!
//! Importing changes the terrarium, so it is a lifecycle change (`crate::lifecycle`), not an
//! export.

pub mod climate;

use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::thread;

use terrarium_engine::timelapse::Lapse;
use terrarium_store::{DataDir, Store, Terrarium};
use winit::event_loop::EventLoopProxy;

use self::climate::ClimateError;
use crate::copy;
use crate::files::write_beside;
use crate::shell::DataFile;
use crate::strip::UserEvent;

/// What an export says.
#[derive(Debug, PartialEq, Eq)]
pub enum Msg {
    /// How far the history's read has come, in percent, for the climate.
    Reading(u8),
    /// The history is read; the file is being written.
    Writing,
    /// The export written to its path, or why not, worded.
    Written(Result<PathBuf, String>),
}

/// Writes `terrarium`'s observations as JSON Lines to `path`, as the store holds them now,
/// through a connection of its own. Returns how many it wrote.
pub fn export_observations(
    data: &DataDir,
    terrarium: &Terrarium,
    path: &Path,
) -> Result<u64, String> {
    let store = Store::open(data).map_err(|err| err.to_string())?;
    let mut written = 0;
    write_beside(path, |file| {
        written = store
            .export_observations(terrarium.id, file)
            .map_err(std::io::Error::other)?;
        Ok(())
    })
    .map_err(|err| err.to_string())?;
    Ok(written)
}

/// Writes `terrarium`'s climate through `at_secs` as JSON to `path`: its history read again
/// from its repository, with its stored observations, and replayed. `progress` hears how far
/// the read has come, in percent. Returns the days written.
pub fn export_climate(
    data: &DataDir,
    terrarium: &Terrarium,
    at_secs: i64,
    progress: &mut dyn FnMut(Msg),
    path: &Path,
) -> Result<u64, String> {
    let lapse = Lapse::for_terrarium(
        data,
        terrarium,
        at_secs,
        &mut |percent| progress(Msg::Reading(percent)),
        &AtomicBool::new(false),
    )
    .map_err(|err| copy::timelapse::failed(&err))?;
    progress(Msg::Writing);
    let mut days = 0;
    let mut failed = None;
    let written = write_beside(path, |file| {
        match climate::write(&lapse, terrarium.seed, file) {
            Ok(written) => days = written,
            Err(ClimateError::Io(err)) => return Err(err),
            Err(ClimateError::Replay(err)) => {
                failed = Some(copy::timelapse::replay_failed(&err));
                return Err(std::io::Error::other("the replay"));
            }
        }
        Ok(())
    });
    match (written, failed) {
        (_, Some(why)) => Err(why),
        (Err(err), None) => Err(err.to_string()),
        (Ok(()), None) => Ok(days),
    }
}

/// Exports `file` of `terrarium` to `path` on a thread of its own, answering as
/// [`UserEvent::Portability`] of `generation`.
pub fn spawn(
    proxy: &EventLoopProxy<UserEvent>,
    generation: u64,
    data: DataDir,
    terrarium: Terrarium,
    file: DataFile,
    path: PathBuf,
    at_secs: i64,
) {
    let proxy = proxy.clone();
    let sent = proxy.clone();
    let ran = thread::Builder::new()
        .name("terrarium-export".into())
        .spawn(move || {
            let say = |msg| {
                let _ = proxy.send_event(UserEvent::Portability(generation, msg));
            };
            let written = panic::catch_unwind(AssertUnwindSafe(|| match file {
                DataFile::Observations => export_observations(&data, &terrarium, &path),
                DataFile::Climate => {
                    export_climate(&data, &terrarium, at_secs, &mut |msg| say(msg), &path)
                }
            }))
            .unwrap_or_else(|_| Err(copy::settings::EXPORT_HALTED.to_owned()));
            say(Msg::Written(written.map(|_| path)));
        });
    if let Err(err) = ran {
        let msg = Msg::Written(Err(err.to_string()));
        let _ = sent.send_event(UserEvent::Portability(generation, msg));
    }
}
