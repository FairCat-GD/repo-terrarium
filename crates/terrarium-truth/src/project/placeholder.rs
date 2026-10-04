//! Stand-in vegetation, read from health alone, so a replayed terrarium has plants before the
//! vegetation sim grows its own.
//!
//! It is depiction scaffolding: `project` derives it on every call, and none of it is kept in
//! `SimState`, so replacing it changes no stored state. Eight mature plants of fixed species
//! share one look: dormant or dead with the terrarium, otherwise thriving from health 0.60,
//! pale or wilting (whichever of light and soil limits) from 0.35, and dead below.

use crate::domain::PresentationState;
use crate::fixed::Q16;
use crate::scene::{GrowthStage, PlantSpecies, PlantState, PlantView};
use crate::tuning::{ALIVE_FROM, NEUTRAL, THRIVING_FROM};

const SPECIES: [PlantSpecies; 8] = [
    PlantSpecies::Oak,
    PlantSpecies::Birch,
    PlantSpecies::Fern,
    PlantSpecies::TallGrass,
    PlantSpecies::Reed,
    PlantSpecies::MossMound,
    PlantSpecies::Clover,
    PlantSpecies::Thistle,
];

pub fn plants(
    state: PresentationState,
    health: Q16,
    light: Option<Q16>,
    soil: Option<Q16>,
) -> Vec<PlantView> {
    let failing = if light.unwrap_or(NEUTRAL) < soil.unwrap_or(NEUTRAL) {
        PlantState::Pale
    } else {
        PlantState::Wilting
    };
    let look = match state {
        PresentationState::Dormant => PlantState::Dormant,
        PresentationState::Dead => PlantState::Dead,
        _ if health >= THRIVING_FROM => PlantState::Thriving,
        _ if health >= ALIVE_FROM => failing,
        _ => PlantState::Dead,
    };
    (0_u16..)
        .zip(SPECIES)
        .map(|(order, species)| PlantView {
            id: u32::from(order) + 1,
            species,
            stage: GrowthStage::Mature,
            state: look,
            bloom: false,
            scorched: false,
            order,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(state: PresentationState, health: i32, light: i32, soil: i32) -> PlantState {
        let q = |percent| Some(Q16::from_ratio(percent, 100));
        let plants = plants(state, Q16::from_ratio(health, 100), q(light), q(soil));
        assert_eq!(plants.len(), SPECIES.len());
        plants[0].state
    }

    #[test]
    fn the_look_follows_the_presentation_then_health() {
        use PresentationState::{Dead, Dormant, Struggling, Thriving};
        assert_eq!(look(Dormant, 90, 90, 90), PlantState::Dormant);
        assert_eq!(look(Dead, 90, 90, 90), PlantState::Dead);
        assert_eq!(look(Thriving, 60, 60, 80), PlantState::Thriving);
        assert_eq!(look(Struggling, 40, 40, 80), PlantState::Pale);
        assert_eq!(look(Struggling, 40, 80, 40), PlantState::Wilting);
        assert_eq!(look(Struggling, 34, 34, 80), PlantState::Dead);
    }
}
