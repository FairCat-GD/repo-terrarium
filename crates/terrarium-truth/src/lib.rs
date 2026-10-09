#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![deny(clippy::float_arithmetic)]
#![deny(clippy::as_conversions)]
#![deny(clippy::disallowed_types)]
#![deny(clippy::disallowed_methods)]

pub mod canon;
pub mod climate;
pub mod domain;
pub mod dump;
pub mod events;
pub mod fauna;
pub mod fixed;
pub mod habitat;
pub mod history;
pub mod identity;
pub mod landmarks;
pub mod project;
pub mod replay;
pub mod rng;
pub mod scene;
pub mod time;
pub mod timeline;
pub mod tuning;
pub mod vegetation;

#[cfg(test)]
mod ecology_tests;
#[cfg(test)]
mod test_support;

pub use canon::{Canonical, canonical_dump, canonical_sha256, from_canonical_dump};
pub use domain::{Climate, CommitFacts, Observation, ObservationKind, PresentationState, Reading};
pub use dump::{ReplayDump, dump};
pub use fixed::{Q16, Q32, log2_int};
pub use identity::{CommitId, RepoSeed, SIM_VERSION, SimVersion};
pub use project::{project, project_showing};
pub use replay::{Basis, Prepared, ReplayError, SimState, observation_tick};
pub use rng::{Pcg32, SplitMix64, StreamTag};
pub use scene::SceneInput;
pub use time::{Tick, TickGrid};

/// Drops the oldest of `items`, which are oldest first, until at most `cap` are left.
pub(crate) fn keep_newest<T>(items: &mut Vec<T>, cap: usize) {
    let excess = items.len().saturating_sub(cap);
    items.drain(..excess);
}
