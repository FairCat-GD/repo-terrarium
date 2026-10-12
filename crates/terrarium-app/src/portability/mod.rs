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
        written = store.export_observations(terrarium.id, file)?;
        Ok::<(), terrarium_store::StoreError>(())
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
    write_beside(path, |file| {
        days = climate::write(&lapse, terrarium.seed, file)?;
        Ok(())
    })
    .map_err(|err| match err {
        ClimateError::Replay(err) => copy::timelapse::replay_failed(&err),
        ClimateError::Io(err) => err.to_string(),
    })?;
    Ok(days)
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

#[cfg(test)]
mod tests {
    use terrarium_adapters::fixture;
    use terrarium_adapters::git::GitRepo;
    use terrarium_store::{NewTerrarium, RepoLocation};

    use super::*;
    use crate::files::beside;

    /// The fixture arc connected in a data directory of its own, holding its observations.
    fn arc() -> (tempfile::TempDir, DataDir, Terrarium) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fixture::arc(&repo).unwrap();
        let (branch, identity) = GitRepo::open(&repo).unwrap().default_identity().unwrap();
        let data = DataDir::at(dir.path().join("data"));
        let mut store = Store::open(&data).unwrap();
        let id = store
            .create_terrarium(&NewTerrarium {
                seed: identity.seed,
                repo: RepoLocation {
                    path: repo.display().to_string(),
                    name: "fern".to_owned(),
                    default_branch: branch,
                },
                install_salt: [0; 16],
                connected_at: fixture::ARC_START_SECS,
            })
            .unwrap();
        let observations = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/arc/observations.jsonl"),
        )
        .unwrap();
        store.import_observations(id, &observations[..]).unwrap();
        let terrarium = store.terrarium(id).unwrap().unwrap();
        (dir, data, terrarium)
    }

    #[test]
    fn the_exports_write_what_the_store_holds_and_every_finished_day() {
        let (dir, data, terrarium) = arc();
        let path = dir.path().join("fern-observations.jsonl");
        let written = export_observations(&data, &terrarium, &path).unwrap();
        let mut held = Vec::new();
        let rows = Store::open(&data)
            .unwrap()
            .export_observations(terrarium.id, &mut held)
            .unwrap();
        assert_eq!((written, std::fs::read(&path).unwrap()), (rows, held));
        assert!(rows > 300);

        let path = dir.path().join("fern-climate.json");
        let mut heard = Vec::new();
        let days = export_climate(
            &data,
            &terrarium,
            fixture::ARC_REPLAY_END_SECS,
            &mut |msg| heard.push(msg),
            &path,
        )
        .unwrap();
        assert!(heard.contains(&Msg::Reading(100)));
        assert_eq!(heard.last(), Some(&Msg::Writing));
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            value["days"].as_array().map(Vec::len),
            Some(usize::try_from(days).unwrap())
        );
        assert!(days > 800);
        assert!(!beside(&path).exists());
    }

    #[test]
    fn an_export_that_cannot_be_made_says_why_and_leaves_nothing() {
        let (dir, data, terrarium) = arc();
        let nowhere = dir.path().join("gone").join("fern-observations.jsonl");
        assert!(export_observations(&data, &terrarium, &nowhere).is_err());
        assert!(!beside(&nowhere).exists());
        let moved = Terrarium {
            repo: RepoLocation {
                path: dir.path().join("moved").display().to_string(),
                ..terrarium.repo.clone()
            },
            ..terrarium
        };
        let path = dir.path().join("fern-climate.json");
        let why = export_climate(
            &data,
            &moved,
            fixture::ARC_REPLAY_END_SECS,
            &mut |_| {},
            &path,
        )
        .unwrap_err();
        assert!(why.starts_with("The history could not be read"), "{why}");
        assert!(!path.exists() && !beside(&path).exists());
    }
}
