//! Giving the reserved edge back however the process ends. Explorer keeps an `AppBar`'s
//! reservation until the bar is removed, so a process that dies without `ABM_REMOVE` leaves
//! maximised windows stopping short of an empty band.
//!
//! One global holds the registered window, and every exit path swaps it to zero before
//! removing the bar, so the removal happens at most once, from whichever thread gets there
//! first:
//! - a normal exit, through the strip window's `release` and `Drop`;
//! - a panic on the thread that registered the bar, through the panic hook;
//! - Ctrl-C, Ctrl-Break and a closed console, through the console control handler;
//! - an unhandled structured exception, through the unhandled-exception filter.
//!
//! Nothing runs for `taskkill /f`, or for a fast-fail abort (a double panic, a stack overflow).
//! The stale-reservation check at the next launch covers those.

use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::{Once, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::System::Diagnostics::Debug::{
    EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS, LPTOP_LEVEL_EXCEPTION_FILTER,
    SetUnhandledExceptionFilter,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Shell::{ABM_REMOVE, APPBARDATA, SHAppBarMessage};
use windows::core::BOOL;

/// The registered bar's window handle, or 0 when no bar is registered.
static REGISTERED: AtomicIsize = AtomicIsize::new(0);
/// The Win32 id of the thread that registers the bar, the event loop's. A panic there ends the
/// process. It is compared with `GetCurrentThreadId`, which needs no thread-local storage, so
/// the check works even in a panic raised while thread-locals are being destroyed.
static OWNER: AtomicU32 = AtomicU32::new(0);
/// The filter that was installed before ours, which ours hands on to.
static PREVIOUS_FILTER: OnceLock<LPTOP_LEVEL_EXCEPTION_FILTER> = OnceLock::new();

fn handle_value(hwnd: HWND) -> isize {
    hwnd.0.addr().cast_signed()
}

/// Records `hwnd` as the registered bar. Called before every `ABM_NEW`, so an exit between the
/// registration and this record cannot leave the edge reserved.
pub(super) fn mark_registered(hwnd: HWND) {
    REGISTERED.store(handle_value(hwnd), Ordering::SeqCst);
}

pub(super) fn is_registered(hwnd: HWND) -> bool {
    REGISTERED.load(Ordering::SeqCst) == handle_value(hwnd)
}

/// Removes the registered bar, if there is one. Safe from any thread and any exit path: it
/// allocates nothing, and only the caller that swaps the handle out sends `ABM_REMOVE`.
pub(super) fn remove_registered() {
    send_remove(REGISTERED.swap(0, Ordering::SeqCst));
}

/// Removes the bar only if it is `hwnd`'s, in one atomic step, so a window being destroyed
/// never removes another window's bar.
pub(super) fn remove_if_registered(hwnd: HWND) {
    let hwnd = handle_value(hwnd);
    if REGISTERED
        .compare_exchange(hwnd, 0, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        send_remove(hwnd);
    }
}

fn send_remove(hwnd: isize) {
    if hwnd == 0 {
        return;
    }
    let mut data = APPBARDATA {
        cbSize: super::struct_size::<APPBARDATA>(),
        hWnd: HWND(std::ptr::without_provenance_mut(hwnd.cast_unsigned())),
        uCallbackMessage: 0,
        uEdge: 0,
        rc: RECT::default(),
        lParam: LPARAM(0),
    };
    // SAFETY: `data` is a complete `APPBARDATA` that outlives the call. `ABM_REMOVE` reads only
    // its size and window handle, and a handle Explorer does not know is ignored.
    unsafe { SHAppBarMessage(ABM_REMOVE, &raw mut data) };
}

/// Installs the panic hook, the console control handler and the unhandled-exception filter,
/// once per process. Call it on the thread that registers the bar.
pub(super) fn install_handlers() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        // SAFETY: reads the calling thread's id; it cannot fail.
        OWNER.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Removed before the previous hook prints, since a full stderr pipe can block the
            // print. Panics on other threads are contained (I-16), so the strip keeps its edge
            // through them. SAFETY: as above.
            if OWNER.load(Ordering::SeqCst) == unsafe { GetCurrentThreadId() } {
                remove_registered();
            }
            previous(info);
        }));
        // SAFETY: the handler is a plain function that lives as long as the process, and it
        // touches only atomics and `SHAppBarMessage`, which are safe on the thread the console
        // creates for it.
        let _ = unsafe { SetConsoleCtrlHandler(Some(on_console_control), true) };
        // SAFETY: the filter is a plain function that lives as long as the process; the filter
        // it replaces is kept and called by ours.
        let previous = unsafe { SetUnhandledExceptionFilter(Some(on_unhandled_exception)) };
        let _ = PREVIOUS_FILTER.set(previous);
    });
}

/// Ctrl-C, Ctrl-Break, a closed console, logoff or shutdown. The bar is removed, and FALSE
/// passes the event on, so the default handler still ends the process.
extern "system" fn on_console_control(_control: u32) -> BOOL {
    remove_registered();
    BOOL(0)
}

/// An exception nothing else handled. It runs on a damaged stack, so it only removes the bar
/// (no allocation, no formatting) before handing on to the filter that was there before.
///
/// # Safety
///
/// Only the system calls it, with the exception's pointers, as an unhandled-exception filter.
unsafe extern "system" fn on_unhandled_exception(info: *const EXCEPTION_POINTERS) -> i32 {
    remove_registered();
    match PREVIOUS_FILTER.get().copied().flatten() {
        // SAFETY: the previous filter was installed in this process for exactly this call, and
        // `info` is the pointer the system gave us for it.
        Some(previous) => unsafe { previous(info) },
        None => EXCEPTION_CONTINUE_SEARCH,
    }
}
