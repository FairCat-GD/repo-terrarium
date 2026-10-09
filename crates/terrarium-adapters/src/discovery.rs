//! Discovery: what a repository has for each adapter, and what it lacks. Facts only; how they
//! are worded, and the hint for each missing adapter, are the app's copy.
//!
//! It looks only where the fixed lists below say, plus a configured path per adapter, and
//! never walks the repository. A `*` matches within one path segment. Every path is relative to
//! the repository root, and a configured one that is absolute or climbs out of it is refused.
//! Nothing is run, and nothing is written.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::git::GitError;
use crate::report::{Format, MAX_FILES, installed_knip, read_bounded};

/// lcov first, then Cobertura, each in the order tools write them: Jest, Vitest, c8 and nyc;
/// cargo-llvm-cov and tarpaulin; coverage.py.
pub const LCOV_PATHS: &[&str] = &["coverage/lcov.info", "lcov.info", "coverage.lcov"];
/// istanbul; coverage.py and pytest-cov; tarpaulin and cargo-llvm-cov.
pub const COBERTURA_PATHS: &[&str] = &[
    "coverage/cobertura-coverage.xml",
    "coverage.xml",
    "cobertura.xml",
];
/// jest-junit's default and common choices for pytest and the Go tools; Vitest 5's default;
/// Maven Surefire; Gradle; cargo-nextest with `junit.path = "junit.xml"`.
pub const JUNIT_PATHS: &[&str] = &[
    "junit.xml",
    "test-results.xml",
    "test-results/junit.xml",
    "reports/junit.xml",
    ".vitest/junit/output.xml",
    "target/surefire-reports/TEST-*.xml",
    "build/test-results/*/TEST-*.xml",
    "target/nextest/*/junit.xml",
];
/// Knip prints its report to stdout; this is the file it is read from.
pub const KNIP_REPORT_PATH: &str = "knip-report.json";
/// Knip's configuration files, as its documentation lists them (and `package.json#knip`).
pub const KNIP_CONFIG_PATHS: &[&str] = &[
    "knip.json",
    "knip.jsonc",
    ".knip.json",
    ".knip.jsonc",
    "knip.ts",
    "knip.js",
    "knip.config.ts",
    "knip.config.js",
];
pub const WORKFLOW_PATHS: &[&str] = &[".github/workflows/*.yml", ".github/workflows/*.yaml"];
const PACKAGE_JSON: &str = "package.json";
/// Configuration files are read only to look for a key or a section, and are small.
const CONFIG_MAX_BYTES: u64 = 1024 * 1024;

/// Test runners, in the order a missing test report's hint prefers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Runner {
    Jest,
    Vitest,
    Pytest,
    Mocha,
    Maven,
    Gradle,
    Nextest,
    Cargo,
    Go,
}

impl Runner {
    pub const ALL: [Runner; 9] = [
        Runner::Jest,
        Runner::Vitest,
        Runner::Pytest,
        Runner::Mocha,
        Runner::Maven,
        Runner::Gradle,
        Runner::Nextest,
        Runner::Cargo,
        Runner::Go,
    ];

    /// The files whose presence alone says the runner is configured.
    const fn config_paths(self) -> &'static [&'static str] {
        match self {
            Runner::Jest => &[
                "jest.config.js",
                "jest.config.ts",
                "jest.config.mjs",
                "jest.config.cjs",
                "jest.config.json",
            ],
            Runner::Vitest => &[
                "vitest.config.ts",
                "vitest.config.js",
                "vitest.config.mts",
                "vitest.config.mjs",
                "vitest.config.cts",
                "vitest.config.cjs",
            ],
            Runner::Pytest => &["pytest.ini", "conftest.py"],
            Runner::Mocha => &[
                ".mocharc.js",
                ".mocharc.cjs",
                ".mocharc.json",
                ".mocharc.jsonc",
                ".mocharc.yaml",
                ".mocharc.yml",
            ],
            Runner::Maven => &["pom.xml"],
            Runner::Gradle => &["build.gradle", "build.gradle.kts"],
            Runner::Nextest => &[".config/nextest.toml"],
            Runner::Cargo => &["Cargo.toml"],
            Runner::Go => &["go.mod"],
        }
    }

    /// Files that configure the runner only when they hold this section.
    const fn config_sections(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Runner::Pytest => &[
                ("pyproject.toml", "[tool.pytest.ini_options]"),
                ("tox.ini", "[pytest]"),
                ("setup.cfg", "[tool:pytest]"),
            ],
            _ => &[],
        }
    }

    /// The `package.json` key that configures the runner.
    const fn package_key(self) -> Option<&'static str> {
        match self {
            Runner::Jest => Some("jest"),
            _ => None,
        }
    }
}

