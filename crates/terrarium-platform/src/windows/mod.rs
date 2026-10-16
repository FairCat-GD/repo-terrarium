//! Windows: the strip as an `AppBar`. It reserves its edge of one monitor, so maximised windows
//! stop above it. It stays out of the taskbar and Alt-Tab, is never activated by a click, stays
//! topmost, and gives the edge back on every exit path it can see.
//!
//! - `subclass`: the window procedure that sees sent messages before winit does.
//! - `exit`: the registered bar, and the handlers that remove it however the process ends.
//! - `facts`: what the strip reads of a window to judge whether it is full screen.
//! - `display`: the display configuration's paths, for ids that survive restarts.
//! - The pure arithmetic and decisions are in `crate::appbar`, `crate::fullscreen`,
//!   `crate::display` and `crate::power`, tested on every OS.
//!
//! Every public function is safe to call. The ones that act on the strip window check that
//! they run on its thread.

mod display;
mod exit;
mod facts;
mod subclass;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle};
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, RECT,
};
use windows::Win32::Graphics::Dwm::{DWMWA_EXCLUDED_FROM_PEEK, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW,
    MonitorFromWindow,
};
use windows::Win32::System::Power::{
    HPOWERNOTIFY, RegisterPowerSettingNotification, UnregisterPowerSettingNotification,
};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::Shell::{
    ABM_GETAUTOHIDEBAREX, ABM_GETSTATE, ABM_GETTASKBARPOS, ABM_NEW, ABM_QUERYPOS, ABM_SETPOS,
    ABS_AUTOHIDE, APPBARDATA, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, ChangeWindowMessageFilterEx, DEVICE_NOTIFY_WINDOW_HANDLE,
    DeregisterShellHookWindow, FindWindowExW, GWL_EXSTYLE, GetClassNameW, GetForegroundWindow,
    GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, HWND_BOTTOM, HWND_NOTOPMOST,
    HWND_TOPMOST, IsIconic, IsWindowVisible, MONITORINFOF_PRIMARY, MSG, MSGFLT_ALLOW,
    RegisterShellHookWindow, RegisterWindowMessageW, SET_WINDOW_POS_FLAGS, SW_HIDE,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow,
    SetWindowLongW, SetWindowPos, ShowWindow,
};
use windows::core::{BOOL, PCWSTR, w};
use winit::event_loop::EventLoopBuilder;
use winit::platform::windows::EventLoopBuilderExtWindows;

use crate::SystemEvent;
use crate::appbar::{self, BarState, PixelRect, Registered, TaskbarAction, band};
use crate::display::{StableName, stable_names};
use crate::fullscreen::{self, Tracked};
use crate::geometry::Edge;
use subclass::Seen;

// The pure half keeps its own copies of these numbers, so it is tested on every OS. Each is
// checked against the `windows` crate here, so a wrong one fails the Windows build.
const _: () = {
    use windows::Win32::UI::Shell as shell;
    use windows::Win32::UI::WindowsAndMessaging as wm;
    assert!(appbar::ABE_TOP == shell::ABE_TOP);
    assert!(appbar::ABE_BOTTOM == shell::ABE_BOTTOM);
    assert!(appbar::ABN_STATECHANGE == shell::ABN_STATECHANGE as usize);
    assert!(appbar::ABN_POSCHANGED == shell::ABN_POSCHANGED as usize);
    assert!(appbar::ABN_FULLSCREENAPP == shell::ABN_FULLSCREENAPP as usize);
    assert!(appbar::ABN_WINDOWARRANGE == shell::ABN_WINDOWARRANGE as usize);
    assert!(appbar::WA_INACTIVE == wm::WA_INACTIVE as usize);
    assert!(appbar::GWL_EXSTYLE == wm::GWL_EXSTYLE.0 as isize);
    assert!(appbar::SPI_SETWORKAREA == wm::SPI_SETWORKAREA.0 as usize);
    assert!(appbar::WS_EX_TOPMOST == wm::WS_EX_TOPMOST.0);
    assert!(appbar::WS_EX_TOOLWINDOW == wm::WS_EX_TOOLWINDOW.0);
    assert!(appbar::WS_EX_APPWINDOW == wm::WS_EX_APPWINDOW.0);
    assert!(appbar::WS_EX_NOACTIVATE == wm::WS_EX_NOACTIVATE.0);
    assert!(appbar::WM_SHOWWINDOW == wm::WM_SHOWWINDOW);
    assert!(appbar::WM_ACTIVATE == wm::WM_ACTIVATE);
    assert!(appbar::WM_SETTINGCHANGE == wm::WM_SETTINGCHANGE);
    assert!(appbar::WM_ACTIVATEAPP == wm::WM_ACTIVATEAPP);
    assert!(appbar::WM_MOUSEACTIVATE == wm::WM_MOUSEACTIVATE);
    assert!(appbar::WM_WINDOWPOSCHANGING == wm::WM_WINDOWPOSCHANGING);
    assert!(appbar::WM_WINDOWPOSCHANGED == wm::WM_WINDOWPOSCHANGED);
    assert!(appbar::WM_STYLECHANGING == wm::WM_STYLECHANGING);
    assert!(appbar::WM_DISPLAYCHANGE == wm::WM_DISPLAYCHANGE);
    assert!(appbar::WM_NCDESTROY == wm::WM_NCDESTROY);
    assert!(appbar::WM_NCACTIVATE == wm::WM_NCACTIVATE);
    assert!(appbar::WM_LBUTTONDOWN == wm::WM_LBUTTONDOWN);
    assert!(appbar::WM_RBUTTONDOWN == wm::WM_RBUTTONDOWN);
    assert!(appbar::WM_POWERBROADCAST == wm::WM_POWERBROADCAST);
    assert!(appbar::WM_DPICHANGED == wm::WM_DPICHANGED);
    assert!(appbar::WM_ENDSESSION == wm::WM_ENDSESSION);
    assert!(appbar::SWP_NOSIZE == wm::SWP_NOSIZE.0);
    assert!(appbar::SWP_NOMOVE == wm::SWP_NOMOVE.0);
    assert!(appbar::PBT_APMSUSPEND == wm::PBT_APMSUSPEND as usize);
    assert!(appbar::PBT_APMRESUMESUSPEND == wm::PBT_APMRESUMESUSPEND as usize);
    assert!(appbar::PBT_APMPOWERSTATUSCHANGE == wm::PBT_APMPOWERSTATUSCHANGE as usize);
    assert!(appbar::PBT_APMRESUMEAUTOMATIC == wm::PBT_APMRESUMEAUTOMATIC as usize);
    assert!(appbar::PBT_POWERSETTINGCHANGE == wm::PBT_POWERSETTINGCHANGE as usize);
    assert!(appbar::WS_BORDER == wm::WS_BORDER.0);
    assert!(appbar::WS_CAPTION == wm::WS_CAPTION.0);
    assert!(appbar::WS_THICKFRAME == wm::WS_THICKFRAME.0);
    assert!(appbar::WS_POPUP == wm::WS_POPUP.0);
    assert!(appbar::WS_EX_TRANSPARENT == wm::WS_EX_TRANSPARENT.0);
    assert!(appbar::WS_EX_LAYERED == wm::WS_EX_LAYERED.0);
    assert!(appbar::HSHELL_WINDOWCREATED == wm::HSHELL_WINDOWCREATED as usize);
    assert!(appbar::HSHELL_WINDOWDESTROYED == wm::HSHELL_WINDOWDESTROYED as usize);
    assert!(appbar::HSHELL_WINDOWACTIVATED == wm::HSHELL_WINDOWACTIVATED as usize);
    assert!(appbar::HSHELL_REDRAW == wm::HSHELL_REDRAW as usize);
    assert!(appbar::HSHELL_MONITORCHANGED == wm::HSHELL_MONITORCHANGED as usize);
    assert!(appbar::HSHELL_HIGHBIT == wm::HSHELL_HIGHBIT as usize);
};

