//! The parsers on fictional reports from `tests/fixtures/reports/`, and on malformed input.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use proptest::prelude::*;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use terrarium_truth::Q16;

use super::*;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/reports")
        .join(name)
}

/// A repository root holding the fixtures at the given paths, `@ROOT@` replaced by the root,
/// as a tool that wrote absolute paths would have written them.
fn stage(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().display().to_string();
    for (name, dest) in files {
        let text = fs::read_to_string(fixture(name))
            .unwrap()
            .replace("@ROOT@", &root);
        let full = dir.path().join(dest);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }
    dir
}

fn read(dir: &TempDir, format: Format, files: &[&str]) -> Report {
    let files: Vec<String> = files.iter().map(|f| (*f).to_owned()).collect();
    Report::read(dir.path(), format, &files)
}

fn known(report: &Report) -> &Known {
    report
        .known()
        .unwrap_or_else(|| panic!("known: {:?}", report.outcome))
}

fn why(report: &Report) -> &Why {
    match &report.outcome {
        Outcome::Unknown(why) => why,
        Outcome::Known(known) => panic!("unknown expected: {known:?}"),
    }
}

fn contributors(pairs: &[(&str, i64)]) -> Vec<Contributor> {
    pairs
        .iter()
        .map(|(path, value)| Contributor {
            path: (*path).to_owned(),
            value: *value,
        })
        .collect()
}

fn coverage(found: u64, hit: u64) -> Reading {
    Reading::Coverage {
        lines_found: found,
        lines_hit: hit,
    }
}

/// One in-memory file through a parser, as [`Report::read`] would parse it.
fn parse_bytes(format: Format, bytes: &[u8]) -> Outcome {
    let file = Loaded {
        path: "report".into(),
        bytes: bytes.to_vec(),
        mtime: None,
    };
    finish(
        Path::new("/nowhere"),
        format,
        vec!["report".into()],
        Ok(vec![file]),
    )
    .outcome
}

// lcov

#[test]
fn lcov_counts_lines_from_da_records() {
    let dir = stage(&[("lcov/jest.info", "coverage/lcov.info")]);
    let report = read(&dir, Format::Lcov, &["coverage/lcov.info"]);
    let known = known(&report);
    assert_eq!(known.reading, coverage(13, 8));
    assert_eq!(known.reading.normalized(), Some(Q16::from_ratio(8, 13)));
    assert_eq!(
        known.contributors,
        contributors(&[("src/reed.ts", 3), ("src/fern.ts", 2)])
    );
    let bytes = fs::read(dir.path().join("coverage/lcov.info")).unwrap();
    assert_eq!(known.content_hash, <[u8; 32]>::from(Sha256::digest(&bytes)));
    assert!(known.mtime.is_some());
}

#[test]
fn lcov_paths_become_repository_relative_and_outside_ones_only_count() {
    let dir = stage(&[("lcov/llvm-cov.info", "lcov.info")]);
    let report = read(&dir, Format::Lcov, &["lcov.info"]);
    let known = known(&report);
    // 4 + 2 lines in the repository, 2 in the toolchain.
    assert_eq!(known.reading, coverage(8, 5));
    assert_eq!(
        known.contributors,
        contributors(&[("src/lib.rs", 1), ("src/soil/clay.rs", 1)])
    );
}

#[test]
fn a_package_s_relative_paths_are_read_from_the_package() {
    // A monorepo package's tool ran in `packages/fern` and wrote paths relative to it.
    let dir = stage(&[("lcov/jest.info", "packages/fern/coverage/lcov.info")]);
    for file in ["src/fern.ts", "src/reed.ts"] {
        let full = dir.path().join("packages/fern").join(file);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, "").unwrap();
    }
    let report = read(&dir, Format::Lcov, &["packages/fern/coverage/lcov.info"]);
    assert_eq!(
        known(&report).contributors,
        contributors(&[
            ("packages/fern/src/reed.ts", 3),
            ("packages/fern/src/fern.ts", 2)
        ])
    );
}

#[test]
fn lcov_tracefiles_merge_as_lcov_merges_them() {
    let dir = stage(&[
        ("lcov/unit.info", "out/unit.info"),
        ("lcov/integration.info", "out/integration.info"),
    ]);
    let report = read(
        &dir,
        Format::Lcov,
        &["out/unit.info", "out/integration.info"],
    );
    let known = known(&report);
    // fern: lines 1..=4, hit 1 and 2; reed: 1 of 1; thistle: its summary, 6 of 10.
    assert_eq!(known.reading, coverage(15, 9));
    assert_eq!(
        known.contributors,
        contributors(&[("src/thistle.ts", 4), ("src/fern.ts", 2)])
    );
    assert_eq!(report.files, ["out/unit.info", "out/integration.info"]);
    // Several files hash with their paths and lengths, in path order whatever the order given.
    let swapped = read(
        &dir,
        Format::Lcov,
        &["out/integration.info", "out/unit.info"],
    );
    assert_eq!(known.content_hash, swapped.known().unwrap().content_hash);
    let one = read(&dir, Format::Lcov, &["out/unit.info"]);
    assert_ne!(known.content_hash, one.known().unwrap().content_hash);
}

