//! The startup budget for the embedded art: decoding the catalog and checking it against its
//! atlas takes under 20 ms in a release build. Run by `cargo xtask bench`.

#![forbid(unsafe_code)]
#![expect(
    clippy::disallowed_methods,
    reason = "a bench measures wall time, which the composer itself never reads"
)]

use std::hint::black_box;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use terrarium_compose::catalog::Art;

const BUDGET: Duration = Duration::from_millis(20);
const WARM_RUNS: usize = 50;

fn main() -> ExitCode {
    // The first decode is the one startup pays: nothing is cached or warmed yet.
    let started = Instant::now();
    let art = Art::load_embedded();
    let cold = started.elapsed();
    if let Err(err) = black_box(art) {
        println!("catalog: the embedded art does not decode: {err}");
        return ExitCode::FAILURE;
    }
    let mut warm: Vec<Duration> = (0..WARM_RUNS)
        .map(|_| {
            let started = Instant::now();
            let _ = black_box(Art::load_embedded());
            started.elapsed()
        })
        .collect();
    warm.sort();
    let median = warm[WARM_RUNS / 2];
    let verdict = if cold < BUDGET { "ok" } else { "OVER BUDGET" };
    println!(
        "catalog load: cold {cold:.2?}, warm median {median:.2?}, budget {BUDGET:?}: {verdict}"
    );
    if cold < BUDGET {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
