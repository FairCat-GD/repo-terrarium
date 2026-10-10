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
use terrarium_engine::{
    Branch, ConnectError, EngineConfig, Live, ManualClock, Replay, Snapshot, Start, lifecycle,
};
use terrarium_store::identifiers::SETTING_REPO_ACTIVE;
use terrarium_store::raw::RefRewriteRaw;
use terrarium_store::{DataDir, InstallKey, SimCache, Store, StoredObservation};
use terrarium_truth::landmarks::fossil_id;
use terrarium_truth::scene::{EventKind, GroundMark, Light, Strength};
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
        Fixture::made_by(days, FixtureRepo::init)
    }

    /// The same, as a bare repository: no worktree.
    fn bare(days: i64) -> Fixture {
        Fixture::made_by(days, FixtureRepo::init_bare)
    }

    fn made_by(days: i64, init: fn(&Path) -> fixture::FixtureResult<FixtureRepo>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = init(&dir.path().join("repo")).unwrap();
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
            .observations(terrarium_store::TerrariumId::new(id), Some(kind))
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
            .observations(terrarium.id, None)
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
    let Start::Connected { terrarium, scene } = live.start() else {
        panic!("connected");
    };
    let install_salt = terrarium.install_salt;
    assert_eq!(*terrarium, *live.terrarium());
    assert_eq!(terrarium.repo.name, "repo");
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
fn a_bare_repository_records_its_local_test_runs() {
    let fx = Fixture::bare(12);
    assert_eq!(GitRepo::open(&fx.repo_path()).unwrap().workdir(), None);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    // `record-tests` keeps the status file in the data directory, so a repository with no
    // worktree has one too.
    let status = fx.data().local_tests(&live.terrarium().seed);
    Status::new(1, 1, 2).write(&status).unwrap();
    fs::File::options()
        .write(true)
        .open(&status)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(1))
        .unwrap();
    refresh(&mut live, Cause::Watch, true);
    let rows = fx.rows(ObservationKind::Weather);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].record.raw.contains(r#""exit_code":1"#));
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
    // Off the old chain: main's last six days, the commit on top and the branch's commit.
    assert_eq!(raw.rewritten, Some(8));
    assert!(raw.fossils.is_empty());
    assert_eq!(live.state(), Some(&fx.full_replay()));
    let quake = rewritten.scene.event.expect("an earthquake");
    assert_eq!(
        (quake.kind, quake.strength),
        (EventKind::Earthquake, Strength::Mid)
    );
    assert_eq!(rewritten.scene.ground_marks.len(), 1);
}

#[test]
fn a_rewrite_that_takes_a_fossil_records_it_and_opens_a_sinkhole_for_it() {
    let mut fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    let at = |fx: &Fixture| Stamp::at(terrarium_engine::Clock::wall_secs(&fx.clock) - MINUTE);
    fx.clock.advance(HOUR);
    let planted = fx
        .repo
        .commit(
            "main",
            at(&fx),
            "plant",
            &[Edit::write("src/old/roots.rs", &"root\n".repeat(600))],
        )
        .unwrap();
    shown(&mut live, Cause::Watch, true);
    fx.clock.advance(2 * HOUR);
    let cleared = fx
        .repo
        .commit(
            "main",
            at(&fx),
            "clear",
            &[Edit::Remove("src/old/roots.rs".into())],
        )
        .unwrap();
    let sinking = shown(&mut live, Cause::Watch, true);
    let event = sinking.scene.event.expect("a sinkhole");
    assert_eq!(event.kind, EventKind::Sinkhole);
    let fossil = event.fossil.expect("names its fossil");
    assert_eq!(
        sinking
            .scene
            .fossils
            .iter()
            .map(|f| f.id)
            .collect::<Vec<_>>(),
        [fossil]
    );
    // The deletion is amended away.
    fx.clock.advance(2 * HOUR);
    fx.repo.reset("main", &planted).unwrap();
    fx.commit_now();
    let shaken = shown(&mut live, Cause::Watch, true);
    let rows = fx.rows(ObservationKind::RefRewrite);
    let raw: RefRewriteRaw = serde_json::from_str(&rows[0].record.raw).unwrap();
    assert!(raw.fossils.is_empty());
    let left: Vec<(&CommitId, &str, u64)> = raw
        .left
        .iter()
        .flat_map(|l| {
            l.deleted
                .iter()
                .map(move |d| (&l.commit, d.path.as_str(), d.lines))
        })
        .collect();
    assert_eq!(
        (raw.rewritten, left),
        (Some(1), vec![(&cleared, "src/old", 600)])
    );
    assert!(shaken.scene.fossils.is_empty());
    assert_eq!(
        shaken.scene.event.map(|e| e.kind),
        Some(EventKind::Earthquake)
    );
    let sinkholes: Vec<Option<u32>> = shaken
        .scene
        .ground_marks
        .iter()
        .filter(|m| m.kind == GroundMark::Sinkhole)
        .map(|m| m.fossil)
        .collect();
    // The replayed history never deleted anything: the one sinkhole is the rewrite's.
    assert_eq!(sinkholes, [Some(fossil)]);
    assert_eq!(live.state(), Some(&fx.full_replay()));
}