/// Why the strip's window could not be attached. The app words it for people.
#[derive(Debug)]
pub enum PlatformError {
    /// The window belongs to another thread. Subclassing works only from the window's own.
    NotOwnerThread,
    /// The window handle is unavailable.
    Handle(HandleError),
    /// The handle is not a Win32 window.
    NotWin32,
    /// comctl32 refused the subclass.
    Subclass,
}

impl From<HandleError> for PlatformError {
    fn from(err: HandleError) -> Self {
        Self::Handle(err)
    }
}

/// A Win32 structure's size, which the structure states about itself.
pub(crate) fn struct_size<T>() -> u32 {
    u32::try_from(size_of::<T>()).unwrap_or(u32::MAX)
}

/// The strip window as an `AppBar`. It can only exist on the window's thread, and cannot leave
/// it.
pub struct StripWindow {
    hwnd: HWND,
    registered: Registered,
    subclass: subclass::Installed,
    /// Held while this process may own a bar; `None` if it could not be created.
    mutex: Option<HANDLE>,
    /// Another running strip holds the mutex, so a short work area is its live bar, not a stale
    /// one.
    other_instance: bool,
    bar: Cell<BarState>,
    /// The rectangle last asked for and the one Explorer granted, so an unchanged placement
    /// sends nothing.
    last: Cell<Option<(PixelRect, PixelRect)>>,
    /// The monitor (its device name) and edge the bar was registered on.
    registered_on: RefCell<Option<(String, Edge)>>,
    /// The full-screen apps the strip knows of (`crate::fullscreen`).
    tracked: RefCell<Vec<Tracked>>,
    /// The full-screen app the strip stands behind, as it was last put; `None` on top.
    behind: Cell<Option<isize>>,
    /// The display-state subscription, while it lasts.
    power: Cell<Option<HPOWERNOTIFY>>,
    /// Whether the window receives the shell hook's messages.
    shell_hook: Cell<bool>,
}

impl StripWindow {
    /// Takes winit's window and makes it a strip: subclassed, a tool window that never
    /// activates, and covered by the exit handlers. Call it before the window is first shown:
    /// a shown window keeps its taskbar button until it is hidden again.
    pub fn attach(window: &impl HasWindowHandle) -> Result<Self, PlatformError> {
        let hwnd = win32_hwnd(window)?;
        // SAFETY: both calls only read. An unknown window yields thread 0, which is never the
        // current thread.
        let owned_here = unsafe { GetWindowThreadProcessId(hwnd, None) == GetCurrentThreadId() };
        if !owned_here {
            return Err(PlatformError::NotOwnerThread);
        }
        let registered = registered_messages();
        let subclass = subclass::install(hwnd, registered).ok_or(PlatformError::Subclass)?;
        set_tool_window(hwnd);
        exclude_from_peek(hwnd);
        allow_messages(hwnd, registered);
        let (mutex, other_instance) = instance_mutex();
        exit::install_handlers();
        Ok(Self {
            hwnd,
            registered,
            subclass,
            mutex,
            other_instance,
            bar: Cell::new(BarState::New),
            last: Cell::new(None),
            registered_on: RefCell::new(None),
            tracked: RefCell::new(Vec::new()),
            behind: Cell::new(None),
            power: Cell::new(subscribe_display_state(hwnd)),
            shell_hook: Cell::new(hook_shell(hwnd)),
        })
    }

