//! Shared test settings and helpers.

pub mod ecology;

use proptest::test_runner::{Config, RngSeed};

/// Property tests run from a fixed seed, so every run checks the same cases, and write no
/// regression files into the source tree.
pub fn proptest_config() -> Config {
    Config {
        rng_seed: RngSeed::Fixed(0x7465_7272_6172_6975),
        failure_persistence: None,
        ..Config::default()
    }
}

/// Monday 2024-01-08, 00:00 UTC: a midnight, so tick 24k begins a UTC day.
pub const ORIGIN: i64 = 1_704_672_000;
pub const HOUR: i64 = 3_600;

pub fn grid() -> crate::TickGrid {
    crate::TickGrid::from_origin(ORIGIN).expect("a whole hour")
}

pub fn basis() -> crate::Basis {
    crate::Basis::new(crate::RepoSeed::from_bytes([7; 32]), grid(), "main")
}

/// The commit id written as `n` in 40 hex digits.
pub fn id(n: u32) -> crate::CommitId {
    crate::CommitId::from_hex(&format!("{n:040x}")).expect("40 hex digits")
}

/// A commit on the default branch's first-parent chain, adding ten lines.
pub fn commit(n: u32, tick: u32) -> crate::CommitFacts {
    crate::CommitFacts {
        id: id(n),
        tick: crate::Tick::new(tick),
        parents: 1,
        on_default_first_parent: true,
        added: 10,
        removed: 0,
        deleted_dirs: Vec::new(),
        renames: Vec::new(),
        tags: Vec::new(),
    }
}

/// An observation recorded a minute past the hour `hours` after the origin, at the default tip
/// `head` on `main`.
pub fn observation(
    value: crate::domain::ObservationValue,
    hours: u32,
    head: u32,
    normalized: Option<crate::Q16>,
) -> crate::Observation {
    crate::Observation {
        id: 0,
        recorded_at: ORIGIN + i64::from(hours) * HOUR + 60,
        head: crate::domain::ObservationHead {
            commit: id(head),
            branch: Some("main".to_owned()),
            at_default_tip: true,
        },
        normalized,
        confidence: crate::Q16::ONE,
        value,
    }
}

pub fn coverage(hours: u32, head: u32, value: crate::Q16) -> crate::Observation {
    observation(
        crate::domain::ObservationValue::Coverage,
        hours,
        head,
        Some(value),
    )
}

/// A weather run of `source` recorded `hours` after the origin with HEAD at `head` on `branch`.
pub fn weather(
    source: crate::domain::WeatherSource,
    hours: u32,
    head: u32,
    branch: &str,
    pass: Option<bool>,
) -> crate::Observation {
    let mut observation = observation(
        crate::domain::ObservationValue::Weather { source },
        hours,
        head,
        pass.map(|pass| {
            if pass {
                crate::Q16::ONE
            } else {
                crate::Q16::ZERO
            }
        }),
    );
    observation.head.branch = Some(branch.to_owned());
    observation.head.at_default_tip = branch == "main";
    observation
}

pub fn ci(branch: &str) -> crate::domain::WeatherSource {
    crate::domain::WeatherSource::Ci {
        branch: branch.to_owned(),
    }
}

/// `per_day[d]` commits on day `d`, from 09:00, each adding ten lines.
pub fn daily_commits(per_day: &[u32]) -> Vec<crate::CommitFacts> {
    let mut commits = Vec::new();
    for (day, &count) in (0_u32..).zip(per_day) {
        for k in 0..count {
            let n = u32::try_from(commits.len()).expect("few") + 1;
            commits.push(commit(n, day * 24 + 9 + k));
        }
    }
    commits
}

/// A dead-code report of `unused` of 100 exported items, known unless `known` is false.
pub fn dead_code(hours: u32, head: u32, unused: u32, known: bool) -> crate::Observation {
    let value = crate::Q16::from_ratio(100 - i32::try_from(unused.min(100)).expect("small"), 100);
    observation(
        crate::domain::ObservationValue::DeadCode {
            unused_items: unused,
            exported_items: 100,
        },
        hours,
        head,
        known.then_some(value),
    )
}

/// A ref rewrite of `reference` recorded `hours` after the origin.
pub fn rewrite(
    hours: u32,
    reference: &str,
    rewritten: Option<u32>,
    fossils: Vec<crate::CommitId>,
) -> crate::Observation {
    observation(
        crate::domain::ObservationValue::RefRewrite {
            reference: reference.to_owned(),
            old: id(9_999),
            new: id(9_998),
            rewritten,
            fossils,
            left: Vec::new(),
        },
        hours,
        1,
        None,
    )
}

/// A ref rewrite of `main` recorded `hours` after the origin that took `left` off the chain.
pub fn rewrite_left(
    hours: u32,
    rewritten: Option<u32>,
    left: Vec<crate::domain::LeftCommit>,
) -> crate::Observation {
    let mut observation = rewrite(hours, "refs/heads/main", rewritten, Vec::new());
    if let crate::domain::ObservationValue::RefRewrite { left: taken, .. } = &mut observation.value
    {
        *taken = left;
    }
    observation
}

/// The commit `n` taken off the chain, dated `hours` after the origin, that deleted a directory
/// of `lines` (none for 0).
pub fn left(n: u32, hours: u32, lines: u64) -> crate::domain::LeftCommit {
    crate::domain::LeftCommit {
        commit: id(n),
        time: ORIGIN + i64::from(hours) * HOUR,
        deleted_dirs: (lines > 0)
            .then(|| crate::domain::DeletedDir {
                path: format!("src/gone{n}"),
                lines,
            })
            .into_iter()
            .collect(),
    }
}
