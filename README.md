# Repo Terrarium

An ambient pixel-art strip for a developer's second monitor. It shows a living terrarium whose
climate is one local git repository: coverage is light, code health is soil, activity is rain,
and CI or local tests are the weather. It is built in Rust for macOS and Windows.

## Status

Pre-release, in a private repository, at version 0.21.0. The strip:
- is a borderless band along the bottom of a monitor;
- on macOS, is pinned below ordinary windows;
- on Windows, is a topmost bar on a reserved edge, the way the taskbar is. It is untested on
  real hardware yet;
- runs on one local repository: with none connected, a window opens to choose and connect one
  (`--repo <path>` connects it from the command line), and a plain run opens the one connected
  last. One app runs at a time; launching it again shows its settings;
- reads the repository's history, its coverage, test and dead-code reports, and the test runs
  `record-tests` records;
- simulates from them, the same way on every computer, and draws:
  - the plants;
  - the residents;
  - the standing stones and the fossils;
  - the events that happen to them, in the sky and on the ground;
- has a raise hotkey (Ctrl+Alt+T) and a right-click **Settings** (where the repository is,
  relinking it, a new terrarium, disconnecting) and **Quit**.

## Prerequisites

- **Rust** through [rustup](https://rustup.rs). The toolchain is pinned in `rust-toolchain.toml`
  (1.98.1, with rustfmt, clippy and the `x86_64-pc-windows-msvc` target), and rustup installs it
  on first use.
- **For `verify`:** `cargo install --locked cargo-nextest@0.9.146 cargo-deny@0.20.2`. `verify`
  checks these exact versions, because `deny.toml` is written for cargo-deny 0.20.

## Build and run

```sh
cargo run -p terrarium-app -- --repo <path>          # connects a repository and runs the strip
cargo run -p terrarium-app                           # the one connected last, or the connect window
cargo run -p terrarium-app -- --demo                 # the strip through its named looks
cargo run -p terrarium-app -- discover <path>        # what each report adapter finds there
cargo run -p terrarium-app -- record-tests -- <cmd>  # runs tests; their result is the weather
cargo run -p terrarium-app -- replay <path> --dump   # a headless replay, as text
cargo run -p terrarium-app -- --version              # prints the version
cargo run -p terrarium-app -- --probe pinning        # the pinning checklist, with a report
cargo xtask verify                                   # the gate: green before every commit
cargo xtask probe-zip                                # the Windows probe zip for testers
cargo xtask render --demo out.png                    # the sample scene, out.png and out@3x.png
```

Right-click the strip for its settings or to quit.

There is no CI. `cargo xtask verify` runs locally before every commit. It includes a clippy pass
for the Windows target, so Windows code is compiled and linted on a Mac.

## Layout

- `crates/`: the seven crates, each with a `README.md` saying what it owns and why.
- `xtask/`: development tasks (`cargo xtask verify`, `assets`, `bench`, `fixture`, `gallery`,
  `golden`, `probe-zip`, `render`).
- `art-assets/`: the art, byte for byte as delivered.
- `tests/fixtures/`: fictional test data.

## License

- Code: MIT or Apache-2.0, at your option (`LICENSE-MIT`, `LICENSE-APACHE`).
- Art in `art-assets/`: CC BY-SA 4.0 (`LICENSE-ART.md`); the bitmap fonts are public domain.
