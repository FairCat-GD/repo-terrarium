//! The engine's thread (I-16): it connects, then refreshes whenever the schedule says, and
//! sends what the strip shows through a sink the app turns into `EventLoopProxy` events. The UI
//! thread never waits on it: it waits for connect's first answer only before the event loop
//! runs, and every later start and stop happens on a thread of the app's.
//!
//! One channel carries everything the thread hears: the app's requests and what the watcher
//! saw. Each wait ends at the schedule's next deadline, so the thread sleeps between
//! events and looks at the wall clock at least once a minute.
//!
//! **Jobs** run beside the refreshes, on threads of their own, and come back on the same
//! channel: the GitHub Actions poll, asked when [`Polling`] says, and the opt-in Knip run,
//! started when the pipeline says. The worker owns the run: a missing repository, a halt or a
//! shutdown stops it. What a job brings back is recorded by a refresh, under `catch_unwind`
//! like every other; what it says of itself goes to the strip as [`Update::Tools`].
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

use crate::ci::{Answered, Ask, CiConfig, Poller, Polling};
use crate::clock::Clock;
use crate::knip::{KnipCall, KnipDone, Running};
use crate::live::{ConnectError, Done, EngineConfig, Live, Problem, Snapshot, Start, Tools};
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
    /// A panic stopped the engine for this session (I-16), or a refresh ended without reaching
    /// the repository though nothing cancelled it: a bug, stopped the same way.
    Halted,
    /// A refresh failed in a way the next may not; sent once until it changes.
    Problem(Problem),
    /// What the Knip run and the GitHub Actions poll say of themselves, when it changed.
    Tools(Tools),
}

pub type Sink = Box<dyn FnMut(Update) + Send>;

/// What the thread hears.
enum Message {
    Touched(Touched),
    Wake,
    Refresh,
    Shutdown,
    /// A poll's answer.
    Ci(Box<Answered>),
    /// The run started `n`-th ended.
    Knip(u64, Box<KnipDone>),
    /// The token in the keychain was saved or removed.
    TokenChanged,
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

    /// The poll to send now, if any. Only reads.
    fn ci_ask(&mut self) -> Option<Ask> {
        None
    }

    /// A poll's answer; true when a refresh should record it. Only keeps it.
    fn ci_answered(&mut self, _: Answered, _: bool) -> bool {
        false
    }

    /// Whether to poll at once: HEAD's branch or the repository changed.
    fn ci_ask_now(&mut self) -> bool {
        false
    }

    /// What to do about Knip after a refresh.
    fn knip_call(&mut self) -> Option<KnipCall> {
        None
    }

    /// A run that ended; true when a refresh should record it. Only keeps it.
    fn knip_ran(&mut self, _: KnipDone) -> bool {
        false
    }

    fn tools(&self) -> Tools {
        Tools::default()
    }
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

    fn ci_ask(&mut self) -> Option<Ask> {
        Live::ci_ask(self)
    }

    fn ci_answered(&mut self, answered: Answered, outage: bool) -> bool {
        Live::ci_answered(self, answered, outage)
    }

    fn ci_ask_now(&mut self) -> bool {
        Live::ci_ask_now(self)
    }

    fn knip_call(&mut self) -> Option<KnipCall> {
        Live::knip_call(self)
    }

    fn knip_ran(&mut self, done: KnipDone) -> bool {
        Live::knip_ran(self, done)
    }

    fn tools(&self) -> Tools {
        Live::tools(self)
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
        let ci = config.ci.clone();
        Engine::spawn_pipeline_with(
            move || {
                Ok(Live::connect(&config)?.map(|live| {
                    let start = live.start();
                    (live, start)
                }))
            },
            clock,
            sink,
            ci,
        )
    }

    /// The engine on another pipeline, polling nothing: what the tests' stand-ins run on.
    #[cfg(test)]
    pub(crate) fn spawn_pipeline<P, F>(
        connect: F,
        clock: Arc<dyn Clock>,
        sink: Sink,
    ) -> Result<(Engine, Start), ConnectError>
    where
        P: Pipeline,
        F: FnOnce() -> Result<Option<(P, Start)>, ConnectError> + Send + 'static,
    {
        Engine::spawn_pipeline_with(connect, clock, sink, None)
    }

