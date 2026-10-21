# Architecture

How Repo Terrarium is built: its crates, how data moves through them, the rules the code keeps
and what enforces them, and a baseline of counts to check a change against.

## The crates

The workspace has eight members. Which of them may depend on which is fixed, and `cargo xtask
verify` checks it, so truth, for one, cannot reach code that does IO.

| Crate | Owns | May depend on |
|---|---|---|
| `terrarium-truth` | fixed point (`Q16`, `Q32`), PCG32, the hour tick grid, domain types, climate and habitat, the presentation timeline, the sim (vegetation, fauna, landmarks), events and arbitration, replay, `project() -> SceneInput`, `SIM_VERSION` | nothing in the workspace; serde, postcard, sha2 |
| `terrarium-adapters` | the gix reader and `CommitFacts`; lcov, Cobertura, JUnit and Knip parsers; tree-sitter metrics; discovery; the GitHub Actions client; `exec`, the one place a process starts; the fixture repositories (feature `fixture`) | truth |
| `terrarium-store` | the SQLite schema, observations, the sim cache, settings, the data folder, JSONL export and import, the pinned identifiers | truth |
| `terrarium-engine` | the headless live pipeline: worker thread, watcher, schedule, refresh, replay and cache, catch-up, CI poller, Knip runner, lifecycle changes, panic containment | truth, adapters, store |
| `terrarium-compose` | the asset catalog, the composer (slots, tweens, motion, events, label, soil panel), the CPU compositor, bitmap text, nine-slice, GIF encoding, `InstallSalt` | truth |
| `terrarium-platform` | everything that needs `unsafe`: the Windows AppBar and window subclass, fullscreen detection, the macOS window level and Spaces, the Core Animation layer, idle time, login items, power and display notices | nothing in the workspace |
| `terrarium-app` | the binary `repo-terrarium`: the winit event loop, presenters, the egui windows, hotkeys, menu, CLI, probes, and `copy`, which holds every word the user reads | all of the above |
| `xtask` | `verify`, `assets`, `golden`, `bench`, `size`, `soak`, `package` and the other development tasks | anything |

Each crate has a README that walks through its modules.

## How it runs

```
                 ┌──────────── terrarium-engine worker thread ─────────────┐
 fs watcher ───► │ debounce 500 ms → adapters → store.append(observations)  │
 CI poll 60 s ─► │ → replay (resumed | full) → SimState(t) → project()      │
 hour timer ───► │ → SceneInput snapshot + readings ──────────────┐         │
                 └────────────────────────────────────────────────┼─────────┘
                                                                  │ EventLoopProxy
 main thread (winit ApplicationHandler) ◄─────────────────────────┘
   SceneInput + OverlayInput + local clock + InstallSalt
     → composer → Frame → compositor → RGBA 640×74
     → presenter (integer upscale) ; egui windows on demand
```

**The engine** (`terrarium-engine`, `worker.rs`, `live/`). `Engine::spawn` starts the worker and
waits for its connect: open the store, create or relink the terrarium by its seed, load the
observations, and accept the `sim_cache` row only if its `SIM_VERSION`, seed, default branch and
tick all match. Its projection is the first scene shown. The worker then waits on one channel
until the schedule says to refresh: a change seen by the watcher (500 ms quiet, at most every
5 s in a burst), a new UTC hour, a wake, a CI or Knip result, or a request from the app.

A refresh reads HEAD and any new history, records a `ref_rewrite` when the default tip moved
without fast-forwarding, measures the source once per default tip, reads reports that have been
still for 500 ms and records those whose content changed, and replays. Replay resumes from the
cached state when truth's own digest of what it consumed says nothing before the cached tick
changed; otherwise it replays in full. The cache row is rewritten after a full replay and about
once an hour. After a sleep or a long absence, truth applies every hour of the gap, and the
strip plays the strongest event it has not shown, once.

If the worker panics, the panic is caught and the pipeline dropped; the strip's scene freezes
and its label reads `halted`. A repository that can no longer be read (moved, emptied, another
seed) stops the watcher and CI, sends `RepoNotFound` once, and is retried every minute.

The CI poller and the Knip runner are threads of their own. The poller asks api.github.com for
HEAD's branch and then the default branch, every 60 s, backing off on errors and rate limits,
and reads the keychain only on its first ask and after the token changes.

