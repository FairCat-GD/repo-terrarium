# xtask

Development tasks, run as `cargo xtask <task>` from anywhere in the repository. Not shipped.
The tasks are `verify`, `arc`, `assets`, `bench`, `fixture`, `gallery`, `golden`, `probe-zip`
and `render`.

## `verify`

The gate every commit passes. It runs every stage, even after one fails, and ends with a
summary. The stages:

1. **fmt**: `cargo fmt --all --check`.
2. **clippy**: `cargo clippy --workspace --all-targets -- -D warnings`, the release build's
   features.
3. **clippy (dev)**: the same with `--all-features`: the debug window and its egui and wgpu.
4. **clippy (windows)**: `--all-features` for `x86_64-pc-windows-msvc`. There is no CI, so
   this is what compiles and lints the Windows code.
5. **clippy (shipped)**: the code that ships, as it is built:
   `cargo clippy --package terrarium-app --package terrarium-adapters -- -D warnings`. Every
   workspace stage compiles adapters with `fixture`, which xtask and the dev-dependencies turn
   on; with no tests and no xtask, nothing does here.
6. **clippy (shipped, windows)**: the same for `x86_64-pc-windows-msvc`, whose own code ships
   too.
7. **nextest**: `cargo nextest run --workspace --all-features`.
8. **nextest (release)**: `cargo nextest run --package terrarium-app`, the app's tests again
   without `dev`, for what only a release build does. The app is the only crate with features.
9. **doctest**: `cargo test --workspace --doc --all-features`, because nextest skips doctests.
10. **deny**: `cargo deny check`, with the policy in `deny.toml`, over every feature.
11. **graph**: the crate-graph check below, including the scan of truth's sources.
12. **assets**: `cargo xtask assets --check`, below. It runs in process, and a panic fails only
    this stage.
13. **golden**: `cargo xtask golden --check`, below, in process like `assets`.

It needs `cargo-nextest` 0.9.146 and `cargo-deny` 0.20.2, pinned in `src/verify.rs`. A stage whose
tool is missing, or at another version, fails and names the install command.

## The crate-graph check

`cargo metadata` gives the dependency graph, resolved with every feature for the shipping
targets listed in `deny.toml`'s `[graph] targets`, and `src/graph.rs` holds the policy as data :

- Each crate depends only on the workspace crates the policy allows, for every dependency kind.
  A new crate fails until it is added to the policy.
- `terrarium-truth` links no external crate but the four the policy allows: `serde`, `postcard`,
  `indexmap` and `sha2`. It uses all but `indexmap` today.
- `ureq` is reachable only through `terrarium-adapters`.
- Among the `src/` files of the crates that ship, only `terrarium-adapters`' `exec` module names
  `std::process::Command` (I-5). Integration tests, benches and xtask may start processes.
- Every target root outside `terrarium-platform` carries `#![forbid(unsafe_code)]`. Platform
  denies `clippy::undocumented_unsafe_blocks`, and truth denies `clippy::float_arithmetic`,
  `clippy::as_conversions`, `clippy::disallowed_types` and `clippy::disallowed_methods`.
- Truth is a single library target. A test, bench, example or build script would be a crate of
  its own, outside the lint walls on truth's root.
- Every manifest inherits the workspace package fields and lints.
- No member enables `dev` by default, directly or through another feature, and none turns it
  on in a workspace crate it depends on: the debug window never ships in release builds.
- `terrarium-adapters`' `fixture` feature, which writes git repositories, is never on by
  default and is turned on only by xtask or a dev-dependency, never named in another
  member's features: the fixture code never ships (I-5). Cargo unifies features across the
  packages a build selects, so this holds for builds of the app alone (`-p terrarium-app`,
  as `probe-zip` does and a test pins), never for `--workspace` release builds.
- No dependency declares a `rust-version` above the workspace's.

### The scan of truth's sources

