//! The strip on a connected repository: the engine's updates, carried to the event loop by its
//! proxy, turned into the scene and the label.
//!
//! The label reads the repository's name, then `branch - weather`, then one note when one
//! applies: `halted`, `repo not found` or the history read's progress; failing those, `tuning`,
//! then the simulation's update and a history rewrite, each until it has had its turn on the
//! plate once; failing those, how many days the terrarium is still settling, or since when it
//! has been dormant. A note shown once comes before those that last, so a dormancy of months
//! never holds it back. When the repository is missing or the engine halted, the scene freezes
//! where it was: no new scene is shown until the repository is read again.
//!
//! The view also keeps what the soil panel and the hover labels read: the climate report and
//! the engine's details beside it.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use terrarium_compose::preset;
use terrarium_engine::{Branch, Details, Update};
use terrarium_store::DataDir;
use terrarium_truth::PresentationState;
use terrarium_truth::climate::ClimateReport;
use terrarium_truth::scene::{SceneInput, Weather};

use crate::copy;
use crate::feed::Feed;
use crate::instance::{self, Claim, Instance, Quit};
use crate::pinning;
use crate::strip::Mode;

/// What a run with a data directory starts from: the repository `--repo` names, if one, and
/// the claim on being the one app running.
pub struct Launch {
    pub data: DataDir,
    pub repo: Option<PathBuf>,
    pub instance: Instance,
}

/// What the label says and the scene shows, from the engine's updates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveView {
    scene: SceneInput,
    facts: LabelFacts,
    /// What the soil panel reads, from the last snapshot.
    panel: Option<PanelFacts>,
}

/// What the soil panel and the hover labels word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelFacts {
    pub report: ClimateReport,
    pub details: Details,
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notes {
    pub halted: bool,
    pub repo_not_found: bool,
    pub progress: Option<u8>,
    /// Truth runs on a dev build's tuning override; nothing sets it until the override exists.
    pub tuning: bool,
    /// Notes shown once each, oldest first.
    pub once: Vec<Once>,
    /// Days until rain and wind are read against the repository's own pace.
    pub settling: Option<u32>,
    /// When the terrarium went dormant, in Unix seconds, while it is.
    pub dormant_since: Option<i64>,
}

/// The note the label shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    Halted,
    RepoNotFound,
    Progress(u8),
    Tuning,
    Once(Once),
    Settling(u32),
    /// Dormant since that moment, in Unix seconds.
    Dormant(i64),
}

impl Notes {
    /// The note the label shows: the first that applies, the most pressing first, then those
    /// shown once, then those that last.
    pub fn showing(&self) -> Option<Note> {
        if self.halted {
            Some(Note::Halted)
        } else if self.repo_not_found {
            Some(Note::RepoNotFound)
        } else if let Some(percent) = self.progress {
            Some(Note::Progress(percent))
        } else if self.tuning {
            Some(Note::Tuning)
        } else if let Some(&once) = self.once.first() {
            Some(Note::Once(once))
        } else if let Some(days) = self.settling {
            Some(Note::Settling(days))
        } else {
            self.dormant_since.map(Note::Dormant)
        }
    }
}

