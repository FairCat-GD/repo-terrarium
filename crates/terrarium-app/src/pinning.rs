//! The OS half of the strip: its window attributes, level, placement, menu and system events.
//! On macOS it goes through `terrarium-platform::macos`; on Windows the strip is an `AppBar`
//! through `terrarium-platform::windows`. They are v1's only targets.

use serde::Serialize;

pub use imp::{
    AttachError, Monitor, Observers, Pinning, PreviousApp, ReopenHook, activate,
    allow_foreground_for_running, app_is_active, configure_event_loop, frontmost_app, on_reopen,
    os_version, separate_spaces, taskbar, window_attributes,
};

#[cfg(not(any(target_os = "macos", windows)))]
compile_error!("repo-terrarium runs on macOS and Windows");

/// Where the strip sits in the window stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LevelMode {
    /// Just above the desktop icons.
    #[cfg(target_os = "macos")]
    DesktopIconPlusOne,
    /// winit's `AlwaysOnBottom`: just below ordinary windows. The macOS checklist compared it
    /// with desktop-icon + 1.
    #[cfg(target_os = "macos")]
    AlwaysOnBottom,
    /// The `AppBar`: topmost, on its reserved edge.
    #[cfg(windows)]
    TopmostAppBar,
}

/// The level the strip runs at. On macOS the pinning checklist chose it.
#[cfg(target_os = "macos")]
pub const DEFAULT_LEVEL: LevelMode = LevelMode::DesktopIconPlusOne;
#[cfg(windows)]
pub const DEFAULT_LEVEL: LevelMode = LevelMode::TopmostAppBar;

/// Whether the OS reserves the strip's edge (the Windows `AppBar`). There the rectangle the OS
/// grants, not the monitor's width, sets the strip's size, and a scale-factor change keeps it.
pub const EDGE_RESERVED: bool = cfg!(windows);

/// The frame the strip was given: points in `AppKit` coordinates on macOS, physical pixels on
/// Windows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Placed {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub unit: &'static str,
    /// What the `AppBar` protocol did (Windows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appbar: Option<AppBarPlaced>,
}

// The Windows report types are compiled everywhere, so their serialisation is tested on macOS;
// only Windows constructs them outside tests.

/// A rectangle in physical pixels, y down, right and bottom exclusive (Win32 `RECT`).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// One `AppBar` placement: the rectangles asked for and granted, and the work area around it.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts of one placement"
)]
pub struct AppBarPlaced {
    pub monitor: PxRect,
    pub work_before: PxRect,
    /// False when the edge could not be reserved and the strip is a plain window in the work
    /// area.
    pub reserved: bool,
    pub proposal: PxRect,
    pub queried: Option<PxRect>,
    pub set: PxRect,
    pub window: Option<PxRect>,
    pub work_after: Option<PxRect>,
    /// Nothing had changed, so nothing was sent.
    pub unchanged: bool,
    /// The granted band is not the strip's size: its height is not the scaled image's, or it
    /// is narrower than the 1× image. The surface and the window then differ.
    pub size_mismatch: bool,
    /// The bar moved to another monitor or edge and was registered there afresh.
    pub fresh: bool,
    /// How many pixels the band stays off the edge for an auto-hide taskbar there.
    pub autohide_inset: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale: Option<StaleReport>,
}

/// What a `TaskbarCreated` did to the bar (Windows).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskbarReport {
    pub action: TaskbarStep,
    /// What `ABM_NEW` answered when the bar was asked for once.
    pub answered: Option<bool>,
    pub shell_hook: bool,
}

/// How the bar was asked for again.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskbarStep {
    /// Asked for once: Explorer had forgotten it, or answered that it still knew it.
    AskOnce,
    /// Registered from the start, the stale check included.
    RegisterAfresh,
    /// Already given back.
    Nothing,
}

/// A look at the full-screen apps that moved the strip behind one, or back (Windows).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FullscreenReport {
    pub yielded: bool,
    pub apps: usize,
    /// The class of the window in front, never its title.
    pub front_class: Option<String>,
}

/// The taskbar as Explorer reports it, for the probe's header (Windows).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskbarState {
    /// Its `ABE_` value: 0 left, 1 top, 2 right, 3 bottom.
    pub edge: u32,
    pub rect: PxRect,
    pub auto_hide: bool,
}

/// The startup check for a reservation a crashed strip left on the edge.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StaleReport {
    pub expected: PxRect,
    pub taskbars: usize,
    pub suspected: bool,
    pub other_instance: bool,
    pub cleared: bool,
    pub work_after: Option<PxRect>,
}

