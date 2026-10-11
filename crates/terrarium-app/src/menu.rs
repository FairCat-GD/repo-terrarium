//! The strip's right-click menu: Settings, Time-lapse, Pause or Resume, and Quit; in dev builds
//! the debug window too. Time-lapse is shown and stays off until there is a window for it to
//! open.

use muda::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
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

/// What a menu item asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Settings,
    Pause,
    Quit,
    #[cfg(feature = "dev")]
    Debug,
}

pub struct StripMenu {
    menu: Menu,
    settings: MenuItem,
    pause: MenuItem,
    quit: MenuItem,
    #[cfg(feature = "dev")]
    debug: MenuItem,
}

/// The pause item's word: what choosing it does.
pub fn pause_text(paused: bool) -> &'static str {
    if paused {
        copy::menu_resume()
    } else {
        copy::menu_pause()
    }
}

impl StripMenu {
    /// Must run on the main thread.
    pub fn new() -> Result<Self, muda::Error> {
        let menu = Menu::new();
        #[cfg(feature = "dev")]
        let debug = MenuItem::new(copy::menu_debug(), true, None);
        #[cfg(feature = "dev")]
        menu.append_items(&[&debug, &PredefinedMenuItem::separator()])?;
        let settings = MenuItem::new(copy::menu_settings(), true, None);
        let time_lapse = MenuItem::new(copy::menu_time_lapse(), false, None);
        let pause = MenuItem::new(pause_text(false), true, None);
        let quit = MenuItem::new(copy::menu_quit(), true, None);
        menu.append_items(&[
            &settings,
            &time_lapse,
            &pause,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;
        Ok(Self {
            menu,
            settings,
            pause,
            quit,
            #[cfg(feature = "dev")]
            debug,
        })
    }

    pub fn menu(&self) -> &Menu {
        &self.menu
    }

    /// What the item `id` asks for, if it is one of this menu's.
    pub fn choice(&self, id: &MenuId) -> Option<Choice> {
        #[cfg(feature = "dev")]
        if self.debug.id() == id {
            return Some(Choice::Debug);
        }
        [
            (&self.settings, Choice::Settings),
            (&self.pause, Choice::Pause),
            (&self.quit, Choice::Quit),
        ]
        .into_iter()
        .find(|(item, _)| item.id() == id)
        .map(|(_, choice)| choice)
    }

    /// Shows the pause item as the strip now is: Pause while it runs, Resume while it is
    /// paused, and off with no scene to pause.
    pub fn sync(&self, paused: Option<bool>) {
        self.pause.set_text(pause_text(paused.unwrap_or(false)));
        self.pause.set_enabled(paused.is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pause_item_says_what_choosing_it_does() {
        assert_eq!(pause_text(false), copy::menu_pause());
        assert_eq!(pause_text(true), copy::menu_resume());
    }
}
