//! Shared test settings.

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
