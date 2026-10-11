//! Where the database lives: `TERRARIUM_DATA_DIR` when it is set, otherwise the OS's per-user
//! app-data directory, in a folder named by the bundle id. Under test the variable is required,
//! so no test can reach a person's real data.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use terrarium_truth::RepoSeed;

use crate::identifiers::{
    BUNDLE_ID, DATA_DIR_ENV, DATABASE_FILE, INSTANCE_LOCK_FILE, LOCAL_TESTS_DIR, LOGS_DIR,
    REQUESTS_DIR, SHOW_SETTINGS_REQUEST,
};

/// A data directory: the database, the local test results, the running app's lock and the
/// requests a second launch leaves it, and the opt-in crash log's reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataDir(PathBuf);

#[derive(Debug, PartialEq, Eq)]
pub enum DataDirError {
    /// `TERRARIUM_DATA_DIR` is empty or relative. A relative path would follow the working
    /// directory, which may be the repository itself.
    NotAbsolute(PathBuf),
    /// The OS reports no home directory to put the data in.
    NoHome,
}

impl fmt::Display for DataDirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataDirError::NotAbsolute(path) => {
                write!(
                    f,
                    "{DATA_DIR_ENV} must be an absolute path, not '{}'",
                    path.display()
                )
            }
            DataDirError::NoHome => f.write_str("the OS reports no home directory"),
        }
    }
}

impl std::error::Error for DataDirError {}

impl DataDir {
    /// A data directory at `path`, for a harness that owns one.
    pub fn at(path: impl Into<PathBuf>) -> DataDir {
        DataDir(path.into())
    }

    /// This process's data directory: `TERRARIUM_DATA_DIR`, else the OS app-data directory
    /// (`~/Library/Application Support/<bundle id>` on macOS, `%LOCALAPPDATA%\<bundle id>` on
    /// Windows).
    ///
    /// # Panics
    ///
    /// Under test when `TERRARIUM_DATA_DIR` is not set: tests never resolve the real directory
    /// (I-14).
    pub fn resolve() -> Result<DataDir, DataDirError> {
        choose(std::env::var_os(DATA_DIR_ENV), under_test(), os_data_dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The database file.
    pub fn database(&self) -> PathBuf {
        self.0.join(DATABASE_FILE)
    }

    /// The directory of the status files `record-tests` writes.
    pub fn local_tests_dir(&self) -> PathBuf {
        self.0.join(LOCAL_TESTS_DIR)
    }

    /// The status file `record-tests` writes for the repository with `seed`.
    pub fn local_tests(&self, seed: &RepoSeed) -> PathBuf {
        self.local_tests_dir()
            .join(format!("{}.json", seed.to_hex()))
    }

    /// Where the opt-in crash log writes its reports.
    pub fn logs_dir(&self) -> PathBuf {
        self.0.join(LOGS_DIR)
    }

    /// The file the running app holds its lock on.
    pub fn instance_lock(&self) -> PathBuf {
        self.0.join(INSTANCE_LOCK_FILE)
    }

    /// Where a second launch leaves its requests for the running app.
    pub fn requests_dir(&self) -> PathBuf {
        self.0.join(REQUESTS_DIR)
    }

    /// The request to show the settings.
    pub fn show_settings_request(&self) -> PathBuf {
        self.requests_dir().join(SHOW_SETTINGS_REQUEST)
    }
}

/// This crate's own tests, or any test nextest runs: nextest sets `NEXTEST` in every test
/// process, and the processes a test starts inherit it. That guards the engine's and the app's
/// tests too, which link this crate without `cfg(test)`.
fn under_test() -> bool {
    cfg!(test) || std::env::var_os("NEXTEST").is_some()
}

fn os_data_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.data_local_dir().to_path_buf())
}

/// The choice behind [`DataDir::resolve`], with its inputs passed in so tests never read the
/// real environment or resolve the real directory.
fn choose(
    env: Option<OsString>,
    under_test: bool,
    os_base: impl FnOnce() -> Option<PathBuf>,
) -> Result<DataDir, DataDirError> {
    if let Some(dir) = env {
        let path = PathBuf::from(dir);
        return if path.is_absolute() {
            Ok(DataDir(path))
        } else {
            Err(DataDirError::NotAbsolute(path))
        };
    }
    assert!(
        !under_test,
        "tests and harnesses set {DATA_DIR_ENV} to a directory they own; the real data \
         directory is never resolved under test"
    );
    os_base()
        .map(|base| DataDir(base.join(BUNDLE_ID)))
        .ok_or(DataDirError::NoHome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never() -> Option<PathBuf> {
        panic!("the OS directory was resolved")
    }

    fn absolute(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn the_variable_wins_and_the_os_directory_is_never_asked() {
        let dir = absolute("terrarium-data");
        let chosen = choose(Some(dir.clone().into_os_string()), true, never);
        assert_eq!(chosen, Ok(DataDir(dir.clone())));
        let data = chosen.unwrap();
        assert_eq!(data.database(), dir.join("terrarium.sqlite3"));
        let seed = RepoSeed::from_bytes([0xab; 32]);
        assert_eq!(
            data.local_tests(&seed),
            dir.join("local-tests")
                .join(format!("{}.json", "ab".repeat(32)))
        );
        assert_eq!(data.instance_lock(), dir.join("instance.lock"));
        assert_eq!(
            data.show_settings_request(),
            dir.join("requests").join("show-settings")
        );
    }

    #[test]
    fn an_empty_or_relative_variable_is_refused() {
        for value in ["", "data", "./data"] {
            assert_eq!(
                choose(Some(OsString::from(value)), false, never),
                Err(DataDirError::NotAbsolute(PathBuf::from(value)))
            );
        }
    }

    #[test]
    fn outside_tests_the_os_directory_gets_a_folder_named_by_the_bundle_id() {
        let base = absolute("app-data");
        assert_eq!(
            choose(None, false, || Some(base.clone())),
            Ok(DataDir(base.join("io.github.faircat-gd.repo-terrarium")))
        );
        assert_eq!(choose(None, false, || None), Err(DataDirError::NoHome));
    }

    #[test]
    #[should_panic(expected = "set TERRARIUM_DATA_DIR to a directory they own")]
    fn under_test_the_real_directory_is_never_resolved() {
        let _ = choose(None, true, never);
    }
}
