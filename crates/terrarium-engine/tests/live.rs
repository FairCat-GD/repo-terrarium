//! The live pipeline on fixture repositories in temporary directories, driven refresh by refresh
//! with a clock the test sets (N-04): connect and relink, reports
//! recorded once, a rewrite replayed in full and recorded as a `ref_rewrite`, the cache resuming
//! to what a full replay gives, the repository going missing and coming back, the clock jumping.
#![forbid(unsafe_code)]

use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use terrarium_adapters::fixture::{self, Edit, FixtureRepo, Stamp};
use terrarium_adapters::git::GitRepo;
use terrarium_adapters::local_tests::Status;
use terrarium_adapters::report::{Format, Report};
use terrarium_engine::live::Done;
use terrarium_engine::observe::record;
use terrarium_engine::schedule::{Cause, Reach, Refresh};
use terrarium_engine::{Branch, EngineConfig, Live, ManualClock, Replay, Snapshot, Start};
use terrarium_store::identifiers::SETTING_REPO_ACTIVE;
use terrarium_store::raw::RefRewriteRaw;
use terrarium_store::{DataDir, InstallKey, SimCache, Store, StoredObservation};
use terrarium_truth::{
    Basis, CommitId, ObservationKind, Prepared, SIM_VERSION, SimState, SimVersion,
};

/// Monday 2024-01-08, 09:00 UTC.
const FIRST: i64 = 1_704_704_400;
const DAY: i64 = 86_400;
const HOUR: i64 = 3_600;
const MINUTE: i64 = 60;

const LCOV: &str = "SF:src/fern_0.rs\nDA:1,1\nDA:2,1\nDA:3,0\nDA:4,1\nend_of_record\n";
const LCOV_LOWER: &str = "SF:src/fern_0.rs\nDA:1,1\nDA:2,0\nDA:3,0\nDA:4,1\nend_of_record\n";
const JUNIT: &str = r#"<testsuite><testcase name="grows"/></testsuite>"#;

/// A repository with a commit on `main` each day, a data directory beside it, and a clock.
struct Fixture {
    dir: tempfile::TempDir,
    repo: FixtureRepo,
    main: Vec<CommitId>,
    clock: ManualClock,
}

impl Fixture {
    fn new(days: i64) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = FixtureRepo::init(&dir.path().join("repo")).unwrap();
        let main = (0..days)
            .map(|day| commit(&mut repo, "main", day * DAY))
            .collect();
        Fixture {
            dir,
            repo,
            main,
            clock: ManualClock::at(FIRST + days * DAY + 2 * HOUR + 30 * MINUTE),
        }
    }

    /// A commit on `main` dated a minute ago: in the hour still open, as a fresh commit is.
    fn commit_now(&mut self) -> CommitId {
        let now = terrarium_engine::Clock::wall_secs(&self.clock);
        commit(&mut self.repo, "main", now - MINUTE - FIRST)
    }

    fn repo_path(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    fn data(&self) -> DataDir {
        DataDir::at(self.dir.path().join("data"))
    }

    fn config(&self, repo: Option<PathBuf>) -> EngineConfig {
        EngineConfig {
            clock: Arc::new(self.clock.clone()),
            ..EngineConfig::new(self.data(), repo)
        }
    }

    /// Connects with `--repo`.
    fn connect(&self) -> Live {
        Live::connect(&self.config(Some(self.repo_path())))
            .unwrap()
            .expect("connected")
    }

    /// Connects as a plain run does.
    fn reconnect(&self) -> Option<Live> {
        Live::connect(&self.config(None)).unwrap()
    }

    /// Writes a report into the worktree, dated two seconds ago so it reads as still.
    fn report(&self, path: &str, text: &str) {
        write_still(&self.repo_path().join(path), text);
    }

    fn store(&self) -> Store {
        Store::open(&self.data()).unwrap()
    }

    fn rows(&self, kind: ObservationKind) -> Vec<StoredObservation> {
        let store = self.store();
        let id = store
            .setting::<i64>(SETTING_REPO_ACTIVE)
            .unwrap()
            .expect("an active terrarium");
        store
            .observations(terrarium_store::TerrariumId::new(id), Some(kind), None)
            .unwrap()
    }

    /// Truth replayed from nothing at the clock's time: the history read afresh and every
    /// stored observation.
    fn full_replay(&self) -> SimState {
        let git = GitRepo::open(&self.repo_path()).unwrap();
        let history = git
            .read_history("main", &mut |_| ControlFlow::Continue(()))
            .unwrap();
        let seed = git.identity("main").unwrap().seed;
        let store = self.store();
        let terrarium = store.terrarium_by_seed(&seed).unwrap().unwrap();
        let observations: Vec<_> = store
            .observations(terrarium.id, None, None)
            .unwrap()
            .iter()
            .map(|row| row.to_observation().unwrap())
            .collect();
        let basis = Basis::new(seed, history.grid, "main");
        let prepared = Prepared::new(basis.clone(), &history.commits, &observations).unwrap();
        let mut state = SimState::new(basis);
        let now = terrarium_engine::Clock::wall_secs(&self.clock);
        state
            .advance_through(&prepared, history.grid.tick_at(now).unwrap())
            .unwrap();
        state
    }
}

