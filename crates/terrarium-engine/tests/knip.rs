//! The opt-in Knip run in the live pipeline (I-5): it runs only on the opt-in given for this
//! repository, at the default tip while HEAD is there, once per tip; its report becomes dead
//! code as a Knip report's does; and the app writes nothing into the repository while it runs
//! (N-10). Knip is a script standing in for it, run by `sh` standing in for node, which says it
//! ran by writing outside the repository.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use terrarium_adapters::binding::RepoBinding;
use terrarium_adapters::exec::find_tool;
use terrarium_adapters::fixture::{Edit, FixtureRepo, Stamp};
use terrarium_engine::knip::{KnipCall, KnipDone, KnipJob};
use terrarium_engine::live::Done;
use terrarium_engine::schedule::{Cause, Reach, Refresh};
use terrarium_engine::{Engine, EngineConfig, KnipStatus, Live, ManualClock, Start, Update};
use terrarium_store::identifiers::{SETTING_KNIP_RUN_OPT_IN, SETTING_REPO_ACTIVE};
use terrarium_store::{DataDir, Store, StoredObservation, TerrariumId};
use terrarium_truth::{CommitId, ObservationKind};

/// Monday 2024-01-08, 09:00 UTC.
const FIRST: i64 = 1_704_704_400;
const DAY: i64 = 86_400;
const DAYS: i64 = 15;
const PATIENCE: Duration = Duration::from_secs(30);

/// Knip 6's report: one export of the two in `src/fern.ts` is unused.
const REPORT: &str = r#"{"issues":[{"file":"src/fern.ts","exports":[{"name":"reed","line":2,"col":14,"pos":40}],"types":[]}]}"#;

struct Fixture {
    dir: tempfile::TempDir,
    repo: FixtureRepo,
    clock: ManualClock,
}

impl Fixture {
    /// TypeScript with two exports on `main`, and a Knip 6 in `node_modules` that prints
    /// [`REPORT`] and leaves a mark beside the repository each time it runs.
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = FixtureRepo::init(&dir.path().join("repo")).unwrap();
        for day in 0..DAYS {
            repo.commit(
                "main",
                Stamp::at(FIRST + day * DAY),
                "tend",
                &[Edit::write(
                    "src/fern.ts",
                    &format!(
                        "export function fern() {{ return {day}; }}\nexport const reed = 1;\n"
                    ),
                )],
            )
            .unwrap();
        }
        let fx = Fixture {
            dir,
            repo,
            clock: ManualClock::at(FIRST + DAYS * DAY + 2 * 3_600),
        };
        let package = fx.repo_path().join("node_modules/knip");
        fs::create_dir_all(package.join("bin")).unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"knip","version":"6.3.0","bin":{"knip":"bin/knip.js"}}"#,
        )
        .unwrap();
        fs::write(
            package.join("bin/knip.js"),
            format!(
                "echo ran >> '{}'\nprintf '%s' '{REPORT}'\nexit 1\n",
                fx.marker().display()
            ),
        )
        .unwrap();
        fx
    }

    fn repo_path(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    fn data(&self) -> DataDir {
        DataDir::at(self.dir.path().join("data"))
    }

    /// Where the stand-in says it ran: outside the repository.
    fn marker(&self) -> PathBuf {
        self.dir.path().join("knip-ran")
    }

    fn runs(&self) -> usize {
        fs::read_to_string(self.marker()).map_or(0, |text| text.lines().count())
    }

    fn config(&self) -> EngineConfig {
        EngineConfig {
            clock: Arc::new(self.clock.clone()),
            node: Arc::new(|_: &Path| find_tool(&["sh"], None, &[Path::new("/bin")], None)),
            ..EngineConfig::new(self.data(), Some(self.repo_path()))
        }
    }

    fn connect(&self) -> Live {
        let mut live = Live::connect(&self.config()).unwrap().expect("connected");
        settled(&mut live, Cause::Connect);
        live
    }

    /// Gives the opt-in for `binding`.
    fn opt_in(&self, binding: &RepoBinding) {
        let mut store = Store::open(&self.data()).unwrap();
        store.set_setting(SETTING_KNIP_RUN_OPT_IN, binding).unwrap();
    }

    fn here(live: &Live) -> RepoBinding {
        let terrarium = live.terrarium();
        RepoBinding::new(&terrarium.seed, &terrarium.repo.path)
    }

    fn dead_code(&self) -> Vec<StoredObservation> {
        let store = Store::open(&self.data()).unwrap();
        let id = store.setting::<i64>(SETTING_REPO_ACTIVE).unwrap().unwrap();
        store
            .observations(TerrariumId::new(id), Some(ObservationKind::DeadCode))
            .unwrap()
    }

    fn commit(&mut self, day: i64) -> CommitId {
        self.repo
            .commit(
                "main",
                Stamp::at(FIRST + day * DAY),
                "tend",
                &[Edit::write(
                    "src/moss.ts",
                    &format!("export const moss = {day};\n"),
                )],
            )
            .unwrap()
    }
}

