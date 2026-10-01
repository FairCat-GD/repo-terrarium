//! Fictional scenes, one per key state, drawn by the composer for `cargo xtask render --preset`
//! and pinned by exact snapshots, by day and by night. Every preset grows from one seeded pool
//! of eight species, as truth's would: two far, three mid and three near. Each holds what such
//! a terrarium would: its weather and light, ground cover, glass, monuments, fossils and
//! residents.

use terrarium_truth::scene::{
    Energy, FossilForm, FossilView, GrowthStage, LifeStage, Light, MonumentAge, MonumentView,
    PlantSpecies, PlantState, PlantView, Presentation, ResidentSpecies, ResidentView, SceneInput,
    Weather, Wind,
};
use terrarium_truth::{PresentationState, Tick};

use crate::catalog::Art;
use crate::clock::{LocalClock, NIGHT_FROM};
use crate::composer::{Composer, OverlayInput};
use crate::frame::Frame;
use crate::salt::InstallSalt;
use crate::sky::LocalDate;
use crate::tween::Millis;
use crate::variants::ArtTable;

/// The salt every preset and snapshot is drawn with.
pub const SALT: InstallSalt = InstallSalt::new(*b"terrarium-preset");
/// The date most presets are drawn on: a full moon.
pub const DATE: LocalDate = LocalDate::new(2026, 3, 3);

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

use Energy::{High, Low, Mid};
use LifeStage::{Adult, Dying, Egg, Juvenile, Pupa};
use ResidentSpecies::{Beetle, Firefly, Ladybird, Snail, Woodlouse};

/// A seeded pool of five, as truth's would be: day and night species, one flyer.
type Dweller = (ResidentSpecies, LifeStage, Energy);

const BUSY: &[Dweller] = &[
    (Snail, Adult, High),
    (Ladybird, Adult, High),
    (Beetle, Adult, Mid),
    (Firefly, Adult, Mid),
    (Woodlouse, Adult, High),
];

use FossilForm::{Bone, LeafPrint, Shell};
use MonumentAge::{Ancient, Lichened, Young};

/// What a preset's terrarium holds beyond its plants.
struct World {
    state: PresentationState,
    plants: &'static [Plant],
    weather: Weather,
    light: Light,
    wind: Wind,
    /// Weeds, litter and fungus.
    cover: (u8, u8, u8),
    /// Condensation and dust.
    glass: (u8, u8),
    /// Each monument's age and carved marks.
    monuments: &'static [(MonumentAge, u8)],
    fossils: &'static [FossilForm],
    residents: &'static [Dweller],
    date: LocalDate,
}

const DEFAULT: World = World {
    state: PresentationState::Thriving,
    plants: &[],
    weather: Weather::Clear,
    light: Light::Neutral,
    wind: Wind::Calm,
    cover: (0, 0, 0),
    glass: (0, 0),
    monuments: &[],
    fossils: &[],
    residents: &[],
    date: DATE,
};