#[test]
fn a_playing_event_yields_only_to_a_stronger_one_and_leaves_after_the_next_hour() {
    let mut fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    let at = |fx: &Fixture| Stamp::at(terrarium_engine::Clock::wall_secs(&fx.clock) - MINUTE);
    fx.clock.advance(HOUR);
    fx.commit_now();
    fx.report("coverage/lcov.info", LCOV);
    shown(&mut live, Cause::Watch, true);
    fx.clock.advance(HOUR);
    fx.repo
        .commit(
            "main",
            at(&fx),
            "plant",
            &[Edit::write("src/old/roots.rs", &"root\n".repeat(600))],
        )
        .unwrap();
    shown(&mut live, Cause::Watch, true);
    fx.clock.advance(HOUR);
    fx.repo
        .commit(
            "main",
            at(&fx),
            "clear",
            &[Edit::Remove("src/old/roots.rs".into())],
        )
        .unwrap();
    let sinking = shown(&mut live, Cause::Watch, true);
    assert_eq!(
        sinking.scene.event.map(|e| e.kind),
        Some(EventKind::Sinkhole)
    );
    // Coverage falls in the same hour: a dusk, which the sinkhole outranks.
    fx.report("coverage/lcov.info", LCOV_LOWER);
    let dusk = shown(&mut live, Cause::Watch, true);
    assert_eq!(dusk.scene.light, Light::Dusk);
    assert_eq!(dusk.scene.event, sinking.scene.event);
    // Through the next hour it stays; after that it leaves the scene.
    fx.clock.advance(HOUR);
    assert!(refresh(&mut live, Cause::Hour, false).snapshot.is_none());
    fx.clock.advance(HOUR);
    let after = shown(&mut live, Cause::Hour, false);
    assert_eq!(after.scene.event, None);
}

#[test]
fn a_rewrite_made_while_closed_is_recorded_at_the_next_start() {
    let mut fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    fx.clock.advance(3 * HOUR);
    refresh(&mut live, Cause::Hour, false);
    drop(live);
    // While nothing watches, main is reset back three days and committed on.
    fx.repo.reset("main", &fx.main[8]).unwrap();
    fx.clock.advance(HOUR);
    let new = fx.commit_now();
    let mut next = fx.reconnect().unwrap();
    let Start::Connected { scene, .. } = next.start() else {
        panic!("connected");
    };
    assert_eq!(
        scene.and_then(|s| s.event),
        None,
        "the cached scene plays nothing"
    );
    let started = shown(&mut next, Cause::Connect, true);
    let rows = fx.rows(ObservationKind::RefRewrite);
    assert_eq!(rows.len(), 1);
    let raw: RefRewriteRaw = serde_json::from_str(&rows[0].record.raw).unwrap();
    // What it took off is read back from the object database: three days' commits.
    assert_eq!(
        (raw.old, raw.new, raw.rewritten),
        (fx.main[11].clone(), new, Some(3))
    );
    let quake = started.scene.event.expect("an earthquake");
    assert_eq!(
        (quake.kind, quake.strength),
        (EventKind::Earthquake, Strength::Mid)
    );
    // The next start records nothing more.
    drop(next);
    let mut again = fx.reconnect().unwrap();
    shown(&mut again, Cause::Connect, true);
    assert_eq!(fx.rows(ObservationKind::RefRewrite).len(), 1);
}

