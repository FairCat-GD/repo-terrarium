//! The Windows `AppBar`'s arithmetic: its rectangles in physical pixels, the extended-style
//! guard, and the names of the window messages the probe counts. It is pure, so it is compiled
//! and tested on every OS; the `windows` module makes the calls.
//!
//! The Win32 numbers are kept here so the tests run on macOS. The `windows` module asserts each
//! one against the `windows` crate at compile time, so a wrong number fails the Windows build.

use crate::geometry::Edge;

/// A rectangle in physical pixels with Win32 `RECT` semantics: virtual-screen coordinates, y
/// growing downward, the right and bottom edges exclusive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PixelRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PixelRect {
    pub fn width(&self) -> i32 {
        self.right.saturating_sub(self.left)
    }

    pub fn height(&self) -> i32 {
        self.bottom.saturating_sub(self.top)
    }
}

/// The band `thickness` pixels deep along `edge` of `area`, as wide as `area`.
///
/// One rule serves the three places the strip's rectangle comes from:
/// - on the monitor (`rcMonitor`), the band the strip proposes to `ABM_QUERYPOS`;
/// - on the rectangle `ABM_QUERYPOS` returns, the band for `ABM_SETPOS`. The system moves the
///   side on `edge` clear of the taskbar and other bars, and the protocol asks the bar to
///   re-derive the far side from its own thickness;
/// - on the work area (`rcWork`), the fallback band when the edge cannot be reserved, which
///   never covers the taskbar.
pub fn band(area: PixelRect, edge: Edge, thickness: u32) -> PixelRect {
    let thickness = i32::try_from(thickness).unwrap_or(i32::MAX);
    match edge {
        Edge::Bottom => PixelRect {
            top: area.bottom.saturating_sub(thickness),
            ..area
        },
        Edge::Top => PixelRect {
            bottom: area.top.saturating_add(thickness),
            ..area
        },
    }
}

/// The work area the taskbars alone explain: `monitor` minus each taskbar that lies along one
/// of its sides. A taskbar belongs to a side when it spans the monitor's width (top or bottom)
/// or height (left or right) and reaches that side.
///
/// Every side only ever moves inward to the bar's near edge, so another monitor's taskbar,
/// which lies wholly outside this one, moves nothing, and all taskbars can be passed in.
pub fn expected_work_area(monitor: PixelRect, taskbars: &[PixelRect]) -> PixelRect {
    let mut work = monitor;
    for bar in taskbars {
        let spans_width = bar.left <= monitor.left && bar.right >= monitor.right;
        let spans_height = bar.top <= monitor.top && bar.bottom >= monitor.bottom;
        if spans_width && bar.bottom >= monitor.bottom {
            work.bottom = work.bottom.min(bar.top);
        } else if spans_width && bar.top <= monitor.top {
            work.top = work.top.max(bar.bottom);
        } else if spans_height && bar.left <= monitor.left {
            work.left = work.left.max(bar.right);
        } else if spans_height && bar.right >= monitor.right {
            work.right = work.right.min(bar.left);
        }
    }
    work
}

/// Whether the work area is shorter on `edge` than the taskbars explain: the band a strip that
/// died without removing its bar leaves reserved.
///
/// Only a smaller work area counts, so an auto-hidden taskbar, or one whose window is larger
/// than its reservation, never does. Only the strip's own edge counts, so a bar docked on
/// another edge (the Magnifier, another app's `AppBar`) never sets off the clearing.
pub fn stale_suspected(
    monitor: PixelRect,
    work: PixelRect,
    taskbars: &[PixelRect],
    edge: Edge,
) -> bool {
    let expected = expected_work_area(monitor, taskbars);
    match edge {
        Edge::Bottom => work.bottom < expected.bottom,
        Edge::Top => work.top > expected.top,
    }
}

/// How deep an auto-hide taskbar's sliver on the strip's edge is taken to be when it cannot be
/// read: Windows leaves a hidden taskbar a line or two on screen, where the pointer reveals it.
pub const AUTOHIDE_SLIVER: i32 = 2;
/// The deepest on-screen part of an auto-hide taskbar still read as its hidden sliver. Deeper,
/// the taskbar is showing, and its height is not what the strip must leave free.
pub const AUTOHIDE_SLIVER_MAX: i32 = 8;

