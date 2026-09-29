# terrarium-platform

Operating-system integration that needs FFI.

## Owns

- `geometry`: where the strip sits on a screen, as pure arithmetic tested on every OS.
- `macos`:
  - the strip's window level (desktop-icon + 1, or floating while raised);
  - its collection behaviour (every Space, fixed in Mission Control, out of the window
    cycle), and immunity to another app's Hide Others;
  - its placement on a screen's frame;
  - an always-active tracking area. Without it an inactive app gets no hover.
  - the screens with their stable display UUIDs;
  - the display, wake, Space and activation notifications winit does not deliver;
  - the context menu.
- `windows`: the context menu. Later the `AppBar` registration, window subclassing and
  fullscreen detection.
- Later: idle time, the login item, and power notifications.

## Why it is a crate of its own

FFI is reviewed line by line, so all of it lives in one small crate (I-9).

## Rules

- It is the only crate allowed `unsafe`, and every `unsafe` block carries a `SAFETY:` comment
  (`clippy::undocumented_unsafe_blocks` is denied).
- Every public function is safe to call. Main-thread requirements are checked, not assumed.
- It depends on no other workspace crate.
- winit links objc2 0.5 and this crate objc2 0.6, so their types never meet. The only bridge
  is the view pointer in winit's raw window handle.

## Requirements

Invariant I-9.
