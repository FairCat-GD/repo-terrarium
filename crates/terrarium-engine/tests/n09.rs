//! N-09: no connection but the configured GitHub Actions poll. With the network blocked, every
//! feature but the CI weather works; with no token saved, or no GitHub repository, the engine
//! asks nothing at all. And the poll as the engine runs it: a verdict recorded, a refused token
//! not asked with again until it changes. The client here counts its calls and never reaches a network; the
//! client that does is tested in `terrarium-adapters::ci` against a loopback server, and the
//! crate-graph stage of `cargo xtask verify` keeps every other source away from sockets.
#![forbid(unsafe_code)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use common::write_still as still;
use terrarium_adapters::ci::{CiError, Fetch, MemoryKeychain, Slug, Token, Verdict};
use terrarium_adapters::fixture::{Edit, FixtureRepo, Stamp};
use terrarium_engine::ci::CiConfig;
use terrarium_engine::{CiStatus, Engine, EngineConfig, ManualClock, Snapshot, Start, Update};
use terrarium_store::identifiers::SETTING_REPO_ACTIVE;
use terrarium_store::{DataDir, Store, TerrariumId};
use terrarium_truth::ObservationKind;
use terrarium_truth::domain::WeatherSource;

/// Monday 2024-01-08, 09:00 UTC.
const FIRST: i64 = 1_704_704_400;
const DAY: i64 = 86_400;
const DAYS: i64 = 15;
const PATIENCE: Duration = Duration::from_secs(30);
/// The poll's period here: many polls in a short test.
const EVERY: Duration = Duration::from_millis(10);

/// A client whose every call is counted and answered with `answer`.
struct Counting {
    calls: Arc<AtomicUsize>,
    answer: Result<Option<Verdict>, CiError>,
}

impl Fetch for Counting {
    fn runs(&mut self, _: &Slug, _: &str, _: &Token) -> Result<Option<Verdict>, CiError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answer.clone()
    }

    fn forget(&mut self) {}
}

struct Session {
    dir: tempfile::TempDir,
    heard: Arc<Mutex<Vec<Update>>>,
    calls: Arc<AtomicUsize>,
    keychain: Arc<MemoryKeychain>,
    engine: Option<Engine>,
}

