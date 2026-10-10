//! The first-connect budget (N-05): reading a synthetic 10,000-commit history and replaying it
//! in full, sim and events included, takes under 30 s in a release build. Run by
//! `cargo xtask bench`.
//!
//! The history is generated into a temporary directory, with three commits after it that the
//! synthetic generator does not make (a large directory planted, then deleted, then a commit
//! after a month's silence, two of them tagged as releases), and read once, untimed, as the
//! history bench does. Observations are generated beside it: weather that fails for a week
//! every month, weekly coverage that drifts, weekly dead-code reports that pile up and are
//! cleared, and a force push every quarter, so every kind of detector runs. Then the timed run:
//! opening the repository, reading it, preparing the inputs, replaying from tick 0 to the last
//! commit, and projecting.

#![forbid(unsafe_code)]

use std::ops::ControlFlow;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use terrarium_adapters::fixture::{self, Edit, FixtureRepo, Stamp};
use terrarium_adapters::git::{GitRepo, History};
use terrarium_truth::domain::{
    DeletedDir, LeftCommit, ObservationHead, ObservationValue, WeatherSource,
};
use terrarium_truth::{Basis, Observation, Prepared, Q16, RepoSeed, SimState, project};

const BUDGET: Duration = Duration::from_secs(30);
const COMMITS: usize = 10_000;
const DAY: i64 = 86_400;
/// Saturday 2025-03-01, after the synthetic history's three years.
const AFTER: i64 = 1_740_787_200;

/// The commits after the synthetic history: a large directory planted and deleted, and a
/// commit after a month without one.
fn extend(repo: &mut FixtureRepo) -> fixture::FixtureResult<()> {
    let planted = repo.commit(
        "main",
        Stamp::at(AFTER),
        "plant",
        &[Edit::write("big/roots.rs", &"root\n".repeat(900))],
    )?;
    repo.tag_lightweight("v1.0.0", &planted)?;
    repo.commit(
        "main",
        Stamp::at(AFTER + DAY),
        "clear",
        &[Edit::Remove("big/roots.rs".into())],
    )?;
    let back = repo.commit(
        "main",
        Stamp::at(AFTER + 31 * DAY),
        "return",
        &[Edit::write("back.rs", "back\n")],
    )?;
    repo.tag_lightweight("v2.0.0", &back)
}

/// A day's observations over the history's span.
fn observations(history: &History) -> Vec<Observation> {
    let start = history.grid.origin_secs();
    let end = history
        .commits
        .last()
        .map_or(start, |c| history.grid.start_secs(c.tick));
    let mut out = Vec::new();
    // The default tip as the days go by: one walk over the tick-ordered commits.
    let mut commits = history.commits.iter().peekable();
    let mut tip = None;
    let mut day = 1;
    while start + day * DAY < end {
        let at = start + day * DAY + 17 * 3_600;
        let tick = history.grid.tick_at(at);
        while let Some(next) = commits.next_if(|c| Some(c.tick) <= tick) {
            if next.on_default_first_parent {
                tip = Some(next.id.clone());
            }
        }
        let Some(head) = tip.clone() else {
            day += 1;
            continue;
        };
        let observe = |value: ObservationValue, offset: i64, normalized: Option<Q16>| Observation {
            id: 0,
            recorded_at: at + offset,
            head: ObservationHead {
                commit: head.clone(),
                branch: Some("main".to_owned()),
                at_default_tip: true,
            },
            normalized,
            confidence: Q16::ONE,
            value,
        };
        if day % 90 == 45 {
            out.push(observe(
                ObservationValue::RefRewrite {
                    reference: "refs/heads/main".to_owned(),
                    old: head.clone(),
                    new: head.clone(),
                    rewritten: Some(u32::try_from(day % 13).unwrap_or(0)),
                    fossils: Vec::new(),
                    left: vec![LeftCommit {
                        commit: head.clone(),
                        time: 0,
                        deleted_dirs: vec![DeletedDir {
                            path: "src/old".to_owned(),
                            lines: 600,
                        }],
                    }],
                },
                180,
                None,
            ));
        }
        let failing = day % 30 < 7;
        out.push(observe(
            ObservationValue::Weather {
                source: WeatherSource::Ci {
                    branch: "main".to_owned(),
                },
            },
            0,
            Some(if failing { Q16::ZERO } else { Q16::ONE }),
        ));
        if day % 7 == 0 {
            let week = i32::try_from(day / 7).unwrap_or(0);
            let coverage = 60 + (week * 7) % 25;
            out.push(observe(
                ObservationValue::Coverage,
                60,
                Some(Q16::from_ratio(coverage, 100)),
            ));
            let unused = u32::try_from(week % 9).unwrap_or(0) * 6;
            out.push(observe(
                ObservationValue::DeadCode {
                    unused_items: unused,
                    exported_items: 100,
                },
                120,
                Some(Q16::from_ratio(
                    100 - i32::try_from(unused).unwrap_or(0),
                    100,
                )),
            ));
        }
        day += 1;
    }
    out
}

fn main() -> ExitCode {
    let dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(err) => {
            println!("replay: no temporary directory: {err}");
            return ExitCode::FAILURE;
        }
    };
    let path = dir.path().join("synthetic");
    if let Err(err) = fixture::synthetic(&path, COMMITS).and_then(|mut repo| extend(&mut repo)) {
        println!("replay: the synthetic history was not built: {err}");
        return ExitCode::FAILURE;
    }
    let read = || {
        GitRepo::open(&path)
            .and_then(|repo| repo.read_history("main", &mut |_| ControlFlow::Continue(())))
    };
    let observations = match read() {
        Ok(history) => observations(&history),
        Err(err) => {
            println!("replay: reading failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    let started = Instant::now();
    let Ok(history) = read() else {
        println!("replay: reading failed");
        return ExitCode::FAILURE;
    };
    let read_time = started.elapsed();
    let Some(last) = history.commits.last().map(|c| c.tick) else {
        println!("replay: no commits");
        return ExitCode::FAILURE;
    };
    let seed = RepoSeed::from_root_commit(&history.commits[0].id);
    let basis = Basis::new(seed, history.grid, "main");
    let replayed = Prepared::new(basis.clone(), &history.commits, &observations).and_then(|p| {
        let mut state = SimState::new(basis);
        state.advance_through(&p, last).map(|()| state)
    });
    let state = match replayed {
        Ok(state) => state,
        Err(err) => {
            println!("replay: {err:?}");
            return ExitCode::FAILURE;
        }
    };
    let scene = project(&state);
    let elapsed = started.elapsed();
    let qualified: u32 = state.events.tally.values().map(|t| t.qualified).sum();
    let kinds = state.events.tally.len();
    if history.commits.len() != COMMITS + 3 || kinds < 10 || scene.validate().is_err() {
        println!(
            "replay: {} commits, {kinds} event kinds, scene {:?}",
            history.commits.len(),
            scene.validate()
        );
        return ExitCode::FAILURE;
    }
    let verdict = if elapsed < BUDGET {
        "ok"
    } else {
        "OVER BUDGET"
    };
    println!(
        "first-connect replay: {} commits, {} observations, {} ticks, {qualified} events \
         of {kinds} kinds in {elapsed:.2?} (read {read_time:.2?}, replay {:.2?}), budget \
         {BUDGET:?}: {verdict}",
        history.commits.len(),
        observations.len(),
        last.hours(),
        elapsed.saturating_sub(read_time)
    );
    if elapsed < BUDGET {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
