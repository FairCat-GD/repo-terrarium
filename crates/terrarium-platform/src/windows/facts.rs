//! What the strip reads of a window to judge whether it is full screen (`crate::fullscreen`).
//! Only reads: nothing here changes a window.

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MapWindowPoints, MonitorFromWindow};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetForegroundWindow, GetPropW, GetWindowLongPtrW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
};
use windows::core::w;

use super::{class_name, from_rect, handle_of, monitor_info, window_rect};
use crate::fullscreen::WindowFacts;

/// The window the person is working in, if there is one.
pub(super) fn foreground() -> Option<HWND> {
    // SAFETY: only reads which window is in front; none gives a null handle.
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_invalid()).then_some(hwnd)
}

/// What the strip reads of `hwnd`; `None` once the window is gone.
pub(super) fn of(hwnd: HWND) -> Option<WindowFacts> {
    // SAFETY: only asks whether a window exists; any value is accepted.
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return None;
    }
    let mut process = 0;
    // SAFETY: only reads the window's process into `process`, which outlives the call.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut process)) };
    // SAFETY: reads this process's id; it cannot fail.
    let own_process = process == unsafe { GetCurrentProcessId() };
    // SAFETY: only reads whether the window is shown.
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    // SAFETY: only reads whether the window is minimised.
    let iconic = unsafe { IsIconic(hwnd) }.as_bool();
    // SAFETY: only reads a property; a window without it gives a null handle.
    let non_rude = !unsafe { GetPropW(hwnd, w!("NonRudeHWND")) }.is_invalid();
    // SAFETY: only reads the window's style; the value is the style's bits.
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
    // SAFETY: as above, for its extended style.
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    // SAFETY: only reads the nearest monitor of a window; any value is accepted.
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    Some(WindowFacts {
        handle: handle_of(hwnd),
        own_process,
        visible,
        iconic,
        cloaked: cloaked(hwnd),
        non_rude,
        class: class_name(hwnd).unwrap_or_default(),
        style: low_bits(style),
        ex_style: low_bits(ex_style),
        window: window_rect(hwnd)?,
        client: client_on_screen(hwnd),
        monitor: monitor_info(monitor).map(|(rects, _)| rects.monitor),
    })
}

/// A style read as a pointer-sized value: its bits are the low 32.
fn low_bits(value: isize) -> u32 {
    u32::try_from(value.cast_unsigned() & 0xFFFF_FFFF).unwrap_or(0)
}

/// Whether DWM hides the window: one on another virtual desktop, or a suspended app's.
fn cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0_u32;
    // SAFETY: `cloaked` is a writable `u32`, the size the attribute takes, and outlives the
    // call; a window DWM does not know makes it fail without writing.
    let read = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            super::struct_size::<u32>(),
        )
    };
    read.is_ok() && cloaked != 0
}

/// The window's client area in screen coordinates, as this per-monitor-aware process sees it.
fn client_on_screen(hwnd: HWND) -> Option<super::PixelRect> {
    let mut client = RECT::default();
    // SAFETY: `client` outlives the call; an unknown window makes it fail.
    unsafe { GetClientRect(hwnd, &raw mut client) }.ok()?;
    let mut corners = [
        POINT {
            x: client.left,
            y: client.top,
        },
        POINT {
            x: client.right,
            y: client.bottom,
        },
    ];
    // SAFETY: maps two points of a window's client area to the screen, in place; the slice
    // gives the call its length.
    unsafe { MapWindowPoints(Some(hwnd), None, &mut corners) };
    Some(from_rect(RECT {
        left: corners[0].x,
        top: corners[0].y,
        right: corners[1].x,
        bottom: corners[1].y,
    }))
}
