# xtask

Development tasks, run as `cargo xtask <task>` from anywhere in the repository. Not shipped.

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

It needs `cargo-nextest` and `cargo-deny`; a stage whose tool is missing fails and names the
install command.

## The crate-graph check

`cargo metadata` gives the dependency graph, and `src/graph.rs` holds the policy as data:

- Each crate depends only on the workspace crates the policy allows, for every dependency kind.
  A new crate fails until it is added to the policy.
- `terrarium-truth` links no external crate except `serde`, `postcard`, `indexmap` and `sha2`.
- `ureq` is reachable only through `terrarium-adapters`.
- Every target root outside `terrarium-platform` carries `#![forbid(unsafe_code)]`. Platform
  denies `clippy::undocumented_unsafe_blocks`, and truth denies `clippy::float_arithmetic`.
- Every manifest inherits the workspace package fields and lints.
- No dependency declares a `rust-version` above the workspace's.
