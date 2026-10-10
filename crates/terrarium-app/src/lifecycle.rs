//! Connecting from the connect window, relinking, re-seeding and disconnecting, run on a thread
//! of their own so the event loop never waits: the engine running is stopped and its thread
//! joined, so nothing it writes follows; the store is changed (`terrarium_engine::lifecycle`);
//! a new engine starts; and the outcome comes back to the event loop as one event.
//!
//! Each engine's updates carry its generation, and the strip drops those of an engine it has
//! replaced.

use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::thread;

use terrarium_engine::lifecycle as change;
use terrarium_engine::{ConnectError, Engine, EngineConfig, Sink, Start};
use terrarium_store::identifiers::SETTING_LOGIN_OPEN_AT_LOGIN;
use terrarium_store::{DataDir, InstallKey, Store, Terrarium, TerrariumId};
use terrarium_truth::scene::SceneInput;
use winit::event_loop::EventLoopProxy;

use crate::strip::UserEvent;
use crate::{copy, discover};

/// What to change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    /// Connects the repository at `path` from the connect window; `first_run` holds the first
    /// run's "Open at login", which is stored with it.
    Connect {
        path: PathBuf,
        first_run: Option<bool>,
    },
    Relink {
        id: TerrariumId,
        path: PathBuf,
    },
    Reseed {
        id: TerrariumId,
    },
    Disconnect {
        id: TerrariumId,
        delete_observations: bool,
    },
}

/// How a job ended.
pub enum Outcome {
    /// Connected: the engine running on it, what the strip starts from, and the discovery
    /// report shown once.
    Connected {
        engine: Engine,
        terrarium: Box<Terrarium>,
        /// The cached state's scene, if it is still valid.
        scene: Option<SceneInput>,
        report: String,
    },
    /// Relinked or re-seeded, or refused (`changed`), and the engine started again on what
    /// the store now holds.
    Restarted {
        changed: Result<(), ConnectError>,
        started: Result<(Engine, Box<Terrarium>), ConnectError>,
    },
    Disconnected,
    /// Nothing was connected, and why.
    NotConnected(ConnectError),
}

/// A sink for the engine of `generation`: its updates go to the event loop, tagged.
pub fn sink(proxy: EventLoopProxy<UserEvent>, generation: u64) -> Sink {
    Box::new(move |update| {
        let _ = proxy.send_event(UserEvent::Engine(generation, update));
    })
}

/// Runs `job` on a thread of its own, after stopping `engine`; the new engine is of
/// `generation`, and the outcome is sent as [`UserEvent::Lifecycle`].
pub fn spawn(
    job: Job,
    engine: Option<Engine>,
    data: DataDir,
    proxy: EventLoopProxy<UserEvent>,
    generation: u64,
) {
    let send = proxy.clone();
    let ran = thread::Builder::new()
        .name("terrarium-lifecycle".into())
        .spawn(move || {
            if let Some(engine) = engine {
                engine.shutdown_and_join();
            }
            let outcome = run(job, &data, &proxy, generation);
            let _ = proxy.send_event(UserEvent::Lifecycle(generation, Box::new(outcome)));
        });
    if let Err(err) = ran {
        let outcome = Outcome::NotConnected(ConnectError::Thread(err));
        let _ = send.send_event(UserEvent::Lifecycle(generation, Box::new(outcome)));
    }
}

fn run(job: Job, data: &DataDir, proxy: &EventLoopProxy<UserEvent>, generation: u64) -> Outcome {
    type Started = (Engine, Box<Terrarium>, Option<SceneInput>);
    let start = |repo: Option<PathBuf>| -> Result<Started, ConnectError> {
        let config = EngineConfig::new(data.clone(), repo);
        match Engine::spawn(config, sink(proxy.clone(), generation))? {
            (engine, Start::Connected { terrarium, scene }) => Ok((engine, terrarium, scene)),
            (_, Start::NotConnected) => Err(ConnectError::Halted),
        }
    };
    let restart = |changed: Result<(), ConnectError>| Outcome::Restarted {
        changed,
        started: start(None).map(|(engine, terrarium, _)| (engine, terrarium)),
    };
    match job {
        Job::Connect { path, first_run } => match start(Some(path)) {
            Ok((engine, terrarium, scene)) => {
                if let Some(open_at_login) = first_run
                    && let Err(err) = first_run_done(data, open_at_login)
                {
                    eprintln!("{}", copy::connect_failed(None, &err));
                }
                let report = panic::catch_unwind(AssertUnwindSafe(|| {
                    discover::at_connect(data, &terrarium)
                }))
                .unwrap_or_else(|_| copy::report_unavailable().to_owned());
                Outcome::Connected {
                    engine,
                    terrarium,
                    scene,
                    report,
                }
            }
            Err(err) => Outcome::NotConnected(err),
        },
        Job::Relink { id, path } => restart(change::relink(data, id, &path).map(|_| ())),
        Job::Reseed { id } => restart(change::reseed(data, id).map(|_| ())),
        Job::Disconnect {
            id,
            delete_observations,
        } => match change::disconnect(data, id, delete_observations) {
            Ok(()) => Outcome::Disconnected,
            Err(err) => restart(Err(err)),
        },
    }
}

/// The first run is done, with "Open at login" as chosen: it is never offered again.
fn first_run_done(data: &DataDir, open_at_login: bool) -> Result<(), ConnectError> {
    let mut store = Store::open(data)?;
    store.set_setting(SETTING_LOGIN_OPEN_AT_LOGIN, &open_at_login)?;
    store.set_install_value(InstallKey::FirstRunDone, "true")?;
    Ok(())
}

/// Whether the first run is still to come: nothing has been connected from the connect window.
pub fn first_run(data: &DataDir) -> bool {
    Store::open(data)
        .and_then(|store| store.install_value(InstallKey::FirstRunDone))
        .is_ok_and(|done| done.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_run_is_done_once_and_keeps_the_login_choice() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::at(dir.path());
        assert!(first_run(&data));
        first_run_done(&data, false).unwrap();
        assert!(!first_run(&data));
        let store = Store::open(&data).unwrap();
        assert_eq!(
            store.setting::<bool>(SETTING_LOGIN_OPEN_AT_LOGIN).unwrap(),
            Some(false)
        );
    }
}
