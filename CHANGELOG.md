# Changelog

What shipped, per version. The format follows [Keep a Changelog](https://keepachangelog.com/).
Each step of development is a minor version, `0.<n>.0`, until the first public release.

## [Unreleased]

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
