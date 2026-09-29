//! Windows: the context menu. The `AppBar` registration is its own module's work.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Pops up `menu` at the pointer over `window`. Call it from the thread that owns the window
/// (the event loop's); it returns when the menu closes.
pub fn show_context_menu(menu: &muda::Menu, window: &impl HasWindowHandle) {
    use muda::ContextMenu;
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: raw-window-handle 0.6 guarantees the HWND is a valid window while the handle is
    // borrowed, which covers the whole call.
    unsafe {
        menu.show_context_menu_for_hwnd(handle.hwnd.get(), None);
    }
}
