# Changelog

What shipped, per version. The format follows [Keep a Changelog](https://keepachangelog.com/).
Each step of development is a minor version, `0.<n>.0`, until the first public release.

## [Unreleased]

## [0.26.0] - 2026-10-12

Soil now reads the code itself: complexity and TODO density come from the code committed on the
default branch, and a Knip report becomes dead code.

### Added

- **Complexity and TODO density** are measured from the TypeScript, TSX, JavaScript, Python,
  Rust and Go committed at the default branch's tip, while it is checked out, with no report
  and nothing run. Complexity is the functions' mean cyclomatic complexity; TODO density is
  `TODO`, `FIXME` and `HACK` in comments per 1,000 lines of code. Each new tip is measured once,
  and only the files it changed are read again. The soil panel names the files with the most
  decisions and the most markers.
- **Dead code** from a Knip report (`knip-report.json`, or the path set in Settings): its unused
  exports and unused files, weighed against the exports counted in the committed code. The
  report is watched like the others.
- **Complexity** and **TODO density** switches in Settings, under Adapters, on by default.
- The discovery report (`discover`, the connect window, Settings) lists the source files at the
  default tip per language.
- `cargo xtask size`: the release binary must stay under 25 MB.

### Changed

- The soil panel says why a soil cause lists no file: off in settings, no Knip report found,
  none read yet, or no code in the languages read.
- The discovery report's Knip line no longer says its counts are not part of soil.

## [0.25.0] - 2026-10-12

A terrarium's history moves to another computer, its climate can be read as data, and the
time-lapse saves as an MP4.

### Added

- **Export observations…** in Settings, under Repository, saves the light, soil and weather
  recorded here as a JSON Lines file.
- **Import observations…** reads such a file into the same repository's terrarium, for
  example on another computer. It asks first, since an import cannot be undone. A file from
  another repository is refused and nothing is written. Afterwards the window says how many
  observations were added and how many were already here, and lists any that differ from the
  ones recorded here, which are kept. The history then replays with them.
- **Export climate…** saves each finished day's climate (light, soil and its causes, rain,
  wind, health, activity, terrain and weather) and every change of the terrarium's state, as
  JSON. The format is described in the app's README.
- **Save MP4…** in the time-lapse window writes the time-lapse as an MP4 through your own
  ffmpeg, at 4× or the scale chosen if larger. It shows when ffmpeg is found on PATH or, on
  macOS, where Homebrew and MacPorts put it; otherwise a line says where it was looked for.
- `replay <path> --mp4 <file>` writes the same MP4 from the command line.

### Changed

- **An import is faster:** each new observation is written straight away, and only one
  already recorded is looked up.

## [0.24.0] - 2026-10-11

The terrarium's whole history can be watched in 30 seconds, and saved as a GIF.

### Added

- **The time-lapse window** (right-click the strip, Time-lapse, or Control-Option-H; Ctrl+Alt+H
  on Windows): the repository's history replayed from its first commit, or over a range of
  days, in 30 seconds.
  - The strip plays at a smaller scale, with Play and Stop and a scrubber that names the day.
  - Everything moves eight times faster: plants and weather change in half a second, storms
    and flowerings play in miniature, clouds race and the creatures scurry.
  - The label shows the date: the day for a range of four months or less, the month for a
    longer one.
  - From and To choose the range; By day and By night choose the look.
  - Days before the terrarium was connected show what git alone says, under neutral light and
    soil.
  - The strip itself is untouched while the window plays, and a paused strip stays paused.
