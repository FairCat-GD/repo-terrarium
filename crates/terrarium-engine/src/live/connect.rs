//! Connecting a repository: what the engine is given, the terrarium a path creates or relinks,
//! and what a connected pipeline starts from: the newest stored rows and the cached base.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use terrarium_adapters::discovery::Enabled;
use terrarium_adapters::exec::{Tool, find_node};
use terrarium_adapters::git::{GitError, GitRepo};
use terrarium_adapters::metrics::{METRICS_VERSION, Metrics};
use terrarium_adapters::report::Format;
use terrarium_store::identifiers::SETTING_REPO_ACTIVE;
use terrarium_store::{
    DataDir, InstallKey, NewTerrarium, Store, StoreError, StoredObservation, Terrarium, TerrariumId,
};
use terrarium_truth::{
    CommitId, ObservationKind, SIM_VERSION, SceneInput, SimState, Tick, project_showing,
};

use super::events::{CiNewest, CiState, KnipState, ci_newest_of, knip_run};
use super::reports::{Newest, patterns};
use super::{Live, News};
use crate::ci::CiConfig;
use crate::clock::{Clock, SystemClock};
use crate::details::{FileCache, Found};
use crate::knip::KnipStatus;
use crate::lifecycle::{self, stored_adapters};
use crate::observe;
use crate::showing::Showing;
use crate::shown::Remembered;

/// How long a history read runs before its progress is shown: a short one never flickers a
/// note onto the label.
const PROGRESS_AFTER: Duration = Duration::from_secs(1);

/// Finds the person's node for a repository's Knip, never inside the repository.
pub type NodeLookup = Arc<dyn Fn(&Path) -> Option<Tool> + Send + Sync>;

/// What the engine connects, where its data lives, the clock it reads, how it polls GitHub
/// Actions, and where it finds node for Knip.
#[derive(Clone)]
pub struct EngineConfig {
    pub data: DataDir,
    /// `--repo <path>`; `None` for the terrarium `repo.active` names.
    pub repo: Option<PathBuf>,
    pub clock: Arc<dyn Clock>,
    pub progress_after: Duration,
    /// `None` polls nothing: no keychain is read and no request is made (N-09). The app gives
    /// one; tests give stand-ins, so none reaches the person's keychain or the network.
    pub ci: Option<CiConfig>,
    pub node: NodeLookup,
}

impl EngineConfig {
    pub fn new(data: DataDir, repo: Option<PathBuf>) -> EngineConfig {
        EngineConfig {
            data,
            repo,
            clock: Arc::new(SystemClock),
            progress_after: PROGRESS_AFTER,
            ci: None,
            node: Arc::new(|repo: &Path| find_node(Some(repo))),
        }
    }
}

/// Why nothing could be connected. Its wording is the app's.
#[derive(Debug)]
pub enum ConnectError {
    Store(StoreError),
    /// The path given is not a repository this build reads, or names no default branch.
    Git(GitError),
    /// The path given cannot be made absolute.
    Path(std::io::Error),
    /// The OS gave no random bytes for a new terrarium's salt.
    Salt(String),
    /// The folder given to relink a terrarium holds another repository.
    OtherRepository,
    /// The worker thread could not start.
    Thread(std::io::Error),
    /// Connecting panicked (I-16).
    Halted,
}

impl From<StoreError> for ConnectError {
    fn from(err: StoreError) -> ConnectError {
        ConnectError::Store(err)
    }
}

impl From<GitError> for ConnectError {
    fn from(err: GitError) -> ConnectError {
        ConnectError::Git(err)
    }
}

/// What the strip starts from, before the first refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    Connected {
        /// Its name, where it lives, and its install salt, which is cosmetic only (I-2): the
        /// composer is built with it.
        terrarium: Box<Terrarium>,
        /// The cached state's scene from the last session, if it is still valid: real truth,
        /// shown while the history is read.
        scene: Option<SceneInput>,
    },
    /// No repository is connected.
    NotConnected,
}

impl Live {
    /// Connects what `config` names, or `None` when it names nothing (a plain run with no
    /// repository connected).
    pub fn connect(config: &EngineConfig) -> Result<Option<Live>, ConnectError> {
        let mut store = Store::open(&config.data)?;
        let now = config.clock.wall_secs();
        let (terrarium, verified) = match &config.repo {
            None => {
                let Some(id) = store.setting::<i64>(SETTING_REPO_ACTIVE)? else {
                    return Ok(None);
                };
                let Some(terrarium) = store
                    .terrarium(TerrariumId::new(id))?
                    .filter(|terrarium| terrarium.active)
                else {
                    return Ok(None);
                };
                (terrarium, false)
            }
            Some(path) => (connect_repo(&mut store, path, now)?, true),
        };
        // The note waits for a snapshot to carry it, and the version is recorded only then. A
        // first install has nothing to note.
        let current = SIM_VERSION.0.to_string();
        let last = store.install_value(InstallKey::LastSimVersion)?;
        if last.is_none() {
            store.set_install_value(InstallKey::LastSimVersion, &current)?;
        }
        let news = News {
            sim_updated: last.is_some_and(|last| last != current),
            history_rewritten: false,
        };
        let adapters = stored_adapters(&store)?;
        let status_dir = config.data.local_tests_dir();
        std::fs::create_dir_all(&status_dir).map_err(StoreError::Io)?;
        let stored = store.observations(terrarium.id, None)?;
        let mut newest = BTreeMap::new();
        let mut ci = CiState {
            on: config.ci.is_some(),
            ..CiState::default()
        };
        let mut observations = Vec::with_capacity(stored.len());
        for row in &stored {
            note_newest(&mut newest, row);
            note_ci_newest(&mut ci.newest, row);
            observations.push(row.to_observation()?);
        }
        let measured = measured_at_connect(&stored, adapters.enabled);
        let (base, cached_tick) = match cached_base(&store, &terrarium)? {
            Some((base, tick)) => (Some(base), Some(tick)),
            None => (None, None),
        };
        let showing = Showing::after(base.as_ref().map(|base| base.events.keys()));
        let remembered = Remembered::read(&store)?;
        let knip_runs = knip_run(&adapters, &terrarium).is_some();
        Ok(Some(Live {
            status_file: config.data.local_tests(&terrarium.seed),
            patterns: patterns(&adapters, knip_runs),
            store,
            terrarium,
            clock: Arc::clone(&config.clock),
            progress_after: config.progress_after,
            adapters,
            status_dir,
            news,
            verified,
            history: None,
            observations,
            newest,
            metrics: Metrics::new(),
            measured,
            knip_weighed: None,
            found: Found::default(),
            files: FileCache::default(),
            base,
            cached_tick,
            view: None,
            shown: None,
            showing,
            remembered,
            stored_seen: None,
            now,
            watch: None,
            head: None,
            node: Arc::clone(&config.node),
            ci,
            knip: KnipState {
                status: if knip_runs {
                    KnipStatus::Waiting
                } else {
                    KnipStatus::Off
                },
                ..KnipState::default()
            },
        }))
    }

