# terrarium-app

The `repo-terrarium` binary.

## Owns

- The winit `ApplicationHandler` and the strip window (`strip`): borderless, along the bottom
  edge of the chosen monitor, below ordinary windows, and never in the Dock or the app
  switcher.
- The monitor choice (`monitor`): the setting, else a non-primary monitor, else the primary.
- The living strip (`animate`): the thriving preset through the composer, kept by day until
  the live terrarium arrives, composed only when the composer says the picture changes and at
  most 15 times a second.
- The `Presenter` that puts composed pixels on screen (`present`), with softbuffer as the first
  backend. It presents on resize, re-placement and the OS's expose requests, and when the
  living strip's pixels change; a still strip draws no frames.
- Hover in native pixels (`hover`), the raise hotkey and its timer (`hotkey`, `raise`), and the
  right-click menu (`menu`).
- The OS half of the strip (`pinning`): the level, placement and system events, through
  `terrarium-platform`. On macOS the strip is a desktop-level window; on Windows it is an
  `AppBar` on a reserved edge, sized to the band Windows grants.
- `--probe pinning` (`probe`): the strip with a recorder, a guided checklist and a report.
- Later: the egui windows (settings, time-lapse, debug panel).

## Why it is a crate of its own

The app wires the other crates together and is the only crate that depends on all of them. Tests
that span crates, such as salt independence across engine and compose, live here.

## Rules

- `#![forbid(unsafe_code)]`: all FFI goes through `terrarium-platform`.
- The version has one source, the workspace manifest, read through `CARGO_PKG_VERSION`.
- User-facing strings live in the `copy` module (I-8).
- The event loop sleeps in `ControlFlow::Wait`. It waits until a deadline only for a raise
  ending or the living strip's next change, never while the strip is covered, and other threads
  wake it only through the `EventLoopProxy`.
- `--probe pinning` shows the still sample, so its idle step can count on no frames.

## Command line

- `repo-terrarium` runs the strip. Right-click it and choose **Quit** to close it.
- `repo-terrarium --version` prints the version.
- `repo-terrarium --probe pinning` runs the strip and walks through the pinning checklist on
  stdout. Each answer is a line on stdin: Enter moves on, and text typed first is kept as a
  note. The JSON and text reports are rewritten after every step, in the current directory.
  On macOS it compares two window levels; on Windows it walks through the `AppBar` checks and
  ends with a `taskkill` that stands in for a crash.
- `repo-terrarium --probe-readme` prints the Windows probe's instructions for testers, which
  `cargo xtask probe-zip` ships as `README.txt`.

## Requirements

Invariants I-8, I-9 and I-10; budget N-03.
