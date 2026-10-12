//! `repo-terrarium discover <path>`: what the repository has for each adapter, what each report
//! found reads as, and, for what is missing, how it connects. The connect window shows the same
//! report once, without the lines on connecting.
//!
//! It reads the repository and its reports and nothing else, and writes nothing. The data
//! directory is only looked in: for the status file `record-tests` leaves, and, when the
//! repository is the one connected, for the report paths its settings hold, which a database
//! that does not exist yet is never made to answer.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use terrarium_adapters::discovery::{Discovery, Overrides, Search, discover};
use terrarium_adapters::git::GitRepo;
use terrarium_adapters::local_tests;
use terrarium_adapters::report::Report;
use terrarium_engine::lifecycle::stored_adapters;
use terrarium_store::identifiers::SETTING_REPO_ACTIVE;
use terrarium_store::{DataDir, Store, Terrarium, TerrariumId};
use terrarium_truth::RepoSeed;

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
    let opened = GitRepo::open(repo);
    let git = opened
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|git| Ok((git, git.default_branch()?)));
    let seed = match &git {
        Ok((git, branch)) => git.identity(branch).ok().map(|identity| identity.seed),
        Err(_) => None,
    };
    let data = seed.and_then(|_| DataDir::resolve().ok());
    let status_file = seed
        .zip(data.as_ref())
        .map(|(seed, data)| data.local_tests(&seed));
    let overrides = seed
        .zip(data.as_ref())
        .and_then(|(seed, data)| connected_overrides(data, &seed))
        .unwrap_or_default();
    let found = discover(repo, git, &overrides, status_file.as_deref());
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

/// The report paths the settings hold, when the repository with `seed` is the one connected
/// and its database already exists.
fn connected_overrides(data: &DataDir, seed: &RepoSeed) -> Option<Overrides> {
    if !data.database().is_file() {
        return None;
    }
    let store = Store::open(data).ok()?;
    let id = store.setting::<i64>(SETTING_REPO_ACTIVE).ok()??;
    let terrarium = store.terrarium(TerrariumId::new(id)).ok()??;
    (terrarium.active && terrarium.seed == *seed)
        .then(|| stored_adapters(&store).ok())
        .flatten()
        .map(|adapters| adapters.overrides)
}

/// The report the connect window shows once, for the terrarium just connected: read with the
/// report paths its settings hold, as the strip reads them.
pub fn at_connect(data: &DataDir, terrarium: &Terrarium) -> String {
    report(data, terrarium, Hints::Left)
}

/// The report the settings show, with a line on how each missing adapter connects.
pub fn for_settings(data: &DataDir, terrarium: &Terrarium) -> String {
    report(data, terrarium, Hints::Shown)
}

fn report(data: &DataDir, terrarium: &Terrarium, hints: Hints) -> String {
    let repo = PathBuf::from(&terrarium.repo.path);
    let overrides = Store::open(data)
        .and_then(|store| stored_adapters(&store))
        .map(|adapters| adapters.overrides)
        .unwrap_or_default();
    let status_file = data.local_tests(&terrarium.seed);
    let opened = GitRepo::open(&repo);
    let git = opened
        .as_ref()
        .map(|git| (git, terrarium.repo.default_branch.clone()))
        .map_err(Clone::clone);
    let found = discover(&repo, git, &overrides, Some(&status_file));
    let reads = read_all(&repo, &found, Some(&status_file));
    copy::discovery_report(&found, &reads, hints)
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