/// What the OS says about the strip when a probe step closes (Windows).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent flags, as Windows reports them"
)]
pub struct OsSnapshot {
    pub work_area: Option<PxRect>,
    pub tool_window: bool,
    pub no_activate: bool,
    pub app_window: bool,
    pub topmost: bool,
    pub visible: bool,
    /// Minimised, which the strip should never be.
    pub iconic: bool,
    /// Standing behind a full-screen app.
    pub yielded: bool,
    /// How often each watched message reached the subclass and winit's message hook since the
    /// last snapshot.
    pub messages: Vec<MessageCount>,
}

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageCount {
    pub name: &'static str,
    pub subclass: u64,
    pub hook: u64,
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::unused_self,
    clippy::unnecessary_wraps,
    reason = "both implementations keep one signature, so each has parameters and `Option`s \
              only the other needs"
)]
mod imp {
    use terrarium_compose::scale::{SizeStep, StripScale};
    use terrarium_platform::SystemEvent;
    use terrarium_platform::geometry::Edge;
    use terrarium_platform::macos::{self, Screen, StripWindow};
    use winit::event_loop::EventLoopBuilder;
    use winit::platform::macos::{
        ActivationPolicy, EventLoopBuilderExtMacOS, WindowAttributesExtMacOS,
    };
    use winit::window::{Window, WindowAttributes, WindowLevel};

    use super::{FullscreenReport, LevelMode, OsSnapshot, Placed, TaskbarReport, TaskbarState};
    use crate::copy;
    use crate::monitor::{MonitorId, MonitorInfo};

    pub use macos::{Observers, PlatformError as AttachError};

    /// An accessory app: no Dock icon, no Cmd-Tab entry, no menu bar, and a launch that asks
    /// not to take focus from the app the user is in.
    pub fn configure_event_loop<T: 'static>(builder: &mut EventLoopBuilder<T>, _probing: bool) {
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_activate_ignoring_other_apps(false)
            .with_default_menu(false);
    }

    pub fn window_attributes() -> WindowAttributes {
        WindowAttributes::default().with_has_shadow(false)
    }

    pub struct Monitor {
        pub info: MonitorInfo,
        screen: Screen,
    }

    impl AsRef<MonitorInfo> for Monitor {
        fn as_ref(&self) -> &MonitorInfo {
            &self.info
        }
    }

    pub struct Pinning {
        strip: StripWindow,
    }

    impl Pinning {
        pub fn attach(window: &Window) -> Result<Self, AttachError> {
            Ok(Self {
                strip: StripWindow::attach(window)?,
            })
        }

        pub fn monitors(&self, _window: &Window) -> Vec<Monitor> {
            macos::screens()
                .into_iter()
                .map(|screen| Monitor {
                    info: MonitorInfo {
                        id: MonitorId(
                            screen
                                .uuid
                                .clone()
                                .unwrap_or_else(|| copy::display_id(screen.display_id)),
                        ),
                        name: screen.name.clone(),
                        physical_width: screen.physical_width(),
                        physical_height: screen.physical_height(),
                        scale: screen.scale,
                        primary: screen.primary,
                        detail: copy::display_detail(
                            screen.display_id,
                            &screen.frame,
                            &screen.visible_frame,
                        ),
                    },
                    screen,
                })
                .collect()
        }

        /// The strip fills the monitor's width at the largest scale that fits.
        pub fn place(
            &self,
            _window: &Window,
            monitor: &Monitor,
            step: SizeStep,
            edge: Edge,
        ) -> Option<(StripScale, Placed)> {
            let scale = StripScale::for_physical_width(monitor.info.physical_width, step);
            let frame = self
                .strip
                .place(&monitor.screen, edge, scale.width, scale.height);
            let placed = Placed {
                x: frame.x,
                y: frame.y,
                width: frame.width,
                height: frame.height,
                unit: copy::PLACED_UNIT,
                appbar: None,
            };
            Some((scale, placed))
        }

        pub fn apply_level(&self, window: &Window, mode: LevelMode) {
            match mode {
                LevelMode::DesktopIconPlusOne => self.strip.set_desktop_level(),
                LevelMode::AlwaysOnBottom => window.set_window_level(WindowLevel::AlwaysOnBottom),
            }
        }

        pub fn raise(&self, _window: &Window) {
            self.strip.raise();
        }

        pub fn show(&self, _window: &Window) {
            self.strip.show();
        }

        /// The window level read back from the OS.
        pub fn level(&self) -> Option<isize> {
            Some(self.strip.level())
        }

        pub fn show_menu(&self, _window: &Window, menu: &muda::Menu) {
            self.strip.show_context_menu(menu);
        }

        pub fn observe(&self, on_event: impl Fn(SystemEvent) + Send + Sync + 'static) -> Observers {
            macos::observe(on_event)
        }

        pub fn snapshot(&self) -> Option<OsSnapshot> {
            None
        }

        /// Nothing is reserved on macOS, so there is nothing to give back.
        pub fn release(&self) {}

        /// Explorer's restart is Windows' alone.
        pub fn taskbar_created(&self) -> Option<TaskbarReport> {
            None
        }

        /// The desktop-level strip is under every app already, full screen or not.
        pub fn check_fullscreen(&self, _window: &Window) -> Option<FullscreenReport> {
            None
        }
    }

    /// A second launch of the installed app sends the running one a reopen event instead.
    pub fn allow_foreground_for_running() {}

    /// macOS has no taskbar.
    pub fn taskbar() -> Option<TaskbarState> {
        None
    }

    pub fn app_is_active() -> Option<bool> {
        Some(macos::app_is_active())
    }

    /// Gives `window` the keyboard, in front of every other app's windows. An accessory app is
    /// not activated by a new window, so it is activated here.
    pub fn activate(window: &Window) {
        macos::bring_forward(window);
    }

    /// The app the user was in, to give the keyboard back to when the last window closes.
    pub use macos::PreviousApp;

    /// Opening the installed app again while it runs: macOS sends it a reopen event rather
    /// than starting another.
    pub use macos::ReopenHook;

    pub fn on_reopen(on_reopen: impl Fn() + 'static) -> Option<ReopenHook> {
        macos::on_reopen(on_reopen)
    }

    pub fn frontmost_app() -> Option<String> {
        macos::frontmost_app()
    }

    pub fn os_version() -> String {
        copy::os_version(&macos::os_version())
    }

    pub fn separate_spaces() -> Option<bool> {
        Some(macos::screens_have_separate_spaces())
    }
}

