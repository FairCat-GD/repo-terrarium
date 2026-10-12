//! What the soil panel and the hover labels need beside the climate report (`OverlayInput`): the habitat, when things happened, and the files that weigh in each reading.
//! Moments are UTC seconds; the app words them. Nothing here moves with the hour on its own (age
//! is the root's date, not a count), so an hourly refresh that changes nothing sends nothing.

use std::collections::{BTreeMap, BTreeSet};

use terrarium_adapters::discovery::Enabled;
use terrarium_store::{Contributor, StoreError};
use terrarium_truth::climate::readings::Latest;
use terrarium_truth::domain::ObservationHead;
use terrarium_truth::landmarks::burying;
use terrarium_truth::{
    CommitFacts, Observation, ObservationKind, PresentationState, Q16, SimState, TickGrid,
    observation_tick,
};

/// The facts the panel and the hover labels word.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Details {
    /// The default tip's text lines.
    pub terrain: u64,
    /// When the default branch's first commit was made, from which age counts.
    pub root: Option<i64>,
    /// When the terrarium went dormant, while it is.
    pub dormant_since: Option<i64>,
    /// Each standing stone, oldest first.
    pub monuments: Vec<Raised>,
    /// Each fossil, oldest first.
    pub fossils: Vec<Buried>,
    pub files: Files,
    /// The newest coverage report taken on HEAD's branch away from the default tip, when it is
    /// newer than the one light reads: shown, never applied.
    pub branch_reading: Option<BranchReading>,
    /// Which reports the last look at the worktree found.
    pub found: Found,
}

/// A standing stone and when it was raised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raised {
    pub id: u32,
    pub at: i64,
}

/// A fossil, when its directory was deleted, and the directory, while its commit is in the
/// history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Buried {
    pub id: u32,
    pub at: i64,
    pub path: Option<String>,
}

/// The files behind each reading the panel opens, heaviest first, at most 20 each.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Files {
    /// Uncovered lines, from the coverage report light reads.
    pub light: Vec<Contributor>,
    /// Failing tests, from the newest run of the weather's source.
    pub weather: Vec<Contributor>,
    pub dead_code: Vec<Contributor>,
    pub complexity: Vec<Contributor>,
    pub todo_density: Vec<Contributor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchReading {
    pub branch: String,
    /// `None` when the report could not tell.
    pub value: Option<Q16>,
}

/// Which reports the worktree has, whether the default tip has source code the metrics read,
/// and which adapters the settings have on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub coverage: bool,
    pub test_report: bool,
    /// A Knip report.
    pub dead_code: bool,
    /// Whether the default tip, when it was last measured in this session, held a source file
    /// in a language the metrics read; `None` before it was.
    pub source_code: Option<bool>,
    pub enabled: Enabled,
}

/// What the details are read from.
pub(crate) struct Sources<'a> {
    pub state: &'a SimState,
    pub grid: TickGrid,
    pub commits: &'a [CommitFacts],
    pub observations: &'a [Observation],
    /// Where HEAD is now.
    pub head: &'a ObservationHead,
    pub found: Found,
}

/// Each observation's contributors as the store gave them, kept while the panel needs them.
#[derive(Debug, Default)]
pub(crate) struct FileCache(BTreeMap<i64, Vec<Contributor>>);

impl FileCache {
    /// The contributors of observation `id`, read from `read` the first time.
    fn get(
        &mut self,
        id: i64,
        read: &mut dyn FnMut(i64) -> Result<Vec<Contributor>, StoreError>,
    ) -> Result<Vec<Contributor>, StoreError> {
        if let Some(files) = self.0.get(&id) {
            return Ok(files.clone());
        }
        let files = read(id)?;
        self.0.insert(id, files.clone());
        Ok(files)
    }
}

