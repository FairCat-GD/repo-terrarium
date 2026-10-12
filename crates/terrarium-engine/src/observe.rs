//! Observations: what an adapter read, or a ref rewrite seen live, with where HEAD was and
//! when, as the store records it.
//!
//! Only a known reading becomes an observation. A report that is missing, unreadable, cut short
//! or says nothing is not recorded, so the last reading holds and turns stale once the default
//! tip moves past it, rather than turning unknown (I-3). The same holds for the source metrics:
//! a commit with no function records no complexity, one with no code no TODO density, and a
//! Knip report weighed against no counted export no dead code.
//!
//! CI weather is the exception, by the weather's fallback: GitHub's runs that can no longer be
//! read (the token removed or refused, the repository gone, GitHub out of reach for a while)
//! are recorded as unknown, which passes their source over for the local results, then haze.

use sha2::{Digest, Sha256};
use terrarium_adapters::ci::Verdict;
use terrarium_adapters::discovery::Enabled;
use terrarium_adapters::metrics::{METRICS_VERSION, Measurement};
use terrarium_adapters::report::{self, Format, Reading, Report};
use terrarium_store::raw::{
    CiUnread, ComplexityRaw, CoverageFormat, CoverageRaw, DeadCodeRaw, LeftRaw, RefRewriteRaw,
    Shape as _, TodoDensityRaw, WeatherRaw, WeatherState,
};
use terrarium_store::{Contributor, MAX_RAW_BYTES, ObservationRecord};
use terrarium_truth::domain::{LeftCommit, ObservationHead, WeatherSource};
use terrarium_truth::{CommitId, ObservationKind, Q16};

/// The `adapter_version` of the ref rewrites the live refresh records.
const REF_REWRITE_VERSION: u32 = 3;
/// The `adapter_version` of GitHub Actions weather.
pub const CI_VERSION: u32 = 1;

/// The observation `report` makes when HEAD was at `head` at `recorded_at` (Unix seconds,
/// UTC), or `None` when it is not recorded. A Knip report makes none here: its dead code is
/// weighed against the exports of the source, in [`dead_code`].
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
        contributors: stored(&known.contributors),
    })
}

fn stored(contributors: &[report::Contributor]) -> Vec<Contributor> {
    contributors
        .iter()
        .map(|c| Contributor {
            path: c.path.clone(),
            value: c.value,
        })
        .collect()
}

/// The complexity and TODO density of the source `measurement` read at `head`, which is the
/// default tip, each when its switch is on and it is known. A row's content hash is the
/// SHA-256 of its `raw`.
pub fn metrics(
    measurement: &Measurement,
    enabled: Enabled,
    head: &ObservationHead,
    recorded_at: i64,
) -> Vec<ObservationRecord> {
    let record =
        |kind, raw: String, normalized, confidence, contributors: &[report::Contributor]| {
            ObservationRecord {
                kind,
                adapter_version: METRICS_VERSION,
                recorded_at,
                head: head.clone(),
                source: None,
                report_mtime: None,
                content_hash: Sha256::digest(raw.as_bytes()).into(),
                raw,
                normalized: Some(normalized),
                confidence,
                contributors: stored(contributors),
            }
        };
    let mut records = Vec::new();
    if let Some(reading) = measurement.complexity().filter(|_| enabled.complexity) {
        let sources = reading.sources;
        let raw = ComplexityRaw {
            functions: reading.functions,
            decision_points: reading.decision_points,
            max_cyclomatic: reading.max_cyclomatic,
            max_nesting: reading.max_nesting,
            nesting_total: reading.nesting_total,
            files: sources.files,
            partial: sources.partial,
            unparsed: sources.unparsed,
            oversized: sources.oversized,
        };
        records.push(record(
            ObservationKind::Complexity,
            raw.to_raw(),
            reading.normalized,
            sources.confidence(),
            &reading.contributors,
        ));
    }
    if let Some(reading) = measurement.todo_density().filter(|_| enabled.todo_density) {
        let sources = reading.sources;
        let raw = TodoDensityRaw {
            lines: reading.lines,
            todo: reading.todo,
            fixme: reading.fixme,
            hack: reading.hack,
            files: sources.files,
            partial: sources.partial,
            unparsed: sources.unparsed,
            oversized: sources.oversized,
        };
        records.push(record(
            ObservationKind::TodoDensity,
            raw.to_raw(),
            reading.normalized,
            sources.confidence(),
            &reading.contributors,
        ));
    }
    records
}

