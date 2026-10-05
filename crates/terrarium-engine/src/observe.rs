//! Observations: what an adapter read, or a ref rewrite seen live, with where HEAD was and
//! when, as the store records it.
//!
//! Only a known reading becomes an observation. A report that is missing, unreadable, cut short
//! or says nothing is not recorded, so the last reading holds and turns stale once the default
//! tip moves past it, rather than turning unknown (I-3). Dead code is not recorded yet: its
//! formula needs the number of exported items, which a Knip report does not hold and the
//! tree-sitter reader will count.

use sha2::{Digest, Sha256};
use terrarium_adapters::report::{Format, Reading, Report};
use terrarium_store::raw::{
    CoverageFormat, CoverageRaw, RefRewriteRaw, Shape as _, WeatherRaw, WeatherState,
};
use terrarium_store::{Contributor, ObservationRecord};
use terrarium_truth::domain::{ObservationHead, WeatherSource};
use terrarium_truth::{CommitId, ObservationKind, Q16};

/// The `adapter_version` of the ref rewrites the live refresh records.
pub const REF_REWRITE_VERSION: u32 = 1;

/// The observation `report` makes when HEAD was at `head` at `recorded_at` (Unix seconds,
/// UTC), or `None` when it is not recorded.
pub fn record(
    report: &Report,
    head: ObservationHead,
    recorded_at: i64,
) -> Option<ObservationRecord> {
    let known = report.known()?;
    let normalized = known.reading.normalized()?;
    let (source, raw) = match (&known.reading, report.format) {
        (
            Reading::Coverage {
                lines_found,
                lines_hit,
            },
            Format::Lcov | Format::Cobertura,
        ) => {
            let format = if report.format == Format::Lcov {
                CoverageFormat::Lcov
            } else {
                CoverageFormat::Cobertura
            };
            let raw = CoverageRaw {
                format,
                lines_found: *lines_found,
                lines_hit: *lines_hit,
            };
            (None, raw.to_raw())
        }
        (
            Reading::Tests {
                tests,
                failed,
                skipped,
                suite_failed,
            },
            Format::JUnit,
        ) => {
            let raw = WeatherRaw {
                tests: Some(*tests),
                failed: Some(*failed),
                skipped: Some(*skipped),
                suite_failed: suite_failed.then_some(true),
                ..WeatherRaw::of(state(&known.reading))
            };
            (Some(WeatherSource::Local), raw.to_raw())
        }
        (Reading::LocalRun { exit_code }, Format::LocalTests) => {
            let raw = WeatherRaw {
                exit_code: Some(*exit_code),
                ..WeatherRaw::of(state(&known.reading))
            };
            (Some(WeatherSource::Local), raw.to_raw())
        }
        _ => return None,
    };
    Some(ObservationRecord {
        kind: report.format.kind(),
        adapter_version: report.format.adapter_version(),
        recorded_at,
        head,
        source,
        report_mtime: known.mtime,
        content_hash: known.content_hash,
        raw,
        normalized: Some(normalized),
        // Everything recorded was read in full.
        confidence: Q16::ONE,
        contributors: known
            .contributors
            .iter()
            .map(|c| Contributor {
                path: c.path.clone(),
                value: c.value,
            })
            .collect(),
    })
}

/// The observation of `branch` moving from `old` to `new` without fast-forwarding, seen while
/// HEAD was at `head`. Its content hash is the SHA-256 of its `raw`.
pub fn ref_rewrite(
    branch: &str,
    old: CommitId,
    new: CommitId,
    head: ObservationHead,
    recorded_at: i64,
) -> ObservationRecord {
    let raw = RefRewriteRaw {
        reference: format!("refs/heads/{branch}"),
        old,
        new,
    }
    .to_raw();
    ObservationRecord {
        kind: ObservationKind::RefRewrite,
        adapter_version: REF_REWRITE_VERSION,
        recorded_at,
        head,
        source: None,
        report_mtime: None,
        content_hash: Sha256::digest(raw.as_bytes()).into(),
        raw,
        normalized: None,
        confidence: Q16::ONE,
        contributors: Vec::new(),
    }
}

