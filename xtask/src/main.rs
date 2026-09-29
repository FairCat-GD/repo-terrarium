#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod graph;
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
    if args == ["verify"] {
        verify::run(root)
    } else {
        eprintln!("usage: cargo xtask verify");
        ExitCode::from(2)
    }
}