    /// Calls `on_event` on this thread for each system event the window receives, for as long
    /// as the strip window lives.
    pub fn observe(&self, on_event: impl Fn(SystemEvent) + 'static) {
        self.subclass.state.set_observer(Rc::new(on_event));
    }

    /// Reserves `edge` of `monitor` for the strip and moves the window onto exactly the
    /// rectangle Explorer grants, topmost unless it stands behind a full-screen app.
    /// `thickness_for_width` gives the strip's height for a width, since the taskbar or another
    /// bar on a side can narrow the band. An auto-hide taskbar on the same edge keeps its sliver,
    /// so the pointer can still reveal it.
    ///
    /// The first call registers the bar, and so does the first on another monitor or edge,
    /// after giving the old one back. Before registering, it clears a reservation a crashed
    /// strip left on that edge. `display` is the monitor's stable id (its device path,
    /// [`display_names`]), which tells another monitor from the same one renumbered. `None`
    /// when the monitor is gone, or the bar was released.
    pub fn place(
        &self,
        monitor: isize,
        display: &str,
        edge: Edge,
        thickness_for_width: impl Fn(u32) -> u32,
    ) -> Option<Placement> {
        // From the check that the bar is still registered to the move, an exit handler waits.
        let _placing = exit::placing();
        let rects = monitor_rects(monitor)?;
        let thickness = |area: PixelRect| thickness_for_width(area.width().max(0).cast_unsigned());
        let fresh = matches!(self.bar.get(), BarState::Reserved | BarState::Unreserved)
            && appbar::fresh_registration(
                self.registered_on
                    .borrow()
                    .as_ref()
                    .map(|(display, edge)| (display.as_str(), *edge)),
                (display, edge),
            );
        if fresh {
            exit::remove_if_registered(self.hwnd);
            self.bar.set(BarState::New);
            self.last.set(None);
        }
        let inset = appbar::autohide_inset(rects.monitor, edge, autohide_bar(rects.monitor, edge));
        let on_monitor = appbar::inset(rects.monitor, edge, inset);
        let mut stale = None;
        if self.bar.get() == BarState::New {
            stale = Some(self.clear_stale(monitor, rects, edge, thickness(on_monitor)));
            self.bar.set(if self.register(edge) {
                BarState::Reserved
            } else {
                BarState::Unreserved
            });
            *self.registered_on.borrow_mut() = Some((display.to_owned(), edge));
        }
        if self.bar.get() == BarState::Reserved && !exit::is_registered(self.hwnd) {
            // An exit handler gave the edge back; placing now would take it once more.
            self.bar.set(BarState::Released);
        }
        let proposal = band(on_monitor, edge, thickness(on_monitor));
        let (queried, wanted) = match self.bar.get() {
            BarState::Reserved => {
                let queried = self.ask(ABM_QUERYPOS, edge, proposal);
                (Some(queried), band(queried, edge, thickness(queried)))
            }
            BarState::Unreserved => {
                let work = appbar::inset(rects.work, edge, inset);
                (None, band(work, edge, thickness(work)))
            }
            BarState::New | BarState::Released => return None,
        };

        // Asking again for what was granted, with the window still on it, changes nothing, so
        // nothing is sent. That also ends any back-and-forth of `ABN_POSCHANGED` with another
        // bar.
        let window = window_rect(self.hwnd);
        let unchanged = self
            .last
            .get()
            .filter(|&(last_wanted, last_set)| last_wanted == wanted && window == Some(last_set));
        let (set, window) = if let Some((_, set)) = unchanged {
            (set, window)
        } else {
            let set = if self.bar.get() == BarState::Reserved {
                self.ask(ABM_SETPOS, edge, wanted)
            } else {
                wanted
            };
            self.move_to(set);
            let mut window = window_rect(self.hwnd);
            if window != Some(set) {
                // The subclass pins the window to `set`, so a resize winit makes for a monitor
                // with another DPI lands there too; this second move is the net under it.
                self.move_to(set);
                window = window_rect(self.hwnd);
            }
            self.last.set(Some((wanted, set)));
            (set, window)
        };
        Some(Placement {
            monitor: rects.monitor,
            work_before: rects.work,
            reserved: self.bar.get() == BarState::Reserved,
            proposal,
            queried,
            set,
            window,
            work_after: monitor_rects(monitor).map(|after| after.work),
            unchanged: unchanged.is_some(),
            fresh,
            autohide_inset: inset,
            stale,
        })
    }