fn commit(repo: &mut FixtureRepo, branch: &str, at: i64) -> CommitId {
    repo.commit(
        branch,
        Stamp::at(FIRST + at),
        "tend",
        &[Edit::write(&format!("src/fern_{at}.rs"), "fn fern() {}\n")],
    )
    .unwrap()
}

fn write_still(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(2))
        .unwrap();
}

fn refresh(live: &mut Live, cause: Cause, reports: bool) -> Done {
    live.refresh(
        Refresh { cause, reports },
        &mut |_| {},
        &AtomicBool::new(false),
    )
}

/// A refresh that reached the repository and sent a snapshot.
fn shown(live: &mut Live, cause: Cause, reports: bool) -> Snapshot {
    let done = refresh(live, cause, reports);
    assert_eq!(done.reach, Some(Reach::Found), "{done:?}");
    assert_eq!(done.problem, None);
    done.snapshot.expect("a snapshot")
}

#[test]
fn connect_derives_the_seed_as_record_tests_does_and_relinks_by_it() {
    let fx = Fixture::new(12);
    let mut live = fx.connect();
    let git = GitRepo::open(&fx.repo_path()).unwrap();
    let seed = git.identity(&git.default_branch().unwrap()).unwrap().seed;
    assert_eq!(live.terrarium().seed, seed);
    let Start::Connected {
        repo_name,
        install_salt,
        scene,
    } = live.start()
    else {
        panic!("connected");
    };
    assert_eq!(repo_name, "repo");
    assert_eq!(scene, None, "no cache yet");
    assert_ne!(install_salt, [0; 16], "a random salt");
    fx.report("coverage/lcov.info", LCOV);
    shown(&mut live, Cause::Connect, true);
    assert_eq!(fx.rows(ObservationKind::Coverage).len(), 1);
    let id = live.terrarium().id;
    drop(live);

    // Moved, and connected again by path: the same terrarium, its salt and its observations.
    fs::rename(fx.repo_path(), fx.dir.path().join("moved")).unwrap();
    let moved = Live::connect(&fx.config(Some(fx.dir.path().join("moved"))))
        .unwrap()
        .unwrap();
    assert_eq!(moved.terrarium().id, id);
    assert_eq!(moved.terrarium().install_salt, install_salt);
    assert!(moved.terrarium().repo.path.ends_with("moved"));
    drop(moved);
    let plain = fx.reconnect().expect("the active terrarium");
    assert_eq!(plain.terrarium().id, id);
    assert_eq!(fx.rows(ObservationKind::Coverage).len(), 1);
}

#[test]
fn a_plain_run_with_nothing_connected_connects_nothing() {
    let fx = Fixture::new(2);
    assert!(fx.reconnect().is_none());
    let not_a_repo = Live::connect(&fx.config(Some(fx.dir.path().to_path_buf())));
    assert!(matches!(
        not_a_repo,
        Err(terrarium_engine::ConnectError::Git(_))
    ));
}