#[test]
fn lcov_must_end_outside_a_record() {
    let dir = stage(&[
        ("lcov/truncated.info", "lcov.info"),
        ("lcov/no-final-newline.info", "coverage/lcov.info"),
    ]);
    assert_eq!(
        why(&read(&dir, Format::Lcov, &["lcov.info"])),
        &Why::Malformed {
            path: "lcov.info".into(),
            line: Some(5)
        }
    );
    assert_eq!(
        known(&read(&dir, Format::Lcov, &["coverage/lcov.info"])).reading,
        coverage(1, 1)
    );
}

#[test]
fn lcov_refuses_what_is_not_lcov() {
    for (text, line) in [
        ("DA:1,1\nend_of_record\n", 1),
        ("SF:a.ts\nSF:b.ts\nend_of_record\n", 2),
        ("SF:a.ts\nDA:x,1\nend_of_record\n", 2),
        ("SF:a.ts\nDA:1\nend_of_record\n", 2),
        ("SF:a.ts\nDA:1,1,sum,extra\nend_of_record\n", 2),
        ("SF:a.ts\nend_of_record\nend_of_record\n", 3),
        ("<coverage/>\n", 1),
        ("SF:\nend_of_record\n", 1),
    ] {
        assert_eq!(
            parse_bytes(Format::Lcov, text.as_bytes()),
            Outcome::Unknown(Why::Malformed {
                path: "report".into(),
                line: Some(line)
            }),
            "{text:?}"
        );
    }
    assert_eq!(parse_bytes(Format::Lcov, b""), Outcome::Unknown(Why::Empty));
    assert_eq!(
        parse_bytes(Format::Lcov, b"TN:\nSF:a.ts\nend_of_record\n"),
        Outcome::Unknown(Why::Empty)
    );
    // CRLF, blank lines, negative counts (not executable) and unknown records are fine.
    let Outcome::Known(known) = parse_bytes(
        Format::Lcov,
        b"TN:\r\n\r\nSF:a.ts\r\nVER:1\r\nDA:1,-1\r\nDA:2,7\r\nMCDC:1,2,t,0,1,x\r\nend_of_record\r\n",
    ) else {
        panic!("known");
    };
    assert_eq!(known.reading, coverage(2, 1));
}

// Cobertura

#[test]
fn cobertura_counts_class_lines_once() {
    let dir = stage(&[("cobertura/istanbul.xml", "coverage/cobertura-coverage.xml")]);
    let report = read(
        &dir,
        Format::Cobertura,
        &["coverage/cobertura-coverage.xml"],
    );
    let known = known(&report);
    // The method's copy of line 1 is not another line; its DOCTYPE is never read.
    assert_eq!(known.reading, coverage(9, 6));
    assert_eq!(
        known.contributors,
        contributors(&[("src/fern.ts", 2), ("src/reed.ts", 1)])
    );
}

#[test]
fn cobertura_file_names_read_against_their_source() {
    let dir = stage(&[("cobertura/coverage-py.xml", "coverage.xml")]);
    let report = read(&dir, Format::Cobertura, &["coverage.xml"]);
    let known = known(&report);
    assert_eq!(known.reading, coverage(6, 4));
    assert_eq!(known.contributors, contributors(&[("moss/soil.py", 2)]));
}

#[test]
fn cobertura_uses_root_totals_only_without_lines() {
    let summary =
        br#"<coverage lines-valid="40" lines-covered="30" line-rate="0.75"><packages/></coverage>"#;
    let Outcome::Known(known) = parse_bytes(Format::Cobertura, summary) else {
        panic!("known");
    };
    assert_eq!(known.reading, coverage(40, 30));
    assert_eq!(
        parse_bytes(
            Format::Cobertura,
            br#"<coverage line-rate="1"><packages/></coverage>"#
        ),
        Outcome::Unknown(Why::Empty)
    );
}

#[test]
fn cobertura_refuses_what_is_not_cobertura() {
    for text in [
        "<report name=\"jacoco\"/>",
        "<coverage><packages><package><classes><class filename=\"a.ts\"><lines><line number=\"1\"/></lines></class></classes></package></packages></coverage>",
        "<coverage>",
        "<coverage/><coverage/>",
        "text<coverage/>",
        "<coverage>&custom;</coverage>",
        "",
    ] {
        assert!(
            matches!(
                parse_bytes(Format::Cobertura, text.as_bytes()),
                Outcome::Unknown(Why::Malformed { .. })
            ),
            "{text:?}"
        );
    }
}