    /// Explorer started again (`TaskbarCreated`) and forgot every `AppBar`, or a login
    /// started the strip before there was a taskbar to register with. The bar is asked for
    /// again, or registered from the start, and the next placement sends its rectangle again:
    /// call it on the window's thread, then place.
    pub fn taskbar_created(&self) -> TaskbarCreated {
        let _placing = exit::placing();
        let action = appbar::on_taskbar_created(self.bar.get());
        let mut answered = None;
        match action {
            TaskbarAction::AskOnce => {
                exit::mark_registered(self.hwnd);
                let mut data = self.data(Edge::default(), PixelRect::default());
                // SAFETY: `data` is a complete `APPBARDATA` for this window that outlives the
                // call; `ABM_NEW` reads only the window and the callback message.
                answered = Some(unsafe { SHAppBarMessage(ABM_NEW, &raw mut data) } != 0);
            }
            TaskbarAction::RegisterAfresh => {
                self.bar.set(BarState::New);
                *self.registered_on.borrow_mut() = None;
            }
            TaskbarAction::Nothing => {}
        }
        self.last.set(None);
        // A new Explorer may not carry the shell hook over.
        unhook_shell(self.hwnd, self.shell_hook.get());
        self.shell_hook.set(hook_shell(self.hwnd));
        TaskbarCreated {
            action,
            answered,
            shell_hook: self.shell_hook.get(),
        }
    }

    /// Looks at the full-screen apps (`crate::fullscreen`), given the strip's monitor and every
    /// monitor's rectangle, and puts the strip behind the one that has its monitor, or back on
    /// top when none does. Only a change of that moves the strip.
    pub fn check_fullscreen(
        &self,
        strip_monitor: PixelRect,
        monitors: &[PixelRect],
    ) -> FullscreenCheck {
        let front = facts::foreground().and_then(facts::of);
        let tracked = self.tracked.borrow().clone();
        let now: Vec<_> = tracked
            .iter()
            .map(|app| (app.handle, facts::of(hwnd_of(app.handle))))
            .collect();
        let next = fullscreen::update(&tracked, front.as_ref(), &now, monitors);
        let wanted = fullscreen::yields(&next, strip_monitor);
        let restack = fullscreen::restack(self.behind.get(), wanted);
        let check = FullscreenCheck {
            yielded: wanted.is_some(),
            changed: restack.is_some(),
            apps: next.len(),
            front_class: front.map(|facts| facts.class),
        };
        *self.tracked.borrow_mut() = next;
        match restack {
            Some(Some(app)) => self.step_behind(app),
            Some(None) => self.set_topmost(),
            None => {}
        }
        check
    }

    /// Puts the strip right behind `handle`, a full-screen window: no longer topmost, so the app
    /// and anything over it cover the strip, and the strip is where it was once the app goes.
    fn step_behind(&self, handle: isize) {
        self.behind.set(Some(handle));
        let app = hwnd_of(handle);
        let order = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        // SAFETY: changes only the z-order of a window this thread owns.
        let _ = unsafe { SetWindowPos(self.hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, order) };
        // SAFETY: as above, behind a window that may be gone, which makes the call fail.
        let behind = unsafe { SetWindowPos(self.hwnd, Some(app), 0, 0, 0, 0, order) };
        if behind.is_err() {
            // SAFETY: as above.
            let _ = unsafe { SetWindowPos(self.hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, order) };
        }
    }

    /// Clears a reservation a strip that died without removing its bar left on `edge`, if the
    /// work area shows one. A temporary bar on this still-hidden window reserves and removes the
    /// edge, which makes Explorer recompute the work area, dropping bars whose window is gone.
    fn clear_stale(
        &self,
        monitor: isize,
        rects: MonitorRects,
        edge: Edge,
        thickness: u32,
    ) -> StaleCheck {
        let taskbars = taskbar_rects();
        let suspected = appbar::stale_suspected(rects.monitor, rects.work, &taskbars, edge);
        let mut check = StaleCheck {
            expected: appbar::expected_work_area(rects.monitor, &taskbars),
            taskbars: taskbars.len(),
            suspected,
            other_instance: self.other_instance,
            cleared: false,
            work_after: None,
        };
        if suspected && !self.other_instance && self.register(edge) {
            let queried = self.ask(ABM_QUERYPOS, edge, band(rects.monitor, edge, thickness));
            self.ask(ABM_SETPOS, edge, band(queried, edge, thickness));
            exit::remove_if_registered(self.hwnd);
            check.cleared = true;
            check.work_after = monitor_rects(monitor).map(|after| after.work);
        }
        check
    }

    /// Registers the bar. FALSE can mean this window is still registered (a removal that did not
    /// take), so the bar is removed and asked for once more before giving up. The exit handlers
    /// know the window before each request, so an exit in between still removes it.
    fn register(&self, edge: Edge) -> bool {
        for _ in 0..2 {
            exit::mark_registered(self.hwnd);
            let mut data = self.data(edge, PixelRect::default());
            // SAFETY: `data` is a complete `APPBARDATA` for this window that outlives the call.
            if unsafe { SHAppBarMessage(ABM_NEW, &raw mut data) } != 0 {
                return true;
            }
            exit::remove_if_registered(self.hwnd);
        }
        false
    }

    /// Sends `message` for the bar with `rect`, and returns the rectangle Explorer answers with.
    fn ask(&self, message: u32, edge: Edge, rect: PixelRect) -> PixelRect {
        let mut data = self.data(edge, rect);
        // SAFETY: `data` is a complete `APPBARDATA` for this window that outlives the call;
        // Explorer writes only its rectangle.
        unsafe { SHAppBarMessage(message, &raw mut data) };
        from_rect(data.rc)
    }