/// Which report format wrote a stored observation, read back from its `raw`: coverage names
/// its format, and of the local weather only a `record-tests` run carries an exit code. `None`
/// for every other kind and source.
pub fn producer(record: &ObservationRecord) -> Option<Format> {
    match (record.kind, &record.source) {
        (ObservationKind::Coverage, None) => {
            let raw: CoverageRaw = serde_json::from_str(&record.raw).ok()?;
            Some(match raw.format {
                CoverageFormat::Lcov => Format::Lcov,
                CoverageFormat::Cobertura => Format::Cobertura,
            })
        }
        (ObservationKind::Weather, Some(WeatherSource::Local)) => {
            let raw: WeatherRaw = serde_json::from_str(&record.raw).ok()?;
            Some(if raw.exit_code.is_some() {
                Format::LocalTests
            } else {
                Format::JUnit
            })
        }
        _ => None,
    }
}

fn state(reading: &Reading) -> WeatherState {
    match reading.passed() {
        Some(true) => WeatherState::Pass,
        Some(false) => WeatherState::Fail,
        None => WeatherState::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use terrarium_adapters::local_tests::Status;
    use terrarium_adapters::report::Why;
    use terrarium_store::{DataDir, NewTerrarium, RepoLocation, Store};
    use terrarium_truth::RepoSeed;
    use terrarium_truth::domain::ObservationValue;

    use super::*;

    fn head() -> ObservationHead {
        ObservationHead {
            commit: CommitId::from_hex(&"c".repeat(40)).unwrap(),
            branch: Some("main".into()),
            at_default_tip: true,
        }
    }

    fn write(root: &Path, path: &str, text: &str) {
        let full = root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }

    fn read(root: &Path, format: Format, path: &str) -> Report {
        Report::read(root, format, &[path.to_owned()])
    }

    /// A report of every recorded format, read from files in a fresh root, and its record.
    fn every_format() -> (tempfile::TempDir, Vec<Report>, Vec<ObservationRecord>) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "lcov.info",
            "SF:src/a.ts\nDA:1,1\nDA:2,0\nend_of_record\n",
        );
        write(
            root,
            "coverage.xml",
            r#"<coverage><packages><package><classes><class filename="src/a.ts"><lines><line number="1" hits="0"/></lines></class></classes></package></packages></coverage>"#,
        );
        write(
            root,
            "junit.xml",
            r#"<testsuite><testcase name="a" file="src/a.test.ts"><failure/></testcase><testcase name="b"/><testcase name="c"><skipped/></testcase></testsuite>"#,
        );
        let status = root.join("local-tests/seed.json");
        Status::new(0, 10, 20).write(&status).unwrap();
        let reports = [
            read(root, Format::Lcov, "lcov.info"),
            read(root, Format::Cobertura, "coverage.xml"),
            read(root, Format::JUnit, "junit.xml"),
            terrarium_adapters::local_tests::read(&status),
        ];
        let records: Vec<ObservationRecord> = reports
            .iter()
            .zip(100..)
            .map(|(report, at)| record(report, head(), at).expect("recorded"))
            .collect();
        (dir, reports.to_vec(), records)
    }

    #[test]
    fn every_recorded_format_carries_its_reading() {
        let (_dir, reports, records) = every_format();

        let coverage = &records[0];
        assert_eq!(coverage.kind, ObservationKind::Coverage);
        assert_eq!(coverage.normalized, Some(Q16::HALF));
        assert_eq!(
            coverage.raw,
            r#"{"format":"lcov","lines_found":2,"lines_hit":1}"#
        );
        assert_eq!(coverage.source, None);
        assert_eq!(coverage.confidence, Q16::ONE);
        assert_eq!(
            coverage.contributors,
            [Contributor {
                path: "src/a.ts".into(),
                value: 1
            }]
        );
        assert_eq!(
            coverage.content_hash,
            reports[0].known().unwrap().content_hash
        );
        assert!(coverage.report_mtime.is_some());
        assert_eq!(records[1].normalized, Some(Q16::ZERO));
        assert!(records[1].raw.starts_with(r#"{"format":"cobertura""#));
        let junit = &records[2];
        assert_eq!(junit.kind, ObservationKind::Weather);
        assert_eq!(junit.source, Some(WeatherSource::Local));
        assert_eq!(junit.normalized, Some(Q16::ZERO));
        assert_eq!(
            junit.raw,
            r#"{"state":"fail","tests":3,"failed":1,"skipped":1}"#
        );
        let run = &records[3];
        assert_eq!(run.normalized, Some(Q16::ONE));
        assert_eq!(run.raw, r#"{"state":"pass","exit_code":0}"#);
    }

    #[test]
    fn every_recorded_format_is_stored_and_replay_reads_it() {
        let (_dir, _, records) = every_format();
        let data = tempfile::tempdir().unwrap();
        let mut store = Store::open(&DataDir::at(data.path())).unwrap();
        let terrarium = store
            .create_terrarium(&NewTerrarium {
                seed: RepoSeed::from_bytes([7; 32]),
                repo: RepoLocation {
                    path: "/moss".into(),
                    name: "moss".into(),
                    default_branch: "main".into(),
                },
                install_salt: [0; 16],
                connected_at: 0,
            })
            .unwrap();
        for r in &records {
            r.check().unwrap();
            store.append_observation(terrarium, r).unwrap();
        }
        let stored = store.observations(terrarium, None, None).unwrap();
        let values: Vec<ObservationValue> = stored
            .iter()
            .map(|s| s.to_observation().unwrap().value)
            .collect();
        assert_eq!(
            values,
            [
                ObservationValue::Coverage,
                ObservationValue::Coverage,
                ObservationValue::Weather {
                    source: WeatherSource::Local
                },
                ObservationValue::Weather {
                    source: WeatherSource::Local
                },
            ]
        );
        assert_eq!(
            stored.iter().map(|s| &s.record).collect::<Vec<_>>(),
            records.iter().collect::<Vec<_>>()
        );
    }

    #[test]
    fn each_record_names_the_format_that_wrote_it() {
        let (_dir, reports, records) = every_format();
        for (report, record) in reports.iter().zip(&records) {
            assert_eq!(producer(record), Some(report.format));
        }
        let rewrite = ref_rewrite(
            "main",
            CommitId::from_hex(&"a".repeat(40)).unwrap(),
            CommitId::from_hex(&"b".repeat(40)).unwrap(),
            head(),
            7,
        );
        assert_eq!(producer(&rewrite), None);
    }

    #[test]
    fn a_ref_rewrite_is_stored_and_replay_reads_its_refs() {
        let old = CommitId::from_hex(&"a".repeat(40)).unwrap();
        let new = CommitId::from_hex(&"b".repeat(40)).unwrap();
        let record = ref_rewrite("trunk", old.clone(), new.clone(), head(), 42);
        record.check().unwrap();
        assert_eq!(
            record.raw,
            format!(
                r#"{{"ref":"refs/heads/trunk","old":"{}","new":"{}"}}"#,
                old.as_str(),
                new.as_str()
            )
        );
        assert_eq!(
            record.content_hash,
            <[u8; 32]>::from(Sha256::digest(record.raw.as_bytes()))
        );
        let data = tempfile::tempdir().unwrap();
        let mut store = Store::open(&DataDir::at(data.path())).unwrap();
        let terrarium = store
            .create_terrarium(&NewTerrarium {
                seed: RepoSeed::from_bytes([7; 32]),
                repo: RepoLocation {
                    path: "/moss".into(),
                    name: "moss".into(),
                    default_branch: "trunk".into(),
                },
                install_salt: [0; 16],
                connected_at: 0,
            })
            .unwrap();
        store.append_observation(terrarium, &record).unwrap();
        let stored = store.observations(terrarium, None, None).unwrap();
        assert_eq!(
            stored[0].to_observation().unwrap().value,
            ObservationValue::RefRewrite {
                reference: "refs/heads/trunk".into(),
                old,
                new
            }
        );
    }

    #[test]
    fn an_unknown_report_or_dead_code_is_not_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let missing = read(root, Format::Lcov, "lcov.info");
        assert!(matches!(
            missing.outcome,
            terrarium_adapters::report::Outcome::Unknown(Why::Missing)
        ));
        assert_eq!(record(&missing, head(), 1), None);
        write(root, "cut.info", "SF:src/a.ts\nDA:1,1\n");
        assert_eq!(
            record(&read(root, Format::Lcov, "cut.info"), head(), 1),
            None
        );
        write(root, "junit.xml", "<testsuite/>");
        assert_eq!(
            record(&read(root, Format::JUnit, "junit.xml"), head(), 1),
            None
        );
        write(
            root,
            "knip-report.json",
            r#"{"issues":[{"file":"a.ts","exports":[{"name":"x"}]}]}"#,
        );
        let knip = read(root, Format::Knip, "knip-report.json");
        assert!(knip.known().is_some());
        assert_eq!(record(&knip, head(), 1), None);
    }
}