/// How far the strip stays off `edge` of `monitor` for an auto-hide taskbar there (`bar`, its
/// window rectangle), so the pointer can still reach the taskbar's sliver and reveal it. An
/// auto-hide taskbar reserves nothing, so the strip would otherwise cover it.
pub fn autohide_inset(monitor: PixelRect, edge: Edge, bar: Option<PixelRect>) -> i32 {
    let Some(bar) = bar else {
        return 0;
    };
    if bar.right <= monitor.left || bar.left >= monitor.right {
        return 0;
    }
    let on_screen = match edge {
        Edge::Bottom if bar.bottom >= monitor.bottom => {
            monitor.bottom.saturating_sub(bar.top.max(monitor.top))
        }
        Edge::Top if bar.top <= monitor.top => {
            bar.bottom.min(monitor.bottom).saturating_sub(monitor.top)
        }
        _ => return 0,
    };
    if (1..=AUTOHIDE_SLIVER_MAX).contains(&on_screen) {
        on_screen
    } else {
        AUTOHIDE_SLIVER
    }
}

/// `area` with `by` pixels taken off its `edge` side.
pub fn inset(area: PixelRect, edge: Edge, by: i32) -> PixelRect {
    match edge {
        Edge::Bottom => PixelRect {
            bottom: area.bottom.saturating_sub(by),
            ..area
        },
        Edge::Top => PixelRect {
            top: area.top.saturating_add(by),
            ..area
        },
    }
}

/// A `WINDOWPOS` as `WM_WINDOWPOSCHANGING` carries it: where the window is about to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPos {
    pub x: i32,
    pub y: i32,
    pub cx: i32,
    pub cy: i32,
    pub flags: u32,
}

/// Where Windows parks a minimised window.
const MINIMISED_PARK: i32 = -32000;

/// The move `pos` corrected onto `pinned`, the rectangle the strip's edge was granted, or `None`
/// when it needs no correction. Whoever moves or sizes the strip (winit's handling of a DPI
/// change, a snap, another app) lands on the granted rectangle, so no move can start a
/// back-and-forth of moves and placements. A change of order or visibility alone, and a
/// minimise, pass as they are.
pub fn pin_window_pos(pos: WindowPos, pinned: PixelRect) -> Option<WindowPos> {
    if pos.x == MINIMISED_PARK && pos.y == MINIMISED_PARK {
        return None;
    }
    let mut pinned_pos = pos;
    if pos.flags & SWP_NOMOVE == 0 {
        pinned_pos.x = pinned.left;
        pinned_pos.y = pinned.top;
    }
    if pos.flags & SWP_NOSIZE == 0 {
        pinned_pos.cx = pinned.width();
        pinned_pos.cy = pinned.height();
    }
    (pinned_pos != pos).then_some(pinned_pos)
}

/// Where the strip's bar is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarState {
    /// Not registered yet; the next placement registers it.
    New,
    /// Registered: the edge is reserved.
    Reserved,
    /// Registration failed: a plain topmost window inside the work area.
    Unreserved,
    /// Given back on exit. Nothing is placed any more, so a placement racing an exit handler
    /// cannot reserve the edge again.
    Released,
}

/// What Explorer starting again asks of the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskbarAction {
    /// Ask for the bar once (`ABM_NEW`). A new Explorer has forgotten it; one that has not
    /// answers FALSE, which also means an error, so there is no removing and asking again.
    AskOnce,
    /// Register from the start, the stale check included: there was no taskbar to register
    /// with (a login that started the strip first), or there is no bar yet.
    RegisterAfresh,
    Nothing,
}

/// What a `TaskbarCreated` asks of a bar in `state`. Either way the next placement sends its
/// rectangle again, since the one Explorer knew may be gone with it.
pub fn on_taskbar_created(state: BarState) -> TaskbarAction {
    match state {
        BarState::Reserved => TaskbarAction::AskOnce,
        BarState::New | BarState::Unreserved => TaskbarAction::RegisterAfresh,
        BarState::Released => TaskbarAction::Nothing,
    }
}

