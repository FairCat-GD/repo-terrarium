# Changelog

What shipped, per version. The format follows [Keep a Changelog](https://keepachangelog.com/).
Each step of development is a minor version, `0.<n>.0`, until the first public release.

## [Unreleased]

## [0.15.0] - 2026-10-05

The strip comes alive: it shows your own repository, and follows it as you work.

### Added

- **`repo-terrarium --repo <path>`** connects a repository and opens the strip on it. After
  that, a plain `repo-terrarium` opens the strip on the repository connected last. Connecting a
  repository you connected before, even from a new folder, keeps everything it has recorded.
- **The strip shows the real terrarium:** its state follows the repository's history and the
  readings recorded on its default branch, and it moves as you work. A commit, a coverage report
  or a test report your tools write is read within about a second of the last write. A report
  still being written is read once it is complete, and one that has not changed is never
  recorded twice.
- **The label** reads the repository's name, then `branch - weather` (`main - clear`), and a
  note when one applies: `history 40%` while a long history is read for the first time,
  `repo not found` while the folder is gone, and `sim updated` once after an update that
  changed how the terrarium grows.
- **Starting is quick:** the terrarium shows where it was at once, and catches up from there.
- **Asleep or away:** after the computer sleeps, or its clock jumps, the terrarium catches up
  once, and never counts an hour twice.
- **When the repository's folder is moved or deleted,** the scene stays as it was, no time
  passes, and the terrarium carries on when the folder is back.
- **A force push or reset that rewrites the default branch** is recorded, for the earthquake a
  later release draws.
- **The terrarium never writes into your repository,** and reads it only when something
  changed or the hour turns.

## [0.14.0] - 2026-10-04

Nothing changes on screen. This release teaches the terrarium to read the reports your tools
already write, and to say what your repository has and what it lacks.

### Added

- **`repo-terrarium discover <path>`** prints what the terrarium finds in a repository: its
  default branch, a coverage report, a test report, recorded test runs, a Knip report, CI
  workflows and test runners. It says what each report reads as, and, for anything missing,
  one line on how it connects. It reads, and never runs or writes anything.
- **`repo-terrarium record-tests -- <command>`** runs your test command exactly as you typed it
  and remembers whether it passed, so local test runs can become the weather. Its exit code is
  your command's, so scripts see no difference, and Ctrl-C reaches your command while
  `record-tests` waits for it to finish. On Windows it finds `npm`, `npx`, `yarn` and `pnpm`
  as a terminal does. The result is kept in the app's data folder, never in your repository.
- **Coverage from lcov and Cobertura,** the formats Jest, Vitest, c8, nyc, cargo-llvm-cov,
  tarpaulin, coverage.py and istanbul write, several files merged into one reading.
- **Test results from `JUnit` XML,** as Maven Surefire, jest-junit, pytest and Vitest write it.
- **Reports are read in UTF-8 or UTF-16,** so a report saved from Windows PowerShell reads too,
  and paths in a report written inside a sub-folder (a monorepo package) point at the right
  files.
- **Knip 6's JSON report is read and checked.** Dead code joins soil once the code reader can
  count every export.
- **A report caught half-written, broken, or in a format the terrarium does not read never
  counts.** The last good reading stays, marked stale once you commit past it.
## [0.13.0] - 2026-10-04

Nothing changes on screen. This release adds the heart of the terrarium: it turns a
repository's history and its recorded readings into climate, and climate into the state the
strip will show.

### Added

- **Climate from history and readings.** Rain and wind follow how often you commit and how much
  you change, each against your repository's own usual pace over the last three months, so a
  steady rhythm reads as a normal day whether you commit every weekday or twice a week. Light
  and soil follow coverage and code health as recorded on the default branch; weather follows
  CI or your local test runs.
- **Readings never fake decay.** A missing reading counts as neutral, a reading the default
  branch has moved past is held as it was and marked stale, and a reading taken on a feature
  branch is kept but never applied.
- **The terrarium's state over time:** thriving, struggling, mature, dormant, dead and
  recovering, with condensation on the glass while it sleeps and dust after a year dead.
- **`repo-terrarium replay <path> --dump`** replays a repository from its first commit and
  prints everything the terrarium makes of it, as text. `--observations <file>` adds readings
  from an export, and `--at <unix-seconds>` replays to another moment. It opens no window and
  leaves your stored data alone.
- **The same history gives the same terrarium on every computer:** a recorded replay of the
  fixture repository is checked byte for byte before every commit.

## [0.12.0] - 2026-10-04

Nothing changes on screen. This release adds the reader that will turn a repository's history
into the terrarium's rain, wind and terrain.

### Added

- **Git history is read inside the app,** without the git command line, so no git install is
  needed and every computer reads the same repository the same way.
- **A repository cannot run anything through the terrarium.** Hooks, filters, text conversion,
  external diff tools and file-system monitors set in a repository are never started, and
  nothing is ever written into it.
- **Settings in one copy of a repository never change its history.** Its diff and rename
  settings, its attributes and its replace refs are ignored, so two clones of the same
  repository always agree.
- **Every local branch counts toward activity,** each commit once. Line counts leave out
  lockfiles, vendored and generated folders, minified files and binary files; a moved folder
  is a rename, never a deletion.
- **The default branch is found the way people expect:** the one the remote points at, then
  `main`, `master`, or the branch you are on.
- **Shallow clones work:** the oldest commit present stands in for the first.
- **Large histories read quickly:** 10,000 commits in about 3.5 seconds on a recent Mac.

## [0.11.0] - 2026-10-04

Nothing changes on screen. This release adds the database that will keep a terrarium's history:
what its light, soil and weather were, and when.

### Added

- **The observation store,** a SQLite database built into the app, in a folder of its own
  (`~/Library/Application Support/io.github.faircat-gd.repo-terrarium` on macOS,
  `%LOCALAPPDATA%\io.github.faircat-gd.repo-terrarium` on Windows). The app does not open it
  yet; the live terrarium starts using it in 0.15.
- **History is append-only.** Once recorded, a reading is never changed or deleted. The one
  exception is deleting a whole terrarium, which takes its history with it.
- **A terrarium is known by its repository's first commit,** so a repository that moves keeps
  its history when it is connected again.
- **Export and import of the history** as JSON Lines, for moving to another machine. An import
  refuses another repository's history, skips what is already there, and reports any reading
  that differs from the one kept.
- **The names data lives under are fixed:** the app's identifier, its executable, its database
  file and its keychain service. Renaming the product will not move anyone's data.

### Changed

- **The Windows probe build** now compiles the database's C code for Windows on the Mac, from
  the same Windows headers the build already uses.

## [0.10.2] - 2026-10-02

The terrarium has its new environment art: a landscape behind the plants, new soil and glass,
and a strip that fills a screen of any width without stretching.

### Changed

- **Each weather is a whole landscape** of sky, distant hills and woodland behind the plants,
  by day and by night, in place of the narrow sky band over a flat beige sky. A change of
  weather crossfades one landscape into the other.
- **The strip fills its screen.** On a screen whose width is no multiple of 640 it shows more
  of the landscape, up to 840 columns, with the glass at the screen's edges and the plants in
  the middle; nothing is stretched. A screen wider still is letterboxed in black.
- **New soil and glass.** The soil has clods, stones and root traces, and its edge is a row
  higher; plants, monuments and creatures stand on it. The young and mature oak sink two rows
  into it, so no ground shows between their roots. The glass is fainter and its corners no
  longer double up over its edges.
- **Clouds only under a clear sky**, and fainter by night. Overcast, haze and storm are cloud
  cover in their own landscapes.
- **The glint on the glass** now glides slowly, fades in and out, and comes every 20 to 30
  minutes instead of every 4 to 7.

### Fixed

- **A gust or a creature's step could start late**, at the next other change, when nothing else
  moved for a while: the strip now wakes for every turn's first motion.

## [0.10.1] - 2026-10-02

### Changed

- **Residents keep new hours.** Every resident except the firefly is now about by day and by
  night; the firefly still signals only after dark and sleeps through the day.

### Added

- **`cargo xtask gallery` adds `pieces/`:** the background art on its own at 3×, by day and by
  night (each weather's sky, the soil, the glass ends, the label plate, the padding beside a
  wider strip, and the empty terrarium), with `contract.csv` listing each piece's size, anchor,
  palette slots and drift.

## [0.10.0] - 2026-10-01

The strip keeps the local time, and a demo shows every look of the terrarium in turn. A plain
launch still shows the thriving terrarium, now by day and by night.

### Added

- **`repo-terrarium --demo`** plays the terrarium through its key looks, a quarter of an hour
  each: thriving, mature, struggling in low light, struggling in poor soil, dormant, dead and
  recovering. Each look eases in over five minutes, and a storm passes during the low-light
  stage. The cycle repeats about every 1 h 45 min.
- **Day and night follow your local time**, from launch and again after the machine wakes.
- **Dim mode** begins after 30 minutes with no keyboard or mouse input anywhere on the
  machine, and ends as soon as there is some.
- **Dev builds** (`--features dev`) add a debug window, opened from the right-click menu. It
  shows any preset or a climate set by sliders, a foreground event, a fixed time of day and dim
  held on or off, and reads out frames, compositions and memory. `--probe egui` measures what
  the window costs.
- **`cargo xtask gallery`** writes the A-kit's pictures to `target/gallery/`: 30 states across
  health and activity by day and by night at 3×, the named looks by day, night and dimmed, a
  contact sheet, and a key.

## [0.9.0] - 2026-10-01

The terrarium moves. The strip now shows a living preview: the thriving terrarium, with its
residents, by day, until the live terrarium reaches it. It draws only when the picture changes,
and never more than 15 times a second.

### Added

- **Residents.** Snails, ladybirds, beetles, fireflies, woodlice, springtails, earthworms and
  harvestmen live in the terrarium, each about a home beside a healthy plant, where it feeds.
  - They breathe in their idle, feeding and sleeping poses all the time.
  - Now and then one walks out from home and back. At most three are out at once, and at most
    one of them is flying or signalling.
  - By night a firefly flies or signals with its lantern. A springtail sometimes hops.
  - Day creatures sleep at night, night creatures by day, and snails keep both hours.
  - A storm sends them into hiding: the snail into its shell, the woodlouse curled up.
  - They hatch, emerge from the pupa and die through their own animations.
  - A busy terrarium is lively, a mature one calm, a dead one nearly still, and in a dormant
    one everything sleeps.
- **Wind.** Plants lean in a breeze, further in a gale, a few at a time.
- **Day and night** follow the local clock: night falls over 45 minutes from 19:00, and day
  returns from 07:00.
- **Dim mode** fades the strip to 40% brightness, and only a quarter as much moves.
- **Slow motion against burn-in.** A faint reflection sweeps across the glass every four to
  seven minutes, the sky band drifts, and the soil's texture creeps a pixel every few minutes.

## [0.8.0] - 2026-10-01

The composer now draws the whole terrarium around its plants: the sky, the ground, the glass and
the label. The strip still shows the sample scene until the live terrarium reaches it. The
preset scenes now show the whole world, by day and by night.

### Added

- **The sky.**
  - A weather band across the top: clear, overcast, storm or haze.
  - Clouds drift slowly to the right. A clear sky has a few small ones, an overcast sky more and
    larger ones, a storm none.
  - The sun by day; by night, the moon in the real phase of the local date, and stars.
  - The dusk and sunburst light washes, fading in and out.
- **The ground.**
  - Pebbles in the soil band, fossils lying in it, and small decorative tufts between the
    plants.
  - Monuments as standing stones that grow lichen with age, glow faintly at night, and carry up
    to three carved marks.
  - Weeds between the plants, merging into weed patches when there are many.
  - Fallen leaves, or dead stems once the terrarium is dead, and fungus that glows at night.
  - Healthy plants glow softly at night.
- **The glass:** corners on the frame, and condensation and dust when the terrarium calls for
  them.
- **The label** now sits at the top left, as in the art's reference scene, clear of the plants.
  It cycles through its lines every 20 seconds and crossfades between them.
- **One message at a time.** Each state of the terrarium quiets the composer's own sky and
  ground decoration so its main signal stands out. Examples: no sun or drifting clouds over a
  dormant or dead terrarium, and no decorative tufts while it struggles. Nothing the terrarium
  reports is ever hidden.

## [0.7.0] - 2026-10-01

The terrarium's plants are now composed from the state truth hands over, though the strip still
shows the sample scene until the live terrarium reaches it. Seven preset scenes show the result.

### Added

- **The composer** places a terrarium's plants on thirty slots along the strip, in far, mid and
  near bands that step tall, medium, small.
  - Each third of the strip holds at most two big trees, never two of one species.
  - No two neighbouring plants of a band are the same species.
  - Between 35% and 70% of the slots hold something. When there are too few plants, the gaps
    fill to suit the scene: grass tufts in a living terrarium, the terrarium's own species
    dormant or dead in a dormant or dead one, or dead-stem litter. A healthy plant is never
    invented.
  - When there are more plants than the slots should hold, the mature ones are shown first.
  - Each install shifts the slots a few pixels, so two installs look a little different and
    never disagree.
- **Every change fades over seconds.** A plant fades in where it arrives and out where it
  leaves. A new look fades in over the old one before the old one goes, so the sky never shows
  through a change. A plant turning pale changes only its leaves, never its stem.
- **`cargo xtask render --preset <name> out.png`** writes a preset scene at 1× and 3×. The
  presets are `thriving`, `struggling-low-light`, `struggling-poor-soil`, `mature`, `dormant`,
  `dead` and `recovering`.

## [0.6.0] - 2026-10-01

The strip now shows the delivered art: a sample scene of a few plants on the soil band, inside
the glass edges, with a stone label reading `repo-terrarium`. It is still static; the living
terrarium comes next.

### Added

- **The compositor.** Every frame is painted from the art on the CPU at 640×74, then scaled by
  a whole number:
  - normal, multiply and add blending at any opacity, in the palette's own colours, never
    linearized;
  - day and night palettes, and any crossfade between them;
  - mirroring, the sway shear, cropping, and tinted glyphs;
  - darkening of the whole frame, for the dim mode to come.
- **Text** in both delivered fonts, with `?` for characters the fonts lack, and the label
  plate, which holds 18 characters and ends a longer line in `~`.
- **Panels** built from the nine-slice tiles, at any size from 9×9 up.
- **`cargo xtask render --demo out.png`** writes the sample scene as `out.png` and, at 3×,
  `out@3x.png`.
- **`cargo xtask bench`** also holds compositing a busy frame to under 1 ms.

### Changed

- **The strip's sides** continue the glass and the soil to the monitor's edges, taken from the
  art instead of fixed colours.

## [0.5.0] - 2026-09-30

Nothing changes on screen. This release checks the delivered art and packs it into the form
the picture will be drawn from.

### Added

- **`cargo xtask assets` checks the art package** and refuses it on any failure:
  - every file matches its SHA-256 in `PACKAGE-CONTENTS.json`;
  - only the listed manifests load, with their counts and revisions;
  - every ID is unique;
  - every sprite's pixels match its atlas;
  - no pixel falls outside the biome palette;
  - every animation has frames and durations;
  - every creature routine hands off to a routine that exists, or loops, or holds;
  - every plant, resident, sky, ground, glass and event state the picture can be asked for
    has art.
- **The runtime atlas and catalog.** The art is repacked into one atlas of palette indices,
  with a catalog of each sprite's metadata, and both are built into the app. Identical images
  are stored once, and every sprite keeps its own entry.
- **`cargo xtask verify` gains an `assets` stage.** It fails when the committed atlas or
  catalog is not exactly what the art produces.
- **`cargo xtask bench`** runs the release-build budgets. The first is loading the catalog in
  under 20 ms.

## [0.4.0] - 2026-09-30

Nothing changes on screen. This release lays truth's foundations, the deterministic core that
later decides what the terrarium shows.

### Added

- **Fixed-point numbers** (`Q16`, `Q32`). Every operation saturates, and every rounding goes
  half away from zero. They include `log2`, a moving average and `clamp01`.
- **Randomness:** `PCG32` seeded through `SplitMix64`, with unbiased bounded draws and one
  stream per purpose, so a new use of randomness never shifts an old one.
- **Time and identity:** the one-hour UTC tick grid, the repository seed and `SIM_VERSION` 1.
- **The domain types:**
  - readings, where unknown is a state and never a number;
  - climate and habitat;
  - the facts read from each commit;
  - the six observation kinds;
  - the presentation classes.
- **`SceneInput`, the frozen contract between truth and the picture.**
  - Its names follow the art.
  - It adds a list of ground marks that outlast their event: the settled crack, the settled
    sinkhole, and fresh soil.
- **The canonical text dump,** truth's golden format, with its SHA-256, and postcard encoding
  for the cache. Both round-trip, and both are pinned by tests.
- **Stronger purity walls in truth.**
  - Any `as` cast, clock type, std hasher or unstable sort is refused.
  - `cargo xtask verify` now scans truth's sources for floats that clippy cannot see.

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