impl Session {
    /// A repository with every report and source the adapters read, on GitHub by its `origin`
    /// when `on_github`, run by the engine with `keychain` and a client that answers `answer`.
    fn start(
        on_github: bool,
        keychain: MemoryKeychain,
        answer: Result<Option<Verdict>, CiError>,
    ) -> Session {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let mut repo = FixtureRepo::init(&root).unwrap();
        for day in 0..DAYS {
            repo.commit(
                "main",
                Stamp::at(FIRST + day * DAY),
                "tend",
                &[Edit::write(
                    "src/fern.ts",
                    &format!("export function fern() {{ return {day}; }}\nexport const reed = 1;\n// TODO trim\n"),
                )],
            )
            .unwrap();
        }
        if on_github {
            repo.append_git_config(
                "[remote \"origin\"]\n\turl = https://github.com/example-garden/moss-bed.git\n",
            )
            .unwrap();
        }
        still(
            &root.join("coverage/lcov.info"),
            "SF:src/fern.ts\nDA:1,1\nDA:2,0\nend_of_record\n",
        );
        still(
            &root.join("junit.xml"),
            r#"<testsuite><testcase name="grows"/></testsuite>"#,
        );
        still(
            &root.join("knip-report.json"),
            r#"{"issues":[{"file":"src/fern.ts","exports":[{"name":"reed"}],"types":[]}]}"#,
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let keychain = Arc::new(keychain);
        let fetch_calls = Arc::clone(&calls);
        let config = EngineConfig {
            clock: Arc::new(ManualClock::at(FIRST + DAYS * DAY + 2 * 3_600)),
            ci: Some(CiConfig {
                keychain: Arc::clone(&keychain) as _,
                fetch: Arc::new(move || {
                    Box::new(Counting {
                        calls: Arc::clone(&fetch_calls),
                        answer: answer.clone(),
                    })
                }),
                every: EVERY,
            }),
            ..EngineConfig::new(DataDir::at(dir.path().join("data")), Some(root))
        };
        let heard = Arc::new(Mutex::new(Vec::new()));
        let into = Arc::clone(&heard);
        let (engine, start) = Engine::spawn(
            config,
            Box::new(move |update| into.lock().unwrap().push(update)),
        )
        .unwrap();
        assert!(matches!(start, Start::Connected { .. }));
        Session {
            dir,
            heard,
            calls,
            keychain,
            engine: Some(engine),
        }
    }

    fn wait_for(&self, what: &str, done: impl Fn(&Session) -> bool) {
        let until = Instant::now() + PATIENCE;
        while !done(self) {
            assert!(
                Instant::now() < until,
                "waited for {what}: {:?}",
                self.heard.lock().unwrap()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn kinds(&self) -> Vec<(ObservationKind, Option<WeatherSource>)> {
        let store = Store::open(&DataDir::at(self.dir.path().join("data"))).unwrap();
        let Some(id) = store.setting::<i64>(SETTING_REPO_ACTIVE).unwrap() else {
            return Vec::new();
        };
        store
            .observations(TerrariumId::new(id), None)
            .unwrap()
            .into_iter()
            .map(|row| (row.record.kind, row.record.source))
            .collect()
    }

    fn last_snapshot(&self) -> Option<Snapshot> {
        self.heard
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|update| match update {
                Update::Snapshot(snapshot) => Some((**snapshot).clone()),
                _ => None,
            })
    }

    fn ci_status(&self) -> Option<CiStatus> {
        self.heard
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|update| match update {
                Update::Tools(tools) => Some(tools.ci.clone()),
                _ => None,
            })
    }

    fn stop(&mut self) {
        if let Some(engine) = self.engine.take() {
            engine.shutdown_and_join();
        }
    }
}

fn slug() -> Slug {
    Slug::parse("example-garden/moss-bed").unwrap()
}

#[test]
fn with_the_network_blocked_every_feature_but_ci_works() {
    let mut session = Session::start(
        true,
        MemoryKeychain::holding("github_pat_fictional"),
        Err(CiError::Unreachable),
    );
    let has = |kind, source: Option<WeatherSource>| {
        move |s: &Session| s.kinds().contains(&(kind, source.clone()))
    };
    session.wait_for("coverage", has(ObservationKind::Coverage, None));
    session.wait_for(
        "the local tests",
        has(ObservationKind::Weather, Some(WeatherSource::Local)),
    );
    session.wait_for("complexity", has(ObservationKind::Complexity, None));
    session.wait_for("TODO density", has(ObservationKind::TodoDensity, None));
    session.wait_for("dead code", has(ObservationKind::DeadCode, None));
    session.wait_for("several polls", |s| s.calls.load(Ordering::SeqCst) >= 3);
    session.wait_for("the poll's status", |s| {
        s.ci_status() == Some(CiStatus::Unreachable(slug()))
    });
    session.stop();
    let weather = session.last_snapshot().unwrap().report.climate.weather;
    assert_eq!(weather.source, Some(WeatherSource::Local));
    assert!(
        !session
            .kinds()
            .iter()
            .any(|(_, source)| matches!(source, Some(WeatherSource::Ci { .. }))),
        "nothing from GitHub was recorded"
    );
    assert_eq!(session.keychain.reads(), 1, "the keychain is read once");
}

/// Runs a session for a while, long enough for a hundred polls at [`EVERY`].
fn run_a_while(session: &mut Session) {
    session.wait_for("a snapshot", |s| s.last_snapshot().is_some());
    thread::sleep(EVERY * 100);
    session.stop();
}

#[test]
fn with_no_token_saved_nothing_is_asked() {
    let mut session = Session::start(true, MemoryKeychain::default(), Ok(None));
    run_a_while(&mut session);
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    assert_eq!(session.keychain.reads(), 1, "the keychain is read once");
    assert_eq!(session.ci_status(), Some(CiStatus::NoToken));
}

#[test]
fn with_no_github_repository_nothing_is_asked_and_the_keychain_is_not_read() {
    let mut session = Session::start(
        false,
        MemoryKeychain::holding("github_pat_fictional"),
        Ok(None),
    );
    run_a_while(&mut session);
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    assert_eq!(session.keychain.reads(), 0);
    assert_eq!(session.ci_status(), Some(CiStatus::NoRepository));
}

#[test]
fn the_engine_records_what_the_poll_reads() {
    let verdict = Verdict {
        passed: false,
        sha: "a1".repeat(20),
        workflows: 1,
        failing: vec![".github/workflows/ci.yml".into()],
        hash: [7; 32],
    };
    let mut session = Session::start(
        true,
        MemoryKeychain::holding("github_pat_fictional"),
        Ok(Some(verdict)),
    );
    let ci_main = Some(WeatherSource::Ci {
        branch: "main".into(),
    });
    session.wait_for("GitHub's weather in effect", |s| {
        s.last_snapshot()
            .is_some_and(|snapshot| snapshot.report.climate.weather.source == ci_main)
    });
    thread::sleep(EVERY * 20);
    session.stop();
    let rows = session
        .kinds()
        .into_iter()
        .filter(|(_, source)| *source == ci_main)
        .count();
    assert_eq!(rows, 1, "the same runs are recorded once");
    assert!(session.calls.load(Ordering::SeqCst) > 3);
}

#[test]
fn a_refused_token_is_not_asked_with_again_until_it_changes() {
    let session = Session::start(
        true,
        MemoryKeychain::holding("github_pat_fictional"),
        Err(CiError::Refused),
    );
    session.wait_for("the refusal", |s| {
        s.ci_status() == Some(CiStatus::Refused(slug()))
    });
    thread::sleep(EVERY * 50);
    assert_eq!(session.calls.load(Ordering::SeqCst), 1);
    session.engine.as_ref().unwrap().token_changed();
    session.wait_for("a poll with the new token", |s| {
        s.calls.load(Ordering::SeqCst) == 2
    });
    assert_eq!(session.keychain.reads(), 2, "read again once it changed");
}

/// A client that fails inside itself on every call.
struct Panicking(Arc<AtomicUsize>);

impl Fetch for Panicking {
    fn runs(&mut self, _: &Slug, _: &str, _: &Token) -> Result<Option<Verdict>, CiError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("a fictional failure inside the client");
    }

    fn forget(&mut self) {}
}

#[test]
fn a_client_that_panics_is_answered_as_github_not_reached_and_polling_goes_on() {
    let mut session = Session::start(
        true,
        MemoryKeychain::holding("github_pat_fictional"),
        Ok(None),
    );
    session.stop();
    // The same repository, run again with a client that panics.
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let config = EngineConfig {
        clock: Arc::new(ManualClock::at(FIRST + DAYS * DAY + 2 * 3_600)),
        ci: Some(CiConfig {
            keychain: Arc::new(MemoryKeychain::holding("github_pat_fictional")),
            fetch: Arc::new(move || Box::new(Panicking(Arc::clone(&counted)))),
            every: EVERY,
        }),
        ..EngineConfig::new(
            DataDir::at(session.dir.path().join("data")),
            Some(session.dir.path().join("repo")),
        )
    };
    let heard = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&heard);
    let (engine, _) = Engine::spawn(
        config,
        Box::new(move |update| into.lock().unwrap().push(update)),
    )
    .unwrap();
    let until = Instant::now() + PATIENCE;
    while calls.load(Ordering::SeqCst) < 3 {
        assert!(Instant::now() < until, "polling stopped after a panic");
        thread::sleep(Duration::from_millis(20));
    }
    engine.shutdown_and_join();
    let unreachable = heard.lock().unwrap().iter().any(|update| {
        matches!(update, Update::Tools(tools) if tools.ci == CiStatus::Unreachable(slug()))
    });
    assert!(unreachable);
}
