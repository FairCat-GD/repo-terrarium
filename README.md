# Repo Terrarium

An ambient pixel-art strip for a developer's second monitor. It shows a living terrarium whose
climate is one local git repository: coverage is light, code health is soil, activity is rain,
and CI or local tests are the weather. It is built in Rust for macOS and Windows.

## Status

Pre-release, in a private repository. Version 0.1.0 is the scaffold: running the app opens a plain
window, and `--version` prints the version.

## Prerequisites

- **Rust** through [rustup](https://rustup.rs). The toolchain is pinned in `rust-toolchain.toml`
  (1.98.1, with rustfmt, clippy and the `x86_64-pc-windows-msvc` target), and rustup installs it
  on first use.
- **For `verify`:** `cargo install --locked cargo-nextest cargo-deny`.

## Build and run

```sh
cargo run -p terrarium-app                 # opens the window
cargo run -p terrarium-app -- --version    # prints the version
cargo xtask verify                         # the gate: must be green before every commit
```

There is no CI. `cargo xtask verify` runs locally before every commit. It includes a clippy pass
for the Windows target, so Windows code is compiled and linted on a Mac.

## Layout

- `crates/`: the eight crates, each with a `README.md` saying what it owns and why.
- `xtask/`: development tasks (`cargo xtask verify`).
- `art-assets/`: the art, byte for byte as delivered.
- `tests/fixtures/`: fictional test data.

## License

- Code: MIT or Apache-2.0, at your option (`LICENSE-MIT`, `LICENSE-APACHE`).
- Art in `art-assets/`: CC BY-SA 4.0 (`LICENSE-ART.md`); the bitmap fonts are public domain.
