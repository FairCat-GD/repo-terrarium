//! The windows the app opens beside the strip: the connect window, the settings window, and in
//! dev builds the debug window. They share one egui host, which exists only while one of them
//! is open; when the last closes, the app the person was in gets the keyboard back.
//!
//! The windows' forms are models; what their presses ask for comes back as [`Intent`]s, which
//! the strip carries out, so nothing is ever opened or changed from inside a window's drawing.

use std::path::PathBuf;
use std::time::Instant;

use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::connect::ConnectForm;
use crate::egui_host::{EguiHost, WindowKind};
use crate::prefs::Pref;
use crate::settings::SettingsForm;
use crate::{copy, pinning};

/// What a folder is picked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picking {
    /// The repository to connect.
    Repository,
    /// Where the connected repository now is.
    Location,
}

/// What a window's press asks the strip for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// The folder picker, the first of two steps.
    Pick(Picking),
    /// Connect the folder picked; the first run's "Open at login" with it.
    Connect {
        path: PathBuf,
        open_at_login: Option<bool>,
    },
    /// Relink the terrarium to the folder located.
    Relink(PathBuf),
    /// A new terrarium, confirmed.
    Reseed,
    /// Disconnect, confirmed.
    Disconnect { delete_observations: bool },
    /// The connect window's report is read.
    Done,
    /// A setting changed in the settings window: applied at once, then saved.
    Set(Pref),
}

#[derive(Default)]
pub struct Shell {
    /// Only while a window is open: dropping it drops the graphics device.
    host: Option<EguiHost>,
    /// The app the person was in when the first window opened.
    previous: Option<pinning::PreviousApp>,
    pub connect: Option<ConnectForm>,
    pub settings: Option<SettingsForm>,
}

impl Shell {
    /// Opens the `kind` window, or brings it forward, with the keyboard. False if it could not
    /// open, which is said on stderr.
    pub fn open(&mut self, event_loop: &ActiveEventLoop, kind: WindowKind) -> bool {
        if self.host.is_none() {
            self.previous = pinning::PreviousApp::frontmost();
        }
        let host = self.host.get_or_insert_with(EguiHost::new);
        match host.open(event_loop, kind) {
            Ok(()) => true,
            Err(err) => {
                eprintln!("{}", copy::host_failed(&err));
                self.drop_empty_host();
                false
            }
        }
    }

    pub fn close(&mut self, kind: WindowKind) {
        if let Some(host) = &mut self.host {
            host.close(kind);
        }
        self.drop_empty_host();
    }

    pub fn is_open(&self, kind: WindowKind) -> bool {
        self.host.as_ref().is_some_and(|host| host.is_open(kind))
    }

    pub fn owns(&self, id: WindowId) -> bool {
        self.host.as_ref().is_some_and(|host| host.owns(id))
    }

    /// The `kind` window, to parent a dialog to.
    pub fn window(&self, kind: WindowKind) -> Option<&Window> {
        self.host.as_ref().and_then(|host| host.window(kind))
    }

    /// Draws the `kind` window again: its form changed from outside.
    pub fn redraw(&self, kind: WindowKind) {
        if let Some(window) = self.window(kind) {
            window.request_redraw();
        }
    }

    /// The loop woke at `now`: windows due a repaint are asked to redraw.
    pub fn woke(&mut self, now: Instant) {
        if let Some(host) = &mut self.host {
            host.redraw_due(now);
        }
    }

    /// The earliest moment a window wants painting.
    pub fn deadline(&mut self) -> Option<Instant> {
        self.host.as_mut().and_then(EguiHost::next_repaint)
    }

    /// Handles `event` for one of the windows, drawing the debug window, if it is that one,
    /// with `debug`. Returns what the windows' presses asked for.
    pub fn window_event(
        &mut self,
        id: WindowId,
        event: &WindowEvent,
        debug: &mut dyn FnMut(&mut egui::Ui),
    ) -> Vec<Intent> {
        let Shell {
            host,
            connect,
            settings,
            ..
        } = self;
        let Some(host) = host.as_mut() else {
            return Vec::new();
        };
        host.window_event(id, event, &mut |kind, ui| match kind {
            WindowKind::Connect => {
                if let Some(form) = connect.as_mut() {
                    form.ui(ui);
                }
            }
            WindowKind::Settings => {
                if let Some(form) = settings.as_mut() {
                    form.ui(ui);
                }
            }
            #[cfg(feature = "dev")]
            WindowKind::Debug => debug(ui),
        });
        #[cfg(not(feature = "dev"))]
        let _ = debug;
        self.drop_empty_host();
        self.connect
            .as_mut()
            .and_then(ConnectForm::take_intent)
            .into_iter()
            .chain(
                self.settings
                    .as_mut()
                    .map(SettingsForm::take_intents)
                    .unwrap_or_default(),
            )
            .collect()
    }

    fn drop_empty_host(&mut self) {
        if self.host.as_ref().is_some_and(EguiHost::is_empty) {
            self.host = None;
            if let Some(previous) = self.previous.take() {
                previous.restore();
            }
        }
    }
}