**The strip** (`terrarium-app`, `strip.rs`, `animate.rs`, `present.rs`). The event loop sleeps in
`ControlFlow::Wait`. The engine's updates arrive through `EventLoopProxy`; a changed scene goes
to the composer, which keeps all depiction state (slots, tweens, animation, resident positions)
and reports when its picture next changes. A redraw composes, composites only the damaged area,
and presents; the loop then waits until the composer's next change, the raise ending, or an
egui deadline. Frames come at most every 67 ms (15 fps), and none while the strip is covered or
nothing changes.

There are three presenters. On macOS the default is a Core Animation layer showing the native
image, which the window server scales nearest-pixel. On Windows it is softbuffer, upscaled on
the CPU with damage rectangles. A wgpu presenter exists and shares one device with the egui
windows, but is chosen only by the soak probe (`--presenter wgpu`).

**The windows.** Connect, Settings, Time-lapse and, in `dev` builds, Debug, at most one of each.
They are egui on egui-winit and egui-wgpu (not eframe), created when the first one opens and
dropped, device and all, when the last one closes. Every dialog is the first of two steps, so a
test drives the step after it directly.

**One instance.** The app holds `instance.lock` in its data folder. A second launch writes
`requests/show-settings` and exits; `--quit` writes `requests/quit` and waits up to a minute for
the lock. `--demo` and the probes take no lock.

**The command line** (`cli.rs`): the strip (`--repo <path>` to connect one), `--version`,
`--demo`, `--quit`, `--probe pinning | soak`, `--probe-readme`, `--package-copy`,
`replay <path> (--dump | --gif | --mp4)`, `discover <path>` and `record-tests -- <cmd>`.

## Data

**The data folder** is the OS app-data folder for the bundle id
(`~/Library/Application Support/io.github.faircat-gd.repo-terrarium`,
`%LOCALAPPDATA%\io.github.faircat-gd.repo-terrarium`), or `TERRARIUM_DATA_DIR`, which every
test and harness must set. It holds `terrarium.sqlite3` (WAL), `logs/` (the opt-in crash log,
newest ten), `local-tests/<seed>.json` (what `record-tests` recorded), `instance.lock` and
`requests/`.

**The database** has five STRICT tables: `settings`, `install`, `terrariums`, `observations`
and `sim_cache`. Triggers refuse every update and delete of an observation, except the one
guarded path that deletes a terrarium along with its observations.

**Observations** are the only record of what the tools said: `coverage`, `dead_code`,
`complexity`, `todo_density`, `weather` (with its source, `ci:<branch>` or `local`) and
`ref_rewrite`. Each records the commit and branch HEAD was at and whether that was the default
tip. Git history is never stored; it is read again and replayed.

