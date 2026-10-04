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
pub mod fixed;
pub mod habitat;
pub mod history;
pub mod identity;
pub mod project;
pub mod replay;
pub mod rng;
pub mod scene;
pub mod time;
pub mod timeline;
pub mod tuning;

#[cfg(test)]
mod test_support;

pub use canon::{Canonical, canonical_dump, canonical_sha256, from_canonical_dump};
pub use domain::{Climate, CommitFacts, Observation, ObservationKind, PresentationState, Reading};
pub use dump::{ReplayDump, dump};
pub use fixed::{Q16, Q32, log2_int};
pub use identity::{CommitId, RepoSeed, SIM_VERSION, SimVersion};
pub use project::project;
pub use replay::{Basis, Prepared, ReplayError, SimState, observation_tick};
pub use rng::{Pcg32, SplitMix64, StreamTag};
pub use scene::SceneInput;
pub use time::{Tick, TickGrid};
