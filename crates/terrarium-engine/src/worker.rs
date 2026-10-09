//! The engine's thread (I-16): it connects, then refreshes whenever the schedule says, and
//! sends what the strip shows through a sink the app turns into `EventLoopProxy` events. The UI
//! thread never waits on it, except for connect's first answer before the window opens.
//!
//! One channel carries everything the thread hears: the app's requests and what the watcher
//! saw. Each wait ends at the schedule's next deadline, so the thread sleeps between
//! events and looks at the wall clock at least once a minute.
//!
//! **A panic** in a refresh is caught. The pipeline is dropped, the scene freezes, `Halted` is
//! sent, and the thread only waits to be shut down. A panic while connecting ends the thread,
//! and `spawn` answers `Halted`. The panic hook has already printed either.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::clock::Clock;
use crate::live::{ConnectError, Done, EngineConfig, Live, Problem, Snapshot, Start};
use crate::schedule::{DEBOUNCE, Event, Reach, Refresh, Schedule};
use crate::watch::{Touched, WatchSet, Watcher};

/// How long shutting down waits for the thread before leaving it to end with the process.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(1);

/// What the engine tells the strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    /// A long history read's percent.
    Progress(u8),
    Snapshot(Box<Snapshot>),
    /// The repository cannot be read: the scene freezes and no time passes until it can.
    RepoNotFound,
    /// A panic stopped the engine for this session (I-16).
    Halted,
    /// A refresh failed in a way the next may not; sent once until it changes.
    Problem(Problem),
}

pub type Sink = Box<dyn FnMut(Update) + Send>;

/// What the thread hears.
enum Message {
    Touched(Touched),
    Wake,
    Refresh,
    Shutdown,
}

/// What the worker runs: [`Live`], or a stand-in in tests.
pub(crate) trait Pipeline {
    fn refresh(
        &mut self,
        refresh: Refresh,
        progress: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
    ) -> Done;

    fn watch_set(&self) -> Option<&WatchSet>;
}

impl Pipeline for Live {
    fn refresh(
        &mut self,
        refresh: Refresh,
        progress: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
    ) -> Done {
        Live::refresh(self, refresh, progress, cancel)
    }

    fn watch_set(&self) -> Option<&WatchSet> {
        Live::watch_set(self)
    }
}

