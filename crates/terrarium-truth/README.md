# terrarium-truth

The deterministic core: what the terrarium *is*, as a pure function of the repository's history
and the observations recorded about it.

## Owns

- Fixed point (`Q16`, `Q32`), the PCG32 random number generator and the one-hour UTC tick grid.
- Domain types, climate and habitat math, and the presentation timeline.
- The compiled tuning values (`tuning`), every one under `SIM_VERSION`.
- The sim, events and their arbitration, and the replay core.
- `project()`, which turns sim state into `SceneInput`, the composer's only truth-derived input.
  `SceneInput` is frozen: any change to it is an amendment and a `SIM_VERSION` bump.
- The two encodings of truth's state: postcard for the cache, and the canonical text dump, with
  its SHA-256, for golden files.
- `SIM_VERSION`.

## Why it is a crate of its own

Two installs replaying the same repository must agree exactly (I-1). With no workspace
dependencies and its own lint walls, purity is a compile-time property rather than a habit.

## Rules

- No IO, wall clock, floats, hash-order iteration, install salt or local time. The inputs are
  git history, recorded observations, the seed, the default branch and `SIM_VERSION`.
- It depends on no other workspace crate. Its external dependencies are `serde`, `postcard` and
  `sha2`, of the four the crate graph allows (`indexmap` is the fourth). `cargo xtask verify`
  checks both.
- `#![forbid(unsafe_code)]`, and `deny` of `clippy::float_arithmetic`, `clippy::as_conversions`
  (so no numeric conversion is silent), `clippy::disallowed_types` and
  `clippy::disallowed_methods`, so the lists below are errors in every clippy run, not only in
  `verify`.
- This directory's `clippy.toml` disallows:
  - the hash-ordered collections and std's hashers;
  - the float types;
  - the clock types and their readers;
  - the unstable sorts, whose order of equal elements may change between Rust releases.
- `cargo xtask verify` scans every source file here for:
  - float literals and float type names, raw identifiers included;
  - a local `allow` of a wall lint or of `warnings`;
  - `include!` and `#[path]`;
  - any path through `std::fs`, `env`, `net`, `process`, `thread` or `io`, however it is named,
    an alias of `std` itself, and the printing macros.

  Truth is one library target, so its tests live in `src/` under the same walls.
- Fixed point saturates and rounds half away from zero. Randomness is `PCG32` through
  `SplitMix64`, one stream per purpose; a stream's seed hashes the purpose's name, so a new
  purpose never shifts another's draws.
- Dumped state has no `usize`: `Canonical` has no impl for it.
- Enum variants are only ever appended, since their order is part of the postcard encoding.
- Any change to truth output bumps `SIM_VERSION` (I-13). The postcard and canonical-dump
  encodings are part of that output.

## Requirements

Invariants I-1, I-3 and I-13.