Clippy cannot see every float: `float_arithmetic` skips `const` and `static` bodies, and a float
that is only compared or passed to a method (`2.0_f64.sqrt() > 1.0`) names no type and does no
arithmetic. `src/purity.rs` lexes every `.rs` file in `crates/terrarium-truth` with
`proc-macro2` and fails on:

- a float literal (`1.5`, `2.`, `1e3`, `1_f32`), though not a tuple index such as `pair.0.1`;
- the names `f16`, `f32`, `f64` and `f128`, including module paths like `core::f64::consts`
  and raw identifiers like `r#f64`;
- an `allow`, `expect` or `warn` of a wall lint (`float_arithmetic`, `as_conversions`,
  `disallowed_types`, `disallowed_methods`), of a group holding one (`all`, `style`,
  `restriction`), or of `warnings`;
- `include!` and `#[path]`, which would bring code in from outside the scan;
- any path through `std::fs`, `std::env`, `std::net`, `std::process`, `std::thread` or
  `std::io`, the items of a `use std::{…}` group among them, so an alias (`use std::env as e`)
  is refused at its `use`;
- an alias of `std` itself (`extern crate std as s`, `use std::{self as s}`), which would hide
  the paths under it;
- the printing macros (`print!`, `println!`, `eprint!`, `eprintln!`, `dbg!`), which write to
  the terminal without naming `std::io`.

It reads tokens, so comments, strings and char literals never trigger it.

## `arc`

`cargo xtask arc` replays the fixture arc a day at a time, as a strip looking once a day would
show it, and writes its key frames for review into `target/arc/`, replacing what an
earlier run left. One composer takes each day's
scene in turn and settles between days. A frame is kept at each change of the presentation
timeline, on the first day of each run of days an event plays on the ground, and on the last day:

- `frames/NN-day.png` and `NN-night.png`: each kept frame at 3×;
- `contact-sheet.png`: every kept frame at 1×, its day beside its night;
- `key.csv`: each frame's day of the arc, tick, presentation state, health and activity class,
  and the events played since the frame before;
- `SHA256SUMS`: every other file's hash. The same build writes the same bytes.

It fails unless the frames show all six presentation states. It is not part of `verify`; the
arc's six states are `terrarium-engine`'s own test.

## `gallery`

`cargo xtask gallery` writes the A-kit into `target/gallery/` (under
`CARGO_TARGET_DIR` when it is set, as cargo's own output is), replacing what an earlier run left:

- `states/NN-day.png` and `NN-night.png`: the synthetic climate's 30 states across health and
  activity, at 3×. `NN` is a fixed shuffle of the grid, so someone rating the pictures cannot
  read the answer from their order; `key.csv` holds it.
- `named/<name>-day.png`, `-night.png` and `-dim.png`: the named looks at 3×, Recovering among
  them.
- `contact-sheet.png`: the grid at 1×, health falling down the rows and activity rising along
  the columns, each cell's day above its night.
- `pieces/`: the background art on its own, for art review, at 3× by day and by night: each
  weather's backdrop, the soil, the glass ends, the label plate, the empty terrarium (also
  dimmed), and the empty terrarium in the 735 columns a 2940 px screen shows at 4× and in all
  840 the art covers. `contract.csv` gives each background sprite's size, anchor, place, layer,
  opacity by day and night, palette (shared or its own), slots and drift, read from the catalog.
- `SHA256SUMS`: the hash of every other file. The same build writes the same bytes every run,
  and a test renders it twice to hold that.

## `assets`

Validates the art package in `art-assets/`, then packs the runtime atlas and catalog that
`terrarium-compose` embeds. The package is read into memory once; a biome's `references/` and
Finder's `.DS_Store` files are never read (I-14). The checks:

1. **Package contents.** Every file's SHA-256 matches `PACKAGE-CONTENTS.json`, and every file
   is listed.
