//! The settings window, so far its repository section: where the repository is, locating it
//! when it is not found there, a new terrarium, and disconnecting.
//!
//! Locating is two steps, as connecting is: the picker fills in the folder, and relinking is a
//! separate press. A new terrarium and a disconnect each ask for a confirmation first, which
//! says plainly what they do; nothing changes before it is given.
//!
//! The form is a model the window only draws; what a press asks for is an [`Intent`] the strip
//! carries out.

use std::path::PathBuf;

use crate::copy::settings as words;
use crate::shell::{Intent, Picking};

/// The connected repository, as the section shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoFacts {
    pub name: String,
    pub path: String,
    pub default_branch: String,
}

pub struct SettingsForm {
    repo: RepoFacts,
    /// The repository is not where it was connected from.
    missing: bool,
    page: Page,
    /// The folder picked to relink to.
    located: Option<PathBuf>,
    delete_observations: bool,
    /// Why the last change failed, said until the next.
    problem: Option<String>,
    /// A change is under way.
    busy: bool,
    intent: Option<Intent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Repository,
    ConfirmReseed,
    ConfirmDisconnect,
}

impl SettingsForm {
    pub fn new(repo: RepoFacts) -> SettingsForm {
        SettingsForm {
            repo,
            missing: false,
            page: Page::Repository,
            located: None,
            delete_observations: false,
            problem: None,
            busy: false,
            intent: None,
        }
    }

    pub fn set_repo(&mut self, repo: RepoFacts) {
        self.repo = repo;
    }

    pub fn is_missing(&self) -> bool {
        self.missing
    }

    /// Whether the repository is not where it was connected from.
    pub fn set_missing(&mut self, missing: bool) {
        self.missing = missing;
        if !missing {
            self.located = None;
        }
    }

    /// "Locate…": asks for the picker.
    pub fn locate(&mut self) {
        if !self.busy {
            self.intent = Some(Intent::Pick(Picking::Location));
        }
    }

    /// What the picker returned; `None` when it was cancelled, which changes nothing.
    pub fn located(&mut self, path: Option<PathBuf>) {
        if let Some(path) = path {
            self.located = Some(path);
            self.problem = None;
        }
    }

    /// "Relink": asks to relink to the folder located.
    pub fn relink(&mut self) {
        let Some(path) = self.located.clone().filter(|_| !self.busy) else {
            return;
        };
        self.start(Intent::Relink(path));
    }

    /// "New terrarium…": the confirmation.
    pub fn ask_reseed(&mut self) {
        if !self.busy {
            self.page = Page::ConfirmReseed;
        }
    }

    /// "New terrarium", confirmed.
    pub fn reseed(&mut self) {
        if self.page == Page::ConfirmReseed && !self.busy {
            self.start(Intent::Reseed);
        }
    }

    /// "Disconnect…": the confirmation, with observations kept unless asked.
    pub fn ask_disconnect(&mut self) {
        if !self.busy {
            self.delete_observations = false;
            self.page = Page::ConfirmDisconnect;
        }
    }

    pub fn set_delete_observations(&mut self, delete: bool) {
        self.delete_observations = delete;
    }

    /// "Disconnect", confirmed.
    pub fn disconnect(&mut self) {
        if self.page == Page::ConfirmDisconnect && !self.busy {
            self.start(Intent::Disconnect {
                delete_observations: self.delete_observations,
            });
        }
    }

    /// "Cancel": back to the section, nothing changed.
    pub fn cancel(&mut self) {
        if !self.busy {
            self.page = Page::Repository;
        }
    }

    fn start(&mut self, intent: Intent) {
        self.busy = true;
        self.problem = None;
        self.intent = Some(intent);
    }

    /// The change asked for ended: `problem` says why it failed, in the window's words.
    pub fn finished(&mut self, problem: Option<String>) {
        self.busy = false;
        self.page = Page::Repository;
        if problem.is_none() {
            self.located = None;
        }
        self.problem = problem;
    }