/// Whether a placement on `now` (a monitor's device name and an edge) registers the bar afresh:
/// it moved to another monitor or edge than the one it was registered on, where a strip that
/// crashed may have left its own. The device name, not the `HMONITOR`, which Windows makes
/// anew on every change of displays.
pub fn fresh_registration(last: Option<(&str, Edge)>, now: (&str, Edge)) -> bool {
    last.is_some_and(|last| last != now)
}

/// The `ABE_` value for an edge.
pub fn abe(edge: Edge) -> u32 {
    match edge {
        Edge::Top => ABE_TOP,
        Edge::Bottom => ABE_BOTTOM,
    }
}

/// The `lParam` for `ABM_ACTIVATE`: whether the `WM_ACTIVATE` it answers activated the window.
/// The low word of that message's `wParam` is `WA_INACTIVE` when it did not.
pub fn activate_lparam(wparam: usize) -> isize {
    isize::from(wparam & 0xFFFF != WA_INACTIVE)
}

/// The extended style the strip keeps, whatever `new` asks for.
///
/// - Tool window: out of the taskbar and Alt-Tab.
/// - No-activate: a click never takes focus from the app in use.
/// - Never an app window, which would force a taskbar button.
/// - Topmost stays as it was, since only `SetWindowPos` changes it.
///
/// winit rewrites the whole extended style whenever its own window flags change, so the strip's
/// `WM_STYLECHANGING` handler passes every change through this.
pub fn guard_ex_style(old: u32, new: u32) -> u32 {
    ((new | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) & !WS_EX_APPWINDOW & !WS_EX_TOPMOST)
        | (old & WS_EX_TOPMOST)
}

/// Message ids Windows assigns when the process registers them, so they differ per session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Registered {
    /// The `AppBar`'s callback message.
    pub appbar: u32,
    /// Broadcast when Explorer starts, and restarts.
    pub taskbar_created: u32,
    /// The shell hook's message: which window was activated, created or destroyed.
    pub shellhook: u32,
}

/// The name of a message the probe counts, or `None` for one it does not. A registered id of 0
/// (registration failed) never matches.
pub fn message_name(msg: u32, wparam: usize, registered: Registered) -> Option<&'static str> {
    if msg != 0 && msg == registered.appbar {
        return Some(match wparam {
            ABN_STATECHANGE => "ABN_STATECHANGE",
            ABN_POSCHANGED => "ABN_POSCHANGED",
            ABN_FULLSCREENAPP => "ABN_FULLSCREENAPP",
            ABN_WINDOWARRANGE => "ABN_WINDOWARRANGE",
            _ => "ABN_other",
        });
    }
    if msg != 0 && msg == registered.taskbar_created {
        return Some("TaskbarCreated");
    }
    if msg != 0 && msg == registered.shellhook {
        let rude = wparam & HSHELL_HIGHBIT != 0;
        return Some(match (wparam & !HSHELL_HIGHBIT, rude) {
            (HSHELL_WINDOWCREATED, _) => "HSHELL_WINDOWCREATED",
            (HSHELL_WINDOWDESTROYED, _) => "HSHELL_WINDOWDESTROYED",
            (HSHELL_WINDOWACTIVATED, false) => "HSHELL_WINDOWACTIVATED",
            (HSHELL_WINDOWACTIVATED, true) => "HSHELL_RUDEAPPACTIVATED",
            (HSHELL_REDRAW, false) => "HSHELL_REDRAW",
            (HSHELL_REDRAW, true) => "HSHELL_FLASH",
            (HSHELL_MONITORCHANGED, _) => "HSHELL_MONITORCHANGED",
            (HSHELL_FULLSCREENENTER, _) => "HSHELL_FULLSCREENENTER",
            (HSHELL_FULLSCREENEXIT, _) => "HSHELL_FULLSCREENEXIT",
            _ => "HSHELL_other",
        });
    }
    if msg == WM_POWERBROADCAST {
        return Some(match wparam {
            PBT_APMSUSPEND => "PBT_APMSUSPEND",
            PBT_APMRESUMESUSPEND => "PBT_APMRESUMESUSPEND",
            PBT_APMPOWERSTATUSCHANGE => "PBT_APMPOWERSTATUSCHANGE",
            PBT_APMRESUMEAUTOMATIC => "PBT_APMRESUMEAUTOMATIC",
            PBT_POWERSETTINGCHANGE => "PBT_POWERSETTINGCHANGE",
            _ => "PBT_other",
        });
    }
    WATCHED
        .iter()
        .find(|(id, _)| *id == msg)
        .map(|(_, name)| *name)
}

