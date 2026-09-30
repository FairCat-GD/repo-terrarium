# Changelog

What shipped, per version. The format follows [Keep a Changelog](https://keepachangelog.com/).
Each step of development is a minor version, `0.<n>.0`, until the first public release.

## [Unreleased]

## [0.3.0] - 2026-09-30

### Added

- **On Windows the strip is an `AppBar`.** It reserves the bottom edge of its monitor the way the
  taskbar does, so maximised windows stop above it. It sits above the taskbar when the two
  share a display.
  - It stays above other windows, and is never in the taskbar or Alt-Tab.
  - Clicking it does not take focus from the app in use.
  - When the taskbar or another bar moves, it asks for its edge again.
  - It is sized to the band Windows grants, which can be narrower than the monitor.
  - The edge is given back when the strip quits, panics, gets Ctrl-C or crashes on an
    exception. A reservation left by a killed strip is cleared the next time it starts.
  - If Windows refuses the reservation, the strip falls back to a plain window above the
    taskbar and says so once on stderr.
  - It is not yet tested on real Windows hardware.
- **`--probe pinning` on Windows** walks through focus on click, maximising, Alt-Tab and the
  taskbar, Win+D, full screen, display scale, the hotkey, idle CPU, a second display, and a
  `taskkill` that stands in for a crash. It records:
  - the work area before and after each placement;
  - the window's style;
  - which window messages arrived sent and which arrived posted.
- **`cargo xtask probe-zip`** cross-builds the Windows exe on a Mac and zips it with the
  tester's instructions and a double-click launcher.
- **`repo-terrarium --probe-readme`** prints those instructions.

### Changed

- On Windows the strip's corners are square, and the report names the foreground window's
  class instead of leaving it unknown.

## [0.2.0] - 2026-09-29

### Added

- The strip. `repo-terrarium` now opens a borderless strip along the bottom edge of a monitor,
  preferring one other than the primary.
- **Size.** The strip shows the 640×74 scene at the largest whole-number scale that fits the
  monitor's width, centred, with the frame continuing to both edges.
- **On macOS the strip sits just above the desktop icons:**
  - below every ordinary window, on every Space;
  - never in the Dock or the Command-Tab switcher;
  - left in place by another app's Hide Others;
  - moved to fit again when displays change or the Mac wakes;
  - the pointer is followed over it even while another app is in front.
- **A placeholder scene** in the palette's colours: sky, the soil band and the glass edges. The
  art arrives later.
- **Raise hotkey.** Ctrl+Alt+T (Control-Option-T on a Mac) raises the strip above other
  windows for 10 seconds without taking focus from the app in use.
- **Quit.** Right-click the strip for a menu with **Quit**.
- **No idle drawing.** The strip draws only when its size or place changes, so a still strip
  draws no frames and uses no CPU.
- **`repo-terrarium --probe pinning`** walks a tester through the pinning checklist and writes
  a JSON and a text report to the current directory.

### Removed

- The plain titled window.

## [0.1.0] - 2026-09-29

### Added

- `repo-terrarium --version` prints `repo-terrarium 0.1.0`.
- `repo-terrarium` opens a plain window titled `Repo Terrarium 0.1.0`, which closes when you
  close it.
- The workspace: eight crates whose dependency graph keeps truth pure, compose away from live
  data, and `unsafe` in one place.
- `cargo xtask verify`, the single gate: formatting, clippy for macOS and for Windows, tests,
  doctests, the supply-chain policy (`cargo-deny`) and the crate-graph check.
- Licenses: code under MIT or Apache-2.0, art under CC BY-SA 4.0, bitmap fonts public domain.