/// A runner and the file that configures it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerFound {
    pub runner: Runner,
    pub config: String,
}

/// Report paths configured per adapter (`adapters.<kind>.report_path`), relative to the
/// repository root. They replace the fixed lists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Overrides {
    pub coverage: Option<String>,
    pub dead_code: Option<String>,
    pub weather: Option<String>,
}

/// Report files that were found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Located {
    pub format: Format,
    /// The fixed or configured path they matched.
    pub pattern: String,
    /// Every file it matched, relative to the repository root, in order.
    pub files: Vec<String>,
}

/// Where an adapter's report is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Search {
    Found(Located),
    NotFound,
    /// A configured path that is absolute or leaves the repository.
    Refused(String),
}

impl Search {
    pub fn located(&self) -> Option<&Located> {
        match self {
            Search::Found(located) => Some(located),
            Search::NotFound | Search::Refused(_) => None,
        }
    }
}

/// What a repository has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovery {
    /// The default branch, or why the repository could not be read.
    pub git: Result<String, GitError>,
    pub coverage: Search,
    pub test_report: Search,
    /// Whether `record-tests` has left a status file for this repository.
    pub local_tests: bool,
    pub dead_code: Search,
    pub knip_config: Option<String>,
    /// The version of the Knip in `node_modules`, if one is installed.
    pub knip_installed: Option<String>,
    pub ci_workflows: Vec<String>,
    pub runners: Vec<RunnerFound>,
}

/// Discovers what the repository at `root` has. `git` is its default branch, or why it could
/// not be read, from the caller's own open of it; `status_file` is where `record-tests` would
/// have written its file for this repository, when the data directory is known.
pub fn discover(
    root: &Path,
    git: Result<String, GitError>,
    overrides: &Overrides,
    status_file: Option<&Path>,
) -> Discovery {
    let Reports {
        coverage,
        test_report,
    } = locate_reports(root, overrides);
    let dead_code = match &overrides.dead_code {
        Some(path) => configured(root, path, Format::Knip),
        None => first_of(root, &[(Format::Knip, &[KNIP_REPORT_PATH])]),
    };
    let package = package_keys(root);
    let knip_config = KNIP_CONFIG_PATHS
        .iter()
        .find(|path| is_file(&root.join(path)))
        .map(|path| (*path).to_owned())
        .or_else(|| {
            package
                .iter()
                .any(|key| key == "knip")
                .then(|| format!("{PACKAGE_JSON}#knip"))
        });
    let mut ci_workflows: Vec<String> = WORKFLOW_PATHS
        .iter()
        .flat_map(|pattern| expand(root, pattern))
        .collect();
    ci_workflows.sort();
    Discovery {
        git,
        coverage,
        test_report,
        local_tests: status_file.is_some_and(is_file),
        dead_code,
        knip_config,
        knip_installed: installed_knip(root),
        ci_workflows,
        runners: runners(root, &package),
    }
}

/// The reports a refresh reads: coverage and test results, found as [`discover`] finds them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reports {
    pub coverage: Search,
    pub test_report: Search,
}

