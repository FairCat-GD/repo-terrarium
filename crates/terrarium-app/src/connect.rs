//! The connect window, open while no repository is connected.
//!
//! Picking a folder and connecting it are two steps: the picker only fills in the path, and
//! connecting is a separate press, so a harness drives the connect without any dialog. On the
//! first run, "Open at login" is offered, checked; never after. Once connected, the window
//! shows the discovery report, once.
//!
//! The form is a model the window only draws; what a press asks for is an [`Intent`] the strip
//! carries out.

use std::path::PathBuf;

use crate::copy::connect as words;
use crate::shell::{Intent, Picking};

pub struct ConnectForm {
    first_run: bool,
    path: Option<PathBuf>,
    open_at_login: bool,
    page: Page,
    /// Why the last connect failed, said until the next.
    problem: Option<String>,
    /// A connect is under way.
    busy: bool,
    intent: Option<Intent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Page {
    Choose,
    /// Connected: the repository's name and the discovery report.
    Report {
        name: String,
        report: String,
    },
}

impl ConnectForm {
    pub fn new(first_run: bool) -> ConnectForm {
        ConnectForm {
            first_run,
            path: None,
            open_at_login: true,
            page: Page::Choose,
            problem: None,
            busy: false,
            intent: None,
        }
    }

    /// "Choose folder…": asks for the picker.
    pub fn choose(&mut self) {
        if !self.busy {
            self.intent = Some(Intent::Pick(Picking::Repository));
        }
    }

    /// What the picker returned; `None` when it was cancelled, which changes nothing.
    pub fn picked(&mut self, path: Option<PathBuf>) {
        if let Some(path) = path {
            self.path = Some(path);
            self.problem = None;
        }
    }

    pub fn set_open_at_login(&mut self, on: bool) {
        self.open_at_login = on;
    }

    /// "Connect": asks to connect the folder picked, with the first run's login choice.
    pub fn connect(&mut self) {
        let Some(path) = self.path.clone().filter(|_| !self.busy) else {
            return;
        };
        self.busy = true;
        self.problem = None;
        self.intent = Some(Intent::Connect {
            path,
            open_at_login: self.first_run.then_some(self.open_at_login),
        });
    }

    /// The connect failed, and why, in the window's words.
    pub fn failed(&mut self, problem: String) {
        self.busy = false;
        self.problem = Some(problem);
    }

    /// Connected `name`: the report shows, once.
    pub fn connected(&mut self, name: String, report: String) {
        self.busy = false;
        self.page = Page::Report { name, report };
    }

    /// "Done": the report is read.
    pub fn done(&mut self) {
        if matches!(self.page, Page::Report { .. }) {
            self.intent = Some(Intent::Done);
        }
    }

    pub fn take_intent(&mut self) -> Option<Intent> {
        self.intent.take()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        match self.page.clone() {
            Page::Choose => self.choose_ui(ui),
            Page::Report { name, report } => {
                ui.heading(words::connected(&name));
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(ui.available_height() - 40.0)
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(report).monospace()).wrap());
                    });
                ui.add_space(8.0);
                if ui.button(words::DONE).clicked() {
                    self.done();
                }
            }
        }
    }

    fn choose_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading(words::HEADING);
        ui.add_space(4.0);
        ui.label(words::INTRO);
        ui.add_space(12.0);
        if ui
            .add_enabled(!self.busy, egui::Button::new(words::CHOOSE))
            .clicked()
        {
            self.choose();
        }
        // A path can be longer than the window is wide: it wraps on a line of its own.
        match &self.path {
            Some(path) => ui.add(
                egui::Label::new(egui::RichText::new(path.display().to_string()).monospace())
                    .wrap(),
            ),
            None => ui.weak(words::NONE_CHOSEN),
        };
        if self.first_run {
            let mut on = self.open_at_login;
            if ui
                .add_enabled(
                    !self.busy,
                    egui::Checkbox::new(&mut on, words::OPEN_AT_LOGIN),
                )
                .changed()
            {
                self.set_open_at_login(on);
            }
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let ready = self.path.is_some() && !self.busy;
            if ui
                .add_enabled(ready, egui::Button::new(words::CONNECT))
                .clicked()
            {
                self.connect();
            }
            if self.busy {
                ui.spinner();
                ui.label(words::READING);
            }
        });
        if let Some(problem) = &self.problem {
            ui.add_space(8.0);
            ui.label(problem);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_picked_then_connected_in_two_steps() {
        let mut form = ConnectForm::new(true);
        // Nothing to connect before a folder is picked.
        form.connect();
        assert_eq!(form.take_intent(), None);
        form.choose();
        assert_eq!(form.take_intent(), Some(Intent::Pick(Picking::Repository)));
        // A cancelled pick changes nothing; a pick fills in the path only.
        form.picked(None);
        assert_eq!(form.path, None);
        form.picked(Some(PathBuf::from("/projects/moss")));
        assert_eq!(form.take_intent(), None);
        form.connect();
        assert_eq!(
            form.take_intent(),
            Some(Intent::Connect {
                path: PathBuf::from("/projects/moss"),
                open_at_login: Some(true),
            })
        );
        // While it connects, nothing else is asked.
        form.connect();
        form.choose();
        assert_eq!(form.take_intent(), None);
    }

    #[test]
    fn open_at_login_is_offered_checked_on_the_first_run_only() {
        let mut first = ConnectForm::new(true);
        first.picked(Some(PathBuf::from("/moss")));
        first.set_open_at_login(false);
        first.connect();
        assert!(matches!(
            first.take_intent(),
            Some(Intent::Connect {
                open_at_login: Some(false),
                ..
            })
        ));
        let mut later = ConnectForm::new(false);
        later.picked(Some(PathBuf::from("/moss")));
        later.connect();
        assert!(matches!(
            later.take_intent(),
            Some(Intent::Connect {
                open_at_login: None,
                ..
            })
        ));
    }

    #[test]
    fn a_failure_is_said_and_another_try_can_follow_and_success_shows_the_report() {
        let mut form = ConnectForm::new(false);
        form.picked(Some(PathBuf::from("/plain")));
        form.connect();
        form.take_intent();
        form.failed("This folder is not a git repository.".into());
        assert_eq!(
            form.problem.as_deref(),
            Some("This folder is not a git repository.")
        );
        form.picked(Some(PathBuf::from("/moss")));
        assert_eq!(form.problem, None);
        form.connect();
        assert!(form.take_intent().is_some());
        // "Done" means nothing before the report.
        form.done();
        assert_eq!(form.take_intent(), None);
        form.connected("moss".into(), "Found\n".into());
        assert_eq!(
            form.page,
            Page::Report {
                name: "moss".into(),
                report: "Found\n".into()
            }
        );
        form.done();
        assert_eq!(form.take_intent(), Some(Intent::Done));
    }
}