#[test]
fn an_entity_bomb_is_never_expanded() {
    // Declared entities are never read, so referring to one is malformed, and quickly.
    let bomb = r#"<?xml version="1.0"?>
<!DOCTYPE coverage [
  <!ENTITY a "aaaaaaaaaa">
  <!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">
  <!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;">
  <!ENTITY d "&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;">
]>
<coverage><sources><source>&d;</source></sources></coverage>"#;
    assert!(matches!(
        parse_bytes(Format::Cobertura, bomb.as_bytes()),
        Outcome::Unknown(Why::Malformed { .. })
    ));
    let external = r#"<!DOCTYPE coverage [<!ENTITY x SYSTEM "file:///etc/hosts">]><coverage lines-valid="1" lines-covered="1">&x;</coverage>"#;
    assert!(matches!(
        parse_bytes(Format::Cobertura, external.as_bytes()),
        Outcome::Unknown(Why::Malformed { .. })
    ));
}

// JUnit

#[test]
fn surefire_files_merge_and_flaky_runs_pass() {
    let dir = stage(&[
        (
            "junit/surefire/TEST-moss.FernTest.xml",
            "target/surefire-reports/TEST-moss.FernTest.xml",
        ),
        (
            "junit/surefire/TEST-moss.ReedTest.xml",
            "target/surefire-reports/TEST-moss.ReedTest.xml",
        ),
    ]);
    let report = read(
        &dir,
        Format::JUnit,
        &[
            "target/surefire-reports/TEST-moss.FernTest.xml",
            "target/surefire-reports/TEST-moss.ReedTest.xml",
        ],
    );
    let known = known(&report);
    assert_eq!(
        known.reading,
        Reading::Tests {
            tests: 5,
            failed: 1,
            skipped: 1,
            suite_failed: false
        }
    );
    assert_eq!(known.reading.normalized(), Some(Q16::ZERO));
    assert_eq!(known.contributors, contributors(&[("moss.ReedTest", 1)]));
    let fern = read(
        &dir,
        Format::JUnit,
        &["target/surefire-reports/TEST-moss.FernTest.xml"],
    );
    assert_eq!(
        known_passed(&fern),
        Some(true),
        "a flaky failure is not one"
    );
}

fn known_passed(report: &Report) -> Option<bool> {
    known(report).reading.passed()
}

#[test]
fn jest_junit_pytest_and_vitest_read_alike() {
    let dir = stage(&[
        ("junit/jest-junit.xml", "junit.xml"),
        ("junit/pytest.xml", "reports/junit.xml"),
        ("junit/vitest.xml", ".vitest/junit/output.xml"),
    ]);
    let jest = read(&dir, Format::JUnit, &["junit.xml"]);
    assert_eq!(
        known(&jest).reading,
        Reading::Tests {
            tests: 4,
            failed: 1,
            skipped: 1,
            suite_failed: false
        }
    );
    assert_eq!(
        known(&jest).contributors,
        contributors(&[("src/fern.test.ts", 1)])
    );
    let pytest = read(&dir, Format::JUnit, &["reports/junit.xml"]);
    assert_eq!(
        known(&pytest).reading,
        Reading::Tests {
            tests: 4,
            failed: 1,
            skipped: 1,
            suite_failed: false
        },
        "a setup error fails, an xfail is skipped"
    );
    assert_eq!(
        known(&pytest).contributors,
        contributors(&[("tests.test_soil", 1)])
    );
    let vitest = read(&dir, Format::JUnit, &[".vitest/junit/output.xml"]);
    assert_eq!(known_passed(&vitest), Some(true));
    assert_eq!(known(&vitest).reading.normalized(), Some(Q16::ONE));
    assert!(known(&vitest).contributors.is_empty());
}

#[test]
fn a_run_where_nothing_ran_says_nothing() {
    let dir = stage(&[("junit/all-skipped.xml", "junit.xml")]);
    assert_eq!(why(&read(&dir, Format::JUnit, &["junit.xml"])), &Why::Empty);
    assert_eq!(
        parse_bytes(Format::JUnit, b"<testsuites/>"),
        Outcome::Unknown(Why::Empty)
    );
}