#[cfg(windows)]
#[expect(
    clippy::unused_self,
    clippy::unnecessary_wraps,
    reason = "both implementations keep one signature, so each has parameters and `Option`s \
              only the other needs"
)]
mod imp {
    use terrarium_compose::scale::{SizeStep, StripScale};
    use terrarium_platform::SystemEvent;
    use terrarium_platform::appbar::{PixelRect, TaskbarAction};
    use terrarium_platform::geometry::Edge;
    use terrarium_platform::windows::{self, Placement, StaleCheck, StripWindow};
    use winit::event_loop::EventLoopBuilder;
    use winit::monitor::MonitorHandle;
    use winit::platform::windows::{
        CornerPreference, MonitorHandleExtWindows, WindowAttributesExtWindows,
    };
    use winit::window::{Window, WindowAttributes, WindowButtons};

    use super::{
        AppBarPlaced, FullscreenReport, LevelMode, MessageCount, OsSnapshot, Placed, PxRect,
        StaleReport, TaskbarReport, TaskbarState, TaskbarStep,
    };
    use crate::copy;
    use crate::menu;
    use crate::monitor::{MonitorId, MonitorInfo};

    pub use windows::PlatformError as AttachError;

    /// Counts window messages for the probe, and only then.
    pub fn configure_event_loop<T: 'static>(builder: &mut EventLoopBuilder<T>, probing: bool) {
        if probing {
            windows::watch_messages(builder);
        }
    }

    /// Kept off the taskbar by winit as well as by the tool-window style: winit adds a taskbar
    /// tab itself unless told to skip it, at creation and again whenever Explorer restarts.
    /// Square corners, since the window keeps a caption style Windows 11 would round. No
    /// minimise button: with no taskbar button and no Alt-Tab entry, a strip that Win+M
    /// minimised could never be brought back.
    pub fn window_attributes() -> WindowAttributes {
        WindowAttributes::default()
            .with_skip_taskbar(true)
            .with_corner_preference(CornerPreference::DoNotRound)
            .with_enabled_buttons(WindowButtons::CLOSE)
    }

    pub struct Monitor {
        pub info: MonitorInfo,
        handle: MonitorHandle,
    }

    impl AsRef<MonitorInfo> for Monitor {
        fn as_ref(&self) -> &MonitorInfo {
            &self.info
        }
    }

    /// The subclass holds the event callback for as long as the strip window lives.
    pub struct Observers;

    pub struct Pinning {
        strip: StripWindow,
    }

    impl Pinning {
        pub fn attach(window: &Window) -> Result<Self, AttachError> {
            Ok(Self {
                strip: StripWindow::attach(window)?,
            })
        }

        /// winit's monitors, each read through platform: its own `rcMonitor`, its GDI name
        /// (`\\.\DISPLAY1`), and its monitor's device path and own name from the display
        /// configuration. The device path is the id, since the GDI name is renumbered as
        /// displays come and go. A monitor unplugged while the list is read is left out, where
        /// winit's own `size` and `name` would panic.
        pub fn monitors(&self, window: &Window) -> Vec<Monitor> {
            let names = windows::display_names();
            window
                .available_monitors()
                .enumerate()
                .filter_map(|(index, handle)| {
                    let (rects, gdi) = windows::monitor_described(handle.hmonitor())?;
                    let stable = windows::display_name(&gdi, &names);
                    let size = |length: i32| u32::try_from(length).unwrap_or(0);
                    let detail = copy::monitor_detail(
                        &px(rects.monitor),
                        &px(rects.work),
                        &gdi,
                        stable.from_display_config,
                    );
                    let name = stable
                        .friendly
                        .unwrap_or_else(|| copy::display_fallback_name(&gdi, index));
                    Some(Monitor {
                        info: MonitorInfo {
                            id: MonitorId(if stable.id.is_empty() {
                                copy::monitor_id(index)
                            } else {
                                stable.id
                            }),
                            name,
                            physical_width: size(rects.monitor.width()),
                            physical_height: size(rects.monitor.height()),
                            scale: handle.scale_factor(),
                            primary: rects.primary,
                            detail,
                        },
                        handle,
                    })
                })
                .collect()
        }

        /// Reserves the edge and sizes the strip to the band Windows grants: a side taskbar or
        /// another bar can make it narrower than the monitor, and the scale follows the width.
        pub fn place(
            &self,
            _window: &Window,
            monitor: &Monitor,
            step: SizeStep,
            edge: Edge,
        ) -> Option<(StripScale, Placed)> {
            let placement = self.strip.place(monitor.handle.hmonitor(), edge, |width| {
                StripScale::for_physical_width(width, step).height
            })?;
            // Said once, by the placement that tried to register the bar.
            if placement.stale.is_some() && !placement.reserved {
                eprintln!("{}", copy::edge_not_reserved());
            }
            let set = placement.set;
            let scale = StripScale::for_physical_width(set.width().max(0).cast_unsigned(), step);
            let placed = Placed {
                x: f64::from(set.left),
                y: f64::from(set.top),
                width: f64::from(set.width()),
                height: f64::from(set.height()),
                unit: copy::PLACED_UNIT,
                appbar: Some(appbar_placed(&placement, &scale)),
            };
            Some((scale, placed))
        }

        pub fn apply_level(&self, _window: &Window, mode: LevelMode) {
            match mode {
                LevelMode::TopmostAppBar => self.strip.set_topmost(),
            }
        }

        /// The `AppBar` is already topmost; the hotkey puts it back on top of other topmost
        /// windows.
        pub fn raise(&self, _window: &Window) {
            self.strip.set_topmost();
        }

        /// Through winit, so its own record of the window's visibility stays true. The window
        /// was created inactive, so winit shows it without activating it.
        pub fn show(&self, window: &Window) {
            window.set_visible(true);
        }

        /// Windows has no numeric level; the snapshot reports whether the strip is topmost.
        pub fn level(&self) -> Option<isize> {
            None
        }

        /// The menu brings the strip to the front; dismissed, the app the person was in gets
        /// the front back. A choice keeps it, so a window the choice opens can come forward.
        pub fn show_menu(&self, window: &Window, menu: &muda::Menu) {
            let chosen = menu::choices();
            if let Some(shown) = windows::show_context_menu(menu, window)
                && menu::choices() == chosen
            {
                windows::give_front_back(shown);
            }
        }

        pub fn observe(&self, on_event: impl Fn(SystemEvent) + Send + Sync + 'static) -> Observers {
            self.strip.observe(on_event);
            Observers
        }

        pub fn snapshot(&self) -> Option<OsSnapshot> {
            let style = self.strip.style();
            Some(OsSnapshot {
                work_area: self.strip.work_area().map(px),
                tool_window: style.tool_window,
                no_activate: style.no_activate,
                app_window: style.app_window,
                topmost: style.topmost,
                visible: style.visible,
                iconic: style.iconic,
                yielded: style.yielded,
                messages: windows::take_message_counts()
                    .into_iter()
                    .map(|count| MessageCount {
                        name: count.name,
                        subclass: count.subclass,
                        hook: count.hook,
                    })
                    .collect(),
            })
        }

        /// Gives the edge back as the event loop exits.
        pub fn release(&self) {
            self.strip.release();
        }

        /// Explorer started again, or a login started the strip before the taskbar: the bar is
        /// asked for again. A placement follows.
        pub fn taskbar_created(&self) -> Option<TaskbarReport> {
            let done = self.strip.taskbar_created();
            Some(TaskbarReport {
                action: match done.action {
                    TaskbarAction::AskOnce => TaskbarStep::AskOnce,
                    TaskbarAction::RegisterAfresh => TaskbarStep::RegisterAfresh,
                    TaskbarAction::Nothing => TaskbarStep::Nothing,
                },
                answered: done.answered,
                shell_hook: done.shell_hook,
            })
        }

        /// Looks at the full-screen apps, and puts the strip behind the one on its monitor or
        /// back on top. Reported only when that changed.
        pub fn check_fullscreen(&self, window: &Window) -> Option<FullscreenReport> {
            let strip_monitor = window
                .current_monitor()
                .and_then(|handle| windows::monitor_rects(handle.hmonitor()))?
                .monitor;
            let monitors: Vec<PixelRect> = window
                .available_monitors()
                .filter_map(|handle| windows::monitor_rects(handle.hmonitor()))
                .map(|rects| rects.monitor)
                .collect();
            let check = self.strip.check_fullscreen(strip_monitor, &monitors);
            check.changed.then_some(FullscreenReport {
                yielded: check.yielded,
                apps: check.apps,
                front_class: check.front_class,
            })
        }
    }

    /// A second launch, which the person started, lets the running app bring its window to the
    /// front.
    pub fn allow_foreground_for_running() {
        windows::allow_any_foreground();
    }

    /// The taskbar's edge, rectangle and auto-hide setting, for the probe's header.
    pub fn taskbar() -> Option<TaskbarState> {
        windows::taskbar_state().map(|state| TaskbarState {
            edge: state.edge,
            rect: px(state.rect),
            auto_hide: state.auto_hide,
        })
    }

    fn px(rect: PixelRect) -> PxRect {
        PxRect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }

    fn appbar_placed(placement: &Placement, scale: &StripScale) -> AppBarPlaced {
        AppBarPlaced {
            monitor: px(placement.monitor),
            work_before: px(placement.work_before),
            reserved: placement.reserved,
            proposal: px(placement.proposal),
            queried: placement.queried.map(px),
            set: px(placement.set),
            window: placement.window.map(px),
            work_after: placement.work_after.map(px),
            unchanged: placement.unchanged,
            size_mismatch: u32::try_from(placement.set.width()) != Ok(scale.width)
                || u32::try_from(placement.set.height()) != Ok(scale.height),
            fresh: placement.fresh,
            autohide_inset: placement.autohide_inset,
            stale: placement.stale.as_ref().map(stale_report),
        }
    }

    fn stale_report(check: &StaleCheck) -> StaleReport {
        StaleReport {
            expected: px(check.expected),
            taskbars: check.taskbars,
            suspected: check.suspected,
            other_instance: check.other_instance,
            cleared: check.cleared,
            work_after: check.work_after.map(px),
        }
    }

    pub fn app_is_active() -> Option<bool> {
        Some(windows::foreground_is_this_process())
    }

    /// A window created active already has the keyboard. winit's `focus_window` would fake
    /// an Alt key press to get it, which is not needed.
    pub fn activate(_window: &Window) {}

    /// Every launch on Windows starts a process, which finds the running app by its lock
    /// (`instance`): there is no reopen event to take.
    pub struct ReopenHook;

    pub fn on_reopen(_on_reopen: impl Fn() + 'static) -> Option<ReopenHook> {
        None
    }

    /// Windows activates the next window in the stack when one closes, and the strip is never
    /// activated, so focus goes back by itself: there is nothing to keep.
    pub struct PreviousApp;

    impl PreviousApp {
        pub fn frontmost() -> Option<PreviousApp> {
            None
        }

        pub fn restore(&self) {}
    }

    /// The foreground window's class, which never carries a document's name.
    pub fn frontmost_app() -> Option<String> {
        windows::foreground_class()
    }

    pub fn os_version() -> String {
        copy::os_version(windows::os_version().map(|v| (v.major, v.minor, v.build)))
    }

    pub fn separate_spaces() -> Option<bool> {
        None
    }
}
