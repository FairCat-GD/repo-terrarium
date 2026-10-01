//! The per-frame budget: compositing a busy frame takes under 1 ms in a release build. The
//! frame is the sample scene with a full-frame dusk overlay, rain across the width and the
//! heaviest condensation, more than any one presentation state draws. Run by `cargo xtask
//! bench`.

#![forbid(unsafe_code)]
#![expect(
    clippy::disallowed_methods,
    reason = "a bench measures wall time, which the composer itself never reads"
)]

use std::hint::black_box;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use terrarium_compose::catalog::Art;
use terrarium_compose::compositor::composite_into;
use terrarium_compose::frame::{Draw, Frame, Rgba640x74};
use terrarium_compose::sample;
use terrarium_compose::variants::ArtTable;

const BUDGET: Duration = Duration::from_millis(1);
const RUNS: usize = 200;

fn main() -> ExitCode {
    let art = Art::embedded();
    let Ok(table) = ArtTable::new(&art.catalog) else {
        println!("compositor: the embedded art is incomplete");
        return ExitCode::FAILURE;
    };
    let frame = busy_frame(art, &table);
    let mut out = Rgba640x74::filled([0; 4]);
    let mut times: Vec<Duration> = (0..RUNS)
        .map(|_| {
            let started = Instant::now();
            composite_into(art, black_box(&frame), &mut out);
            black_box(&out);
            started.elapsed()
        })
        .collect();
    times.sort();
    let median = times[RUNS / 2];
    let p99 = times[RUNS * 99 / 100];
    let max = times[RUNS - 1];
    let verdict = if median < BUDGET { "ok" } else { "OVER BUDGET" };
    println!(
        "compositor: {} draws, median {median:.2?}, p99 {p99:.2?}, max {max:.2?}, budget \
         {BUDGET:?}: {verdict}",
        frame.draws.len()
    );
    if median < BUDGET {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn busy_frame(art: &Art, table: &ArtTable) -> Frame {
    let catalog = &art.catalog;
    let mut frame = sample::frame(art, table, "repo-terrarium", 0, 0);
    let named = |name: &str| catalog.id(name).expect(name);
    frame
        .draws
        .push(Draw::new(catalog, named("tmp.sky.light.dusk"), 0, 0));
    let rain = named("tmp.sky.rain.f01");
    frame
        .draws
        .extend((0..640).step_by(8).map(|x| Draw::new(catalog, rain, x, 0)));
    frame
        .draws
        .push(Draw::new(catalog, named("tmp.glass.condensation.3"), 0, 0));
    frame.draws.sort_by_key(|draw| draw.layer);
    frame
}