/// The dead code a known Knip `report` makes against the exports `measurement` counted at
/// HEAD's commit (`head`), or `None` when it is not recorded. It is a report's reading: its
/// content hash and modification time are the report's.
pub fn dead_code(
    report: &Report,
    measurement: &Measurement,
    head: ObservationHead,
    recorded_at: i64,
) -> Option<ObservationRecord> {
    let known = report.known()?;
    let Reading::DeadCode(counts) = &known.reading else {
        return None;
    };
    let reading = measurement.dead_code(counts)?;
    let raw = DeadCodeRaw {
        unused_items: reading.unused_items,
        exported_items: reading.exported_items,
    };
    Some(ObservationRecord {
        kind: ObservationKind::DeadCode,
        adapter_version: Format::Knip.adapter_version(),
        recorded_at,
        head,
        source: None,
        report_mtime: known.mtime,
        content_hash: known.content_hash,
        raw: raw.to_raw(),
        normalized: Some(reading.normalized),
        confidence: reading.confidence,
        contributors: stored(&reading.contributors),
    })
}

/// The weather `branch`'s runs on GitHub give (source `ci:<branch>`), recorded with where HEAD
/// is, `head`, which may be another branch: truth follows HEAD's branch, so the weather in
/// effect is that branch's runs when it has some, else the default branch's. Its content hash
/// is the verdict's, so the same runs are recorded once.
pub fn ci_weather(
    branch: &str,
    verdict: &Verdict,
    head: ObservationHead,
    recorded_at: i64,
) -> ObservationRecord {
    let raw = WeatherRaw {
        sha: Some(verdict.sha.clone()),
        workflows: Some(verdict.workflows),
        failing_workflows: Some(u64::try_from(verdict.failing.len()).unwrap_or(u64::MAX)),
        ..WeatherRaw::of(if verdict.passed {
            WeatherState::Pass
        } else {
            WeatherState::Fail
        })
    };
    ObservationRecord {
        kind: ObservationKind::Weather,
        adapter_version: CI_VERSION,
        recorded_at,
        head,
        source: Some(WeatherSource::Ci {
            branch: branch.to_owned(),
        }),
        report_mtime: None,
        content_hash: verdict.hash,
        raw: raw.to_raw(),
        normalized: Some(if verdict.passed { Q16::ONE } else { Q16::ZERO }),
        confidence: Q16::ONE,
        contributors: verdict
            .failing
            .iter()
            .map(|path| Contributor {
                path: path.clone(),
                value: 1,
            })
            .collect(),
    }
}

/// `branch`'s CI weather as unknown, and why: its runs can no longer be read here. Its content
/// hash names the reason alone, so it is recorded once until a known one comes between.
pub fn ci_unread(
    branch: &str,
    why: CiUnread,
    head: ObservationHead,
    recorded_at: i64,
) -> ObservationRecord {
    let raw = WeatherRaw {
        unread: Some(why),
        ..WeatherRaw::of(WeatherState::Unknown)
    };
    let raw = raw.to_raw();
    ObservationRecord {
        kind: ObservationKind::Weather,
        adapter_version: CI_VERSION,
        recorded_at,
        head,
        source: Some(WeatherSource::Ci {
            branch: branch.to_owned(),
        }),
        report_mtime: None,
        content_hash: Sha256::digest(raw.as_bytes()).into(),
        raw,
        normalized: None,
        confidence: Q16::ONE,
        contributors: Vec::new(),
    }
}

/// A default branch that moved without fast-forwarding, and what the move took off its
/// first-parent chain, as git read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rewrite {
    pub old: CommitId,
    pub new: CommitId,
    /// The commits that left the chain, oldest first; `None` when they are no longer in the
    /// repository.
    pub left: Option<Vec<LeftCommit>>,
}

