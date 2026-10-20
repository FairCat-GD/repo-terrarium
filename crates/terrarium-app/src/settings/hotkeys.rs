//! The hotkeys tab (`hotkeys.*`): the keys that raise the strip and open the time-lapse. Keys
//! are chosen with the modifiers' boxes and a list, never by pressing them: a registered hotkey
//! never reaches this window, so it could not be pressed again here.
//!
//! A binding the rules refuse is said at once; one the OS refuses comes back from the strip,
//! and the keys before stay.

use global_hotkey::hotkey::Code;

use crate::copy::{self, settings as words};
use crate::hotkey::{Binding, KEYS, Platform, Which};
use crate::prefs::{Pref, Prefs};

pub struct HotkeysSection {
    /// The bindings the strip runs on, and the ones being chosen.
    current: [Binding; 2],
    chosen: [Binding; 2],
    /// Why a binding was not taken.
    refused: [Option<String>; 2],
    /// Why the keys the strip runs on are not registered with the OS, while they are not.
    unregistered: [Option<String>; 2],
    platform: Platform,
}

impl HotkeysSection {
    pub fn new(prefs: &Prefs) -> HotkeysSection {
        let current = [prefs.raise, prefs.timelapse];
        HotkeysSection {
            current,
            chosen: current,
            refused: [None, None],
            unregistered: [None, None],
            platform: Platform::THIS,
        }
    }

    /// Shows the bindings the strip runs on. A hotkey whose binding changed shows the new one;
    /// keys chosen for the other and not applied yet stay.
    pub fn set_prefs(&mut self, prefs: &Prefs) {
        let current = [prefs.raise, prefs.timelapse];
        for (chosen, (now, before)) in self.chosen.iter_mut().zip(current.iter().zip(self.current))
        {
            if *now != before {
                *chosen = *now;
            }
        }
        self.current = current;
    }

    pub fn choose(&mut self, which: Which, binding: Binding) {
        self.chosen[which.index()] = binding;
    }

    /// The binding chosen for `which`, if the rules let it be offered to the OS.
    pub fn apply(&mut self, which: Which) -> Option<Pref> {
        let binding = self.chosen[which.index()];
        let other = self.current[which.other().index()];
        if let Err(refusal) = binding.check(other) {
            self.refused[which.index()] = Some(words::refusal(refusal, self.platform).to_owned());
            return None;
        }
        self.refused[which.index()] = None;
        (binding != self.current[which.index()]).then_some(Pref::Hotkey(which, binding))
    }

    /// Back to `which`'s own keys.
    pub fn reset(&mut self, which: Which) -> Option<Pref> {
        self.choose(which, which.default_binding());
        self.apply(which)
    }

    /// The OS refused the keys chosen for `which`, for `why`.
    pub fn refused(&mut self, which: Which, why: String) {
        self.chosen[which.index()] = self.current[which.index()];
        self.refused[which.index()] = Some(why);
    }

