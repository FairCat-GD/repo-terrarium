//! The live pipeline's budgets, through `Live` as the app runs it: the store's writes, the
//! replay cache and the source metrics included, which the replay bench leaves out. Run by
//! `cargo xtask bench`.
//!
//! On the synthetic 10,000-commit history (3,000 source files), in a data directory and a
//! repository the bench owns, on a clock it sets, with no CI:
//! - the first connect and its first refresh (N-05): under 30 s;
//! - connecting again from the cache that wrote, which the app waits for before its strip
//!   opens: under 250 ms;
//! - that connect's first refresh, which reads the history again: under 5 s;
//! - a refresh after one new commit: under 1 s;
//! - a refresh after a release tag on a commit already read, which replays in full but reads
//!   no commit again: under 2 s (a whole read again takes about 4).

#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use terrarium_adapters::fixture::{self, Edit, FixtureRepo, Stamp};
use terrarium_engine::schedule::{Cause, Refresh};
use terrarium_engine::{Clock, EngineConfig, Live, ManualClock};
use terrarium_store::DataDir;

const COMMITS: usize = 10_000;
/// Two days after the synthetic history's three years, which end at 2025-01-02.
const NOW: i64 = 1_735_948_800;

/// (what, its budget).
const FIRST: (&str, Duration) = ("first connect and refresh", Duration::from_secs(30));
const CONNECT: (&str, Duration) = ("connect again from the cache", Duration::from_millis(250));
const AGAIN: (&str, Duration) = ("its first refresh", Duration::from_secs(5));
const COMMIT: (&str, Duration) = ("a refresh after one commit", Duration::from_secs(1));
const TAG: (&str, Duration) = (
    "a refresh after a tag on an old commit",
    Duration::from_secs(2),
);

fn main() -> ExitCode {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("repo");
    let mut repo = fixture::synthetic(&path, COMMITS).expect("the synthetic history is written");
    let old = repo.tip("main");
    let clock = ManualClock::at(NOW);
    let config = EngineConfig {
        clock: Arc::new(clock.clone()),
        ..EngineConfig::new(DataDir::at(dir.path().join("data")), Some(path.clone()))
    };
    let mut results = Vec::new();

    let start = Instant::now();
    let mut live = connect(&config);
    refresh(&mut live, Cause::Connect, true);
    results.push((FIRST, start.elapsed()));
    drop(live);

    let start = Instant::now();
    let mut live = connect(&EngineConfig {
        repo: None,
        clock: Arc::new(clock.clone()),
        ..EngineConfig::new(DataDir::at(dir.path().join("data")), None)
    });
    results.push((CONNECT, start.elapsed()));
    let start = Instant::now();
    refresh(&mut live, Cause::Connect, true);
    results.push((AGAIN, start.elapsed()));

    commit(&mut repo, &clock);
    let start = Instant::now();
    refresh(&mut live, Cause::Watch, false);
    results.push((COMMIT, start.elapsed()));

    repo.tag_lightweight("v1.0.0", &old)
        .expect("the tag is written");
    let start = Instant::now();
    refresh(&mut live, Cause::Watch, false);
    results.push((TAG, start.elapsed()));

    let mut within = true;
    for ((what, budget), took) in results {
        let ok = took < budget;
        within &= ok;
        println!(
            "live: {what}: {:.3} s (budget {} s): {}",
            took.as_secs_f64(),
            budget.as_secs_f64(),
            if ok { "ok" } else { "FAILED" }
        );
    }
    if within {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn connect(config: &EngineConfig) -> Live {
    Live::connect(config)
        .expect("the store opens")
        .expect("the repository is connected")
}

/// A refresh for `cause`, which must reach the repository and find nothing wrong.
fn refresh(live: &mut Live, cause: Cause, reports: bool) {
    let done = live.refresh(
        Refresh { cause, reports },
        &mut |_| {},
        &AtomicBool::new(false),
    );
    assert!(done.reach.is_some(), "the refresh reached the repository");
    assert!(done.problem.is_none(), "{:?}", done.problem);
}

/// A commit a minute old, an hour on: the next hour's work.
fn commit(repo: &mut FixtureRepo, clock: &ManualClock) {
    clock.advance(3_600);
    repo.commit(
        "main",
        Stamp::at(clock.wall_secs() - 60),
        "tend the forest",
        &[Edit::write("tended.rs", "pub const TENDED: u32 = 1;\n")],
    )
    .expect("the commit is written");
}