    fn data(&self, edge: Edge, rect: PixelRect) -> APPBARDATA {
        APPBARDATA {
            cbSize: struct_size::<APPBARDATA>(),
            hWnd: self.hwnd,
            uCallbackMessage: self.registered.appbar,
            uEdge: appbar::abe(edge),
            rc: to_rect(rect),
            lParam: LPARAM(0),
        }
    }

    /// Moves the window onto `rect`, topmost, or keeping its place in the order while it stands
    /// behind a full-screen app. The subclass is pinned to `rect` first, so this move passes it
    /// and any other is held to it.
    fn move_to(&self, rect: PixelRect) {
        self.subclass.state.pin(Some(rect));
        let (after, flags): (Option<HWND>, SET_WINDOW_POS_FLAGS) = if self.behind.get().is_some() {
            (None, SWP_NOACTIVATE | SWP_NOZORDER)
        } else {
            (Some(HWND_TOPMOST), SWP_NOACTIVATE)
        };
        // SAFETY: moves a window this thread owns, without activating it or changing whether it
        // is shown.
        let _ = unsafe {
            SetWindowPos(
                self.hwnd,
                after,
                rect.left,
                rect.top,
                rect.width(),
                rect.height(),
                flags,
            )
        };
    }

    /// Puts the strip back above ordinary windows, without activating it, even over a
    /// full-screen app until the next look at them. Placement already does this; it is only for
    /// the launch and the raise hotkey, since doing it more often could cover the taskbar's
    /// flyouts, which open over the strip.
    pub fn set_topmost(&self) {
        self.behind.set(None);
        // SAFETY: changes only the z-order of a window this thread owns.
        let _ = unsafe {
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
    }

    /// Gives the edge back and hides the strip. The app calls it as the event loop exits:
    /// winit only posts its window's destruction, which is never handled once the loop has
    /// ended, so no `WM_NCDESTROY` arrives then.
    pub fn release(&self) {
        exit::remove_if_registered(self.hwnd);
        self.bar.set(BarState::Released);
        self.subclass.state.pin(None);
        unhook_shell(self.hwnd, self.shell_hook.replace(false));
        unsubscribe(self.power.take());
        // SAFETY: hides a window this thread owns.
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    /// The work area of the monitor the strip is on, as it is now.
    pub fn work_area(&self) -> Option<PixelRect> {
        // SAFETY: looks up the monitor nearest a window; any window value is accepted.
        let monitor = unsafe { MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST) };
        monitor_info(monitor).map(|(rects, _)| rects.work)
    }

    /// The window's style as Windows reports it, to show whether winit changed it.
    pub fn style(&self) -> StyleFlags {
        // SAFETY: both calls only read the style of a window this thread owns.
        let (ex, visible) = unsafe {
            (
                GetWindowLongW(self.hwnd, GWL_EXSTYLE).cast_unsigned(),
                IsWindowVisible(self.hwnd).as_bool(),
            )
        };
        // SAFETY: only reads whether a window this thread owns is minimised.
        let iconic = unsafe { IsIconic(self.hwnd) }.as_bool();
        StyleFlags {
            tool_window: ex & appbar::WS_EX_TOOLWINDOW != 0,
            no_activate: ex & appbar::WS_EX_NOACTIVATE != 0,
            app_window: ex & appbar::WS_EX_APPWINDOW != 0,
            topmost: ex & appbar::WS_EX_TOPMOST != 0,
            visible,
            iconic,
            yielded: self.behind.get().is_some(),
        }
    }
}

impl Drop for StripWindow {
    fn drop(&mut self) {
        unhook_shell(self.hwnd, self.shell_hook.replace(false));
        unsubscribe(self.power.take());
        self.subclass.remove(self.hwnd);
        exit::remove_if_registered(self.hwnd);
        if let Some(mutex) = self.mutex.take() {
            // SAFETY: the handle `CreateMutexW` returned, closed once.
            let _ = unsafe { CloseHandle(mutex) };
        }
    }
}

/// What one placement did, for the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub monitor: PixelRect,
    /// The monitor's work area before this placement.
    pub work_before: PixelRect,
    /// Whether the edge is reserved, or the strip is the unreserved fallback.
    pub reserved: bool,
    /// The band proposed to `ABM_QUERYPOS`.
    pub proposal: PixelRect,
    /// What `ABM_QUERYPOS` answered; `None` for the fallback.
    pub queried: Option<PixelRect>,
    /// The rectangle `ABM_SETPOS` granted, which the window was moved to.
    pub set: PixelRect,
    /// The window's rectangle afterwards.
    pub window: Option<PixelRect>,
    /// The work area afterwards. Explorer may still change it after answering, so the probe
    /// also records work-area changes as they arrive.
    pub work_after: Option<PixelRect>,
    /// Nothing changed, so nothing was sent.
    pub unchanged: bool,
    /// The bar moved to another monitor or edge, and was registered there afresh.
    pub fresh: bool,
    /// How far the band stays off the edge for an auto-hide taskbar there.
    pub autohide_inset: i32,
    /// The stale-reservation check, on the placement that registered the bar.
    pub stale: Option<StaleCheck>,
}

/// What a `TaskbarCreated` did, for the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskbarCreated {
    pub action: TaskbarAction,
    /// What `ABM_NEW` answered, when the bar was asked for once: FALSE when Explorer still
    /// knew it, or could not register it.
    pub answered: Option<bool>,
    /// Whether the shell hook was taken again.
    pub shell_hook: bool,
}

