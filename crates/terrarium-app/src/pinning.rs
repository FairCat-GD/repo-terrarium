//! The OS half of the strip: its window attributes, level, placement, menu and system events.
//! On macOS it goes through `terrarium-platform::macos`; on Windows the strip is an `AppBar`
//! through `terrarium-platform::windows`. They are v1's only targets.

use serde::Serialize;

pub use imp::{
    AttachError, Monitor, Observers, Pinning, app_is_active, configure_event_loop, frontmost_app,
    os_version, separate_spaces, window_attributes,
};
#[cfg(feature = "dev")]
pub use imp::{PreviousApp, activate};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale: Option<StaleReport>,
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

    use super::{LevelMode, OsSnapshot, Placed};
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
    }

    pub fn app_is_active() -> Option<bool> {
        Some(macos::app_is_active())
    }

    /// Gives `window` the keyboard. An accessory app is not activated by a new window, so it
    /// is activated here; winit does that through `activateIgnoringOtherApps`.
    #[cfg(feature = "dev")]
    pub fn activate(window: &Window) {
        window.focus_window();
    }

    /// The app the user was in, to give the keyboard back to when the last window closes.
    #[cfg(feature = "dev")]
    pub use macos::PreviousApp;

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
    use terrarium_platform::appbar::PixelRect;
    use terrarium_platform::geometry::Edge;
    use terrarium_platform::windows::{self, Placement, StaleCheck, StripWindow};
    use winit::event_loop::EventLoopBuilder;
    use winit::monitor::MonitorHandle;
    use winit::platform::windows::{
        CornerPreference, MonitorHandleExtWindows, WindowAttributesExtWindows,
    };
    use winit::window::{Window, WindowAttributes};

    use super::{AppBarPlaced, LevelMode, MessageCount, OsSnapshot, Placed, PxRect, StaleReport};
    use crate::copy;
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
    /// Square corners, since the window keeps a caption style Windows 11 would round.
    pub fn window_attributes() -> WindowAttributes {
        WindowAttributes::default()
            .with_skip_taskbar(true)
            .with_corner_preference(CornerPreference::DoNotRound)
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

        /// winit's monitors, each read through platform: its own `rcMonitor` and its device
        /// name (`\\.\DISPLAY1`), which is the id. A monitor unplugged while the list is read
        /// is left out, where winit's own `size` and `name` would panic.
        pub fn monitors(&self, window: &Window) -> Vec<Monitor> {
            window
                .available_monitors()
                .enumerate()
                .filter_map(|(index, handle)| {
                    let (rects, name) = windows::monitor_described(handle.hmonitor())?;
                    let size = |length: i32| u32::try_from(length).unwrap_or(0);
                    let detail = copy::monitor_detail(&px(rects.monitor), &px(rects.work));
                    Some(Monitor {
                        info: MonitorInfo {
                            id: MonitorId(if name.is_empty() {
                                copy::monitor_id(index)
                            } else {
                                name.clone()
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

        pub fn show_menu(&self, window: &Window, menu: &muda::Menu) {
            windows::show_context_menu(menu, window);
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
    #[cfg(feature = "dev")]
    pub fn activate(_window: &Window) {}

    /// Windows activates the next window in the stack when one closes, and the strip is never
    /// activated, so focus goes back by itself: there is nothing to keep.
    #[cfg(feature = "dev")]
    pub struct PreviousApp;

    #[cfg(feature = "dev")]
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
