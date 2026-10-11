//! The adapters tab (`adapters.*`): what the repository has for each adapter, with a line on
//! how each missing one connects, and for each report whether it is read and from where.
//!
//! Running Knip and the GitHub Actions weather are shown and stay off until a later version
//! brings them: no [`Pref`] names their settings, so nothing here can write them.

use terrarium_adapters::discovery::{Enabled, Overrides, configured_path};
use terrarium_engine::lifecycle::AdapterSettings;

use crate::copy::settings as words;
use crate::prefs::{AdapterKind, Pref};

pub struct AdaptersSection {
    enabled: Enabled,
    /// The paths the strip reads, and the ones typed and not applied yet.
    applied: Overrides,
    typed: [String; 3],
    /// The report, once discovery has made it.
    report: Option<String>,
    /// A path typed that was refused, and for which report.
    refused: Option<AdapterKind>,
}

fn index(kind: AdapterKind) -> usize {
    match kind {
        AdapterKind::Coverage => 0,
        AdapterKind::Tests => 1,
        AdapterKind::DeadCode => 2,
    }
}

impl AdaptersSection {
    pub fn new(settings: &AdapterSettings) -> AdaptersSection {
        let mut section = AdaptersSection {
            enabled: settings.enabled,
            applied: Overrides::default(),
            typed: Default::default(),
            report: None,
            refused: None,
        };
        section.set_settings(settings);
        section
    }

    /// Shows the settings the strip runs on. A report whose path changed shows the new one;
    /// a path typed for another and not applied yet stays.
    pub fn set_settings(&mut self, settings: &AdapterSettings) {
        self.enabled = settings.enabled;
        for kind in AdapterKind::ALL {
            let path = kind.path(&settings.overrides);
            if kind.path(&self.applied) != path {
                path.unwrap_or_default()
                    .clone_into(&mut self.typed[index(kind)]);
            }
        }
        self.applied.clone_from(&settings.overrides);
    }

    pub fn set_report(&mut self, report: Option<String>) {
        self.report = report;
    }

    pub fn set_on(&mut self, kind: AdapterKind, on: bool) -> Option<Pref> {
        (kind.enabled(self.enabled) != on).then_some(Pref::AdapterEnabled(kind, on))
    }

    /// The path typed for `kind`, applied: an empty one goes back to the usual places, and one
    /// outside the repository is refused, as discovery would refuse it.
    pub fn apply_path(&mut self, kind: AdapterKind) -> Option<Pref> {
        let typed = self.typed[index(kind)].trim();
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
        self.typed[index(kind)].clear();
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
            ui.horizontal(|ui| {
                ui.label(words::REPORT_PATH);
                let field = egui::TextEdit::singleline(&mut self.typed[index(kind)])
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
        ui.add_enabled(false, egui::Checkbox::new(&mut false, words::KNIP_RUN));
        ui.label(words::KNIP_LATER);
        ui.add_space(8.0);
        ui.strong(words::CI);
        let mut blank = String::new();
        ui.add_enabled_ui(false, |ui| {
            egui::Grid::new("ci").num_columns(2).show(ui, |ui| {
                ui.label(words::CI_REPOSITORY);
                ui.text_edit_singleline(&mut blank);
                ui.end_row();
                ui.label(words::CI_TOKEN);
                ui.add(egui::TextEdit::singleline(&mut blank).password(true));
                ui.end_row();
            });
        });
        ui.label(words::CI_LATER);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section() -> AdaptersSection {
        AdaptersSection::new(&AdapterSettings::default())
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
}
