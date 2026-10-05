//! The replay dump: what `repo-terrarium replay --dump` prints and the golden files hold
//! (I-13).
//!
//! Its canonical text begins with `sim_version`, so a golden file's header names the version
//! that wrote it; the seed, grid and default branch are in the state's basis. Beside the final
//! state it carries the climate and a census of the vegetation every seventh UTC day, so a
//! change anywhere along the history shows, even one that later growth covers over. It names no
//! paths, only what truth reads.

use serde::{Deserialize, Serialize};

use crate::canon::canonical_struct;
use crate::climate::{ClimateReport, DailyClimate};
use crate::domain::Habitat;
use crate::identity::SimVersion;
use crate::project::project;
use crate::replay::{Prepared, ReplayError, SimState, first_close};
use crate::scene::{GrowthStage, PlantState, SceneInput};
use crate::time::Tick;

/// A week of ticks between samples.
const SAMPLE_TICKS: u32 = 7 * 24;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayDump {
    pub sim_version: SimVersion,
    /// The last tick applied.
    pub through: Tick,
    /// The record left by the first close and by every seventh close after it.
    pub samples: Vec<DailyClimate>,
    /// The vegetation at the same closes.
    pub census: Vec<Census>,
    pub report: ClimateReport,
    pub habitat: Habitat,
    pub state: SimState,
    pub scene: SceneInput,
}

canonical_struct!(ReplayDump {
    sim_version,
    through,
    samples,
    census,
    report,
    habitat,
    state,
    scene,
});

/// The vegetation at one close, counted as the composer is shown it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Census {
    pub tick: Tick,
    /// Plants by look.
    pub thriving: u32,
    pub pale: u32,
    pub wilting: u32,
    pub dormant: u32,
    pub dead: u32,
    /// Plants by stage.
    pub seedlings: u32,
    pub young: u32,
    pub mature: u32,
    pub weeds: u8,
    pub litter: u8,
    pub fungus: u8,
    /// How many plants have ever germinated.
    pub germinated: u32,
}

canonical_struct!(Census {
    tick,
    thriving,
    pale,
    wilting,
    dormant,
    dead,
    seedlings,
    young,
    mature,
    weeds,
    litter,
    fungus,
    germinated,
});

impl Census {
    pub fn of(state: &SimState) -> Census {
        let scene = project(state);
        let looks = |look| count(scene.plants.iter().filter(|p| p.state == look));
        let stages = |stage| count(scene.plants.iter().filter(|p| p.stage == stage));
        Census {
            tick: state.now(),
            thriving: looks(PlantState::Thriving),
            pale: looks(PlantState::Pale),
            wilting: looks(PlantState::Wilting),
            dormant: looks(PlantState::Dormant),
            dead: looks(PlantState::Dead),
            seedlings: stages(GrowthStage::Seedling),
            young: stages(GrowthStage::Young),
            mature: stages(GrowthStage::Mature),
            weeds: scene.weeds,
            litter: scene.litter,
            fungus: scene.fungus,
            germinated: state.vegetation.next_id.saturating_sub(1),
        }
    }
}

fn count<T>(items: impl Iterator<Item = T>) -> u32 {
    u32::try_from(items.count()).unwrap_or(u32::MAX)
}

/// Replays `prepared` from tick 0 through `through`.
pub fn dump(prepared: &Prepared<'_>, through: Tick) -> Result<ReplayDump, ReplayError> {
    let basis = prepared.basis().clone();
    let mut state = SimState::new(basis.clone());
    let mut samples = Vec::new();
    let mut census = Vec::new();
    let mut close = Some(first_close(basis.grid));
    while let Some(tick) = close.filter(|&tick| tick <= through) {
        state.advance_through(prepared, tick)?;
        samples.extend(state.daily.last().cloned());
        census.push(Census::of(&state));
        close = tick.hours().checked_add(SAMPLE_TICKS).map(Tick::new);
    }
    state.advance_through(prepared, through)?;
    Ok(ReplayDump {
        sim_version: basis.sim_version,
        through,
        samples,
        census,
        report: state.report(),
        habitat: state.habitat(),
        scene: project(&state),
        state,
    })
}