    /// What the strip starts from.
    pub fn start(&self) -> Start {
        Start::Connected {
            terrarium: Box::new(self.terrarium.clone()),
            scene: self.base.as_ref().map(|base| project_showing(base, None)),
        }
    }
}

/// Notes `row` as its CI source's newest stored row, if it is: at connect, the largest
/// (`recorded_at`, row id).
fn note_ci_newest(newest: &mut BTreeMap<String, CiNewest>, row: &StoredObservation) {
    let Some((branch, row)) = ci_newest_of(row) else {
        return;
    };
    if newest.get(&branch).is_none_or(|known| known.at < row.at) {
        newest.insert(branch, row);
    }
}

/// Creates the terrarium for the repository at `path`, or relinks the one with its seed, and
/// makes it the active one.
fn connect_repo(store: &mut Store, path: &Path, now: i64) -> Result<Terrarium, ConnectError> {
    let path = lifecycle::absolute(path)?;
    let (default_branch, identity) = GitRepo::open(&path)?.default_identity()?;
    let seed = identity.seed;
    let location = lifecycle::location(&path, default_branch);
    let id = if let Some(known) = store.terrarium_by_seed(&seed)? {
        store.activate(known.id, &location)?;
        known.id
    } else {
        store.create_terrarium(&NewTerrarium {
            seed,
            repo: location,
            install_salt: lifecycle::new_salt()?,
            connected_at: now,
        })?
    };
    store.set_setting(SETTING_REPO_ACTIVE, &id.get())?;
    Ok(store
        .terrarium(id)?
        .ok_or(StoreError::NoSuchTerrarium(id))?)
}

/// The cached base state, when it was written under this `SIM_VERSION` for this terrarium's
/// seed and default branch and still decodes; and the tick it was cached at.
fn cached_base(
    store: &Store,
    terrarium: &Terrarium,
) -> Result<Option<(SimState, Tick)>, StoreError> {
    let Some(cache) = store.sim_cache(terrarium.id)? else {
        return Ok(None);
    };
    if cache.sim_version != SIM_VERSION {
        return Ok(None);
    }
    Ok(postcard::from_bytes::<SimState>(&cache.state)
        .ok()
        .filter(|state| {
            state.basis.sim_version == SIM_VERSION
                && state.basis.seed == terrarium.seed
                && state.basis.default_branch == terrarium.repo.default_branch
                && state.next_tick == cache.tick
        })
        .map(|state| (state, cache.tick)))
}

/// The tip the source was last measured at, and the metrics recorded there: the newest
/// default-tip row of either metric names the tip, and each metric counts as measured there
/// only when it has a row at it, so one switched on since is measured again.
fn measured_at_connect(stored: &[StoredObservation], on: Enabled) -> Option<(CommitId, Enabled)> {
    let metric = |row: &&StoredObservation| {
        let record = &row.record;
        matches!(
            record.kind,
            ObservationKind::Complexity | ObservationKind::TodoDensity
        ) && record.adapter_version == METRICS_VERSION
            && record.head.at_default_tip
    };
    let tip = stored
        .iter()
        .filter(metric)
        .max_by_key(|row| (row.record.recorded_at, row.id.get()))?
        .record
        .head
        .commit
        .clone();
    let has = |kind| {
        stored
            .iter()
            .filter(metric)
            .any(|row| row.record.kind == kind && row.record.head.commit == tip)
    };
    let under = Enabled {
        complexity: has(ObservationKind::Complexity),
        todo_density: has(ObservationKind::TodoDensity),
        ..on
    };
    Some((tip, under))
}

/// Notes `row` as its format's newest stored observation, if it is: at connect, the largest
/// (`recorded_at`, row id).
fn note_newest(newest: &mut BTreeMap<Format, Newest>, row: &StoredObservation) {
    let Some(format) = observe::producer(&row.record) else {
        return;
    };
    let at = (row.record.recorded_at, row.id.get());
    if newest.get(&format).is_none_or(|known| known.at < at) {
        newest.insert(
            format,
            Newest {
                at,
                hash: row.record.content_hash,
            },
        );
    }
}
