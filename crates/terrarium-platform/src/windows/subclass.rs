//! The strip window's subclass: it sees every message sent to winit's window before winit does.
//! winit's message hook sees only posted messages, and the `AppBar` protocol needs sent ones:
//! `WM_ACTIVATE`, `WM_WINDOWPOSCHANGED` and the style changes winit makes.
//!
//! The procedure runs on the window's thread, inside whatever call sent the message, including
//! winit's own handlers and `SHAppBarMessage` calls. So it holds no borrow across a call that can
//! re-enter it, and nothing in it can panic: a panic cannot unwind out of an `extern "system"`
//! function, and would abort the process.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::Shell::{
    ABM_ACTIVATE, ABM_WINDOWPOSCHANGED, APPBARDATA, SHAppBarMessage, SUBCLASSPROC,
};
use windows::Win32::UI::WindowsAndMessaging::STYLESTRUCT;
use windows::core::BOOL;

use super::exit;
use crate::SystemEvent;
use crate::appbar::{self, Registered};

// The subclass functions are imported by ordinal, as a program built with the Windows SDK
// imports them: Microsoft's own `comctl32.lib` names them 410, 412 and 413. The `comctl32.dll`
// an app without a Common Controls 6 manifest loads may export them by ordinal only, and a name
// it lacks would stop the program from starting at all. `cargo xtask probe-zip` checks the
// built exe for this.
#[link(name = "comctl32.dll", kind = "raw-dylib", modifiers = "+verbatim")]
unsafe extern "system" {
    #[link_ordinal(410)]
    fn SetWindowSubclass(hwnd: HWND, proc: SUBCLASSPROC, id: usize, data: usize) -> BOOL;
    #[link_ordinal(412)]
    fn RemoveWindowSubclass(hwnd: HWND, proc: SUBCLASSPROC, id: usize) -> BOOL;
    #[link_ordinal(413)]
    fn DefSubclassProc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
}

/// Ours among the window's subclasses; comctl32 tells them apart by procedure and id.
const SUBCLASS_ID: usize = 0x7465_7272; // "terr"

/// Receives the system events the window sees.
type Observer = Rc<dyn Fn(SystemEvent)>;

/// What the procedure needs from the strip window.
pub(super) struct Subclass {
    registered: Registered,
    on_event: RefCell<Option<Observer>>,
}

impl Subclass {
    /// Events leave through `on_event`, which only posts to the event loop, so calling it here
    /// never re-enters winit.
    fn emit(&self, event: SystemEvent) {
        let on_event = self.on_event.try_borrow().ok().and_then(|f| f.clone());
        if let Some(on_event) = on_event {
            on_event(event);
        }
    }

    pub(super) fn set_observer(&self, on_event: Observer) {
        if let Ok(mut slot) = self.on_event.try_borrow_mut() {
            *slot = Some(on_event);
        }
    }
}

/// The installed subclass. Dropping it does not remove it; `remove` does, from the window's
/// thread.
pub(super) struct Installed {
    pub(super) state: Rc<Subclass>,
    /// The reference data comctl32 holds: one strong reference to `state`, released by whichever
    /// of `remove` and `WM_NCDESTROY` takes the subclass off the window.
    data: usize,
}

/// Subclasses `hwnd`. Call it on the window's thread.
pub(super) fn install(hwnd: HWND, registered: Registered) -> Option<Installed> {
    let state = Rc::new(Subclass {
        registered,
        on_event: RefCell::new(None),
    });
    let data = Rc::into_raw(Rc::clone(&state)).expose_provenance();
    // SAFETY: `hwnd` belongs to this thread (checked by the caller), `proc` has the subclass
    // procedure's signature, and `data` is a strong reference that stays alive until the
    // subclass is removed.
    let installed = unsafe { SetWindowSubclass(hwnd, Some(proc), SUBCLASS_ID, data) };
    if installed.as_bool() {
        Some(Installed { state, data })
    } else {
        // SAFETY: comctl32 refused the subclass, so nothing else holds this reference; it is
        // released once, here.
        drop(unsafe { Rc::from_raw(std::ptr::with_exposed_provenance::<Subclass>(data)) });
        None
    }
}

/// Takes the subclass off `hwnd` and releases comctl32's reference, once: after the window is
/// destroyed, or once it was already removed, `RemoveWindowSubclass` finds nothing and this does
/// nothing.
pub(super) fn remove(hwnd: HWND, data: usize) {
    // SAFETY: removing our own subclass, on the window's thread; an unknown window or a
    // subclass already gone makes the call return FALSE.
    let removed = unsafe { RemoveWindowSubclass(hwnd, Some(proc), SUBCLASS_ID) };
    if removed.as_bool() {
        // SAFETY: the subclass is off the window, so comctl32 no longer holds the reference
        // `install` gave it, and only the call that removed it gets here.
        drop(unsafe { Rc::from_raw(std::ptr::with_exposed_provenance::<Subclass>(data)) });
    }
}

impl Installed {
    pub(super) fn remove(&self, hwnd: HWND) {
        remove(hwnd, self.data);
    }
}