- **Save GIF…** writes the range as a 30-second GIF that repeats forever, at 1× to 4× (or the
  strip's own scale). The same range always makes the same file.
- **Save postcard…** writes the day shown as a PNG, settled, at the same scales.
- `replay <path> --gif <file>` writes a repository's whole history as the same GIF from the
  command line, with `--night` and `--scale <n>`.

### Changed

- **The time-lapse hotkey** set in Settings now opens the time-lapse, and the menu's
  Time-lapse item is on while a repository is connected.

## [0.23.0] - 2026-10-11

Everything the settings promised is in them, and the strip can be paused.

### Added

- **Settings has six tabs:**
  - **Repository** works as before.
  - **Strip** chooses the display the strip lies on (or leaves the choice to it), the top or
    the bottom edge, and the largest scale that fits or one step smaller, each showing its
    scale. A display chosen and then disconnected is kept: the strip comes back to it when it
    returns.
  - **Adapters** shows what the repository has and how each missing report connects. Each of
    the coverage report, the test results and the dead-code report can be turned off, and can
    be read from a path of its own inside the repository. A change is read at once. Running
    Knip and the GitHub Actions weather are shown, off, until a later version brings them.
  - **Hotkeys** sets the keys that raise the strip and that will open the time-lapse: two
    modifiers or more, at least one of them Control or Command. Keys macOS refuses, or that
    the other hotkey has, are refused with the reason.
  - **Time** sets when day and night begin, to the quarter hour, and how long without input
    before the strip dims, from 5 to 120 minutes.
  - **General** turns opening at login and a crash log on or off, and shows the version.
- **Pause** (right-click the strip): the plants, residents, sky and events stand still while
  the repository's history keeps counting, and the label reads `paused`. The soil panel and
  the hover labels still work, naming what the still picture shows. Resume brings the picture
  to the present with its usual tweens. Dim mode still dims a paused strip. A launch always
  starts running.
- **A crash log,** off unless turned on in General. A crash then leaves a report in the app's
  own data folder, the newest ten kept, to attach to an issue. Nothing is sent anywhere.
- **The right-click menu** reads Settings, Time-lapse (off until the time-lapse window
  exists), Pause or Resume, and Quit.

### Changed

- **Of several coverage or test reports in the usual places, the newest is read.** Before, a
  stale `coverage/lcov.info` was read over a fresh `lcov.info`.
- **`discover`** reads the connected repository with the report paths its settings hold, and
  never makes a data folder that is not there.
- **A turned-off adapter's page in the soil panel** says it is off in settings, rather than
  that no report was found.

## [0.22.0] - 2026-10-10

The soil can be read: hovering it raises a panel with the terrarium's numbers, and the stones
and fossils say when they came.

### Added

- **The soil panel.** Hovering the soil band raises a panel out of the soil, centred where the
  pointer came in, and moving off it and the soil lowers it again. It lists light, soil, rain,
  wind, weather, terrain and age, each with its value, which way it moved over the week, and a
  mark when the reading is stale or unknown.
  - A click pins the panel, so it stays when the pointer leaves; a click off it closes it.
  - Light opens the files with the most uncovered lines, weather the files with failing tests,
    and soil its three causes (dead code, complexity, TODO density), each of which opens its
    files: the top 20, their paths cut at the start so the file's name shows. The title of a
    lower page goes back up.
  - A page with nothing to list says why: no coverage report found, no failing tests, and so
    on.
  - On a branch other than the default, light's page shows that branch's own coverage beside
    the files, without applying it.
  - Dim mode closes the panel, pinned or not, and any hover label.
- **Hover labels.** A standing stone shows the year it was raised, on a plate over the stone;
  a fossil shows the folder it was and the year it was deleted.
- **The label's notes** gain `settling N days` (the first two weeks of a repository's history)
  and `dormant since <month>` (or the year, after eleven months). A note shown once, such as
  `sim updated`, takes its turn before them.

### Changed

- **Terrain has a trend:** the panel shows whether the code grew or shrank by 2% or more over
  the week. The simulation's version is 8: every terrarium replays once, and the label says
  `sim updated`.

## [0.21.0] - 2026-10-10

A repository is connected from a window, and the terrarium's life around it is in reach: open
at login, one app at a time, relinking a moved repository, a new terrarium, and disconnecting.

### Added

