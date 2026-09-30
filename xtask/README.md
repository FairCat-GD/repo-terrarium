# xtask

Development tasks, run as `cargo xtask <task>` from anywhere in the repository. Not shipped.
The tasks are `verify`, `assets`, `bench` and `probe-zip`.

## `verify`

The gate every commit passes. It runs every stage, even after one fails, and ends with a
summary. The stages:

1. **fmt**: `cargo fmt --all --check`.
2. **clippy**: `cargo clippy --workspace --all-targets -- -D warnings`.
3. **clippy (windows)**: the same for `x86_64-pc-windows-msvc`. There is no CI, so this is what
   compiles and lints the Windows code.
4. **nextest**: `cargo nextest run --workspace`.
5. **doctest**: `cargo test --workspace --doc`, because nextest skips doctests.
6. **deny**: `cargo deny check`, with the policy in `deny.toml`.
7. **graph**: the crate-graph check below, including the scan of truth's sources.
8. **assets**: `cargo xtask assets --check`, below. It runs in process, and a panic fails only
   this stage.

It needs `cargo-nextest` 0.9.146 and `cargo-deny` 0.20.2, pinned in `src/verify.rs`. A stage whose
tool is missing, or at another version, fails and names the install command.

## The crate-graph check

`cargo metadata` gives the dependency graph, resolved for the shipping targets listed in
`deny.toml`'s `[graph] targets`, and `src/graph.rs` holds the policy as data:

- Each crate depends only on the workspace crates the policy allows, for every dependency kind.
  A new crate fails until it is added to the policy.
- `terrarium-truth` links no external crate except `serde`, `postcard`, `indexmap` and `sha2`.
- `ureq` is reachable only through `terrarium-adapters`.
- Every target root outside `terrarium-platform` carries `#![forbid(unsafe_code)]`. Platform
  denies `clippy::undocumented_unsafe_blocks`, and truth denies `clippy::float_arithmetic`,
  `clippy::as_conversions`, `clippy::disallowed_types` and `clippy::disallowed_methods`.
- Truth is a single library target. A test, bench, example or build script would be a crate of
  its own, outside the lint walls on truth's root.
- Every manifest inherits the workspace package fields and lints.
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
- `include!` and `#[path]`, which would bring code in from outside the scan.

It reads tokens, so comments, strings and char literals never trigger it.

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
   - The packed runtime atlas is read back against every sprite.
4. **Palette**. Every index is at most 32. Every file's `PLTE` and `tRNS` are the biome palette:
   index 0 the only transparent entry, 1..=32 the day colours. The palette PNG matches the
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
shelves 1024 wide, or 2048 wide when 1024 would need more than 2048 rows. Each byte is the raw
PNG index.

## `bench`

The release-build budgets. Each is a `harness = false` bench target that prints its
measurement and exits non-zero over budget. Cargo's bench profile inherits the release
profile, so the timings are those of a shipped build.

- `terrarium-compose` `catalog`: decoding the embedded catalog and checking it against its
  atlas takes under 20 ms, measured cold, the way startup pays it.

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
- **Output:** `target/probe/repo-terrarium-<version>-probe-windows-x64.zip`, holding:
  - `repo-terrarium.exe`;
  - `README.txt`, the tester's instructions. They are app copy, so the task takes them from
    `repo-terrarium --probe-readme`;
  - `run-probe.cmd`, which runs `--probe pinning` from the unzipped folder.
