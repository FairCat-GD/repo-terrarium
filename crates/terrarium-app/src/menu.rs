//! The strip's right-click menu. Its only item today is Quit.

use muda::{Menu, MenuEvent, MenuId, MenuItem};
use winit::event_loop::EventLoopProxy;

use crate::copy;
use crate::strip::UserEvent;

/// Sends menu choices to the event loop. muda keeps the first handler it is given, so this runs
/// before any menu exists.
pub fn forward_events(proxy: EventLoopProxy<UserEvent>) {
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let _ = proxy.send_event(UserEvent::Menu(event.id));
    }));
}

pub struct StripMenu {
    menu: Menu,
    quit: MenuItem,
}

impl StripMenu {
    /// Must run on the main thread.
    pub fn new() -> Result<Self, muda::Error> {
        let menu = Menu::new();
        let quit = MenuItem::new(copy::menu_quit(), true, None);
        menu.append(&quit)?;
        Ok(Self { menu, quit })
    }

    pub fn menu(&self) -> &Menu {
        &self.menu
    }

    pub fn is_quit(&self, id: &MenuId) -> bool {
        self.quit.id() == id
    }
}