fn world(name: &str) -> Option<World> {
    Some(match name {
        "thriving" => World {
            plants: THRIVING,
            wind: Wind::Breezy,
            residents: BUSY,
            cover: (0, 1, 0),
            monuments: &[(Young, 0)],
            ..DEFAULT
        },
        "struggling-low-light" => World {
            state: PresentationState::Struggling,
            plants: LOW_LIGHT,
            residents: &[
                (Snail, Adult, Mid),
                (Ladybird, Adult, Low),
                (Woodlouse, Juvenile, Mid),
            ],
            weather: Weather::Overcast,
            light: Light::Dusk,
            cover: (1, 2, 0),
            monuments: &[(Lichened, 1)],
            fossils: &[Shell],
            ..DEFAULT
        },
        "struggling-poor-soil" => World {
            state: PresentationState::Struggling,
            plants: POOR_SOIL,
            residents: &[
                (Woodlouse, Adult, Mid),
                (Beetle, Adult, Low),
                (Snail, Egg, Low),
            ],
            weather: Weather::Overcast,
            cover: (6, 4, 2),
            monuments: &[(Lichened, 0)],
            fossils: &[Bone, LeafPrint],
            ..DEFAULT
        },
        "mature" => World {
            state: PresentationState::Mature,
            plants: MATURE,
            residents: &[
                (Snail, Adult, Low),
                (Ladybird, Adult, Low),
                (Beetle, Adult, Low),
                (Woodlouse, Adult, Low),
            ],
            cover: (0, 2, 1),
            monuments: &[(Lichened, 2), (Ancient, 3)],
            fossils: &[LeafPrint],
            ..DEFAULT
        },
        "dormant" => World {
            state: PresentationState::Dormant,
            plants: DORMANT,
            residents: &[
                (Snail, Adult, Low),
                (Beetle, Adult, Low),
                (Woodlouse, Adult, Low),
            ],
            cover: (0, 2, 0),
            glass: (1, 0),
            monuments: &[(Lichened, 1)],
            fossils: &[Shell],
            // A waxing crescent.
            date: LocalDate::new(2026, 3, 21),
            ..DEFAULT
        },
        "dead" => World {
            state: PresentationState::Dead,
            plants: DEAD,
            residents: &[(Woodlouse, Adult, Low), (Snail, Dying, Low)],
            weather: Weather::Haze,
            cover: (2, 8, 4),
            glass: (0, 1),
            monuments: &[(Ancient, 3), (Ancient, 1), (Lichened, 0)],
            fossils: &[Bone, Shell, LeafPrint],
            // A last quarter.
            date: LocalDate::new(2026, 3, 11),
            ..DEFAULT
        },
        "recovering" => World {
            state: PresentationState::Recovering,
            plants: RECOVERING,
            residents: &[
                (Ladybird, Juvenile, High),
                (Snail, Egg, Mid),
                (Woodlouse, Juvenile, Mid),
                (Beetle, Pupa, Mid),
            ],
            light: Light::Sunburst,
            wind: Wind::Breezy,
            cover: (1, 4, 1),
            monuments: &[(Ancient, 2)],
            fossils: &[Bone],
            ..DEFAULT
        },
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
    let monuments = (0_u32..)
        .zip(world.monuments)
        .map(|(i, &(age, marks))| MonumentView {
            id: 10 + i,
            age,
            marks,
        })
        .collect();
    let fossils = (0_u32..)
        .zip(world.fossils)
        .map(|(i, &form)| FossilView { id: 20 + i, form })
        .collect();
    let residents = (0_u32..)
        .zip(world.residents)
        .map(|(i, &(species, stage, energy))| ResidentView {
            id: 30 + i,
            species,
            stage,
            energy,
        })
        .collect();
    Some(SceneInput {
        presentation: Presentation {
            state: world.state,
            since_tick: Tick::new(24 * 90),
        },
        weeds,
        litter,
        fungus,
        monuments,
        fossils,
        residents,
        weather: world.weather,
        light: world.light,
        wind: world.wind,
        condensation,
        dust,
        ..bare(world.state, plants)
    })
}

/// The named preset's overlay: its label reading `label`, at noon on its date.
pub fn overlay(name: &str, label: &str) -> Option<OverlayInput> {
    Some(OverlayInput::new(
        vec![label.to_owned()],
        LocalClock::noon(world(name)?.date),
    ))
}

/// The local time a picture at `palette_mix` is taken at: noon by day, midnight by night, so
/// the residents keep the hours the palette shows.
pub fn clock(date: LocalDate, palette_mix: u8) -> LocalClock {
    if palette_mix >= NIGHT_FROM {
        LocalClock::midnight(date)
    } else {
        LocalClock::noon(date)
    }
}

/// A scene of these plants and nothing else: no ground cover, residents, monuments or marks,
/// a clear and calm sky, clean glass and no event. The presets and the tests start from it.
pub fn bare(state: PresentationState, plants: Vec<PlantView>) -> SceneInput {
    SceneInput {
        presentation: Presentation {
            state,
            since_tick: Tick::new(0),
        },
        plants,
        weeds: 0,
        litter: 0,
        fungus: 0,
        residents: Vec::new(),
        monuments: Vec::new(),
        fossils: Vec::new(),
        ground_marks: Vec::new(),
        weather: Weather::Clear,
        light: Light::Neutral,
        wind: Wind::Calm,
        condensation: 0,
        dust: 0,
        event: None,
    }
}

/// The named preset composed from nothing and settled, at `palette_mix` (0 by day, 255 by
/// night), with its label reading `label`.
pub fn frame(
    art: &Art,
    table: &ArtTable,
    name: &str,
    label: &str,
    palette_mix: u8,
) -> Option<Frame> {
    let input = scene(name)?;
    let mut composer = Composer::new(table, SALT);
    let overlay = OverlayInput::new(
        vec![label.to_owned()],
        clock(world(name)?.date, palette_mix),
    );
    composer.update(&input, &overlay, Millis(0));
    let scenery = composer.scenery(art, table);
    Some(composer.frame(art, table, &scenery, palette_mix, composer.settled_at()))
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
            for mix in [0, 255] {
                let frame = frame(art, &table, name, "repo-terrarium", mix).unwrap();
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

    #[test]
    fn the_preset_dates_show_three_moons() {
        use crate::sky::{MoonPhase, moon_phase};
        let phase = |name| moon_phase(overlay(name, "").unwrap().clock.date);
        assert_eq!(phase("mature"), MoonPhase::Full);
        assert_eq!(phase("dormant"), MoonPhase::WaxingCrescent);
        assert_eq!(phase("dead"), MoonPhase::LastQuarter);
    }
}
