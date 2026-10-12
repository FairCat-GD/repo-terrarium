//! The adapters tab (`adapters.*`): what the repository has for each adapter, with a line on
//! how each missing one connects, for each report whether it is read and from where, and
//! whether each metric of the committed source is measured.
//!
//! Running Knip, and the GitHub repository typed for the CI weather, each hold for the
//! connected repository at its folder (I-5): a switch on for one repository is off for the
//! next. The token is saved to the keychain by the strip's writer, never as a setting, and the
//! field never shows it again; whether one is saved is what the poll says.

use terrarium_adapters::binding::{CiRepository, RepoBinding};
use terrarium_adapters::ci::{KeychainError, Slug, Token};
use terrarium_adapters::discovery::{Enabled, Overrides, configured_path};
use terrarium_engine::Tools;
use terrarium_engine::lifecycle::AdapterSettings;

use crate::copy::settings as words;
use crate::prefs::{AdapterKind, Pref, TokenChange};

pub struct AdaptersSection {
    enabled: Enabled,
    /// The paths the strip reads, and the ones typed and not applied yet.
    applied: Overrides,
    /// By [`index`]: the reports only, since a metric has no path.
    typed: [String; 3],
    /// The report, once discovery has made it.
    report: Option<String>,
    /// A path typed that was refused, and for which report.
    refused: Option<AdapterKind>,
    /// The connected repository, as a setting given for it names it.
    here: RepoBinding,
    /// The repository Knip may run in, and the GitHub repository typed, as the strip runs on
    /// them: each for the repository it was given for.
    knip_run: Option<RepoBinding>,
    ci_repository: Option<CiRepository>,
    typed_repository: String,
    repository_refused: bool,
    typed_token: String,
    token_refused: bool,
    /// The token saved or removed, for the strip to carry out.
    token: Option<TokenChange>,
    /// Why the keychain would not take the last change.
    keychain_problem: Option<KeychainError>,
    /// What the Knip run and the poll last said of themselves.
    tools: Tools,
}

/// Where a report's typed path is kept; `None` for a metric, which has no path.
fn index(kind: AdapterKind) -> Option<usize> {
    match kind {
        AdapterKind::Coverage => Some(0),
        AdapterKind::Tests => Some(1),
        AdapterKind::DeadCode => Some(2),
        AdapterKind::Complexity | AdapterKind::TodoDensity => None,
    }
}

impl AdaptersSection {
    pub fn new(settings: &AdapterSettings, here: RepoBinding) -> AdaptersSection {
        let mut section = AdaptersSection {
            enabled: settings.enabled,
            applied: Overrides::default(),
            typed: Default::default(),
            report: None,
            refused: None,
            here,
            knip_run: None,
            ci_repository: None,
            typed_repository: String::new(),
            repository_refused: false,
            typed_token: String::new(),
            token_refused: false,
            token: None,
            keychain_problem: None,
            tools: Tools::default(),
        };
        section.set_settings(settings);
        section
    }

    /// The connected repository changed: what was set for the one before no longer shows.
    pub fn set_here(&mut self, here: RepoBinding) {
        if here != self.here {
            self.here = here;
            self.typed_repository = self.repository_here().unwrap_or_default();
        }
    }

    /// What the Knip run and the poll say now. A keychain problem holds until the poll says
    /// something new of the token.
    pub fn set_tools(&mut self, tools: Tools) {
        if tools.ci != self.tools.ci {
            self.keychain_problem = None;
        }
        self.tools = tools;
    }

    /// The keychain took the token change, or `problem` says why not.
    pub fn token_saved(&mut self, problem: Option<KeychainError>) {
        self.keychain_problem = problem;
    }

    pub fn take_token(&mut self) -> Option<TokenChange> {
        self.token.take()
    }

    /// Whether Knip may run in the connected repository.
    fn knip_here(&self) -> bool {
        self.knip_run.as_ref() == Some(&self.here)
    }

    /// The GitHub repository typed for the connected repository, if one is.
    fn repository_here(&self) -> Option<String> {
        self.ci_repository
            .as_ref()
            .filter(|typed| typed.binding == self.here)
            .map(|typed| typed.repo.to_string())
    }

    /// Knip on or off for the connected repository; a change only when it is one.
    pub fn set_knip(&mut self, on: bool) -> Option<Pref> {
        (self.knip_here() != on).then(|| Pref::KnipRun(on.then(|| self.here.clone())))
    }