/// The observation of `branch` making `rewrite`, seen while HEAD was at `head`: how many
/// commits it took off the chain, and those that deleted whole directories, which truth reads
/// its fossils from. A row that would pass [`MAX_RAW_BYTES`] keeps the newest of them, which
/// are the ones whose fossils showed. Its content hash is the SHA-256 of its `raw`.
pub fn ref_rewrite(
    branch: &str,
    rewrite: Rewrite,
    head: ObservationHead,
    recorded_at: i64,
) -> ObservationRecord {
    let rewritten = rewrite
        .left
        .as_ref()
        .map(|left| u32::try_from(left.len()).unwrap_or(u32::MAX));
    let mut shape = RefRewriteRaw {
        reference: format!("refs/heads/{branch}"),
        old: rewrite.old,
        new: rewrite.new,
        rewritten,
        fossils: Vec::new(),
        left: rewrite
            .left
            .iter()
            .flatten()
            .filter(|commit| !commit.deleted_dirs.is_empty())
            .map(LeftRaw::from)
            .collect(),
    };
    let mut raw = shape.to_raw();
    while raw.len() > MAX_RAW_BYTES && !shape.left.is_empty() {
        // Entries are of a size, so the excess over their mean length names about how many
        // to drop; the loop makes up any shortfall.
        let mean = raw.len() / shape.left.len();
        let over = (raw.len() - MAX_RAW_BYTES).div_ceil(mean.max(1));
        shape.left.drain(..over.clamp(1, shape.left.len()));
        raw = shape.to_raw();
    }
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
/// its format, dead code is Knip's, and of the local weather only a `record-tests` run carries
/// an exit code. `None` for every other kind and source.
pub fn producer(record: &ObservationRecord) -> Option<Format> {
    match (record.kind, &record.source) {
        (ObservationKind::DeadCode, None) => Some(Format::Knip),
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
    use terrarium_adapters::metrics::{Counted, FileFacts, Language, Read};
    use terrarium_adapters::report::Why;
    use terrarium_store::{DataDir, NewTerrarium, RepoLocation, Store};
    use terrarium_truth::RepoSeed;
    use terrarium_truth::domain::{DeletedDir, ObservationValue};

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
        let stored = store.observations(terrarium, None).unwrap();
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
            Rewrite {
                old: CommitId::from_hex(&"a".repeat(40)).unwrap(),
                new: CommitId::from_hex(&"b".repeat(40)).unwrap(),
                left: None,
            },
            head(),
            7,
        );
        assert_eq!(producer(&rewrite), None);
    }

    #[test]
    fn a_ref_rewrite_is_stored_and_replay_reads_its_refs() {
        let old = CommitId::from_hex(&"a".repeat(40)).unwrap();
        let new = CommitId::from_hex(&"b".repeat(40)).unwrap();
        let taken = |n: char, lines: u64| LeftCommit {
            commit: CommitId::from_hex(&n.to_string().repeat(40)).unwrap(),
            time: 1_700_000_000,
            deleted_dirs: (lines > 0)
                .then(|| DeletedDir {
                    path: "src/old".into(),
                    lines,
                })
                .into_iter()
                .collect(),
        };
        let deleting = taken('c', 640);
        let rewrite = Rewrite {
            old: old.clone(),
            new: new.clone(),
            left: Some(vec![taken('e', 0), deleting.clone(), taken('f', 0)]),
        };
        let record = ref_rewrite("trunk", rewrite, head(), 42);
        record.check().unwrap();
        assert_eq!(record.adapter_version, 3);
        // How many left, and those that deleted something.
        assert_eq!(
            record.raw,
            format!(
                r#"{{"ref":"refs/heads/trunk","old":"{}","new":"{}","rewritten":3,"left":[{{"commit":"{}","time":1700000000,"deleted":[{{"path":"src/old","lines":640}}]}}]}}"#,
                old.as_str(),
                new.as_str(),
                "c".repeat(40)
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
        let stored = store.observations(terrarium, None).unwrap();
        assert_eq!(
            stored[0].to_observation().unwrap().value,
            ObservationValue::RefRewrite {
                reference: "refs/heads/trunk".into(),
                old,
                new,
                rewritten: Some(3),
                fossils: Vec::new(),
                left: vec![deleting],
            }
        );
    }

    #[test]
    fn a_rewrite_too_large_for_a_row_keeps_its_newest_deletions() {
        let id = |n: u32| CommitId::from_hex(&format!("{n:040x}")).unwrap();
        let left: Vec<LeftCommit> = (0..2_000)
            .map(|n| LeftCommit {
                commit: id(n),
                time: 1_700_000_000,
                deleted_dirs: vec![DeletedDir {
                    path: format!("src/module{n}"),
                    lines: 600,
                }],
            })
            .collect();
        let record = ref_rewrite(
            "main",
            Rewrite {
                old: id(9_000),
                new: id(9_001),
                left: Some(left),
            },
            head(),
            7,
        );
        record.check().unwrap();
        assert!(record.raw.len() <= MAX_RAW_BYTES);
        let raw: RefRewriteRaw = serde_json::from_str(&record.raw).unwrap();
        assert_eq!(raw.rewritten, Some(2_000));
        assert!(raw.left.len() > 100, "{}", raw.left.len());
        assert_eq!(raw.left.last().map(|l| &l.commit), Some(&id(1_999)));
    }

    fn facts(language: Language, counted: Counted, exports: u64) -> FileFacts {
        FileFacts {
            language,
            read: Read::Clean,
            counted,
            exports,
        }
    }

    #[test]
    fn the_metrics_records_carry_what_they_were_computed_from() {
        let counted = Counted {
            lines: 400,
            functions: 10,
            decision_points: 70,
            max_cyclomatic: 9,
            max_nesting: 3,
            nesting_total: 12,
            todo: 2,
            fixme: 1,
            hack: 1,
        };
        let measurement = Measurement {
            files: [("src/bed.rs".to_owned(), facts(Language::Rust, counted, 0))].into(),
        };
        let records = metrics(&measurement, Enabled::default(), &head(), 50);
        let [complexity, todo] = &records[..] else {
            panic!("{records:?}");
        };
        assert_eq!(complexity.kind, ObservationKind::Complexity);
        assert_eq!(complexity.adapter_version, METRICS_VERSION);
        // A mean of 8: (8 − 1) / 14 = 0.5.
        assert_eq!(complexity.normalized, Some(Q16::HALF));
        assert_eq!(
            complexity.raw,
            r#"{"functions":10,"decision_points":70,"max_cyclomatic":9,"max_nesting":3,"nesting_total":12,"files":1,"partial":0,"unparsed":0,"oversized":0}"#
        );
        assert_eq!(
            complexity.content_hash,
            <[u8; 32]>::from(Sha256::digest(complexity.raw.as_bytes()))
        );
        assert_eq!(
            (complexity.source.clone(), complexity.report_mtime),
            (None, None)
        );
        assert_eq!(complexity.confidence, Q16::ONE);
        assert_eq!(
            complexity.contributors,
            [Contributor {
                path: "src/bed.rs".into(),
                value: 70
            }]
        );
        assert_eq!(todo.kind, ObservationKind::TodoDensity);
        // 4 markers in 400 lines is 10 per 1,000: 0.5.
        assert_eq!(todo.normalized, Some(Q16::HALF));
        assert_eq!(
            todo.raw,
            r#"{"lines":400,"todo":2,"fixme":1,"hack":1,"files":1,"partial":0,"unparsed":0,"oversized":0}"#
        );
        for record in &records {
            record.check().unwrap();
            assert_eq!(producer(record), None);
        }
        let only_todo = Enabled {
            complexity: false,
            ..Enabled::default()
        };
        let kinds: Vec<_> = metrics(&measurement, only_todo, &head(), 50)
            .iter()
            .map(|r| r.kind)
            .collect();
        assert_eq!(kinds, [ObservationKind::TodoDensity]);
        assert!(metrics(&Measurement::default(), Enabled::default(), &head(), 50).is_empty());
    }

    #[test]
    fn dead_code_is_a_knip_reports_reading_against_the_exports() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "knip-report.json",
            r#"{"issues":[{"file":"web/a.ts","exports":[{"name":"x"}]}]}"#,
        );
        let knip = read(dir.path(), Format::Knip, "knip-report.json");
        let measurement = Measurement {
            files: [(
                "web/a.ts".to_owned(),
                facts(Language::TypeScript, Counted::default(), 4),
            )]
            .into(),
        };
        let record = dead_code(&knip, &measurement, head(), 9).expect("recorded");
        record.check().unwrap();
        assert_eq!(record.raw, r#"{"unused_items":1,"exported_items":4}"#);
        assert_eq!(
            record.normalized,
            Some(Q16::ONE.saturating_sub(Q16::from_ratio(1, 4)))
        );
        let known = knip.known().unwrap();
        assert_eq!(record.content_hash, known.content_hash);
        assert_eq!(record.report_mtime, known.mtime);
        assert_eq!(producer(&record), Some(Format::Knip));
        // No export counted: none.
        assert_eq!(dead_code(&knip, &Measurement::default(), head(), 9), None);
        // Not a Knip report: none.
        write(dir.path(), "lcov.info", "SF:a.ts\nDA:1,1\nend_of_record\n");
        let lcov = read(dir.path(), Format::Lcov, "lcov.info");
        assert_eq!(dead_code(&lcov, &measurement, head(), 9), None);
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
