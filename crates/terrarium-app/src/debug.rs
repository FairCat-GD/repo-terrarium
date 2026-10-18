//! The debug window (dev builds only): what the strip shows, chosen by hand, and how much it
//! costs. It is the harness for judging the looks: presets, or the synthetic climate's knobs
//! with health and activity as two sliders; and the sim tuning harness: the real sim, driven by
//! climate sliders and three actions. Then a trigger for each foreground event; the time of
//! day; dim; and the frames, compositions and memory the strip is spending.
//!
//! The panel keeps its own state, so it survives its window closing and the egui host being
//! dropped. The strip takes its changes (`take_change`) after each pass and gives them to the
//! feed.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use egui::Ui;
use terrarium_compose::clock::LocalClock;
use terrarium_compose::preset;
use terrarium_compose::synthetic::{self, Knobs, MAX_RATIO};
use terrarium_platform::memory::Memory;
use terrarium_truth::PresentationState;
use terrarium_truth::scene::{EventKind, Light, Weather};

use crate::copy::debug as text;
use crate::feed::{Feed, Overrides, Trigger};
use crate::sandbox::{self, Settings};

/// How often the open window refreshes its readout: often enough to read, rarely enough that
/// an open window costs the strip almost nothing.
pub const READOUT_EVERY: Duration = Duration::from_millis(500);
const MINUTES_IN_DAY: u32 = 24 * 60;

/// What the strip shows, as the panel offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    Live,
    Thriving,
    Demo,
    Preset(usize),
    Synthetic,
    Simulation,
}

/// Dim as the panel sets it: after idle time, as the strip does, or held on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimChoice {
    Idle,
    On,
    Off,
}

/// The strip's spending, as the readout shows it.
pub struct Readout {
    pub frames: u64,
    pub fps: usize,
    pub compositions: u64,
    pub memory: Option<Memory>,
    pub state: PresentationState,
    pub clock: LocalClock,
    pub dimmed: bool,
    /// `--demo`'s stage, while the demo is shown.
    pub stage: Option<&'static str>,
    /// How many days the simulation has run, while it is shown.
    pub sim_day: Option<u32>,
}

pub struct DebugPanel {
    choice: Choice,
    knobs: Knobs,
    sim: Settings,
    fixed_time: bool,
    minute_of_day: u32,
    dim: DimChoice,
    event: Option<Trigger>,
    presses: u32,
    changed: bool,
}

impl DebugPanel {
    /// A panel showing what the strip shows now.
    pub fn new(feed: &Feed, overrides: Overrides) -> DebugPanel {
        let sim = match feed {
            Feed::Simulation(settings) => *settings,
            _ => Settings::default(),
        };
        let (choice, knobs) = match feed {
            Feed::Live => (Choice::Live, synthetic::NAMED[0].1),
            Feed::Thriving => (Choice::Thriving, synthetic::NAMED[0].1),
            Feed::Demo => (Choice::Demo, synthetic::NAMED[0].1),
            Feed::Preset(name) => (
                Choice::Preset(preset::NAMES.iter().position(|n| n == name).unwrap_or(0)),
                synthetic::NAMED[0].1,
            ),
            Feed::Synthetic(knobs) => (Choice::Synthetic, *knobs),
            Feed::Simulation(_) => (Choice::Simulation, synthetic::NAMED[0].1),
        };
        DebugPanel {
            choice,
            knobs,
            sim,
            fixed_time: overrides.time_of_day.is_some(),
            minute_of_day: overrides.time_of_day.map_or(12 * 60, |ms| ms / 60_000),
            dim: match overrides.dim {
                None => DimChoice::Idle,
                Some(true) => DimChoice::On,
                Some(false) => DimChoice::Off,
            },
            event: overrides.event,
            presses: overrides.presses,
            changed: false,
        }
    }

    /// What the strip should now show, if anything was changed since last asked.
    pub fn take_change(&mut self) -> Option<(Feed, Overrides)> {
        std::mem::take(&mut self.changed).then(|| (self.feed(), self.overrides()))
    }

