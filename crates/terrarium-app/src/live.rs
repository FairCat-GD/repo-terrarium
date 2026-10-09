//! The strip on a connected repository: the engine is started before the window, so the
//! composer is built with the terrarium's own salt from the first frame, and its updates are
//! carried to the event loop and turned into the scene and the label.
//!
//! The label reads the repository's name, then `branch - weather`, then one note when one
//! applies: `halted`, `repo not found`, the history read's progress, or the simulation's update.
//! When the repository is missing or the engine halted, the scene freezes where it was: no new
//! scene is shown until the repository is read again.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};

use terrarium_compose::preset;
use terrarium_compose::salt::InstallSalt;
use terrarium_engine::{Branch, Engine, EngineConfig, Sink, Start, Update};
use terrarium_store::DataDir;
use terrarium_truth::PresentationState;
use terrarium_truth::scene::{SceneInput, Weather};
use winit::event_loop::EventLoopProxy;

use crate::copy;
use crate::feed::Feed;
use crate::strip::{Mode, UserEvent};

/// The engine's updates, held until the event loop exists and passed on from then.
#[derive(Clone)]
pub struct Relay(Arc<Mutex<Link>>);

enum Link {
    Holding(Vec<Update>),
    Forwarding(EventLoopProxy<UserEvent>),
}

impl Relay {
    pub fn new() -> Relay {
        Relay(Arc::new(Mutex::new(Link::Holding(Vec::new()))))
    }

    /// Where the engine sends its updates.
    pub fn sink(&self) -> Sink {
        let link = Arc::clone(&self.0);
        Box::new(move |update| {
            let mut link = link.lock().unwrap_or_else(PoisonError::into_inner);
            match &mut *link {
                Link::Holding(held) => held.push(update),
                Link::Forwarding(proxy) => {
                    let _ = proxy.send_event(UserEvent::Engine(update));
                }
            }
        })
    }

    /// From now on, updates go to `proxy`: those held first, in order.
    pub fn forward_to(&self, proxy: EventLoopProxy<UserEvent>) {
        let mut link = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Link::Holding(held) = &mut *link {
            for update in held.drain(..) {
                let _ = proxy.send_event(UserEvent::Engine(update));
            }
        }
        *link = Link::Forwarding(proxy);
    }
}

/// A connected repository, as the strip runs it.
pub struct Connection {
    pub engine: Engine,
    pub relay: Relay,
    pub view: LiveView,
    pub salt: InstallSalt,
}

/// What the label says and the scene shows, from the engine's updates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveView {
    scene: SceneInput,
    facts: LabelFacts,
}

/// The facts the label words (`OverlayInput`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelFacts {
    pub repo_name: String,
    /// Where `HEAD` is, and the sky's weather; unknown until the first refresh.
    pub branch: Option<(Branch, Weather)>,
    pub note: Notes,
}

/// What may need a note, of which the label shows one (`copy::live_label`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Notes {
    pub halted: bool,
    pub repo_not_found: bool,
    pub progress: Option<u8>,
    pub sim_updated: bool,
}

/// What an update changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Changed {
    Nothing,
    Label,
    Scene,
}

/// The scene before truth has said anything: empty, under haze, which says nothing about health
/// (I-7) until the first refresh does.
pub fn unknown_scene() -> SceneInput {
    SceneInput {
        weather: Weather::Haze,
        ..preset::bare(PresentationState::Mature, Vec::new())
    }
}

impl LiveView {
    /// The view at connect: the cached state's scene, or [`unknown_scene`].
    pub fn new(repo_name: String, scene: Option<SceneInput>) -> LiveView {
        LiveView {
            scene: scene.unwrap_or_else(unknown_scene),
            facts: LabelFacts {
                repo_name,
                branch: None,
                note: Notes::default(),
            },
        }
    }

    pub fn scene(&self) -> &SceneInput {
        &self.scene
    }

    pub fn label(&self) -> Vec<String> {
        copy::live_label(&self.facts)
    }

    pub fn apply(&mut self, update: Update) -> Changed {
        let before = self.facts.clone();
        let notes = &mut self.facts.note;
        match update {
            Update::Halted => notes.halted = true,
            _ if notes.halted => {}
            Update::Progress(percent) => notes.progress = Some(percent),
            Update::RepoNotFound => notes.repo_not_found = true,
            Update::Snapshot(snapshot) => {
                notes.progress = None;
                notes.repo_not_found = false;
                notes.sim_updated = snapshot.sim_note;
                self.facts.branch = Some((snapshot.branch, snapshot.scene.weather));
                if snapshot.scene != self.scene {
                    self.scene = snapshot.scene;
                    return Changed::Scene;
                }
            }
            Update::Problem(problem) => eprintln!("{}", copy::engine_problem(&problem)),
        }
        if self.facts == before {
            Changed::Nothing
        } else {
            Changed::Label
        }
    }
}

