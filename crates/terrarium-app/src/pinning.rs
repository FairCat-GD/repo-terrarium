//! The OS half of the strip: its window attributes, level, placement, menu and system events.
//! On macOS it goes through `terrarium-platform`; elsewhere a plain winit window stands in
//! until the Windows `AppBar` exists.

use serde::Serialize;

pub use imp::{
    Monitor, Observers, Pinning, app_is_active, configure_event_loop, frontmost_app, os_version,
    separate_spaces, window_attributes,
};

/// Where the strip sits in the window stack. The checklist decides between the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LevelMode {
    /// Just above the desktop icons.
    DesktopIconPlusOne,
    /// winit's `AlwaysOnBottom`: just below ordinary windows.
    AlwaysOnBottom,
}

/// The level the strip runs at. The macOS pinning checklist picks it.
pub const DEFAULT_LEVEL: LevelMode = LevelMode::DesktopIconPlusOne;

/// The frame the strip was given: points in `AppKit` coordinates on macOS, physical pixels
/// elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Placed {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub unit: &'static str,
}

// Both implementations keep one signature, so each has parameters and `Option`s only the other
// needs.
#[cfg(target_os = "macos")]
#[allow(clippy::unused_self, clippy::unnecessary_wraps)]
mod imp {
    use terrarium_compose::scale::StripScale;
    use terrarium_platform::SystemEvent;
    use terrarium_platform::geometry::Edge;
    use terrarium_platform::macos::{self, PlatformError, Screen, StripWindow};
    use winit::event_loop::EventLoopBuilder;
    use winit::platform::macos::{
        ActivationPolicy, EventLoopBuilderExtMacOS, WindowAttributesExtMacOS,
    };
    use winit::window::{Window, WindowAttributes, WindowLevel};

    use super::{LevelMode, Placed};
    use crate::monitor::{MonitorId, MonitorInfo};

    pub use macos::Observers;

    /// An accessory app: no Dock icon, no Cmd-Tab entry, no menu bar, and a launch that asks
    /// not to take focus from the app the user is in.
    pub fn configure_event_loop<T>(builder: &mut EventLoopBuilder<T>) {
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

    pub struct Pinning {
        strip: StripWindow,
    }

    impl Pinning {
        pub fn attach(window: &Window) -> Result<Self, PlatformError> {
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
                                .unwrap_or_else(|| format!("display-{}", screen.display_id)),
                        ),
                        name: screen.name.clone(),
                        physical_width: screen.physical_width(),
                        physical_height: screen.physical_height(),
                        scale: screen.scale,
                        primary: screen.primary,
                        detail: format!(
                            "display {}, frame {:?}, visible {:?} (points)",
                            screen.display_id, screen.frame, screen.visible_frame
                        ),
                    },
                    screen,
                })
                .collect()
        }

        pub fn place(
            &self,
            _window: &Window,
            monitor: &Monitor,
            scale: &StripScale,
            edge: Edge,
        ) -> Placed {
            let frame = self
                .strip
                .place(&monitor.screen, edge, scale.width, scale.height);
            Placed {
                x: frame.x,
                y: frame.y,
                width: frame.width,
                height: frame.height,
                unit: "pt",
            }
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
    }

    pub fn app_is_active() -> Option<bool> {
        Some(macos::app_is_active())
    }

    pub fn frontmost_app() -> Option<String> {
        macos::frontmost_app()
    }

    pub fn os_version() -> String {
        format!("macOS {}", macos::os_version())
    }

    pub fn separate_spaces() -> Option<bool> {
        Some(macos::screens_have_separate_spaces())
    }
}

#[cfg(not(target_os = "macos"))]
#[allow(clippy::unused_self, clippy::unnecessary_wraps)]
mod imp {
    use std::convert::Infallible;