/// The running engine. Dropping it shuts it down.
pub struct Engine {
    tx: Sender<Message>,
    cancel: Arc<AtomicBool>,
    /// Disconnects when the thread ends.
    finished: Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl Engine {
    /// Starts the engine and waits for connect's answer: what the strip starts from, or why
    /// nothing could be connected.
    pub fn spawn(config: EngineConfig, sink: Sink) -> Result<(Engine, Start), ConnectError> {
        let clock = Arc::clone(&config.clock);
        Engine::spawn_pipeline(
            move || {
                Ok(Live::connect(&config)?.map(|live| {
                    let start = live.start();
                    (live, start)
                }))
            },
            clock,
            sink,
        )
    }

    pub(crate) fn spawn_pipeline<P, F>(
        connect: F,
        clock: Arc<dyn Clock>,
        sink: Sink,
    ) -> Result<(Engine, Start), ConnectError>
    where
        P: Pipeline,
        F: FnOnce() -> Result<Option<(P, Start)>, ConnectError> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished) = mpsc::channel::<()>();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            rx,
            tx: tx.clone(),
            sink,
            clock,
            cancel: Arc::clone(&cancel),
        };
        let thread = thread::Builder::new()
            .name("terrarium-engine".into())
            .spawn(move || {
                let _finished = finished_tx;
                // A panic here ends the thread before it answers, which `spawn` reads as
                // `Halted`.
                let pipeline = match connect() {
                    Ok(Some((pipeline, start))) => {
                        let _ = started_tx.send(Ok(start));
                        pipeline
                    }
                    Ok(None) => {
                        let _ = started_tx.send(Ok(Start::NotConnected));
                        return;
                    }
                    Err(err) => {
                        let _ = started_tx.send(Err(err));
                        return;
                    }
                };
                worker.run(pipeline);
            })
            .map_err(ConnectError::Thread)?;
        let engine = Engine {
            tx,
            cancel,
            finished,
            thread: Some(thread),
        };
        let start = started_rx.recv().map_err(|_| ConnectError::Halted)??;
        Ok((engine, start))
    }

    /// The machine woke: catch up at once.
    pub fn wake(&self) {
        let _ = self.tx.send(Message::Wake);
    }

    /// Refreshes everything at once.
    pub fn refresh(&self) {
        let _ = self.tx.send(Message::Refresh);
    }

    /// Stops the engine: a history read in progress stops at its next commit. Waits up to a
    /// second, then leaves the thread to end with the process.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Message::Shutdown);
        if let Some(thread) = self.thread.take()
            && self.finished.recv_timeout(SHUTDOWN_WAIT) == Err(RecvTimeoutError::Disconnected)
        {
            let _ = thread.join();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The running watch, and the set last tried, whether or not it started.
#[derive(Default)]
struct Watching {
    watcher: Option<Watcher>,
    tried: Option<WatchSet>,
}

struct Worker {
    rx: Receiver<Message>,
    /// Handed to each watcher.
    tx: Sender<Message>,
    sink: Sink,
    clock: Arc<dyn Clock>,
    cancel: Arc<AtomicBool>,
}

impl Worker {
    fn run<P: Pipeline>(mut self, pipeline: P) {
        let mut pipeline = Some(pipeline);
        let started = Instant::now();
        let mut schedule = Schedule::new(self.clock.wall_secs());
        let mut watching = Watching::default();
        let mut reach = Reach::Found;
        let mut problem: Option<Problem> = None;
        loop {
            let Some(running) = pipeline.as_mut() else {
                // Halted: only a shutdown is answered.
                match self.rx.recv() {
                    Ok(Message::Shutdown) | Err(_) => return,
                    Ok(_) => continue,
                }
            };
            if let Some(refresh) = schedule.take(started.elapsed(), self.clock.wall_secs()) {
                let sink = &mut self.sink;
                let cancel = &self.cancel;
                let ran = panic::catch_unwind(AssertUnwindSafe(|| {
                    running.refresh(
                        refresh,
                        &mut |percent| sink(Update::Progress(percent)),
                        cancel,
                    )
                }));
                let Ok(done) = ran else {
                    pipeline = None;
                    watching = Watching::default();
                    (self.sink)(Update::Halted);
                    continue;
                };
                let Some(reached) = done.reach else {
                    return;
                };
                let mono = started.elapsed();
                schedule.reached(reached, mono);
                if reached == Reach::NotFound {
                    watching = Watching::default();
                    if reach != Reach::NotFound {
                        (self.sink)(Update::RepoNotFound);
                    }
                }
                reach = reached;
                if let Some(snapshot) = done.snapshot {
                    (self.sink)(Update::Snapshot(Box::new(snapshot)));
                }
                if let Some(delay) = done.follow_up {
                    schedule.follow_up(mono, delay);
                }
                if done.problem != problem {
                    if let Some(problem) = &done.problem {
                        (self.sink)(Update::Problem(problem.clone()));
                    }
                    problem = done.problem;
                }
                if reached == Reach::Found {
                    self.rewatch(running.watch_set(), &mut watching, &mut schedule, mono);
                }
                continue;
            }
            let wait = schedule.wait(started.elapsed(), self.clock.wall_secs());
            match self.rx.recv_timeout(wait) {
                Ok(message) => {
                    if !hear(&message, &mut schedule, started.elapsed()) {
                        return;
                    }
                    // Fold a backlog into the refresh it is waiting for.
                    while let Ok(message) = self.rx.try_recv() {
                        if !hear(&message, &mut schedule, started.elapsed()) {
                            return;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// Watches `wanted` if it was not tried already. A new watch reads the reports again once
    /// the debounce has passed, for anything written while it was being armed. A watch that
    /// cannot start leaves the one before it running, and is not tried again until the set
    /// changes: the hourly catch-up still reads what it would have shown.
    fn rewatch(
        &self,
        wanted: Option<&WatchSet>,
        watching: &mut Watching,
        schedule: &mut Schedule,
        mono: Duration,
    ) {
        let Some(wanted) = wanted else {
            return;
        };
        if watching.tried.as_ref() == Some(wanted) {
            return;
        }
        watching.tried = Some(wanted.clone());
        let tx = self.tx.clone();
        if let Ok(watcher) = Watcher::start(wanted, move |touched| {
            let _ = tx.send(Message::Touched(touched));
        }) {
            watching.watcher = Some(watcher);
            schedule.follow_up(mono, DEBOUNCE);
        }
    }
}

/// Tells the schedule about `message`; false to stop.
fn hear(message: &Message, schedule: &mut Schedule, mono: Duration) -> bool {
    let event = match message {
        Message::Shutdown => return false,
        Message::Touched(touched) => Event::Touched {
            reports: touched.reports,
        },
        Message::Wake => Event::Wake,
        Message::Refresh => Event::Request,
    };
    schedule.on(event, mono);
    true
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use terrarium_truth::{Basis, RepoSeed, SimState, TickGrid};

    use super::*;
    use crate::clock::ManualClock;
    use crate::live::{Branch, RefreshStats, Replay};
    use crate::schedule::Cause;

    /// A pipeline whose second refresh panics, as a truth bug would.
    struct Fragile {
        refreshes: u32,
    }

    impl Pipeline for Fragile {
        fn refresh(&mut self, refresh: Refresh, _: &mut dyn FnMut(u8), _: &AtomicBool) -> Done {
            self.refreshes += 1;
            assert!(self.refreshes < 2, "an injected truth panic");
            let state = SimState::new(Basis::new(
                RepoSeed::from_bytes([1; 32]),
                TickGrid::from_origin(0).unwrap(),
                "main",
            ));
            Done {
                reach: Some(Reach::Found),
                snapshot: Some(Snapshot {
                    scene: terrarium_truth::project(&state),
                    report: state.report(),
                    branch: Branch::Named("main".into()),
                    sim_note: false,
                    stats: RefreshStats {
                        cause: refresh.cause,
                        replay: Replay::Full,
                        working: true,
                        adapters_to_snapshot: Duration::ZERO,
                    },
                }),
                ..Done::default()
            }
        }

        fn watch_set(&self) -> Option<&WatchSet> {
            None
        }
    }

    fn recording() -> (Sink, Arc<Mutex<Vec<Update>>>) {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let into = Arc::clone(&heard);
        (
            Box::new(move |update| into.lock().unwrap().push(update)),
            heard,
        )
    }

    fn wait_for(heard: &Mutex<Vec<Update>>, what: impl Fn(&[Update]) -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        while !what(&heard.lock().unwrap()) {
            assert!(Instant::now() < until, "{:?}", heard.lock().unwrap());
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_panic_in_a_refresh_halts_the_engine_and_never_the_process() {
        let (sink, heard) = recording();
        let (engine, start) = Engine::spawn_pipeline(
            || Ok(Some((Fragile { refreshes: 0 }, Start::NotConnected))),
            Arc::new(ManualClock::at(0)),
            sink,
        )
        .unwrap();
        assert_eq!(start, Start::NotConnected);
        wait_for(&heard, |u| matches!(u, [Update::Snapshot(_)]));
        engine.refresh();
        wait_for(&heard, |u| u.len() == 2);
        assert_eq!(heard.lock().unwrap()[1], Update::Halted);
        // Halted is final: nothing more refreshes, and it still shuts down.
        engine.refresh();
        engine.wake();
        thread::sleep(Duration::from_millis(100));
        assert_eq!(heard.lock().unwrap().len(), 2);
        let stopped = Instant::now();
        engine.shutdown();
        assert!(stopped.elapsed() < SHUTDOWN_WAIT);
    }

    #[test]
    fn a_panic_while_connecting_is_an_error_not_a_crash() {
        let (sink, _) = recording();
        let failed = Engine::spawn_pipeline::<Fragile, _>(
            || panic!("an injected connect panic"),
            Arc::new(ManualClock::at(0)),
            sink,
        );
        assert!(matches!(failed, Err(ConnectError::Halted)));
    }

    #[test]
    fn the_first_refresh_is_the_connects() {
        let (sink, heard) = recording();
        let (_engine, _) = Engine::spawn_pipeline(
            || Ok(Some((Fragile { refreshes: 0 }, Start::NotConnected))),
            Arc::new(ManualClock::at(0)),
            sink,
        )
        .unwrap();
        wait_for(&heard, |u| !u.is_empty());
        let Update::Snapshot(snapshot) = &heard.lock().unwrap()[0] else {
            panic!("a snapshot first");
        };
        assert_eq!(snapshot.stats.cause, Cause::Connect);
    }

    /// A pipeline whose watch set names directories that do not exist, so no watch starts.
    struct Unwatchable {
        refreshes: Arc<std::sync::atomic::AtomicU32>,
        set: WatchSet,
    }

    impl Pipeline for Unwatchable {
        fn refresh(&mut self, _: Refresh, _: &mut dyn FnMut(u8), _: &AtomicBool) -> Done {
            self.refreshes.fetch_add(1, Ordering::SeqCst);
            Done {
                reach: Some(Reach::Found),
                ..Done::default()
            }
        }

        fn watch_set(&self) -> Option<&WatchSet> {
            Some(&self.set)
        }
    }

    #[test]
    fn a_watch_that_cannot_start_is_not_retried_in_a_loop() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone");
        let set = WatchSet::new(&crate::watch::Places {
            git_dir: &missing,
            common_dir: &missing,
            root: None,
            patterns: &[],
            status_dir: &missing,
            status_name: "abc",
        });
        let refreshes = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counted = Arc::clone(&refreshes);
        let (sink, _) = recording();
        let (engine, _) = Engine::spawn_pipeline(
            move || {
                Ok(Some((
                    Unwatchable {
                        refreshes: counted,
                        set,
                    },
                    Start::NotConnected,
                )))
            },
            Arc::new(ManualClock::at(0)),
            sink,
        )
        .unwrap();
        thread::sleep(Duration::from_millis(2_500));
        // The connect's, and at most one follow-up where a watch over nothing starts anyway.
        let seen = refreshes.load(Ordering::SeqCst);
        assert!((1..=2).contains(&seen), "{seen} refreshes");
        engine.shutdown();
    }
}