#[test]
fn reading_back_a_move_made_while_closed_never_sends_the_history_s_percent_back() {
    let mut fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    fx.clock.advance(3 * HOUR);
    refresh(&mut live, Cause::Hour, false);
    drop(live);
    fx.repo.reset("main", &fx.main[4]).unwrap();
    fx.clock.advance(HOUR);
    fx.commit_now();
    let config = EngineConfig {
        progress_after: Duration::ZERO,
        ..fx.config(None)
    };
    let mut next = Live::connect(&config).unwrap().unwrap();
    let mut heard = Vec::new();
    let done = next.refresh(
        Refresh {
            cause: Cause::Connect,
            reports: true,
        },
        &mut |percent| heard.push(percent),
        &AtomicBool::new(false),
    );
    assert_eq!(done.reach, Some(Reach::Found));
    assert_eq!(
        fx.rows(ObservationKind::RefRewrite).len(),
        1,
        "the move was read back"
    );
    assert!(!heard.is_empty());
    assert!(heard.windows(2).all(|pair| pair[0] <= pair[1]), "{heard:?}");
}

#[test]
fn a_fast_forward_made_while_closed_through_a_merge_records_nothing() {
    let mut fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    fx.clock.advance(3 * HOUR);
    refresh(&mut live, Cause::Hour, false);
    drop(live);
    // A branch merges main into itself, and main fast-forwards to the merge: the old tip is
    // an ancestor of the new one, though no longer on its first-parent chain.
    let old_tip = fx.repo.tip("main");
    fx.repo.branch("moss", "main").unwrap();
    fx.repo.reset("moss", &fx.main[6]).unwrap();
    commit(&mut fx.repo, "moss", 12 * DAY + 2 * HOUR + 40 * MINUTE);
    let at = terrarium_engine::Clock::wall_secs(&fx.clock) - MINUTE;
    let merged = fx
        .repo
        .merge("moss", "main", Stamp::at(at), "merge main")
        .unwrap();
    fx.repo.reset("main", &merged).unwrap();
    fx.clock.advance(HOUR);
    let mut next = fx.reconnect().unwrap();
    shown(&mut next, Cause::Connect, true);
    let history = GitRepo::open(&fx.repo_path())
        .unwrap()
        .read_history("main", &mut |_| ControlFlow::Continue(()))
        .unwrap();
    assert!(!history.on_default_chain(&old_tip), "off the chain");
    assert!(fx.rows(ObservationKind::RefRewrite).is_empty());
}

#[test]
fn a_rewrite_opens_sinkholes_for_exactly_the_shown_fossils_it_took() {
    for kept in [false, true] {
        let mut fx = Fixture::new(12);
        let mut live = fx.connect();
        shown(&mut live, Cause::Connect, true);
        let at = |fx: &Fixture| Stamp::at(terrarium_engine::Clock::wall_secs(&fx.clock) - MINUTE);
        // Seven modules planted and cleared, so seven fossils, of which five show.
        let mut cleared = Vec::new();
        let mut planted = Vec::new();
        for k in 0..7 {
            fx.clock.advance(HOUR);
            let file = format!("src/m{k}/roots.rs");
            planted.push(
                fx.repo
                    .commit(
                        "main",
                        at(&fx),
                        "plant",
                        &[
                            Edit::write("src/keep.rs", "keep\n"),
                            Edit::write(&file, &"root\n".repeat(600)),
                        ],
                    )
                    .unwrap(),
            );
            fx.clock.advance(HOUR);
            cleared.push(
                fx.repo
                    .commit("main", at(&fx), "clear", &[Edit::Remove(file)])
                    .unwrap(),
            );
        }
        let before = shown(&mut live, Cause::Watch, true);
        let showing: Vec<u32> = before.scene.fossils.iter().map(|f| f.id).collect();
        let ids = |commits: &[CommitId]| -> Vec<u32> { commits.iter().map(fossil_id).collect() };
        assert_eq!(showing, ids(&cleared[2..]));
        let state = live.state().unwrap().clone();
        // The last two clearings are amended away.
        if kept {
            fx.repo.branch("kept", "main").unwrap();
        }
        fx.clock.advance(HOUR);
        fx.repo.reset("main", &planted[5]).unwrap();
        fx.commit_now();
        let shaken = shown(&mut live, Cause::Watch, true);
        let history = GitRepo::open(&fx.repo_path())
            .unwrap()
            .read_history("main", &mut |_| ControlFlow::Continue(()))
            .unwrap();
        let chain = history.default_chain();
        // What the state shown before held and the move took off the chain.
        let took: Vec<u32> = state
            .landmarks
            .fossils
            .iter()
            .filter(|f| !chain.contains(&f.commit))
            .map(|f| f.id)
            .collect();
        let sinkholes: Vec<u32> = shaken
            .scene
            .ground_marks
            .iter()
            .filter(|m| m.kind == GroundMark::Sinkhole)
            .filter_map(|m| m.fossil)
            .collect();
        assert_eq!(took, ids(&cleared[5..]), "kept: {kept}");
        assert_eq!(sinkholes, took, "kept: {kept}");
        assert_eq!(live.state(), Some(&fx.full_replay()));
    }
}

