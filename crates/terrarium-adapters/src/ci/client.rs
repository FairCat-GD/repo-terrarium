//! The GitHub Actions client: the one place a request leaves the machine (I-6), and the only
//! code that names `ureq` (`cargo xtask verify`'s graph stage).
//!
//! It asks `api.github.com` and nothing else: TLS checks GitHub's certificate against
//! Mozilla's root list compiled in, so no other host is asked about it; no proxy from the
//! environment is used; and no redirect is followed but one back to `api.github.com`, which a
//! renamed repository answers with. Each branch's last answer is kept with its `ETag`, so an
//! unchanged list costs GitHub a `304` and no rate limit.

use std::collections::BTreeMap;
use std::time::Duration;

use ureq::Agent;
use ureq::config::ConfigBuilder;
use ureq::typestate::AgentScope;

use super::request::{ACCEPT, API_BASE, API_VERSION, CiError, runs_path, user_agent};
use super::runs::{Runs, Verdict, verdict};
use super::{Slug, Token};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const WHOLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Thirty runs are tens of kilobytes; anything past this is not the runs list.
const MAX_BODY: u64 = 2 * 1024 * 1024;

/// What a poll asks of GitHub, or of a stand-in in tests.
pub trait Fetch: Send {
    /// The verdict of `branch`'s runs, `None` when no commit listed has one.
    fn runs(
        &mut self,
        slug: &Slug,
        branch: &str,
        token: &Token,
    ) -> Result<Option<Verdict>, CiError>;

    /// Forgets every earlier answer: the token changed, and they were its.
    fn forget(&mut self);
}

/// What GitHub said last for one branch's runs.
struct Known {
    etag: String,
    verdict: Option<Verdict>,
}

/// The client for `api.github.com`.
pub struct GitHub {
    agent: Agent,
    base: String,
    known: BTreeMap<String, Known>,
}

/// One response, read.
enum Answer {
    Body {
        etag: Option<String>,
        bytes: Vec<u8>,
    },
    NotModified,
    Moved(String),
}

impl GitHub {
    pub fn new() -> GitHub {
        GitHub::on(API_BASE, true)
    }

    fn on(base: &str, https_only: bool) -> GitHub {
        GitHub {
            agent: settled(Agent::config_builder(), https_only).build().into(),
            base: base.to_owned(),
            known: BTreeMap::new(),
        }
    }

    /// A client for a test server on the loopback interface, over plain HTTP.
    #[cfg(test)]
    pub(crate) fn on_loopback(base: &str) -> GitHub {
        GitHub::on(base, false)
    }

    #[cfg(test)]
    pub(crate) fn agent(&self) -> &Agent {
        &self.agent
    }

    fn call(&self, url: &str, etag: Option<&str>, token: &Token) -> Result<Answer, CiError> {
        let mut request = self
            .agent
            .get(url)
            .header("Authorization", format!("Bearer {}", token.secret()))
            .header(API_VERSION.0, API_VERSION.1);
        if let Some(etag) = etag {
            request = request.header("If-None-Match", etag);
        }
        let mut response = request.call().map_err(|_| CiError::Unreachable)?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let status = response.status().as_u16();
        match status {
            200 => {
                let etag = header("etag");
                let bytes = response
                    .body_mut()
                    .with_config()
                    .limit(MAX_BODY)
                    .read_to_vec()
                    .map_err(|err| match err {
                        ureq::Error::BodyExceedsLimit(_) => CiError::Malformed,
                        _ => CiError::Unreachable,
                    })?;
                Ok(Answer::Body { etag, bytes })
            }
            304 => Ok(Answer::NotModified),
            301 | 302 | 307 | 308 => header("location")
                .map(Answer::Moved)
                .ok_or(CiError::NotFound),
            401 => Err(CiError::Refused),
            403 | 429 => {
                let retry_after = header("retry-after").and_then(|s| s.trim().parse().ok());
                let exhausted = header("x-ratelimit-remaining").as_deref() == Some("0");
                let reset_at = header("x-ratelimit-reset")
                    .and_then(|s| s.trim().parse().ok())
                    .filter(|_| exhausted);
                if status == 429 || retry_after.is_some() || exhausted {
                    Err(CiError::RateLimited {
                        retry_after,
                        reset_at,
                    })
                } else {
                    // A token without read access to the repository's Actions.
                    Err(CiError::Refused)
                }
            }
            404 => Err(CiError::NotFound),
            status => Err(CiError::Server(status)),
        }
    }
}

/// The client's settings, over whatever `builder` held: a proxy the environment named among
/// them is dropped.
fn settled(builder: ConfigBuilder<AgentScope>, https_only: bool) -> ConfigBuilder<AgentScope> {
    builder
        .https_only(https_only)
        .proxy(None)
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(WHOLE_TIMEOUT))
        .user_agent(user_agent())
        .accept(ACCEPT)
}

/// [`settled`], for the tests to start from a builder holding a proxy.
#[cfg(test)]
pub(crate) fn settled_over(builder: ConfigBuilder<AgentScope>) -> ConfigBuilder<AgentScope> {
    settled(builder, true)
}

impl Default for GitHub {
    fn default() -> GitHub {
        GitHub::new()
    }
}

impl Fetch for GitHub {
    fn runs(
        &mut self,
        slug: &Slug,
        branch: &str,
        token: &Token,
    ) -> Result<Option<Verdict>, CiError> {
        let path = runs_path(slug, branch);
        let etag = self.known.get(&path).map(|known| known.etag.as_str());
        let mut answer = self.call(&format!("{}{path}", self.base), etag, token)?;
        if let Answer::Moved(location) = answer {
            // Only back to the API itself, once: a renamed repository's new address.
            if !location.starts_with(&format!("{}/", self.base)) {
                return Err(CiError::NotFound);
            }
            answer = match self.call(&location, None, token)? {
                Answer::Moved(_) => return Err(CiError::NotFound),
                answer => answer,
            };
        }
        match answer {
            Answer::Body { etag, bytes } => {
                let runs: Runs = serde_json::from_slice(&bytes).map_err(|_| CiError::Malformed)?;
                let verdict = verdict(&runs.workflow_runs);
                match etag {
                    Some(etag) => {
                        self.known.insert(
                            path,
                            Known {
                                etag,
                                verdict: verdict.clone(),
                            },
                        );
                    }
                    None => {
                        self.known.remove(&path);
                    }
                }
                Ok(verdict)
            }
            Answer::NotModified => self
                .known
                .get(&path)
                .map(|known| known.verdict.clone())
                .ok_or(CiError::Malformed),
            Answer::Moved(_) => Err(CiError::NotFound),
        }
    }

    fn forget(&mut self) {
        self.known.clear();
    }
}
