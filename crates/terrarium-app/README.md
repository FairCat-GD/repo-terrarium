# terrarium-app

The `repo-terrarium` binary.

## Owns

- The winit `ApplicationHandler` and the strip window (`strip`): borderless, along the edge of
  the monitor the settings name, below ordinary windows, and never in the Dock or the app
  switcher. A pause stands the scene still while truth keeps time.
- The monitor choice (`monitor`): the setting, else a non-primary monitor, else the primary.
- The living strip (`animate`): a scene through the composer, composed only when the composer
  says the picture changes and at most 15 times a second.
- The live strip (`live`): the engine started on the repository `--repo` names or the one
  connected last, its updates applied to the scene and the label, and the label's words,
  among them the notes shown once.
- What it shows (`feed`): the live scene or `--demo`'s cycle (`demo`), always on the local
  clock, taken again every hour and after a wake, with day and night when the settings say, and
  dimmed after the settings' while without input (`idle`); held as it stood while paused.
- The settings (`prefs`): every setting typed, loaded with a default for anything absent or
  unreadable, and saved by a thread of its own. The opt-in crash log (`crash`).
- The windows beside the strip (`shell`), on one egui host (`egui_host`) made with the first
  window and dropped with the last, painted only when egui asks (`repaint`): the connect window
  (`connect`), open while no repository is connected, and the settings window (`settings`), a
  tab each for the repository, the strip, the adapters, the hotkeys, the time and the rest.
  Their forms are tested models whose presses come back as intents; every picker is the
  first of two steps.
- The terrarium's lifecycle (`lifecycle`): connecting from the window, relinking, re-seeding,
  importing observations and disconnecting, each on a thread of its own that stops the engine,
  changes the store and starts a new engine, whose generation tags its updates.
- The terrarium's data (`portability`), from the settings' repository tab: its observations
  exported as JSON Lines (the store's format), which the same repository's terrarium on another
  computer imports, and its climate exported as JSON (below). Each export reads through a
  connection of its own on a thread of its own; every file the app writes is written beside its
  path and takes the name once whole (`files`).
- One app at a time (`instance`): a lock in the data directory, and the requests a second
  launch leaves there. Opening at login (`login`), through the platform's login item.
- The `Presenter` that puts composed pixels on screen (`present`), with softbuffer as the first
  backend. It presents on resize, re-placement and the OS's expose requests, and when the
  living strip's pixels change; a still strip draws no frames.
- Hover in native pixels (`hover`), the hotkeys, their bindings and the raise's timer
  (`hotkey`, `raise`), and the right-click menu (`menu`).
- The OS half of the strip (`pinning`): the level, placement and system events, through
  `terrarium-platform`. On macOS the strip is a desktop-level window; on Windows it is an
  `AppBar` on a reserved edge, sized to the band Windows grants.
- `--probe pinning` (`probe`): the strip with a recorder, a guided checklist and a report.
- The time-lapse window (`timelapse`): the repository's history replayed by a replay and a
  composer of its own, played in 30 seconds over a range of days, by day or by night, and
  written as a GIF, a postcard PNG, or, when the person's ffmpeg is found, an MP4 (`mp4`): raw
  RGB upscaled to 4× or more fed to it, which writes H.264 in `yuv420p`. Its reading,
  replaying and writing run on threads of their own; the live strip's engine and composer are
  never touched.
- In dev builds (`--features dev`): the debug window (`debug`), with its simulation of the real
  sim over a history its sliders make (`sandbox`), and `--probe egui` (`egui_probe`).

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
  the `EventLoopProxy`. It never waits on another thread: the engine starts before it runs,
  and every later start and stop happens on a thread of the app's.
- Nothing is opened or changed from inside a window's drawing; dialogs are split from the
  writes they lead to, so a harness drives every write without a dialog.
- The debug window and `--probe egui` exist only with the `dev` feature, which no build enables
  by default (`cargo xtask verify`'s graph check).
- `--probe pinning` shows the still sample, so its idle step can count on no frames.

## Command line

- `repo-terrarium --repo <path>` connects the repository at `<path>` (creating its terrarium,
  or relinking the one with its seed) and runs the strip on it.
- `repo-terrarium` runs the strip on the repository connected last, or opens the connect
  window when none is. Right-click the strip for **Settings**, **Time-lapse**, **Pause** or
  **Quit**; the time-lapse hotkey (Control-Option-H by default) opens the time-lapse too. Launched again
  while it runs, it shows the running app's settings and ends; with `--repo`, it connects
  nothing and says so.
- `repo-terrarium --demo` runs the strip through the named looks, a quarter of an hour each.
- `repo-terrarium --version` prints the version.
- `repo-terrarium replay <path> --dump` replays a repository through the engine's headless
  pipeline and prints truth's canonical dump (`replay`). `--observations <file>` imports an
  export into its store first, and `--at <unix-seconds>` replays to that moment instead of
  now. Its store lives in a temporary directory, so it never opens the data directory.
  `--gif <file>` writes the whole history's time-lapse instead, as the window does, with
  `--night` by night and `--scale <n>` (1 to 8) times the scene's size; the same command
  always writes the same bytes. `cargo xtask hero` writes the fixture arc's this way.
  `--mp4 <file>` writes it as an MP4 through the person's ffmpeg, at `--scale` or 4×,
  whichever is larger, and exits 1 when no ffmpeg is found.
- `repo-terrarium discover <path>` prints what the repository has for each adapter: the
  source files at the default tip in each language the metrics read, what each report found
  reads as, and how the missing ones connect (`discover`).
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

## Climate export (format version 1)

One JSON object in UTF-8: the header's fields, then `days`, one line each, then `timeline`,
one line each.

```json
{"format":"repo-terrarium-climate","format_version":1,"seed":"<64 hex>","sim_version":8,"days":[
{"date":"2024-03-14","light":{"value":0.80914306640625,"stale":false},"soil":null,
 "dead_code":null,"complexity":0.75,"todo_density":0.9,"rain":1.25,"wind":1.0,"health":0.5,
 "activity":0.5,"terrain":12345,"weather":{"state":"pass","source":"ci:main","stale":false}}
],"timeline":[
{"at":"2024-03-14T05:00:00Z","state":"thriving","health":"healthy","activity":"active"}
]}
```

The day is wrapped here; in a file each is one line.

- A day is each UTC day the history has finished, from its first commit, as that day's close
  left it, with the observations recorded here.
- `light` and `soil` are smoothed, as health reads them, with whether the reading behind each
  is stale; `dead_code`, `complexity` and `todo_density` are the soil's components as last
  reported. Each is 1 at its best.
- `rain` and `wind` are against the repository's own pace, 1 being its usual; `health` and
  `activity` are what the presentation reads; `terrain` is the default tip's text lines.
- `weather.state` is `pass` or `fail`, `source` is `ci:<branch>` or `local`, and both are
  `null` while unknown or hazy.
- Unknown is `null`, never 0. Every fraction is exact: a whole number of 65536ths.
- `timeline` lists every change of the presentation's state, health class or activity class,
  each from the UTC hour it began.

## Requirements

Invariants I-8, I-9 and I-10; budget N-03.
