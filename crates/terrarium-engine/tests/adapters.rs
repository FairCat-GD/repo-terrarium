//! Every report adapter, from a file in a repository's worktree through `observe::record` (or,
//! for Knip, `observe::dead_code` against the source's exports), the store and the headless
//! replay (I-3): a missing report is unknown, never zero; a reading the default tip has moved
//! past is held and marked stale, never lowered, even when its file then stops reading; a
//! feature branch's reading is stored but never applied.
#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};

use terrarium_adapters::fixture::{Edit, FixtureRepo, Stamp};
use terrarium_adapters::git::GitRepo;
use terrarium_adapters::local_tests::{self, Status};
use terrarium_adapters::metrics::Metrics;
use terrarium_adapters::report::{Format, Report};
use terrarium_engine::headless::{Request, replay_dump};
use terrarium_engine::observe::{dead_code, record};
use terrarium_store::{DataDir, NewTerrarium, ObservationRecord, RepoLocation, Store};
use terrarium_truth::domain::{ObservationHead, WeatherSource};
use terrarium_truth::{CommitId, Q16, Reading, ReplayDump};

/// Monday 2024-01-08, 09:00 UTC.
const FIRST: i64 = 1_704_704_400;
const DAY: i64 = 86_400;
const HOUR: i64 = 3_600;

/// A repository with a commit on `main` every day from day 0 to day 12 (day 0's with a
/// TypeScript module), and a feature branch that leaves `main` at day 10.
struct Fixture {
    dir: tempfile::TempDir,
    main: Vec<CommitId>,
    feature: CommitId,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = FixtureRepo::init(&dir.path().join("repo")).unwrap();
        let mut main = Vec::new();
        let mut feature = None;
        for day in 0..=12 {
            let mut edits = vec![Edit::write(&format!("src/fern_{day}.rs"), "fn fern() {}\n")];
            if day == 0 {
                edits.push(Edit::write(
                    "web/garden.ts",
                    "export const sow = 1;\nexport function weed() {}\nexport class Bed {}\n",
                ));
            }
            let id = repo
                .commit("main", Stamp::at(FIRST + day * DAY), "tend", &edits)
                .unwrap();
            main.push(id);
            if day == 10 {
                repo.branch("feature/moss", "main").unwrap();
                feature = Some(
                    repo.commit(
                        "feature/moss",
                        Stamp::at(FIRST + day * DAY + HOUR),
                        "moss",
                        &[Edit::write("src/moss.rs", "fn moss() {}\n")],
                    )
                    .unwrap(),
                );
            }
        }
        Fixture {
            dir,
            main,
            feature: feature.unwrap(),
        }
    }

    fn repo(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    fn write(&self, path: &str, text: &str) {
        let full = self.repo().join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }

    /// HEAD on `main` at day `day`'s commit, its tip then.
    fn on_main(&self, day: usize) -> ObservationHead {
        ObservationHead {
            commit: self.main[day].clone(),
            branch: Some("main".into()),
            at_default_tip: true,
        }
    }

    fn on_feature(&self) -> ObservationHead {
        ObservationHead {
            commit: self.feature.clone(),
            branch: Some("feature/moss".into()),
            at_default_tip: false,
        }
    }

    /// The replay through `at` with `records`, exported and imported as a person moving
    /// machines would carry them.
    fn replay(&self, records: &[ObservationRecord], at: i64) -> ReplayDump {
        let repo = self.repo();
        let git = GitRepo::open(&repo).unwrap();
        let seed = git.identity("main").unwrap().seed;
        let data = tempfile::tempdir().unwrap();
        let mut store = Store::open(&DataDir::at(data.path().join("export"))).unwrap();
        let terrarium = store
            .create_terrarium(&NewTerrarium {
                seed,
                repo: RepoLocation {
                    path: repo.display().to_string(),
                    name: "repo".into(),
                    default_branch: "main".into(),
                },
                install_salt: [0; 16],
                connected_at: FIRST,
            })
            .unwrap();
        for r in records {
            store.append_observation(terrarium, r).unwrap();
        }
        let mut export = Vec::new();
        store.export_observations(terrarium, &mut export).unwrap();
        replay_dump(Request {
            repo: &repo,
            data: &DataDir::at(data.path().join("replay")),
            observations: Some(&mut export.as_slice()),
            at_secs: at,
            install_salt: [0; 16],
        })
        .unwrap()
    }
}