    use terrarium_compose::scale::StripScale;
    use terrarium_platform::SystemEvent;
    use terrarium_platform::geometry::Edge;
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::event_loop::EventLoopBuilder;
    use winit::monitor::MonitorHandle;
    use winit::window::{Window, WindowAttributes, WindowLevel};

    use super::{LevelMode, Placed};
    use crate::monitor::{MonitorId, MonitorInfo};

    pub fn configure_event_loop<T>(_builder: &mut EventLoopBuilder<T>) {}

    pub fn window_attributes() -> WindowAttributes {
        let attributes = WindowAttributes::default();
        #[cfg(windows)]
        let attributes = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_skip_taskbar(true)
        };
        attributes
    }

    pub struct Monitor {
        pub info: MonitorInfo,
        handle: MonitorHandle,
    }

    /// Nothing to observe until the Windows platform module exists.
    pub struct Observers;

    pub struct Pinning;

    impl Pinning {
        pub fn attach(_window: &Window) -> Result<Self, Infallible> {
            Ok(Self)
        }

        pub fn monitors(&self, window: &Window) -> Vec<Monitor> {
            let primary = window.primary_monitor();
            window
                .available_monitors()
                .enumerate()
                .map(|(index, handle)| {
                    let name = handle.name().unwrap_or_default();
                    let (size, position) = (handle.size(), handle.position());
                    Monitor {
                        info: MonitorInfo {
                            id: MonitorId(if name.is_empty() {
                                format!("monitor-{index}")
                            } else {
                                name.clone()
                            }),
                            name,
                            physical_width: size.width,
                            physical_height: size.height,
                            scale: handle.scale_factor(),
                            primary: primary.as_ref() == Some(&handle),
                            detail: format!("origin ({}, {}) px", position.x, position.y),
                        },
                        handle,
                    }
                })
                .collect()
        }

        pub fn place(
            &self,
            window: &Window,
            monitor: &Monitor,
            scale: &StripScale,
            edge: Edge,
        ) -> Placed {
            let (origin, size) = (monitor.handle.position(), monitor.handle.size());
            let signed = |value: u32| i32::try_from(value).unwrap_or(i32::MAX);
            let x = origin.x + (signed(size.width) - signed(scale.width)) / 2;
            let y = match edge {
                Edge::Bottom => origin.y + signed(size.height) - signed(scale.height),
                Edge::Top => origin.y,
            };
            window.set_outer_position(PhysicalPosition::new(x, y));
            let _ = window.request_inner_size(PhysicalSize::new(scale.width, scale.height));
            Placed {
                x: f64::from(x),
                y: f64::from(y),
                width: f64::from(scale.width),
                height: f64::from(scale.height),
                unit: "px",
            }
        }

        pub fn apply_level(&self, window: &Window, mode: LevelMode) {
            window.set_window_level(match mode {
                LevelMode::DesktopIconPlusOne => WindowLevel::Normal,
                LevelMode::AlwaysOnBottom => WindowLevel::AlwaysOnBottom,
            });
        }

        pub fn raise(&self, window: &Window) {
            window.set_window_level(WindowLevel::AlwaysOnTop);
        }

        pub fn show(&self, window: &Window) {
            window.set_visible(true);
        }

        pub fn level(&self) -> Option<isize> {
            None
        }

        pub fn show_menu(&self, window: &Window, menu: &muda::Menu) {
            #[cfg(windows)]
            terrarium_platform::windows::show_context_menu(menu, window);
            #[cfg(not(windows))]
            let _ = (window, menu);
        }

        pub fn observe(
            &self,
            _on_event: impl Fn(SystemEvent) + Send + Sync + 'static,
        ) -> Observers {
            Observers
        }
    }

    pub fn app_is_active() -> Option<bool> {
        None
    }

    pub fn frontmost_app() -> Option<String> {
        None
    }

    pub fn os_version() -> String {
        std::env::consts::OS.to_owned()
    }

    pub fn separate_spaces() -> Option<bool> {
        None
    }
}
