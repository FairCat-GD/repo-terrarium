//! What a dev build adds to the strip's loop: the debug panel's state, the frame meter, the
//! loop's wake count, and the egui memory probe. The debug window opens on the shell's host,
//! beside the app's own windows.

use std::path::PathBuf;
use std::time::Instant;

use winit::event_loop::ActiveEventLoop;

use crate::copy;
use crate::debug::{DebugPanel, FpsMeter, Readout};
use crate::egui_host::WindowKind;
use crate::egui_probe::{EguiProbe, Phase, Step};
use crate::feed::{Feed, Overrides};
use crate::pinning;
use crate::shell::Shell;

#[derive(Default)]
pub struct Dev {
    /// Kept when its window closes, so it opens again as it was left.
    panel: Option<DebugPanel>,
    pub fps: FpsMeter,
    /// How often the loop has woken since launch.
    wakes: u64,
    probe: Option<EguiProbe>,
}

impl Dev {
    pub fn new(probing: bool) -> Dev {
        Dev {
            probe: probing.then(|| EguiProbe::start(Instant::now())),
            ..Dev::default()
        }
    }

    /// The loop woke.
    pub fn woke(&mut self) {
        self.wakes += 1;
    }

    /// The moment the probe wants its next step.
    pub fn deadline(&self) -> Option<Instant> {
        self.probe.as_ref().and_then(EguiProbe::due)
    }

    /// Opens the debug window, showing `feed` and `overrides` the first time.
    pub fn open_debug(
        &mut self,
        shell: &mut Shell,
        event_loop: &ActiveEventLoop,
        feed: &Feed,
        overrides: Overrides,
    ) {
        self.panel
            .get_or_insert_with(|| DebugPanel::new(feed, overrides));
        shell.open(event_loop, WindowKind::Debug);
    }

    /// Draws the debug window's panel with `readout`.
    pub fn draw(&mut self, ui: &mut egui::Ui, readout: Option<&Readout>) {
        if let (Some(panel), Some(readout)) = (self.panel.as_mut(), readout) {
            panel.ui(ui, readout);
        }
    }

    /// The panel's changes, if any, for the strip to show.
    pub fn take_change(&mut self) -> Option<(Feed, Overrides)> {
        self.panel.as_mut().and_then(DebugPanel::take_change)
    }

    /// Whether the probe has a step due by `now`.
    pub fn probe_due(&self, now: Instant) -> bool {
        self.probe
            .as_ref()
            .and_then(EguiProbe::due)
            .is_some_and(|due| due <= now)
    }

    /// The probe's step due by `now`, carried out. True when the probe has finished and the
    /// strip should exit.
    pub fn probe(
        &mut self,
        shell: &mut Shell,
        event_loop: &ActiveEventLoop,
        now: Instant,
        shown: (&Feed, Overrides),
    ) -> bool {
        let Some(step) = self.probe.as_mut().and_then(|probe| probe.take(now)) else {
            return false;
        };
        match step {
            Step::Open => self.open_debug(shell, event_loop, shown.0, shown.1),
            Step::Close => shell.close(WindowKind::Debug),
            Step::Sample(phase) => self.sample(phase, now),
            Step::Finish => {
                self.write_report();
                return true;
            }
        }
        false
    }

    fn sample(&mut self, phase: Phase, now: Instant) {
        let wakes = self.wakes;
        if let Some(probe) = &mut self.probe {
            probe.sample(
                phase,
                now,
                wakes,
                terrarium_platform::memory::usage(),
                pinning::app_is_active(),
                pinning::frontmost_app(),
            );
        }
    }

    fn write_report(&self) {
        let Some(probe) = &self.probe else {
            return;
        };
        let path = PathBuf::from(copy::debug::egui_memory_file(crate::now_secs()));
        match std::fs::write(&path, copy::debug::egui_probe_report(probe.samples())) {
            Ok(()) => println!("{}", copy::debug::egui_probe_written(&path)),
            Err(err) => eprintln!("{}", copy::debug::egui_probe_write_failed(&path, &err)),
        }
    }
}
