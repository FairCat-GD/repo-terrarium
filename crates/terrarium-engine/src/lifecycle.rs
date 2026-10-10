//! What changes a terrarium besides its refreshes: relinking a repository that moved,
//! re-seeding, disconnecting. Each opens the store itself; the app stops the engine first, so
//! nothing else writes meanwhile, and starts it again after.
//!
//! Connecting is [`Live::connect`](crate::Live::connect) with a path: it creates the terrarium,
//! or relinks the one with the repository's seed wherever it now lives.

use std::path::{Component, Path, PathBuf};

use terrarium_adapters::discovery::Overrides;
use terrarium_adapters::git::{GitError, GitRepo};
use terrarium_store::identifiers::{
    SETTING_COVERAGE_REPORT_PATH, SETTING_DEAD_CODE_REPORT_PATH, SETTING_REPO_ACTIVE,
    SETTING_WEATHER_REPORT_PATH,
};
use terrarium_store::{
    DataDir, InstallKey, RepoLocation, Store, StoreError, Terrarium, TerrariumId,
};

use crate::live::ConnectError;
use crate::shown;

/// Relinks the terrarium `id` to the repository at `path`, which must be the same repository:
/// the seed its recorded default branch reaches, or, if that branch is gone, the seed of the
/// branch connecting would take. Another repository is refused, and nothing changes.
pub fn relink(data: &DataDir, id: TerrariumId, path: &Path) -> Result<Terrarium, ConnectError> {
    let mut store = Store::open(data)?;
    let terrarium = store
        .terrarium(id)?
        .ok_or(StoreError::NoSuchTerrarium(id))?;
    let path = absolute(path)?;
    let git = GitRepo::open(&path)?;
    let recorded = &terrarium.repo.default_branch;
    let (branch, seed) = match git.identity(recorded) {
        Ok(identity) => (recorded.clone(), identity.seed),
        Err(GitError::BranchMissing(_)) => {
            let (branch, identity) = git.default_identity()?;
            (branch, identity.seed)
        }
        Err(err) => return Err(err.into()),
    };
    if seed != terrarium.seed {
        return Err(ConnectError::OtherRepository);
    }
    store.activate(id, &location(&path, branch))?;
    store.set_setting(SETTING_REPO_ACTIVE, &id.get())?;
    Ok(store
        .terrarium(id)?
        .ok_or(StoreError::NoSuchTerrarium(id))?)
}

/// Gives the terrarium `id` a new install salt and drops its cached replay; its observations,
/// and so its truth, are kept. Returns the new salt.
pub fn reseed(data: &DataDir, id: TerrariumId) -> Result<[u8; 16], ConnectError> {
    let mut store = Store::open(data)?;
    let salt = new_salt()?;
    store.reseed(id, salt)?;
    Ok(salt)
}

/// Disconnects the terrarium `id`. Kept, its observations relink when the repository is
/// connected again; deleted, they go with its row, its cached replay, the status file
/// `record-tests` writes for it and what its scenes last played.
pub fn disconnect(
    data: &DataDir,
    id: TerrariumId,
    delete_observations: bool,
) -> Result<(), ConnectError> {
    let mut store = Store::open(data)?;
    if !delete_observations {
        store.disconnect(id)?;
        return Ok(());
    }
    let terrarium = store
        .terrarium(id)?
        .ok_or(StoreError::NoSuchTerrarium(id))?;
    store.delete_terrarium(id)?;
    match std::fs::remove_file(data.local_tests(&terrarium.seed)) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            return Err(StoreError::Io(err).into());
        }
        _ => {}
    }
    if shown::names(&store, &terrarium.seed)? {
        store.clear_install_value(InstallKey::ShownEvents)?;
    }
    Ok(())
}

/// The report paths configured per adapter.
pub fn stored_overrides(store: &Store) -> Result<Overrides, StoreError> {
    Ok(Overrides {
        coverage: store.setting(SETTING_COVERAGE_REPORT_PATH)?,
        dead_code: store.setting(SETTING_DEAD_CODE_REPORT_PATH)?,
        weather: store.setting(SETTING_WEATHER_REPORT_PATH)?,
    })
}

/// A new terrarium's install salt, from the OS's random source.
pub(crate) fn new_salt() -> Result<[u8; 16], ConnectError> {
    let mut salt = [0; 16];
    getrandom::fill(&mut salt).map_err(|err| ConnectError::Salt(err.to_string()))?;
    Ok(salt)
}

/// `path` made absolute, with its `.` and `..` resolved as text.
pub(crate) fn absolute(path: &Path) -> Result<PathBuf, ConnectError> {
    Ok(normalized(
        &std::path::absolute(path).map_err(ConnectError::Path)?,
    ))
}

/// Where the repository at the absolute `path` is connected from, named by its folder.
pub(crate) fn location(path: &Path, default_branch: String) -> RepoLocation {
    RepoLocation {
        path: path.display().to_string(),
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        default_branch,
    }
}

/// `path` with its `.` and `..` resolved as text, so `--repo ..` names the folder it means.
/// Canonicalizing would also resolve symlinks, and on Windows give `\\?\` paths.
pub(crate) fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            part => out.push(part),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_normalized_as_text() {
        let base = std::path::absolute("moss").unwrap();
        assert_eq!(normalized(&base.join("sub/..")), base);
        assert_eq!(normalized(&base.join("./a/./b/../c")), base.join("a/c"));
        assert_eq!(
            normalized(&base.join("..")).file_name(),
            base.parent().unwrap().file_name()
        );
    }

    #[test]
    fn an_unknown_terrarium_is_neither_reseeded_nor_disconnected() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::at(dir.path());
        let ghost = TerrariumId::new(4);
        assert!(matches!(
            reseed(&data, ghost),
            Err(ConnectError::Store(StoreError::NoSuchTerrarium(_)))
        ));
        for delete in [false, true] {
            assert!(matches!(
                disconnect(&data, ghost, delete),
                Err(ConnectError::Store(StoreError::NoSuchTerrarium(_)))
            ));
        }
    }
}