#[test]
fn a_suite_that_failed_before_its_cases_ran_fails() {
    for text in [
        r#"<testsuites><testsuite name="fern" tests="0" failures="0" errors="1"/></testsuites>"#,
        r#"<testsuites><testsuite name="fern"><error message="Cannot find module"/></testsuite></testsuites>"#,
        r#"<testsuites tests="0" failures="0" errors="2"></testsuites>"#,
    ] {
        let Outcome::Known(known) = parse_bytes(Format::JUnit, text.as_bytes()) else {
            panic!("known: {text}");
        };
        assert_eq!(known.reading.passed(), Some(false), "{text}");
        assert_eq!(known.reading.normalized(), Some(Q16::ZERO), "{text}");
        let Reading::Tests {
            failed,
            suite_failed,
            ..
        } = known.reading
        else {
            panic!("tests");
        };
        assert_eq!(
            (failed, suite_failed),
            (0, true),
            "no test case is made up: {text}"
        );
    }
}

#[test]
fn junit_refuses_what_is_not_junit() {
    for text in [
        "<coverage/>",
        "<testsuite><testcase>",
        "{\"tests\":1}",
        "<testsuite><testcase name=\"a\"></testsuite>",
    ] {
        assert!(
            matches!(
                parse_bytes(Format::JUnit, text.as_bytes()),
                Outcome::Unknown(Why::Malformed { .. })
            ),
            "{text:?}"
        );
    }
}

#[test]
fn a_suite_that_announces_only_its_cases_failures_is_no_suite_failure() {
    let text = r#"<testsuites failures="1" errors="1"><testsuite failures="1" errors="1"><testcase name="a"><failure/></testcase><testcase name="b"><error/></testcase><testcase name="c"/></testsuite></testsuites>"#;
    let Outcome::Known(known) = parse_bytes(Format::JUnit, text.as_bytes()) else {
        panic!("known");
    };
    assert_eq!(
        known.reading,
        Reading::Tests {
            tests: 3,
            failed: 2,
            skipped: 0,
            suite_failed: false
        }
    );
}

// Encodings

#[test]
fn reports_with_a_byte_order_mark_or_in_utf_16_read() {
    let lcov = "SF:src/a.ts\nDA:1,1\nDA:2,0\nend_of_record\n";
    let with_bom = [&[0xef, 0xbb, 0xbf][..], lcov.as_bytes()].concat();
    assert!(matches!(
        parse_bytes(Format::Lcov, &with_bom),
        Outcome::Known(_)
    ));
    // What Windows PowerShell 5's `>` writes: UTF-16, little-endian, with its mark.
    let utf16 = |text: &str, big_endian: bool| -> Vec<u8> {
        let mut bytes = if big_endian {
            vec![0xfe, 0xff]
        } else {
            vec![0xff, 0xfe]
        };
        for unit in text.encode_utf16() {
            bytes.extend(if big_endian {
                unit.to_be_bytes()
            } else {
                unit.to_le_bytes()
            });
        }
        bytes
    };
    let knip = r#"{"issues":[{"file":"src/a.ts","exports":[{"name":"x"}]}]}"#;
    for big_endian in [false, true] {
        let Outcome::Known(known) = parse_bytes(Format::Knip, &utf16(knip, big_endian)) else {
            panic!("known");
        };
        let Reading::DeadCode(counts) = known.reading else {
            panic!("dead code");
        };
        assert_eq!(counts.unused_exports, 1);
    }
    let junit = "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\r\n<testsuite><testcase name=\"a\"/></testsuite>\r\n";
    let Outcome::Known(known) = parse_bytes(Format::JUnit, &utf16(junit, false)) else {
        panic!("known");
    };
    assert_eq!(known.reading.passed(), Some(true));
    let Outcome::Known(known) = parse_bytes(Format::Lcov, &utf16(lcov, false)) else {
        panic!("known");
    };
    assert_eq!(known.reading, coverage(2, 1));
    // An odd byte, or a lone surrogate, is not UTF-16.
    let mut odd = utf16(lcov, false);
    odd.push(b'\n');
    assert!(matches!(
        parse_bytes(Format::Lcov, &odd),
        Outcome::Unknown(Why::Malformed { line: None, .. })
    ));
    assert!(matches!(
        parse_bytes(Format::Lcov, &[0xff, 0xfe, 0x00, 0xd8]),
        Outcome::Unknown(Why::Malformed { line: None, .. })
    ));
}

// Knip

