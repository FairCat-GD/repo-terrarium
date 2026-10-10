#![doc = include_str!("../README.md")]
#![allow(unsafe_code)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod appbar;
pub mod clock;
pub mod geometry;
pub mod idle;
pub mod interrupt;
pub mod login;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod memory;
#[cfg(windows)]
pub mod windows;

/// System changes the strip reacts to. winit reports none of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemEvent {
    /// A display was connected, removed, rearranged or changed resolution.
    ScreensChanged,
    Woke,
    ScreensWoke,
    SpaceChanged,
    AppBecameActive,
    AppResignedActive,
    /// The taskbar or another `AppBar` moved, so the strip's reserved edge must be asked for
    /// again (Windows).
    AppBarPosChanged,
    /// The taskbar's auto-hide or always-on-top setting changed (Windows).
    AppBarStateChanged,
    /// A hint that a full-screen app opened (`true`) or closed on some monitor (Windows).
    FullscreenApp(bool),
    /// Some monitor's work area changed (Windows).
    WorkAreaChanged,
    /// Explorer started again, and forgot every `AppBar` (Windows).
    TaskbarCreated,
}
