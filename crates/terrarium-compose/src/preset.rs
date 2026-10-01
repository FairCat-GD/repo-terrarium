//! Fictional scenes, one per key state, drawn by the composer for `cargo xtask render --preset`
//! and pinned by exact snapshots. Every preset grows from one seeded pool of eight species, as
//! truth's would: two far, three mid and three near.
//!
//! Only the plants and the presentation state reach the picture so far. The weather, ground
//! cover and glass counts are what such a terrarium would hold, ready for the world to draw.

use terrarium_truth::scene::{
    GrowthStage, Light, PlantSpecies, PlantState, PlantView, Presentation, SceneInput, Weather,
    Wind,
};
use terrarium_truth::{PresentationState, Tick};

use crate::catalog::Art;
use crate::composer::Composer;
use crate::frame::Frame;
use crate::salt::InstallSalt;
use crate::tween::Millis;
use crate::variants::ArtTable;

/// The salt every preset and snapshot is drawn with.
pub const SALT: InstallSalt = InstallSalt::new(*b"terrarium-preset");

pub const NAMES: [&str; 7] = [
    "thriving",
    "struggling-low-light",
    "struggling-poor-soil",
    "mature",
    "dormant",
    "dead",
    "recovering",
];

use GrowthStage::{Mature as M, Seedling as S, Young as Y};
use PlantSpecies::{Clover, Fern, Foxglove, MossMound, Oak, Reed, Rowan, Thistle};
use PlantState::{Dead, Dormant, Pale, Thriving, Wilting};

type Plant = (PlantSpecies, GrowthStage, PlantState, bool);

const THRIVING: &[Plant] = &[
    (Oak, M, Thriving, false),
    (Fern, M, Thriving, false),
    (MossMound, M, Thriving, false),
    (Rowan, M, Thriving, false),
    (Foxglove, M, Thriving, true),
    (Clover, M, Thriving, true),
    (Reed, M, Thriving, false),
    (Thistle, M, Thriving, false),
    (Oak, Y, Thriving, false),
    (Fern, Y, Thriving, false),
    (Clover, Y, Thriving, false),
    (Rowan, Y, Thriving, false),
    (Reed, Y, Thriving, false),
    (MossMound, Y, Thriving, false),
    (Foxglove, S, Thriving, false),
    (Thistle, Y, Thriving, false),
    (Clover, S, Thriving, false),
    (MossMound, S, Thriving, false),
];

const LOW_LIGHT: &[Plant] = &[
    (Oak, M, Pale, false),
    (Fern, M, Pale, false),
    (MossMound, M, Pale, false),
    (Rowan, M, Pale, false),
    (Foxglove, M, Pale, false),
    (Clover, M, Pale, false),
    (Reed, M, Pale, false),
    (Thistle, Y, Pale, false),
    (Oak, Y, Pale, false),
    (Fern, Y, Pale, false),
    (Clover, Y, Thriving, false),
    (Reed, Y, Pale, false),
    (MossMound, Y, Thriving, false),
    (Foxglove, S, Pale, false),
    (Thistle, S, Pale, false),
    (Clover, S, Pale, false),
];

const POOR_SOIL: &[Plant] = &[
    (Oak, M, Wilting, false),
    (Fern, M, Wilting, false),
    (MossMound, M, Wilting, false),
    (Rowan, M, Wilting, false),
    (Foxglove, M, Dead, false),
    (Clover, M, Wilting, false),
    (Reed, M, Wilting, false),
    (Thistle, Y, Wilting, false),
    (Oak, Y, Dead, false),
    (Fern, Y, Wilting, false),
    (Clover, Y, Thriving, false),
    (Reed, Y, Wilting, false),
    (MossMound, Y, Wilting, false),
    (Foxglove, S, Wilting, false),
    (Thistle, S, Thriving, false),
    (Clover, S, Wilting, false),
];

const MATURE: &[Plant] = &[
    (Oak, M, Thriving, false),
    (Fern, M, Thriving, false),
    (MossMound, M, Thriving, false),
    (Rowan, M, Thriving, false),
    (Foxglove, M, Thriving, false),
    (Clover, M, Thriving, false),
    (Reed, M, Thriving, false),
    (Thistle, M, Thriving, false),
    (Oak, M, Thriving, false),
    (Fern, M, Thriving, false),
    (Clover, M, Thriving, false),
    (Rowan, M, Thriving, false),
    (Reed, M, Thriving, false),
    (MossMound, M, Thriving, false),
    (Foxglove, M, Thriving, false),
    (Thistle, M, Thriving, false),
    (Clover, Y, Thriving, false),
    (Reed, Y, Thriving, false),
    (Oak, Y, Thriving, false),
    (Thistle, Y, Thriving, false),
];

const DORMANT: &[Plant] = &[
    (Oak, M, Dormant, false),
    (Fern, M, Dormant, false),
    (MossMound, M, Dormant, false),
    (Rowan, M, Dormant, false),
    (Foxglove, M, Dormant, false),
    (Clover, M, Dormant, false),
    (Reed, M, Dormant, false),
    (Thistle, M, Dormant, false),
    (Oak, Y, Dormant, false),
    (Fern, Y, Dormant, false),
    (Clover, Y, Dormant, false),
    (Rowan, Y, Dormant, false),
    (Reed, Y, Dormant, false),
    (MossMound, Y, Dormant, false),
    (Foxglove, S, Dormant, false),
    (Thistle, Y, Dormant, false),
];