**Export and import:** observations as JSONL (a header naming the format, the seed and
`SIM_VERSION`, then one row per line), merged on import by kind, source, commit and time; the
climate as JSON; the time-lapse as GIF, MP4 (through the user's ffmpeg) or a PNG postcard.

**Identifiers that never change**, pinned by a test in `terrarium-store/src/identifiers.rs`:
the bundle id and keychain service `io.github.faircat-gd.repo-terrarium`, the binary
`repo-terrarium`, the crate prefix `terrarium-`, `terrarium.sqlite3`, the table and trigger
names, the settings keys, and the Windows install folder, uninstall key and login value.

## Invariants

| # | Rule | Enforced by |
|---|---|---|
| I-1 | Truth is a pure function of git history, recorded observations, the seed, the default branch and `SIM_VERSION`: no IO, wall clock, floats, hash-order iteration, salt or local time. | The graph check (truth depends on nothing in the workspace; serde, postcard, sha2 outside it); xtask's scan of truth's sources (`purity.rs`: no floats, no `std::fs`, `env`, `net`, `process`, `thread` or `io`, no printing); `float_arithmetic`, `as_conversions` and the disallowed types and methods in `crates/terrarium-truth/clippy.toml`; the golden replay; the salt and input-order tests. |
| I-2 | The install salt changes only layout, equivalent art and animation phase. | `InstallSalt` lives in compose; `the_salt_moves_slots_and_nothing_else` and engine's `neither_import_order_nor_salt_changes_the_dump`. |
| I-3 | A missing reading counts as middling, never as zero, and an old reading is never taken as worse than it was. | `Reading` has no `Default` (a `compile_fail` doctest); truth's property tests (an unknown reading never lowers health below neutral, a stale one keeps its value); unusable reports are skipped, never recorded. |
| I-4 | Observations are append-only; replay never runs a tool. | The SQLite triggers and their tests; replay takes only `CommitFacts` and stored rows. That only the engine's refresh calls adapters is held by the graph and review. |
| I-5 | Nothing is written into the repository, and nothing runs from it without its opt-in. | gix opened read-only and isolated; `process::Command` allowed only in `adapters/src/exec.rs` and `git` never run (graph check); the Knip opt-in check; `reading_never_writes_into_the_repository` and `a_live_session_never_writes_into_the_repository`. Knip's own cache in `node_modules` is the one exemption. |
| I-6 | No network except the configured GitHub Actions poll; no telemetry. | Only adapters may depend on `ureq`, and only `adapters/src/ci/` may name it or a socket (graph check); `engine/tests/n09.rs` runs every feature with the network stood in by a counter. |
| I-7 | The composer never fabricates health. | `every_constraint_holds_through_generated_histories` (subset, placement, one message, continuity) and `values_and_palette_hold_on_rendered_frames`. |
| I-8 | The world never talks: no praise, shame, nagging, instructions, notifications or `!`. | Every user-facing word lives in `app/src/copy.rs` (and `copy/probe.rs`), whose tests ban `!` and a word list. No notification API is linked, by review. |
| I-9 | `unsafe` only in `terrarium-platform`, each block with a `SAFETY:` comment. | `#![forbid(unsafe_code)]` required at every other target root (graph check); platform's root denies `undocumented_unsafe_blocks` (required) and `multiple_unsafe_ops_per_block`. |
| I-10 | Frames only when pixels change, at most 15 fps. | The pacer and frame-gate tests in compose and the app; the soak's busiest-second check. The anti-burn-in steps (N-12) are the only frames a still scene makes. |
| I-11 | Art is used as delivered, at integer scale with nearest sampling. | `xtask assets --check`; exact pixel snapshots; the integer `StripScale` and nearest sampling in all three presenters. |
| I-12 | Machine identifiers, table names and applied migrations never change. | `identifiers_are_pinned`, `applied_migrations_are_pinned`, `the_schema_has_the_pinned_tables_and_triggers_all_strict`. |
| I-13 | A change to truth's output bumps `SIM_VERSION`. | `xtask golden --check` compares `tests/golden/` byte for byte; `--bless` refuses a changed dump at the same version. |
| I-14 | Tests never read `docs/` or the art's references, and never touch a real data folder. | The data folder resolves only from `TERRARIUM_DATA_DIR` under test or nextest (`under_test_the_real_directory_is_never_resolved`); `assets` skips `references/`; the rest by review. |
| I-15 | Every visible change tweens; no cuts; the camera never moves. | The composer's continuity check, the day-night and pause tests, and the app's re-seed and wake tests. |
| I-16 | A panic never crashes the app. | The worker's `catch_unwind` and its four tests; a composer panic freezes the picture too (`a_composer_panic_freezes_the_picture_and_the_label_crosses_to_halted`). |

## Budgets

| ID | Budget | Where it stands |
|---|---|---|
| N-01 | CPU under 1% of a core, an hour's average | 0.69% over an hour on Apple silicon (`xtask soak`), and 0.80% over ten minutes with today's plant count. Windows unmeasured. |
| N-02 | Memory under 80 MB | About 20 MB settled and 48 MB at peak on Apple silicon. Windows unmeasured. |
| N-03 | At most 15 fps; none when still | The pacer, the frame gate and the soak. |
| N-04 | Readings updated within 60 ms of the adapters | `engine/tests/live.rs`. |
| N-05 | First connect of 10,000 commits under 30 s | `xtask bench`. |
| N-06 | One refresh per commit; CI every 60 s | The schedule's tests; the poller. |
| N-07 | Identical truth on every OS | The golden replay: matched on Windows by the release workflow at `SIM_VERSION` 10, and on Linux at 5. |
| N-08 | Each binary under 25,000,000 bytes | `xtask size`, and `package` for what it ships. |
| N-09 | No network but the CI poll | I-6. |
| N-10 | Never writes into the repository | I-5. |
| N-11 | Any state back to thriving within 60 days of good climate | Truth's property test. |
| N-12 | No region of the strip static for an hour | Every 8×8 block changes within the hour (`compose/tests/schedule.rs`), except while paused. |
| N-13 | Readable at 1.5 m | Not measured. |

## Pins

The toolchain is pinned in `rust-toolchain.toml` (1.98.1, with the Windows target), the minimum
Rust in `[workspace.package]` (1.95), and every dependency in the workspace manifest, which
says why each is there. gix, insta, keyring and keyring-core are pinned exactly. A pin moves
only on purpose, never in passing; a gix bump can change truth.

## Verification

`cargo xtask verify` is the gate before every commit. It cleans `target/` first when it holds
more than 30 GiB, then runs its 13 stages, each even after one fails:

1. fmt;
2. clippy, default features;
3. clippy, all features (`dev`);
4. clippy for `x86_64-pc-windows-msvc`, its C built against the xwin splat;
5. clippy for what ships, the app and adapters;
6. the same for Windows;
7. nextest, all features;
8. nextest, the app as released;
9. doctests;
10. cargo-deny;
11. the crate-graph check;
12. `assets --check`;
13. `golden --check`.

The heavier harnesses run when a change could move what they measure: `bench` (the release
budgets), `size` (N-08), `soak` (N-01 to N-03), `gallery`, `arc` and `hero` (pictures that are
the same bytes every run), and the probes. A new check is trusted only after it has been broken
and seen to fail at its own step.

## Packaging and release

`cargo xtask package` builds the app once as it ships (paths remapped so no build-machine folder
is left in it, macOS 13 as the floor, Windows' C runtime static and its resources linked), checks
each binary, and makes:

- `repo-terrarium-<version>-macos-arm64.dmg`: the app sealed ad hoc beside a link to
  Applications, in a window with its own picture. xtask writes the window's `.DS_Store`
  and alias itself and checks the finished DMG;
- `repo-terrarium-<version>-windows-x64-setup.exe`: a quiet per-user NSIS installer whose
  uninstaller quits the strip and removes its login item. Both keep the data folder.

`.github/workflows/release.yml` runs only when started by hand: it packages on a Mac and on
Windows, runs `install-test` and the golden replay on Windows, and drafts a GitHub Release.

## Baseline

| At 0.32.0 | Count |
|---|---|
| Tests (nextest, all features) | 1,373 passed, 0 skipped |
| per crate | adapters 162, app 285, compose 321, engine 140, platform 76, store 53, truth 174, xtask 162 |
| The app as released (verify's second nextest) | 271 passed |
| Doctests | 2 |
| Pixel snapshots | 81 pictures (compose 69, app 12), each a PNG and a hash of its raw pixels, and 2 hash-only snapshots (the time-lapse GIF, the climate export): 164 `.snap` files |
| Golden replay | `tests/golden/arc.dump`, 6,446 lines at `SIM_VERSION` 10, SHA-256 `54d27544e6066a765c1de7d427dd33f8aca5d804ae756fd8dd98ca35dcd8a971` |
| Atlas | 1024×1183, 1,211,392 bytes; a palette of 72 slots |
| Catalog | 778 sprites (160 of them glyphs) over 661 distinct images, 114 animations, 2 fonts, 10 event groups |
| verify | 13 stages; 8 workspace members |

| Bench (`cargo xtask bench`) | Measured | Budget |
|---|---|---|
| Loading the catalog, cold | 2.46 ms | 20 ms |
| Compositing a busy frame (278 draws), median | 0.240 ms | 1 ms |
| Reading 10,000 commits | 3.66 s | 10 s |
| Measuring 107,100 lines of source; one changed file | 211 ms; 0.57 ms | 1 s; 50 ms |
| First-connect replay of 10,003 commits (N-05) | 3.59 s, the replay itself 6.65 ms | 30 s |
| Live: first connect and refresh | 3.81 s | 30 s |
| Live: connect again from the cache; its first refresh | under 1 ms; 3.62 s | 250 ms; 5 s |
| Live: a refresh after a commit; after a tag on an old commit | 0.361 s; 0.255 s | 1 s; 2 s |

| Size (`cargo xtask size`, N-08) | Bytes |
|---|---|
| macOS, Apple silicon | 22,607,648 |
| Windows x64, cross-built on the Mac | 24,696,832 (303 KB under the budget) |