/// Runs the strip on the repository at `repo`, or, with none, on the one connected last; with
/// nothing connected, on the sample terrarium.
pub fn run(repo: Option<PathBuf>) -> ExitCode {
    let data = match DataDir::resolve() {
        Ok(data) => data,
        Err(err) => {
            eprintln!("{}", copy::data_dir_unavailable(&err));
            return match repo {
                Some(_) => ExitCode::FAILURE,
                None => crate::run(&Mode::Live(Feed::Thriving), None),
            };
        }
    };
    let relay = Relay::new();
    let named = repo.clone();
    match Engine::spawn(EngineConfig::new(data, repo), relay.sink()) {
        Err(err) => {
            eprintln!("{}", copy::connect_failed(named.as_deref(), &err));
            ExitCode::FAILURE
        }
        Ok((_, Start::NotConnected)) => crate::run(&Mode::Live(Feed::Thriving), None),
        Ok((
            engine,
            Start::Connected {
                repo_name,
                install_salt,
                scene,
            },
        )) => crate::run(
            &Mode::Live(Feed::Live),
            Some(Connection {
                engine,
                relay,
                view: LiveView::new(repo_name, scene),
                salt: InstallSalt::new(install_salt),
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use terrarium_engine::schedule::Cause;
    use terrarium_engine::{Problem, RefreshStats, Replay, Snapshot};
    use terrarium_truth::climate::ClimateReport;
    use terrarium_truth::{Basis, CommitId, RepoSeed, SimState, TickGrid};

    use super::*;

    fn snapshot(weather: Weather, sim_note: bool) -> Update {
        let state = SimState::new(Basis::new(
            RepoSeed::from_bytes([1; 32]),
            TickGrid::from_origin(0).unwrap(),
            "main",
        ));
        let report: ClimateReport = state.report();
        Update::Snapshot(Box::new(Snapshot {
            scene: SceneInput {
                weather,
                ..terrarium_truth::project(&state)
            },
            report,
            branch: Branch::Named("main".into()),
            sim_note,
            stats: RefreshStats {
                cause: Cause::Connect,
                replay: Replay::Full,
                working: true,
                adapters_to_snapshot: Duration::ZERO,
            },
        }))
    }

    #[test]
    fn the_label_follows_the_engine_and_the_scene_freezes_while_the_repository_is_gone() {
        let mut view = LiveView::new("moss".into(), None);
        assert_eq!(view.scene().weather, Weather::Haze);
        assert!(view.scene().plants.is_empty());
        assert_eq!(view.label(), ["moss"]);
        assert_eq!(view.apply(Update::Progress(40)), Changed::Label);
        assert_eq!(view.label(), ["moss", "history 40%"]);
        assert_eq!(view.apply(snapshot(Weather::Clear, false)), Changed::Scene);
        assert_eq!(view.label(), ["moss", "main - clear"]);
        assert_eq!(
            view.apply(snapshot(Weather::Clear, false)),
            Changed::Nothing
        );
        assert_eq!(view.apply(Update::RepoNotFound), Changed::Label);
        assert_eq!(view.label(), ["moss", "main - clear", "repo not found"]);
        let frozen = view.scene().clone();
        assert_eq!(view.apply(Update::Progress(10)), Changed::Label);
        assert_eq!(view.label(), ["moss", "main - clear", "repo not found"]);
        assert_eq!(view.scene(), &frozen);
        // Found again: the next snapshot shows, and the note goes.
        assert_eq!(view.apply(snapshot(Weather::Storm, true)), Changed::Scene);
        assert_eq!(view.label(), ["moss", "main - storm", "sim updated"]);
        assert_eq!(
            view.apply(Update::Problem(Problem::Store("disk".into()))),
            Changed::Nothing
        );
    }

    #[test]
    fn a_halt_is_final() {
        let mut view = LiveView::new("moss".into(), None);
        view.apply(snapshot(Weather::Clear, false));
        assert_eq!(view.apply(Update::Halted), Changed::Label);
        assert_eq!(view.label(), ["moss", "main - clear", "halted"]);
        let frozen = view.scene().clone();
        assert_eq!(
            view.apply(snapshot(Weather::Storm, false)),
            Changed::Nothing
        );
        assert_eq!(view.scene(), &frozen);
        assert_eq!(view.apply(Update::RepoNotFound), Changed::Nothing);
    }

    #[test]
    fn a_detached_head_shows_its_commit() {
        let mut view = LiveView::new("moss".into(), None);
        let Update::Snapshot(mut snapshot) = snapshot(Weather::Overcast, false) else {
            unreachable!()
        };
        snapshot.branch = Branch::Detached(CommitId::from_hex(&"ab12cd3".repeat(6)[..40]).unwrap());
        view.apply(Update::Snapshot(snapshot));
        assert_eq!(view.label(), ["moss", "ab12cd3 - overcast"]);
    }
}