#[test]
fn after_a_gap_the_strongest_unseen_event_plays_and_stays_until_a_newer_one() {
    let mut fx = Fixture::new(20);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    fx.clock.advance(HOUR);
    let earlier = fx.commit_now();
    fx.report("coverage/lcov.info", LCOV);
    let first = shown(&mut live, Cause::Watch, true);
    assert_eq!(first.scene.event, None);
    // Five hours on: coverage fell (a dusk), and the earlier hour's commit was tagged as a
    // release, which dates the flowering to that commit's hour.
    fx.clock.advance(5 * HOUR);
    fx.commit_now();
    fx.report("coverage/lcov.info", LCOV_LOWER);
    fx.repo.tag_lightweight("v1.0.0", &earlier).unwrap();
    let caught_up = shown(&mut live, Cause::Watch, true);
    let flowering = caught_up.scene.event.expect("the flowering plays");
    assert_eq!(flowering.kind, EventKind::Flowering);
    assert!(flowering.started_tick < live.state().unwrap().now());
    // Nothing new: the scene, still playing it, is not sent again.
    assert!(refresh(&mut live, Cause::Watch, true).snapshot.is_none());
    // An hour on, it still plays, so the scene is unchanged; the composer plays an instance
    // once.
    fx.clock.advance(HOUR);
    assert!(refresh(&mut live, Cause::Hour, false).snapshot.is_none());
    // A newer event replaces it.
    fx.clock.advance(HOUR);
    fx.commit_now();
    fx.report("coverage/lcov.info", LCOV);
    let sunburst = shown(&mut live, Cause::Watch, true);
    assert_eq!(
        sunburst.scene.event.map(|e| e.kind),
        Some(EventKind::Sunburst)
    );
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
    let mut fx = Fixture::new(12);
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
    assert!(snapshot.notes.sim_updated);
    assert_eq!(
        fx.store()
            .sim_cache(id)
            .unwrap()
            .map(|cache: SimCache| cache.sim_version),
        Some(SIM_VERSION)
    );
    // One snapshot carries it, and the next start has nothing to note.
    fx.commit_now();
    assert!(!shown(&mut upgraded, Cause::Watch, true).notes.sim_updated);
    drop(upgraded);
    let mut next = fx.reconnect().unwrap();
    assert!(!shown(&mut next, Cause::Connect, true).notes.sim_updated);
}

