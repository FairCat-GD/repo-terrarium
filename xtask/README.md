# xtask

Development tasks, run as `cargo xtask <task>` from anywhere in the repository. Not shipped.
The tasks are `verify` and `probe-zip`.

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
  denies `clippy::undocumented_unsafe_blocks`, and truth denies `clippy::float_arithmetic` and
  `clippy::as_conversions`.
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
- the names `f16`, `f32`, `f64` and `f128`, including module paths like `core::f64::consts`;
- an `allow`, `expect` or `warn` of a wall lint (`float_arithmetic`, `as_conversions`,
  `disallowed_types`, `disallowed_methods`) or of a group holding one (`all`, `style`,
  `restriction`);
- `include!` and `#[path]`, which would bring code in from outside the scan.

It reads tokens, so comments, strings and char literals never trigger it.

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
