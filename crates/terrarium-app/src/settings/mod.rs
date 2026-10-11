//! The settings window: a tab for each part of the app, each a model the window only draws.
//!
//! What a press asks for is an [`Intent`] the strip carries out. That is either a lifecycle
//! change, or a setting ([`Pref`]) the strip applies at once and then saves, so nothing is
//! registered, moved or written from inside the drawing. The OS's light or dark appearance
//! applies to this window only; the terrarium keeps its own day and night.

mod adapters;
mod general;
mod hotkeys;
mod repo;
mod strip;
mod time;

use std::path::PathBuf;

use terrarium_platform::login::LoginStatus;

use self::adapters::AdaptersSection;
use self::general::GeneralSection;
use self::hotkeys::HotkeysSection;
pub use self::repo::RepoFacts;
use self::repo::RepoSection;
pub use self::strip::MonitorChoice;
use self::strip::StripSection;
use self::time::TimeSection;
use crate::copy::settings as words;
use crate::hotkey::Which;
use crate::prefs::{Pref, Prefs};
use crate::shell::Intent;

/// The window's tabs, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Repository,
    Strip,
    Adapters,
    Hotkeys,
    Time,
    General,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Repository,
        Tab::Strip,
        Tab::Adapters,
        Tab::Hotkeys,
        Tab::Time,
        Tab::General,
    ];

    fn title(self) -> &'static str {
        match self {
            Tab::Repository => words::REPOSITORY,
            Tab::Strip => words::STRIP,
            Tab::Adapters => words::ADAPTERS,
            Tab::Hotkeys => words::HOTKEYS,
            Tab::Time => words::TIME,
            Tab::General => words::GENERAL,
        }
    }
}

pub struct SettingsForm {
    tab: Tab,
    repo: RepoSection,
    strip: StripSection,
    adapters: AdaptersSection,
    hotkeys: HotkeysSection,
    time: TimeSection,
    general: GeneralSection,
    /// The settings changed since the strip last took them.
    changes: Vec<Pref>,
    /// Why the last setting could not be saved, said until the next is.
    problem: Option<String>,
}

impl SettingsForm {
    /// The window for the connected repository `repo`, showing `prefs`; the crash log writes
    /// into `logs`.
    pub fn new(repo: RepoFacts, prefs: &Prefs, logs: String) -> SettingsForm {
        SettingsForm {
            tab: Tab::Repository,
            repo: RepoSection::new(repo),
            strip: StripSection::new(prefs),
            adapters: AdaptersSection::new(&prefs.adapters),
            hotkeys: HotkeysSection::new(prefs),
            time: TimeSection::new(prefs),
            general: GeneralSection::new(prefs, logs),
            changes: Vec::new(),
            problem: None,
        }
    }

    #[cfg(test)]
    pub fn show(&mut self, tab: Tab) {
        self.tab = tab;
    }

    pub fn set_repo(&mut self, repo: RepoFacts) {
        self.repo.set_repo(repo);
    }

    pub fn is_missing(&self) -> bool {
        self.repo.is_missing()
    }

    pub fn set_missing(&mut self, missing: bool) {
        self.repo.set_missing(missing);
    }

    pub fn located(&mut self, path: Option<PathBuf>) {
        self.repo.located(path);
    }

    /// The lifecycle change asked for ended: `problem` says why it failed.
    pub fn finished(&mut self, problem: Option<String>) {
        self.repo.finished(problem);
    }

    /// Shows `prefs`, as the strip now runs on them, in every tab.
    pub fn set_prefs(&mut self, prefs: &Prefs) {
        self.strip.set_prefs(prefs);
        self.adapters.set_settings(&prefs.adapters);
        self.hotkeys.set_prefs(prefs);
        self.time.set_prefs(prefs);
        self.general.set_prefs(prefs);
    }

    /// The displays connected, and the strip's scales on the one it is on.
    pub fn set_monitors(&mut self, monitors: Vec<MonitorChoice>, scales: Option<[u32; 2]>) {
        self.strip.set_monitors(monitors, scales);
    }

    /// The discovery report for the adapters' tab; `None` while it is being made.
    pub fn set_report(&mut self, report: Option<String>) {
        self.adapters.set_report(report);
    }

    pub fn set_login_status(&mut self, status: LoginStatus) {
        self.general.set_login_status(status);
    }

    /// The strip saved `pref`, or says why it could not; `login` is why the OS would not open
    /// the app at login.
    pub fn saved(&mut self, pref: &Pref, problem: Option<String>, login: Option<String>) {
        if let Pref::OpenAtLogin(_) = pref {
            self.general.login_problem(login);
        }
        self.problem = problem;
    }