/// The fixed messages the probe counts: activation, placement, style, display and session
/// changes, and the two button presses (power changes are named by their event above). Whether
/// each arrives sent (the subclass sees it) or posted (winit's message hook sees it too) is what
/// the probe compares.
const WATCHED: &[(u32, &str)] = &[
    (WM_SHOWWINDOW, "WM_SHOWWINDOW"),
    (WM_ACTIVATE, "WM_ACTIVATE"),
    (WM_SETTINGCHANGE, "WM_SETTINGCHANGE"),
    (WM_ACTIVATEAPP, "WM_ACTIVATEAPP"),
    (WM_MOUSEACTIVATE, "WM_MOUSEACTIVATE"),
    (WM_WINDOWPOSCHANGING, "WM_WINDOWPOSCHANGING"),
    (WM_WINDOWPOSCHANGED, "WM_WINDOWPOSCHANGED"),
    (WM_STYLECHANGING, "WM_STYLECHANGING"),
    (WM_DISPLAYCHANGE, "WM_DISPLAYCHANGE"),
    (WM_NCACTIVATE, "WM_NCACTIVATE"),
    (WM_LBUTTONDOWN, "WM_LBUTTONDOWN"),
    (WM_RBUTTONDOWN, "WM_RBUTTONDOWN"),
    (WM_DPICHANGED, "WM_DPICHANGED"),
    (WM_ENDSESSION, "WM_ENDSESSION"),
];

// Win32 values, checked against the `windows` crate in `windows::constants`.
pub const ABE_TOP: u32 = 1;
pub const ABE_BOTTOM: u32 = 3;
pub const ABN_STATECHANGE: usize = 0;
pub const ABN_POSCHANGED: usize = 1;
pub const ABN_FULLSCREENAPP: usize = 2;
pub const ABN_WINDOWARRANGE: usize = 3;
pub const WA_INACTIVE: usize = 0;
pub const GWL_EXSTYLE: isize = -20;
pub const SPI_SETWORKAREA: usize = 0x002F;
pub const WS_EX_TOPMOST: u32 = 0x0000_0008;
pub const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
pub const WS_EX_APPWINDOW: u32 = 0x0004_0000;
pub const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
pub const WM_SHOWWINDOW: u32 = 0x0018;
pub const WM_ACTIVATE: u32 = 0x0006;
pub const WM_SETTINGCHANGE: u32 = 0x001A;
pub const WM_ACTIVATEAPP: u32 = 0x001C;
pub const WM_MOUSEACTIVATE: u32 = 0x0021;
pub const WM_WINDOWPOSCHANGING: u32 = 0x0046;
pub const WM_WINDOWPOSCHANGED: u32 = 0x0047;
pub const WM_STYLECHANGING: u32 = 0x007C;
pub const WM_DISPLAYCHANGE: u32 = 0x007E;
pub const WM_NCDESTROY: u32 = 0x0082;
pub const WM_NCACTIVATE: u32 = 0x0086;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_POWERBROADCAST: u32 = 0x0218;
pub const WM_DPICHANGED: u32 = 0x02E0;
pub const WM_ENDSESSION: u32 = 0x0016;
pub const SWP_NOSIZE: u32 = 0x0001;
pub const SWP_NOMOVE: u32 = 0x0002;
pub const PBT_APMSUSPEND: usize = 0x0004;
pub const PBT_APMRESUMESUSPEND: usize = 0x0007;
pub const PBT_APMPOWERSTATUSCHANGE: usize = 0x000A;
pub const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
pub const PBT_POWERSETTINGCHANGE: usize = 0x8013;
pub const WS_BORDER: u32 = 0x0080_0000;
pub const WS_CAPTION: u32 = 0x00C0_0000;
pub const WS_THICKFRAME: u32 = 0x0004_0000;
pub const WS_POPUP: u32 = 0x8000_0000;
pub const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
pub const WS_EX_LAYERED: u32 = 0x0008_0000;
pub const HSHELL_WINDOWCREATED: usize = 1;
pub const HSHELL_WINDOWDESTROYED: usize = 2;
pub const HSHELL_WINDOWACTIVATED: usize = 4;
pub const HSHELL_REDRAW: usize = 6;
pub const HSHELL_MONITORCHANGED: usize = 16;
pub const HSHELL_HIGHBIT: usize = 0x8000;
// Not in Windows' headers: what the shell hook has been seen to send as a window enters and
// leaves full screen in place. The probe counts whether they arrive.
pub const HSHELL_FULLSCREENENTER: usize = 53;
pub const HSHELL_FULLSCREENEXIT: usize = 54;