/// A report written into the worktree, as a tool leaves it.
struct Adapter {
    format: Format,
    path: &'static str,
    good: &'static str,
    /// The same report caught half-written.
    cut: &'static str,
}

const COVERAGE: [Adapter; 2] = [
    Adapter {
        format: Format::Lcov,
        path: "coverage/lcov.info",
        good: "SF:src/fern_0.rs\nDA:1,1\nDA:2,1\nDA:3,0\nDA:4,1\nend_of_record\n",
        cut: "SF:src/fern_0.rs\nDA:1,1\nDA:2,",
    },
    Adapter {
        format: Format::Cobertura,
        path: "coverage.xml",
        good: r#"<coverage><packages><package><classes><class filename="src/fern_0.rs"><lines><line number="1" hits="1"/><line number="2" hits="1"/><line number="3" hits="0"/><line number="4" hits="1"/></lines></class></classes></package></packages></coverage>"#,
        cut: r#"<coverage><packages><package><classes><class filename="src/fern_0.rs"><lines><line number="1" hits="1"/>"#,
    },
];

fn read(fx: &Fixture, adapter: &Adapter) -> Report {
    Report::read(&fx.repo(), adapter.format, &[adapter.path.to_owned()])
}

#[test]
fn coverage_unknown_is_neutral_and_stale_is_held() {
    let fx = Fixture::new();
    for adapter in &COVERAGE {
        // No report: nothing is recorded, light is unknown, and health is neutral, not zero.
        assert_eq!(record(&read(&fx, adapter), fx.on_main(12), FIRST), None);
        let bare = fx.replay(&[], FIRST + 14 * DAY);
        assert_eq!(bare.report.climate.light, Reading::Unknown);
        assert_eq!(bare.report.health, Q16::HALF);

        // A report at day 10's tip: light reads it after the next midnight.
        fx.write(adapter.path, adapter.good);
        let at = FIRST + 10 * DAY + 2 * HOUR;
        let good = record(&read(&fx, adapter), fx.on_main(10), at).expect("recorded");
        assert_eq!(good.normalized, Some(Q16::from_ratio(3, 4)));
        let fresh = fx.replay(std::slice::from_ref(&good), FIRST + 10 * DAY + 20 * HOUR);
        let Reading::Known { value, stale, .. } = fresh.report.climate.light else {
            panic!("{:?}: light known", adapter.format);
        };
        assert_eq!(value, Q16::from_ratio(3, 4));
        assert!(!stale);

        // The tip moves on and the file is caught half-written: nothing is recorded, and the
        // reading is held, stale, never lower.
        fx.write(adapter.path, adapter.cut);
        assert_eq!(
            record(&read(&fx, adapter), fx.on_main(12), FIRST + 12 * DAY + HOUR),
            None
        );
        let later = fx.replay(std::slice::from_ref(&good), FIRST + 14 * DAY);
        assert_eq!(
            later.report.climate.light,
            Reading::Known {
                value,
                stale: true,
                since_tick: match fresh.report.climate.light {
                    Reading::Known { since_tick, .. } => since_tick,
                    Reading::Unknown => unreachable!(),
                },
            },
            "{:?}",
            adapter.format
        );

        // On a feature branch the same report is stored but never applied.
        fx.write(adapter.path, adapter.good);
        let feature = record(&read(&fx, adapter), fx.on_feature(), at).expect("recorded");
        assert!(!feature.head.at_default_tip);
        let applied = fx.replay(&[feature], FIRST + 14 * DAY);
        assert_eq!(applied.report.climate.light, Reading::Unknown);
        fs::remove_file(fx.repo().join(adapter.path)).unwrap();
    }
}

