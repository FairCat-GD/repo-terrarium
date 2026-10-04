//! `JUnit` XML test results (local weather), as Maven Surefire, jest-junit, pytest, Vitest and
//! the Go converters write them.
//!
//! The counts on `<testsuites>` and `<testsuite>` differ from tool to tool (pytest writes none on
//! the root, several leave out `skipped`), so tests are counted from `<testcase>` elements. A run
//! fails when any `<failure>` or `<error>` stands anywhere, inside a test case or not, or when a
//! suite announces more failures and errors than its test cases hold (a module that did not
//! load, an unhandled error). Such a failure is kept apart from the failing tests, never counted
//! as one. Surefire's `flakyFailure` and `rerunFailure` are not failures. A run in which no test
//! ran and nothing failed says nothing.

use std::borrow::Cow;
use std::collections::BTreeMap;

use super::paths::Context;
use super::xml::{Step, attribute, walk};
use super::{Contributor, Loaded, Parsed, Reading, Why, count};

#[derive(Default)]
struct Totals {
    tests: u64,
    failed: u64,
    skipped: u64,
    /// A failure outside any test case, or one a suite's counts announce: a module that did not
    /// load, an unhandled error. It fails the run without being a failing test.
    suite_failed: bool,
    /// Failing test cases per file (or class, when no file is given).
    failing: BTreeMap<String, i64>,
}

/// The test case being read.
struct Case {
    origin: Option<String>,
    failed: bool,
    skipped: bool,
}

pub(crate) fn parse(context: &Context, files: &[Loaded]) -> Parsed {
    let mut totals = Totals::default();
    for file in files {
        read_file(context, file, &mut totals)?;
    }
    if totals.failed == 0
        && !totals.suite_failed
        && totals.tests.saturating_sub(totals.skipped) == 0
    {
        return Err(Why::Empty);
    }
    let contributors = totals
        .failing
        .into_iter()
        .map(|(path, value)| Contributor { path, value })
        .collect();
    Ok((
        Reading::Tests {
            tests: totals.tests,
            failed: totals.failed,
            skipped: totals.skipped,
            suite_failed: totals.suite_failed,
        },
        contributors,
    ))
}

fn read_file(context: &Context, file: &Loaded, totals: &mut Totals) -> Result<(), Why> {
    let mut case: Option<Case> = None;
    let mut root: Option<String> = None;
    // Per open suite: the failures and errors it announces, and those its test cases hold.
    let mut suites: Vec<(u64, u64)> = Vec::new();
    walk(file, |around, step| match step {
        Step::Start { name, element } => {
            if around.is_empty() {
                root = Some(name.to_owned());
            }
            match name {
                "testsuites" | "testsuite" => {
                    let announced = ["failures", "errors"].iter().fold(0_u64, |sum, key| {
                        let n = attribute(element, key).as_deref().and_then(count);
                        sum.saturating_add(n.unwrap_or(0))
                    });
                    suites.push((announced, 0));
                }
                "testcase" => {
                    let origin = attribute(element, "file")
                        .and_then(|path| context.locate(&path, &file.path))
                        .or_else(|| {
                            attribute(element, "classname")
                                .filter(|class| !class.is_empty())
                                .map(Cow::into_owned)
                        });
                    case = Some(Case {
                        origin,
                        failed: false,
                        skipped: false,
                    });
                }
                "failure" | "error" => match case.as_mut() {
                    Some(open) => {
                        open.failed = true;
                        for (_, seen) in &mut suites {
                            *seen = seen.saturating_add(1);
                        }
                    }
                    None => totals.suite_failed = true,
                },
                "skipped" => {
                    if let Some(open) = case.as_mut() {
                        open.skipped = true;
                    }
                }
                _ => {}
            }
        }
        Step::End { name } => {
            if matches!(name, "testsuites" | "testsuite")
                && let Some((announced, seen)) = suites.pop()
            {
                // More than its test cases hold: a failure none of them carries.
                totals.suite_failed |= announced > seen;
            }
            if name == "testcase"
                && let Some(done) = case.take()
            {
                totals.tests = totals.tests.saturating_add(1);
                if done.failed {
                    totals.failed = totals.failed.saturating_add(1);
                    if let Some(origin) = done.origin {
                        *totals.failing.entry(origin).or_insert(0) += 1;
                    }
                } else if done.skipped {
                    totals.skipped = totals.skipped.saturating_add(1);
                }
            }
        }
        Step::Text(_) => {}
    })?;
    if !matches!(root.as_deref(), Some("testsuites" | "testsuite")) {
        return Err(Why::Malformed {
            path: file.path.clone(),
            line: None,
        });
    }
    Ok(())
}