#[cfg(test)]
mod tests {
    use super::*;

    const fn rect(left: i32, top: i32, right: i32, bottom: i32) -> PixelRect {
        PixelRect {
            left,
            top,
            right,
            bottom,
        }
    }

    /// A 2560×1440 primary monitor with a 48 px taskbar along the bottom.
    const MONITOR: PixelRect = rect(0, 0, 2560, 1440);
    const TASKBAR: PixelRect = rect(0, 1392, 2560, 1440);

    #[test]
    fn the_proposal_is_the_monitors_full_width_on_its_edge() {
        assert_eq!(band(MONITOR, Edge::Bottom, 296), rect(0, 1144, 2560, 1440));
        assert_eq!(band(MONITOR, Edge::Top, 296), rect(0, 0, 2560, 296));
    }

    #[test]
    fn a_monitor_left_of_and_above_the_primary_keeps_its_own_coordinates() {
        let monitor = rect(-1920, -1080, 0, 0);
        assert_eq!(band(monitor, Edge::Bottom, 222), rect(-1920, -222, 0, 0));
        assert_eq!(band(monitor, Edge::Top, 222), rect(-1920, -1080, 0, -858));
    }

    #[test]
    fn after_the_query_the_band_sits_on_the_side_the_system_kept() {
        // Above a bottom taskbar: the system moved the bottom up and left the top where it was.
        let queried = rect(0, 1144, 2560, 1392);
        assert_eq!(band(queried, Edge::Bottom, 296), rect(0, 1096, 2560, 1392));
        // Below a top taskbar: the system moved the top down.
        let queried = rect(0, 48, 2560, 296);
        assert_eq!(band(queried, Edge::Top, 296), rect(0, 48, 2560, 344));
    }

    #[test]
    fn a_side_taskbar_narrows_the_queried_band_and_the_width_is_kept() {
        let queried = rect(62, 1144, 2560, 1440);
        let set = band(queried, Edge::Bottom, 296);
        assert_eq!(set, rect(62, 1144, 2560, 1440));
        assert_eq!(set.width(), 2498);
    }

    #[test]
    fn the_fallback_band_stays_inside_the_work_area() {
        let work = rect(0, 0, 2560, 1392);
        assert_eq!(band(work, Edge::Bottom, 296), rect(0, 1096, 2560, 1392));
    }

    #[test]
    fn a_huge_thickness_saturates_instead_of_overflowing() {
        let band = band(MONITOR, Edge::Top, u32::MAX);
        assert_eq!(band.bottom, i32::MAX);
    }

    #[test]
    fn the_expected_work_area_subtracts_each_taskbar_from_its_side() {
        assert_eq!(
            expected_work_area(MONITOR, &[TASKBAR]),
            rect(0, 0, 2560, 1392)
        );
        let top = rect(0, 0, 2560, 40);
        assert_eq!(expected_work_area(MONITOR, &[top]), rect(0, 40, 2560, 1440));
        let left = rect(0, 0, 62, 1440);
        let right = rect(2500, 0, 2560, 1440);
        assert_eq!(
            expected_work_area(MONITOR, &[left, right]),
            rect(62, 0, 2500, 1440)
        );
    }