    /// The GitHub repository typed, applied for the connected repository: an empty one goes
    /// back to the one `origin` names, and a name GitHub could not hold is refused.
    pub fn apply_repository(&mut self) -> Option<Pref> {
        let typed = self.typed_repository.trim();
        let repo = if typed.is_empty() {
            None
        } else {
            let Some(slug) = Slug::parse(typed) else {
                self.repository_refused = true;
                return None;
            };
            Some(slug)
        };
        self.repository_refused = false;
        let current = self.repository_here();
        (current != repo.as_ref().map(Slug::to_string)).then(|| {
            Pref::CiRepository(repo.map(|repo| CiRepository {
                binding: self.here.clone(),
                repo,
            }))
        })
    }

    pub fn clear_repository(&mut self) -> Option<Pref> {
        self.typed_repository.clear();
        self.apply_repository()
    }

    /// The token typed, to save; one with a space inside is refused. The field is emptied.
    pub fn save_token(&mut self) {
        match Token::new(&self.typed_token) {
            Some(token) => {
                self.token = Some(TokenChange::Save(token));
                self.token_refused = false;
                self.typed_token.clear();
            }
            None => self.token_refused = true,
        }
    }

    pub fn remove_token(&mut self) {
        self.typed_token.clear();
        self.token_refused = false;
        self.token = Some(TokenChange::Remove);
    }

    /// Shows the settings the strip runs on. A report whose path changed shows the new one;
    /// a path typed for another and not applied yet stays.
    pub fn set_settings(&mut self, settings: &AdapterSettings) {
        self.enabled = settings.enabled;
        for kind in AdapterKind::ALL {
            let path = kind.path(&settings.overrides);
            if let Some(i) = index(kind)
                && kind.path(&self.applied) != path
            {
                path.unwrap_or_default().clone_into(&mut self.typed[i]);
            }
        }
        self.applied.clone_from(&settings.overrides);
        let typed_before = self.repository_here();
        self.knip_run.clone_from(&settings.knip_run);
        self.ci_repository.clone_from(&settings.ci_repository);
        let typed_now = self.repository_here();
        if typed_now != typed_before {
            self.typed_repository = typed_now.unwrap_or_default();
        }
    }

    pub fn set_report(&mut self, report: Option<String>) {
        self.report = report;
    }

    pub fn set_on(&mut self, kind: AdapterKind, on: bool) -> Option<Pref> {
        (kind.enabled(self.enabled) != on).then_some(Pref::AdapterEnabled(kind, on))
    }

    /// The path typed for `kind`, applied: an empty one goes back to the usual places, and one
    /// outside the repository is refused, as discovery would refuse it. A metric has none.
    pub fn apply_path(&mut self, kind: AdapterKind) -> Option<Pref> {
        let typed = self.typed[index(kind)?].trim();
        let path = if typed.is_empty() {
            None
        } else {
            let Some(path) = configured_path(typed) else {
                self.refused = Some(kind);
                return None;
            };
            Some(path)
        };
        self.refused = None;
        (kind.path(&self.applied) != path.as_deref()).then_some(Pref::ReportPath(kind, path))
    }

