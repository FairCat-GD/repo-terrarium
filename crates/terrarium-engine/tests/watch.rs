//! The engine's thread with its real watcher on fixture repositories (N-06, N-10): one
//! commit is one refresh, a storm of ref writes coalesces, and every way a tool or git writes is
//! seen: a ref moved into `packed-refs`, a report renamed into place, a new Surefire file, a
//! `record-tests` run. The repository's bytes never change.
//!
//! File events arrive when the OS delivers them, so each test waits for what it expects, up to
//! [`PATIENCE`], and counts only after a quiet spell.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use sha2::{Digest, Sha256};
use terrarium_adapters::fixture::{Edit, FixtureRepo, Stamp};
use terrarium_adapters::local_tests::Status;
use terrarium_engine::{Engine, EngineConfig, ManualClock, Snapshot, Start, Update};
use terrarium_store::identifiers::SETTING_REPO_ACTIVE;
use terrarium_store::{DataDir, Store, StoredObservation, TerrariumId};
use terrarium_truth::{CommitId, ObservationKind};

/// Monday 2024-01-08, 09:00 UTC.
const FIRST: i64 = 1_704_704_400;
const DAY: i64 = 86_400;
/// How long a test waits for what it expects.
const PATIENCE: Duration = Duration::from_secs(10);
/// How long nothing must happen before a count is final: past the debounce, the settle and a
/// follow-up, twice.
const QUIET: Duration = Duration::from_millis(2_000);

struct Watched {
    dir: tempfile::TempDir,
    repo: FixtureRepo,
    engine: Option<Engine>,
    heard: Arc<Mutex<Vec<Update>>>,
    /// The next commit's time, a minute after the last.
    next: i64,
}

