//! What changes a terrarium besides its refreshes: relinking a repository that moved,
//! re-seeding, importing observations, disconnecting. Each opens the store itself; the app
//! stops the engine first, so nothing else writes meanwhile, and starts it again after, so the
//! engine reads what changed.
//!
//! Connecting is [`Live::connect`](crate::Live::connect) with a path: it creates the terrarium,
//! or relinks the one with the repository's seed wherever it now lives.

use std::fs::File;
use std::io::BufReader;
use std::path::{Component, Path, PathBuf};

use serde::de::DeserializeOwned;
use terrarium_adapters::discovery::{Enabled, Overrides};
use terrarium_adapters::git::{GitError, GitRepo};
use terrarium_store::identifiers::{
    SETTING_COMPLEXITY_ENABLED, SETTING_COVERAGE_ENABLED, SETTING_COVERAGE_REPORT_PATH,
    SETTING_DEAD_CODE_ENABLED, SETTING_DEAD_CODE_REPORT_PATH, SETTING_REPO_ACTIVE,
    SETTING_TODO_DENSITY_ENABLED, SETTING_WEATHER_ENABLED, SETTING_WEATHER_REPORT_PATH,
};
use terrarium_store::{
    DataDir, ImportError, ImportReport, InstallKey, RepoLocation, Store, StoreError, Terrarium,
    TerrariumId,
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

/// Merges the observations export at `path` into the terrarium `id`: another terrarium's file,
/// or any line that does not read, is refused and writes nothing; otherwise new observations
/// are added, and one already stored with other values is kept as stored.
pub fn import(data: &DataDir, id: TerrariumId, path: &Path) -> Result<ImportReport, ImportError> {
    let mut store = Store::open(data)?;
    let file = BufReader::new(File::open(path)?);
    store.import_observations(id, file)
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

/// What the settings say of each adapter: the report path configured for it, if any, and
/// whether it is on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdapterSettings {
    pub overrides: Overrides,
    pub enabled: Enabled,
}

/// The adapters' settings. A value this version cannot read counts as never set, so a setting
/// written by hand never stops the terrarium from connecting.
pub fn stored_adapters(store: &Store) -> Result<AdapterSettings, StoreError> {
    fn read<T: DeserializeOwned>(store: &Store, key: &str) -> Result<Option<T>, StoreError> {
        match store.setting(key) {
            Err(StoreError::Json(_)) => Ok(None),
            read => read,
        }
    }
    let on = |key| -> Result<bool, StoreError> { Ok(read(store, key)?.unwrap_or(true)) };
    Ok(AdapterSettings {
        overrides: Overrides {
            coverage: read(store, SETTING_COVERAGE_REPORT_PATH)?,
            dead_code: read(store, SETTING_DEAD_CODE_REPORT_PATH)?,
            weather: read(store, SETTING_WEATHER_REPORT_PATH)?,
        },
        enabled: Enabled {
            coverage: on(SETTING_COVERAGE_ENABLED)?,
            dead_code: on(SETTING_DEAD_CODE_ENABLED)?,
            weather: on(SETTING_WEATHER_ENABLED)?,
            complexity: on(SETTING_COMPLEXITY_ENABLED)?,
            todo_density: on(SETTING_TODO_DENSITY_ENABLED)?,
        },
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
    use std::sync::atomic::AtomicBool;

    use terrarium_adapters::fixture;
    use terrarium_store::NewTerrarium;
    use terrarium_truth::{Prepared, RepoSeed, canonical_dump};

    use super::*;
    use crate::timelapse::Lapse;

    /// The fixture arc's repository in `dir`, and its seed.
    fn arc(dir: &Path) -> (PathBuf, RepoSeed) {
        let repo = dir.join("repo");
        fixture::arc(&repo).unwrap();
        let seed = GitRepo::open(&repo)
            .unwrap()
            .default_identity()
            .unwrap()
            .1
            .seed;
        (repo, seed)
    }

    /// The repository at `repo` connected, with seed `seed`, in a data directory of its own.
    fn connect(data: &DataDir, repo: &Path, seed: RepoSeed) -> Terrarium {
        let mut store = Store::open(data).unwrap();
        let id = store
            .create_terrarium(&NewTerrarium {
                seed,
                repo: location(repo, "main".to_owned()),
                install_salt: [0; 16],
                connected_at: fixture::ARC_START_SECS,
            })
            .unwrap();
        store.terrarium(id).unwrap().unwrap()
    }

    fn observations(data: &DataDir, id: TerrariumId) -> usize {
        Store::open(data)
            .unwrap()
            .observations(id, None)
            .unwrap()
            .len()
    }

    /// The terrarium's replay through the arc's end, as truth's canonical dump.
    fn replayed(data: &DataDir, terrarium: &Terrarium) -> String {
        let lapse = Lapse::for_terrarium(
            data,
            terrarium,
            fixture::ARC_REPLAY_END_SECS,
            &mut |_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        let prepared =
            Prepared::new(lapse.basis.clone(), &lapse.commits, &lapse.observations).unwrap();
        canonical_dump(&terrarium_truth::dump(&prepared, lapse.through).unwrap())
    }

    #[test]
    fn an_export_imported_on_another_install_replays_the_same_history() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, seed) = arc(dir.path());
        let here = DataDir::at(dir.path().join("here"));
        let mine = connect(&here, &repo, seed);
        let committed = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/arc/observations.jsonl");
        let recorded = import(&here, mine.id, &committed).unwrap();
        assert!(recorded.inserted > 300);
        let file = dir.path().join("moss-observations.jsonl");
        Store::open(&here)
            .unwrap()
            .export_observations(mine.id, std::fs::File::create(&file).unwrap())
            .unwrap();

        let there = DataDir::at(dir.path().join("there"));
        let theirs = connect(&there, &repo, seed);
        let moved = import(&there, theirs.id, &file).unwrap();
        assert_eq!((moved.inserted, moved.duplicates), (recorded.inserted, 0));
        assert!(moved.conflicts.is_empty());
        assert_eq!(replayed(&there, &theirs), replayed(&here, &mine));
        // Moved twice, nothing changes.
        let again = import(&there, theirs.id, &file).unwrap();
        assert_eq!((again.inserted, again.duplicates), (0, recorded.inserted));
    }

    #[test]
    fn an_unreadable_file_or_another_terrarium_s_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, seed) = arc(dir.path());
        let data = DataDir::at(dir.path().join("data"));
        let other = connect(&data, &repo, RepoSeed::from_bytes([7; 32]));
        assert!(matches!(
            import(&data, other.id, &dir.path().join("missing.jsonl")),
            Err(ImportError::Read(_))
        ));
        let committed = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/arc/observations.jsonl");
        assert!(matches!(
            import(&data, other.id, &committed),
            Err(ImportError::SeedMismatch { file, terrarium }) if file == seed && terrarium == other.seed
        ));
        assert_eq!(observations(&data, other.id), 0);
    }

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