    #[test]
    fn other_monitors_taskbars_change_nothing() {
        let others = [
            // Right of it, on the same row.
            rect(2560, 1392, 5120, 1440),
            // Below it, wider, and on top of that monitor.
            rect(-100, 1440, 3000, 1488),
            // Above it, at the bottom of that monitor.
            rect(0, -48, 2560, 0),
            // Left of it, down that monitor's right side.
            rect(-62, 0, 0, 1440),
            // Right of it, down that monitor's right side.
            rect(5058, 0, 5120, 1440),
            // Below it, at the bottom of that monitor, far from this one.
            rect(-100, 2832, 3000, 2880),
            // Above it, at the top of that monitor.
            rect(0, -1440, 2560, -1400),
        ];
        assert_eq!(expected_work_area(MONITOR, &others), MONITOR);
    }

    #[test]
    fn a_work_area_short_by_an_old_strip_on_the_strips_edge_is_suspected() {
        let work = rect(0, 0, 2560, 1392 - 296);
        assert!(stale_suspected(MONITOR, work, &[TASKBAR], Edge::Bottom));
    }

    #[test]
    fn a_work_area_the_taskbar_explains_is_not_suspected() {
        let work = rect(0, 0, 2560, 1392);
        assert!(!stale_suspected(MONITOR, work, &[TASKBAR], Edge::Bottom));
    }

    #[test]
    fn an_auto_hidden_taskbar_is_not_suspected() {
        // Hidden, the taskbar window keeps a 2 px sliver on screen and reserves nothing.
        let hidden = rect(0, 1438, 2560, 1486);
        assert!(!stale_suspected(MONITOR, MONITOR, &[hidden], Edge::Bottom));
    }

    #[test]
    fn a_reservation_on_another_edge_is_not_suspected() {
        let work = rect(0, 0, 2560, 1392 - 296);
        assert!(!stale_suspected(MONITOR, work, &[TASKBAR], Edge::Top));
        let docked_on_top = rect(0, 200, 2560, 1392);
        assert!(!stale_suspected(
            MONITOR,
            docked_on_top,
            &[TASKBAR],
            Edge::Bottom
        ));
        assert!(stale_suspected(
            MONITOR,
            docked_on_top,
            &[TASKBAR],
            Edge::Top
        ));
    }

    #[test]
    fn activation_is_read_from_the_low_word() {
        assert_eq!(activate_lparam(0), 0);
        assert_eq!(activate_lparam(1), 1);
        assert_eq!(activate_lparam(2), 1);
        // Minimised and inactive: the high word is set, the low word is `WA_INACTIVE`.
        assert_eq!(activate_lparam(0x0001_0000), 0);
    }

    #[test]
    fn the_guard_keeps_the_strip_a_tool_window_that_never_activates() {
        let winit_default = 0x0000_0100 | 0x0000_0010 | WS_EX_APPWINDOW;
        let guarded = guard_ex_style(0, winit_default);
        assert_eq!(guarded & WS_EX_TOOLWINDOW, WS_EX_TOOLWINDOW);
        assert_eq!(guarded & WS_EX_NOACTIVATE, WS_EX_NOACTIVATE);
        assert_eq!(guarded & WS_EX_APPWINDOW, 0);
        assert_eq!(guarded & 0x0000_0110, 0x0000_0110);
    }

    #[test]
    fn the_guard_leaves_topmost_as_it_was() {
        assert_eq!(
            guard_ex_style(WS_EX_TOPMOST, 0) & WS_EX_TOPMOST,
            WS_EX_TOPMOST
        );
        assert_eq!(guard_ex_style(0, WS_EX_TOPMOST) & WS_EX_TOPMOST, 0);
    }

    #[test]
    fn messages_are_named_by_id_and_appbar_callbacks_by_code() {
        let registered = Registered {
            appbar: 0xC100,
            taskbar_created: 0xC101,
            shellhook: 0xC102,
        };
        assert_eq!(
            message_name(WM_ACTIVATE, 0, registered),
            Some("WM_ACTIVATE")
        );
        assert_eq!(
            message_name(0xC100, ABN_POSCHANGED, registered),
            Some("ABN_POSCHANGED")
        );
        assert_eq!(
            message_name(0xC100, ABN_FULLSCREENAPP, registered),
            Some("ABN_FULLSCREENAPP")
        );
        assert_eq!(message_name(0xC101, 0, registered), Some("TaskbarCreated"));
        assert_eq!(message_name(0x000F, 0, registered), None);
    }