impl Watched {
    /// A repository with a commit a day for ten days, prepared by `before`, then watched.
    fn new(before: impl FnOnce(&mut FixtureRepo, &Path)) -> Watched {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = FixtureRepo::init(&dir.path().join("repo")).unwrap();
        for day in 0..10 {
            repo.commit(
                "main",
                Stamp::at(FIRST + day * DAY),
                "tend",
                &[Edit::write(&format!("src/fern_{day}.rs"), "fn fern() {}\n")],
            )
            .unwrap();
        }
        let root = dir.path().join("repo");
        before(&mut repo, &root);
        let mut watched = Watched {
            dir,
            repo,
            engine: None,
            heard: Arc::default(),
            next: FIRST + 10 * DAY,
        };
        watched.start();
        watched
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    fn data(&self) -> DataDir {
        DataDir::at(self.dir.path().join("data"))
    }

    fn start(&mut self) {
        let heard = Arc::clone(&self.heard);
        let config = EngineConfig {
            // Within one hour, so no hourly catch-up runs while a test counts.
            clock: Arc::new(ManualClock::at(FIRST + 10 * DAY + 2 * 3_600)),
            ..EngineConfig::new(self.data(), Some(self.root()))
        };
        let (engine, start) = Engine::spawn(
            config,
            Box::new(move |update| heard.lock().unwrap().push(update)),
        )
        .unwrap();
        assert!(matches!(start, Start::Connected { .. }));
        self.engine = Some(engine);
        self.wait_for("the first snapshot", |s| !s.is_empty());
        self.quiet();
    }

    fn snapshots(&self) -> Vec<Snapshot> {
        self.heard
            .lock()
            .unwrap()
            .iter()
            .filter_map(|update| match update {
                Update::Snapshot(snapshot) => Some((**snapshot).clone()),
                _ => None,
            })
            .collect()
    }

    fn working(&self) -> usize {
        self.snapshots().iter().filter(|s| s.stats.working).count()
    }

    fn wait_for(&self, what: &str, done: impl Fn(&[Snapshot]) -> bool) {
        let until = Instant::now() + PATIENCE;
        while !done(&self.snapshots()) {
            assert!(
                Instant::now() < until,
                "waited for {what}: {:?}",
                self.heard.lock().unwrap()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits until no update has come for [`QUIET`].
    fn quiet(&self) {
        let mut seen = self.heard.lock().unwrap().len();
        let mut since = Instant::now();
        let until = Instant::now() + PATIENCE;
        while since.elapsed() < QUIET {
            assert!(Instant::now() < until, "never quiet");
            thread::sleep(Duration::from_millis(50));
            let now = self.heard.lock().unwrap().len();
            if now != seen {
                seen = now;
                since = Instant::now();
            }
        }
    }

    fn commit(&mut self, branch: &str) -> CommitId {
        self.next += 60;
        self.repo
            .commit(
                branch,
                Stamp::at(self.next),
                "tend",
                &[Edit::write(
                    &format!("src/moss_{}.rs", self.next),
                    "fn moss() {}\n",
                )],
            )
            .unwrap()
    }

    fn rows(&self, kind: ObservationKind) -> Vec<StoredObservation> {
        let store = Store::open(&self.data()).unwrap();
        let id = store.setting::<i64>(SETTING_REPO_ACTIVE).unwrap().unwrap();
        store
            .observations(TerrariumId::new(id), Some(kind))
            .unwrap()
    }

    fn stop(&mut self) {
        self.engine.take().unwrap().shutdown();
    }
}

/// Writes `text` at `path` the way tools that write atomically do: a temporary file beside it,
/// then a rename over it.
fn write_by_rename(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let temp = path.with_extension("tmp");
    fs::write(&temp, text).unwrap();
    fs::rename(temp, path).unwrap();
}

#[test]
fn one_commit_is_one_refresh() {
    let mut w = Watched::new(|_, _| {});
    let before = w.working();
    w.commit("main");
    w.wait_for("the commit", |s| {
        s.iter().filter(|s| s.stats.working).count() > before
    });
    w.quiet();
    assert_eq!(w.working() - before, 1, "{:?}", w.snapshots());
    w.stop();
}

#[test]
fn a_storm_of_ref_writes_coalesces() {
    let mut w = Watched::new(|_, _| {});
    // A rebase's worth of writes: a marker, a detached HEAD, a run of commits each moving the
    // branch, then the marker gone and HEAD back on the branch.
    w.repo
        .write_git_file("rebase-merge/head-name", b"refs/heads/main\n")
        .unwrap();
    let base = w.repo.tip("main");
    w.repo.detach_head(&base).unwrap();
    let commits = 12;
    for _ in 0..commits {
        w.commit("main");
    }
    fs::remove_dir_all(w.repo.git_dir().join("rebase-merge")).unwrap();
    w.repo.set_head("main").unwrap();
    let before = w.working();
    w.wait_for("the storm", |s| {
        s.iter().filter(|s| s.stats.working).count() > before
    });
    w.quiet();
    let refreshes = w.working() - before;
    assert!(
        (1..commits).contains(&refreshes),
        "{refreshes} refreshes for {commits} commits"
    );
    w.stop();
}

#[test]
fn a_ref_moved_into_packed_refs_is_seen() {
    let mut archived = None;
    let mut w = Watched::new(|repo, _| {
        // A commit no branch reaches yet.
        repo.branch("archive", "main").unwrap();
        archived = Some(
            repo.commit(
                "archive",
                Stamp::at(FIRST + 10 * DAY + 30),
                "keep",
                &[Edit::write("old.rs", "fn old() {}\n")],
            )
            .unwrap(),
        );
        repo.delete_branch("archive").unwrap();
    });
    let archived = archived.unwrap();
    let packed = format!(
        "# pack-refs with: peeled fully-peeled sorted \n{} refs/heads/archive\n",
        archived.as_str()
    );
    let temp = w.repo.git_dir().join("packed-refs.lock");
    fs::write(&temp, packed).unwrap();
    let before = w.working();
    fs::rename(&temp, w.repo.git_dir().join("packed-refs")).unwrap();
    w.wait_for("the packed ref", |s| {
        s.iter().filter(|s| s.stats.working).count() > before
    });
    w.stop();
}

#[test]
fn reports_are_seen_however_they_are_written() {
    let mut w = Watched::new(|_, root| {
        fs::create_dir_all(root.join("target/surefire-reports")).unwrap();
    });
    // Renamed into place in a directory that did not exist when watching began.
    write_by_rename(
        &w.root().join("coverage/lcov.info"),
        "SF:src/fern_0.rs\nDA:1,1\nDA:2,0\nend_of_record\n",
    );
    w.wait_for("the coverage report", |_| {
        !w.rows(ObservationKind::Coverage).is_empty()
    });
    // A new file in a watched directory, as Surefire writes one per test class.
    fs::write(
        w.root()
            .join("target/surefire-reports/TEST-moss.FernTest.xml"),
        r#"<testsuite><testcase name="grows"/></testsuite>"#,
    )
    .unwrap();
    w.wait_for("the test report", |_| {
        w.rows(ObservationKind::Weather)
            .iter()
            .any(|row| row.record.raw.contains("tests"))
    });
    // A run `record-tests` recorded.
    let seed = Store::open(&w.data())
        .unwrap()
        .terrarium(TerrariumId::new(1))
        .unwrap()
        .unwrap()
        .seed;
    Status::new(1, 0, 1)
        .write(&w.data().local_tests(&seed))
        .unwrap();
    w.wait_for("the recorded run", |_| {
        w.rows(ObservationKind::Weather)
            .iter()
            .any(|row| row.record.raw.contains("exit_code"))
    });
    w.quiet();
    assert_eq!(w.rows(ObservationKind::Coverage).len(), 1);
    assert_eq!(w.rows(ObservationKind::Weather).len(), 2);
    w.stop();
}

/// Every file under `root`, by its path, to its bytes' hash.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, [u8; 32]> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, [u8; 32]>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                let bytes = fs::read(&path).unwrap();
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    Sha256::digest(bytes).into(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

#[test]
fn a_live_session_never_writes_into_the_repository() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = FixtureRepo::init(&dir.path().join("repo")).unwrap();
    for day in 0..5 {
        repo.commit(
            "main",
            Stamp::at(FIRST + day * DAY),
            "tend",
            &[Edit::write(&format!("src/fern_{day}.rs"), "fn fern() {}\n")],
        )
        .unwrap();
    }
    fs::write(
        dir.path().join("repo/junit.xml"),
        r#"<testsuite><testcase name="grows"/></testsuite>"#,
    )
    .unwrap();
    fs::File::options()
        .write(true)
        .open(dir.path().join("repo/junit.xml"))
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(2))
        .unwrap();
    let before = snapshot(&dir.path().join("repo"));
    let heard: Arc<Mutex<Vec<Update>>> = Arc::default();
    let into = Arc::clone(&heard);
    let (engine, _) = Engine::spawn(
        EngineConfig {
            clock: Arc::new(ManualClock::at(FIRST + 6 * DAY)),
            ..EngineConfig::new(
                DataDir::at(dir.path().join("data")),
                Some(dir.path().join("repo")),
            )
        },
        Box::new(move |update| into.lock().unwrap().push(update)),
    )
    .unwrap();
    let until = Instant::now() + PATIENCE;
    while heard.lock().unwrap().is_empty() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(20));
    }
    engine.refresh();
    thread::sleep(Duration::from_millis(1_500));
    engine.shutdown();
    assert_eq!(snapshot(&dir.path().join("repo")), before);
}
