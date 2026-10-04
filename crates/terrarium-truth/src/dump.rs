//! The replay dump: what `repo-terrarium replay --dump` prints and the golden files hold
//! (I-13).
//!
//! Its canonical text begins with `sim_version`, so a golden file's header names the version
//! that wrote it. Beside the final state it carries the climate of every seventh UTC day, so a
//! change anywhere along the history shows. It names no paths, only what truth reads.

use serde::{Deserialize, Serialize};

use crate::canon::canonical_struct;
use crate::climate::{ClimateReport, DailyClimate};
use crate::domain::Habitat;
use crate::identity::{RepoSeed, SimVersion};
use crate::project::project;
use crate::replay::{Prepared, ReplayError, SimState, first_close};
use crate::scene::SceneInput;
use crate::time::{Tick, TickGrid};

/// A week of ticks between samples.
const SAMPLE_TICKS: u32 = 7 * 24;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayDump {
    pub sim_version: SimVersion,
    pub seed: RepoSeed,
    pub default_branch: String,
    pub grid: TickGrid,
    /// The last tick applied.
    pub through: Tick,
    /// The record left by the first close and by every seventh close after it.
    pub samples: Vec<DailyClimate>,
    pub report: ClimateReport,
    pub habitat: Habitat,
    pub state: SimState,
    pub scene: SceneInput,
}

canonical_struct!(ReplayDump {
    sim_version,
    seed,
    default_branch,
    grid,
    through,
    samples,
    report,
    habitat,
    state,
    scene,
});

/// Replays `prepared` from tick 0 through `through`.
pub fn dump(prepared: &Prepared<'_>, through: Tick) -> Result<ReplayDump, ReplayError> {
    let basis = prepared.basis().clone();
    let mut state = SimState::new(basis.clone());
    let mut samples = Vec::new();
    let mut close = Some(first_close(basis.grid));
    while let Some(tick) = close.filter(|&tick| tick <= through) {
        state.advance_through(prepared, tick)?;
        samples.extend(state.daily.last().cloned());
        close = tick.hours().checked_add(SAMPLE_TICKS).map(Tick::new);
    }
    state.advance_through(prepared, through)?;
    Ok(ReplayDump {
        sim_version: basis.sim_version,
        seed: basis.seed,
        default_branch: basis.default_branch,
        grid: basis.grid,
        through,
        samples,
        report: state.report(),
        habitat: state.habitat(),
        scene: project(&state),
        state,
    })
}