    #[test]
    fn every_appbar_notification_has_its_own_name() {
        let registered = Registered {
            appbar: 0xC100,
            taskbar_created: 0xC101,
            shellhook: 0xC102,
        };
        let names: Vec<_> = [
            ABN_STATECHANGE,
            ABN_POSCHANGED,
            ABN_FULLSCREENAPP,
            ABN_WINDOWARRANGE,
            9,
        ]
        .into_iter()
        .map(|code| message_name(0xC100, code, registered))
        .collect();
        assert_eq!(
            names,
            [
                Some("ABN_STATECHANGE"),
                Some("ABN_POSCHANGED"),
                Some("ABN_FULLSCREENAPP"),
                Some("ABN_WINDOWARRANGE"),
                Some("ABN_other"),
            ]
        );
    }

    #[test]
    fn a_failed_registration_never_names_wm_null() {
        let failed = Registered {
            appbar: 0,
            taskbar_created: 0,
            shellhook: 0,
        };
        assert_eq!(message_name(0, 0, failed), None);
    }

    #[test]
    fn shell_hook_codes_are_named_with_and_without_the_high_bit() {
        let registered = Registered {
            appbar: 0xC100,
            taskbar_created: 0xC101,
            shellhook: 0xC102,
        };
        let name = |code| message_name(0xC102, code, registered);
        assert_eq!(name(4), Some("HSHELL_WINDOWACTIVATED"));
        assert_eq!(name(0x8004), Some("HSHELL_RUDEAPPACTIVATED"));
        assert_eq!(name(6), Some("HSHELL_REDRAW"));
        assert_eq!(name(0x8006), Some("HSHELL_FLASH"));
        assert_eq!(name(16), Some("HSHELL_MONITORCHANGED"));
        assert_eq!(name(53), Some("HSHELL_FULLSCREENENTER"));
        assert_eq!(name(54), Some("HSHELL_FULLSCREENEXIT"));
        assert_eq!(name(2), Some("HSHELL_WINDOWDESTROYED"));
        assert_eq!(name(9), Some("HSHELL_other"));
    }

    #[test]
    fn power_broadcasts_are_named_by_their_event() {
        let none = Registered {
            appbar: 0,
            taskbar_created: 0,
            shellhook: 0,
        };
        let name = |code| message_name(WM_POWERBROADCAST, code, none);
        assert_eq!(name(0x12), Some("PBT_APMRESUMEAUTOMATIC"));
        assert_eq!(name(0x7), Some("PBT_APMRESUMESUSPEND"));
        assert_eq!(name(0x4), Some("PBT_APMSUSPEND"));
        assert_eq!(name(0x8013), Some("PBT_POWERSETTINGCHANGE"));
        assert_eq!(name(0x99), Some("PBT_other"));
        assert_eq!(message_name(WM_ENDSESSION, 1, none), Some("WM_ENDSESSION"));
    }

    const GRANTED: PixelRect = rect(0, 1096, 2560, 1392);