    fn feed(&self) -> Feed {
        match self.choice {
            Choice::Live => Feed::Live,
            Choice::Thriving => Feed::Thriving,
            Choice::Demo => Feed::Demo,
            Choice::Preset(index) => Feed::Preset(preset::NAMES[index]),
            Choice::Synthetic => Feed::Synthetic(self.knobs),
            Choice::Simulation => Feed::Simulation(self.sim),
        }
    }

    fn overrides(&self) -> Overrides {
        Overrides {
            time_of_day: self.fixed_time.then_some(self.minute_of_day * 60_000),
            dim: match self.dim {
                DimChoice::Idle => None,
                DimChoice::On => Some(true),
                DimChoice::Off => Some(false),
            },
            event: self.event,
            presses: self.presses,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, readout: &Readout) {
        ui.ctx().request_repaint_after(READOUT_EVERY);
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.scene(ui);
            ui.separator();
            if self.choice == Choice::Synthetic {
                self.climate(ui);
                ui.separator();
            }
            if self.choice == Choice::Simulation {
                self.simulation(ui);
                ui.separator();
            }
            self.events(ui);
            ui.separator();
            self.clock(ui);
            ui.separator();
            show_readout(ui, readout);
        });
    }

    fn scene(&mut self, ui: &mut Ui) {
        ui.heading(text::SCENE);
        let before = self.choice;
        ui.radio_value(&mut self.choice, Choice::Live, text::LIVE);
        ui.radio_value(&mut self.choice, Choice::Thriving, text::THRIVING);
        ui.radio_value(&mut self.choice, Choice::Demo, text::DEMO);
        for (index, name) in preset::NAMES.iter().enumerate() {
            ui.radio_value(&mut self.choice, Choice::Preset(index), text::preset(name));
        }
        ui.radio_value(&mut self.choice, Choice::Synthetic, text::SYNTHETIC);
        ui.radio_value(&mut self.choice, Choice::Simulation, text::SIMULATION);
        self.changed |= self.choice != before;
    }

    fn simulation(&mut self, ui: &mut Ui) {
        ui.heading(text::SIMULATION);
        ui.label(text::SIMULATION_NOTE);
        let before = self.sim;
        let sim = &mut self.sim;
        ui.add(egui::Slider::new(&mut sim.light, 0..=synthetic::FULL).text(text::LIGHT));
        ui.add(egui::Slider::new(&mut sim.soil, 0..=synthetic::FULL).text(text::SOIL));
        ui.add(egui::Slider::new(&mut sim.rain, 0..=MAX_RATIO).text(text::RAIN));
        ui.add(egui::Slider::new(&mut sim.wind, 0..=MAX_RATIO).text(text::WIND));
        egui::ComboBox::from_label(text::WEATHER)
            .selected_text(text::weather(sim.weather))
            .show_ui(ui, |ui| {
                for weather in sandbox::WEATHERS {
                    ui.selectable_value(&mut sim.weather, weather, text::weather(weather));
                }
            });
        ui.add(egui::Slider::new(&mut sim.speed, 0..=30).text(text::SPEED));
        ui.horizontal_wrapped(|ui| {
            if ui.button(text::TAG_RELEASE).clicked() {
                sim.releases += 1;
            }
            if ui.button(text::DELETE_DIRECTORY).clicked() {
                sim.deletions += 1;
            }
            if ui.button(text::REWRITE_HISTORY).clicked() {
                sim.rewrites += 1;
            }
            if ui.button(text::START_OVER).clicked() {
                sim.restarts += 1;
            }
        });
        self.changed |= self.sim != before;
    }

    fn climate(&mut self, ui: &mut Ui) {
        ui.heading(text::CLIMATE);
        let before = self.knobs;
        let mut health = self.knobs.health();
        if ui
            .add(egui::Slider::new(&mut health, 0..=synthetic::FULL).text(text::HEALTH))
            .changed()
        {
            self.knobs.set_health(health);
        }
        let mut activity = self.knobs.activity();
        if ui
            .add(egui::Slider::new(&mut activity, 0..=synthetic::FULL).text(text::ACTIVITY))
            .changed()
        {
            self.knobs.set_activity(activity);
        }
        let knobs = &mut self.knobs;
        ui.add(egui::Slider::new(&mut knobs.light, 0..=synthetic::FULL).text(text::LIGHT));
        ui.add(egui::Slider::new(&mut knobs.soil, 0..=synthetic::FULL).text(text::SOIL));
        ui.add(egui::Slider::new(&mut knobs.rain, 0..=MAX_RATIO).text(text::RAIN));
        ui.add(egui::Slider::new(&mut knobs.wind, 0..=MAX_RATIO).text(text::WIND));
        egui::ComboBox::from_label(text::WEATHER)
            .selected_text(text::weather(knobs.weather))
            .show_ui(ui, |ui| {
                for &weather in Weather::ALL {
                    ui.selectable_value(&mut knobs.weather, weather, text::weather(weather));
                }
            });
        egui::ComboBox::from_label(text::LIGHT_SHIFT)
            .selected_text(text::light(knobs.shift))
            .show_ui(ui, |ui| {
                for &light in Light::ALL {
                    ui.selectable_value(&mut knobs.shift, light, text::light(light));
                }
            });
        ui.add(egui::Slider::new(&mut knobs.idle_days, 0..=800).text(text::IDLE_DAYS));
        ui.add(egui::Slider::new(&mut knobs.age_years, 0..=10).text(text::AGE));
        ui.checkbox(&mut knobs.recovering, text::RECOVERING);
        ui.label(text::derived(
            knobs.health(),
            knobs.activity(),
            knobs.state(),
        ));
        self.changed |= self.knobs != before;
    }

    fn events(&mut self, ui: &mut Ui) {
        ui.heading(text::EVENT);
        ui.label(text::EVENT_TRIGGERS);
        ui.horizontal_wrapped(|ui| {
            for &kind in EventKind::ALL {
                let on = self.event.is_some_and(|trigger| trigger.kind == kind);
                if ui.selectable_label(on, text::event(kind)).clicked() {
                    self.press(kind);
                }
            }
            if ui.button(text::CLEAR_EVENT).clicked() {
                self.changed |= self.event.take().is_some();
            }
        });
    }

    /// Puts `kind` on the strip as a new instance, which plays once, however often the same
    /// kind was pressed before.
    fn press(&mut self, kind: EventKind) {
        self.presses += 1;
        self.event = Some(Trigger {
            kind,
            press: self.presses,
        });
        self.changed = true;
    }

    fn clock(&mut self, ui: &mut Ui) {
        ui.heading(text::CLOCK);
        let before = (self.fixed_time, self.minute_of_day, self.dim);
        ui.checkbox(&mut self.fixed_time, text::FIXED_TIME);
        ui.add_enabled(
            self.fixed_time,
            egui::Slider::new(&mut self.minute_of_day, 0..=MINUTES_IN_DAY - 1)
                .custom_formatter(|minutes, _| text::time_of_day(minutes)),
        );
        ui.horizontal(|ui| {
            ui.label(text::DIM);
            ui.radio_value(&mut self.dim, DimChoice::Idle, text::DIM_IDLE);
            ui.radio_value(&mut self.dim, DimChoice::On, text::DIM_ON);
            ui.radio_value(&mut self.dim, DimChoice::Off, text::DIM_OFF);
        });
        self.changed |= (self.fixed_time, self.minute_of_day, self.dim) != before;
    }
}

