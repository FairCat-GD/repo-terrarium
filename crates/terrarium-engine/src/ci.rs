//! The GitHub Actions poll (N-09): a thread of its own asks GitHub, so a slow network never
//! holds a refresh, and the worker decides when it asks.
//!
//! - **The poller** holds the client and the token. It reads the keychain the first time it is
//!   asked, and again only after the token changed, so a keychain that asks the person asks
//!   once, and one with no token is not read every minute. It asks the current branch, then the default branch when the
//!   current one has no run that decides, and answers through the worker's channel.
//! - **[`Polling`]** is the worker's clock for it: every `every` while GitHub answers, longer
//!   while it cannot ([`Backoff`]), never while the token is refused. It also says when GitHub
//!   has been out of reach long enough for its weather to stop counting: three failed polls in
//!   a row over at least ten periods, counted from the first failure after an answer and only
//!   while the machine is awake, since a wake starts the count again.
//! - Nothing here runs without a [`CiConfig`]; the app gives one, and with no token saved and
//!   no repository known the poller never builds a request.

use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use terrarium_adapters::ci::{
    Backoff, CiError, Fetch, Keychain, KeychainError, Slug, Token, Verdict,
};

/// What the engine polls GitHub with: the keychain the token is in, a maker of the client, and
/// the period.
#[derive(Clone)]
pub struct CiConfig {
    pub keychain: Arc<dyn Keychain>,
    pub fetch: Arc<dyn Fn() -> Box<dyn Fetch> + Send + Sync>,
    /// A minute in the app; tests poll in milliseconds.
    pub every: Duration,
}

/// The poll's period in the app.
pub const POLL_EVERY: Duration = Duration::from_secs(60);

/// What the worker asks the poller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub slug: Slug,
    /// The branches to ask, in order: HEAD's, then the default one.
    pub branches: Vec<String>,
}

/// What a poll learned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// No token is saved; `removed` when one was, earlier in the session.
    NoToken {
        removed: bool,
    },
    /// The keychain could not be read.
    Keychain(KeychainError),
    /// The first branch asked whose runs decide, and their verdict; `None` when none did.
    Decided(Option<(String, Verdict)>),
    Failed(CiError),
}

/// A poll's answer, for the repository it asked about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answered {
    pub slug: Slug,
    pub read: Read,
}

enum Job {
    Ask(Ask),
    TokenChanged,
}

/// The poller's thread. Dropping it ends the thread once a poll in progress has answered.
pub(crate) struct Poller {
    jobs: Sender<Job>,
}

impl Poller {
    /// Starts the poller; each answer goes to `answer`. `None` when its thread cannot start.
    pub(crate) fn start(
        config: &CiConfig,
        answer: impl Fn(Answered) + Send + 'static,
    ) -> Option<Poller> {
        let (jobs, rx) = mpsc::channel::<Job>();
        let keychain = Arc::clone(&config.keychain);
        let fetch = Arc::clone(&config.fetch);
        thread::Builder::new()
            .name("terrarium-ci".into())
            .spawn(move || {
                let mut client = fetch();
                // The keychain as last read, until the token changes: a keychain that asks the
                // person, or refuses, does so once.
                let mut held: Option<Result<Option<Token>, KeychainError>> = None;
                let mut had_token = false;
                for job in rx {
                    match job {
                        Job::TokenChanged => {
                            held = None;
                            client.forget();
                        }
                        Job::Ask(ask) => {
                            let read = match held.get_or_insert_with(|| keychain.token()) {
                                Err(err) => Read::Keychain(*err),
                                Ok(None) => Read::NoToken {
                                    removed: std::mem::take(&mut had_token),
                                },
                                Ok(Some(token)) => {
                                    had_token = true;
                                    ask_branches(client.as_mut(), &ask, token)
                                }
                            };
                            answer(Answered {
                                slug: ask.slug,
                                read,
                            });
                        }
                    }
                }
            })
            .ok()?;
        Some(Poller { jobs })
    }

    pub(crate) fn ask(&self, ask: Ask) {
        let _ = self.jobs.send(Job::Ask(ask));
    }

    pub(crate) fn token_changed(&self) {
        let _ = self.jobs.send(Job::TokenChanged);
    }
}

/// Asks each branch in turn until one's runs decide; a failure ends the poll.
fn ask_branches(client: &mut dyn Fetch, ask: &Ask, token: &Token) -> Read {
    for branch in &ask.branches {
        match client.runs(&ask.slug, branch, token) {
            Ok(Some(verdict)) => return Read::Decided(Some((branch.clone(), verdict))),
            Ok(None) => {}
            Err(err) => return Read::Failed(err),
        }
    }
    Read::Decided(None)
}

/// How many failed polls in a row, and over how many periods, make GitHub out of reach.
const OUT_OF_REACH_POLLS: u32 = 3;
const OUT_OF_REACH_PERIODS: u32 = 10;

/// When the worker asks next, and whether GitHub is out of reach (see the module's doc). Its
/// times are the worker's monotonic clock.
#[derive(Clone, Debug)]
pub(crate) struct Polling {
    every: Duration,
    backoff: Backoff,
    /// When the next poll is due; `None` while one is out, or once the token was refused.
    next: Option<Duration>,
    asking: bool,
    /// The first failure since GitHub last answered, and how many in a row.
    failing_since: Option<Duration>,
    failures: u32,
}

impl Polling {
    /// A first poll at once.
    pub(crate) fn new(every: Duration, mono: Duration) -> Polling {
        Polling {
            every,
            backoff: Backoff::new(every),
            next: Some(mono),
            asking: false,
            failing_since: None,
            failures: 0,
        }
    }

