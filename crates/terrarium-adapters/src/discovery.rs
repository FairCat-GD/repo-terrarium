//! Discovery: what a repository has for each adapter, and what it lacks. Facts only; how they
//! are worded, and the hint for each missing adapter, are the app's copy.
//!
//! It looks only where the fixed lists below say, plus a configured path per adapter, and
//! never walks the worktree; the source code it lists is read from the default tip's committed
//! tree, by path alone. A `*` matches within one path segment. Every path is relative to
//! the repository root, and a configured one that is absolute or climbs out of it is refused.
//! Where several fixed paths hold a report, the one written last is read. Nothing is run, and
//! nothing is written.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use serde_json::Value;

use crate::ci::Slug;
use crate::git::{GitError, GitRepo};
use crate::metrics::{Language, languages_at};
use crate::report::{Format, MAX_FILES, installed_knip, read_bounded};

/// lcov first, then Cobertura, each in the order tools write them: Jest, Vitest, c8 and nyc;
/// cargo-llvm-cov and tarpaulin; coverage.py. The order settles only reports written at the
/// same moment; otherwise the newest is read.
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

/// Which adapters read (`adapters.<kind>.enabled`): each is on until the settings turn it off.
/// A report adapter that is off is still discovered; it is neither read nor watched. Complexity
/// and TODO density are measured from the committed source, not a report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one switch per adapter, as the settings show them: any of them may be on"
)]
pub struct Enabled {
    pub coverage: bool,
    pub dead_code: bool,
    /// Test results: a `JUnit` report and the `record-tests` status file.
    pub weather: bool,
    pub complexity: bool,
    pub todo_density: bool,
}

impl Default for Enabled {
    fn default() -> Self {
        Enabled {
            coverage: true,
            dead_code: true,
            weather: true,
            complexity: true,
            todo_density: true,
        }
    }
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
    /// The GitHub repository `origin` names, whose runs the weather can read.
    pub github: Option<Slug>,
    pub runners: Vec<RunnerFound>,
    /// The source files at the default tip in each language the source metrics read; empty
    /// when there are none, or the tip could not be read.
    pub source_code: BTreeMap<Language, u64>,
}

/// Discovers what the repository at `root` has. `git` is the caller's own open of it with its
/// default branch, or why it could not be read; `status_file` is where `record-tests` would
/// have written its file for this repository, when the data directory is known.
pub fn discover(
    root: &Path,
    git: Result<(&GitRepo, String), GitError>,
    overrides: &Overrides,
    status_file: Option<&Path>,
) -> Discovery {
    let Reports {
        coverage,
        test_report,
        dead_code,
    } = locate_reports(root, overrides);
    let source_code = git.as_ref().map_or_else(
        |_| BTreeMap::new(),
        |(repo, branch)| source_code(repo, branch),
    );
    let github = git
        .as_ref()
        .ok()
        .and_then(|(repo, _)| repo.origin_url())
        .and_then(|url| Slug::from_remote_url(&url));
    let git = git.map(|(_, branch)| branch);
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
        github,
        runners: runners(root, &package),
        source_code,
    }
}

/// The source files at the tip of `default_branch`, counted per language from the paths of
/// its tree.
fn source_code(git: &GitRepo, default_branch: &str) -> BTreeMap<Language, u64> {
    git.state()
        .ok()
        .and_then(|state| state.branches.get(default_branch).cloned())
        .and_then(|tip| languages_at(git, &tip).ok())
        .unwrap_or_default()
}

/// The reports a refresh reads, found as [`discover`] finds them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reports {
    pub coverage: Search,
    pub test_report: Search,
    pub dead_code: Search,
}

/// Where the coverage, test and Knip reports under `root` are, and nothing else: what the live
/// refresh looks up each time, without discovery's configuration checks.
pub fn locate_reports(root: &Path, overrides: &Overrides) -> Reports {
    let coverage = match &overrides.coverage {
        Some(path) => configured(root, path, coverage_format(path)),
        None => newest_of(
            root,
            &[
                (Format::Lcov, LCOV_PATHS),
                (Format::Cobertura, COBERTURA_PATHS),
            ],
        ),
    };
    let test_report = match &overrides.weather {
        Some(path) => configured(root, path, Format::JUnit),
        None => newest_of(root, &[(Format::JUnit, JUNIT_PATHS)]),
    };
    let dead_code = match &overrides.dead_code {
        Some(path) => configured(root, path, Format::Knip),
        None => newest_of(root, &[(Format::Knip, &[KNIP_REPORT_PATH])]),
    };
    Reports {
        coverage,
        test_report,
        dead_code,
    }
}

