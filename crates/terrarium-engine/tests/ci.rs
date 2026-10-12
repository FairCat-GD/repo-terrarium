//! GitHub Actions weather in the live pipeline: what a poll's answer records, with where HEAD
//! is, and how the weather in effect follows HEAD's branch, falls back to the default branch's
//! runs, then to the local results. The answers are given by hand; the poller and the client
//! are tested on their own, and `n09.rs` runs them in the engine.
#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use terrarium_adapters::binding::{CiRepository, RepoBinding};
use terrarium_adapters::ci::{CiError, Fetch, MemoryKeychain, Slug, Token, Verdict};
use terrarium_adapters::fixture::{Edit, FixtureRepo, Stamp};
use terrarium_adapters::local_tests::Status;
use terrarium_engine::ci::{Answered, Ask, CiConfig, CiStatus, Read};
use terrarium_engine::live::Done;
use terrarium_engine::schedule::{Cause, Reach, Refresh};
use terrarium_engine::{EngineConfig, Live, ManualClock, Snapshot};
use terrarium_store::identifiers::{SETTING_CI_GITHUB_REPO, SETTING_REPO_ACTIVE};
use terrarium_store::raw::{CiUnread, WeatherRaw};
use terrarium_store::{DataDir, Store, StoredObservation, TerrariumId};
use terrarium_truth::domain::WeatherSource;
use terrarium_truth::{CommitId, ObservationKind, Q16, Reading};

/// Monday 2024-01-08, 09:00 UTC.
const FIRST: i64 = 1_704_704_400;
const DAY: i64 = 86_400;
const DAYS: i64 = 20;

/// A client no test here asks: the pipeline is given answers by hand.
struct Unasked;

impl Fetch for Unasked {
    fn runs(&mut self, _: &Slug, _: &str, _: &Token) -> Result<Option<Verdict>, CiError> {
        unreachable!("the pipeline never asks GitHub itself")
    }

    fn forget(&mut self) {}
}

struct Fixture {
    dir: tempfile::TempDir,
    repo: FixtureRepo,
    clock: ManualClock,
}

