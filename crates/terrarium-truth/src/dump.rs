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
use crate::scene::{Energy, GrowthStage, LifeStage, PlantState, SceneInput};
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
    /// The living things and landmarks at the same closes.
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

/// The vegetation, the residents and the landmarks at one close, counted as the composer is
/// shown them.
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
    /// Residents by stage; one dying shows for a day, so `died` counts them instead.
    pub eggs: u32,
    pub juveniles: u32,
    pub pupae: u32,
    pub adults: u32,
    /// Residents by energy.
    pub low: u32,
    pub mid: u32,
    pub high: u32,
    /// Running totals: residents that arrived, eggs laid, deaths of old age, and leavings.
    pub arrived: u32,
    pub laid: u32,
    pub died: u32,
    pub left: u32,
    pub monuments: u32,
    pub fossils: u32,
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
    eggs,
    juveniles,
    pupae,
    adults,
    low,
    mid,
    high,
    arrived,
    laid,
    died,
    left,
    monuments,
    fossils,
});

impl Census {
    pub fn of(state: &SimState) -> Census {
        let scene = project(state);
        let looks = |look| count(scene.plants.iter().filter(|p| p.state == look));
        let stages = |stage| count(scene.plants.iter().filter(|p| p.stage == stage));
        let lives = |stage| count(scene.residents.iter().filter(|r| r.stage == stage));
        let energies = |energy| count(scene.residents.iter().filter(|r| r.energy == energy));
        let fauna = &state.fauna;
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
            eggs: lives(LifeStage::Egg),
            juveniles: lives(LifeStage::Juvenile),
            pupae: lives(LifeStage::Pupa),
            adults: lives(LifeStage::Adult),
            low: energies(Energy::Low),
            mid: energies(Energy::Mid),
            high: energies(Energy::High),
            arrived: fauna.arrived,
            laid: fauna.laid,
            died: fauna.died,
            left: fauna.left,
            monuments: count(scene.monuments.iter()),
            fossils: count(scene.fossils.iter()),
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