/// A note the label shows once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Once {
    SimUpdated,
    HistoryRewritten,
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
            panel: None,
        }
    }

    pub fn scene(&self) -> &SceneInput {
        &self.scene
    }

    /// The label's lines now.
    pub fn label(&self) -> Vec<String> {
        self.label_at(terrarium_engine::wall_secs())
    }

    /// The label's lines with the wall clock at `now`, in Unix seconds.
    pub fn label_at(&self, now: i64) -> Vec<String> {
        copy::live_label(&self.facts, now)
    }

    /// What the soil panel reads, once a snapshot has said.
    pub fn panel(&self) -> Option<&PanelFacts> {
        self.panel.as_ref()
    }

    pub fn apply(&mut self, update: Update) -> Changed {
        let before = (self.facts.clone(), self.panel.clone());
        let notes = &mut self.facts.note;
        match update {
            Update::Halted => notes.halted = true,
            _ if notes.halted => {}
            Update::Progress(percent) => notes.progress = Some(percent),
            Update::RepoNotFound => notes.repo_not_found = true,
            Update::Snapshot(snapshot) => {
                notes.progress = None;
                notes.repo_not_found = false;
                for (news, once) in [
                    (snapshot.notes.sim_updated, Once::SimUpdated),
                    (snapshot.notes.history_rewritten, Once::HistoryRewritten),
                ] {
                    if news && !notes.once.contains(&once) {
                        notes.once.push(once);
                    }
                }
                notes.settling = snapshot.report.settling_days_left;
                notes.dormant_since = snapshot.details.dormant_since;
                self.facts.branch = Some((snapshot.branch, snapshot.scene.weather));
                self.panel = Some(PanelFacts {
                    report: snapshot.report,
                    details: snapshot.details,
                });
                if snapshot.scene != self.scene {
                    self.scene = snapshot.scene;
                    return Changed::Scene;
                }
            }
            Update::Problem(problem) => eprintln!("{}", copy::engine_problem(&problem)),
            // The settings window's, not the strip's.
            Update::Tools(_) => {}
        }
        if (&self.facts, &self.panel) == (&before.0, &before.1) {
            Changed::Nothing
        } else {
            Changed::Label
        }
    }

    /// The engine started again on the same terrarium (a relink, a re-seed), as `repo_name`:
    /// what its last run said of the repository and itself no longer holds; the scene stands
    /// until the new run's first snapshot.
    pub fn restarted(&mut self, repo_name: String) -> Changed {
        let before = self.facts.clone();
        self.facts.repo_name = repo_name;
        let notes = &mut self.facts.note;
        notes.halted = false;
        notes.repo_not_found = false;
        notes.progress = None;
        if self.facts == before {
            Changed::Nothing
        } else {
            Changed::Label
        }
    }

    /// Whether the repository is not where it was connected from.
    pub fn repo_missing(&self) -> bool {
        self.facts.note.repo_not_found
    }

    /// The index in [`label`](Self::label) of the note shown once, while one shows.
    pub fn once_line(&self) -> Option<usize> {
        let shows = matches!(self.facts.note.showing(), Some(Note::Once(_)));
        shows.then(|| self.label().len() - 1)
    }

    /// The note shown once has had its turn: it goes, and the next, if any, takes its place.
    pub fn once_shown(&mut self) -> Changed {
        if self.once_line().is_none() {
            return Changed::Nothing;
        }
        self.facts.note.once.remove(0);
        Changed::Label
    }
}

/// Runs the strip on the repository at `repo`, or, with none, on the one connected last; with
/// nothing connected, the connect window opens. A second launch shows the running app's
/// settings and ends.
pub fn run(repo: Option<PathBuf>) -> ExitCode {
    let data = match DataDir::resolve() {
        Ok(data) => data,
        Err(err) => {
            eprintln!("{}", copy::data_dir_unavailable(&err));
            return ExitCode::FAILURE;
        }
    };
    let instance = match Instance::claim(&data) {
        Ok(Claim::First(instance)) => instance,
        Ok(Claim::Running) if repo.is_some() => {
            eprintln!("{}", copy::repo_while_running());
            return ExitCode::FAILURE;
        }
        Ok(Claim::Running) => {
            pinning::allow_foreground_for_running();
            return match instance::ask_to_show(&data) {
                Ok(()) => {
                    eprintln!("{}", copy::shown_in_running());
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    eprintln!("{}", copy::request_failed(&err));
                    ExitCode::FAILURE
                }
            };
        }
        Err(err) => {
            eprintln!("{}", copy::instance_unavailable(&err));
            return ExitCode::FAILURE;
        }
    };
    crate::run(
        &Mode::Live(Feed::Live),
        Some(Launch {
            data,
            repo,
            instance,
        }),
    )
}

/// How long `--quit` waits for the running app: its first connect can hold its loop for half
/// a minute before it hears the request.
const QUIT_WAIT: Duration = Duration::from_secs(60);