- **A connect window** opens when no repository is connected, instead of the sample terrarium:
  choose the repository's folder, then connect it. The first time, "Open at login" is offered,
  checked; it is not offered again. Once connected, the strip appears and the window shows what
  was found in the repository and what was not, once, ending with where light, soil and weather
  are kept. Closing the window with nothing connected quits.
- **Open at login** registers the installed app as a login item on macOS and adds it to the
  Windows Run key. A build run from the command line is not an installed app; it says so and
  registers nothing.
- **One app at a time.** Launching it again while it runs shows the running one's settings
  (or its connect window); `--repo` with one running connects nothing and says so.
- **Settings** (right-click the strip, or launch again) show the connected repository's name,
  folder and default branch, and three changes, none made by a single press:
  - **Locate and relink**, while the repository is not where it was connected from: choose its
    new folder, then relink. Only the same repository relinks; its history is kept.
  - **New terrarium** lays the same history out anew: plants, residents and stones take new
    places, dissolving from the old layout over a few seconds. What the terrarium shows of the
    repository stays as it is.
  - **Disconnect**, keeping the recorded observations unless asked to delete them, and saying
    that light, soil and weather history would be lost. The strip closes and the connect
    window opens.
- **The label notes a history rewrite once:** `history rewritten`, for one turn of its cycle.
- **The discovery report** (`discover`, and the connect window) ends with a sentence on where
  light, soil and weather are kept: on this machine only.

### Changed

- **`sim updated`** shows for one turn of the label's cycle, not the whole session.
- **A force push is recorded as git read it:** which commits it took off the default branch,
  and which of them deleted whole directories; the terrarium decides from those which fossils
  the push took. A push made while the app was closed is read back the same way at the next
  start, so its earthquake has its true strength. The simulation's version is 7: every
  terrarium replays once.
- **A restart within an event's hour** no longer plays the event again.

### Fixed

- A fast-forward of the default branch through a merge, made while the app was closed, no
  longer shakes the terrarium as if it were a force push.

## [0.20.0] - 2026-10-09

The ground's events play, the marks they leave stay as long as truth keeps them, and the arc
and the real sim can be watched.

### Added

- **A clearing burn** burns off every weed in its frames, the weeds giving way and fresh soil
  coming in step by step as the art keys them; when the ash clears, a few flowers come up on the
  fresh soil for a moment. The fresh soil stays two weeks.
- **Lightning scorches the plant it strikes:** after the bolt the scorched foliage rises over
  the plant, holds, and gives back whatever foliage the plant has by then; it never heals the
  plant by itself. A bloom and the night glow hide while it shows.
- **A release flowers across the strip:** each plant that blooms opens in turn from one side,
  over about a minute, and stays in bloom for the day.
- **An earthquake** opens a crack in three frames into the settled crack, which stays until the
  next rain after a drought or for 30 days; residents hide for a moment, then come out. A
  rewrite that takes a fossil opens its sinkhole with the crack.
- **A sinkhole** opens over the fossil it reveals, as near the plant it swallowed as a fossil's
  place allows, and stays as the crack does.
- **Marks in the ground** (the crack, the sinkhole, fresh soil) now show whenever the
  repository's history leaves them, fading in and out like the rest of the ground.
- **`cargo xtask arc`** replays the fixture arc a day at a time and writes a picture, by day and
  by night, of each change of state, each day an event plays on the ground and the last day,
  with a contact sheet and a key, to `target/arc/`.
- **The debug window's Simulation** (dev builds): sliders for light, soil, rain, wind and
  weather drive the real sim a day at a time, at up to 30 days a second, with buttons to tag a
  release, delete a directory, rewrite the default branch, or start over.

## [0.19.0] - 2026-10-09

The sky's events play: storms roll in and clear, lightning strikes, rain falls after a drought,
and a windstorm carries leaves across.

### Added

