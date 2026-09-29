#![doc = include_str!("../README.md")]
#![allow(unsafe_code)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod geometry;
#[cfg(target_os = "macos")]
pub mod macos;
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
}