/// Every path a coverage, test or Knip report may be found at, a configured one replacing its
/// fixed list, as `/`-separated patterns relative to the root: what a watcher watches for. A refused
/// configured path is left out, and so is every path of an adapter that is off.
pub fn report_patterns(overrides: &Overrides, enabled: Enabled) -> Vec<String> {
    let mut patterns = Vec::new();
    let mut add = |on: bool, configured: &Option<String>, fixed: &[&[&str]]| match configured {
        _ if !on => {}
        Some(path) => patterns.extend(crate::report::relative_parts(path).map(|p| p.join("/"))),
        None => patterns.extend(
            fixed
                .iter()
                .flat_map(|list| list.iter())
                .map(|p| (*p).to_owned()),
        ),
    };
    add(
        enabled.coverage,
        &overrides.coverage,
        &[LCOV_PATHS, COBERTURA_PATHS],
    );
    add(enabled.weather, &overrides.weather, &[JUNIT_PATHS]);
    add(
        enabled.dead_code,
        &overrides.dead_code,
        &[&[KNIP_REPORT_PATH]],
    );
    patterns
}

/// A configured report path as discovery reads it: relative to the root and `/`-separated;
/// `None` when it is refused, being absolute or climbing out of the repository.
pub fn configured_path(path: &str) -> Option<String> {
    crate::report::relative_parts(path).map(|parts| parts.join("/"))
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

/// Of the fixed paths that match anything, the one whose newest file was written last, with
/// everything it matches: a fresh report is read wherever the tools left it, and a stale one
/// elsewhere is passed over. A path matching many files (one report per test class) counts as
/// new as its newest. Equal times go by the lists' order, and a file whose time cannot be read
/// counts as the oldest.
fn newest_of(root: &Path, lists: &[(Format, &[&str])]) -> Search {
    let mut newest: Option<(Option<SystemTime>, Located)> = None;
    for (format, patterns) in lists {
        for pattern in *patterns {
            let files = expand(root, pattern);
            if files.is_empty() {
                continue;
            }
            let written = files
                .iter()
                .filter_map(|file| {
                    fs::metadata(root.join(file))
                        .and_then(|m| m.modified())
                        .ok()
                })
                .max();
            if newest.as_ref().is_none_or(|(time, _)| written > *time) {
                newest = Some((
                    written,
                    Located {
                        format: *format,
                        pattern: (*pattern).to_owned(),
                        files,
                    },
                ));
            }
        }
    }
    newest.map_or(Search::NotFound, |(_, located)| Search::Found(located))
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
        let opened = GitRepo::open(root);
        let git = opened
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|repo| Ok((repo, repo.default_branch()?)));
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
            // Written at one moment, so only a test that dates a file sets which is newest.
            stamp(&repo, path, 0);
        }
        (dir, repo)
    }

    /// Dates the file at `path` `seconds` after a fixed moment.
    fn stamp(repo: &FixtureRepo, path: &str, seconds: u64) {
        let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_790_000_000 + seconds);
        fs::File::options()
            .write(true)
            .open(repo.path().join(path))
            .unwrap()
            .set_modified(at)
            .unwrap();
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
        assert_eq!(found.github, None);
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
        repo.append_git_config(
            "[remote \"origin\"]\n\turl = https://github.com/example-garden/moss.git\n",
        )
        .unwrap();
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
            "written at one moment, lcov comes before Cobertura"
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
        assert_eq!(found.github, Slug::parse("example-garden/moss"));
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
    fn the_newest_report_is_read_wherever_the_tools_left_it() {
        let (_dir, repo) = repo_with(&[
            ("coverage/lcov.info", "SF:a.ts\nDA:1,1\nend_of_record\n"),
            ("lcov.info", "SF:a.ts\nDA:1,0\nend_of_record\n"),
            ("coverage.xml", "<coverage/>"),
            ("junit.xml", "<testsuite/>"),
            ("target/surefire-reports/TEST-a.xml", "<testsuite/>"),
            ("target/surefire-reports/TEST-b.xml", "<testsuite/>"),
        ]);
        let coverage = |repo: &FixtureRepo, overrides: &Overrides| {
            locate_reports(repo.path(), overrides)
                .coverage
                .located()
                .map(|located| (located.format, located.files.clone()))
        };
        stamp(&repo, "lcov.info", 60);
        assert_eq!(
            coverage(&repo, &Overrides::default()),
            Some((Format::Lcov, vec!["lcov.info".to_owned()])),
            "a fresher lcov.info is read over coverage/lcov.info"
        );
        stamp(&repo, "coverage.xml", 120);
        assert_eq!(
            coverage(&repo, &Overrides::default()),
            Some((Format::Cobertura, vec!["coverage.xml".to_owned()])),
            "a fresher Cobertura report is read over an older lcov"
        );
        // A configured path is read whatever the dates.
        let configured = Overrides {
            coverage: Some("coverage/lcov.info".into()),
            ..Overrides::default()
        };
        assert_eq!(
            coverage(&repo, &configured),
            Some((Format::Lcov, vec!["coverage/lcov.info".to_owned()]))
        );
        // A report of many files is as new as its newest, and is read whole.
        stamp(&repo, "junit.xml", 200);
        stamp(&repo, "target/surefire-reports/TEST-b.xml", 300);
        let tests = locate_reports(repo.path(), &Overrides::default()).test_report;
        assert_eq!(
            tests.located().map(|located| located.files.clone()),
            Some(vec![
                "target/surefire-reports/TEST-a.xml".to_owned(),
                "target/surefire-reports/TEST-b.xml".to_owned(),
            ])
        );
        // Discovery says the same.
        let found = discover_in(repo.path(), &Overrides::default(), None);
        assert_eq!(found.test_report, tests);
    }

    #[test]
    fn report_patterns_follow_the_overrides_and_leave_out_what_is_off() {
        let all = Enabled::default();
        let fixed = report_patterns(&Overrides::default(), all);
        assert_eq!(
            fixed.len(),
            LCOV_PATHS.len() + COBERTURA_PATHS.len() + JUNIT_PATHS.len() + 1
        );
        assert!(fixed.iter().any(|p| p == KNIP_REPORT_PATH));
        assert!(fixed.iter().any(|p| p == "target/nextest/*/junit.xml"));
        let configured = report_patterns(
            &Overrides {
                coverage: Some("./out/cov.info".into()),
                weather: Some("../outside.xml".into()),
                dead_code: Some("reports/knip.json".into()),
            },
            all,
        );
        assert_eq!(configured, ["out/cov.info", "reports/knip.json"]);
        assert_eq!(
            configured_path("./out/cov.info").as_deref(),
            Some("out/cov.info")
        );
        assert_eq!(
            configured_path("out\\junit.xml").as_deref(),
            Some("out/junit.xml")
        );
        for refused in ["../outside.xml", "/abs/lcov.info", "C:\\lcov.info", "", "."] {
            assert_eq!(configured_path(refused), None, "{refused}");
        }
        let no_coverage = report_patterns(
            &Overrides::default(),
            Enabled {
                coverage: false,
                dead_code: false,
                ..all
            },
        );
        assert_eq!(no_coverage, JUNIT_PATHS);
        let no_tests = report_patterns(
            &Overrides {
                coverage: Some("out/cov.info".into()),
                ..Overrides::default()
            },
            Enabled {
                weather: false,
                dead_code: false,
                ..all
            },
        );
        assert_eq!(no_tests, ["out/cov.info"]);
        let no_dead_code = report_patterns(
            &Overrides::default(),
            Enabled {
                coverage: false,
                weather: false,
                ..all
            },
        );
        assert_eq!(no_dead_code, [KNIP_REPORT_PATH]);
    }

    #[test]
    fn the_source_code_is_counted_from_the_default_tips_tree() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = FixtureRepo::init(&dir.path().join("moss")).unwrap();
        let stamp = Stamp::at(1_709_510_400);
        repo.commit(
            "main",
            stamp,
            "seed",
            &[
                Edit::write("src/a.rs", "fn a() {}\n"),
                Edit::write("web/b.ts", "export const b = 1;\n"),
                Edit::write("node_modules/c/index.js", "module.exports = 1;\n"),
            ],
        )
        .unwrap();
        repo.branch("feature", "main").unwrap();
        repo.commit(
            "feature",
            stamp,
            "elsewhere",
            &[Edit::write("c.go", "package c\n")],
        )
        .unwrap();
        // A file only in the worktree is not committed source.
        fs::write(repo.path().join("d.py"), "x = 1\n").unwrap();
        let found = discover_in(repo.path(), &Overrides::default(), None);
        assert_eq!(
            found.source_code,
            [(Language::TypeScript, 1), (Language::Rust, 1)].into()
        );
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
        assert_eq!(reports.dead_code, found.dead_code);
        assert!(reports.coverage.located().is_some());
    }
}