    /// Whether a poll is due at `mono`; taking it marks it out.
    pub(crate) fn take(&mut self, mono: Duration) -> bool {
        if self.asking || self.next.is_none_or(|next| mono < next) {
            return false;
        }
        self.asking = true;
        self.next = None;
        true
    }

    /// Nothing to ask at `mono` (no repository known): look again a period later.
    pub(crate) fn skipped(&mut self, mono: Duration) {
        self.asking = false;
        self.next = Some(mono + self.every);
    }

    /// How long until the next poll is due, if one is.
    pub(crate) fn wait(&self, mono: Duration) -> Option<Duration> {
        self.next.map(|next| next.saturating_sub(mono))
    }

    /// Takes in a poll's answer at `mono` (wall `now`, for GitHub's reset times) and schedules
    /// the next; true when GitHub has now been out of reach long enough (see the module's doc).
    pub(crate) fn answered(&mut self, read: &Read, mono: Duration, now: i64) -> bool {
        self.asking = false;
        let last = match read {
            Read::Failed(err) => Err(*err),
            _ => Ok(()),
        };
        self.next = self.backoff.after(last, now).map(|wait| mono + wait);
        match read {
            Read::Failed(err) if err.is_transient() => {
                let since = *self.failing_since.get_or_insert(mono);
                self.failures = self.failures.saturating_add(1);
                self.failures >= OUT_OF_REACH_POLLS
                    && mono.saturating_sub(since) >= self.every.saturating_mul(OUT_OF_REACH_PERIODS)
            }
            _ => {
                self.failing_since = None;
                self.failures = 0;
                false
            }
        }
    }

    /// The token changed, or the machine woke: ask at once, and start counting failures again.
    pub(crate) fn restart(&mut self, mono: Duration) {
        self.backoff.reset();
        self.failing_since = None;
        self.failures = 0;
        if !self.asking {
            self.next = Some(mono);
        }
    }

    /// The repository is missing or the engine halted: no poll until [`Polling::restart`].
    pub(crate) fn pause(&mut self) {
        self.next = None;
    }
}

/// What the settings window says of the CI weather; its wording is the app's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CiStatus {
    /// This build polls nothing (no keychain was given), or no answer came yet.
    #[default]
    Quiet,
    /// No GitHub repository: `origin` is not on GitHub and none was typed.
    NoRepository,
    NoToken,
    /// GitHub's runs are being read.
    Reading(Slug),
    Refused(Slug),
    NotFound(Slug),
    /// GitHub could not be reached; the weather follows the local results meanwhile.
    Unreachable(Slug),
    /// GitHub asked to wait.
    Waiting(Slug),
    Keychain(KeychainError),
}

impl CiStatus {
    pub(crate) fn of(answered: &Answered) -> CiStatus {
        let slug = answered.slug.clone();
        match &answered.read {
            Read::NoToken { .. } => CiStatus::NoToken,
            Read::Keychain(err) => CiStatus::Keychain(*err),
            Read::Decided(_) => CiStatus::Reading(slug),
            Read::Failed(CiError::Refused) => CiStatus::Refused(slug),
            Read::Failed(CiError::NotFound) => CiStatus::NotFound(slug),
            Read::Failed(CiError::RateLimited { .. }) => CiStatus::Waiting(slug),
            Read::Failed(_) => CiStatus::Unreachable(slug),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed() -> Read {
        Read::Failed(CiError::Unreachable)
    }

    #[test]
    fn a_poll_is_due_at_once_then_every_period_one_at_a_time() {
        let every = Duration::from_secs(60);
        let secs = Duration::from_secs;
        let mut polling = Polling::new(every, secs(5));
        assert!(polling.take(secs(5)));
        assert!(!polling.take(secs(6)), "one at a time");
        assert_eq!(polling.wait(secs(6)), None);
        assert!(!polling.answered(&Read::Decided(None), secs(10), 0));
        assert_eq!(polling.wait(secs(10)), Some(every));
        assert!(!polling.take(secs(69)));
        assert!(polling.take(secs(70)));
        polling.answered(&Read::Failed(CiError::Refused), secs(71), 0);
        assert_eq!(
            polling.wait(secs(71)),
            None,
            "never while the token is refused"
        );
        polling.restart(secs(80));
        assert!(polling.take(secs(80)));
        polling.skipped(secs(81));
        assert_eq!(polling.wait(secs(81)), Some(every));
    }

    #[test]
    fn github_is_out_of_reach_after_three_failures_over_ten_periods_awake() {
        let every = Duration::from_secs(60);
        let mins = |m: u64| Duration::from_secs(m * 60);
        let mut polling = Polling::new(every, Duration::ZERO);
        // Three failures inside ten minutes are not enough; the third past them is.
        assert!(!polling.answered(&failed(), mins(0), 0));
        assert!(!polling.answered(&failed(), mins(1), 0));
        assert!(!polling.answered(&failed(), mins(3), 0));
        assert!(polling.answered(&failed(), mins(10), 0));
        // An answer starts the count again.
        assert!(!polling.answered(&Read::Decided(None), mins(11), 0));
        assert!(!polling.answered(&failed(), mins(12), 0));
        assert!(!polling.answered(&failed(), mins(13), 0));
        assert!(!polling.answered(&failed(), mins(14), 0));
        // A wake does too, however long the failures have run.
        polling.restart(mins(30));
        assert!(!polling.answered(&failed(), mins(30), 0));
        assert!(!polling.answered(&failed(), mins(31), 0));
        assert!(!polling.answered(&failed(), mins(32), 0));
        assert!(polling.answered(&failed(), mins(40), 0));
        // GitHub refusing is no outage: it answered.
        polling.restart(mins(50));
        for at in [50, 60, 70, 80] {
            assert!(!polling.answered(&Read::Failed(CiError::NotFound), mins(at), 0));
        }
    }
}