fn show_readout(ui: &mut Ui, readout: &Readout) {
    ui.heading(text::READOUT);
    for line in text::readout(readout) {
        ui.label(line);
    }
}

/// Frames presented in the last second.
#[derive(Debug, Default)]
pub struct FpsMeter {
    presents: VecDeque<Instant>,
}

impl FpsMeter {
    pub fn presented(&mut self, at: Instant) {
        self.presents.push_back(at);
        self.forget_before(at);
    }

    pub fn per_second(&mut self, now: Instant) -> usize {
        self.forget_before(now);
        self.presents.len()
    }

    fn forget_before(&mut self, now: Instant) {
        while self
            .presents
            .front()
            .is_some_and(|&at| now.saturating_duration_since(at) >= Duration::from_secs(1))
        {
            self.presents.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use terrarium_compose::sky::LocalDate;
    use terrarium_compose::tween::Millis;

    use super::*;
    use crate::feed::{Feeder, Os};

    fn readout() -> Readout {
        Readout {
            frames: 12,
            fps: 3,
            compositions: 40,
            memory: Some(Memory {
                resident: 53 << 20,
                resident_peak: None,
                footprint: Some(41 << 20),
                footprint_peak: Some(44 << 20),
                private: None,
                private_peak: None,
            }),
            state: PresentationState::Thriving,
            clock: LocalClock::noon(LocalDate::new(2026, 10, 1)),
            dimmed: false,
            stage: Some("mature"),
            sim_day: Some(130),
        }
    }

    #[test]
    fn an_open_panel_asks_to_repaint_only_for_its_readout() {
        let ctx = egui::Context::default();
        let mut panel = DebugPanel::new(&Feed::Demo, Overrides::default());
        let mut delay = Duration::ZERO;
        for _ in 0..5 {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| panel.ui(ui, &readout()));
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            output.drop_without_applying_deltas();
        }
        assert!(delay >= READOUT_EVERY / 2, "{delay:?}");
        assert_eq!(panel.take_change(), None);
    }

    #[test]
    fn the_panel_starts_from_what_the_strip_shows_and_gives_back_its_changes() {
        let overrides = Overrides {
            time_of_day: Some(22 * 3_600_000),
            dim: Some(true),
            event: Some(Trigger {
                kind: EventKind::Storm,
                press: 3,
            }),
            presses: 3,
        };
        let mut panel = DebugPanel::new(&Feed::Preset("dormant"), overrides);
        assert_eq!(panel.feed(), Feed::Preset("dormant"));
        assert_eq!(panel.overrides(), overrides);
        panel.choice = Choice::Synthetic;
        panel.knobs.set_health(30);
        panel.changed = true;
        let (feed, _) = panel.take_change().unwrap();
        assert_eq!(
            feed,
            Feed::Synthetic(Knobs {
                light: 30,
                soil: 30,
                ..synthetic::NAMED[0].1
            })
        );
        assert_eq!(panel.take_change(), None);
    }

    #[test]
    fn each_press_is_a_new_instance() {
        let mut panel = DebugPanel::new(&Feed::Thriving, Overrides::default());
        panel.press(EventKind::Storm);
        let (_, first) = panel.take_change().unwrap();
        panel.press(EventKind::Storm);
        let (_, again) = panel.take_change().unwrap();
        assert_eq!(first.event.map(|t| t.kind), Some(EventKind::Storm));
        assert_ne!(
            first.event, again.event,
            "the same kind pressed again plays again"
        );
        // Taken off and the window opened anew, the next press is still new.
        let mut reopened = DebugPanel::new(
            &Feed::Thriving,
            Overrides {
                event: None,
                ..again
            },
        );
        reopened.press(EventKind::Storm);
        let (_, later) = reopened.take_change().unwrap();
        assert!(later.event.unwrap().press > again.event.unwrap().press);
        // The strip's scene carries each press as an instance of its own.
        let instance = |overrides: Overrides| {
            let mut feeder = Feeder::new(
                Feed::Thriving,
                Millis(0),
                &Os {
                    clock: None,
                    idle: None,
                },
            );
            feeder.choose(
                Feed::Thriving,
                overrides,
                Millis(1),
                &Os {
                    clock: None,
                    idle: None,
                },
            );
            feeder
                .scene()
                .event
                .map(|event| (event.kind, event.started_tick))
        };
        assert_ne!(instance(first), instance(again));
        // Far from truth's own ticks, which count hours from a repository's first commit.
        let tick = instance(first).unwrap().1.hours();
        assert!(tick > u32::MAX - 1000, "{tick}");
        assert_eq!(
            instance(first).map(|(kind, _)| kind),
            Some(EventKind::Storm)
        );
    }

    #[test]
    fn the_meter_counts_the_last_second() {
        let base = Instant::now();
        let mut meter = FpsMeter::default();
        for ms in [0, 100, 200, 900] {
            meter.presented(base + Duration::from_millis(ms));
        }
        assert_eq!(meter.per_second(base + Duration::from_millis(950)), 4);
        assert_eq!(meter.per_second(base + Duration::from_millis(1150)), 2);
        assert_eq!(meter.per_second(base + Duration::from_secs(3)), 0);
    }
}