    fn pos(x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> WindowPos {
        WindowPos {
            x,
            y,
            cx,
            cy,
            flags,
        }
    }

    #[test]
    fn a_move_and_resize_off_the_granted_rect_is_put_back() {
        // winit's DPI handling: the suggested top-left, the old logical size.
        let moved = pos(0, 1000, 3200, 370, 0);
        assert_eq!(
            pin_window_pos(moved, GRANTED),
            Some(pos(0, 1096, 2560, 296, 0))
        );
    }

    #[test]
    fn a_size_only_change_gets_the_granted_size_and_keeps_its_flags() {
        let sized = pos(0, 0, 3200, 370, SWP_NOMOVE);
        assert_eq!(
            pin_window_pos(sized, GRANTED),
            Some(pos(0, 0, 2560, 296, SWP_NOMOVE))
        );
    }

    #[test]
    fn a_move_only_change_gets_the_granted_position() {
        let moved = pos(40, 40, 0, 0, SWP_NOSIZE);
        assert_eq!(
            pin_window_pos(moved, GRANTED),
            Some(pos(0, 1096, 0, 0, SWP_NOSIZE))
        );
    }

    #[test]
    fn a_change_of_order_alone_is_left_alone() {
        let ordered = pos(5, 5, 9, 9, SWP_NOMOVE | SWP_NOSIZE);
        assert_eq!(pin_window_pos(ordered, GRANTED), None);
    }

    #[test]
    fn a_minimise_is_left_alone() {
        let parked = pos(-32000, -32000, 160, 28, 0);
        assert_eq!(pin_window_pos(parked, GRANTED), None);
    }

    #[test]
    fn a_move_onto_the_granted_rect_needs_no_correction() {
        let ours = pos(0, 1096, 2560, 296, 0);
        assert_eq!(pin_window_pos(ours, GRANTED), None);
    }

    #[test]
    fn without_an_autohide_taskbar_there_is_no_inset() {
        assert_eq!(autohide_inset(MONITOR, Edge::Bottom, None), 0);
    }

    #[test]
    fn a_hidden_taskbar_keeps_its_sliver_free() {
        let hidden = rect(0, 1438, 2560, 1486);
        assert_eq!(autohide_inset(MONITOR, Edge::Bottom, Some(hidden)), 2);
        let deeper = rect(0, 1435, 2560, 1483);
        assert_eq!(autohide_inset(MONITOR, Edge::Bottom, Some(deeper)), 5);
        let gone = rect(0, 1440, 2560, 1488);
        assert_eq!(
            autohide_inset(MONITOR, Edge::Bottom, Some(gone)),
            AUTOHIDE_SLIVER
        );
    }

    #[test]
    fn a_showing_taskbar_insets_by_the_sliver_not_its_height() {
        assert_eq!(
            autohide_inset(MONITOR, Edge::Bottom, Some(TASKBAR)),
            AUTOHIDE_SLIVER
        );
    }

    #[test]
    fn a_taskbar_beside_the_monitor_or_on_another_edge_insets_nothing() {
        let beside = rect(2560, 1438, 5120, 1486);
        assert_eq!(autohide_inset(MONITOR, Edge::Bottom, Some(beside)), 0);
        assert_eq!(autohide_inset(MONITOR, Edge::Top, Some(TASKBAR)), 0);
    }

    #[test]
    fn the_top_edge_insets_downward() {
        let hidden = rect(0, -46, 2560, 2);
        assert_eq!(autohide_inset(MONITOR, Edge::Top, Some(hidden)), 2);
        assert_eq!(inset(MONITOR, Edge::Top, 2), rect(0, 2, 2560, 1440));
        assert_eq!(inset(MONITOR, Edge::Bottom, 2), rect(0, 0, 2560, 1438));
        assert_eq!(
            band(inset(MONITOR, Edge::Bottom, 2), Edge::Bottom, 296),
            rect(0, 1142, 2560, 1438)
        );
    }

    #[test]
    fn a_reserved_bar_asks_explorer_once() {
        assert_eq!(
            on_taskbar_created(BarState::Reserved),
            TaskbarAction::AskOnce
        );
    }

    #[test]
    fn an_unreserved_or_new_bar_registers_from_the_start() {
        assert_eq!(
            on_taskbar_created(BarState::Unreserved),
            TaskbarAction::RegisterAfresh
        );
        assert_eq!(
            on_taskbar_created(BarState::New),
            TaskbarAction::RegisterAfresh
        );
    }

    #[test]
    fn a_released_bar_stays_released() {
        assert_eq!(
            on_taskbar_created(BarState::Released),
            TaskbarAction::Nothing
        );
    }

    #[test]
    fn a_new_monitor_or_edge_registers_afresh_and_the_same_one_does_not() {
        let one = r"\\.\DISPLAY1";
        let two = r"\\.\DISPLAY2";
        assert!(!fresh_registration(None, (one, Edge::Bottom)));
        assert!(!fresh_registration(
            Some((one, Edge::Bottom)),
            (one, Edge::Bottom)
        ));
        assert!(fresh_registration(
            Some((one, Edge::Bottom)),
            (two, Edge::Bottom)
        ));
        assert!(fresh_registration(
            Some((one, Edge::Bottom)),
            (one, Edge::Top)
        ));
    }

    #[test]
    fn edges_map_to_their_abe_values() {
        assert_eq!(abe(Edge::Top), ABE_TOP);
        assert_eq!(abe(Edge::Bottom), ABE_BOTTOM);
    }
}
