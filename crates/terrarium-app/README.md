# terrarium-app

The `repo-terrarium` binary.

## Owns

- The winit `ApplicationHandler` and the strip window.
- The `Presenter` that puts composed pixels on screen.
- The egui windows: settings, time-lapse and the debug panel.
- Hotkeys, the context menu and the command line.

## Why it is a crate of its own

The app wires the other crates together and is the only crate that depends on all of them. Tests
that span crates, such as salt independence across engine and compose, live here.

## Rules

- `#![forbid(unsafe_code)]`: all FFI goes through `terrarium-platform`.
- The version has one source, the workspace manifest, read through `CARGO_PKG_VERSION`.
- User-facing strings live in the `copy` module (I-8).

## Command line

- `repo-terrarium` opens the window.
- `repo-terrarium --version` prints the version.

## Requirements

Invariants I-8 and I-9.
