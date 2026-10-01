# Changelog

What shipped, per version. The format follows [Keep a Changelog](https://keepachangelog.com/).
Each step of development is a minor version, `0.<n>.0`, until the first public release.

## [Unreleased]

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