    pub fn take_intent(&mut self) -> Option<Intent> {
        self.intent.take()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading(words::REPOSITORY);
        ui.add_space(8.0);
        egui::Grid::new("repository")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label(words::NAME);
                ui.label(&self.repo.name);
                ui.end_row();
                ui.label(words::DEFAULT_BRANCH);
                ui.monospace(&self.repo.default_branch);
                ui.end_row();
            });
        // A path can be longer than the window is wide: it wraps on a line of its own.
        ui.label(words::FOLDER);
        ui.add(egui::Label::new(egui::RichText::new(&self.repo.path).monospace()).wrap());
        ui.add_space(8.0);
        match self.page {
            Page::Repository => self.repository_ui(ui),
            Page::ConfirmReseed => {
                ui.label(words::RESEED_STATEMENT);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.busy, egui::Button::new(words::RESEED))
                        .clicked()
                    {
                        self.reseed();
                    }
                    if ui
                        .add_enabled(!self.busy, egui::Button::new(words::CANCEL))
                        .clicked()
                    {
                        self.cancel();
                    }
                });
            }
            Page::ConfirmDisconnect => {
                ui.label(words::disconnect_statement(&self.repo.name));
                ui.add_space(8.0);
                let mut delete = self.delete_observations;
                if ui
                    .add_enabled(
                        !self.busy,
                        egui::Checkbox::new(&mut delete, words::DELETE_OBSERVATIONS),
                    )
                    .changed()
                {
                    self.set_delete_observations(delete);
                }
                ui.label(words::DELETE_STATEMENT);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.busy, egui::Button::new(words::DISCONNECT))
                        .clicked()
                    {
                        self.disconnect();
                    }
                    if ui
                        .add_enabled(!self.busy, egui::Button::new(words::CANCEL))
                        .clicked()
                    {
                        self.cancel();
                    }
                });
            }
        }
        if self.busy {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(words::WORKING);
            });
        }
        if let Some(problem) = &self.problem {
            ui.add_space(8.0);
            ui.label(problem);
        }
    }

    fn repository_ui(&mut self, ui: &mut egui::Ui) {
        if self.missing {
            ui.label(words::NOT_FOUND);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.busy, egui::Button::new(words::LOCATE))
                    .clicked()
                {
                    self.locate();
                }
                let ready = self.located.is_some() && !self.busy;
                if ui
                    .add_enabled(ready, egui::Button::new(words::RELINK))
                    .clicked()
                {
                    self.relink();
                }
            });
            if let Some(path) = &self.located {
                let chosen = words::chosen(&path.display().to_string());
                ui.add(egui::Label::new(egui::RichText::new(chosen).monospace()).wrap());
            }
            ui.add_space(8.0);
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.busy, egui::Button::new(words::NEW_TERRARIUM))
                .clicked()
            {
                self.ask_reseed();
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new(words::DISCONNECT_BUTTON))
                .clicked()
            {
                self.ask_disconnect();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> SettingsForm {
        SettingsForm::new(RepoFacts {
            name: "moss".into(),
            path: "/projects/moss".into(),
            default_branch: "main".into(),
        })
    }

    #[test]
    fn nothing_changes_before_a_confirmation_and_cancel_changes_nothing() {
        let mut form = form();
        // The confirmed presses do nothing from the section itself.
        form.reseed();
        form.disconnect();
        assert_eq!(form.take_intent(), None);
        form.ask_reseed();
        assert_eq!(form.take_intent(), None);
        form.cancel();
        form.reseed();
        assert_eq!(form.take_intent(), None);
        form.ask_reseed();
        form.reseed();
        assert_eq!(form.take_intent(), Some(Intent::Reseed));
        form.finished(None);
        assert_eq!(form.page, Page::Repository);
    }

    #[test]
    fn a_disconnect_keeps_observations_unless_asked_each_time() {
        let mut form = form();
        form.ask_disconnect();
        form.disconnect();
        assert_eq!(
            form.take_intent(),
            Some(Intent::Disconnect {
                delete_observations: false
            })
        );
        form.finished(Some("The data directory failed.".into()));
        form.ask_disconnect();
        form.set_delete_observations(true);
        form.cancel();
        // Asked again, the box starts unchecked.
        form.ask_disconnect();
        assert!(!form.delete_observations);
        form.set_delete_observations(true);
        form.disconnect();
        assert_eq!(
            form.take_intent(),
            Some(Intent::Disconnect {
                delete_observations: true
            })
        );
        // While it runs, nothing else is asked.
        form.locate();
        form.ask_reseed();
        assert_eq!(form.take_intent(), None);
    }

    #[test]
    fn a_missing_repository_is_located_then_relinked_in_two_steps() {
        let mut form = form();
        form.set_missing(true);
        form.relink();
        assert_eq!(form.take_intent(), None, "nothing located yet");
        form.locate();
        assert_eq!(form.take_intent(), Some(Intent::Pick(Picking::Location)));
        form.located(None);
        form.relink();
        assert_eq!(form.take_intent(), None, "the pick was cancelled");
        form.located(Some(PathBuf::from("/moved/moss")));
        assert_eq!(form.take_intent(), None);
        form.relink();
        assert_eq!(
            form.take_intent(),
            Some(Intent::Relink(PathBuf::from("/moved/moss")))
        );
        form.finished(Some("That folder holds another repository.".into()));
        assert_eq!(form.located, Some(PathBuf::from("/moved/moss")));
        form.set_missing(false);
        assert_eq!(form.located, None);
    }
}
