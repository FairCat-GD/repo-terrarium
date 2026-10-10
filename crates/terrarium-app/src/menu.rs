//! The strip's right-click menu: Settings and Quit, and in dev builds the debug window.

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
    settings: MenuItem,
    quit: MenuItem,
    #[cfg(feature = "dev")]
    debug: MenuItem,
}

impl StripMenu {
    /// Must run on the main thread.
    pub fn new() -> Result<Self, muda::Error> {
        let menu = Menu::new();
        #[cfg(feature = "dev")]
        let debug = MenuItem::new(copy::menu_debug(), true, None);
        #[cfg(feature = "dev")]
        menu.append(&debug)?;
        let settings = MenuItem::new(copy::menu_settings(), true, None);
        menu.append(&settings)?;
        let quit = MenuItem::new(copy::menu_quit(), true, None);
        menu.append(&quit)?;
        Ok(Self {
            menu,
            settings,
            quit,
            #[cfg(feature = "dev")]
            debug,
        })
    }

    pub fn menu(&self) -> &Menu {
        &self.menu
    }

    pub fn is_quit(&self, id: &MenuId) -> bool {
        self.quit.id() == id
    }

    pub fn is_settings(&self, id: &MenuId) -> bool {
        self.settings.id() == id
    }

    #[cfg(feature = "dev")]
    pub fn is_debug(&self, id: &MenuId) -> bool {
        self.debug.id() == id
    }
}