pub(crate) fn details(
    sources: &Sources<'_>,
    cache: &mut FileCache,
    read: &mut dyn FnMut(i64) -> Result<Vec<Contributor>, StoreError>,
) -> Result<Details, StoreError> {
    let Sources {
        state, grid, head, ..
    } = *sources;
    let at = |tick| grid.start_secs(tick);
    let (presentation, since) = state.timeline.presentation();
    let readings = &state.readings;
    let rows = [
        report_row(
            sources,
            ObservationKind::Coverage,
            readings.light_from.as_ref(),
        ),
        weather_row(sources),
        report_row(
            sources,
            ObservationKind::DeadCode,
            readings.dead_code.as_ref(),
        ),
        report_row(
            sources,
            ObservationKind::Complexity,
            readings.complexity.as_ref(),
        ),
        report_row(
            sources,
            ObservationKind::TodoDensity,
            readings.todo_density.as_ref(),
        ),
    ];
    let wanted: BTreeSet<i64> = rows.iter().flatten().copied().collect();
    cache.0.retain(|id, _| wanted.contains(id));
    let mut read_files = |row: Option<i64>| row.map_or(Ok(Vec::new()), |id| cache.get(id, read));
    let [light, weather, dead_code, complexity, todo_density] = rows;
    let files = Files {
        light: read_files(light)?,
        weather: read_files(weather)?,
        dead_code: read_files(dead_code)?,
        complexity: read_files(complexity)?,
        todo_density: read_files(todo_density)?,
    };
    // Only a fossil's commit is looked for, and only while there are fossils.
    let fossils = &state.landmarks.fossils;
    let commits = if fossils.is_empty() {
        &[][..]
    } else {
        sources.commits
    };
    let fossil_commits: BTreeMap<_, _> = commits
        .iter()
        .filter(|commit| fossils.iter().any(|f| f.commit == commit.id))
        .map(|commit| (&commit.id, commit))
        .collect();
    Ok(Details {
        terrain: state.habitat.terrain_lines(),
        root: state.habitat.root.map(at),
        dormant_since: (presentation == PresentationState::Dormant).then(|| at(since)),
        monuments: state
            .landmarks
            .monuments
            .iter()
            .map(|monument| Raised {
                id: monument.id,
                at: at(monument.founded),
            })
            .collect(),
        fossils: state
            .landmarks
            .fossils
            .iter()
            .map(|fossil| Buried {
                id: fossil.id,
                at: at(fossil.tick),
                path: fossil_commits
                    .get(&fossil.commit)
                    .and_then(|commit| burying(&commit.deleted_dirs))
                    .map(|dir| dir.path.clone()),
            })
            .collect(),
        files,
        branch_reading: branch_reading(sources, readings.light_from.as_ref(), head),
        found: sources.found,
    })
}

/// The row of the report `latest` names: the held observation of `kind` with its head and time.
fn report_row(
    sources: &Sources<'_>,
    kind: ObservationKind,
    latest: Option<&Latest>,
) -> Option<i64> {
    let latest = latest?;
    sources
        .observations
        .iter()
        .find(|o| {
            o.value.kind() == kind
                && o.recorded_at == latest.recorded_at
                && o.head.commit == latest.head
        })
        .map(|o| o.id)
}

/// The newest applied run of the weather's source in effect.
fn weather_row(sources: &Sources<'_>) -> Option<i64> {
    let source = &sources.state.weather.in_effect.as_ref()?.source;
    sources
        .observations
        .iter()
        .filter(|o| o.value.source() == Some(source) && applied(sources, o))
        .max_by_key(|o| (o.recorded_at, o.id))
        .map(|o| o.id)
}

fn branch_reading(
    sources: &Sources<'_>,
    light: Option<&Latest>,
    head: &ObservationHead,
) -> Option<BranchReading> {
    let branch = head.branch.as_deref().filter(|_| !head.at_default_tip)?;
    let newest = sources
        .observations
        .iter()
        .filter(|o| {
            o.value.kind() == ObservationKind::Coverage
                && !o.head.at_default_tip
                && o.head.branch.as_deref() == Some(branch)
                && applied(sources, o)
        })
        .max_by_key(|o| (o.recorded_at, o.id))?;
    light
        .is_none_or(|light| newest.recorded_at > light.recorded_at)
        .then(|| BranchReading {
            branch: branch.to_owned(),
            value: newest.normalized,
        })
}

/// Whether replay has reached the hour `o` was recorded in.
fn applied(sources: &Sources<'_>, o: &Observation) -> bool {
    observation_tick(sources.grid, o.recorded_at).is_some_and(|t| t < sources.state.next_tick)
}