/// What a look at the full-screen apps found, for the probe. It names a window's class, never
/// its title, which can name a person's documents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullscreenCheck {
    /// The strip stands behind a full-screen app on its monitor.
    pub yielded: bool,
    /// This look moved it there, or back.
    pub changed: bool,
    /// How many full-screen apps the strip knows of.
    pub apps: usize,
    /// The class of the window in front.
    pub front_class: Option<String>,
}

/// The taskbar's edge and whether it hides itself, for the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskbarState {
    /// `ABE_LEFT` 0, `ABE_TOP` 1, `ABE_RIGHT` 2, `ABE_BOTTOM` 3.
    pub edge: u32,
    pub rect: PixelRect,
    pub auto_hide: bool,
}

/// The check for a reservation a crashed strip left behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleCheck {
    /// The work area the taskbars alone explain.
    pub expected: PixelRect,
    pub taskbars: usize,
    pub suspected: bool,
    /// Another strip was running, so nothing was cleared.
    pub other_instance: bool,
    pub cleared: bool,
    pub work_after: Option<PixelRect>,
}

/// The extended style bits the strip depends on, and whether it is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent flags, as Windows reports them"
)]
pub struct StyleFlags {
    pub tool_window: bool,
    pub no_activate: bool,
    pub app_window: bool,
    pub topmost: bool,
    pub visible: bool,
    /// Minimised: a strip that should never be.
    pub iconic: bool,
    /// It stands behind a full-screen app.
    pub yielded: bool,
}

/// A monitor's rectangles, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorRects {
    /// `rcMonitor`: the whole monitor, never the primary's size (`SM_CXSCREEN`).
    pub monitor: PixelRect,
    /// `rcWork`: the monitor minus the taskbar and every `AppBar`.
    pub work: PixelRect,
    pub primary: bool,
}

/// The rectangles of the monitor with this `HMONITOR`, as winit's `MonitorHandle::hmonitor`
/// gives it. `None` for a monitor that is gone.
pub fn monitor_rects(monitor: isize) -> Option<MonitorRects> {
    monitor_described(monitor).map(|(rects, _)| rects)
}

/// The monitor's rectangles and its device name (`\\.\DISPLAY1`), read in one call. `None` for
/// a monitor that is gone. winit's own `MonitorHandle::size` and `name` panic then, so the
/// strip reads monitors through this.
pub fn monitor_described(monitor: isize) -> Option<(MonitorRects, String)> {
    monitor_info(HMONITOR(std::ptr::without_provenance_mut(
        monitor.cast_unsigned(),
    )))
}

fn monitor_info(monitor: HMONITOR) -> Option<(MonitorRects, String)> {
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: struct_size::<MONITORINFOEXW>(),
            ..MONITORINFO::default()
        },
        ..MONITORINFOEXW::default()
    };
    // SAFETY: `info` is a `MONITORINFOEXW` whose size field says so, which the call accepts in
    // place of a `MONITORINFO`, and it outlives the call; an unknown or stale monitor handle
    // makes the call fail without writing.
    let found = unsafe { GetMonitorInfoW(monitor, (&raw mut info).cast()) }.as_bool();
    if !found {
        return None;
    }
    let device = &info.szDevice;
    let len = device.iter().position(|&c| c == 0).unwrap_or(device.len());
    let rects = MonitorRects {
        monitor: from_rect(info.monitorInfo.rcMonitor),
        work: from_rect(info.monitorInfo.rcWork),
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
    };
    Some((rects, String::from_utf16_lossy(&device[..len])))
}

/// Each display's stable name by its GDI name (`crate::display`), read from the display
/// configuration now.
pub fn display_names() -> std::collections::BTreeMap<String, StableName> {
    stable_names(&display::paths())
}

/// The auto-hide taskbar on `edge` of `monitor`, if there is one: its window's rectangle.
fn autohide_bar(monitor: PixelRect, edge: Edge) -> Option<PixelRect> {
    let mut data = APPBARDATA {
        cbSize: struct_size::<APPBARDATA>(),
        uEdge: appbar::abe(edge),
        rc: to_rect(monitor),
        ..APPBARDATA::default()
    };
    // SAFETY: `data` is a complete `APPBARDATA` naming an edge and a monitor's rectangle, which
    // outlives the call; the answer is a window handle, or zero for none.
    let found = unsafe { SHAppBarMessage(ABM_GETAUTOHIDEBAREX, &raw mut data) };
    (found != 0)
        .then(|| hwnd_of(found.cast_signed()))
        .and_then(window_rect)
}

/// The taskbar's edge, rectangle and auto-hide setting, as Explorer reports them.
pub fn taskbar_state() -> Option<TaskbarState> {
    let mut data = APPBARDATA {
        cbSize: struct_size::<APPBARDATA>(),
        ..APPBARDATA::default()
    };
    // SAFETY: `data` is a complete `APPBARDATA` that outlives the call; Explorer writes the
    // taskbar's edge and rectangle into it.
    let found = unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &raw mut data) } != 0;
    if !found {
        return None;
    }
    let mut state = APPBARDATA {
        cbSize: struct_size::<APPBARDATA>(),
        ..APPBARDATA::default()
    };
    // SAFETY: as above; `ABM_GETSTATE` reads only the size and answers in its return value.
    let flags = unsafe { SHAppBarMessage(ABM_GETSTATE, &raw mut state) };
    Some(TaskbarState {
        edge: data.uEdge,
        rect: from_rect(data.rc),
        auto_hide: u32::try_from(flags).is_ok_and(|flags| flags & ABS_AUTOHIDE != 0),
    })
}