fn refresh(live: &mut Live, cause: Cause) -> Done {
    live.refresh(
        Refresh {
            cause,
            reports: matches!(cause, Cause::Connect | Cause::Request),
        },
        &mut |_| {},
        &AtomicBool::new(false),
    )
}

/// A refresh, then the follow-ups a first refresh leaves (the source measured).
fn settled(live: &mut Live, cause: Cause) {
    let mut done = refresh(live, cause);
    while done.follow_up.is_some() {
        assert_eq!(done.reach, Some(Reach::Found), "{done:?}");
        done = refresh(live, Cause::FollowUp);
    }
    assert_eq!(done.reach, Some(Reach::Found), "{done:?}");
    assert_eq!(done.problem, None);
}

/// The run the pipeline starts now, run here as the worker's thread would.
fn run_now(live: &mut Live) -> KnipJob {
    match live.knip_call() {
        Some(KnipCall::Start(job)) => job,
        other => panic!("no run started: {other:?}"),
    }
}

fn finish(live: &mut Live, job: KnipJob) {
    let outcome = job.run.run(&job.node, &AtomicBool::new(false));
    assert!(live.knip_ran(KnipDone {
        head: job.head,
        outcome,
    }));
    settled(live, Cause::Job);
}

#[test]
fn knip_runs_on_the_opt_in_and_its_report_is_dead_code_at_the_tip() {
    let fx = Fixture::new();
    let mut live = fx.connect();
    assert!(live.knip_call().is_none());
    assert_eq!(live.tools().knip, KnipStatus::Off);
    fx.opt_in(&Fixture::here(&live));
    settled(&mut live, Cause::Request);
    let job = run_now(&mut live);
    assert_eq!(live.tools().knip, KnipStatus::Running);
    let tip = job.head.commit.clone();
    finish(&mut live, job);
    assert_eq!(fx.runs(), 1);
    let rows = fx.dead_code();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].record.head.commit, tip);
    assert!(rows[0].record.head.at_default_tip);
    assert_eq!(live.tools().knip, KnipStatus::Ran { unused: 1 });
    // Once per tip.
    assert!(live.knip_call().is_none());
    settled(&mut live, Cause::Request);
    assert!(live.knip_call().is_none());
    assert_eq!(fx.runs(), 1);
}

#[test]
fn without_the_opt_in_for_this_repository_knip_never_runs() {
    let fx = Fixture::new();
    let mut live = fx.connect();
    let here = Fixture::here(&live);
    for elsewhere in [
        None,
        Some(RepoBinding {
            seed: "00".repeat(32),
            ..here.clone()
        }),
        Some(RepoBinding {
            path: format!("{}-fork", here.path),
            ..here.clone()
        }),
    ] {
        if let Some(binding) = &elsewhere {
            fx.opt_in(binding);
        }
        settled(&mut live, Cause::Request);
        assert!(live.knip_call().is_none(), "{elsewhere:?}");
        assert_eq!(live.tools().knip, KnipStatus::Off);
    }
    // The engine itself, left to run: nothing ever runs.
    drop(live);
    let heard = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&heard);
    let (engine, start) = Engine::spawn(
        fx.config(),
        Box::new(move |update| into.lock().unwrap().push(update)),
    )
    .unwrap();
    assert!(matches!(start, Start::Connected { .. }));
    wait_for(&heard, "a snapshot", |updates| {
        updates.iter().any(|u| matches!(u, Update::Snapshot(_)))
    });
    thread::sleep(Duration::from_millis(500));
    engine.shutdown_and_join();
    assert_eq!(fx.runs(), 0, "Knip ran without the opt-in");
    assert!(fx.dead_code().is_empty());
}

#[test]
fn a_report_from_a_tip_head_has_left_is_dropped_and_the_tip_runs_again() {
    let mut fx = Fixture::new();
    let mut live = fx.connect();
    fx.opt_in(&Fixture::here(&live));
    settled(&mut live, Cause::Request);
    let job = run_now(&mut live);
    fx.repo.branch("feat/moss", "main").unwrap();
    fx.repo.set_head("feat/moss").unwrap();
    fx.repo
        .commit(
            "feat/moss",
            Stamp::at(FIRST + DAYS * DAY),
            "try",
            &[Edit::write("src/try.ts", "export const t = 1;\n")],
        )
        .unwrap();
    settled(&mut live, Cause::Watch);
    finish(&mut live, job);
    assert!(fx.dead_code().is_empty(), "read while HEAD was elsewhere");
    assert_eq!(live.tools().knip, KnipStatus::Waiting);
    assert!(live.knip_call().is_none(), "not while HEAD is off the tip");
    fx.repo.set_head("main").unwrap();
    settled(&mut live, Cause::Watch);
    let job = run_now(&mut live);
    finish(&mut live, job);
    assert_eq!(fx.dead_code().len(), 1);
}

