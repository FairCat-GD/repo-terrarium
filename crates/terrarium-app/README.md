# terrarium-app

The `repo-terrarium` binary.

## Owns

- The winit `ApplicationHandler` and the strip window (`strip`): borderless, along the bottom
  edge of the chosen monitor, below ordinary windows, and never in the Dock or the app
  switcher.
- The monitor choice (`monitor`): the setting, else a non-primary monitor, else the primary.
- The living strip (`animate`): a scene through the composer, composed only when the composer
  says the picture changes and at most 15 times a second.
- The live strip (`live`): the engine started on the repository `--repo` names or the one
  connected last, its updates applied to the scene and the label, and the label's words.
- What it shows (`feed`): the live scene, the thriving preset when nothing is connected, or
  `--demo`'s cycle (`demo`), always on the local clock, taken again every hour and after a
  wake, and dimmed after 30 minutes without input (`idle`).
- The `Presenter` that puts composed pixels on screen (`present`), with softbuffer as the first
  backend. It presents on resize, re-placement and the OS's expose requests, and when the
  living strip's pixels change; a still strip draws no frames.
- Hover in native pixels (`hover`), the raise hotkey and its timer (`hotkey`, `raise`), and the
  right-click menu (`menu`).
- The OS half of the strip (`pinning`): the level, placement and system events, through
  `terrarium-platform`. On macOS the strip is a desktop-level window; on Windows it is an
  `AppBar` on a reserved edge, sized to the band Windows grants.
- `--probe pinning` (`probe`): the strip with a recorder, a guided checklist and a report.
- In dev builds (`--features dev`): the egui windows (`egui_host`), on one wgpu device made with
  the first window and dropped with the last, painted only when egui asks (`repaint`); the debug
  window (`debug`); and `--probe egui` (`egui_probe`).

## Why it is a crate of its own

The app wires the other crates together and is the only crate that depends on all of them. Its
integration tests run the binary's command line (`tests/cli.rs`); salt independence is tested
where each half lives, compose's layout in `tests/composer.rs` and truth's dump in the engine's
`tests/arc.rs`.

## Rules

- `#![forbid(unsafe_code)]`: all FFI goes through `terrarium-platform`.
- The version has one source, the workspace manifest, read through `CARGO_PKG_VERSION`.
- User-facing strings live in the `copy` module (I-8).
- The event loop sleeps in `ControlFlow::Wait`. It waits until a deadline only for a raise
  ending, the living strip's next change, the feed's next look, or an open egui window's next
  repaint; the strip asks for none while it is covered, and other threads wake it only through
  the `EventLoopProxy`.
- The debug window and `--probe egui` exist only with the `dev` feature, which no build enables
  by default (`cargo xtask verify`'s graph check).
- `--probe pinning` shows the still sample, so its idle step can count on no frames.

## Command line

- `repo-terrarium --repo <path>` connects the repository at `<path>` (creating its terrarium,
  or relinking the one with its seed) and runs the strip on it.
- `repo-terrarium` runs the strip on the repository connected last, or on the thriving preset
  when none is. Right-click it and choose **Quit** to close it.
- `repo-terrarium --demo` runs the strip through the named looks, a quarter of an hour each.
- `repo-terrarium --version` prints the version.
- `repo-terrarium replay <path> --dump` replays a repository through the engine's headless
  pipeline and prints truth's canonical dump (`replay`). `--observations <file>` imports an
  export into its store first, and `--at <unix-seconds>` replays to that moment instead of
  now. Its store lives in a temporary directory, so it never opens the data directory.
- `repo-terrarium discover <path>` prints what the repository has for each report adapter,
  what each report found reads as, and how the missing ones connect (`discover`).
- `repo-terrarium record-tests -- <command>` runs a test command in the current repository and
  writes how it ended to the data directory, where the strip reads it as local weather
  (`record-tests`). It exits with the command's own status.
- `repo-terrarium --probe pinning` runs the strip and walks through the pinning checklist on
  stdout. Each answer is a line on stdin: Enter moves on, and text typed first is kept as a
  note. The JSON and text reports are rewritten after every step, in the current directory.
  On macOS it compares two window levels; on Windows it walks through the `AppBar` checks and
  ends with a `taskkill` that stands in for a crash.
- `repo-terrarium --probe-readme` prints the Windows probe's instructions for testers, which
  `cargo xtask probe-zip` ships as `README.txt`.
- In dev builds, `repo-terrarium --probe egui` opens and closes the debug window five times on
  timers and writes `egui-memory-<time>.txt` to the current directory: memory, loop wakes and
  focus per phase. Run it on a release build.

## Requirements

Invariants I-8, I-9 and I-10; budget N-03.
