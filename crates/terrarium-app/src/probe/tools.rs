//! The probe's tools step: the parts of the app that reach outside it, each tried once where
//! the probe runs: the keychain, node run the way Knip is, and the TLS the GitHub weather
//! speaks (without a connection). What it reports names no path and nothing a tool printed to
//! stderr, which carry the person's user name.

use std::thread;

use serde::Serialize;
use terrarium_adapters::ci::{Keychain, KeychainError, NativeKeychain, TlsCheck, Token, tls_check};
use terrarium_adapters::exec::{Finished, NodeCheck, RunCheck, check_node, find_node};
use terrarium_store::identifiers::{KEYCHAIN_ACCOUNT_PROBE, KEYCHAIN_SERVICE};
use winit::event_loop::EventLoopProxy;

use crate::strip::UserEvent;

/// The made-up secret the keychain check saves, reads back and removes.
const PROBE_SECRET: &str = "repo-terrarium-probe-check";

/// What the tools step found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolsReport {
    pub keychain: KeychainReport,
    /// `None` when no node was found.
    pub node: Option<NodeReport>,
    pub tls: TlsReport,
}

/// The keychain check. Each step runs on a thread of its own, as the app saves the token on
/// its settings thread and reads it on its GitHub thread; Windows' store says a read on one
/// thread can miss a write on another, so a read that misses is tried once more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeychainReport {
    pub saved: Outcome,
    pub read: Outcome,
    /// The second try, when the first did not find the secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_again: Option<Outcome>,
    pub removed: Outcome,
    /// A read after the removal: `missing` is right.
    pub gone: Outcome,
}

/// How one keychain step went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Done,
    /// The read found the secret saved.
    Matched,
    Missing,
    /// The read found something else.
    Different,
    Unavailable,
    Refused,
    Failed,
    /// The step's thread panicked.
    Panicked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeReport {
    pub version: RunReport,
    pub script: RunReport,
    pub temp_in_system_root: Option<bool>,
}

/// How one run of node went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum RunReport {
    Answered { answer: String },
    Unexpected,
    Failed { code: i32 },
    CannotRun,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent checks, each passed or not"
)]
pub struct TlsReport {
    pub random: bool,
    pub exchange: bool,
    pub config: bool,
    pub roots: usize,
    pub passed: bool,
}

/// Runs the checks on a thread of their own, and sends what they found to the event loop.
pub fn start(proxy: EventLoopProxy<UserEvent>) {
    let started = thread::Builder::new()
        .name("terrarium-probe-tools".into())
        .spawn(move || {
            let keychain = NativeKeychain::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT_PROBE);
            let report = ToolsReport {
                keychain: check_keychain(&keychain),
                node: find_node(None).map(|node| node_report(&check_node(&node))),
                tls: tls_report(tls_check()),
            };
            let _ = proxy.send_event(UserEvent::ProbeTools(Box::new(report)));
        });
    // Without a thread there is nothing to report; the step's note still is.
    drop(started);
}

/// Saves, reads, removes and reads again, each on a thread of its own.
pub fn check_keychain(keychain: &dyn Keychain) -> KeychainReport {
    let secret = Token::new(PROBE_SECRET);
    let on_thread = |step: &(dyn Fn(&dyn Keychain) -> Outcome + Sync)| {
        thread::scope(|scope| {
            scope
                .spawn(|| step(keychain))
                .join()
                .unwrap_or(Outcome::Panicked)
        })
    };
    let read = |keychain: &dyn Keychain| match keychain.token() {
        Ok(Some(found)) if Some(&found) == secret.as_ref() => Outcome::Matched,
        Ok(Some(_)) => Outcome::Different,
        Ok(None) => Outcome::Missing,
        Err(err) => failed(err),
    };
    let saved = on_thread(&|keychain: &dyn Keychain| match &secret {
        Some(secret) => keychain
            .save(secret)
            .map_or_else(failed, |()| Outcome::Done),
        None => Outcome::Failed,
    });
    let first = on_thread(&read);
    let read_again = (first != Outcome::Matched).then(|| on_thread(&read));
    let removed = on_thread(&|keychain: &dyn Keychain| {
        keychain.remove().map_or_else(failed, |()| Outcome::Done)
    });
    let gone = on_thread(&read);
    KeychainReport {
        saved,
        read: first,
        read_again,
        removed,
        gone,
    }
}

