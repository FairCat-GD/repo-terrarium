//! Weather from GitHub Actions (I-6): which repository, what its runs say, the client that
//! asks, and the token it asks with.
//!
//! - [`Slug`]: the repository, typed in the settings or read from `origin`.
//! - [`verdict`]: a branch's runs, newest commit first, as pass or fail.
//! - [`GitHub`]: the client, behind [`Fetch`] so the engine's tests ask a stand-in.
//! - [`Backoff`]: when the next poll may ask.
//! - [`Keychain`]: where the token is kept; [`Token`] never shows itself.
//!
//! Nothing here runs unless a token is saved and a repository is known; with neither, the app
//! makes no connection (N-09).

mod client;
mod keychain;
mod request;
mod runs;
mod slug;
#[cfg(test)]
mod tests;

pub use client::{Fetch, GitHub};
#[cfg(any(test, feature = "fixture"))]
pub use keychain::MemoryKeychain;
pub use keychain::{Keychain, KeychainError, NativeKeychain, Token};
pub use request::{API_HOST, Backoff, CiError, runs_path, user_agent};
pub use runs::{Run, Runs, Verdict, verdict};
pub use slug::Slug;