#[test]
fn a_plain_run_keeps_the_recorded_default_branch() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = FixtureRepo::init(&dir.path().join("repo")).unwrap();
    repo.set_head("trunk").unwrap();
    for day in 0..5 {
        commit(&mut repo, "trunk", day * DAY);
    }
    let clock = ManualClock::at(FIRST + 6 * DAY);
    let config = |repo: Option<PathBuf>| EngineConfig {
        clock: Arc::new(clock.clone()),
        ..EngineConfig::new(DataDir::at(dir.path().join("data")), repo)
    };
    let live = Live::connect(&config(Some(dir.path().join("repo"))))
        .unwrap()
        .unwrap();
    assert_eq!(live.terrarium().repo.default_branch, "trunk");
    drop(live);
    // With no main or master, the default branch would now follow HEAD; the recorded one holds.
    repo.branch("moss", "trunk").unwrap();
    repo.set_head("moss").unwrap();
    commit(&mut repo, "moss", 5 * DAY);
    assert_eq!(
        GitRepo::open(&dir.path().join("repo"))
            .unwrap()
            .default_branch()
            .unwrap(),
        "moss"
    );
    let mut plain = Live::connect(&config(None)).unwrap().unwrap();
    let snapshot = shown(&mut plain, Cause::Connect, true);
    assert_eq!(snapshot.branch, Branch::Named("moss".into()));
    assert_eq!(plain.state().unwrap().basis.default_branch, "trunk");
}

#[test]
fn a_report_is_recorded_once_and_again_only_when_it_changes() {
    let fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    assert!(fx.rows(ObservationKind::Coverage).is_empty());

    fx.report("coverage/lcov.info", LCOV);
    let first = refresh(&mut live, Cause::Watch, true);
    assert!(first.snapshot.unwrap().stats.working);
    let rows = fx.rows(ObservationKind::Coverage);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].record.head.commit, fx.main[11]);
    assert!(rows[0].record.head.at_default_tip);

    // Unchanged: nothing recorded, nothing to show.
    let again = refresh(&mut live, Cause::Watch, true);
    assert_eq!(again.snapshot, None);
    assert_eq!(fx.rows(ObservationKind::Coverage).len(), 1);

    // A refresh that does not read reports leaves a changed one for later.
    fx.report("coverage/lcov.info", LCOV_LOWER);
    refresh(&mut live, Cause::Hour, false);
    assert_eq!(fx.rows(ObservationKind::Coverage).len(), 1);
    refresh(&mut live, Cause::Watch, true);
    assert_eq!(fx.rows(ObservationKind::Coverage).len(), 2);
}

#[test]
fn a_report_still_being_written_waits_for_a_follow_up() {
    let fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    let path = fx.repo_path().join("coverage/lcov.info");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    // Cut between two records, as lcov can be with no end marker to say so.
    fs::write(&path, "SF:src/fern_0.rs\nDA:1,1\nend_of_record\n").unwrap();
    let done = refresh(&mut live, Cause::Watch, true);
    assert!(
        done.follow_up
            .is_some_and(|wait| wait <= Duration::from_millis(500))
    );
    assert!(fx.rows(ObservationKind::Coverage).is_empty());
    fx.report("coverage/lcov.info", LCOV);
    refresh(&mut live, Cause::FollowUp, true);
    assert_eq!(fx.rows(ObservationKind::Coverage).len(), 1);
}