#[test]
fn the_sim_note_waits_for_a_snapshot_to_carry_it() {
    let fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    drop(live);
    fx.store()
        .set_install_value(InstallKey::LastSimVersion, &(SIM_VERSION.0 - 1).to_string())
        .unwrap();
    // The repository is away when the upgraded app starts: nothing is shown, nothing noted.
    let away = fx.dir.path().join("away");
    fs::rename(fx.repo_path(), &away).unwrap();
    let mut upgraded = fx.reconnect().unwrap();
    assert_eq!(
        refresh(&mut upgraded, Cause::Connect, true).reach,
        Some(Reach::NotFound)
    );
    drop(upgraded);
    let mut again = fx.reconnect().unwrap();
    assert_eq!(
        refresh(&mut again, Cause::Connect, true).reach,
        Some(Reach::NotFound)
    );
    // Back: the first snapshot carries the note.
    fs::rename(&away, fx.repo_path()).unwrap();
    assert!(shown(&mut again, Cause::Watch, true).notes.sim_updated);
    assert_eq!(
        fx.store()
            .install_value(InstallKey::LastSimVersion)
            .unwrap()
            .as_deref(),
        Some(SIM_VERSION.0.to_string().as_str())
    );
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

// Lifecycle: connect (above), relink, re-seed and disconnect, each as the app runs them with
// the engine stopped.

/// A second repository beside the fixture's, with history of its own.
fn other_repository(fx: &Fixture) -> PathBuf {
    let path = fx.dir.path().join("other");
    let mut other = FixtureRepo::init(&path).unwrap();
    commit(&mut other, "main", 3 * DAY + 7 * HOUR);
    path
}

#[test]
fn a_moved_repository_is_relinked_to_its_terrarium_and_resumes() {
    let fx = Fixture::new(12);
    let mut live = fx.connect();
    fx.report("coverage/lcov.info", LCOV);
    let before = shown(&mut live, Cause::Connect, true);
    let terrarium = live.terrarium().clone();
    let moved = fx.dir.path().join("moved");
    fs::rename(fx.repo_path(), &moved).unwrap();
    assert_eq!(
        refresh(&mut live, Cause::Watch, true).reach,
        Some(Reach::NotFound)
    );
    drop(live);
    let relinked = lifecycle::relink(&fx.data(), terrarium.id, &moved).unwrap();
    assert_eq!(relinked.repo.path, moved.display().to_string());
    assert_eq!(relinked.repo.name, "moved");
    assert_eq!(
        (
            relinked.id,
            relinked.install_salt,
            relinked.seed,
            relinked.active
        ),
        (terrarium.id, terrarium.install_salt, terrarium.seed, true)
    );
    let mut resumed = fx.reconnect().expect("the relinked terrarium");
    let after = shown(&mut resumed, Cause::Connect, true);
    assert_eq!(after.scene, before.scene);
    assert_eq!(fx.rows(ObservationKind::Coverage).len(), 1);
}

#[test]
fn relinking_to_another_repository_or_a_plain_folder_changes_nothing() {
    let fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    let terrarium = live.terrarium().clone();
    drop(live);
    let other = other_repository(&fx);
    assert!(matches!(
        lifecycle::relink(&fx.data(), terrarium.id, &other),
        Err(ConnectError::OtherRepository)
    ));
    let plain = fx.dir.path().join("plain");
    fs::create_dir(&plain).unwrap();
    assert!(matches!(
        lifecycle::relink(&fx.data(), terrarium.id, &plain),
        Err(ConnectError::Git(_))
    ));
    assert_eq!(fx.store().terrarium(terrarium.id).unwrap(), Some(terrarium));
}

#[test]
fn a_reseed_lays_the_terrarium_out_anew_and_keeps_its_truth() {
    let fx = Fixture::new(12);
    let mut live = fx.connect();
    fx.report("coverage/lcov.info", LCOV);
    let before = shown(&mut live, Cause::Connect, true);
    let terrarium = live.terrarium().clone();
    drop(live);
    assert!(fx.store().sim_cache(terrarium.id).unwrap().is_some());
    let salt = lifecycle::reseed(&fx.data(), terrarium.id).unwrap();
    assert_ne!(salt, terrarium.install_salt);
    assert_eq!(fx.store().sim_cache(terrarium.id).unwrap(), None);
    assert_eq!(fx.rows(ObservationKind::Coverage).len(), 1);
    let mut reseeded = fx.reconnect().expect("still connected");
    let Start::Connected {
        terrarium: now,
        scene,
    } = reseeded.start()
    else {
        panic!("connected");
    };
    assert_eq!(now.install_salt, salt);
    assert_eq!(scene, None, "nothing cached to start from");
    let after = shown(&mut reseeded, Cause::Connect, true);
    assert_eq!(after.stats.replay, Replay::Full);
    assert_eq!((after.scene, after.report), (before.scene, before.report));
}

#[test]
fn a_disconnect_keeping_observations_connects_nothing_until_the_repository_comes_back() {
    let fx = Fixture::new(12);
    let mut live = fx.connect();
    fx.report("coverage/lcov.info", LCOV);
    shown(&mut live, Cause::Connect, true);
    let id = live.terrarium().id;
    drop(live);
    lifecycle::disconnect(&fx.data(), id, false).unwrap();
    assert!(fx.reconnect().is_none(), "a plain run connects nothing");
    let mut store = fx.store();
    assert!(!store.terrarium(id).unwrap().unwrap().active);
    // Nor when the setting still names it, as an older install's disconnect left it.
    store.set_setting(SETTING_REPO_ACTIVE, &id.get()).unwrap();
    assert!(fx.reconnect().is_none());
    assert_eq!(store.sim_cache(id).unwrap(), None);
    assert_eq!(store.observations(id, None).unwrap().len(), 1);
    drop(store);
    // Connected again: the same terrarium, its history kept.
    let again = fx.connect();
    assert_eq!(again.terrarium().id, id);
    assert!(again.terrarium().active);
    assert_eq!(fx.rows(ObservationKind::Coverage).len(), 1);
}

#[test]
fn a_disconnect_deleting_observations_leaves_nothing_of_the_terrarium() {
    let fx = Fixture::new(12);
    let mut live = fx.connect();
    fx.report("coverage/lcov.info", LCOV);
    shown(&mut live, Cause::Connect, true);
    let terrarium = live.terrarium().clone();
    drop(live);
    let status = fx.data().local_tests(&terrarium.seed);
    write_still(&status, "{}");
    lifecycle::disconnect(&fx.data(), terrarium.id, true).unwrap();
    let store = fx.store();
    assert_eq!(store.terrarium(terrarium.id).unwrap(), None);
    assert!(store.observations(terrarium.id, None).unwrap().is_empty());
    assert_eq!(store.sim_cache(terrarium.id).unwrap(), None);
    assert_eq!(store.install_value(InstallKey::ShownEvents).unwrap(), None);
    assert_eq!(store.setting::<i64>(SETTING_REPO_ACTIVE).unwrap(), None);
    drop(store);
    assert!(!status.exists(), "its local test runs went with it");
    assert!(fx.reconnect().is_none());
    // Connected again, it starts afresh: a new salt and no observations.
    let mut again = fx.connect();
    assert_ne!(again.terrarium().install_salt, terrarium.install_salt);
    shown(&mut again, Cause::Connect, false);
    assert!(fx.rows(ObservationKind::Coverage).is_empty());
}

#[test]
fn a_restart_within_an_event_s_hour_does_not_play_it_again() {
    let mut fx = Fixture::new(12);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    fx.repo.reset("main", &fx.main[5]).unwrap();
    fx.commit_now();
    let shaken = shown(&mut live, Cause::Watch, true);
    assert_eq!(
        shaken.scene.event.map(|e| e.kind),
        Some(EventKind::Earthquake)
    );
    assert!(shaken.notes.history_rewritten);
    drop(live);
    // Restarted in the same hour: the earthquake was shown, so it does not play again.
    let mut restarted = fx.reconnect().unwrap();
    let again = shown(&mut restarted, Cause::Connect, true);
    assert_eq!(again.scene.event, None);
    assert!(!again.notes.history_rewritten, "noted once");
}

#[test]
fn a_recorded_rewrite_is_noted_by_one_snapshot() {
    let mut fx = Fixture::new(12);
    let mut live = fx.connect();
    assert!(
        !shown(&mut live, Cause::Connect, true)
            .notes
            .history_rewritten
    );
    fx.repo.reset("main", &fx.main[5]).unwrap();
    fx.commit_now();
    assert!(shown(&mut live, Cause::Watch, true).notes.history_rewritten);
    fx.commit_now();
    assert!(!shown(&mut live, Cause::Watch, true).notes.history_rewritten);
}

#[test]
fn the_details_date_the_root_the_stones_the_fossils_and_a_dormancy() {
    // A first commit, another a year and five days on, then a month of quiet.
    let dir = tempfile::tempdir().unwrap();
    let mut repo = FixtureRepo::init(&dir.path().join("repo")).unwrap();
    commit(&mut repo, "main", 0);
    repo.commit(
        "main",
        Stamp::at(FIRST + 370 * DAY),
        "plant",
        &[Edit::write("src/old/roots.rs", &"root\n".repeat(600))],
    )
    .unwrap();
    let cleared = repo
        .commit(
            "main",
            Stamp::at(FIRST + 371 * DAY),
            "clear",
            &[Edit::Remove("src/old/roots.rs".into())],
        )
        .unwrap();
    let main = vec![repo.tip("main")];
    let fx = Fixture {
        dir,
        repo,
        main,
        clock: ManualClock::at(FIRST + 371 * DAY + 2 * HOUR),
    };
    let mut live = fx.connect();
    let first = shown(&mut live, Cause::Connect, true);
    let details = &first.details;
    assert_eq!(details.root, Some(FIRST));
    assert!(details.terrain > 0);
    assert_eq!(details.dormant_since, None);
    let [stone] = details.monuments.as_slice() else {
        panic!("one stone: {:?}", details.monuments)
    };
    // Raised at the first close a full year after the root.
    assert!(
        (FIRST + 365 * DAY..FIRST + 367 * DAY).contains(&stone.at),
        "{stone:?}"
    );
    assert_eq!(first.scene.monuments[0].id, stone.id);
    let [fossil] = details.fossils.as_slice() else {
        panic!("one fossil: {:?}", details.fossils)
    };
    assert_eq!(fossil.path.as_deref(), Some("src/old"));
    assert_eq!(fossil.at, FIRST + 371 * DAY);
    assert_eq!(first.scene.fossils[0].id, fossil.id);
    assert!(details.fossils.iter().all(|f| f.id == fossil_id(&cleared)));
    // Thirty days without a commit, and the terrarium goes dormant.
    fx.clock.advance(45 * DAY);
    let quiet = shown(&mut live, Cause::Hour, false);
    let since = quiet.details.dormant_since.expect("dormant");
    assert!(
        (FIRST + 401 * DAY..FIRST + 416 * DAY).contains(&since),
        "{since}"
    );
    // An hour later nothing it says has moved, so nothing is sent.
    fx.clock.advance(HOUR);
    assert!(refresh(&mut live, Cause::Hour, false).snapshot.is_none());
}

#[test]
fn light_s_files_are_those_of_the_report_it_reads() {
    let mut fx = Fixture::new(3);
    let mut live = fx.connect();
    fx.report("coverage/lcov.info", LCOV);
    let recorded = shown(&mut live, Cause::Connect, true);
    assert!(recorded.details.found.coverage);
    assert!(!recorded.details.found.test_report);
    // Light takes the report at the next close, and its files with it.
    assert_eq!(recorded.details.files.light, []);
    fx.clock.advance(DAY);
    let read = shown(&mut live, Cause::Hour, false);
    let one = vec![terrarium_store::Contributor {
        path: "src/fern_0.rs".into(),
        value: 1,
    }];
    assert_eq!(read.details.files.light, one);
    // A newer report waits for the close that takes it, files and all.
    fx.commit_now();
    fx.report("coverage/lcov.info", LCOV_LOWER);
    let newer = shown(&mut live, Cause::Watch, true);
    assert_eq!(newer.details.files.light, one);
    fx.clock.advance(DAY);
    let taken = shown(&mut live, Cause::Hour, false);
    assert_eq!(taken.details.files.light[0].value, 2);
}

#[test]
fn a_branch_s_own_coverage_is_shown_beside_light_and_only_there() {
    let mut fx = Fixture::new(3);
    let mut live = fx.connect();
    shown(&mut live, Cause::Connect, true);
    fx.repo.branch("feature/moss", "main").unwrap();
    commit(&mut fx.repo, "feature/moss", 3 * DAY);
    fx.repo.set_head("feature/moss").unwrap();
    fx.report("coverage/lcov.info", LCOV);
    let on_branch = shown(&mut live, Cause::Watch, true);
    let reading = on_branch
        .details
        .branch_reading
        .expect("the branch's reading");
    assert_eq!(reading.branch, "feature/moss");
    assert_eq!(reading.value, Some(terrarium_truth::Q16::from_ratio(3, 4)));
    // Never applied to light.
    assert_eq!(
        on_branch.report.climate.light,
        terrarium_truth::Reading::Unknown
    );
    assert_eq!(on_branch.details.files.light, []);
    fx.repo.set_head("main").unwrap();
    let back = shown(&mut live, Cause::Watch, true);
    assert_eq!(back.details.branch_reading, None);
}

#[test]
fn weather_s_files_are_the_failing_tests_of_the_run_in_effect() {
    let mut fx = Fixture::new(3);
    let mut live = fx.connect();
    fx.report(
        "junit.xml",
        r#"<testsuite><testcase name="grows" file="src/fern_0.rs"><failure/></testcase><testcase name="roots" file="src/fern_0.rs"><failure/></testcase><testcase name="leaves" file="src/fern_1.rs"/></testsuite>"#,
    );
    let failing = shown(&mut live, Cause::Connect, true);
    assert!(failing.details.found.test_report);
    assert_eq!(
        failing.details.files.weather,
        [terrarium_store::Contributor {
            path: "src/fern_0.rs".into(),
            value: 2,
        }]
    );
    fx.commit_now();
    fx.report("junit.xml", JUNIT);
    let passing = shown(&mut live, Cause::Watch, true);
    assert_eq!(passing.details.files.weather, []);
}
