//! The repository tab: where the repository is, locating it when it is not found there, a new
//! terrarium, disconnecting, and the terrarium's data: its observations exported and imported,
//! and its climate exported.
//!
//! Locating is two steps, as connecting is: the picker fills in the folder, and relinking is a
//! separate press. Importing is the same: the picker fills in the file, and importing is a
//! separate press. A new terrarium, a disconnect and an import each ask for a confirmation
//! first, which says plainly what they do; nothing changes before it is given. An export
//! writes once its save dialog names the file.

use std::path::PathBuf;

use crate::copy::settings as words;
use crate::portability::Msg;
use crate::shell::{DataFile, Intent, Picking};

/// The connected repository, as the section shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoFacts {
    pub name: String,
    pub path: String,
    pub default_branch: String,
}

pub struct RepoSection {
    repo: RepoFacts,
    /// The repository is not where it was connected from.
    missing: bool,
    page: Page,
    /// The folder picked to relink to.
    located: Option<PathBuf>,
    delete_observations: bool,
    /// The observations export picked to import.
    import_file: Option<PathBuf>,
    /// Why the last change failed, said until the next.
    problem: Option<String>,
    /// What the last export or import did, said until the next.
    report: Vec<String>,
    /// What is under way; one thing at a time.
    doing: Doing,
    intent: Option<Intent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Repository,
    ConfirmReseed,
    ConfirmDisconnect,
    ConfirmImport,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Doing {
    Nothing,
    /// A lifecycle change: relink, new terrarium, disconnect or import.
    Changing,
    /// An export, and how far it has come, worded.
    Exporting(String),
}

impl RepoSection {
    pub fn new(repo: RepoFacts) -> RepoSection {
        RepoSection {
            repo,
            missing: false,
            page: Page::Repository,
            located: None,
            delete_observations: false,
            import_file: None,
            problem: None,
            report: Vec::new(),
            doing: Doing::Nothing,
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

    fn busy(&self) -> bool {
        self.doing != Doing::Nothing
    }

    /// "Locate…": asks for the picker.
    pub fn locate(&mut self) {
        if !self.busy() {
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
        let Some(path) = self.located.clone().filter(|_| !self.busy()) else {
            return;
        };
        self.start(Intent::Relink(path));
    }

    /// "New terrarium…": the confirmation.
    pub fn ask_reseed(&mut self) {
        if !self.busy() {
            self.page = Page::ConfirmReseed;
        }
    }

    /// "New terrarium", confirmed.
    pub fn reseed(&mut self) {
        if self.page == Page::ConfirmReseed && !self.busy() {
            self.start(Intent::Reseed);
        }
    }

    /// "Disconnect…": the confirmation, with observations kept unless asked.
    pub fn ask_disconnect(&mut self) {
        if !self.busy() {
            self.delete_observations = false;
            self.page = Page::ConfirmDisconnect;
        }
    }

    pub fn set_delete_observations(&mut self, delete: bool) {
        self.delete_observations = delete;
    }

    /// "Disconnect", confirmed.
    pub fn disconnect(&mut self) {
        if self.page == Page::ConfirmDisconnect && !self.busy() {
            self.start(Intent::Disconnect {
                delete_observations: self.delete_observations,
            });
        }
    }

    /// "Export observations…" or "Export climate…": asks for the save dialog.
    pub fn export(&mut self, file: DataFile) {
        if !self.busy() && self.page == Page::Repository {
            self.intent = Some(Intent::Pick(Picking::SaveData(file)));
        }
    }

    /// The save dialog answered: a path chosen exports `file` there; `None` changes nothing.
    pub fn export_to(&mut self, file: DataFile, path: Option<PathBuf>) {
        if let Some(path) = path.filter(|_| !self.busy()) {
            self.doing = Doing::Exporting(words::exporting(None));
            self.problem = None;
            self.report.clear();
            self.intent = Some(Intent::Export { file, path });
        }
    }

    /// How far the export has come.
    pub fn exporting(&mut self, msg: &Msg) {
        let words = match msg {
            Msg::Reading(percent) => words::exporting(Some(*percent)),
            Msg::Writing => words::exporting(None),
            Msg::Written(_) => return,
        };
        if let Doing::Exporting(said) = &mut self.doing {
            *said = words;
        }
    }

    /// The export is written, or why not.
    pub fn exported(&mut self, written: &Result<PathBuf, String>) {
        if matches!(self.doing, Doing::Exporting(_)) {
            self.doing = Doing::Nothing;
            self.report = vec![words::exported(written)];
        }
    }

    /// "Import observations…": asks for the file picker.
    pub fn choose_import(&mut self) {
        if !self.busy() && self.page == Page::Repository {
            self.intent = Some(Intent::Pick(Picking::Import));
        }
    }

    /// The file picker answered: a file chosen asks for the import's confirmation; `None`
    /// changes nothing.
    pub fn import_chosen(&mut self, path: Option<PathBuf>) {
        if let Some(path) = path.filter(|_| !self.busy()) {
            self.import_file = Some(path);
            self.page = Page::ConfirmImport;
        }
    }

    /// "Import", confirmed.
    pub fn import(&mut self) {
        if self.page != Page::ConfirmImport || self.busy() {
            return;
        }
        if let Some(path) = self.import_file.clone() {
            self.report.clear();
            self.start(Intent::Import(path));
        }
    }

    /// The import is done: what it did, in the window's words.
    pub fn imported(&mut self, report: Vec<String>) {
        self.finished(None);
        self.report = report;
    }

    /// "Cancel": back to the section, nothing changed.
    pub fn cancel(&mut self) {
        if !self.busy() {
            self.page = Page::Repository;
            self.import_file = None;
        }
    }

    fn start(&mut self, intent: Intent) {
        self.doing = Doing::Changing;
        self.problem = None;
        self.intent = Some(intent);
    }

    /// The change asked for ended: `problem` says why it failed, in the window's words.
    pub fn finished(&mut self, problem: Option<String>) {
        self.doing = Doing::Nothing;
        self.page = Page::Repository;
        self.import_file = None;
        if problem.is_none() {
            self.located = None;
        }
        self.problem = problem;
    }

    pub fn take_intent(&mut self) -> Option<Intent> {
        self.intent.take()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
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
            Page::ConfirmReseed => self.confirm_ui(ui, words::RESEED_STATEMENT, words::RESEED),
            Page::ConfirmDisconnect => self.disconnect_ui(ui),
            Page::ConfirmImport => {
                if let Some(path) = &self.import_file {
                    let chosen = words::chosen(&path.display().to_string());
                    ui.add(egui::Label::new(egui::RichText::new(chosen).monospace()).wrap());
                }
                self.confirm_ui(ui, words::IMPORT_STATEMENT, words::IMPORT);
            }
        }
        self.status_ui(ui);
    }

    /// A confirmation: what `confirm` does, said plainly, then it and Cancel.
    fn confirm_ui(&mut self, ui: &mut egui::Ui, statement: &str, confirm: &str) {
        ui.add(egui::Label::new(statement).wrap());
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.busy(), egui::Button::new(confirm))
                .clicked()
            {
                match self.page {
                    Page::ConfirmReseed => self.reseed(),
                    Page::ConfirmImport => self.import(),
                    Page::Repository | Page::ConfirmDisconnect => {}
                }
            }
            if ui
                .add_enabled(!self.busy(), egui::Button::new(words::CANCEL))
                .clicked()
            {
                self.cancel();
            }
        });
    }