/// Where the coverage and test reports under `root` are, and nothing else: what the live
/// refresh looks up each time, without discovery's configuration checks.
pub fn locate_reports(root: &Path, overrides: &Overrides) -> Reports {
    let coverage = match &overrides.coverage {
        Some(path) => configured(root, path, coverage_format(path)),
        None => first_of(
            root,
            &[
                (Format::Lcov, LCOV_PATHS),
                (Format::Cobertura, COBERTURA_PATHS),
            ],
        ),
    };
    let test_report = match &overrides.weather {
        Some(path) => configured(root, path, Format::JUnit),
        None => first_of(root, &[(Format::JUnit, JUNIT_PATHS)]),
    };
    Reports {
        coverage,
        test_report,
    }
}

/// Every path a coverage or test report may be found at, a configured one replacing its fixed
/// list, as `/`-separated patterns relative to the root: what a watcher watches for. A refused
/// configured path is left out.
pub fn report_patterns(overrides: &Overrides) -> Vec<String> {
    let mut patterns = Vec::new();
    let mut add = |configured: &Option<String>, fixed: &[&[&str]]| match configured {
        Some(path) => patterns.extend(crate::report::relative_parts(path).map(|p| p.join("/"))),
        None => patterns.extend(
            fixed
                .iter()
                .flat_map(|list| list.iter())
                .map(|p| (*p).to_owned()),
        ),
    };
    add(&overrides.coverage, &[LCOV_PATHS, COBERTURA_PATHS]);
    add(&overrides.weather, &[JUNIT_PATHS]);
    patterns
}

/// Whether the `/`-separated path `relative` is one `pattern` matches, `*` matching within one
/// segment.
pub fn path_matches(pattern: &str, relative: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = relative.split('/').collect();
    pattern.len() == path.len() && pattern.iter().zip(&path).all(|(p, s)| wildcard(p, s))
}

/// Whether the `/`-separated path `relative` is a directory a match of `pattern` would lie
/// under: every one of its segments matches the pattern's segment at the same depth, and the
/// pattern goes deeper.
pub fn leads_to(pattern: &str, relative: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = relative.split('/').collect();
    path.len() < pattern.len() && pattern.iter().zip(&path).all(|(p, s)| wildcard(p, s))
}

/// A configured coverage path is Cobertura when it names XML, and lcov otherwise.
fn coverage_format(path: &str) -> Format {
    if path.to_ascii_lowercase().ends_with(".xml") {
        Format::Cobertura
    } else {
        Format::Lcov
    }
}

/// The first fixed path that matches anything, with everything it matches.
fn first_of(root: &Path, lists: &[(Format, &[&str])]) -> Search {
    for (format, patterns) in lists {
        for pattern in *patterns {
            let files = expand(root, pattern);
            if !files.is_empty() {
                return Search::Found(Located {
                    format: *format,
                    pattern: (*pattern).to_owned(),
                    files,
                });
            }
        }
    }
    Search::NotFound
}

fn configured(root: &Path, path: &str, format: Format) -> Search {
    let Some(parts) = crate::report::relative_parts(path) else {
        return Search::Refused(path.to_owned());
    };
    let pattern = parts.join("/");
    let files = expand(root, &pattern);
    if files.is_empty() {
        Search::NotFound
    } else {
        Search::Found(Located {
            format,
            pattern,
            files,
        })
    }
}

/// The regular files under `root` that `pattern` matches, sorted, at most one more than
/// [`MAX_FILES`] (which a report then refuses to read).
fn expand(root: &Path, pattern: &str) -> Vec<String> {
    let parts: Vec<&str> = pattern.split('/').collect();
    let mut found = Vec::new();
    expand_into(root, &parts, "", &mut found);
    found.sort();
    found
}

fn expand_into(dir: &Path, parts: &[&str], prefix: &str, found: &mut Vec<String>) {
    let Some((first, rest)) = parts.split_first() else {
        return;
    };
    let names: Vec<String> = if first.contains('*') {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| wildcard(first, name))
            .collect();
        names.sort();
        names
    } else {
        vec![(*first).to_owned()]
    };
    for name in names {
        if found.len() > MAX_FILES {
            return;
        }
        let path = dir.join(&name);
        let shown = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if rest.is_empty() {
            if is_file(&path) {
                found.push(shown);
            }
        } else if fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            expand_into(&path, rest, &shown, found);
        }
    }
}