const DEAD: &[Plant] = &[
    (Oak, M, Dead, false),
    (Fern, M, Dead, false),
    (MossMound, M, Dead, false),
    (Rowan, Y, Dead, false),
    (Reed, Y, Dead, false),
    (Thistle, M, Dead, false),
];

const RECOVERING: &[Plant] = &[
    (Oak, M, Dead, false),
    (Fern, M, Dead, false),
    (Reed, M, Dead, false),
    (MossMound, M, Dead, false),
    (Rowan, Y, Thriving, false),
    (Foxglove, Y, Thriving, false),
    (Clover, Y, Thriving, false),
    (Oak, S, Thriving, false),
    (Fern, S, Thriving, false),
    (Thistle, S, Thriving, false),
    (Clover, S, Thriving, false),
    (MossMound, S, Thriving, false),
    (Reed, S, Thriving, false),
    (Foxglove, S, Thriving, false),
];

/// What a preset's terrarium holds beyond its plants.
struct World {
    state: PresentationState,
    plants: &'static [Plant],
    weather: Weather,
    wind: Wind,
    /// Weeds, litter and fungus.
    cover: (u8, u8, u8),
    /// Condensation and dust.
    glass: (u8, u8),
}

fn world(name: &str) -> Option<World> {
    let world = |state, plants, weather, wind, cover, glass| World {
        state,
        plants,
        weather,
        wind,
        cover,
        glass,
    };
    Some(match name {
        "thriving" => world(
            PresentationState::Thriving,
            THRIVING,
            Weather::Clear,
            Wind::Breezy,
            (0, 1, 0),
            (0, 0),
        ),
        "struggling-low-light" => world(
            PresentationState::Struggling,
            LOW_LIGHT,
            Weather::Overcast,
            Wind::Calm,
            (1, 2, 0),
            (0, 0),
        ),
        "struggling-poor-soil" => world(
            PresentationState::Struggling,
            POOR_SOIL,
            Weather::Overcast,
            Wind::Calm,
            (6, 4, 2),
            (0, 0),
        ),
        "mature" => world(
            PresentationState::Mature,
            MATURE,
            Weather::Clear,
            Wind::Calm,
            (0, 2, 1),
            (0, 0),
        ),
        "dormant" => world(
            PresentationState::Dormant,
            DORMANT,
            Weather::Clear,
            Wind::Calm,
            (0, 2, 0),
            (1, 0),
        ),
        "dead" => world(
            PresentationState::Dead,
            DEAD,
            Weather::Haze,
            Wind::Calm,
            (2, 8, 4),
            (0, 1),
        ),
        "recovering" => world(
            PresentationState::Recovering,
            RECOVERING,
            Weather::Clear,
            Wind::Breezy,
            (1, 4, 1),
            (0, 0),
        ),
        _ => return None,
    })
}

/// The named preset's scene, or `None` for a name not in [`NAMES`].
pub fn scene(name: &str) -> Option<SceneInput> {
    let world = world(name)?;
    let plants = (0_u16..)
        .zip(world.plants)
        .map(|(order, &(species, stage, state, bloom))| PlantView {
            id: 100 + u32::from(order) * 7,
            species,
            stage,
            state,
            bloom,
            scorched: false,
            order,
        })
        .collect();
    let (weeds, litter, fungus) = world.cover;
    let (condensation, dust) = world.glass;
    Some(SceneInput {
        presentation: Presentation {
            state: world.state,
            since_tick: Tick::new(24 * 90),
        },
        plants,
        weeds,
        litter,
        fungus,
        residents: Vec::new(),
        monuments: Vec::new(),
        fossils: Vec::new(),
        ground_marks: Vec::new(),
        weather: world.weather,
        light: Light::Neutral,
        wind: world.wind,
        condensation,
        dust,
        event: None,
    })
}

/// The named preset composed from nothing and settled, by day, with its label reading `label`.
pub fn frame(art: &Art, table: &ArtTable, name: &str, label: &str) -> Option<Frame> {
    let input = scene(name)?;
    let mut composer = Composer::new(table, SALT);
    composer.update(&input, Millis(0));
    Some(composer.frame(art, table, label, 0, composer.settled_at()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_a_valid_scene_with_its_state() {
        for name in NAMES {
            let scene = scene(name).unwrap();
            assert_eq!(scene.validate(), Ok(()), "{name}");
            let state = scene.presentation.state.name().to_ascii_lowercase();
            assert!(name.starts_with(&state), "{name} is {state}");
        }
        assert_eq!(scene("lush"), None);
    }

    #[test]
    fn the_presets_grow_from_one_pool_of_eight_species() {
        let mut species: Vec<PlantSpecies> = NAMES
            .iter()
            .flat_map(|name| scene(name).unwrap().plants)
            .map(|plant| plant.species)
            .collect();
        species.sort();
        species.dedup();
        assert_eq!(species.len(), 8);
    }

    #[test]
    fn every_preset_draws_plants_and_scenery() {
        let art = Art::embedded();
        let table = ArtTable::new(&art.catalog).unwrap();
        for name in NAMES {
            let frame = frame(art, &table, name, "repo-terrarium").unwrap();
            assert!(frame.draws.len() > 20, "{name}");
            assert!(
                frame
                    .draws
                    .windows(2)
                    .all(|pair| pair[0].layer <= pair[1].layer)
            );
        }
    }
}
