//! The strip tab (`strip.*`): which display the strip lies on, along which edge, and at the
//! largest scale that fits or one step smaller.

use terrarium_compose::scale::SizeStep;
use terrarium_platform::geometry::Edge;

use crate::copy::settings as words;
use crate::monitor::MonitorId;
use crate::prefs::{Pref, Prefs};

/// A display the strip can lie on, as the list names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorChoice {
    pub id: MonitorId,
    pub label: String,
}

pub struct StripSection {
    monitors: Vec<MonitorChoice>,
    /// The strip's scale at each size step, on the display it is on.
    scales: Option<[u32; 2]>,
    chosen: Option<MonitorId>,
    edge: Edge,
    size_step: SizeStep,
}

impl StripSection {
    pub fn new(prefs: &Prefs) -> StripSection {
        StripSection {
            monitors: Vec::new(),
            scales: None,
            chosen: prefs.monitor.clone(),
            edge: prefs.edge,
            size_step: prefs.size_step,
        }
    }

    pub fn set_prefs(&mut self, prefs: &Prefs) {
        self.chosen.clone_from(&prefs.monitor);
        self.edge = prefs.edge;
        self.size_step = prefs.size_step;
    }

    pub fn set_monitors(&mut self, monitors: Vec<MonitorChoice>, scales: Option<[u32; 2]>) {
        self.monitors = monitors;
        self.scales = scales;
    }

    /// The display chosen is not connected now: the strip is on another until it is.
    pub fn chosen_missing(&self) -> bool {
        self.chosen
            .as_ref()
            .is_some_and(|id| !self.monitors.iter().any(|m| &m.id == id))
    }

    /// Chooses the display, or the strip's own choice with `None`.
    pub fn choose_monitor(&mut self, id: Option<MonitorId>) -> Option<Pref> {
        (self.chosen != id).then(|| {
            self.chosen.clone_from(&id);
            Pref::Monitor(id)
        })
    }

    pub fn choose_edge(&mut self, edge: Edge) -> Option<Pref> {
        (self.edge != edge).then(|| {
            self.edge = edge;
            Pref::Edge(edge)
        })
    }

    pub fn choose_size(&mut self, step: SizeStep) -> Option<Pref> {
        (self.size_step != step).then(|| {
            self.size_step = step;
            Pref::SizeStep(step)
        })
    }

    fn selected_text(&self) -> String {
        match &self.chosen {
            None => words::AUTOMATIC.to_owned(),
            Some(id) => self
                .monitors
                .iter()
                .find(|m| &m.id == id)
                .map_or_else(|| words::MISSING_MONITOR.to_owned(), |m| m.label.clone()),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, changes: &mut Vec<Pref>) {
        ui.label(words::MONITOR);
        let mut chosen = self.chosen.clone();
        egui::ComboBox::from_id_salt("monitor")
            .selected_text(self.selected_text())
            .width(ui.available_width().min(360.0))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, None, words::AUTOMATIC);
                for monitor in &self.monitors {
                    ui.selectable_value(
                        &mut chosen,
                        Some(monitor.id.clone()),
                        monitor.label.as_str(),
                    );
                }
                if self.chosen_missing() {
                    ui.selectable_value(&mut chosen, self.chosen.clone(), words::MISSING_MONITOR);
                }
            });
        changes.extend(self.choose_monitor(chosen));
        if self.chosen.is_none() {
            ui.label(words::AUTOMATIC_FACT);
        }
        ui.add_space(8.0);
        ui.label(words::EDGE);
        let mut edge = self.edge;
        ui.horizontal(|ui| {
            ui.radio_value(&mut edge, Edge::Bottom, words::BOTTOM);
            ui.radio_value(&mut edge, Edge::Top, words::TOP);
        });
        changes.extend(self.choose_edge(edge));
        ui.add_space(8.0);
        ui.label(words::SIZE);
        let mut step = self.size_step;
        let scale = |i: usize| {
            self.scales
                .map(|scales| format!(" ({})", words::scale(scales[i])))
                .unwrap_or_default()
        };
        ui.horizontal(|ui| {
            ui.radio_value(
                &mut step,
                SizeStep::Full,
                format!("{}{}", words::LARGEST, scale(0)),
            );
            ui.radio_value(
                &mut step,
                SizeStep::OneSmaller,
                format!("{}{}", words::ONE_SMALLER, scale(1)),
            );
        });
        changes.extend(self.choose_size(step));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section() -> StripSection {
        let mut section = StripSection::new(&Prefs::default());
        section.set_monitors(
            vec![MonitorChoice {
                id: MonitorId("display-a".into()),
                label: "Garden Display (5120×2880, main)".into(),
            }],
            Some([8, 7]),
        );
        section
    }

    #[test]
    fn a_choice_asks_for_a_change_only_when_it_is_one() {
        let mut section = section();
        assert_eq!(section.choose_monitor(None), None);
        let a = Some(MonitorId("display-a".into()));
        assert_eq!(
            section.choose_monitor(a.clone()),
            Some(Pref::Monitor(a.clone()))
        );
        assert_eq!(section.choose_monitor(a), None);
        assert_eq!(section.choose_monitor(None), Some(Pref::Monitor(None)));
        assert_eq!(section.choose_edge(Edge::Bottom), None);
        assert_eq!(section.choose_edge(Edge::Top), Some(Pref::Edge(Edge::Top)));
        assert_eq!(
            section.choose_size(SizeStep::OneSmaller),
            Some(Pref::SizeStep(SizeStep::OneSmaller))
        );
        assert_eq!(section.choose_size(SizeStep::OneSmaller), None);
    }

    #[test]
    fn a_display_chosen_and_gone_is_named_as_not_connected() {
        let mut section = section();
        assert!(!section.chosen_missing());
        assert_eq!(section.selected_text(), words::AUTOMATIC);
        section.set_prefs(&Prefs {
            monitor: Some(MonitorId("display-gone".into())),
            ..Prefs::default()
        });
        assert!(section.chosen_missing());
        assert_eq!(section.selected_text(), words::MISSING_MONITOR);
        section.set_prefs(&Prefs {
            monitor: Some(MonitorId("display-a".into())),
            ..Prefs::default()
        });
        assert!(!section.chosen_missing());
        assert_eq!(section.selected_text(), "Garden Display (5120×2880, main)");
    }
}