/// Whether `name` matches `pattern`, where `*` stands for any run of characters.
fn wildcard(pattern: &str, name: &str) -> bool {
    let mut pieces = pattern.split('*');
    let first = pieces.next().unwrap_or("");
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let pieces: Vec<&str> = pieces.collect();
    let Some((last, middle)) = pieces.split_last() else {
        // No `*` at all: the whole name.
        return rest.is_empty();
    };
    for piece in middle {
        match rest.find(piece) {
            Some(at) => rest = &rest[at + piece.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

fn is_file(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

/// The top-level keys of `package.json`, if it reads as a JSON object.
fn package_keys(root: &Path) -> Vec<String> {
    read_bounded(&root.join(PACKAGE_JSON), PACKAGE_JSON, CONFIG_MAX_BYTES)
        .ok()
        .and_then(|file| serde_json::from_str::<Value>(&crate::report::text(&file).ok()?).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map.keys().cloned().collect()),
            _ => None,
        })
        .unwrap_or_default()
}

fn runners(root: &Path, package: &[String]) -> Vec<RunnerFound> {
    let has_section = |path: &str, section: &str| {
        read_bounded(&root.join(path), path, CONFIG_MAX_BYTES).is_ok_and(|file| {
            crate::report::text(&file)
                .is_ok_and(|text| text.lines().any(|line| line.trim() == section))
        })
    };
    Runner::ALL
        .iter()
        .filter_map(|&runner| {
            let config = runner
                .config_paths()
                .iter()
                .find(|path| is_file(&root.join(path)))
                .map(|path| (*path).to_owned())
                .or_else(|| {
                    runner
                        .config_sections()
                        .iter()
                        .find(|(path, section)| has_section(path, section))
                        .map(|(path, _)| (*path).to_owned())
                })
                .or_else(|| {
                    runner
                        .package_key()
                        .filter(|key| package.iter().any(|k| k == key))
                        .map(|key| format!("{PACKAGE_JSON}#{key}"))
                })?;
            Some(RunnerFound { runner, config })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Edit, FixtureRepo, Stamp};
    use crate::git::GitRepo;

    fn discover_in(root: &Path, overrides: &Overrides, status_file: Option<&Path>) -> Discovery {
        let git = GitRepo::open(root).and_then(|repo| repo.default_branch());
        discover(root, git, overrides, status_file)
    }

    /// A repository with one commit, and these files written into its worktree.
    fn repo_with(files: &[(&str, &str)]) -> (tempfile::TempDir, FixtureRepo) {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = FixtureRepo::init(&dir.path().join("moss")).unwrap();
        repo.commit(
            "main",
            Stamp::at(1_709_510_400),
            "seed",
            &[Edit::write("a.txt", "a\n")],
        )
        .unwrap();
        for (path, text) in files {
            let full = repo.path().join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, text).unwrap();
        }
        (dir, repo)
    }

    #[test]
    fn wildcards_match_within_a_segment() {
        assert!(wildcard("TEST-*.xml", "TEST-moss.FernTest.xml"));
        assert!(wildcard("*.yml", "ci.yml"));
        assert!(wildcard("*", "anything"));
        assert!(wildcard("a*b*c", "abc"));
        assert!(!wildcard("a*b*c", "acb"));
        assert!(!wildcard("TEST-*.xml", "TEST-.xm"));
        assert!(!wildcard("*.yml", "ci.yaml"));
        assert!(wildcard("junit.xml", "junit.xml"));
        assert!(!wildcard("junit.xml", "junit.xml.bak"));
        assert!(!wildcard("ab*ba", "aba"));
    }

    #[test]
    fn an_empty_repository_has_git_and_nothing_else() {
        let (_dir, repo) = repo_with(&[]);
        let found = discover_in(repo.path(), &Overrides::default(), None);
        assert_eq!(found.git, Ok("main".to_owned()));
        assert_eq!(found.coverage, Search::NotFound);
        assert_eq!(found.test_report, Search::NotFound);
        assert_eq!(found.dead_code, Search::NotFound);
        assert!(!found.local_tests);
        assert_eq!(found.knip_config, None);
        assert_eq!(found.knip_installed, None);
        assert!(found.ci_workflows.is_empty());
        assert!(found.runners.is_empty());
    }

    #[test]
    fn a_folder_that_is_not_a_repository_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let found = discover_in(dir.path(), &Overrides::default(), None);
        assert!(matches!(found.git, Err(GitError::Open(_))));
    }

    #[test]
    fn reports_and_configs_are_found_where_tools_write_them() {
        let (_dir, repo) = repo_with(&[
            ("coverage/lcov.info", "SF:a.ts\nDA:1,1\nend_of_record\n"),
            ("coverage.xml", "<coverage/>"),
            (
                "target/surefire-reports/TEST-moss.FernTest.xml",
                "<testsuite/>",
            ),
            (
                "target/surefire-reports/TEST-moss.ReedTest.xml",
                "<testsuite/>",
            ),
            ("target/surefire-reports/moss.FernTest.txt", "text"),
            ("knip-report.json", "{\"issues\":[]}"),
            ("knip.jsonc", "{}"),
            ("node_modules/knip/package.json", "{\"version\":\"6.39.0\"}"),
            (".github/workflows/ci.yml", "on: push"),
            (".github/workflows/nightly.yaml", "on: schedule"),
            (".github/workflows/notes.md", "no"),
            ("pom.xml", "<project/>"),
            (
                "pyproject.toml",
                "[project]\nname = \"moss\"\n\n[tool.pytest.ini_options]\n",
            ),
            ("package.json", "{\"name\":\"moss\",\"jest\":{}}"),
        ]);
        let status = repo.path().join("status.json");
        fs::write(&status, "{}").unwrap();
        let found = discover_in(repo.path(), &Overrides::default(), Some(&status));
        assert_eq!(
            found.coverage,
            Search::Found(Located {
                format: Format::Lcov,
                pattern: "coverage/lcov.info".into(),
                files: vec!["coverage/lcov.info".into()],
            }),
            "lcov comes before Cobertura"
        );
        assert_eq!(
            found.test_report,
            Search::Found(Located {
                format: Format::JUnit,
                pattern: "target/surefire-reports/TEST-*.xml".into(),
                files: vec![
                    "target/surefire-reports/TEST-moss.FernTest.xml".into(),
                    "target/surefire-reports/TEST-moss.ReedTest.xml".into(),
                ],
            })
        );
        assert!(found.local_tests);
        assert_eq!(
            found.dead_code.located().map(|l| l.files.clone()),
            Some(vec!["knip-report.json".to_owned()])
        );
        assert_eq!(found.knip_config.as_deref(), Some("knip.jsonc"));
        assert_eq!(found.knip_installed.as_deref(), Some("6.39.0"));
        assert_eq!(
            found.ci_workflows,
            [".github/workflows/ci.yml", ".github/workflows/nightly.yaml"]
        );
        assert_eq!(
            found.runners,
            [
                RunnerFound {
                    runner: Runner::Jest,
                    config: "package.json#jest".into()
                },
                RunnerFound {
                    runner: Runner::Pytest,
                    config: "pyproject.toml".into()
                },
                RunnerFound {
                    runner: Runner::Maven,
                    config: "pom.xml".into()
                },
            ]
        );
    }

    #[test]
    fn cobertura_is_found_without_lcov() {
        let (_dir, repo) = repo_with(&[("coverage.xml", "<coverage/>")]);
        let found = discover_in(repo.path(), &Overrides::default(), None);
        assert_eq!(
            found
                .coverage
                .located()
                .map(|l| (l.format, l.files.clone())),
            Some((Format::Cobertura, vec!["coverage.xml".to_owned()]))
        );
    }

    #[test]
    fn a_configured_path_replaces_the_list_and_stays_inside() {
        let (_dir, repo) = repo_with(&[
            ("coverage/lcov.info", "SF:a.ts\nDA:1,1\nend_of_record\n"),
            ("packages/fern/coverage/cobertura.xml", "<coverage/>"),
            ("out/tests/one.xml", "<testsuite/>"),
            ("out/tests/two.xml", "<testsuite/>"),
        ]);
        let overrides = Overrides {
            coverage: Some("packages/fern/./coverage/cobertura.xml".into()),
            dead_code: Some("../elsewhere/knip.json".into()),
            weather: Some(r"out\tests\*.xml".into()),
        };
        let found = discover_in(repo.path(), &overrides, None);
        assert_eq!(
            found.coverage,
            Search::Found(Located {
                format: Format::Cobertura,
                pattern: "packages/fern/coverage/cobertura.xml".into(),
                files: vec!["packages/fern/coverage/cobertura.xml".into()],
            })
        );
        assert_eq!(found.test_report.located().map(|l| l.files.len()), Some(2));
        assert_eq!(
            found.dead_code,
            Search::Refused("../elsewhere/knip.json".into())
        );
        for outside in ["/etc/lcov.info", r"C:\lcov.info", "a/../../lcov.info"] {
            let overrides = Overrides {
                coverage: Some(outside.into()),
                ..Overrides::default()
            };
            assert_eq!(
                discover_in(repo.path(), &overrides, None).coverage,
                Search::Refused(outside.into())
            );
        }
        let missing = Overrides {
            coverage: Some("nowhere/lcov.info".into()),
            ..Overrides::default()
        };
        assert_eq!(
            discover_in(repo.path(), &missing, None).coverage,
            Search::NotFound
        );
    }

    #[test]
    fn only_regular_files_are_found() {
        let (_dir, repo) = repo_with(&[]);
        fs::create_dir_all(repo.path().join("junit.xml")).unwrap();
        let found = discover_in(repo.path(), &Overrides::default(), None);
        assert_eq!(found.test_report, Search::NotFound);
    }

    #[test]
    fn report_paths_are_matched_and_led_to_as_a_watcher_needs() {
        assert!(path_matches("coverage/lcov.info", "coverage/lcov.info"));
        assert!(path_matches(
            "target/surefire-reports/TEST-*.xml",
            "target/surefire-reports/TEST-moss.xml"
        ));
        assert!(path_matches(
            "build/test-results/*/TEST-*.xml",
            "build/test-results/test/TEST-a.xml"
        ));
        assert!(!path_matches("coverage/lcov.info", "coverage"));
        assert!(!path_matches("lcov.info", "coverage/lcov.info"));
        assert!(leads_to("coverage/lcov.info", "coverage"));
        assert!(leads_to(
            "build/test-results/*/TEST-*.xml",
            "build/test-results/test"
        ));
        assert!(!leads_to("coverage/lcov.info", "coverage/lcov.info"));
        assert!(!leads_to("coverage/lcov.info", "src"));
        assert!(!leads_to("lcov.info", "lcov.info"));
    }

    #[test]
    fn report_patterns_follow_the_overrides() {
        let fixed = report_patterns(&Overrides::default());
        assert_eq!(
            fixed.len(),
            LCOV_PATHS.len() + COBERTURA_PATHS.len() + JUNIT_PATHS.len()
        );
        assert!(fixed.iter().any(|p| p == "target/nextest/*/junit.xml"));
        let configured = report_patterns(&Overrides {
            coverage: Some("./out/cov.info".into()),
            weather: Some("../outside.xml".into()),
            dead_code: None,
        });
        assert_eq!(configured, ["out/cov.info"]);
    }

    #[test]
    fn locating_reports_agrees_with_discovery() {
        let (_dir, repo) = repo_with(&[
            ("coverage.xml", "<coverage/>"),
            ("target/surefire-reports/TEST-a.xml", "<testsuite/>"),
        ]);
        let found = discover_in(repo.path(), &Overrides::default(), None);
        let reports = locate_reports(repo.path(), &Overrides::default());
        assert_eq!(reports.coverage, found.coverage);
        assert_eq!(reports.test_report, found.test_report);
        assert!(reports.coverage.located().is_some());
    }
}