/// `--quit`: asks the app running on this data directory to quit, and waits for it. Ends well
/// when it quit, or when none was running.
pub fn quit() -> ExitCode {
    let data = match DataDir::resolve() {
        Ok(data) => data,
        Err(err) => {
            eprintln!("{}", copy::data_dir_unavailable(&err));
            return ExitCode::FAILURE;
        }
    };
    match instance::ask_to_quit(&data, QUIT_WAIT) {
        Ok(Quit::NotRunning | Quit::Ended) => ExitCode::SUCCESS,
        Ok(Quit::StillRunning) => {
            eprintln!("{}", copy::quit_timed_out());
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("{}", copy::quit_failed(&err));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use terrarium_engine::schedule::Cause;
    use terrarium_engine::{News, Problem, RefreshStats, Replay, Snapshot};
    use terrarium_truth::{Basis, CommitId, RepoSeed, SimState, TickGrid};

    use super::*;

    fn snapshot(weather: Weather, sim_updated: bool) -> Update {
        snapshot_with(
            weather,
            News {
                sim_updated,
                history_rewritten: false,
            },
        )
    }

    fn snapshot_with(weather: Weather, notes: News) -> Update {
        let state = SimState::new(Basis::new(
            RepoSeed::from_bytes([1; 32]),
            TickGrid::from_origin(0).unwrap(),
            "main",
        ));
        // Settled, unless a test says otherwise.
        let report = ClimateReport {
            settling_days_left: None,
            ..state.report()
        };
        Update::Snapshot(Box::new(Snapshot {
            scene: SceneInput {
                weather,
                ..terrarium_truth::project(&state)
            },
            report,
            details: Details::default(),
            branch: Branch::Named("main".into()),
            notes,
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
        assert_eq!(view.once_line(), Some(2));
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
    fn a_note_shown_once_goes_after_its_turn_and_the_next_follows() {
        let mut view = LiveView::new("moss".into(), None);
        assert_eq!(view.once_line(), None);
        let both = News {
            sim_updated: true,
            history_rewritten: true,
        };
        view.apply(snapshot_with(Weather::Clear, both));
        assert_eq!(view.label(), ["moss", "main - clear", "sim updated"]);
        // A more pressing note shows first, and the one shown once waits behind it.
        view.apply(Update::Progress(30));
        assert_eq!(view.once_line(), None);
        assert_eq!(view.once_shown(), Changed::Nothing);
        view.apply(snapshot_with(Weather::Clear, News::default()));
        assert_eq!(view.once_line(), Some(2));
        assert_eq!(view.once_shown(), Changed::Label);
        assert_eq!(view.label(), ["moss", "main - clear", "history rewritten"]);
        // The same news twice is one note.
        view.apply(snapshot_with(Weather::Clear, both));
        assert_eq!(view.once_shown(), Changed::Label);
        assert_eq!(view.label(), ["moss", "main - clear", "sim updated"]);
        assert_eq!(view.once_shown(), Changed::Label);
        assert_eq!(view.label(), ["moss", "main - clear"]);
        assert_eq!(view.once_line(), None);
    }

    #[test]
    fn a_restarted_engine_clears_what_the_last_run_said() {
        let mut view = LiveView::new("moss".into(), None);
        view.apply(snapshot(Weather::Clear, false));
        let shown = view.scene().clone();
        view.apply(Update::RepoNotFound);
        assert_eq!(view.restarted("fern".into()), Changed::Label);
        assert_eq!(view.label(), ["fern", "main - clear"]);
        assert_eq!(view.scene(), &shown);
        view.apply(Update::Halted);
        view.restarted("fern".into());
        assert_eq!(view.apply(snapshot(Weather::Storm, false)), Changed::Scene);
        assert_eq!(view.restarted("fern".into()), Changed::Nothing);
    }

    /// 2024-06-15, 12:00 UTC.
    const JUNE: i64 = 1_718_452_800;
    const DAY: i64 = 86_400;

    fn noted(view: &mut LiveView, settling: Option<u32>, dormant_since: Option<i64>) {
        let Update::Snapshot(mut snapshot) = snapshot(Weather::Clear, false) else {
            unreachable!()
        };
        snapshot.report.settling_days_left = settling;
        snapshot.details.dormant_since = dormant_since;
        view.apply(Update::Snapshot(snapshot));
    }

    #[test]
    fn a_settling_terrarium_counts_its_days_and_a_dormant_one_names_its_month_or_year() {
        let mut view = LiveView::new("moss".into(), None);
        noted(&mut view, Some(9), None);
        assert_eq!(
            view.label_at(JUNE),
            ["moss", "main - clear", "settling 9 days"]
        );
        noted(&mut view, Some(1), None);
        assert_eq!(view.label_at(JUNE).last().unwrap(), "settling 1 day");
        noted(&mut view, None, Some(JUNE));
        let at = |now: i64| view.label_at(now).last().cloned().unwrap();
        assert_eq!(at(JUNE), "dormant since jun");
        // Eleven months on, the month still says it; a year on, the year does.
        assert_eq!(at(JUNE + 330 * DAY), "dormant since jun");
        assert_eq!(at(JUNE + 366 * DAY), "dormant since 2024");
        noted(&mut view, None, None);
        assert_eq!(view.label_at(JUNE), ["moss", "main - clear"]);
    }

    #[test]
    fn a_note_shown_once_comes_before_one_that_lasts() {
        let mut view = LiveView::new("moss".into(), None);
        noted(&mut view, None, Some(JUNE));
        assert_eq!(view.once_line(), None);
        let Update::Snapshot(mut news) = snapshot(Weather::Clear, true) else {
            unreachable!()
        };
        news.details.dormant_since = Some(JUNE);
        view.apply(Update::Snapshot(news));
        assert_eq!(view.label_at(JUNE).last().unwrap(), "sim updated");
        assert_eq!(view.once_line(), Some(2));
        assert_eq!(view.once_shown(), Changed::Label);
        assert_eq!(view.label_at(JUNE).last().unwrap(), "dormant since jun");
        assert_eq!(view.once_line(), None);
        // Tuning comes before both, and the pressing notes before it.
        view.facts.note.tuning = true;
        view.facts.note.once.push(Once::HistoryRewritten);
        assert_eq!(view.label_at(JUNE).last().unwrap(), "tuning");
        assert_eq!(view.once_line(), None);
        view.apply(Update::Progress(5));
        assert_eq!(view.label_at(JUNE).last().unwrap(), "history 5%");
    }

    #[test]
    fn every_note_fits_the_plate() {
        let mut notes = vec![
            Note::Halted,
            Note::RepoNotFound,
            Note::Progress(100),
            Note::Tuning,
            Note::Once(Once::SimUpdated),
            Note::Once(Once::HistoryRewritten),
            Note::Settling(14),
            Note::Settling(1),
        ];
        // Every month, and a year long ago and far ahead.
        notes.extend((0..12).map(|month| Note::Dormant(JUNE + month * 31 * DAY)));
        notes.extend([
            Note::Dormant(-3_000_000_000),
            Note::Dormant(250_000_000_000),
        ]);
        for note in notes {
            let mut view = LiveView::new("moss".into(), None);
            view.facts.note = Notes::default();
            match note {
                Note::Halted => view.facts.note.halted = true,
                Note::RepoNotFound => view.facts.note.repo_not_found = true,
                Note::Progress(percent) => view.facts.note.progress = Some(percent),
                Note::Tuning => view.facts.note.tuning = true,
                Note::Once(once) => view.facts.note.once.push(once),
                Note::Settling(days) => view.facts.note.settling = Some(days),
                Note::Dormant(since) => view.facts.note.dormant_since = Some(since),
            }
            assert_eq!(view.facts.note.showing(), Some(note));
            let line = view.label_at(JUNE + 365 * DAY).pop().unwrap();
            assert!(line.chars().count() <= 18, "{line:?} is too long");
        }
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