#[test]
fn both_local_weather_writers_record_without_colliding_and_neither_repeats_the_other() {
    let fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    let status = fx.data().local_tests(&live.terrarium().seed);
    let run = |exit_code| {
        Status::new(exit_code, 1, 2).write(&status).unwrap();
        fs::File::options()
            .write(true)
            .open(&status)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(1))
            .unwrap();
    };
    // `record-tests` ran a command that wrote a JUnit report: both change in one refresh, at
    // the same HEAD and the same second.
    fx.report("junit.xml", JUNIT);
    run(0);
    refresh(&mut live, Cause::Watch, true);
    let rows = fx.rows(ObservationKind::Weather);
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].record.recorded_at, rows[1].record.recorded_at);
    assert!(
        rows[1].record.raw.contains("exit_code"),
        "the newer file last"
    );

    // Another run fails; the JUnit report is unchanged and is not recorded again.
    run(1);
    refresh(&mut live, Cause::Watch, true);
    let rows = fx.rows(ObservationKind::Weather);
    assert_eq!(rows.len(), 3);
    assert!(rows[2].record.raw.contains(r#""exit_code":1"#));
    // After a restart, the newest of each writer is known: nothing repeats.
    drop(live);
    let mut plain = fx.reconnect().unwrap();
    refresh(&mut plain, Cause::Connect, true);
    assert_eq!(fx.rows(ObservationKind::Weather).len(), 3);
}

#[test]
fn a_rewrite_replays_in_full_and_records_one_ref_rewrite() {
    let mut fx = Fixture::new(12);
    let mut live = fx.connect();
    assert_eq!(
        shown(&mut live, Cause::Connect, true).stats.replay,
        Replay::Full
    );
    // A commit on top fast-forwards: no rewrite, and the cached state resumes.
    fx.commit_now();
    let forward = shown(&mut live, Cause::Watch, true);
    assert_eq!(forward.stats.replay, Replay::Resumed);
    // So does a branch already read moving the default branch on: the reader reads that in
    // full (the first-parent chain moved), but the old tip is an ancestor of the new one.
    fx.repo.branch("moss", "main").unwrap();
    commit(&mut fx.repo, "moss", 12 * DAY + 2 * HOUR + 20 * MINUTE);
    refresh(&mut live, Cause::Watch, true);
    let tip = fx.repo.tip("moss");
    fx.repo.reset("main", &tip).unwrap();
    shown(&mut live, Cause::Watch, true);
    assert!(fx.rows(ObservationKind::RefRewrite).is_empty());

    // Reset back a week and committed again: history before the open hour changed.
    fx.repo.reset("main", &fx.main[5]).unwrap();
    let new = fx.commit_now();
    let rewritten = shown(&mut live, Cause::Watch, true);
    assert_eq!(rewritten.stats.replay, Replay::Full);
    let rewrites = fx.rows(ObservationKind::RefRewrite);
    assert_eq!(rewrites.len(), 1);
    let raw: RefRewriteRaw = serde_json::from_str(&rewrites[0].record.raw).unwrap();
    assert_eq!(raw.reference, "refs/heads/main");
    assert_eq!(raw.old, tip);
    assert_eq!(raw.new, new);
    assert_eq!(rewrites[0].record.head.commit, new);
    assert_eq!(live.state(), Some(&fx.full_replay()));
}

#[test]
fn refreshes_across_hours_and_a_restart_equal_a_full_replay() {
    let mut fx = Fixture::new(20);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    for (hours, lcov) in [(1, LCOV), (5, LCOV_LOWER), (30, LCOV)] {
        fx.clock.advance(hours * HOUR);
        fx.commit_now();
        fx.report("coverage/lcov.info", lcov);
        let snapshot = shown(&mut live, Cause::Watch, true);
        assert_eq!(snapshot.stats.replay, Replay::Resumed);
        assert_eq!(live.state(), Some(&fx.full_replay()));
    }
    let shown_before = live.state().cloned();
    drop(live);

    // The next session starts from the cache, which resumes to the same state.
    let mut next = fx.reconnect().unwrap();
    let Start::Connected { scene, .. } = next.start() else {
        panic!("connected");
    };
    assert!(scene.is_some(), "the cached scene is shown at once");
    let resumed = shown(&mut next, Cause::Connect, true);
    assert_eq!(resumed.stats.replay, Replay::Resumed);
    assert_eq!(next.state(), shown_before.as_ref());
    fx.clock.advance(50 * HOUR);
    let later = shown(&mut next, Cause::Hour, false);
    assert_eq!(later.stats.replay, Replay::Resumed);
    assert_eq!(next.state(), Some(&fx.full_replay()));
    let cache = fx.store().sim_cache(next.terrarium().id).unwrap().unwrap();
    assert_eq!(cache.sim_version, SIM_VERSION);
    // The cache holds the state before the open hour; the live view is through it.
    assert_eq!(
        cache.tick.saturating_add(1),
        next.state().unwrap().next_tick
    );
}

#[test]
fn a_refresh_reaches_its_snapshot_within_60_ms_of_the_adapters() {
    let fx = Fixture::new(60);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    let fastest = [LCOV, LCOV_LOWER, LCOV]
        .iter()
        .map(|lcov| {
            fx.report("coverage/lcov.info", lcov);
            let snapshot = shown(&mut live, Cause::Watch, true);
            assert!(snapshot.stats.working);
            assert_eq!(snapshot.stats.replay, Replay::Resumed);
            snapshot.stats.adapters_to_snapshot
        })
        .min()
        .unwrap();
    assert!(fastest < Duration::from_millis(60), "N-04: {fastest:?}");
}

#[test]
fn a_cache_from_another_sim_version_is_ignored_and_the_label_notes_it_once() {
    let fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    let id = live.terrarium().id;
    drop(live);
    let mut store = fx.store();
    let mut cache = store.sim_cache(id).unwrap().unwrap();
    cache.sim_version = SimVersion(SIM_VERSION.0 - 1);
    store.write_sim_cache(id, &cache).unwrap();
    store
        .set_install_value(InstallKey::LastSimVersion, &(SIM_VERSION.0 - 1).to_string())
        .unwrap();
    drop(store);

    let mut upgraded = fx.reconnect().unwrap();
    let Start::Connected { scene, .. } = upgraded.start() else {
        panic!("connected");
    };
    assert_eq!(scene, None, "an old version's cache is never decoded");
    let snapshot = shown(&mut upgraded, Cause::Connect, true);
    assert_eq!(snapshot.stats.replay, Replay::Full);
    assert!(snapshot.sim_note);
    assert_eq!(
        fx.store()
            .sim_cache(id)
            .unwrap()
            .map(|cache: SimCache| cache.sim_version),
        Some(SIM_VERSION)
    );
    drop(upgraded);
    let mut next = fx.reconnect().unwrap();
    assert!(!shown(&mut next, Cause::Connect, true).sim_note);
}

#[test]
fn a_missing_repository_freezes_and_its_return_resumes() {
    let mut fx = Fixture::new(12);
    let mut live = fx.connect();
    let before = shown(&mut live, Cause::Connect, true);
    let away = fx.dir.path().join("away");
    fs::rename(fx.repo_path(), &away).unwrap();
    let missing = refresh(&mut live, Cause::Hour, false);
    assert_eq!(missing.reach, Some(Reach::NotFound));
    assert_eq!(missing.snapshot, None);
    assert!(live.watch_set().is_none());
    fs::rename(&away, fx.repo_path()).unwrap();
    let back = shown(&mut live, Cause::Retry, true);
    assert_eq!(back.scene, before.scene);

    // Another repository at the same path is not this terrarium's.
    fs::rename(fx.repo_path(), &away).unwrap();
    let mut other = FixtureRepo::init(&fx.repo_path()).unwrap();
    other
        .commit(
            "main",
            Stamp::at(FIRST),
            "other",
            &[Edit::write("b", "b\n")],
        )
        .unwrap();
    assert_eq!(
        refresh(&mut live, Cause::Retry, true).reach,
        Some(Reach::NotFound)
    );
    fs::remove_dir_all(fx.repo_path()).unwrap();
    fs::rename(&away, fx.repo_path()).unwrap();
    assert_eq!(
        refresh(&mut live, Cause::Retry, true).reach,
        Some(Reach::Found)
    );

    // The default branch deleted: frozen the same way.
    fx.repo.branch("moss", "main").unwrap();
    fx.repo.set_head("moss").unwrap();
    fx.repo.delete_branch("main").unwrap();
    assert_eq!(
        refresh(&mut live, Cause::Watch, true).reach,
        Some(Reach::NotFound)
    );
}

#[test]
fn a_clock_jump_catches_up_once_and_a_clock_set_back_never_rewinds() {
    let fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    // Asleep through a UTC midnight: one catch-up, equal to replaying from the start.
    fx.clock.advance(15 * HOUR);
    let caught_up = shown(&mut live, Cause::Hour, false);
    assert_eq!(caught_up.stats.replay, Replay::Resumed);
    assert_eq!(live.state(), Some(&fx.full_replay()));
    let at = live.state().unwrap().next_tick;
    // The same hour again changes nothing.
    assert_eq!(refresh(&mut live, Cause::Hour, false).snapshot, None);

    // Set back three hours: the state stays where it was, and a report read now is dated no
    // earlier than the session's time.
    fx.clock.advance(-3 * HOUR);
    fx.report("coverage/lcov.info", LCOV);
    refresh(&mut live, Cause::Watch, true);
    assert_eq!(live.state().unwrap().next_tick, at);
    let recorded = &fx.rows(ObservationKind::Coverage)[0].record;
    assert!(recorded.recorded_at >= FIRST + 12 * DAY + 17 * HOUR + 30 * MINUTE);
}

#[test]
fn a_long_history_read_reports_progress_and_stops_when_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    fixture::synthetic(&dir.path().join("repo"), 300).unwrap();
    let config = EngineConfig {
        progress_after: Duration::ZERO,
        clock: Arc::new(ManualClock::at(2_000_000_000)),
        ..EngineConfig::new(
            DataDir::at(dir.path().join("data")),
            Some(dir.path().join("repo")),
        )
    };
    let mut live = Live::connect(&config).unwrap().unwrap();
    let cancel = AtomicBool::new(false);
    let mut heard = Vec::new();
    let done = live.refresh(
        Refresh {
            cause: Cause::Connect,
            reports: true,
        },
        &mut |percent| {
            heard.push(percent);
            if percent >= 10 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
        &cancel,
    );
    assert_eq!(done.reach, None, "cancelled");
    assert!(live.state().is_none());
    assert!(heard.windows(2).all(|pair| pair[0] < pair[1]), "{heard:?}");
    assert!(
        heard.last().is_some_and(|&p| (10..100).contains(&p)),
        "{heard:?}"
    );

    let mut all = Vec::new();
    let done = live.refresh(
        Refresh {
            cause: Cause::Connect,
            reports: true,
        },
        &mut |percent| all.push(percent),
        &AtomicBool::new(false),
    );
    assert_eq!(done.reach, Some(Reach::Found));
    assert_eq!(all.last(), Some(&100));
}

/// Appends, behind the pipeline's back, the observation `text` makes as a report of `format`
/// at `path`, read at the repository's HEAD and dated `at`.
fn stored_behind(fx: &Fixture, format: Format, path: &str, text: &str, at: i64) {
    let elsewhere = fx.dir.path().join("elsewhere");
    write_still(&elsewhere.join(path), text);
    let report = Report::read(&elsewhere, format, &[path.to_owned()]);
    let head = GitRepo::open(&fx.repo_path())
        .unwrap()
        .head("main")
        .unwrap();
    let mut store = fx.store();
    let id = store.setting::<i64>(SETTING_REPO_ACTIVE).unwrap().unwrap();
    store
        .append_observation(
            terrarium_store::TerrariumId::new(id),
            &record(&report, head, at).unwrap(),
        )
        .unwrap();
}

#[test]
fn a_refresh_that_fails_after_reading_new_history_replays_on_the_next() {
    let mut fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    let new = fx.commit_now();
    // A row at the second, HEAD and kind the next report takes: its append fails, after the
    // new commit was read.
    let now = terrarium_engine::Clock::wall_secs(&fx.clock);
    stored_behind(&fx, Format::Lcov, "coverage/lcov.info", LCOV_LOWER, now);
    fx.report("coverage/lcov.info", LCOV);
    let failed = refresh(&mut live, Cause::Watch, true);
    assert!(failed.problem.is_some(), "{failed:?}");
    fs::remove_file(fx.repo_path().join("coverage/lcov.info")).unwrap();
    fx.clock.advance(1);
    refresh(&mut live, Cause::Watch, true);
    assert_eq!(live.state().unwrap().habitat.default_tip, Some(new));
}

#[test]
fn a_report_is_not_recorded_again_after_a_row_dated_later() {
    let fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    drop(live);
    // A row dated a day ahead, as one recorded while the clock ran fast would be.
    let now = terrarium_engine::Clock::wall_secs(&fx.clock);
    stored_behind(
        &fx,
        Format::JUnit,
        "junit.xml",
        r#"<testsuite><testcase name="old"/></testsuite>"#,
        now + DAY,
    );
    let mut live = fx.reconnect().unwrap();
    fx.report("junit.xml", JUNIT);
    refresh(&mut live, Cause::Connect, true);
    assert_eq!(fx.rows(ObservationKind::Weather).len(), 2);
    for _ in 0..3 {
        fx.clock.advance(5);
        refresh(&mut live, Cause::Watch, true);
    }
    assert_eq!(fx.rows(ObservationKind::Weather).len(), 2);
}

#[test]
fn a_path_through_a_parent_names_the_folder_it_means() {
    let fx = Fixture::new(2);
    fs::create_dir_all(fx.repo_path().join("sub")).unwrap();
    let live = Live::connect(&fx.config(Some(fx.repo_path().join("sub/.."))))
        .unwrap()
        .unwrap();
    assert_eq!(live.terrarium().repo.name, "repo");
    assert!(!live.terrarium().repo.path.contains(".."));
}