    pub fn clear_path(&mut self, kind: AdapterKind) -> Option<Pref> {
        self.typed[index(kind)?].clear();
        self.apply_path(kind)
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, changes: &mut Vec<Pref>) {
        match &self.report {
            Some(report) => {
                ui.add(egui::Label::new(egui::RichText::new(report).monospace()).wrap());
            }
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(words::LOOKING);
                });
            }
        }
        ui.separator();
        ui.label(words::ADAPTERS_FACT);
        ui.add_space(4.0);
        for kind in AdapterKind::ALL {
            ui.add_space(4.0);
            ui.strong(words::adapter(kind));
            let mut on = kind.enabled(self.enabled);
            if ui.checkbox(&mut on, words::READ).changed() {
                changes.extend(self.set_on(kind, on));
            }
            let Some(i) = index(kind) else {
                ui.label(words::MEASURED_FROM_SOURCE);
                continue;
            };
            ui.horizontal(|ui| {
                ui.label(words::REPORT_PATH);
                let field = egui::TextEdit::singleline(&mut self.typed[i])
                    .hint_text(words::NO_PATH)
                    .desired_width(220.0);
                ui.add(field);
                if ui.button(words::APPLY).clicked() {
                    changes.extend(self.apply_path(kind));
                }
                if ui.button(words::CLEAR).clicked() {
                    changes.extend(self.clear_path(kind));
                }
            });
            if self.refused == Some(kind) {
                ui.label(words::OUTSIDE);
            }
        }
        ui.add_space(8.0);
        self.knip_ui(ui, changes);
        ui.add_space(8.0);
        self.ci_ui(ui, changes);
    }

    fn knip_ui(&mut self, ui: &mut egui::Ui, changes: &mut Vec<Pref>) {
        let mut on = self.knip_here();
        if ui.checkbox(&mut on, words::KNIP_RUN).changed() {
            changes.extend(self.set_knip(on));
        }
        ui.label(words::KNIP_RUN_FACT);
        if let Some(status) = words::knip_status(&self.tools.knip) {
            ui.label(status);
        }
    }

    fn ci_ui(&mut self, ui: &mut egui::Ui, changes: &mut Vec<Pref>) {
        ui.strong(words::CI);
        egui::Grid::new("ci").num_columns(2).show(ui, |ui| {
            ui.label(words::CI_REPOSITORY);
            ui.horizontal(|ui| {
                let field = egui::TextEdit::singleline(&mut self.typed_repository)
                    .hint_text(words::CI_REPOSITORY_HINT)
                    .desired_width(220.0);
                ui.add(field);
                if ui.button(words::APPLY).clicked() {
                    changes.extend(self.apply_repository());
                }
                if ui.button(words::CLEAR).clicked() {
                    changes.extend(self.clear_repository());
                }
            });
            ui.end_row();
            ui.label(words::CI_TOKEN);
            ui.horizontal(|ui| {
                let field = egui::TextEdit::singleline(&mut self.typed_token)
                    .password(true)
                    .desired_width(220.0);
                ui.add(field);
                if ui.button(words::SAVE).clicked() {
                    self.save_token();
                }
                if ui.button(words::REMOVE).clicked() {
                    self.remove_token();
                }
            });
            ui.end_row();
        });
        if self.repository_refused {
            ui.label(words::CI_REPOSITORY_REFUSED);
        }
        if self.token_refused {
            ui.label(words::CI_TOKEN_REFUSED);
        }
        ui.label(words::CI_FACT);
        if let Some(problem) = self.keychain_problem {
            ui.label(words::keychain_problem(problem));
        } else if let Some(status) = words::ci_status(&self.tools.ci) {
            ui.label(status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moss() -> RepoBinding {
        RepoBinding {
            seed: "5e".repeat(32),
            path: "/projects/moss".into(),
        }
    }

    fn section() -> AdaptersSection {
        AdaptersSection::new(&AdapterSettings::default(), moss())
    }

    #[test]
    fn a_switch_asks_for_a_change_only_when_it_is_one() {
        let mut section = section();
        assert_eq!(section.set_on(AdapterKind::Coverage, true), None);
        assert_eq!(
            section.set_on(AdapterKind::Coverage, false),
            Some(Pref::AdapterEnabled(AdapterKind::Coverage, false))
        );
    }

    #[test]
    fn a_metric_has_a_switch_and_no_path() {
        let mut section = section();
        assert_eq!(
            section.set_on(AdapterKind::Complexity, false),
            Some(Pref::AdapterEnabled(AdapterKind::Complexity, false))
        );
        assert_eq!(section.set_on(AdapterKind::TodoDensity, true), None);
        assert_eq!(index(AdapterKind::TodoDensity), None);
        assert_eq!(section.apply_path(AdapterKind::TodoDensity), None);
        assert_eq!(section.clear_path(AdapterKind::Complexity), None);
        assert_eq!(section.refused, None);
    }

    #[test]
    fn a_path_is_applied_as_discovery_reads_it_and_one_outside_is_refused() {
        let mut section = section();
        section.typed[0] = " ./out/lcov.info ".into();
        assert_eq!(
            section.apply_path(AdapterKind::Coverage),
            Some(Pref::ReportPath(
                AdapterKind::Coverage,
                Some("out/lcov.info".into())
            ))
        );
        for outside in ["../elsewhere/lcov.info", "/Users/moss/lcov.info"] {
            section.typed[1] = outside.into();
            assert_eq!(section.apply_path(AdapterKind::Tests), None, "{outside}");
            assert_eq!(section.refused, Some(AdapterKind::Tests));
        }
        // Applied and shown again, the same path asks for nothing; clearing goes back.
        let mut settings = AdapterSettings::default();
        settings.overrides.coverage = Some("out/lcov.info".into());
        section.set_settings(&settings);
        assert_eq!(section.typed[0], "out/lcov.info");
        assert_eq!(section.apply_path(AdapterKind::Coverage), None);
        assert_eq!(section.refused, None);
        assert_eq!(
            section.clear_path(AdapterKind::Coverage),
            Some(Pref::ReportPath(AdapterKind::Coverage, None))
        );
        // Nothing set and nothing typed: nothing to clear.
        assert_eq!(section.clear_path(AdapterKind::DeadCode), None);
    }

    #[test]
    fn a_path_typed_and_not_applied_stays_while_another_setting_changes() {
        let mut section = section();
        section.typed[0] = "out/lcov.info".into();
        let mut settings = AdapterSettings::default();
        settings.enabled.weather = false;
        settings.overrides.dead_code = Some("out/knip.json".into());
        section.set_settings(&settings);
        assert_eq!(section.typed[0], "out/lcov.info");
        assert_eq!(section.typed[2], "out/knip.json");
    }

    #[test]
    fn knip_is_switched_on_for_the_connected_repository_only() {
        let mut section = section();
        assert_eq!(section.set_knip(false), None);
        assert_eq!(section.set_knip(true), Some(Pref::KnipRun(Some(moss()))));
        let mut settings = AdapterSettings {
            knip_run: Some(moss()),
            ..AdapterSettings::default()
        };
        section.set_settings(&settings);
        assert!(section.knip_here());
        assert_eq!(section.set_knip(true), None);
        assert_eq!(section.set_knip(false), Some(Pref::KnipRun(None)));
        // Another repository connected: the switch given for moss is off here, and switching
        // it on here names this one.
        let fern = RepoBinding {
            path: "/projects/fern".into(),
            ..moss()
        };
        section.set_here(fern.clone());
        assert!(!section.knip_here());
        assert_eq!(section.set_knip(true), Some(Pref::KnipRun(Some(fern))));
        settings.knip_run = None;
        section.set_settings(&settings);
        assert!(!section.knip_here());
    }

    #[test]
    fn a_github_repository_is_typed_for_the_connected_repository() {
        let mut section = section();
        section.typed_repository = " example-garden/moss-bed ".into();
        let typed = CiRepository {
            binding: moss(),
            repo: Slug::parse("example-garden/moss-bed").unwrap(),
        };
        assert_eq!(
            section.apply_repository(),
            Some(Pref::CiRepository(Some(typed.clone())))
        );
        section.typed_repository = "not a repository".into();
        assert_eq!(section.apply_repository(), None);
        assert!(section.repository_refused);
        section.set_settings(&AdapterSettings {
            ci_repository: Some(typed),
            ..AdapterSettings::default()
        });
        assert_eq!(section.typed_repository, "example-garden/moss-bed");
        assert_eq!(
            section.apply_repository(),
            None,
            "the same name asks for nothing"
        );
        assert!(!section.repository_refused);
        assert_eq!(section.clear_repository(), Some(Pref::CiRepository(None)));
        // Another repository connected: the name typed for moss is not its own.
        section.set_here(RepoBinding {
            path: "/projects/fern".into(),
            ..moss()
        });
        assert_eq!(section.typed_repository, "");
        assert_eq!(section.clear_repository(), None);
    }

    #[test]
    fn a_token_is_handed_on_once_and_never_kept_in_the_field() {
        let mut section = section();
        section.typed_token = "github pat".into();
        section.save_token();
        assert!(section.token_refused);
        assert_eq!(section.take_token(), None);
        section.typed_token = " github_pat_fictional ".into();
        section.save_token();
        assert_eq!(section.typed_token, "");
        assert!(!section.token_refused);
        assert_eq!(
            section.take_token(),
            Some(TokenChange::Save(
                Token::new("github_pat_fictional").unwrap()
            ))
        );
        assert_eq!(section.take_token(), None);
        section.remove_token();
        assert_eq!(section.take_token(), Some(TokenChange::Remove));
    }

    #[test]
    fn a_keychain_problem_gives_way_to_what_the_poll_says_next() {
        use terrarium_engine::CiStatus;

        let mut section = section();
        section.token_saved(Some(KeychainError::Refused));
        section.set_tools(Tools::default());
        assert_eq!(section.keychain_problem, Some(KeychainError::Refused));
        section.set_tools(Tools {
            ci: CiStatus::Reading(Slug::parse("example-garden/moss-bed").unwrap()),
            ..Tools::default()
        });
        assert_eq!(section.keychain_problem, None);
    }
}
