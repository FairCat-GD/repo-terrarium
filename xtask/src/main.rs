#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod arc;
mod assets;
mod bench;
mod fixture;
mod gallery;
mod golden;
mod graph;
mod hero;
mod install_test;
mod macho;
mod package;
mod pe;
mod pieces;
mod probe_zip;
mod purity;
mod release;
mod render;
mod size;
mod soak;
mod sweep;
mod verify;
mod windows;

use std::path::Path;
use std::process::ExitCode;

use sha2::{Digest, Sha256};
use terrarium_truth::canon::to_hex;

fn main() -> ExitCode {
    // xtask lives one level below the workspace root; resolving it here lets `cargo xtask`
    // run from any directory in the repo.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits inside the workspace");
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [task] if task == "verify" => verify::run(root),
        [task] if task == "assets" => assets::run(root, assets::Mode::Write),
        [task, flag] if task == "assets" && flag == "--check" => {
            assets::run(root, assets::Mode::Check)
        }
        [task] if task == "arc" => arc::run(root),
        [task] if task == "bench" => bench::run(root),
        [task, flag] if task == "fixture" && flag == "--bless" => fixture::bless(root),
        [task, out] if task == "fixture" && !out.starts_with('-') => {
            fixture::run(std::path::Path::new(out))
        }
        [task] if task == "gallery" => gallery::run(root),
        [task] if task == "hero" => hero::run(root),
        [task] if task == "golden" => golden::run(root, golden::Mode::Check),
        [task, flag] if task == "golden" && flag == "--check" => {
            golden::run(root, golden::Mode::Check)
        }
        [task, flag] if task == "golden" && flag == "--bless" => golden::run(
            root,
            golden::Mode::Bless {
                same_version: false,
            },
        ),
        [task, bless, same]
            if task == "golden" && bless == "--bless" && same == "--same-version" =>
        {
            golden::run(root, golden::Mode::Bless { same_version: true })
        }
        [task] if task == "install-test" => install_test::run(root),
        [task, rest @ ..] if task == "package" => package::run(root, rest),
        [task] if task == "probe-zip" => probe_zip::run(root),
        [task] if task == "size" => size::run(root),
        [task, rest @ ..] if task == "soak" => soak::run(root, rest),
        [task, flag, out] if task == "render" && flag == "--demo" => {
            render::run(std::path::Path::new(out))
        }
        [task, flag, name, out] if task == "render" && flag == "--preset" => {
            render::run_preset(name, std::path::Path::new(out))
        }
        _ => {
            eprintln!(
                "usage: cargo xtask verify | arc | assets [--check] | bench | fixture <dir> | fixture \
                 --bless | gallery | golden [--check | --bless [--same-version]] | hero | \
                 install-test | package [--windows | --target <triple>] | probe-zip | \
                 render --demo <out.png> | render --preset <name> <out.png> | size | \
                 soak [--minutes <n>] [--presenter layer | softbuffer | wgpu | both] | soak --report <file>"
            );
            ExitCode::from(2)
        }
    }
}

/// The SHA-256 of `bytes` in lowercase hex, as `shasum -a 256` prints it.
pub fn sha256_hex(bytes: &[u8]) -> String {
    to_hex(&Sha256::digest(bytes))
}
