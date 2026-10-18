use std::fs;
use std::sync::Mutex;
use std::thread;
use std::time::SystemTime;

use terrarium_adapters::fixture::{Edit, FixtureRepo, Stamp};
use terrarium_store::DataDir;
use terrarium_truth::domain::LeftCommit;

use super::replay::read_back;
use super::reports::unsettled;
use super::*;
use crate::schedule::DEBOUNCE;
use crate::worker::{Engine, Pipeline, Update};

/// A connected pipeline that replays before any history is read: the state no refresh
/// reaches, met head on.
struct Unread(Live);

impl Pipeline for Unread {
    fn refresh(&mut self, _: Refresh, _: &mut dyn FnMut(u8), _: &AtomicBool) -> Done {
        let ran = self.0.replay(None).map(|_| Done::default());
        self.0.done(ran)
    }

    fn watch_set(&self) -> Option<&WatchSet> {
        None
    }
}

#[test]
fn the_reports_are_read_from_the_folder_connected_wherever_the_config_puts_the_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let fixture = FixtureRepo::init(&repo).unwrap();
    let elsewhere = dir.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    let config = fixture.git_dir().join("config");
    let text = fs::read_to_string(&config).unwrap();
    let placed = format!("{text}[core]\n\tworktree = {}\n", elsewhere.display());
    fs::write(&config, placed).unwrap();
    let git = GitRepo::open(&repo).unwrap();
    assert_eq!(
        git.workdir().map(fs::canonicalize).transpose().unwrap(),
        Some(fs::canonicalize(&elsewhere).unwrap()),
        "the config does move the worktree"
    );
    assert_eq!(report_root(&git, &repo), Some(repo.clone()));

    let bare = dir.path().join("bare");
    FixtureRepo::init_bare(&bare).unwrap();
    assert_eq!(report_root(&GitRepo::open(&bare).unwrap(), &bare), None);
}

#[test]
fn a_refresh_missing_its_history_is_reported_and_never_ends_the_engine_silently() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    FixtureRepo::init(&repo)
        .unwrap()
        .commit(
            "main",
            Stamp::at(1_704_704_400),
            "tend",
            &[Edit::write("src/fern.rs", "fn fern() {}\n")],
        )
        .unwrap();
    let config = EngineConfig::new(DataDir::at(dir.path().join("data")), Some(repo));
    let clock = Arc::clone(&config.clock);
    let heard = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&heard);
    let (_engine, start) = Engine::spawn_pipeline(
        move || {
            Ok(Live::connect(&config)?.map(|live| {
                let start = live.start();
                (Unread(live), start)
            }))
        },
        clock,
        Box::new(move |update| into.lock().unwrap().push(update)),
    )
    .unwrap();
    assert!(matches!(start, Start::Connected { .. }));
    let until = Instant::now() + Duration::from_secs(10);
    while !heard
        .lock()
        .unwrap()
        .iter()
        .any(|update| matches!(update, Update::Problem(Problem::Missing("history"))))
    {
        assert!(Instant::now() < until, "{:?}", heard.lock().unwrap());
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_move_that_cannot_be_read_back_is_unknown_and_only_a_stop_ends_the_refresh() {
    let left = vec![LeftCommit {
        commit: CommitId::from_hex(&"a".repeat(40)).unwrap(),
        time: 0,
        deleted_dirs: Vec::new(),
    }];
    assert!(matches!(read_back(Ok(Some(left.clone()))), Ok(Some(l)) if l == left));
    assert!(matches!(read_back(Ok(None)), Ok(None)));
    assert!(matches!(
        read_back(Err(GitError::Read("a tree is gone".into()))),
        Ok(None)
    ));
    assert!(matches!(read_back(Err(GitError::NoCommits)), Ok(None)));
    assert!(matches!(
        read_back(Err(GitError::Cancelled)),
        Err(Fault::Cancelled)
    ));
}

#[test]
fn a_file_still_being_written_waits_and_a_still_one_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = dir.path().join("fresh.info");
    std::fs::write(&fresh, "SF:a\n").unwrap();
    let left = unsettled([fresh.clone()].into_iter()).expect("waits");
    assert!(left <= DEBOUNCE);
    let still = dir.path().join("still.info");
    std::fs::write(&still, "SF:a\n").unwrap();
    let file = std::fs::File::options().write(true).open(&still).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(2))
        .unwrap();
    assert_eq!(unsettled([still.clone()].into_iter()), None);
    // The newest of several decides.
    assert!(unsettled([still, fresh].into_iter()).is_some());
    assert_eq!(unsettled([dir.path().join("gone")].into_iter()), None);
    let future = dir.path().join("future.info");
    std::fs::write(&future, "SF:a\n").unwrap();
    let file = std::fs::File::options().write(true).open(&future).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(60))
        .unwrap();
    assert_eq!(unsettled([future].into_iter()), None);
}
