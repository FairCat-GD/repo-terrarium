#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

pub mod ci;
pub mod clock;
pub mod details;
pub mod headless;
pub mod knip;
pub mod lifecycle;
pub mod live;
pub mod observe;
pub mod schedule;
pub mod showing;
mod shown;
pub mod timelapse;
pub mod watch;
pub mod worker;

pub use ci::{CiConfig, CiStatus, POLL_EVERY};
pub use clock::{Clock, ManualClock, SystemClock};
pub use details::{Buried, DeadCodeFrom, Details, Files, Found, Raised};
pub use knip::KnipStatus;
pub use live::{
    Branch, ConnectError, EngineConfig, Live, News, NodeLookup, Problem, RefreshStats, Replay,
    Snapshot, Start, Tools,
};
pub use worker::{Engine, Sink, Update};