/// Keeps the strip visible while Aero Peek shows the desktop, as the taskbar does.
fn exclude_from_peek(hwnd: HWND) {
    let excluded = BOOL::from(true);
    // SAFETY: the attribute takes a `BOOL`, whose address and size are given, and which
    // outlives the call.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_EXCLUDED_FROM_PEEK,
            (&raw const excluded).cast(),
            struct_size::<BOOL>(),
        )
    };
}

/// Asks for the shell hook's messages: which window is activated or gone, so the strip knows
/// when to look at the full-screen apps. Whether Windows took the window.
fn hook_shell(hwnd: HWND) -> bool {
    // SAFETY: registers a window this thread owns; it is deregistered before it goes.
    unsafe { RegisterShellHookWindow(hwnd) }.as_bool()
}

fn unhook_shell(hwnd: HWND, hooked: bool) {
    if hooked {
        // SAFETY: deregisters a window this thread registered.
        let _ = unsafe { DeregisterShellHookWindow(hwnd) };
    }
}

/// Subscribes the window to the console display's state, so the strip hears the screens come
/// back on, also where the machine never sleeps (Modern Standby).
fn subscribe_display_state(hwnd: HWND) -> Option<HPOWERNOTIFY> {
    let setting = GUID_CONSOLE_DISPLAY_STATE;
    // SAFETY: the GUID outlives the call, and the recipient is a window this thread owns; the
    // subscription is ended before the window goes.
    unsafe {
        RegisterPowerSettingNotification(
            HANDLE(hwnd.0),
            &raw const setting,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    }
    .ok()
}

fn unsubscribe(power: Option<HPOWERNOTIFY>) {
    if let Some(power) = power {
        // SAFETY: ends a subscription this process made, once: the handle was taken out.
        let _ = unsafe { UnregisterPowerSettingNotification(power) };
    }
}

/// Lets the running app bring its window to the front when this launch asks it to: Windows
/// lets only the process in front do that, and this launch, started by the person, is.
pub fn allow_any_foreground() {
    // SAFETY: passes this process's right to bring a window forward to any process; no
    // structure is passed.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
}

/// The window rectangles of the taskbars: the primary one and one per other display.
fn taskbar_rects() -> Vec<PixelRect> {
    let mut rects = Vec::new();
    for class in [w!("Shell_TrayWnd"), w!("Shell_SecondaryTrayWnd")] {
        let mut after = None;
        // Bounded, so a list that changes during the walk cannot keep it going.
        for _ in 0..16 {
            // SAFETY: searches top-level windows by class name; `after` is `None` or a window
            // this walk returned.
            let Ok(found) = (unsafe { FindWindowExW(None, after, class, PCWSTR::null()) }) else {
                break;
            };
            rects.extend(window_rect(found));
            after = Some(found);
        }
    }
    rects
}

/// A window handle as the number it is.
pub(super) fn handle_of(hwnd: HWND) -> isize {
    hwnd.0.addr().cast_signed()
}

pub(super) fn hwnd_of(handle: isize) -> HWND {
    HWND(std::ptr::without_provenance_mut(handle.cast_unsigned()))
}

fn window_rect(hwnd: HWND) -> Option<PixelRect> {
    let mut rect = RECT::default();
    // SAFETY: `rect` outlives the call; an unknown window makes it fail.
    unsafe { GetWindowRect(hwnd, &raw mut rect) }
        .ok()
        .map(|()| from_rect(rect))
}

fn from_rect(rect: RECT) -> PixelRect {
    PixelRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

fn to_rect(rect: PixelRect) -> RECT {
    RECT {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

fn win32_hwnd(window: &impl HasWindowHandle) -> Result<HWND, PlatformError> {
    let RawWindowHandle::Win32(handle) = window.window_handle()?.as_raw() else {
        return Err(PlatformError::NotWin32);
    };
    Ok(HWND(std::ptr::without_provenance_mut(
        handle.hwnd.get().cast_unsigned(),
    )))
}

/// Out of the taskbar and Alt-Tab, and never activated by a click. The subclass is already in
/// place, so this write, like every later one winit makes, passes through its guard.
fn set_tool_window(hwnd: HWND) {
    // SAFETY: reads and writes the extended style of a window this thread owns, then applies it
    // with a frame change that neither moves, sizes, orders nor activates the window.
    unsafe {
        let old = GetWindowLongW(hwnd, GWL_EXSTYLE).cast_unsigned();
        SetWindowLongW(
            hwnd,
            GWL_EXSTYLE,
            appbar::guard_ex_style(old, old).cast_signed(),
        );
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Lets the `AppBar` callback, `TaskbarCreated` and the shell hook through the message filter
/// an elevated process has, which would otherwise drop Explorer's messages.
fn allow_messages(hwnd: HWND, registered: Registered) {
    for message in [
        registered.appbar,
        registered.taskbar_created,
        registered.shellhook,
    ] {
        if message != 0 {
            // SAFETY: changes the message filter of a window this thread owns; no structure is
            // passed.
            let _ = unsafe { ChangeWindowMessageFilterEx(hwnd, message, MSGFLT_ALLOW, None) };
        }
    }
}

/// Opens the named mutex a strip holds while it may own a bar, and whether another process
/// already held it.
fn instance_mutex() -> (Option<HANDLE>, bool) {
    // SAFETY: creates or opens a named mutex with default security and no initial owner. The
    // handle is closed when the strip window is dropped.
    let mutex = unsafe {
        CreateMutexW(
            None,
            false,
            w!("Local\\io.github.faircat-gd.repo-terrarium.appbar"),
        )
    }
    .ok();
    // SAFETY: reads this thread's last error, which `CreateMutexW` has just set.
    let existed = mutex.is_some() && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    (mutex, existed)
}

/// The message ids registered for this session: the bar's callback, `TaskbarCreated` and the
/// shell hook's.
fn registered_messages() -> Registered {
    static REGISTERED: OnceLock<Registered> = OnceLock::new();
    *REGISTERED.get_or_init(|| {
        // SAFETY: registers message names from static strings; a failure returns 0, which is
        // never matched.
        unsafe {
            Registered {
                appbar: RegisterWindowMessageW(w!("io.github.faircat-gd.repo-terrarium.AppBar")),
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                shellhook: RegisterWindowMessageW(w!("SHELLHOOK")),
            }
        }
    })
}

/// Counts watched messages from here on, both in the subclass and in winit's message hook,
/// which sees only posted messages. For the probe; set on the event loop before it runs.
pub fn watch_messages<T: 'static>(builder: &mut EventLoopBuilder<T>) {
    subclass::start_counting();
    let registered = registered_messages();
    builder.with_msg_hook(move |msg| {
        // SAFETY: winit passes the `MSG` it has just taken from the queue, valid for this call.
        if let Some(msg) = unsafe { msg.cast::<MSG>().as_ref() }
            && let Some(name) = appbar::message_name(msg.message, msg.wParam.0, registered)
        {
            subclass::count(name, Seen::Hook);
        }
        false
    });
}

/// How often one watched message reached the subclass and winit's message hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageCount {
    pub name: &'static str,
    pub subclass: u64,
    pub hook: u64,
}

/// The counts since the last call. Empty unless `watch_messages` ran.
pub fn take_message_counts() -> Vec<MessageCount> {
    subclass::take_counts()
}

/// Whether the window the user is working in belongs to this process.
pub fn foreground_is_this_process() -> bool {
    let mut process = 0;
    // SAFETY: all three only read; with no foreground window the process id stays 0.
    unsafe {
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&raw mut process));
        process != 0 && process == GetCurrentProcessId()
    }
}

/// The window class of the window the user is working in. The class, not the title: a title
/// can name a person's documents, and the probe report is sent to someone else.
pub fn foreground_class() -> Option<String> {
    // SAFETY: only reads which window is in front; none gives a null handle.
    class_name(unsafe { GetForegroundWindow() })
}

/// A window's class.
fn class_name(hwnd: HWND) -> Option<String> {
    let mut name = [0_u16; 256];
    // SAFETY: the buffer is writable for its whole length, which the wrapper passes as the
    // limit; an unknown or null window makes the call return 0.
    let len = unsafe { GetClassNameW(hwnd, &mut name) };
    let len = usize::try_from(len).ok().filter(|&len| len > 0)?;
    name.get(..len).map(String::from_utf16_lossy)
}

/// The Windows version, read from the kernel, which does not shim it for old apps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OsVersion {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
}

pub fn os_version() -> Option<OsVersion> {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: struct_size::<OSVERSIONINFOW>(),
        ..OSVERSIONINFOW::default()
    };
    // SAFETY: `info` states its size and outlives the call.
    let status = unsafe { RtlGetVersion(&raw mut info) };
    status.is_ok().then_some(OsVersion {
        major: info.dwMajorVersion,
        minor: info.dwMinorVersion,
        build: info.dwBuildNumber,
    })
}