#[test]
fn knip_6_counts_what_is_unused() {
    let dir = stage(&[("knip/v6.json", "knip-report.json")]);
    let report = read(&dir, Format::Knip, &["knip-report.json"]);
    let known = known(&report);
    assert_eq!(
        known.reading,
        Reading::DeadCode(KnipCounts {
            unused_exports: 3,
            unused_exports_per_file: [("src/math.ts".to_owned(), 3)].into(),
            unused_members: 1,
            unused_files: vec!["src/legacy.ts".into()],
            unused_dependencies: 2,
            version: None,
        })
    );
    assert_eq!(
        known.reading.normalized(),
        None,
        "dead code is weighed against the exports the source metrics count"
    );
    assert_eq!(
        known.contributors,
        contributors(&[("src/math.ts", 4), ("src/legacy.ts", 1)])
    );
    assert_eq!(
        parse_bytes(Format::Knip, br#"{"issues":[]}"#),
        Outcome::Known(Known {
            reading: Reading::DeadCode(KnipCounts::default()),
            contributors: Vec::new(),
            content_hash: Sha256::digest(br#"{"issues":[]}"#).into(),
            mtime: None,
        }),
        "a clean report is a reading"
    );
}

#[test]
fn knip_before_6_is_refused() {
    let dir = stage(&[("knip/v5.json", "knip-report.json")]);
    assert_eq!(
        why(&read(&dir, Format::Knip, &["knip-report.json"])),
        &Why::KnipBefore6 { version: None }
    );
    // A report in the 6 shape from an installed Knip 5 is refused too.
    let dir = stage(&[("knip/v6.json", "knip-report.json")]);
    let manifest = dir.path().join("node_modules/knip/package.json");
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(&manifest, r#"{"name":"knip","version":"5.88.0"}"#).unwrap();
    assert_eq!(
        why(&read(&dir, Format::Knip, &["knip-report.json"])),
        &Why::KnipBefore6 {
            version: Some("5.88.0".into())
        }
    );
    fs::write(&manifest, r#"{"name":"knip","version":"6.39.0"}"#).unwrap();
    let report = read(&dir, Format::Knip, &["knip-report.json"]);
    let Reading::DeadCode(counts) = &known(&report).reading else {
        panic!("dead code");
    };
    assert_eq!(counts.version.as_deref(), Some("6.39.0"));
    for v5 in [
        // Knip 5 kept unused files at the root; a 6 reader would see a clean report.
        r#"{"files":["src/legacy.ts"],"issues":[]}"#,
        r#"{"issues":[{"file":"a.ts","enumMembers":{"E":[{"name":"x"}]}}]}"#,
        r#"{"issues":[{"file":"a.ts","classMembers":[]}]}"#,
    ] {
        assert_eq!(
            parse_bytes(Format::Knip, v5.as_bytes()),
            Outcome::Unknown(Why::KnipBefore6 { version: None }),
            "{v5}"
        );
    }
}

#[test]
fn knip_refuses_what_is_not_a_knip_report() {
    for text in [
        "",
        "[]",
        "{}",
        r#"{"issues":{}}"#,
        r#"{"issues":[{"exports":[]}]}"#,
        r#"{"issues":[{"file":"a.ts","exports":{}}]}"#,
        r#"{"issues":[{"file":"a.ts","files":[{"name":3}]}]}"#,
        r#"{"issues":[]"#,
    ] {
        assert!(
            matches!(
                parse_bytes(Format::Knip, text.as_bytes()),
                Outcome::Unknown(Why::Malformed { .. })
            ),
            "{text:?}"
        );
    }
}

// record-tests

#[test]
fn record_tests_status_files_read_as_runs() {
    let read = |name: &str| crate::local_tests::read(&fixture(name));
    let pass = read("local-tests/pass.json");
    assert_eq!(pass.files, ["local-tests/pass.json"]);
    assert_eq!(known(&pass).reading, Reading::LocalRun { exit_code: 0 });
    assert_eq!(known(&pass).reading.normalized(), Some(Q16::ONE));
    let fail = read("local-tests/fail.json");
    assert_eq!(known(&fail).reading, Reading::LocalRun { exit_code: 101 });
    assert_eq!(known(&fail).reading.normalized(), Some(Q16::ZERO));
    assert!(matches!(
        why(&read("local-tests/wrong-format.json")),
        Why::Malformed { .. }
    ));
}

// Reading the files

#[test]
fn only_regular_files_inside_the_repository_are_read() {
    let dir = stage(&[("lcov/jest.info", "coverage/lcov.info")]);
    assert_eq!(why(&read(&dir, Format::Lcov, &[])), &Why::Missing);
    assert_eq!(
        why(&read(&dir, Format::Lcov, &["lcov.info"])),
        &Why::Missing
    );
    assert_eq!(
        why(&read(&dir, Format::Lcov, &["coverage"])),
        &Why::NotAFile("coverage".into())
    );
    for outside in ["../lcov.info", "/etc/hosts"] {
        assert_eq!(
            why(&read(&dir, Format::Lcov, &[outside])),
            &Why::OutsideRepository(outside.into())
        );
    }
    let big = fs::File::create(dir.path().join("big.xml")).unwrap();
    big.set_len(Format::JUnit.max_file_bytes() + 1).unwrap();
    assert_eq!(
        why(&read(&dir, Format::JUnit, &["big.xml"])),
        &Why::TooLarge("big.xml".into())
    );
    assert!(matches!(
        parse_bytes(Format::Lcov, &[b'S', b'F', 0xff, b'\n']),
        Outcome::Unknown(Why::Malformed { line: Some(1), .. })
    ));
}

#[test]
fn a_report_that_hashes_as_the_one_seen_is_not_read_again() {
    let dir = stage(&[("lcov/jest.info", "coverage/lcov.info")]);
    let files = vec!["coverage/lcov.info".to_owned()];
    let read = Report::read(dir.path(), Format::Lcov, &files);
    let seen = read.known().expect("the fixture parses").content_hash;
    assert_eq!(
        Report::read_unless(dir.path(), Format::Lcov, &files, Some(&seen)),
        None
    );
    // Another hash, or none, reads it as `read` does.
    assert_eq!(
        Report::read_unless(dir.path(), Format::Lcov, &files, Some(&[0; 32])).as_ref(),
        Some(&read)
    );
    assert_eq!(
        Report::read_unless(dir.path(), Format::Lcov, &files, None).as_ref(),
        Some(&read)
    );
    // Changed by a byte, it is read again.
    let path = dir.path().join("coverage/lcov.info");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push('\n');
    fs::write(&path, text).unwrap();
    let changed = Report::read_unless(dir.path(), Format::Lcov, &files, Some(&seen));
    assert!(changed.is_some_and(|report| report.known().is_some_and(|k| k.content_hash != seen)));
}

#[cfg(unix)]
#[test]
fn a_link_to_a_pipe_is_never_opened() {
    let dir = tempfile::tempdir().unwrap();
    // A link a repository could commit, to something that is not a regular file.
    std::os::unix::fs::symlink("/dev/zero", dir.path().join("lcov.info")).unwrap();
    assert_eq!(
        Report::read(dir.path(), Format::Lcov, &["lcov.info".into()]).outcome,
        Outcome::Unknown(Why::NotAFile("lcov.info".into()))
    );
}

/// A repository at `dir/repo` committing `links` (path, target) on `main`, its index written,
/// each link made on disk too, with a coverage report at `dir/elsewhere/lcov.info` and one at
/// `repo/build/lcov.info`.
#[cfg(unix)]
fn linked(dir: &TempDir, links: &[(&str, &str)]) -> std::path::PathBuf {
    use crate::fixture::{Edit, FixtureRepo, Stamp};
    let repo_dir = dir.path().join("repo");
    let mut repo = FixtureRepo::init(&repo_dir).unwrap();
    let edits: Vec<Edit> = links
        .iter()
        .map(|(path, target)| Edit::Symlink((*path).to_owned(), (*target).to_owned()))
        .chain([Edit::write("README", "moss\n")])
        .collect();
    repo.commit("main", Stamp::at(1_704_704_400), "links", &edits)
        .unwrap();
    repo.write_index("main").unwrap();
    let report = fs::read_to_string(fixture("lcov/jest.info")).unwrap();
    for place in [dir.path().join("elsewhere"), repo_dir.join("build")] {
        fs::create_dir_all(&place).unwrap();
        fs::write(place.join("lcov.info"), &report).unwrap();
    }
    for (path, target) in links {
        let full = repo_dir.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(target, full).unwrap();
    }
    repo_dir
}

#[cfg(unix)]
#[test]
fn a_committed_link_leading_outside_is_refused_and_named() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = dir.path().join("elsewhere");
    let outside = elsewhere.join("lcov.info").display().to_string();
    let inside = "build/lcov.info";
    let repo = linked(
        &dir,
        &[
            ("coverage/lcov.info", &outside),
            ("reports", &elsewhere.display().to_string()),
            ("lcov.info", inside),
        ],
    );
    let read = |path: &str| Report::read(&repo, Format::Lcov, &[path.to_owned()]).outcome;
    assert_eq!(
        read("coverage/lcov.info"),
        Outcome::Unknown(Why::CommittedLink("coverage/lcov.info".into()))
    );
    // A committed directory link on the way.
    assert_eq!(
        read("reports/lcov.info"),
        Outcome::Unknown(Why::CommittedLink("reports".into()))
    );
    // A committed link that stays inside reads.
    assert!(matches!(read("lcov.info"), Outcome::Known(_)));
    // Where the file system ignores case, the index is asked in any case.
    if cfg!(target_os = "macos") {
        assert_eq!(
            read("Coverage/lcov.info"),
            Outcome::Unknown(Why::CommittedLink("Coverage/lcov.info".into()))
        );
    }
}

#[cfg(unix)]
#[test]
fn a_link_the_index_lets_go_of_reads_again() {
    use crate::fixture::{Edit, FixtureRepo, Stamp};
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = dir.path().join("elsewhere").display().to_string();
    let root = linked(&dir, &[("reports", &elsewhere)]);
    let read = || Report::read(&root, Format::Lcov, &["reports/lcov.info".to_owned()]).outcome;
    assert_eq!(
        read(),
        Outcome::Unknown(Why::CommittedLink("reports".into()))
    );
    // Untracked (the index written from a tree without it), the same link is a tool's.
    let mut repo = FixtureRepo::init(&dir.path().join("plain")).unwrap();
    repo.commit(
        "main",
        Stamp::at(1_704_704_400),
        "plain",
        &[Edit::write("README", "moss\n")],
    )
    .unwrap();
    let index = repo.git_dir().join("index");
    repo.write_index("main").unwrap();
    fs::copy(index, root.join(".git/index")).unwrap();
    assert!(matches!(read(), Outcome::Known(_)));
}

#[cfg(unix)]
#[test]
fn a_link_the_repository_does_not_hold_still_reads() {
    let dir = tempfile::tempdir().unwrap();
    let repo = linked(&dir, &[]);
    // A tool's link, such as Bazel's, made in the worktree and never committed.
    std::os::unix::fs::symlink(dir.path().join("elsewhere"), repo.join("bazel-testlogs")).unwrap();
    let report = Report::read(
        &repo,
        Format::Lcov,
        &["bazel-testlogs/lcov.info".to_owned()],
    );
    assert!(matches!(report.outcome, Outcome::Known(_)));
}

#[cfg(unix)]
#[test]
fn an_index_that_cannot_be_read_refuses_links_leading_outside() {
    let dir = tempfile::tempdir().unwrap();
    let repo = linked(&dir, &[]);
    std::os::unix::fs::symlink(dir.path().join("elsewhere"), repo.join("bazel-testlogs")).unwrap();
    fs::write(repo.join(".git/index"), b"not an index").unwrap();
    let read = |path: &str| Report::read(&repo, Format::Lcov, &[path.to_owned()]).outcome;
    assert_eq!(
        read("bazel-testlogs/lcov.info"),
        Outcome::Unknown(Why::CommittedLink("bazel-testlogs".into()))
    );
    assert!(matches!(read("build/lcov.info"), Outcome::Known(_)));
}

#[test]
fn contributors_are_the_heaviest_twenty() {
    let mut many: Vec<Contributor> = (0..30)
        .map(|i| Contributor {
            path: format!("src/f{i:02}.ts"),
            value: i % 7,
        })
        .collect();
    many.push(Contributor {
        path: "x".repeat(2_000),
        value: 100,
    });
    let kept = top(many);
    assert_eq!(kept.len(), MAX_CONTRIBUTORS);
    assert_eq!(kept[0].path, "src/f06.ts");
    assert!(kept.iter().all(|c| c.value > 0 && c.path.len() < 100));
    assert!(
        kept.windows(2)
            .all(|w| w[0].value > w[1].value || (w[0].value == w[1].value && w[0].path < w[1].path))
    );
}

#[test]
fn ratios_hold_for_any_count() {
    assert_eq!(ratio(1, 2), Q16::HALF);
    assert_eq!(ratio(0, 5), Q16::ZERO);
    assert_eq!(ratio(5, 5), Q16::ONE);
    assert_eq!(ratio(u64::MAX, u64::MAX), Q16::ONE);
    assert_eq!(ratio(u64::MAX / 2, u64::MAX), Q16::HALF);
    assert_eq!(ratio(7, 0), Q16::ZERO);
}

// Truncation and noise

/// Every strict prefix of a valid report, as a file caught half-written.
fn prefixes(name: &str) -> (String, Vec<String>) {
    let text = fs::read_to_string(fixture(name)).unwrap();
    let cuts = (0..text.trim_end().len())
        .filter(|&cut| text.is_char_boundary(cut))
        .map(|cut| text[..cut].to_owned())
        .collect();
    (text, cuts)
}

#[test]
fn a_half_written_xml_or_json_report_is_never_known() {
    for (name, format) in [
        ("cobertura/istanbul.xml", Format::Cobertura),
        ("cobertura/coverage-py.xml", Format::Cobertura),
        ("junit/jest-junit.xml", Format::JUnit),
        ("junit/pytest.xml", Format::JUnit),
        ("junit/surefire/TEST-moss.ReedTest.xml", Format::JUnit),
        ("knip/v6.json", Format::Knip),
        ("local-tests/pass.json", Format::LocalTests),
    ] {
        let (text, cuts) = prefixes(name);
        assert!(
            matches!(parse_bytes(format, text.as_bytes()), Outcome::Known(_)),
            "{name}"
        );
        for cut in cuts {
            assert!(
                matches!(parse_bytes(format, cut.as_bytes()), Outcome::Unknown(_)),
                "{name} cut to {} bytes",
                cut.len()
            );
        }
    }
}

#[test]
fn a_half_written_lcov_reads_as_its_complete_records_or_not_at_all() {
    // lcov has no end-of-document marker: a cut between records reads as the records before it.
    for name in ["lcov/jest.info", "lcov/unit.info", "lcov/integration.info"] {
        let (_, cuts) = prefixes(name);
        for cut in cuts {
            let outcome = parse_bytes(Format::Lcov, cut.as_bytes());
            let Outcome::Known(known) = outcome else {
                continue;
            };
            let end = cut
                .rfind("end_of_record")
                .expect("a known cut closed a record")
                + "end_of_record".len();
            assert!(
                !cut[end..].contains("SF:"),
                "{name}: a cut inside a record reads as part of the report"
            );
            let Outcome::Known(complete) = parse_bytes(Format::Lcov, &cut.as_bytes()[..end]) else {
                panic!("{name}: the records before the cut read");
            };
            assert_eq!(
                known.reading,
                complete.reading,
                "{name} cut at {}",
                cut.len()
            );
        }
    }
}

proptest! {
    // A fixed seed, as the git reader's: a failure replays.
    #![proptest_config(ProptestConfig {
        rng_seed: proptest::test_runner::RngSeed::Fixed(0x7265_706f_7274),
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn noise_never_panics_and_never_reads_out_of_range(
        bytes in proptest::collection::vec(any::<u8>(), 0..512),
    ) {
        for format in [Format::Lcov, Format::Cobertura, Format::JUnit, Format::Knip, Format::LocalTests] {
            if let Outcome::Known(known) = parse_bytes(format, &bytes) {
                if let Some(value) = known.reading.normalized() {
                    prop_assert!((Q16::ZERO..=Q16::ONE).contains(&value));
                }
                prop_assert!(known.contributors.len() <= MAX_CONTRIBUTORS);
            }
        }
    }

    #[test]
    fn text_without_the_formats_markers_is_unknown(text in "[^<&_]{0,300}") {
        // No `end_of_record`, no element, no `issues` key: nothing to read.
        prop_assume!(!text.contains("issues"));
        for format in [Format::Lcov, Format::Cobertura, Format::JUnit, Format::Knip] {
            prop_assert!(matches!(parse_bytes(format, text.as_bytes()), Outcome::Unknown(_)));
        }
    }

    #[test]
    fn lcov_lines_in_any_order_and_any_count_stay_in_range(
        lines in proptest::collection::vec((1_u64..50, -2_i64..5), 1..60),
    ) {
        let mut text = String::from("SF:src/fern.ts\n");
        for (number, hits) in &lines {
            writeln!(text, "DA:{number},{hits}").unwrap();
        }
        text.push_str("end_of_record\n");
        let Outcome::Known(known) = parse_bytes(Format::Lcov, text.as_bytes()) else {
            panic!("known");
        };
        let distinct: std::collections::BTreeSet<u64> = lines.iter().map(|(n, _)| *n).collect();
        let hit: std::collections::BTreeSet<u64> =
            lines.iter().filter(|(_, h)| *h > 0).map(|(n, _)| *n).collect();
        prop_assert_eq!(known.reading, coverage(distinct.len() as u64, hit.len() as u64));
    }
}

#[test]
fn a_printed_report_reads_as_a_written_one_and_its_hash_names_its_commit() {
    let dir = stage(&[]);
    let bytes = fs::read(fixture("knip/v6.json")).unwrap();
    let printed = |tag: &[u8]| {
        Report::from_output(
            dir.path(),
            Format::Knip,
            "knip",
            bytes.clone(),
            1_760_000_000,
            tag,
        )
    };
    let at_a = printed(b"aaaa");
    let known = at_a.known().expect("known");
    assert_eq!(known.mtime, Some(1_760_000_000));
    assert_eq!(at_a.files, ["knip"]);
    assert_eq!(
        printed(b"aaaa").known().unwrap().content_hash,
        known.content_hash
    );
    assert_ne!(
        printed(b"bbbb").known().unwrap().content_hash,
        known.content_hash
    );
    let written = stage(&[("knip/v6.json", "knip-report.json")]);
    let read = Report::read(
        written.path(),
        Format::Knip,
        &["knip-report.json".to_owned()],
    );
    assert_eq!(read.known().unwrap().reading, known.reading);
    let cut = Report::from_output(
        dir.path(),
        Format::Knip,
        "knip",
        b"{\"issues\":[".to_vec(),
        0,
        b"a",
    );
    assert!(cut.known().is_none());
}