#[test]
fn the_same_report_at_a_new_tip_is_recorded_and_a_relaunch_does_not_run_it_again() {
    let mut fx = Fixture::new();
    let mut live = fx.connect();
    fx.opt_in(&Fixture::here(&live));
    settled(&mut live, Cause::Request);
    let job = run_now(&mut live);
    finish(&mut live, job);
    fx.clock.advance(60);
    fx.commit(DAYS);
    settled(&mut live, Cause::Watch);
    let job = run_now(&mut live);
    finish(&mut live, job);
    let rows = fx.dead_code();
    assert_eq!(rows.len(), 2, "the same output, at another commit");
    assert_ne!(rows[0].record.content_hash, rows[1].record.content_hash);
    drop(live);
    let mut again = fx.connect();
    assert!(again.knip_call().is_none(), "the tip has its reading");
    assert_eq!(fx.runs(), 2);
}

#[test]
fn withdrawing_the_opt_in_stops_the_run() {
    let fx = Fixture::new();
    let mut live = fx.connect();
    fx.opt_in(&Fixture::here(&live));
    settled(&mut live, Cause::Request);
    let job = run_now(&mut live);
    let mut store = Store::open(&fx.data()).unwrap();
    store.clear_setting(SETTING_KNIP_RUN_OPT_IN).unwrap();
    settled(&mut live, Cause::Request);
    assert!(matches!(live.knip_call(), Some(KnipCall::Stop)));
    // Its end, when it comes, records nothing.
    let outcome = job.run.run(&job.node, &AtomicBool::new(true));
    assert!(!live.knip_ran(KnipDone {
        head: job.head,
        outcome,
    }));
    assert_eq!(live.tools().knip, KnipStatus::Off);
}

/// Every file under `root`, by path, hashed.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, [u8; 32]> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<PathBuf, [u8; 32]>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let bytes = fs::read(&path).unwrap();
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    Sha256::digest(&bytes).into(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn wait_for(heard: &Mutex<Vec<Update>>, what: &str, done: impl Fn(&[Update]) -> bool) {
    let until = Instant::now() + PATIENCE;
    while !done(&heard.lock().unwrap()) {
        assert!(Instant::now() < until, "waited for {what}");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_engine_runs_knip_on_its_own_thread_and_writes_nothing_into_the_repository() {
    let fx = Fixture::new();
    {
        let live = fx.connect();
        fx.opt_in(&Fixture::here(&live));
    }
    let before = snapshot(&fx.repo_path());
    let heard = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&heard);
    let (engine, _) = Engine::spawn(
        fx.config(),
        Box::new(move |update| into.lock().unwrap().push(update)),
    )
    .unwrap();
    wait_for(&heard, "Knip's report", |updates| {
        updates.iter().any(
            |u| matches!(u, Update::Tools(tools) if tools.knip == KnipStatus::Ran { unused: 1 }),
        )
    });
    engine.shutdown_and_join();
    assert_eq!(fx.runs(), 1);
    assert_eq!(fx.dead_code().len(), 1);
    assert_eq!(
        snapshot(&fx.repo_path()),
        before,
        "the app wrote into the repository"
    );
}

#[test]
fn in_a_bare_repository_knip_is_tried_once_per_tip() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = FixtureRepo::init_bare(&dir.path().join("repo")).unwrap();
    for day in 0..DAYS {
        repo.commit(
            "main",
            Stamp::at(FIRST + day * DAY),
            "tend",
            &[Edit::write("src/fern.ts", "export const reed = 1;\n")],
        )
        .unwrap();
    }
    let fx = Fixture {
        dir,
        repo,
        clock: ManualClock::at(FIRST + DAYS * DAY + 2 * 3_600),
    };
    let mut live = fx.connect();
    fx.opt_in(&Fixture::here(&live));
    settled(&mut live, Cause::Request);
    let job = run_now(&mut live);
    finish(&mut live, job);
    assert_eq!(live.tools().knip, KnipStatus::NotInstalled);
    assert!(live.knip_call().is_none(), "not again at this tip");
    settled(&mut live, Cause::Watch);
    assert!(live.knip_call().is_none());
}

#[test]
fn a_run_stopped_while_the_repository_is_missing_runs_again_once_it_is_back() {
    let fx = Fixture::new();
    let mut live = fx.connect();
    fx.opt_in(&Fixture::here(&live));
    settled(&mut live, Cause::Request);
    let _stopped = run_now(&mut live);
    let away = fx.dir.path().join("away");
    fs::rename(fx.repo_path(), &away).unwrap();
    assert_eq!(
        refresh(&mut live, Cause::Watch).reach,
        Some(Reach::NotFound)
    );
    fs::rename(&away, fx.repo_path()).unwrap();
    settled(&mut live, Cause::Retry);
    let job = run_now(&mut live);
    finish(&mut live, job);
    assert_eq!(fx.dead_code().len(), 1);
}
