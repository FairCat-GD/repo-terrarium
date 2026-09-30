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
7. **graph**: the crate-graph check below.

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
  denies `clippy::undocumented_unsafe_blocks`, and truth denies `clippy::float_arithmetic`.
- Every manifest inherits the workspace package fields and lints.
- No dependency declares a `rust-version` above the workspace's.

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
- **Output:** `target/probe/repo-terrarium-<version>-probe-windows-x64.zip`, holding:
  - `repo-terrarium.exe`;
  - `README.txt`, the tester's instructions, from `probe/README.txt`;
  - `run-probe.cmd`, which runs `--probe pinning` from the unzipped folder.
