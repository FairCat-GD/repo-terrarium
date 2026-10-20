//! The time tab (`daynight.*`, `dim.after_minutes`): when the day and the night begin, to the
//! quarter hour, and how long the machine goes without input before the strip dims.

use crate::copy::settings as words;
use crate::prefs::{DIM_MAX, DIM_MIN, DIM_STEP, Pref, Prefs, TimeOfDay};

/// The minutes a time can be set to.
const QUARTERS: [u16; 4] = [0, 15, 30, 45];

pub struct TimeSection {
    day: TimeOfDay,
    night: TimeOfDay,
    dim_after: u32,
    /// The day and the night were asked to begin together.
    same: bool,
}

/// Which begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Begins {
    Day,
    Night,
}

impl TimeSection {
    pub fn new(prefs: &Prefs) -> TimeSection {
        TimeSection {
            day: prefs.day_starts,
            night: prefs.night_starts,
            dim_after: prefs.dim_after_minutes,
            same: false,
        }
    }

    pub fn set_prefs(&mut self, prefs: &Prefs) {
        self.day = prefs.day_starts;
        self.night = prefs.night_starts;
        self.dim_after = prefs.dim_after_minutes;
    }

    /// The day or the night begins at `time`, unless the other one already does.
    pub fn set(&mut self, begins: Begins, time: TimeOfDay) -> Option<Pref> {
        let (this, other) = match begins {
            Begins::Day => (self.day, self.night),
            Begins::Night => (self.night, self.day),
        };
        self.same = time == other;
        if self.same || time == this {
            return None;
        }
        Some(match begins {
            Begins::Day => {
                self.day = time;
                Pref::DayStarts(time)
            }
            Begins::Night => {
                self.night = time;
                Pref::NightStarts(time)
            }
        })
    }

    /// Dims after `minutes`, one of the steps settings offers.
    pub fn set_dim_after(&mut self, minutes: u32) -> Option<Pref> {
        let minutes = minutes.clamp(DIM_MIN, DIM_MAX);
        (minutes != self.dim_after).then(|| {
            self.dim_after = minutes;
            Pref::DimAfter(minutes)
        })
    }

    fn time_ui(&mut self, ui: &mut egui::Ui, begins: Begins, changes: &mut Vec<Pref>) {
        let time = match begins {
            Begins::Day => self.day,
            Begins::Night => self.night,
        };
        let (mut hour, mut minute) = (time.hour(), time.minute());
        ui.horizontal(|ui| {
            ui.label(match begins {
                Begins::Day => words::DAY_STARTS,
                Begins::Night => words::NIGHT_STARTS,
            });
            egui::ComboBox::from_id_salt(("hour", begins == Begins::Day))
                .selected_text(words::two_digits(hour))
                .width(48.0)
                .show_ui(ui, |ui| {
                    for h in 0..24 {
                        ui.selectable_value(&mut hour, h, words::two_digits(h));
                    }
                });
            egui::ComboBox::from_id_salt(("minute", begins == Begins::Day))
                .selected_text(words::two_digits(minute))
                .width(48.0)
                .show_ui(ui, |ui| {
                    for m in QUARTERS {
                        ui.selectable_value(&mut minute, m, words::two_digits(m));
                    }
                });
        });
        if let Some(chosen) = TimeOfDay::new(hour, minute)
            && chosen != time
        {
            changes.extend(self.set(begins, chosen));
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, changes: &mut Vec<Pref>) {
        self.time_ui(ui, Begins::Day, changes);
        self.time_ui(ui, Begins::Night, changes);
        if self.same {
            ui.label(words::SAME_TIMES);
        }
        ui.weak(words::CROSSFADE_FACT);
        ui.add_space(12.0);
        let mut minutes = self.dim_after;
        ui.horizontal(|ui| {
            ui.strong(words::DIM_AFTER);
            egui::ComboBox::from_id_salt("dim")
                .selected_text(words::minutes(minutes))
                .show_ui(ui, |ui| {
                    for m in (DIM_MIN..=DIM_MAX).step_by(DIM_STEP as usize) {
                        ui.selectable_value(&mut minutes, m, words::minutes(m));
                    }
                });
        });
        changes.extend(self.set_dim_after(minutes));
        ui.weak(words::DIM_FACT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u16, minute: u16) -> TimeOfDay {
        TimeOfDay::new(hour, minute).unwrap()
    }

    #[test]
    fn day_and_night_never_begin_together() {
        let mut section = TimeSection::new(&Prefs::default());
        assert_eq!(section.set(Begins::Day, at(7, 0)), None, "unchanged");
        assert_eq!(section.set(Begins::Day, at(19, 0)), None);
        assert!(section.same);
        assert_eq!(section.day, at(7, 0));
        assert_eq!(
            section.set(Begins::Day, at(6, 30)),
            Some(Pref::DayStarts(at(6, 30)))
        );
        assert!(!section.same);
        assert_eq!(section.set(Begins::Night, at(6, 30)), None);
        assert_eq!(
            section.set(Begins::Night, at(0, 15)),
            Some(Pref::NightStarts(at(0, 15)))
        );
    }

    #[test]
    fn the_dim_timeout_stays_in_its_range() {
        let mut section = TimeSection::new(&Prefs::default());
        assert_eq!(section.set_dim_after(30), None);
        assert_eq!(section.set_dim_after(5), Some(Pref::DimAfter(5)));
        assert_eq!(section.set_dim_after(0), None, "already the shortest");
        assert_eq!(section.set_dim_after(600), Some(Pref::DimAfter(DIM_MAX)));
    }
}
