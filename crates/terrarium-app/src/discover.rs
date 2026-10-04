//! `repo-terrarium discover <path>`: what the repository has for each adapter, what each report
//! found reads as, and, for what is missing, how it connects. The settings window shows the
//! same report.
//!
//! It reads the repository and its reports and nothing else, and writes nothing. The data
//! directory is only looked in, for the status file `record-tests` leaves.

use std::io::{ErrorKind, Write as _};
use std::path::Path;
use std::process::ExitCode;

use terrarium_adapters::discovery::{Discovery, Overrides, discover};
use terrarium_adapters::git::GitRepo;
use terrarium_adapters::local_tests;
use terrarium_adapters::report::Report;
use terrarium_store::DataDir;

use crate::copy;

/// What each report discovery found reads as.
pub struct Reads {
    pub coverage: Option<Report>,
    pub test_report: Option<Report>,
    pub local_tests: Option<Report>,
    pub dead_code: Option<Report>,
}

pub fn run(repo: &Path) -> ExitCode {
    let status_file = status_file(repo);
    let found = discover(repo, &Overrides::default(), status_file.as_deref());
    if let Err(err) = &found.git {
        eprintln!("{}", copy::discover_failed(repo, err));
        return ExitCode::FAILURE;
    }
    let reads = read_all(repo, &found, status_file.as_deref());
    let text = copy::discovery_report(&found, &reads);
    let mut out = std::io::stdout().lock();
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if err.kind() == ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{}", copy::discover_write_failed(&err));
            ExitCode::FAILURE
        }
    }
}

/// Where `record-tests` keeps this repository's status file, when the data directory and the
/// repository's seed are both known.
fn status_file(repo: &Path) -> Option<std::path::PathBuf> {
    let git = GitRepo::open(repo).ok()?;
    let seed = git.identity(&git.default_branch().ok()?).ok()?.seed;
    Some(DataDir::resolve().ok()?.local_tests(&seed))
}

pub fn read_all(repo: &Path, found: &Discovery, status_file: Option<&Path>) -> Reads {
    let read = |search: &terrarium_adapters::discovery::Search| {
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