impl Fixture {
    /// A repository on GitHub (by its `origin`), a commit a day on `main`.
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = FixtureRepo::init(&dir.path().join("repo")).unwrap();
        for day in 0..DAYS {
            repo.commit(
                "main",
                Stamp::at(FIRST + day * DAY),
                "tend",
                &[Edit::write(&format!("src/fern_{day}.rs"), "fn fern() {}\n")],
            )
            .unwrap();
        }
        repo.append_git_config(
            "[remote \"origin\"]\n\turl = git@github.com:example-garden/moss-bed.git\n",
        )
        .unwrap();
        Fixture {
            dir,
            repo,
            clock: ManualClock::at(FIRST + DAYS * DAY + 2 * 3_600),
        }
    }

    fn connect(&self) -> Live {
        let config = EngineConfig {
            clock: Arc::new(self.clock.clone()),
            ci: Some(CiConfig {
                keychain: Arc::new(MemoryKeychain::holding("github_pat_fictional")),
                fetch: Arc::new(|| Box::new(Unasked)),
                every: Duration::from_secs(60),
            }),
            ..EngineConfig::new(
                DataDir::at(self.dir.path().join("data")),
                Some(self.dir.path().join("repo")),
            )
        };
        let mut live = Live::connect(&config).unwrap().expect("connected");
        shown(&mut live, Cause::Connect);
        live
    }

    /// A branch from `main`'s tip with a commit of its own, checked out.
    fn checkout_new(&mut self, branch: &str, day: i64) -> CommitId {
        self.repo.branch(branch, "main").unwrap();
        let commit = self
            .repo
            .commit(
                branch,
                Stamp::at(FIRST + day * DAY + 3_600),
                "try",
                &[Edit::write(&format!("src/{branch}.rs"), "fn try_it() {}\n")],
            )
            .unwrap();
        self.repo.set_head(branch).unwrap();
        commit
    }

    fn weather_rows(&self) -> Vec<StoredObservation> {
        let store = Store::open(&DataDir::at(self.dir.path().join("data"))).unwrap();
        let id = store.setting::<i64>(SETTING_REPO_ACTIVE).unwrap().unwrap();
        store
            .observations(TerrariumId::new(id), Some(ObservationKind::Weather))
            .unwrap()
    }

    fn ci_rows(&self) -> Vec<StoredObservation> {
        self.weather_rows()
            .into_iter()
            .filter(|row| matches!(row.record.source, Some(WeatherSource::Ci { .. })))
            .collect()
    }

    fn status_file(&self, exit_code: i64) -> PathBuf {
        let data = DataDir::at(self.dir.path().join("data"));
        let store = Store::open(&data).unwrap();
        let id = store.setting::<i64>(SETTING_REPO_ACTIVE).unwrap().unwrap();
        let seed = store.terrarium(TerrariumId::new(id)).unwrap().unwrap().seed;
        let path = data.local_tests(&seed);
        Status::new(exit_code, FIRST, FIRST + 60)
            .write(&path)
            .unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(5);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        path
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

fn shown(live: &mut Live, cause: Cause) -> Option<Snapshot> {
    let done = refresh(live, cause);
    assert_eq!(done.reach, Some(Reach::Found), "{done:?}");
    assert_eq!(done.problem, None);
    done.snapshot
}

fn slug() -> Slug {
    Slug::parse("example-garden/moss-bed").unwrap()
}

fn verdict(passed: bool, mark: u8) -> Verdict {
    Verdict {
        passed,
        sha: format!("{mark:02x}").repeat(20),
        workflows: 2,
        failing: if passed {
            Vec::new()
        } else {
            vec![".github/workflows/ci.yml".to_owned()]
        },
        hash: [mark; 32],
    }
}

fn decided(branch: &str, verdict: Verdict) -> Answered {
    Answered {
        slug: slug(),
        read: Read::Decided(Some((branch.to_owned(), verdict))),
    }
}

/// Gives `live` an answer; when it has something to record, the refresh that records it runs,
/// and its snapshot comes back.
fn answer(live: &mut Live, answered: Answered, outage: bool) -> Option<Snapshot> {
    if !live.ci_answered(answered, outage) {
        return None;
    }
    shown(live, Cause::Job)
}

/// The weather in effect: its source, and whether it passes (`None` when unknown).
fn in_effect(snapshot: &Snapshot) -> (Option<WeatherSource>, Option<bool>) {
    let weather = &snapshot.report.climate.weather;
    let pass = match weather.state {
        Reading::Known { value, .. } => Some(value >= Q16::HALF),
        Reading::Unknown => None,
    };
    (weather.source.clone(), pass)
}

/// The source `ci:<branch>`.
fn ci(branch: &str) -> WeatherSource {
    WeatherSource::Ci {
        branch: branch.to_owned(),
    }
}

#[test]
fn a_poll_asks_heads_branch_then_the_default_one() {
    let mut fx = Fixture::new();
    let mut live = fx.connect();
    assert_eq!(
        live.ci_ask(),
        Some(Ask {
            slug: slug(),
            branches: vec!["main".into()]
        })
    );
    fx.checkout_new("feat/moss", DAYS);
    shown(&mut live, Cause::Watch);
    assert!(live.ci_ask_now(), "a checkout polls at once");
    assert!(!live.ci_ask_now());
    assert_eq!(
        live.ci_ask().unwrap().branches,
        ["feat/moss".to_owned(), "main".to_owned()]
    );
}

#[test]
fn runs_are_recorded_with_heads_head_once_each() {
    let fx = Fixture::new();
    let mut live = fx.connect();
    let snapshot = answer(&mut live, decided("main", verdict(true, 1)), false).unwrap();
    assert_eq!(in_effect(&snapshot), (Some(ci("main")), Some(true)));
    let rows = fx.ci_rows();
    assert_eq!(rows.len(), 1);
    let row = &rows[0].record;
    assert_eq!(row.source, Some(ci("main")));
    assert_eq!(row.head.branch.as_deref(), Some("main"));
    assert!(row.head.at_default_tip);
    assert_eq!(row.report_mtime, None);
    assert_eq!(row.normalized, Some(Q16::ONE));
    // The same runs again record nothing.
    assert!(!live.ci_answered(decided("main", verdict(true, 1)), false));
    let snapshot = answer(&mut live, decided("main", verdict(false, 2)), false).unwrap();
    assert_eq!(in_effect(&snapshot), (Some(ci("main")), Some(false)));
    let rows = fx.ci_rows();
    assert_eq!(rows.len(), 2);
    let raw: WeatherRaw = serde_json::from_str(&rows[1].record.raw).unwrap();
    assert_eq!(raw.failing_workflows, Some(1));
    assert_eq!(
        rows[1].record.contributors[0].path,
        ".github/workflows/ci.yml"
    );
    assert_eq!(live.tools().ci, CiStatus::Reading(slug()));
}

#[test]
fn a_checkout_back_to_the_default_branch_is_recorded_though_its_runs_are_unchanged() {
    let mut fx = Fixture::new();
    let mut live = fx.connect();
    answer(&mut live, decided("main", verdict(true, 1)), false).unwrap();
    fx.checkout_new("feat/moss", DAYS);
    shown(&mut live, Cause::Watch);
    let snapshot = answer(&mut live, decided("feat/moss", verdict(false, 2)), false).unwrap();
    assert_eq!(in_effect(&snapshot), (Some(ci("feat/moss")), Some(false)));
    // Back on main, whose runs did not change (a 304 gives the verdict kept): truth learns
    // where HEAD is only from a row, so one is recorded.
    fx.repo.set_head("main").unwrap();
    shown(&mut live, Cause::Watch);
    let snapshot = answer(&mut live, decided("main", verdict(true, 1)), false)
        .expect("recorded though unchanged");
    assert_eq!(in_effect(&snapshot), (Some(ci("main")), Some(true)));
    let last = fx.ci_rows().pop().unwrap();
    assert_eq!(last.record.head.branch.as_deref(), Some("main"));
    // And not again.
    assert!(!live.ci_answered(decided("main", verdict(true, 1)), false));
}

#[test]
fn a_branch_without_runs_takes_the_default_branchs_weather() {
    let mut fx = Fixture::new();
    let mut live = fx.connect();
    answer(&mut live, decided("main", verdict(false, 1)), false).unwrap();
    fx.checkout_new("feat/fern", DAYS);
    shown(&mut live, Cause::Watch);
    // The branch's own runs decide nothing: the default branch's answer, recorded from here.
    let snapshot = answer(&mut live, decided("main", verdict(false, 1)), false).unwrap();
    assert_eq!(in_effect(&snapshot), (Some(ci("main")), Some(false)));
    assert_eq!(
        fx.ci_rows().pop().unwrap().record.head.branch.as_deref(),
        Some("feat/fern")
    );
    // Neither decides on the next branch: an unknown row says where HEAD is.
    fx.checkout_new("feat/reed", DAYS);
    shown(&mut live, Cause::Watch);
    let nothing = Answered {
        slug: slug(),
        read: Read::Decided(None),
    };
    let snapshot = answer(&mut live, nothing.clone(), false).unwrap();
    assert_eq!(in_effect(&snapshot), (Some(ci("main")), Some(false)));
    let row = fx.ci_rows().pop().unwrap().record;
    assert_eq!((row.source, row.normalized), (Some(ci("feat/reed")), None));
    let raw: WeatherRaw = serde_json::from_str(&row.raw).unwrap();
    assert_eq!(raw.unread, Some(CiUnread::NoRun));
    assert!(!live.ci_answered(nothing, false));
    // An answer for a branch HEAD has left is not recorded.
    assert!(!live.ci_answered(decided("feat/fern", verdict(true, 3)), false));
}

#[test]
fn ci_that_cannot_be_read_any_more_gives_way_to_the_local_results() {
    let fx = Fixture::new();
    let mut live = fx.connect();
    fx.status_file(0);
    shown(&mut live, Cause::Request);
    let snapshot = answer(&mut live, decided("main", verdict(false, 1)), false).unwrap();
    assert_eq!(in_effect(&snapshot), (Some(ci("main")), Some(false)));
    // A token never saved, or a failure GitHub may get over, records nothing.
    let quiet = |read| Answered { slug: slug(), read };
    assert!(!live.ci_answered(quiet(Read::NoToken { removed: false }), false));
    assert!(!live.ci_answered(quiet(Read::Failed(CiError::Unreachable)), false));
    // Out of reach long enough: unknown, and the local run takes effect.
    let snapshot = answer(&mut live, quiet(Read::Failed(CiError::Unreachable)), true).unwrap();
    assert_eq!(
        in_effect(&snapshot),
        (Some(WeatherSource::Local), Some(true))
    );
    let raw: WeatherRaw = serde_json::from_str(&fx.ci_rows().pop().unwrap().record.raw).unwrap();
    assert_eq!(raw.unread, Some(CiUnread::Unreachable));
    assert!(!live.ci_answered(quiet(Read::Failed(CiError::Unreachable)), true));
    // Back, then the token removed: unknown again, at once.
    answer(&mut live, decided("main", verdict(false, 1)), false).unwrap();
    let snapshot = answer(&mut live, quiet(Read::NoToken { removed: true }), false).unwrap();
    assert_eq!(
        in_effect(&snapshot),
        (Some(WeatherSource::Local), Some(true))
    );
    for (read, why) in [
        (Read::Failed(CiError::Refused), CiUnread::Refused),
        (Read::Failed(CiError::NotFound), CiUnread::NotFound),
    ] {
        answer(&mut live, decided("main", verdict(false, 1)), false).unwrap();
        let snapshot = answer(&mut live, quiet(read), false).unwrap();
        assert_eq!(
            in_effect(&snapshot),
            (Some(WeatherSource::Local), Some(true))
        );
        let raw: WeatherRaw =
            serde_json::from_str(&fx.ci_rows().pop().unwrap().record.raw).unwrap();
        assert_eq!(raw.unread, Some(why));
    }
}

#[test]
fn weather_of_one_source_in_one_second_applies_in_the_order_recorded() {
    let mut fx = Fixture::new();
    let mut live = fx.connect();
    answer(&mut live, decided("main", verdict(true, 1)), false).unwrap();
    // The clock does not move: the next row of `ci:main`, at another head, is in the same
    // second, and goes a second later.
    fx.checkout_new("feat/moss", DAYS);
    shown(&mut live, Cause::Watch);
    let snapshot = answer(&mut live, decided("main", verdict(false, 2)), false).unwrap();
    assert_eq!(in_effect(&snapshot), (Some(ci("main")), Some(false)));
    let rows = fx.ci_rows();
    assert_eq!(rows[1].record.recorded_at, rows[0].record.recorded_at + 1);
}

#[test]
fn an_answer_for_another_repository_is_not_recorded() {
    let fx = Fixture::new();
    let mut live = fx.connect();
    let elsewhere = Answered {
        slug: Slug::parse("example-garden/other").unwrap(),
        read: Read::Decided(Some(("main".into(), verdict(true, 1)))),
    };
    assert!(!live.ci_answered(elsewhere, false));
    assert!(fx.ci_rows().is_empty());
}

#[test]
fn a_repository_typed_for_another_folder_is_not_read() {
    let fx = Fixture::new();
    let mut live = fx.connect();
    let typed = |binding: RepoBinding| {
        let mut store = Store::open(&DataDir::at(fx.dir.path().join("data"))).unwrap();
        let typed = CiRepository {
            binding,
            repo: Slug::parse("example-garden/upstream").unwrap(),
        };
        store.set_setting(SETTING_CI_GITHUB_REPO, &typed).unwrap();
    };
    let here = {
        let terrarium = live.terrarium();
        RepoBinding::new(&terrarium.seed, &terrarium.repo.path)
    };
    typed(RepoBinding {
        path: format!("{}-fork", here.path),
        ..here.clone()
    });
    shown(&mut live, Cause::Request);
    assert_eq!(
        live.ci_ask().unwrap().slug,
        slug(),
        "origin's, not the fork's"
    );
    typed(here);
    shown(&mut live, Cause::Request);
    assert_eq!(
        live.ci_ask().unwrap().slug,
        Slug::parse("example-garden/upstream").unwrap()
    );
}
