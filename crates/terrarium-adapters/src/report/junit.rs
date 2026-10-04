//! `JUnit` XML test results (local weather), as Maven Surefire, jest-junit, pytest, Vitest and
//! the Go converters write them.
//!
//! The counts on `<testsuites>` and `<testsuite>` differ from tool to tool (pytest writes none on
//! the root, several leave out `skipped`), so tests are counted from `<testcase>` elements. A run
//! fails when any `<failure>` or `<error>` stands anywhere, inside a test case or not, or when a
//! suite says it had failures or errors; Surefire's `flakyFailure` and `rerunFailure` are not
//! failures. A run in which no test ran and nothing failed says nothing.

use std::collections::BTreeMap;

use super::paths::Context;
use super::xml::{Step, attribute, walk};
use super::{Contributor, Loaded, Parsed, Reading, Why, count, text};

#[derive(Default)]
struct Totals {
    tests: u64,
    failed: u64,
    skipped: u64,
    /// Failures outside any test case, or announced by a suite's counts.
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
    if totals.failed == 0 && totals.suite_failed {
        // A suite failed before any of its cases ran (a module that does not load).
        totals.failed = 1;
    }
    if totals.failed == 0 && totals.tests.saturating_sub(totals.skipped) == 0 {
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
        },
        contributors,
    ))
}

fn read_file(context: &Context, file: &Loaded, totals: &mut Totals) -> Result<(), Why> {
    let text = text(file)?;
    let mut case: Option<Case> = None;
    let mut root: Option<String> = None;
    walk(file, text, |around, step| match step {
        Step::Start { name, attributes } => {
            if around.is_empty() {
                root = Some(name.to_owned());
            }
            match name {
                "testsuites" | "testsuite" => {
                    let says_failed = ["failures", "errors"]
                        .iter()
                        .any(|key| attribute(attributes, key).and_then(count).unwrap_or(0) > 0);
                    totals.suite_failed |= says_failed;
                }
                "testcase" => {
                    let origin = attribute(attributes, "file")
                        .and_then(|path| context.relative(path))
                        .or_else(|| {
                            attribute(attributes, "classname")
                                .filter(|class| !class.is_empty())
                                .map(str::to_owned)
                        });
                    case = Some(Case {
                        origin,
                        failed: false,
                        skipped: false,
                    });
                }
                "failure" | "error" => match case.as_mut() {
                    Some(open) => open.failed = true,
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