- **A storm rolls in from its side.** When the tests start failing, the storm front crosses the
  strip a pixel at a time, from the left or the right, the dark sky behind it and the old sky
  ahead, which holds still until it is covered. When they pass again, the clearing comes in
  from the same side and carries the storm on out the far side. A crossing takes 45 to 58
  seconds, by the strip's width.
- **Lightning strikes** the plant it scorches, in two frames, or the ground where that plant
  is not drawn.
- **Rain after drought** falls across the whole strip, fading in, holding for longer the
  longer the drought, and fading out. Its streaks are drawn in the stone's light colour at
  150, so they read against the pale daytime sky.
- **A windstorm carries leaves** across from its side, each at its own pace and height.
- **Dusk and sunburst** settle over the scene as their own light.
- **The plants sway harder while a storm or a windstorm is under way,** from the next turn of
  motion.
- **Each event plays once and one at a time,** however long the repository's history keeps it
  current; one that arrives while another plays is passed over, and its weather or light
  arrives as usual.
- **The debug window's event triggers** (dev builds): each press plays its event again, with
  the weather or light it brings.

### Changed

- **The strip draws fewer frames** while the light or a glass film fades, and none for a cloud
  the storm has covered: only when something on screen changes.

## [0.18.2] - 2026-10-09

A repository coming back from a long silence now reads at its own pace, and a cleanup of the
whole codebase.

### Changed

- **A return after a long silence reads at the repository's own pace.**
  - The rain and wind no longer measure it against the silence itself: the days past two
    silent weeks leave the repository's usual pace out of it, and the first commit day back
    starts the reckoning afresh.
  - The weeks after a long break no longer read as a gale or rush every plant back at once.
  - A steady or sparse rhythm, such as a commit or two a week, reads exactly as before.
- **`cargo xtask verify` lints the shipped code on its own,** for macOS and Windows: the app
  and the adapters as a release builds them, without the test fixtures. It has 13 stages.
- **`cargo xtask gallery` and `probe-zip` write under `CARGO_TARGET_DIR`** when it is set, as
  cargo does.
- The terrarium's history replays once more after this update (simulation version 6).

### Fixed

- **A bare repository records its `record-tests` runs:** the status file lives in the data
  directory, so a repository without a working tree has one too.
- **A refresh that finds itself without its history or its scene is reported and tried again.**
  Before, the engine stopped there silently, and the strip froze with no note. Any refresh
  that ends that way when nothing stopped the engine now halts it and the label says so.
- **`--version` and `--probe-readme` no longer fail** when the reader closes early, as `head`
  does.

## [0.18.0] - 2026-10-05

Things now happen in the terrarium: the repository's moments arrive as events, one at a time.

### Added

- **Eleven events**, each answering something real:
  - **storm** when the tests start failing, **clearing** when they pass again, leaving by the
    side the storm came in;
  - **lightning** on each day the tests stay red, from the third: it scorches one mature
    plant and dims it a little, never to death, and never while the repository sleeps;
  - **flowering** for a release tag such as `v1.2.0`: thriving mature plants bloom for a day;
  - **carved marks:** each new major version (v1, v2, …) is carved on the newest standing
    stone with room, up to three a stone; a release before the first stone waits for it;
  - **clearing burn** when a fifth of the unused code (at least five items) is cleared: the
    weeds burn and fresh soil shows for two weeks;
  - **dusk** and **sunburst** when coverage falls or rises by two points or more;
  - **windstorm** on a day with at least three times the repository's usual changed lines,
    blowing in leaf litter;
  - **rain after drought:** the first commit after two silent weeks washes the settled marks
    away and sprouts more seedlings at the next midnight;
  - **sinkhole** when a commit deletes a whole directory of 500 lines or more: it swallows one
    plant and leaves its fossil;
  - **earthquake** when the default branch is force-pushed: the ground cracks, and a fossil
    whose commit the push removed sinks with it.
- **One event at a time:** when several happen together, the most important plays; after the
  computer sleeps, or the app was closed, the most important thing that happened meanwhile
  plays once, while everything else still takes effect.
