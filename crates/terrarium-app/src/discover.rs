//! `repo-terrarium discover <path>`: what the repository has for each adapter, what each report
//! found reads as, and, for what is missing, how it connects. The connect window shows the same
//! report once, without the lines on connecting.
//!
//! It reads the repository and its reports and nothing else, and writes nothing. The data
//! directory is only looked in, for the status file `record-tests` leaves.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use terrarium_adapters::discovery::{Discovery, Overrides, Search, discover};
use terrarium_adapters::git::GitRepo;
use terrarium_adapters::local_tests;
use terrarium_adapters::report::Report;
use terrarium_engine::lifecycle::stored_overrides;
use terrarium_store::{DataDir, Store, Terrarium};

use crate::copy::Hints;
use crate::{copy, output};

/// What each report discovery found reads as.
pub struct Reads {
    pub coverage: Option<Report>,
    pub test_report: Option<Report>,
    pub local_tests: Option<Report>,
    pub dead_code: Option<Report>,
}

pub fn run(repo: &Path) -> ExitCode {
    let git = GitRepo::open(repo);
    let default_branch = git
        .as_ref()
        .map_err(Clone::clone)
        .and_then(GitRepo::default_branch);
    let status_file = match (&git, &default_branch) {
        (Ok(git), Ok(branch)) => status_file(git, branch),
        _ => None,
    };
    let found = discover(
        repo,
        default_branch,
        &Overrides::default(),
        status_file.as_deref(),
    );
    if let Err(err) = &found.git {
        eprintln!("{}", copy::discover_failed(repo, err));
        return ExitCode::FAILURE;
    }
    let reads = read_all(repo, &found, status_file.as_deref());
    output::print(
        &copy::discovery_report(&found, &reads, Hints::Shown),
        copy::discover_write_failed,
    )
}

/// The report the connect window shows once, for the terrarium just connected: read with the
/// report paths its settings hold, as the strip reads them.
pub fn at_connect(data: &DataDir, terrarium: &Terrarium) -> String {
    let repo = PathBuf::from(&terrarium.repo.path);
    let overrides = Store::open(data)
        .and_then(|store| stored_overrides(&store))
        .unwrap_or_default();
    let status_file = data.local_tests(&terrarium.seed);
    let found = discover(
        &repo,
        Ok(terrarium.repo.default_branch.clone()),
        &overrides,
        Some(&status_file),
    );
    let reads = read_all(&repo, &found, Some(&status_file));
    copy::discovery_report(&found, &reads, Hints::Left)
}

/// Where `record-tests` keeps this repository's status file, when the data directory and the
/// repository's seed are both known.
fn status_file(git: &GitRepo, default_branch: &str) -> Option<PathBuf> {
    let seed = git.identity(default_branch).ok()?.seed;
    Some(DataDir::resolve().ok()?.local_tests(&seed))
}

pub fn read_all(repo: &Path, found: &Discovery, status_file: Option<&Path>) -> Reads {
    let read = |search: &Search| {
        search
            .located()
            .map(|located| Report::read(repo, located.format, &located.files))
    };
    Reads {
        coverage: read(&found.coverage),
        test_report: read(&found.test_report),
        local_tests: status_file
            .filter(|_| found.local_tests)
            .map(local_tests::read),
        dead_code: read(&found.dead_code),
    }
}