fn failed(err: KeychainError) -> Outcome {
    match err {
        KeychainError::Unavailable => Outcome::Unavailable,
        KeychainError::Refused => Outcome::Refused,
        KeychainError::Failed => Outcome::Failed,
    }
}

fn node_report(check: &NodeCheck) -> NodeReport {
    NodeReport {
        version: run_report(&check.version),
        script: run_report(&check.script),
        temp_in_system_root: check.temp_in_system_root,
    }
}

fn run_report(check: &RunCheck) -> RunReport {
    match check {
        RunCheck::Answered(answer) => RunReport::Answered {
            answer: answer.clone(),
        },
        RunCheck::Unexpected => RunReport::Unexpected,
        RunCheck::Failed(Finished::Code(code) | Finished::Signal(code)) => {
            RunReport::Failed { code: *code }
        }
        RunCheck::CannotRun => RunReport::CannotRun,
        RunCheck::TimedOut => RunReport::TimedOut,
    }
}

fn tls_report(check: TlsCheck) -> TlsReport {
    TlsReport {
        random: check.random,
        exchange: check.exchange,
        config: check.config,
        roots: check.roots,
        passed: check.passed,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use terrarium_adapters::ci::MemoryKeychain;

    use super::*;

    #[test]
    fn the_keychain_check_saves_reads_back_removes_and_finds_it_gone() {
        let keychain = MemoryKeychain::default();
        let report = check_keychain(&keychain);
        assert_eq!(
            report,
            KeychainReport {
                saved: Outcome::Done,
                read: Outcome::Matched,
                read_again: None,
                removed: Outcome::Done,
                gone: Outcome::Missing,
            }
        );
        assert_eq!(keychain.held(), None, "nothing is left behind");
    }

    /// A keychain whose first read misses a write made on another thread, as Windows' store
    /// says one can.
    #[derive(Default)]
    struct SlowToSee {
        inner: MemoryKeychain,
        reads: AtomicUsize,
    }

    impl Keychain for SlowToSee {
        fn token(&self) -> Result<Option<Token>, KeychainError> {
            if self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
                return Ok(None);
            }
            self.inner.token()
        }
        fn save(&self, token: &Token) -> Result<(), KeychainError> {
            self.inner.save(token)
        }
        fn remove(&self) -> Result<(), KeychainError> {
            self.inner.remove()
        }
    }

    #[test]
    fn a_read_that_misses_is_tried_once_more_and_both_are_reported() {
        let report = check_keychain(&SlowToSee::default());
        assert_eq!(report.read, Outcome::Missing);
        assert_eq!(report.read_again, Some(Outcome::Matched));
        assert_eq!(report.gone, Outcome::Missing);
    }

    #[test]
    fn the_report_names_outcomes_and_no_path() {
        let report = ToolsReport {
            keychain: check_keychain(&MemoryKeychain::default()),
            node: Some(node_report(&NodeCheck {
                version: RunCheck::Answered("v22.11.0".to_owned()),
                script: RunCheck::Failed(Finished::Code(1)),
                temp_in_system_root: None,
            })),
            tls: tls_report(tls_check()),
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["keychain"]["read"], "matched");
        assert!(json["keychain"].get("read_again").is_none());
        assert_eq!(json["node"]["version"]["outcome"], "answered");
        assert_eq!(json["node"]["version"]["answer"], "v22.11.0");
        assert_eq!(json["node"]["script"]["code"], 1);
        assert_eq!(json["tls"]["passed"], true);
    }
}