2. **Manifests.** Only the manifests `active-manifests.json` lists are loaded.
   - Their revisions, piece counts and the declared totals match what loads.
   - IDs are unique and keep their `tmp.` prefix.
   - Every cross-reference resolves.
   - A key the parser does not know fails, naming it, rather than being dropped.
3. **Pixels.** Every PNG is read as raw indices, never expanded.
   - Every rect lies inside its atlas.
   - The atlas holds exactly the individual PNG's pixels.
   - A backdrop is opaque and covers the widest viewport the strip composes (840×74) from
     (0, 0), with the 640-wide scene at its centre, for viewports to crop.
   - The packed runtime atlas is read back against every sprite.
4. **Palette**. Every shared index is at most 32, and every shared file's `PLTE` and `tRNS` are
   the biome palette: index 0 the only transparent entry, 1..=32 the day colours. Art with its
   own colours (`paletteMode: paired-palette-import`) carries its manifest's day list instead
   and uses no index past it; each day and night pair it uses becomes a runtime slot after the
   shared 32, reused where a slot already holds it, up to 255. The palette PNG matches the
   palette JSON.
5. **Animations.** Every sequence has frames and durations.
   - A creature routine's terminal names a routine of the same species that exists; the
     destination is the leftmost match of the terminal regex.
   - A terminal with no destination passes only if the routine loops or its text starts with
     `hold`.
   - A creature frame that names a routine `animations[]` does not list fails. An effect's
     frames are the attached-effect sprites hosted on its routine's frames.
6. **Variants.** Every `SceneInput` variant has art, as `terrarium_compose::variants` maps it.
7. **Generated files.** The regenerated files equal the committed ones, byte for byte.

Checks 1 to 6 all run, so a delivery shows every problem at once. The exception is a biome
whose manifests fail check 2: checks 3 to 6 need its sprite table, so they are skipped for
it, and the summary names the biome.
Packing and check 7 run only once 1 to 6 pass, so a broken package never writes output.

- `cargo xtask assets --check` compares and writes nothing; verify runs this.
- `cargo xtask assets` rewrites only the files whose bytes differ, so an unchanged run
  rebuilds nothing.

### The atlas

Sprites with identical pixels share one place, and every ID keeps its own entry. The distinct
images are ordered by the smallest ID that uses them. They are laid left to right into
shelves 1024 wide, or 2048 wide when 1024 would need more than 2048 rows. Each byte is the
logical slot + 1 the catalog gives it, which for shared art is the raw PNG index.

## `bench`

The release-build budgets. Each is a `harness = false` bench target that prints its
measurement and exits non-zero over budget. Cargo's bench profile inherits the release
profile, so the timings are those of a shipped build.

- `terrarium-compose` `catalog`: decoding the embedded catalog and checking it against its
  atlas takes under 20 ms, measured cold, the way startup pays it.
- `terrarium-compose` `compositor`: compositing a busy frame takes a median under 1 ms over 200
  runs. The frame is the sample scene plus a full-frame dusk overlay, rain across the width
  and the heaviest condensation, more than any one presentation state draws. It also prints
  the 99th percentile and the slowest run.
- `terrarium-adapters` `history` (built with `fixture`): reading a synthetic 10,000-commit
  history into `CommitFacts` takes under 10 s (N-05). The history is 3,000 files four
  directories deep plus three large blobs, then commits of one to three changes with merged
  side branches, renames and deletions, so a reader that walked whole trees would blow the
  budget. It is generated in a temporary directory and read once untimed, because a read
  straight after writing 100,000 loose objects races the file system flushing them; the
  second read is timed.
- `terrarium-engine` `replay` (N-05): reading the same kind of 10,000-commit history and
  replaying it in full, sim and events included, takes under 30 s, the first connect's budget.
  Observations of every kind are generated beside it, so every detector runs.

## `fixture`

The scripted arc (`terrarium_adapters::fixture::arc`): about two years of a fictional
repository that thrives, struggles, falls silent and recovers, with tags, a merged feature
branch, a rename, a 660-line directory deletion and an unmerged experiment.