    fn disconnect_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(words::disconnect_statement(&self.repo.name));
        ui.add_space(8.0);
        let mut delete = self.delete_observations;
        if ui
            .add_enabled(
                !self.busy(),
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
                .add_enabled(!self.busy(), egui::Button::new(words::DISCONNECT))
                .clicked()
            {
                self.disconnect();
            }
            if ui
                .add_enabled(!self.busy(), egui::Button::new(words::CANCEL))
                .clicked()
            {
                self.cancel();
            }
        });
    }

    /// What is under way, why the last change failed, and what the last export or import did.
    fn status_ui(&self, ui: &mut egui::Ui) {
        let doing = match &self.doing {
            Doing::Nothing => None,
            Doing::Changing => Some(words::WORKING),
            Doing::Exporting(said) => Some(said.as_str()),
        };
        if let Some(doing) = doing {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(doing);
            });
        }
        if let Some(problem) = &self.problem {
            ui.add_space(8.0);
            ui.add(egui::Label::new(problem.as_str()).wrap());
        }
        if !self.report.is_empty() {
            ui.add_space(8.0);
            for line in &self.report {
                ui.add(egui::Label::new(line.as_str()).wrap());
            }
        }
    }

    fn repository_ui(&mut self, ui: &mut egui::Ui) {
        let free = !self.busy();
        if self.missing {
            ui.label(words::NOT_FOUND);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(free, egui::Button::new(words::LOCATE))
                    .clicked()
                {
                    self.locate();
                }
                let ready = self.located.is_some() && free;
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
                .add_enabled(free, egui::Button::new(words::NEW_TERRARIUM))
                .clicked()
            {
                self.ask_reseed();
            }
            if ui
                .add_enabled(free, egui::Button::new(words::DISCONNECT_BUTTON))
                .clicked()
            {
                self.ask_disconnect();
            }
        });
        ui.separator();
        ui.add(egui::Label::new(words::DATA_FACT).wrap());
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(free, egui::Button::new(words::EXPORT_OBSERVATIONS))
                .clicked()
            {
                self.export(DataFile::Observations);
            }
            if ui
                .add_enabled(free, egui::Button::new(words::IMPORT_OBSERVATIONS))
                .clicked()
            {
                self.choose_import();
            }
            if ui
                .add_enabled(free, egui::Button::new(words::EXPORT_CLIMATE))
                .clicked()
            {
                self.export(DataFile::Climate);
            }
        });
    }

    /// The name the save dialog offers for `file`.
    pub fn file_name(&self, file: DataFile) -> String {
        match file {
            DataFile::Observations => words::observations_name(&self.repo.name),
            DataFile::Climate => words::climate_name(&self.repo.name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> RepoSection {
        RepoSection::new(RepoFacts {
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
    fn an_import_is_chosen_then_confirmed_and_says_what_it_did() {
        let mut form = form();
        form.import();
        assert_eq!(form.take_intent(), None, "nothing chosen");
        form.choose_import();
        assert_eq!(form.take_intent(), Some(Intent::Pick(Picking::Import)));
        form.import_chosen(None);
        assert_eq!(form.page, Page::Repository, "the picker was closed");
        form.import_chosen(Some(PathBuf::from("/in/moss.jsonl")));
        assert_eq!(form.page, Page::ConfirmImport);
        assert_eq!(form.take_intent(), None, "nothing before the confirmation");
        // Cancelled, the file is forgotten.
        form.cancel();
        form.import();
        assert_eq!(form.take_intent(), None);
        form.import_chosen(Some(PathBuf::from("/in/moss.jsonl")));
        form.import();
        assert_eq!(
            form.take_intent(),
            Some(Intent::Import(PathBuf::from("/in/moss.jsonl")))
        );
        // While it runs, nothing else is asked.
        form.export(DataFile::Observations);
        form.choose_import();
        form.ask_reseed();
        assert_eq!(form.take_intent(), None);
        form.imported(vec!["Imported 2 observations; 0 were already here.".into()]);
        assert_eq!((form.page, form.busy()), (Page::Repository, false));
        assert_eq!(form.report.len(), 1);
        // A refusal is said, and the file is not kept.
        form.import_chosen(Some(PathBuf::from("/in/fern.jsonl")));
        form.import();
        form.take_intent();
        form.finished(Some(
            "That file holds another repository's observations.".into(),
        ));
        assert_eq!(form.import_file, None);
        assert!(form.problem.is_some() && form.report.is_empty());
    }

    #[test]
    fn an_export_asks_where_then_writes_there_and_says_how_it_went() {
        let mut form = form();
        assert_eq!(
            form.file_name(DataFile::Observations),
            "moss-observations.jsonl"
        );
        assert_eq!(form.file_name(DataFile::Climate), "moss-climate.json");
        form.export(DataFile::Climate);
        assert_eq!(
            form.take_intent(),
            Some(Intent::Pick(Picking::SaveData(DataFile::Climate)))
        );
        form.export_to(DataFile::Climate, None);
        assert_eq!(form.take_intent(), None, "a dialog closed writes nothing");
        form.export_to(DataFile::Climate, Some(PathBuf::from("/out/c.json")));
        assert_eq!(
            form.take_intent(),
            Some(Intent::Export {
                file: DataFile::Climate,
                path: PathBuf::from("/out/c.json")
            })
        );
        form.exporting(&Msg::Reading(40));
        assert_eq!(
            form.doing,
            Doing::Exporting("Reading the history: 40%".into())
        );
        // While it runs, nothing else is asked.
        form.export(DataFile::Observations);
        form.ask_disconnect();
        form.export_to(DataFile::Observations, Some(PathBuf::from("/out/o.jsonl")));
        assert_eq!(form.take_intent(), None);
        assert_eq!(form.page, Page::Repository);
        form.exported(&Ok(PathBuf::from("/out/c.json")));
        assert!(!form.busy());
        assert_eq!(form.report, ["Saved c.json"]);
        form.export(DataFile::Observations);
        form.take_intent();
        form.export_to(DataFile::Observations, Some(PathBuf::from("/out/o.jsonl")));
        form.exported(&Err("disk full".into()));
        assert_eq!(form.report, ["Not saved: disk full"]);
        // A lifecycle change's end does not end an export it never started.
        form.export_to(DataFile::Observations, Some(PathBuf::from("/out/o.jsonl")));
        form.exported(&Ok(PathBuf::from("/out/o.jsonl")));
        form.exported(&Ok(PathBuf::from("/out/again.jsonl")));
        assert_eq!(
            form.report,
            ["Saved o.jsonl"],
            "an answer with nothing under way"
        );
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
