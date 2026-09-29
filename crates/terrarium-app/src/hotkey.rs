//! The global hotkey that raises the strip. Its events are forwarded to the event loop as they
//! happen; nothing polls for them.

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use winit::event_loop::EventLoopProxy;

use crate::strip::UserEvent;

/// Sends hotkey presses to the event loop. global-hotkey keeps the first handler it is given,
/// so this runs before any hotkey is registered.
pub fn forward_events(proxy: EventLoopProxy<UserEvent>) {
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        let _ = proxy.send_event(UserEvent::HotKey(event));
    }));
}

/// The registered raise hotkey. Dropping it unregisters the hotkey.
pub struct RaiseHotkey {
    _manager: GlobalHotKeyManager,
    id: u32,
}

impl RaiseHotkey {
    /// `Ctrl+Alt+T` (Control-Option-T on a Mac). macOS 15 refuses hotkeys whose only modifiers
    /// are Option or Shift; this one has Control. Must run on the main thread.
    pub fn register() -> Result<Self, global_hotkey::Error> {
        let manager = GlobalHotKeyManager::new()?;
        let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyT);
        manager.register(hotkey)?;
        Ok(Self {
            _manager: manager,
            id: hotkey.id(),
        })
    }

    pub fn is_pressed(&self, event: GlobalHotKeyEvent) -> bool {
        event.id == self.id && event.state == HotKeyState::Pressed
    }
}