    /// The OS refused `which`'s new keys, for `why`; the keys before stay.
    pub fn hotkey_refused(&mut self, which: Which, why: String) {
        self.hotkeys.refused(which, why);
    }

    /// The repository tab, as a test presses it.
    #[cfg(test)]
    pub fn repo(&mut self) -> &mut RepoSection {
        &mut self.repo
    }

    pub fn take_intents(&mut self) -> Vec<Intent> {
        self.repo
            .take_intent()
            .into_iter()
            .chain(self.changes.drain(..).map(Intent::Set))
            .collect()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for tab in Tab::ALL {
                ui.selectable_value(&mut self.tab, tab, tab.title());
            }
        });
        ui.separator();
        let changes = &mut self.changes;
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| match self.tab {
                Tab::Repository => self.repo.ui(ui),
                Tab::Strip => self.strip.ui(ui, changes),
                Tab::Adapters => self.adapters.ui(ui, changes),
                Tab::Hotkeys => self.hotkeys.ui(ui, changes),
                Tab::Time => self.time.ui(ui, changes),
                Tab::General => self.general.ui(ui, changes),
            });
        if let Some(problem) = &self.problem {
            ui.separator();
            ui.label(problem);
        }
    }
}

#[cfg(test)]
mod tests {
    use terrarium_compose::scale::SizeStep;
    use terrarium_platform::geometry::Edge;

    use super::*;
    use crate::monitor::MonitorId;
    use crate::prefs::AdapterKind;

    fn form() -> SettingsForm {
        SettingsForm::new(
            RepoFacts {
                name: "moss".into(),
                path: "/projects/moss".into(),
                default_branch: "main".into(),
            },
            &Prefs::default(),
            "/data/logs".into(),
        )
    }

    fn monitors() -> Vec<MonitorChoice> {
        vec![
            MonitorChoice {
                id: MonitorId("display-a".into()),
                label: "Garden Display (5120×2880, main)".into(),
            },
            MonitorChoice {
                id: MonitorId("display-b".into()),
                label: "Fern 27 (2560×1440)".into(),
            },
        ]
    }

    /// Draws `form` once, headless.
    fn draw(ctx: &egui::Context, form: &mut SettingsForm) {
        let output = ctx.run_ui(egui::RawInput::default(), |ui| form.ui(ui));
        output.drop_without_applying_deltas();
    }

    #[test]
    fn every_tab_draws_in_every_state_and_drawing_changes_nothing() {
        let ctx = egui::Context::default();
        let mut form = form();
        for tab in Tab::ALL {
            form.show(tab);
            draw(&ctx, &mut form);
        }
        // With everything a tab can show: a report, displays, a stored display gone, a refusal,
        // login waiting, problems.
        let mut prefs = Prefs {
            monitor: Some(MonitorId("gone".into())),
            edge: Edge::Top,
            size_step: SizeStep::OneSmaller,
            crash_log: true,
            ..Prefs::default()
        };
        prefs.adapters.overrides.coverage = Some("out/lcov.info".into());
        prefs.adapters.enabled.weather = false;
        form.set_prefs(&prefs);
        form.set_monitors(monitors(), Some([4, 3]));
        form.set_report(Some("Found\n  git  default branch main\n".into()));
        form.set_login_status(LoginStatus::RequiresApproval);
        form.hotkey_refused(Which::Raise, "These keys could not be registered.".into());
        form.saved(
            &Pref::OpenAtLogin(true),
            Some("The setting could not be saved.".into()),
            Some("Open at login applies to the installed app.".into()),
        );
        form.set_missing(true);
        for tab in Tab::ALL {
            form.show(tab);
            draw(&ctx, &mut form);
        }
        form.set_report(None);
        form.show(Tab::Adapters);
        draw(&ctx, &mut form);
        assert!(form.take_intents().is_empty(), "drawing asked for nothing");
    }

    #[test]
    fn a_change_in_any_tab_reaches_the_strip_once_in_order() {
        let mut form = form();
        form.strip.set_monitors(monitors(), Some([4, 3]));
        let mut changes = Vec::new();
        changes.extend(form.strip.choose_edge(Edge::Top));
        changes.extend(form.adapters.set_on(AdapterKind::Tests, false));
        changes.extend(form.general.set_crash_log(true));
        form.changes.extend(changes.iter().cloned());
        let intents = form.take_intents();
        assert_eq!(
            intents,
            changes.into_iter().map(Intent::Set).collect::<Vec<_>>()
        );
        assert_eq!(intents.len(), 3);
        assert!(form.take_intents().is_empty());
    }
}