#[test]
fn local_weather_from_junit_or_record_tests_holds_through_an_unreadable_report() {
    let fx = Fixture::new();
    let bare = fx.replay(&[], FIRST + 12 * DAY);
    assert_eq!(bare.report.climate.weather.state, Reading::Unknown);
    assert_eq!(bare.report.climate.weather.source, None);

    let status = fx.dir.path().join("data/local-tests/seed.json");
    let readers: [(&str, Reader<'_>); 2] = [
        (
            "junit",
            Box::new(|text: &str| {
                fx.write("junit.xml", text);
                Report::read(&fx.repo(), Format::JUnit, &["junit.xml".to_owned()])
            }),
        ),
        (
            "record-tests",
            Box::new(|text: &str| {
                fs::create_dir_all(status.parent().unwrap()).unwrap();
                fs::write(&status, text).unwrap();
                local_tests::read(&status)
            }),
        ),
    ];
    let pass_status = status_text(0);
    let fail_status = status_text(1);
    for (name, read) in &readers {
        let (pass, fail, cut) = if *name == "junit" {
            (
                r#"<testsuite><testcase name="grows"/></testsuite>"#,
                r#"<testsuite><testcase name="grows"><failure/></testcase></testsuite>"#,
                r#"<testsuite><testcase name="grows"/>"#,
            )
        } else {
            (pass_status.as_str(), fail_status.as_str(), "{\"format\":")
        };
        let passed = record(&read(pass), fx.on_main(10), FIRST + 10 * DAY + 2 * HOUR).unwrap();
        assert_eq!(passed.source, Some(WeatherSource::Local), "{name}");
        let failed = record(&read(fail), fx.on_main(11), FIRST + 11 * DAY + 2 * HOUR).unwrap();
        // Caught half-written: nothing recorded, so the last run's weather stands.
        assert_eq!(
            record(&read(cut), fx.on_main(12), FIRST + 12 * DAY + 2 * HOUR),
            None
        );

        let after_pass = fx.replay(std::slice::from_ref(&passed), FIRST + 12 * DAY + 3 * HOUR);
        assert_eq!(
            after_pass.report.climate.weather.state.known_value(),
            Some(Q16::ONE),
            "{name}"
        );
        assert_eq!(
            after_pass.report.climate.weather.source,
            Some(WeatherSource::Local)
        );
        let after_fail = fx.replay(&[passed, failed], FIRST + 12 * DAY + 3 * HOUR);
        assert_eq!(
            after_fail.report.climate.weather.state.known_value(),
            Some(Q16::ZERO),
            "{name}"
        );
    }
}

/// Writes a report's text where its adapter reads it, and reads it.
type Reader<'a> = Box<dyn Fn(&str) -> Report + 'a>;

fn status_text(exit_code: i64) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.json");
    Status::new(exit_code, 1, 2).write(&path).unwrap();
    fs::read_to_string(path).unwrap()
}

#[test]
fn a_knip_report_weighed_against_the_sources_exports_is_soils_dead_code() {
    let fx = Fixture::new();
    fx.write(
        "knip-report.json",
        r#"{"issues":[{"file":"web/garden.ts","exports":[{"name":"weed"}]}]}"#,
    );
    let report = Report::read(&fx.repo(), Format::Knip, &["knip-report.json".to_owned()]);
    assert!(report.known().is_some());
    assert_eq!(
        record(&report, fx.on_main(12), FIRST + 12 * DAY),
        None,
        "a report alone holds no export count"
    );
    let git = GitRepo::open(&fx.repo()).unwrap();
    let measurement = Metrics::new()
        .measure(&git, &fx.main[12], &mut || false)
        .unwrap();
    let observed = dead_code(
        &report,
        &measurement,
        fx.on_main(12),
        FIRST + 12 * DAY + HOUR,
    )
    .expect("one unused export of three");
    assert_eq!(observed.raw, r#"{"unused_items":1,"exported_items":3}"#);
    let dump = fx.replay(std::slice::from_ref(&observed), FIRST + 12 * DAY + 2 * HOUR);
    assert!(
        matches!(
            dump.report.causes.dead_code,
            Reading::Known { value, stale: false, .. } if Some(value) == observed.normalized
        ),
        "{:?}",
        dump.report.causes
    );
    // Without it, soil has nothing to go on.
    let none = fx.replay(&[], FIRST + 12 * DAY + 2 * HOUR);
    assert_eq!(none.report.climate.soil, Reading::Unknown);
}

#[test]
fn head_is_read_from_the_repository() {
    let fx = Fixture::new();
    let head = GitRepo::open(Path::new(&fx.repo()))
        .unwrap()
        .head("main")
        .unwrap();
    assert_eq!(head, fx.on_main(12));
}
