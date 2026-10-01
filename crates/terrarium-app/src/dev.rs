//! What a dev build adds to the strip's loop: the egui host while a window is open, the debug
//! panel's state, the frame meter, the loop's wake count, and the egui memory probe.

use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

use crate::copy;
use crate::debug::{DebugPanel, FpsMeter, Readout};
use crate::egui_host::{EguiHost, WindowKind};
use crate::egui_probe::{EguiProbe, Phase, Step};
use crate::feed::{Feed, Overrides};
use crate::pinning;

#[derive(Default)]
pub struct Dev {
    /// Only while an egui window is open: dropping it drops the graphics device.
    host: Option<EguiHost>,
    /// The app the user was in when the first window opened, given the keyboard back when the
    /// last one closes.
    previous: Option<pinning::PreviousApp>,
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

    /// The loop woke at `now`: windows due a repaint are asked to redraw.
    pub fn woke(&mut self, now: Instant) {
        self.wakes += 1;
        if let Some(host) = &mut self.host {
            host.redraw_due(now);
        }
    }

    /// The earliest moment a window wants painting or the probe its next step.
    pub fn deadline(&mut self) -> Option<Instant> {
        let repaint = self.host.as_mut().and_then(EguiHost::next_repaint);
        let probe = self.probe.as_ref().and_then(EguiProbe::due);
        [repaint, probe].into_iter().flatten().min()
    }

    /// Opens the debug window, showing `feed` and `overrides` the first time.
    pub fn open_debug(&mut self, event_loop: &ActiveEventLoop, feed: &Feed, overrides: Overrides) {
        self.panel
            .get_or_insert_with(|| DebugPanel::new(feed, overrides));
        if self.host.is_none() {
            self.previous = pinning::PreviousApp::frontmost();
        }
        let host = self.host.get_or_insert_with(EguiHost::new);
        if let Err(err) = host.open(event_loop, WindowKind::Debug) {
            eprintln!("{}", copy::debug::host_failed(&err));
            self.drop_empty_host();
        }
    }

    fn close_debug(&mut self) {
        if let Some(host) = &mut self.host {
            host.close(WindowKind::Debug);
        }
        self.drop_empty_host();
    }

    fn drop_empty_host(&mut self) {
        if self.host.as_ref().is_some_and(EguiHost::is_empty) {
            self.host = None;
            if let Some(previous) = self.previous.take() {
                previous.restore();
            }
        }
    }

    pub fn owns(&self, id: WindowId) -> bool {
        self.host.as_ref().is_some_and(|host| host.owns(id))
    }

    /// Handles `event` for an egui window. The panel's changes, if any, are returned for the
    /// strip to show.
    pub fn window_event(
        &mut self,
        id: WindowId,
        event: &WindowEvent,
        readout: Option<&Readout>,
    ) -> Option<(Feed, Overrides)> {
        let Dev { host, panel, .. } = self;
        let host = host.as_mut()?;
        host.window_event(id, event, &mut |kind, ui| match kind {
            WindowKind::Debug => {
                if let (Some(panel), Some(readout)) = (panel.as_mut(), readout) {
                    panel.ui(ui, readout);
                }
            }
        });
        self.drop_empty_host();
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
        event_loop: &ActiveEventLoop,
        now: Instant,
        shown: (&Feed, Overrides),
    ) -> bool {
        let Some(step) = self.probe.as_mut().and_then(|probe| probe.take(now)) else {
            return false;
        };
        match step {
            Step::Open => self.open_debug(event_loop, shown.0, shown.1),
            Step::Close => self.close_debug(),
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
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let path = PathBuf::from(format!("egui-memory-{started}.txt"));
        match std::fs::write(&path, copy::debug::egui_probe_report(probe.samples())) {
            Ok(()) => println!("{}", copy::debug::egui_probe_written(&path)),
            Err(err) => eprintln!("{}", copy::debug::egui_probe_write_failed(&path, &err)),
        }
    }
}
