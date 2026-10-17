//! The opt-in Knip run (I-5): what the worker starts, on a thread of its own so a run of
//! minutes never holds a refresh, and what comes back. Whether it may run is decided by
//! [`KnipRun::permitted`] alone, from the opt-in the settings hold for this repository.

use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use terrarium_adapters::exec::{Finished, KnipError, KnipRun, Tool};
use terrarium_truth::domain::ObservationHead;

/// A run to start: Knip, permitted in this repository, the person's node, and where HEAD was.
#[derive(Clone, Debug)]
pub struct KnipJob {
    pub run: KnipRun,
    pub node: Tool,
    pub head: ObservationHead,
}

/// What the worker does about Knip after a refresh.
#[derive(Debug)]
pub enum KnipCall {
    Start(KnipJob),
    /// The run going is no longer wanted: the opt-in was withdrawn, or Dead code switched off.
    Stop,
}

/// A run that ended: where HEAD was when it started, and its report or why there is none.
#[derive(Debug)]
pub struct KnipDone {
    pub head: ObservationHead,
    pub outcome: Result<Vec<u8>, KnipError>,
}

/// A run going, owned by the worker: stopping it kills Knip and waits for its thread.
pub(crate) struct Running {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Running {
    /// Starts `job`; its outcome goes to `done`, which hears that it could not run when its
    /// thread cannot start.
    pub(crate) fn start(job: KnipJob, done: impl Fn(KnipDone) + Send + Sync + 'static) -> Running {
        let KnipJob { run, node, head } = job;
        Running::start_with(head, move |stop| run.run(&node, stop), done)
    }

    /// Runs `run` for the HEAD `head` was at. A panic in it is heard as a run that could not
    /// start, so the worker never waits on a run that is gone.
    fn start_with(
        head: ObservationHead,
        run: impl FnOnce(&AtomicBool) -> Result<Vec<u8>, KnipError> + Send + 'static,
        done: impl Fn(KnipDone) + Send + Sync + 'static,
    ) -> Running {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let (done, failed) = {
            let done = Arc::new(done);
            (Arc::clone(&done), done)
        };
        let ran_for = head.clone();
        let thread = thread::Builder::new()
            .name("terrarium-knip".into())
            .spawn(move || {
                let outcome = panic::catch_unwind(AssertUnwindSafe(|| run(&flag)))
                    .unwrap_or_else(|_| Err(KnipError::CannotRun(io::Error::other("panicked"))));
                done(KnipDone {
                    head: ran_for,
                    outcome,
                });
            });
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(err) => {
                failed(KnipDone {
                    head,
                    outcome: Err(KnipError::CannotRun(err)),
                });
                None
            }
        };
        Running { stop, thread }
    }

    /// Stops the run and waits for it: Knip is killed within moments. A process Knip itself
    /// started may outlive it; stopping a whole process group would need code this crate
    /// cannot hold (I-9).
    pub(crate) fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The most of a version the repository's `package.json` names that the settings window shows.
const VERSION_SHOWN: usize = 32;

/// `text` cut to at most `most` characters.
fn capped(text: &str, most: usize) -> String {
    text.chars().take(most).collect()
}

/// What the settings window says of the Knip run; its wording is the app's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum KnipStatus {
    /// Not opted in for this repository, or Dead code is off.
    #[default]
    Off,
    /// Opted in; it runs when HEAD is at the default branch's tip.
    Waiting,
    Running,
    /// It ran, and found this many unused exports.
    Ran {
        unused: u64,
    },
    /// No node outside the repository.
    NoNode,
    NotInstalled,
    Before6(String),
    Outside,
    TimedOut,
    /// It ended without a report: its exit code (none for a signal) and the end of its stderr.
    Failed {
        code: Option<i32>,
        stderr: String,
    },
    /// What it printed is not a Knip report this reads.
    NotAReport,
    TooLarge,
    CannotRun,
}

impl KnipStatus {
    pub(crate) fn of(err: &KnipError) -> KnipStatus {
        match err {
            KnipError::NotInstalled => KnipStatus::NotInstalled,
            KnipError::Before6(version) => KnipStatus::Before6(capped(version, VERSION_SHOWN)),
            KnipError::Outside => KnipStatus::Outside,
            KnipError::CannotRun(_) => KnipStatus::CannotRun,
            KnipError::TimedOut => KnipStatus::TimedOut,
            KnipError::Stopped => KnipStatus::Waiting,
            KnipError::TooLarge => KnipStatus::TooLarge,
            KnipError::Failed { ended, stderr } => KnipStatus::Failed {
                code: match ended {
                    Finished::Code(code) => Some(*code),
                    Finished::Signal(_) => None,
                },
                stderr: stderr.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use terrarium_truth::CommitId;

    use super::*;

    fn head() -> ObservationHead {
        ObservationHead {
            commit: CommitId::from_hex(&"a".repeat(40)).unwrap(),
            branch: Some("main".into()),
            at_default_tip: true,
        }
    }

    #[test]
    fn a_run_that_panics_is_heard_as_one_that_could_not_run() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let into = Arc::clone(&heard);
        let running = Running::start_with(
            head(),
            |_| panic!("an injected Knip panic"),
            move |done| into.lock().unwrap().push(done),
        );
        let until = Instant::now() + Duration::from_secs(10);
        while heard.lock().unwrap().is_empty() && Instant::now() < until {
            thread::sleep(Duration::from_millis(5));
        }
        running.stop();
        let heard = heard.lock().unwrap();
        assert!(matches!(
            heard.as_slice(),
            [KnipDone {
                outcome: Err(KnipError::CannotRun(_)),
                ..
            }]
        ));
    }

    #[test]
    fn a_version_the_repository_names_is_shown_capped() {
        let long = "6".repeat(4096);
        assert_eq!(
            KnipStatus::of(&KnipError::Before6(long)),
            KnipStatus::Before6("6".repeat(VERSION_SHOWN))
        );
    }
}