/// The subclass procedure.
///
/// # Safety
///
/// Only comctl32 calls it, on the window's thread, with the `data` that `install` passed.
unsafe extern "system" fn proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let ptr = std::ptr::with_exposed_provenance::<Subclass>(data);
    // SAFETY: `data` is the strong reference `install` made, which lives until the subclass is
    // removed, and the procedure runs only while it is installed. This call takes its own
    // reference, so the state outlives the call even if the strip window is dropped during it.
    let state = unsafe {
        Rc::increment_strong_count(ptr);
        Rc::from_raw(ptr)
    };
    if counting()
        && let Some(name) = appbar::message_name(msg, wparam.0, state.registered)
    {
        count(name, Seen::Subclass);
    }

    if msg != 0 && msg == state.registered.appbar {
        match wparam.0 {
            appbar::ABN_POSCHANGED => state.emit(SystemEvent::AppBarPosChanged),
            appbar::ABN_STATECHANGE => state.emit(SystemEvent::AppBarStateChanged),
            // Only a hint. The whole answer is in `lParam`; Microsoft's sample reads an
            // uninitialised variable here and falls through.
            appbar::ABN_FULLSCREENAPP => state.emit(SystemEvent::FullscreenApp(lparam.0 != 0)),
            _ => {}
        }
        return LRESULT(0);
    }
    if msg != 0 && msg == state.registered.taskbar_created {
        state.emit(SystemEvent::TaskbarCreated);
    }

    match msg {
        appbar::WM_STYLECHANGING if wparam.0.cast_signed() == appbar::GWL_EXSTYLE => {
            let style =
                std::ptr::with_exposed_provenance_mut::<STYLESTRUCT>(lparam.0.cast_unsigned());
            // SAFETY: for `WM_STYLECHANGING`, `lParam` points to a `STYLESTRUCT` the system owns
            // for the length of the message, and the window may change its `styleNew`.
            if let Some(style) = unsafe { style.as_mut() } {
                style.styleNew = appbar::guard_ex_style(style.styleOld, style.styleNew);
            }
        }
        appbar::WM_ACTIVATE if exit::is_registered(hwnd) => {
            notify(hwnd, ABM_ACTIVATE, appbar::activate_lparam(wparam.0));
        }
        appbar::WM_WINDOWPOSCHANGED if exit::is_registered(hwnd) => {
            notify(hwnd, ABM_WINDOWPOSCHANGED, 0);
        }
        appbar::WM_DISPLAYCHANGE => state.emit(SystemEvent::ScreensChanged),
        appbar::WM_SETTINGCHANGE if wparam.0 == appbar::SPI_SETWORKAREA => {
            state.emit(SystemEvent::WorkAreaChanged);
        }
        appbar::WM_NCDESTROY => {
            exit::remove_if_registered(hwnd);
            remove(hwnd, data);
        }
        _ => {}
    }
    // SAFETY: passing the message on down the window's subclass chain, from inside this
    // subclass's procedure, with the arguments it came with.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Tells Explorer about the bar's activation or move, as the `AppBar` protocol asks.
fn notify(hwnd: HWND, message: u32, lparam: isize) {
    let mut data = APPBARDATA {
        cbSize: super::struct_size::<APPBARDATA>(),
        hWnd: hwnd,
        uCallbackMessage: 0,
        uEdge: 0,
        rc: RECT::default(),
        lParam: LPARAM(lparam),
    };
    // SAFETY: `data` is a complete `APPBARDATA` for our registered bar that outlives the call.
    unsafe { SHAppBarMessage(message, &raw mut data) };
}

/// Where a counted message was seen.
#[derive(Debug, Clone, Copy)]
pub(super) enum Seen {
    /// The subclass: sent and posted messages to the strip window.
    Subclass,
    /// winit's message hook: posted messages to any of the thread's windows.
    Hook,
}

thread_local! {
    /// Counts per message name, `(subclass, hook)`. `None` until the probe turns counting on.
    static TALLY: RefCell<Option<BTreeMap<&'static str, (u64, u64)>>> =
        const { RefCell::new(None) };
}

/// Whether the probe turned counting on. Checked before a message is named, so a strip that is
/// not probing does no work per message.
pub(super) fn counting() -> bool {
    TALLY.with(|tally| tally.try_borrow().is_ok_and(|tally| tally.is_some()))
}

pub(super) fn start_counting() {
    TALLY.with(|tally| {
        if let Ok(mut tally) = tally.try_borrow_mut() {
            tally.get_or_insert_with(BTreeMap::new);
        }
    });
}

pub(super) fn count(name: &'static str, seen: Seen) {
    TALLY.with(|tally| {
        if let Ok(mut tally) = tally.try_borrow_mut()
            && let Some(tally) = tally.as_mut()
        {
            let (subclass, hook) = tally.entry(name).or_default();
            let counter = match seen {
                Seen::Subclass => subclass,
                Seen::Hook => hook,
            };
            *counter = counter.saturating_add(1);
        }
    });
}

/// The counts since the last call, and a fresh start.
pub(super) fn take_counts() -> Vec<super::MessageCount> {
    TALLY.with(|tally| {
        let Ok(mut tally) = tally.try_borrow_mut() else {
            return Vec::new();
        };
        tally.as_mut().map_or_else(Vec::new, |tally| {
            std::mem::take(tally)
                .into_iter()
                .map(|(name, (subclass, hook))| super::MessageCount {
                    name,
                    subclass,
                    hook,
                })
                .collect()
        })
    })
}
