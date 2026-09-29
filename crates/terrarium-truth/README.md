# terrarium-truth

The deterministic core: what the terrarium *is*, as a pure function of the repository's history
and the observations recorded about it.

## Owns

- Fixed point (`Q16`, `Q32`), the PCG32 random number generator and the one-hour UTC tick grid.
- Domain types, climate and habitat math, and the presentation timeline.
- The sim, events and their arbitration, and the replay core.
- `project()`, which turns sim state into `SceneInput`, the composer's only truth-derived input.
- `SIM_VERSION`.

## Why it is a crate of its own

Two installs replaying the same repository must agree exactly (I-1). With no workspace
dependencies and its own lint walls, purity is a compile-time property rather than a habit.

## Rules

- No IO, wall clock, floats, hash-order iteration, install salt or local time. The inputs are
  git history, recorded observations, the seed, the default branch and `SIM_VERSION`.
- It depends on no other workspace crate, and its external dependencies are limited to `serde`,
  `postcard`, `indexmap` and `sha2`. `cargo xtask verify` checks both.
- `#![forbid(unsafe_code)]` and `#![deny(clippy::float_arithmetic)]`. This directory's
  `clippy.toml` disallows the hash-ordered collections, the float types and the wall-clock
  readers.
- Any change to truth output bumps `SIM_VERSION` (I-13).

## Requirements

Invariants I-1, I-3 and I-13.