- `cargo xtask fixture <dir>` writes it to `<dir>/repo`, and its observations to
  `<dir>/observations.jsonl`, for running the app or a replay against it.
- `cargo xtask fixture --bless` rewrites `tests/fixtures/arc/observations.jsonl`, the copy
  golden replays and the engine's tests import. A test fails while the committed file differs
  from a fresh build in any observation line, or in the header's format or seed; the header's
  `sim_version` is not compared, so a `SIM_VERSION` bump does not touch an input.

The observations are written through a temporary store and its own export: weekly coverage,
weekly dead code and twice-weekly weather, following the arc's phases, plus two readings on side
branches that replay must never apply. Coverage jumps as the arc recovers, so its replay passes
through all six presentation states.

## `golden`

The golden replay (I-13). It builds the arc in a temporary directory, replays it through the
engine's headless pipeline with `tests/fixtures/arc/observations.jsonl`, through five months
past its last commit, and compares the canonical dump with `tests/golden/arc.dump`.
`tests/golden/arc.sha256` holds the dump's SHA-256 in `shasum` format. The dump's first line
names the `SIM_VERSION` that wrote it.

- `cargo xtask golden` or `--check` replays and compares byte for byte. A difference at the
  same `SIM_VERSION` is reported as truth's output changing without a bump.
- `cargo xtask golden --bless` rewrites both files and prints the difference, line by line
  paired by path, for review. It refuses a changed dump at the `SIM_VERSION` already on disk:
  bump `SIM_VERSION` first. `--bless --same-version` overrides that, only to re-bless a version
  whose golden is not committed yet.

## `render`

Writes a scene by day, in two sizes:

- `<out.png>` at 1×, 640×74;
- `<out>@3x.png` beside it at 3×, 1920×222.

The scenes:

- `cargo xtask render --demo <out.png>` is the sample scene the strip shows.
- `cargo xtask render --preset <name> <out.png>` is one of the composer's presets, composed
  from nothing and settled, with the presets' fixed salt. The names are `thriving`,
  `struggling-low-light`, `struggling-poor-soil`, `mature`, `dormant`, `dead` and
  `recovering`. An unknown name fails and lists them.

Both go through the strip's own compositor and upscaler, and the one PNG encoder in
`terrarium_compose::export`. The label reads `repo-terrarium`, the binary's name. The app takes
the same words from its copy module, so the two literals must change together. The presets'
pictures are also exact snapshots in `terrarium-compose`'s tests.

## `probe-zip`

The Windows probe build for testers. There is no CI, so it is cross-built on the Mac for
`x86_64-pc-windows-msvc`, the target `verify` lints and the release ships.

- **Linker:** the `rust-lld` the toolchain ships, so no LLVM install is needed. The C runtime is
  linked statically, so testers need no Visual C++ redistributable.
- **Libraries:** the MSVC C runtime and the Windows SDK come from `xwin` 0.10.0
  (`cargo install --locked xwin@0.10.0`). They are splatted once into
  `~/Library/Caches/repo-terrarium/xwin`, or `$TERRARIUM_XWIN_DIR`.
- **The license.** Getting the libraries downloads Microsoft's packages and accepts
  Microsoft's license for them. That is a person's decision, so the task never runs it. When
  the libraries are missing, it prints the one-time command.
- **The import check.** It fails an exe that imports `comctl32.dll` by name rather than by
  ordinal, since such an exe may not start where comctl32 exports only ordinals.
- **Output:** `target/probe/repo-terrarium-<version>-probe-windows-x64.zip` (under
  `CARGO_TARGET_DIR` when it is set), holding:
  - `repo-terrarium.exe`;
  - `README.txt`, the tester's instructions. They are app copy, so the task takes them from
    `repo-terrarium --probe-readme`;
  - `run-probe.cmd`, which runs `--probe pinning` from the unzipped folder.