    /// Whether `which`'s keys are registered with the OS: `None` when they are, else why not.
    pub fn set_unregistered(&mut self, which: Which, why: Option<String>) {
        self.unregistered[which.index()] = why;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, changes: &mut Vec<Pref>) {
        let names = words::modifier_names(self.platform);
        for which in Which::ALL {
            let i = which.index();
            ui.strong(match which {
                Which::Raise => words::RAISE,
                Which::TimeLapse => words::TIME_LAPSE,
            });
            ui.label(words::now_set(&copy::hotkey(
                self.current[i],
                self.platform,
            )));
            if let Some(why) = &self.unregistered[i] {
                ui.label(why);
            }
            let mut binding = self.chosen[i];
            ui.horizontal_wrapped(|ui| {
                let mods = &mut binding.mods;
                for (flag, name) in [
                    &mut mods.control,
                    &mut mods.option,
                    &mut mods.shift,
                    &mut mods.command,
                ]
                .into_iter()
                .zip(names)
                {
                    ui.checkbox(flag, name);
                }
                ui.label(words::KEY);
                egui::ComboBox::from_id_salt(("key", i))
                    .selected_text(copy::key_word(binding.key))
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        for (code, _) in KEYS {
                            ui.selectable_value::<Code>(
                                &mut binding.key,
                                code,
                                copy::key_word(code),
                            );
                        }
                    });
            });
            self.choose(which, binding);
            ui.horizontal(|ui| {
                if ui.button(words::APPLY).clicked() {
                    changes.extend(self.apply(which));
                }
                if ui.button(words::DEFAULT).clicked() {
                    changes.extend(self.reset(which));
                }
            });
            if let Some(why) = &self.refused[i] {
                ui.label(why);
            }
            ui.add_space(8.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::Refusal;

    fn binding(text: &str) -> Binding {
        Binding::parse(text).unwrap()
    }

    #[test]
    fn an_unregistered_hotkey_says_so_until_its_keys_are_held() {
        let mut section = HotkeysSection::new(&Prefs::default());
        section.set_unregistered(Which::TimeLapse, Some("taken".into()));
        // A rules refusal of the other and the unregistered line never clear each other.
        section.choose(Which::Raise, binding("shift+T"));
        assert_eq!(section.apply(Which::Raise), None);
        assert_eq!(section.unregistered[1].as_deref(), Some("taken"));
        assert!(section.refused[0].is_some());
        section.set_unregistered(Which::TimeLapse, None);
        assert_eq!(section.unregistered, [None, None]);
        assert!(section.refused[0].is_some());
    }

    #[test]
    fn a_binding_is_applied_only_within_the_rules_and_when_it_changes() {
        let mut section = HotkeysSection::new(&Prefs::default());
        assert_eq!(section.apply(Which::Raise), None, "unchanged");
        section.choose(Which::Raise, binding("alt+shift+R"));
        assert_eq!(section.apply(Which::Raise), None);
        assert_eq!(
            section.refused[0].as_deref(),
            Some(words::refusal(Refusal::OptionShift, Platform::THIS))
        );
        // The time-lapse's keys are taken.
        section.choose(Which::Raise, binding("ctrl+alt+H"));
        assert_eq!(section.apply(Which::Raise), None);
        assert_eq!(
            section.refused[0].as_deref(),
            Some(words::refusal(Refusal::Taken, Platform::THIS))
        );
        section.choose(Which::Raise, binding("ctrl+shift+R"));
        assert_eq!(
            section.apply(Which::Raise),
            Some(Pref::Hotkey(Which::Raise, binding("ctrl+shift+R")))
        );
        assert_eq!(section.refused[0], None);
    }

    #[test]
    fn an_os_refusal_keeps_the_keys_before_and_a_reset_brings_back_the_default() {
        let prefs = Prefs {
            raise: binding("ctrl+shift+R"),
            ..Prefs::default()
        };
        let mut section = HotkeysSection::new(&prefs);
        section.choose(Which::Raise, binding("ctrl+super+Space"));
        section.refused(Which::Raise, "These keys could not be registered.".into());
        assert_eq!(section.chosen[0], binding("ctrl+shift+R"));
        assert!(section.refused[0].is_some());
        assert_eq!(
            section.reset(Which::Raise),
            Some(Pref::Hotkey(Which::Raise, Which::Raise.default_binding()))
        );
        assert_eq!(section.reset(Which::TimeLapse), None, "already its own");
    }

    #[test]
    fn keys_chosen_and_not_applied_stay_while_the_other_hotkey_changes() {
        let mut section = HotkeysSection::new(&Prefs::default());
        section.choose(Which::Raise, binding("ctrl+shift+R"));
        let prefs = Prefs {
            timelapse: binding("ctrl+super+L"),
            ..Prefs::default()
        };
        section.set_prefs(&prefs);
        assert_eq!(
            section.chosen,
            [binding("ctrl+shift+R"), binding("ctrl+super+L")]
        );
    }
}