- **Events stay rare:** each kind waits a while (an hour for a clearing, a week for rain after
  drought) before it can play again.
- **The settled crack and sinkhole** stay in the ground for up to 30 days.
- **A force push made while the app was closed** is noticed at the next start.

### Changed

- **The presentation no longer flickers** between busy and quiet when activity hovers at the
  threshold.
- **Fossils keep their identity** across a history rewrite: the ones that remain stay where
  they were.
- **`cargo xtask bench`** also replays 10,000 commits in full, events included, against the
  30-second first-connect budget.
- **`cargo xtask verify` cleans the build directory** when it grows past 30 GiB.
- The terrarium's history replays once more after this update (simulation version 5).

## [0.17.0] - 2026-10-05

The terrarium is inhabited: residents live out their lives in it, standing stones mark its
years, and fossils keep its large deletions.

### Added

- **A cast of its own:** each repository hosts 5 of the 8 resident species, picked from its
  first commit, so the same repository has the same cast on every computer. A bigger codebase
  holds more residents, from 1 up to 6.
- **Residents arrive once the plants are up,** one every couple of days at the repository's
  usual pace, the species not yet present first.
- **Lifecycles:** residents hatch from eggs (a brood for the woodlouse, a cocoon for the
  earthworm), grow as juveniles, pupate (the ladybird, the beetle and the firefly), and live
  as adults for about 3 to 12 months by species. An adult that has lived its days dies, and
  shows it for a day.
- **Eggs:** a well-fed adult lays one about every two weeks while there is room, as long as
  its kind is not already two.
- **Energy follows the plants:** residents are lively while the plants thrive, and slower as
  the plants falter. A resident that goes hungry for two weeks leaves, and comes back once the
  plants recover.
- **Decomposers live on decay:** the woodlouse, the springtail and the earthworm also feed on
  dead plants, leaf litter and fungus, so a dead terrarium can keep a few until the decay is
  gone. They never breed on it.
- **Dormancy:** while the repository rests, residents sleep and nothing ages or hatches; the
  first commit back wakes them.
- **Standing stones:** one rises for each full year since the repository's first commit, up to
  three, and each weathers from young to lichened at two years and ancient at five.
- **Fossils:** a commit on the main branch that deletes a whole directory of 500 lines or more
  leaves a fossil in the soil. A renamed or moved directory leaves none. Up to five are kept,
  the oldest leaving first.

### Changed

- **The terrarium is replayed once** after this update, because how it grows has changed; the
  label notes `sim updated` the first time.

## [0.16.0] - 2026-10-05

The plants are alive: they grow from your repository's history instead of standing in place.

### Added

- **A species pool of its own:** each repository grows 8 of the 12 plant species, two trees,
  three mid-height plants and three ground plants, picked from its first commit, so the same
  repository grows the same species on every computer.
- **Room to grow:** a bigger codebase holds more plants, from 6 up to 24. Seedlings sprout into
  free room while the repository is worked on, grow young and then mature over days and weeks,
  and keep their place for life.
- **Plants show the repository's health:** they thrive while health is fair or better, and
  falter when it falls. Pale plants mean too little light (test coverage); wilting plants mean
  poor soil (code health). A repository with no coverage or code-health tool grows thriving
  plants; a missing tool is never held against it.
- **The pace follows the rain:** a busy repository's plants respond within days, a quiet one's
  more slowly.
- **Plants die only when health stays poor:** a plant whose health is badly short (well past
  pale or wilting) for three weeks dies, stands for a month, then falls as leaf litter, which
  turns to fungus after two weeks and is gone two months later. Pale and wilting plants live on.
- **Weeds** move into free room while the soil is poor, and withdraw as it recovers.
- **Dormancy:** while the repository rests, its plants go dormant and nothing new sprouts; the
  first commit back wakes them, and seeds sprout again into the room the losses left.

### Changed

- **The terrarium is replayed once** after this update, because how it grows has changed; the
  label notes `sim updated` the first time.

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