/// The window that was in front before the strip's menu, so it can have the keyboard back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuShown {
    strip: isize,
    before: isize,
}

/// Pops up `menu` at the pointer over `window`. Call it from the thread that owns the window
/// (the event loop's); it returns when the menu closes. muda brings the strip to the front for
/// its menu, which a click on the strip lets it do; what it returns lets the caller give the
/// front back ([`give_front_back`]).
pub fn show_context_menu(menu: &muda::Menu, window: &impl HasWindowHandle) -> Option<MenuShown> {
    use muda::ContextMenu;
    let hwnd = win32_hwnd(window).ok()?;
    // SAFETY: only reads which window is in front; none gives a null handle.
    let before = unsafe { GetForegroundWindow() };
    // SAFETY: `window` is borrowed for the whole call, so the window it names outlives it, and
    // this runs on the thread that owns it.
    unsafe {
        menu.show_context_menu_for_hwnd(handle_of(hwnd), None);
    }
    Some(MenuShown {
        strip: handle_of(hwnd),
        before: handle_of(before),
    })
}

/// Gives the front back to the window that had it before the menu, if the strip still holds
/// it: the app the person was in keeps the keyboard. The caller skips it when an item was
/// chosen, so a window the choice opens can come to the front.
pub fn give_front_back(shown: MenuShown) {
    if shown.before == 0 || shown.before == shown.strip {
        return;
    }
    // SAFETY: only reads which window is in front.
    let front = handle_of(unsafe { GetForegroundWindow() });
    if front == shown.strip {
        // SAFETY: brings a window to the front; this process is in front, so Windows allows
        // it, and a window gone since makes the call fail.
        let _ = unsafe { SetForegroundWindow(hwnd_of(shown.before)) };
    }
}
