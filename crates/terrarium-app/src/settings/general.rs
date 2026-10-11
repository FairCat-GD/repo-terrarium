//! The general tab (`login.open_at_login`, `crashlog.opt_in`): opening at login, the opt-in
//! crash log, and the version.

use terrarium_platform::login::LoginStatus;

use crate::copy::settings as words;
use crate::prefs::{Pref, Prefs};

pub struct GeneralSection {
    open_at_login: bool,
    /// What the OS says of it, once asked.
    status: Option<LoginStatus>,
    /// Why the OS would not set it.
    login_problem: Option<String>,
    crash_log: bool,
    /// Where the crash log writes.
    logs: String,
}

impl GeneralSection {
    pub fn new(prefs: &Prefs, logs: String) -> GeneralSection {
        GeneralSection {
            open_at_login: prefs.open_at_login,
            status: None,
            login_problem: None,
            crash_log: prefs.crash_log,
            logs,
        }
    }

    pub fn set_prefs(&mut self, prefs: &Prefs) {
        self.open_at_login = prefs.open_at_login;
        self.crash_log = prefs.crash_log;
    }

    pub fn set_login_status(&mut self, status: LoginStatus) {
        self.status = Some(status);
    }

    pub fn login_problem(&mut self, problem: Option<String>) {
        self.login_problem = problem;
    }

    pub fn set_open_at_login(&mut self, on: bool) -> Option<Pref> {
        (on != self.open_at_login).then(|| {
            self.open_at_login = on;
            Pref::OpenAtLogin(on)
        })
    }

    pub fn set_crash_log(&mut self, on: bool) -> Option<Pref> {
        (on != self.crash_log).then(|| {
            self.crash_log = on;
            Pref::CrashLog(on)
        })
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, changes: &mut Vec<Pref>) {
        let mut open = self.open_at_login;
        if ui.checkbox(&mut open, words::OPEN_AT_LOGIN).changed() {
            changes.extend(self.set_open_at_login(open));
        }
        match (
            &self.login_problem,
            self.status.and_then(words::login_status),
        ) {
            (Some(problem), _) => {
                ui.label(problem);
            }
            (None, Some(line)) if self.open_at_login => {
                ui.label(line);
            }
            _ => {}
        }
        ui.add_space(8.0);
        let mut crash = self.crash_log;
        if ui.checkbox(&mut crash, words::CRASH_LOG).changed() {
            changes.extend(self.set_crash_log(crash));
        }
        ui.add(egui::Label::new(words::crash_log_fact(&self.logs)).wrap());
        ui.add_space(12.0);
        ui.weak(words::version());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_box_asks_for_a_change_only_when_it_is_one() {
        let mut section = GeneralSection::new(&Prefs::default(), "/data/logs".into());
        assert_eq!(section.set_open_at_login(false), None);
        assert_eq!(
            section.set_open_at_login(true),
            Some(Pref::OpenAtLogin(true))
        );
        assert_eq!(section.set_crash_log(false), None);
        assert_eq!(section.set_crash_log(true), Some(Pref::CrashLog(true)));
        section.set_prefs(&Prefs::default());
        assert!(!section.open_at_login && !section.crash_log);
    }
}
