//! What a poll asks GitHub, and when the next one may ask: pure, so the client and the engine's
//! schedule share one answer.

use std::time::Duration;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use super::Slug;

/// The one host a poll reaches.
pub const API_HOST: &str = "api.github.com";
pub const API_BASE: &str = "https://api.github.com";
/// GitHub's media type and the REST API version the client was written against.
pub const ACCEPT: &str = "application/vnd.github+json";
pub const API_VERSION: (&str, &str) = ("X-GitHub-Api-Version", "2022-11-28");
/// How many runs a poll lists: enough to reach a decided commit past a few still running.
const PER_PAGE: u32 = 30;

/// Everything but RFC 3986's unreserved characters is encoded: a branch name may hold `/`,
/// `#`, `&`, `+`, `%`, `=` or any non-ASCII character.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// The product and its version, and nothing that names the person (no address, no account).
pub fn user_agent() -> String {
    format!("repo-terrarium/{}", env!("CARGO_PKG_VERSION"))
}

/// The path and query listing `branch`'s runs, every name encoded.
pub fn runs_path(slug: &Slug, branch: &str) -> String {
    format!(
        "/repos/{}/{}/actions/runs?branch={}&per_page={PER_PAGE}&exclude_pull_requests=true",
        utf8_percent_encode(slug.owner(), COMPONENT),
        utf8_percent_encode(slug.name(), COMPONENT),
        utf8_percent_encode(branch, COMPONENT),
    )
}

/// Why a poll learned nothing new.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CiError {
    /// No answer: the network, a timeout, a certificate, a connection cut short.
    Unreachable,
    /// GitHub refused the token: unknown, revoked, expired, or without read access to Actions.
    Refused,
    /// No such repository for this token.
    NotFound,
    /// GitHub asked to wait: `retry_after` seconds, or until `reset_at` (Unix seconds).
    RateLimited {
        retry_after: Option<u64>,
        reset_at: Option<i64>,
    },
    /// GitHub's own failure, with its status.
    Server(u16),
    /// An answer that is not the runs list.
    Malformed,
}

impl CiError {
    /// Whether the next poll may answer otherwise without the token changing: a network or
    /// service hiccup, not GitHub saying no.
    pub fn is_transient(self) -> bool {
        matches!(
            self,
            CiError::Unreachable
                | CiError::RateLimited { .. }
                | CiError::Server(_)
                | CiError::Malformed
        )
    }
}

/// When the next poll may ask, given how the last one went: every `every` while GitHub
/// answers; doubling from `every` to ten times it while it cannot be reached or fails; what
/// GitHub asks for when it limits the rate, else doubling to fifteen times `every`; fifteen
/// times `every` for a repository not found; and not at all once the token is refused, until
/// it changes.
#[derive(Clone, Debug)]
pub struct Backoff {
    every: Duration,
    streak: u32,
}

impl Backoff {
    pub fn new(every: Duration) -> Backoff {
        Backoff { every, streak: 0 }
    }

    /// The wait before the next poll after `last`, at `now` (Unix seconds); `None` for no poll
    /// until the token changes.
    pub fn after(&mut self, last: Result<(), CiError>, now: i64) -> Option<Duration> {
        let doubled = |streak: u32, cap: u32| {
            let factor = 2_u32.saturating_pow(streak.min(16)).min(cap);
            self.every.saturating_mul(factor)
        };
        let wait = match last {
            Ok(()) => {
                self.streak = 0;
                return Some(self.every);
            }
            Err(CiError::Refused) => {
                self.streak = 0;
                return None;
            }
            Err(CiError::NotFound) => self.every.saturating_mul(15),
            Err(CiError::RateLimited {
                retry_after,
                reset_at,
            }) => match (retry_after, reset_at) {
                (Some(seconds), _) => Duration::from_secs(seconds).max(self.every),
                (None, Some(at)) => {
                    Duration::from_secs(u64::try_from(at.saturating_sub(now)).unwrap_or(0))
                        .max(self.every)
                }
                (None, None) => doubled(self.streak, 15),
            },
            Err(CiError::Unreachable | CiError::Server(_) | CiError::Malformed) => {
                doubled(self.streak, 10)
            }
        };
        self.streak = self.streak.saturating_add(1);
        Some(wait)
    }

    /// Back to every `every`: the token changed, or the machine woke.
    pub fn reset(&mut self) {
        self.streak = 0;
    }
}
