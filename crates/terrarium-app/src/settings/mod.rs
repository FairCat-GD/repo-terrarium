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

use terrarium_adapters::ci::KeychainError;
use terrarium_engine::Tools;
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
use crate::portability::Msg;
use crate::prefs::{Pref, Prefs};
use crate::shell::{DataFile, Intent};

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
            adapters: AdaptersSection::new(&prefs.adapters, repo.binding.clone()),
            repo: RepoSection::new(repo),
            strip: StripSection::new(prefs),
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
        self.adapters.set_here(repo.binding.clone());
        self.repo.set_repo(repo);
    }

    /// What the Knip run and the GitHub Actions poll last said of themselves.
    pub fn set_tools(&mut self, tools: Tools) {
        self.adapters.set_tools(tools);
    }

    /// The keychain took the token change, or `problem` says why not.
    pub fn token_saved(&mut self, problem: Option<KeychainError>) {
        self.adapters.token_saved(problem);
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

    /// An import is done, and what it did, worded.
    pub fn imported(&mut self, report: Vec<String>) {
        self.repo.imported(report);
    }

    /// The save dialog for `file` answered.
    pub fn export_to(&mut self, file: DataFile, path: Option<PathBuf>) {
        self.repo.export_to(file, path);
    }

    /// What an export says.
    pub fn export_said(&mut self, msg: &Msg) {
        match msg {
            Msg::Written(written) => self.repo.exported(written),
            msg => self.repo.exporting(msg),
        }
    }

    /// The file picker for an import answered.
    pub fn import_chosen(&mut self, path: Option<PathBuf>) {
        self.repo.import_chosen(path);
    }

    /// The name the save dialog offers for `file`.
    pub fn file_name(&self, file: DataFile) -> String {
        self.repo.file_name(file)
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

    /// Whether `which`'s keys are registered with the OS: `None` when they are, else why not.
    pub fn hotkey_unregistered(&mut self, which: Which, why: Option<String>) {
        self.hotkeys.set_unregistered(which, why);
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
            .chain(self.adapters.take_token().map(Intent::Token))
            .collect()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for tab in Tab::ALL {
                ui.selectable_value(&mut self.tab, tab, tab.title());
            }
        });
        ui.separator();
        ui.add_space(4.0);
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
                binding: terrarium_adapters::binding::RepoBinding {
                    seed: "5e".repeat(32),
                    path: "/projects/moss".into(),
                },
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
        // The repository tab exporting, then asking to import, then reporting.
        form.show(Tab::Repository);
        form.export_to(DataFile::Climate, Some("/out/moss-climate.json".into()));
        form.export_said(&Msg::Reading(30));
        draw(&ctx, &mut form);
        form.export_said(&Msg::Written(Ok("/out/moss-climate.json".into())));
        form.import_chosen(Some("/in/moss-observations.jsonl".into()));
        draw(&ctx, &mut form);
        form.imported(vec!["Imported 3 observations; 0 were already here.".into()]);
        draw(&ctx, &mut form);
        let asked = form.take_intents();
        assert!(
            matches!(
                &asked[..],
                [Intent::Export {
                    file: DataFile::Climate,
                    ..
                }]
            ),
            "{asked:?}"
        );
        form.set_report(None);
        form.show(Tab::Adapters);
        draw(&ctx, &mut form);
        assert!(form.take_intents().is_empty(), "drawing asked for nothing");
    }

    #[test]
    fn the_adapters_tab_draws_every_word_the_tools_say_and_asks_nothing() {
        use terrarium_adapters::ci::{KeychainError, Slug};
        use terrarium_engine::{CiStatus, KnipStatus};

        let ctx = egui::Context::default();
        let mut form = form();
        form.show(Tab::Adapters);
        let slug = Slug::parse("example-garden/moss-bed").unwrap();
        let knip = [
            KnipStatus::Off,
            KnipStatus::Waiting,
            KnipStatus::Running,
            KnipStatus::Ran { unused: 1 },
            KnipStatus::NoNode,
            KnipStatus::NotInstalled,
            KnipStatus::Before6("5.66.0".into()),
            KnipStatus::Outside,
            KnipStatus::TimedOut,
            KnipStatus::Failed {
                code: Some(2),
                stderr: "ERROR: knip.json is not valid\n".into(),
            },
            KnipStatus::Failed {
                code: None,
                stderr: String::new(),
            },
            KnipStatus::NotAReport,
            KnipStatus::TooLarge,
            KnipStatus::CannotRun,
        ];
        let ci = [
            CiStatus::Quiet,
            CiStatus::NoRepository,
            CiStatus::NoToken,
            CiStatus::Reading(slug.clone()),
            CiStatus::Refused(slug.clone()),
            CiStatus::NotFound(slug.clone()),
            CiStatus::Unreachable(slug.clone()),
            CiStatus::Certificate(slug.clone()),
            CiStatus::Waiting(slug),
            CiStatus::Keychain(KeychainError::Refused),
        ];
        for (i, knip) in knip.iter().enumerate() {
            form.set_tools(Tools {
                knip: knip.clone(),
                ci: ci[i % ci.len()].clone(),
            });
            draw(&ctx, &mut form);
        }
        form.token_saved(Some(KeychainError::Failed));
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
