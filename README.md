# Repo Terrarium

An ambient pixel-art strip for a developer's second monitor. It shows a living terrarium whose
climate is one local git repository: coverage is light, code health is soil, activity is rain,
and CI or local tests are the weather. It is built in Rust for macOS and Windows.

## Status

Pre-release, in a private repository. Version 0.6.0 shows the strip:
- a borderless band along the bottom of a monitor;
- on macOS, pinned below ordinary windows;
- on Windows, a topmost bar on a reserved edge, the way the taskbar is. It is untested on real
  hardware yet;
- a sample scene drawn from the delivered art: a few plants on the soil band, the glass edges
  and a label;
- a raise hotkey (Ctrl+Alt+T);
- a right-click **Quit**.

## Prerequisites

- **Rust** through [rustup](https://rustup.rs). The toolchain is pinned in `rust-toolchain.toml`
  (1.98.1, with rustfmt, clippy and the `x86_64-pc-windows-msvc` target), and rustup installs it
  on first use.
- **For `verify`:** `cargo install --locked cargo-nextest@0.9.146 cargo-deny@0.20.2`. `verify`
  checks these exact versions, because `deny.toml` is written for cargo-deny 0.20.

## Build and run

```sh
cargo run -p terrarium-app                        # runs the strip; right-click it to quit
cargo run -p terrarium-app -- --version           # prints the version
cargo run -p terrarium-app -- --probe pinning     # the pinning checklist, with a report
cargo xtask verify                                # the gate: must be green before every commit
cargo xtask probe-zip                             # the Windows probe zip for testers
cargo xtask render --demo out.png                 # the sample scene as out.png and out@3x.png
```

There is no CI. `cargo xtask verify` runs locally before every commit. It includes a clippy pass
for the Windows target, so Windows code is compiled and linted on a Mac.

## Layout

- `crates/`: the eight crates, each with a `README.md` saying what it owns and why.
- `xtask/`: development tasks (`cargo xtask verify`, `assets`, `bench`, `probe-zip`, `render`).
- `art-assets/`: the art, byte for byte as delivered.
- `tests/fixtures/`: fictional test data.

## License

- Code: MIT or Apache-2.0, at your option (`LICENSE-MIT`, `LICENSE-APACHE`).
- Art in `art-assets/`: CC BY-SA 4.0 (`LICENSE-ART.md`); the bitmap fonts are public domain.
