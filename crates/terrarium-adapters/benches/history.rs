//! The first-connect budget for git history (N-05): reading a synthetic 10,000-commit history
//! into `CommitFacts` takes under 10 s in a release build. Run by `cargo xtask bench`.
//!
//! The history is generated into a temporary directory first, untimed. It is then read once,
//! untimed: the generator has just written over 100,000 loose objects, and a read straight
//! after races the file system flushing them, which no repository a person has worked in does.
//! The second read is timed, from opening the repository to the last commit's facts.

#![forbid(unsafe_code)]

use std::ops::ControlFlow;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use terrarium_adapters::fixture;
use terrarium_adapters::git::GitRepo;

const BUDGET: Duration = Duration::from_secs(10);
const COMMITS: usize = 10_000;

fn main() -> ExitCode {
    let dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(err) => {
            println!("history: no temporary directory: {err}");
            return ExitCode::FAILURE;
        }
    };
    let path = dir.path().join("synthetic");
    let generated = Instant::now();
    if let Err(err) = fixture::synthetic(&path, COMMITS) {
        println!("history: the synthetic history was not built: {err}");
        return ExitCode::FAILURE;
    }
    let generated = generated.elapsed();
    let read = || {
        let started = Instant::now();
        let read = GitRepo::open(&path)
            .and_then(|repo| repo.read_history("main", &mut |_| ControlFlow::Continue(())));
        (read, started.elapsed())
    };
    let (_, settling) = read();
    let (read, elapsed) = read();
    let history = match read {
        Ok(history) => history,
        Err(err) => {
            println!("history: reading failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    if history.commits.len() != COMMITS {
        println!(
            "history: read {} commits, expected {COMMITS}",
            history.commits.len()
        );
        return ExitCode::FAILURE;
    }
    let verdict = if elapsed < BUDGET {
        "ok"
    } else {
        "OVER BUDGET"
    };
    println!(
        "history read: {COMMITS} commits in {elapsed:.2?} (generated in {generated:.2?}, \
         first read {settling:.2?}), budget {BUDGET:?}: {verdict}"
    );
    if elapsed < BUDGET {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
