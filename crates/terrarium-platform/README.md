# terrarium-platform

Operating-system integration that needs FFI.

## Owns

- `geometry`: where the strip sits on a macOS screen, as pure arithmetic tested on every OS.
- `appbar`: the Windows `AppBar`'s pure half, compiled and tested on every OS:
  - the band on an edge, before and after Windows adjusts it, and the line an auto-hide
    taskbar keeps free;
  - the check for a reservation a crashed strip left, and when a move to another monitor or
    edge registers the bar afresh;
  - what Explorer starting again asks of the bar;
  - the pin that holds every move of the strip to its granted band;
  - the extended-style guard;
  - the names of the window messages the probe counts.

  Its Win32 numbers are checked against the `windows` crate when the Windows build compiles.
- `fullscreen`: when the strip steps behind a full-screen app, as `ManagedShell` decides it,
  tested on every OS.
- `display`: a Windows display's id from its monitor's device path, tested on every OS.
- `power`: Windows' resume and display-state notifications read as a wake, tested on every OS.
- `console`: letting go of a console Windows opened for this launch alone.
- `macos`:
  - the strip's window level (desktop-icon + 1, or floating while raised);
  - its collection behaviour (every Space, fixed in Mission Control, out of the window
    cycle), and immunity to another app's Hide Others;
  - its placement on a screen's frame;
  - an always-active tracking area. Without it an inactive app gets no hover.
  - the screens with their stable display UUIDs;
  - the display, wake, Space and activation notifications winit does not deliver;
  - the context menu.
- `windows`: the strip as an `AppBar`.
  - The strip reserves its edge, is topmost, stays out of the taskbar and Alt-Tab, and is
    never activated by a click.
  - A `SetWindowSubclass` procedure sees the sent messages winit's hook cannot.
  - The edge is given back on a normal exit, a panic, Ctrl-C, an unhandled exception and the
    end of a session, and a reservation left by a crash is cleared when the bar registers.
  - It is asked for again when Explorer restarts, stays out of Aero Peek, steps behind a
    full-screen app on its monitor, and hears sleep, wake and the screens' power.
  - Also: the display configuration's device paths, the taskbar's state, the probe's message
    counts, the foreground window, the OS version, and the context menu, which gives the front
    back when dismissed.
- `clock`: the local date and time of day, for day and night (`localtime_r` on macOS,
  `GetLocalTime` on Windows).
- `idle`: how long the machine has gone without input, for dim mode (`CGEventSource` on macOS,
  `GetLastInputInfo` on Windows). Neither asks the user for permission.
- `interrupt`: Ctrl-C left to the command `record-tests` runs, so the wrapper keeps waiting
  for it instead of ending first (a do-nothing signal handler on macOS, which a started
  program does not inherit; a console control handler on Windows).
- `memory`: the process's resident size, and on macOS its footprint, for the debug window and
  the egui memory probe.
- On macOS, the app the user was in, so an egui window that took the keyboard can give it back
  (`macos::PreviousApp`); bringing a window forward with the keyboard though the app is an
  accessory (`macos::bring_forward`); and the reopen event a second launch of the installed
  app sends (`macos::on_reopen`, through the Apple Event manager, since winit keeps the
  application delegate).
- `login`: opening at login, through `SMAppService` on macOS (only in an app bundle; any other
  build says so and touches nothing) and the per-user `Run` key on Windows.

## Why it is a crate of its own

FFI is reviewed line by line, so all of it lives in one small crate (I-9).

## Rules

- It is the only crate allowed `unsafe`, and every `unsafe` block holds one operation and
  carries a `SAFETY:` comment (`clippy::undocumented_unsafe_blocks` and
  `clippy::multiple_unsafe_ops_per_block` are denied); `unsafe extern` blocks and unsafe
  attributes, which no lint reaches, carry one too.
- Every public function is safe to call. Main-thread requirements are checked, not assumed.
- It depends on no other workspace crate. On Windows it links winit, but only to install
  winit's message hook, so the raw `MSG` pointer is read here and nowhere else.
- winit links objc2 0.5 and this crate objc2 0.6, so their types never meet. The only bridge
  is the view pointer in winit's raw window handle.

## Requirements

Invariant I-9.