    fn spawn_pipeline_with<P, F>(
        connect: F,
        clock: Arc<dyn Clock>,
        sink: Sink,
        ci: Option<CiConfig>,
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
            ci,
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

    /// Refreshes everything at once, the reports and the adapters' settings among them: what
    /// the strip asks for once a changed setting has been saved.
    pub fn refresh(&self) {
        let _ = self.tx.send(Message::Refresh);
    }

    /// The GitHub token was saved or removed: the poll reads it again and asks at once.
    pub fn token_changed(&self) {
        let _ = self.tx.send(Message::TokenChanged);
    }

    /// Stops the engine: a history read in progress stops at its next commit. Waits up to a
    /// second, then leaves the thread to end with the process.
    pub fn shutdown(mut self) {
        self.stop();
    }

    /// Stops the engine and waits for its thread to end, however long a replay in progress
    /// takes, so nothing it writes can follow: for the app's own thread, before it changes the
    /// terrarium.
    pub fn shutdown_and_join(mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Message::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
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
    /// Handed to each watcher and job.
    tx: Sender<Message>,
    sink: Sink,
    clock: Arc<dyn Clock>,
    cancel: Arc<AtomicBool>,
    ci: Option<CiConfig>,
}

/// The jobs beside the refreshes.
struct Jobs {
    poller: Option<Poller>,
    polling: Option<Polling>,
    /// The Knip run going, and its number, so the end of one stopped earlier is told apart.
    knip: Option<(u64, Running)>,
    started: u64,
    /// What the strip was last told of them.
    told: Tools,
}

impl Jobs {
    fn stop_knip(&mut self) {
        if let Some((_, run)) = self.knip.take() {
            run.stop();
        }
    }
}

impl Worker {
    fn run<P: Pipeline>(mut self, pipeline: P) {
        let mut pipeline = Some(pipeline);
        let started = Instant::now();
        let mut schedule = Schedule::new(self.clock.wall_secs());
        let mut watching = Watching::default();
        let mut reach = Reach::Found;
        let mut problem: Option<Problem> = None;
        let mut jobs = self.start_jobs(started.elapsed());
        loop {
            let Some(running) = pipeline.as_mut() else {
                // Halted: no job runs, and only a shutdown is answered.
                jobs.stop_knip();
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
                // Only a stop cancels, and a stopped worker ends. A panic, or a refresh that ends
                // without a reach when nothing cancelled it, is a bug: the engine halts and says
                // so, rather than freezing the strip with no note.
                if matches!(&ran, Ok(done) if done.reach.is_none())
                    && self.cancel.load(Ordering::Relaxed)
                {
                    jobs.stop_knip();
                    return;
                }
                let Some((reached, done)) = ran.ok().and_then(|done| Some((done.reach?, done)))
                else {
                    pipeline = None;
                    watching = Watching::default();
                    (self.sink)(Update::Halted);
                    continue;
                };
                let mono = started.elapsed();
                schedule.reached(reached, mono);
                if reached == Reach::NotFound {
                    watching = Watching::default();
                    jobs.stop_knip();
                    if let Some(polling) = &mut jobs.polling {
                        polling.pause();
                    }
                    if reach != Reach::NotFound {
                        (self.sink)(Update::RepoNotFound);
                    }
                } else if reach == Reach::NotFound
                    && let Some(polling) = &mut jobs.polling
                {
                    polling.restart(mono);
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
                    self.after_refresh(running, &mut jobs, mono);
                }
                self.tell_tools(running, &mut jobs);
                continue;
            }
            if reach == Reach::Found {
                Worker::poll(running, &mut jobs, started.elapsed());
            }
            let mono = started.elapsed();
            let polls_in = jobs
                .polling
                .as_ref()
                .and_then(|polling| polling.wait(mono))
                .unwrap_or(Duration::MAX);
            let wait = schedule.wait(mono, self.clock.wall_secs()).min(polls_in);
            if !self.listen(wait, running, &mut schedule, &mut jobs, started) {
                jobs.stop_knip();
                return;
            }
        }
    }

    /// Waits up to `wait` for a message, then takes it and the backlog behind it in; false to
    /// stop.
    fn listen<P: Pipeline>(
        &mut self,
        wait: Duration,
        running: &mut P,
        schedule: &mut Schedule,
        jobs: &mut Jobs,
        started: Instant,
    ) -> bool {
        match self.rx.recv_timeout(wait) {
            Ok(message) => {
                if !self.hear(message, running, schedule, jobs, started.elapsed()) {
                    return false;
                }
                // Fold a backlog into the refresh it is waiting for.
                while let Ok(message) = self.rx.try_recv() {
                    if !self.hear(message, running, schedule, jobs, started.elapsed()) {
                        return false;
                    }
                }
                self.tell_tools(running, jobs);
                true
            }
            Err(RecvTimeoutError::Timeout) => true,
            Err(RecvTimeoutError::Disconnected) => false,
        }
    }

    /// The poller and its clock, when the engine polls GitHub Actions.
    fn start_jobs(&self, mono: Duration) -> Jobs {
        let poller = self.ci.as_ref().and_then(|ci| {
            let tx = self.tx.clone();
            Poller::start(ci, move |answered| {
                let _ = tx.send(Message::Ci(Box::new(answered)));
            })
        });
        let polling = poller
            .as_ref()
            .and(self.ci.as_ref())
            .map(|ci| Polling::new(ci.every, mono));
        Jobs {
            poller,
            polling,
            knip: None,
            started: 0,
            told: Tools::default(),
        }
    }

    /// Asks GitHub when a poll is due and there is something to ask about.
    fn poll<P: Pipeline>(running: &mut P, jobs: &mut Jobs, mono: Duration) {
        let (Some(polling), Some(poller)) = (&mut jobs.polling, &jobs.poller) else {
            return;
        };
        if polling.take(mono) {
            match running.ci_ask() {
                Some(ask) => poller.ask(ask),
                None => polling.skipped(mono),
            }
        }
    }

    /// After a refresh that reached the repository: a poll at once if HEAD's branch or the
    /// repository changed, and Knip started or stopped as the pipeline says.
    fn after_refresh<P: Pipeline>(&self, running: &mut P, jobs: &mut Jobs, mono: Duration) {
        if running.ci_ask_now()
            && let Some(polling) = &mut jobs.polling
        {
            polling.restart(mono);
        }
        match running.knip_call() {
            Some(KnipCall::Start(job)) => {
                jobs.stop_knip();
                jobs.started += 1;
                let number = jobs.started;
                let tx = self.tx.clone();
                let run = Running::start(job, move |done| {
                    let _ = tx.send(Message::Knip(number, Box::new(done)));
                });
                jobs.knip = Some((number, run));
            }
            Some(KnipCall::Stop) => jobs.stop_knip(),
            None => {}
        }
    }

    /// Tells the strip what the jobs say of themselves, when it changed.
    fn tell_tools<P: Pipeline>(&mut self, running: &P, jobs: &mut Jobs) {
        let tools = running.tools();
        if tools != jobs.told {
            (self.sink)(Update::Tools(tools.clone()));
            jobs.told = tools;
        }
    }

    /// Takes in `message`; false to stop.
    fn hear<P: Pipeline>(
        &self,
        message: Message,
        running: &mut P,
        schedule: &mut Schedule,
        jobs: &mut Jobs,
        mono: Duration,
    ) -> bool {
        let event = match message {
            Message::Shutdown => return false,
            Message::Touched(touched) => Event::Touched {
                reports: touched.reports,
            },
            Message::Wake => {
                if let Some(polling) = &mut jobs.polling {
                    polling.restart(mono);
                }
                Event::Wake
            }
            Message::Refresh => Event::Request,
            Message::TokenChanged => {
                if let Some(poller) = &jobs.poller {
                    poller.token_changed();
                }
                if let Some(polling) = &mut jobs.polling {
                    polling.restart(mono);
                }
                return true;
            }
            Message::Ci(answered) => {
                let outage = jobs.polling.as_mut().is_some_and(|polling| {
                    polling.answered(&answered.read, mono, self.clock.wall_secs())
                });
                if !running.ci_answered(*answered, outage) {
                    return true;
                }
                Event::Job
            }
            Message::Knip(number, done) => {
                // The end of a run stopped earlier is not this one's.
                if jobs.knip.as_ref().is_none_or(|(going, _)| *going != number) {
                    return true;
                }
                jobs.stop_knip();
                if !running.knip_ran(*done) {
                    return true;
                }
                Event::Job
            }
        };
        schedule.on(event, mono);
        true
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use terrarium_truth::{Basis, RepoSeed, SimState, TickGrid};

    use super::*;
    use crate::clock::ManualClock;
    use crate::details::Details;
    use crate::live::{Branch, News, RefreshStats, Replay};
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
                    details: Details::default(),
                    branch: Branch::Named("main".into()),
                    notes: News::default(),
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

    /// A pipeline whose refresh ends without a reach, though nothing cancelled it.
    struct Unreached;

    impl Pipeline for Unreached {
        fn refresh(&mut self, _: Refresh, _: &mut dyn FnMut(u8), _: &AtomicBool) -> Done {
            Done::default()
        }

        fn watch_set(&self) -> Option<&WatchSet> {
            None
        }
    }

    /// A pipeline whose refresh takes `ms` and ignores the cancel, as a replay does.
    struct Slow {
        ms: u64,
        ended: Arc<AtomicBool>,
    }

    impl Pipeline for Slow {
        fn refresh(&mut self, _: Refresh, _: &mut dyn FnMut(u8), _: &AtomicBool) -> Done {
            thread::sleep(Duration::from_millis(self.ms));
            self.ended.store(true, Ordering::SeqCst);
            Done {
                reach: Some(Reach::Found),
                ..Done::default()
            }
        }

        fn watch_set(&self) -> Option<&WatchSet> {
            None
        }
    }

    #[test]
    fn a_join_waits_for_a_refresh_past_the_shutdown_wait() {
        let ended = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ended);
        let (sink, _) = recording();
        let ms = u64::try_from(SHUTDOWN_WAIT.as_millis()).unwrap() + 300;
        let (engine, _) = Engine::spawn_pipeline(
            move || Ok(Some((Slow { ms, ended: flag }, Start::NotConnected))),
            Arc::new(ManualClock::at(0)),
            sink,
        )
        .unwrap();
        thread::sleep(Duration::from_millis(50));
        engine.shutdown_and_join();
        assert!(ended.load(Ordering::SeqCst), "the refresh ended first");
    }

    #[test]
    fn a_refresh_that_ends_unreached_without_a_cancel_halts_the_engine() {
        let (sink, heard) = recording();
        let (engine, _) = Engine::spawn_pipeline(
            || Ok(Some((Unreached, Start::NotConnected))),
            Arc::new(ManualClock::at(0)),
            sink,
        )
        .unwrap();
        wait_for(&heard, |u| matches!(u, [Update::Halted]));
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
