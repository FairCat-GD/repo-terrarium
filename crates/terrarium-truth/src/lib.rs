#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![deny(clippy::float_arithmetic)]
#![deny(clippy::as_conversions)]

pub mod canon;
pub mod domain;
pub mod fixed;
pub mod identity;
pub mod rng;
pub mod scene;
pub mod time;

#[cfg(test)]
mod test_support;

pub use canon::{Canonical, canonical_dump, canonical_sha256, from_canonical_dump};
pub use domain::{Climate, CommitFacts, Observation, ObservationKind, PresentationState, Reading};
pub use fixed::{Q16, Q32, log2_int};
pub use identity::{CommitId, RepoSeed, SIM_VERSION, SimVersion};
pub use rng::{Pcg32, SplitMix64, StreamTag};
pub use scene::SceneInput;
pub use time::{Tick, TickGrid};
