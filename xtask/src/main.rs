#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod assets;
mod bench;
mod graph;
mod probe_zip;
mod purity;
mod verify;

use std::path::Path;
use std::process::ExitCode;

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
        [task] if task == "bench" => bench::run(root),
        [task] if task == "probe-zip" => probe_zip::run(root),
        _ => {
            eprintln!("usage: cargo xtask verify | assets [--check] | bench | probe-zip");
            ExitCode::from(2)
        }
    }
}
