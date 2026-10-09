//! A terrarium from a few climate knobs: the stand-in for truth's projection that `--demo`, the
//! debug window's synthetic look, `cargo xtask gallery` and the synthetic snapshots draw. The
//! live strip draws truth's own.
//!
//! It reads truth's climate and ecology rules statically: one moment, no history. Health is the lower of light and
//! soil, activity mixes rain and wind as truth will, and the presentation state follows truth's
//! rules with the hysteresis band counted as healthy, the initial class. Everything else (the
//! plants' vigor, ground cover, residents, glass, monuments) is a plain integer function of the
//! knobs, so the same knobs always give the same scene. Every row keeps its id whatever the
//! knobs, so a moving knob tweens the terrarium rather than replacing it. Events are not a knob:
//! [`set_event`] puts one on any scene, and a sinkhole reveals and names its fossil.

use terrarium_truth::scene::{
    ActiveEvent, Energy, EventKind, FossilForm, FossilView, GrowthStage, LifeStage, Light,
    MonumentAge, MonumentView, PlantSpecies, PlantState, PlantView, Presentation, ResidentSpecies,
    ResidentView, SceneInput, Side, Strength, Weather, Wind,
};
use terrarium_truth::{PresentationState, Tick};

use crate::catalog::Art;
use crate::composer::{Composer, OverlayInput};
use crate::frame::{Frame, Viewport};
use crate::preset;
use crate::tween::Millis;
use crate::variants::ArtTable;

/// The most light, soil, health or activity: they are percentages.
pub const FULL: u8 = 100;
/// The most rain or wind, as a percentage of the repo's own baseline.
pub const MAX_RATIO: u16 = 400;

/// Health from which the terrarium counts as healthy. Truth's band between 45 and 55 keeps the
/// previous class, and with no history that is the initial one, healthy.
pub const HEALTHY_FROM: u8 = 45;
/// Activity from which the repo counts as active.
pub const ACTIVE_FROM: u8 = 35;
/// Days without a commit from which the repo counts as inactive.
pub const INACTIVE_DAYS: u16 = 30;

/// Vigor from which foliage thrives, and below which it dies.
const THRIVING_FROM: u8 = 60;
const ALIVE_FROM: u8 = 35;
/// Soil below which weeds take free slots.
const WEEDS_BELOW: u8 = 45;
/// The most weeds: a third of the plant capacity, at most 8.
const MAX_WEEDS: u32 = 6;
const MAX_LITTER: u32 = 8;
/// The fewest plants present: the plant capacity's floor.
const MIN_PLANTS: u32 = 6;
/// Rain or wind, as a percentage of baseline, from which the wind is Breezy, and from which it
/// is a Gale: the windstorm ratio.
const BREEZY_FROM: u16 = 100;
const GALE_FROM: u16 = 300;
/// Activity below which residents are slow, and from which they are busy.
const MID_ENERGY_FROM: u8 = 25;
const HIGH_ENERGY_FROM: u8 = 55;
/// Days dormant from which condensation reaches levels 2 and 3, and days dead from which dust
/// reaches levels 1, 2 and 3.
const CONDENSATION_DAYS: [u16; 2] = [30, 90];
const DUST_DAYS: [u16; 3] = [365, 540, 730];
/// The most monuments: one per year of age.
const MAX_MONUMENTS: u8 = 3;

/// What the terrarium is given: the climate and habitat truth would derive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Knobs {
    /// Coverage, 0–100.
    pub light: u8,
    /// Soil health, 0–100.
    pub soil: u8,
    /// Commits, as a percentage of the repo's own baseline, 0–400.
    pub rain: u16,
    /// Changed lines, as a percentage of the repo's own baseline, 0–400.
    pub wind: u16,
    pub weather: Weather,
    /// The light of a coverage change: neutral, dusk or sunburst.
    pub shift: Light,
    /// Days since the last commit on any branch.
    pub idle_days: u16,
    /// Health has risen since a struggling, dormant or dead spell.
    pub recovering: bool,
    /// The repo's age in whole years.
    pub age_years: u8,
}

impl Knobs {
    /// A typical day at `health`: both climates at it, rain and wind at baseline, a clear sky.
    pub const fn typical(health: u8) -> Knobs {
        Knobs {
            light: health,
            soil: health,
            rain: 100,
            wind: 100,
            weather: Weather::Clear,
            shift: Light::Neutral,
            idle_days: 0,
            recovering: false,
            age_years: 3,
        }
    }

    /// The lower of light and soil.
    pub fn health(&self) -> u8 {
        self.light.min(self.soil).min(FULL)
    }

    /// Rain and wind mixed as truth mixes them, 50 on a typical day.
    pub fn activity(&self) -> u8 {
        let mixed = (75 * u32::from(self.rain) + 25 * u32::from(self.wind)) / 200;
        percent(mixed)
    }

    /// Sets light and soil to `health` together.
    pub fn set_health(&mut self, health: u8) {
        let health = health.min(FULL);
        self.light = health;
        self.soil = health;
    }

    /// Sets rain and wind together so the activity reads `activity`.
    pub fn set_activity(&mut self, activity: u8) {
        let ratio = u16::from(activity.min(FULL)) * 2;
        self.rain = ratio;
        self.wind = ratio;
    }

    pub fn inactive(&self) -> bool {
        self.idle_days >= INACTIVE_DAYS
    }

    pub fn healthy(&self) -> bool {
        self.health() >= HEALTHY_FROM
    }

    /// The presentation state, by truth's rules in their order of precedence.
    pub fn state(&self) -> PresentationState {
        let healthy = self.healthy();
        if self.inactive() {
            if healthy {
                PresentationState::Dormant
            } else {
                PresentationState::Dead
            }
        } else if self.recovering {
            PresentationState::Recovering
        } else if !healthy {
            PresentationState::Struggling
        } else if self.activity() >= ACTIVE_FROM {
            PresentationState::Thriving
        } else {
            PresentationState::Mature
        }
    }

    /// Light limits when it is the lower: low vigor then shows as pale, not wilting.
    fn light_limits(&self) -> bool {
        self.light < self.soil
    }

    fn wind_class(&self) -> Wind {
        if self.wind >= GALE_FROM {
            Wind::Gale
        } else if self.wind >= BREEZY_FROM {
            Wind::Breezy
        } else {
            Wind::Calm
        }
    }
}

fn percent(value: u32) -> u8 {
    u8::try_from(value.min(u32::from(FULL))).unwrap_or(FULL)
}

use GrowthStage::{Mature as M, Seedling as S, Young as Y};
use PlantSpecies::{Clover, Fern, Foxglove, MossMound, Oak, Reed, Rowan, Thistle};

/// The plant rows, from one seeded pool of eight, the presets' own: each row's species, its
/// stage in an ordinary year, and how far its vigor sits from the climate's. The hardiest come
/// first, since a failing terrarium loses rows from the end.
const ROWS: [(PlantSpecies, GrowthStage, i8); 20] = [
    (Oak, M, 12),
    (Fern, M, -3),
    (MossMound, M, 7),
    (Rowan, M, -12),
    (Reed, M, 2),
    (Thistle, M, 15),
    (Foxglove, M, -8),
    (Clover, M, 5),
    (Oak, Y, -15),
    (Fern, Y, 10),
    (Reed, Y, -5),
    (Clover, Y, 0),
    (Rowan, Y, 13),
    (MossMound, Y, -10),
    (Thistle, Y, 4),
    (Foxglove, S, -1),
    (Fern, S, 8),
    (Clover, S, -14),
    (MossMound, S, 11),
    (Reed, S, -6),
];

/// The resident pool of five, as truth's would be: day and night species and one flyer, the
/// hardiest first.
const DWELLERS: [ResidentSpecies; 5] = [
    ResidentSpecies::Woodlouse,
    ResidentSpecies::Snail,
    ResidentSpecies::Beetle,
    ResidentSpecies::Ladybird,
    ResidentSpecies::Firefly,
];

/// How the young look while the terrarium recovers, per dweller.
const RECOVERING_STAGES: [LifeStage; 5] = [
    LifeStage::Juvenile,
    LifeStage::Egg,
    LifeStage::Pupa,
    LifeStage::Juvenile,
    LifeStage::Juvenile,
];

const FOSSIL_FORMS: [FossilForm; 3] = [FossilForm::Shell, FossilForm::Bone, FossilForm::LeafPrint];

/// The terrarium these knobs describe.
pub fn scene(knobs: &Knobs) -> SceneInput {
    let state = knobs.state();
    let plants = plants(knobs, state);
    let dead = plants
        .iter()
        .filter(|plant| plant.state == PlantState::Dead)
        .count();
    let lost = ROWS.len() - plants.len();
    let wind = knobs.wind_class();
    let gust = if wind == Wind::Gale { 2 } else { 0 };
    let litter = count(
        (u32::try_from(lost + dead).unwrap_or(u32::MAX)) / 2 + gust,
        MAX_LITTER,
    );
    let still = knobs.inactive() || knobs.activity() < ACTIVE_FROM;
    let fungus = if still { litter / 2 } else { 0 };
    let weeds = if knobs.soil < WEEDS_BELOW {
        let short = u32::from(WEEDS_BELOW - knobs.soil);
        count(
            (short * MAX_WEEDS).div_ceil(u32::from(WEEDS_BELOW)),
            MAX_WEEDS,
        )
    } else {
        0
    };
    let idle_since = knobs.idle_days.saturating_sub(INACTIVE_DAYS);
    let condensation = if state == PresentationState::Dormant {
        level(idle_since, &[0, CONDENSATION_DAYS[0], CONDENSATION_DAYS[1]])
    } else {
        0
    };
    let dust = if state == PresentationState::Dead {
        level(idle_since, &DUST_DAYS)
    } else {
        0
    };
    SceneInput {
        presentation: Presentation {
            state,
            since_tick: Tick::ZERO,
        },
        plants,
        weeds,
        litter,
        fungus,
        residents: residents(knobs, state),
        monuments: monuments(knobs.age_years),
        fossils: fossils(knobs.age_years),
        ground_marks: Vec::new(),
        weather: knobs.weather,
        light: knobs.shift,
        wind,
        condensation,
        dust,
        event: None,
    }
}

/// Makes `kind` the scene's foreground event, of middling strength, from the left. A sinkhole
/// reveals a fossil, and names it.
pub fn set_event(scene: &mut SceneInput, kind: EventKind) {
    let fossil = (kind == EventKind::Sinkhole).then(|| {
        let id = scene
            .fossils
            .iter()
            .map(|f| f.id.saturating_add(1))
            .max()
            .unwrap_or(20);
        scene.fossils.push(FossilView {
            id,
            form: FossilForm::Bone,
        });
        id
    });
    scene.event = Some(ActiveEvent {
        kind,
        started_tick: Tick::ZERO,
        strength: Strength::Mid,
        side: Side::Left,
        fossil,
    });
}

fn count(value: u32, max: u32) -> u8 {
    u8::try_from(value.min(max)).unwrap_or(u8::MAX)
}

/// How many of `thresholds` `days` has reached.
fn level(days: u16, thresholds: &[u16]) -> u8 {
    count(
        u32::try_from(thresholds.iter().filter(|&&from| days >= from).count()).unwrap_or(0),
        u32::from(terrarium_truth::scene::MAX_GLASS_LEVEL),
    )
}

fn plants(knobs: &Knobs, whole: PresentationState) -> Vec<PlantView> {
    let health = knobs.health();
    let rows = u32::try_from(ROWS.len()).unwrap_or(u32::MAX);
    let present = MIN_PLANTS + u32::from(health) * (rows - MIN_PLANTS) / u32::from(FULL);
    let present = usize::try_from(present).unwrap_or(ROWS.len());
    let failing = if knobs.light_limits() {
        PlantState::Pale
    } else {
        PlantState::Wilting
    };
    (0_u16..)
        .zip(ROWS.iter().take(present))
        .map(|(order, &(species, usual, offset))| {
            let vigor = vigor(health, offset);
            let alive = if vigor >= THRIVING_FROM {
                PlantState::Thriving
            } else if vigor >= ALIVE_FROM {
                failing
            } else {
                PlantState::Dead
            };
            let (grown, look) = match whole {
                // Every living plant rests while the repo is dormant.
                PresentationState::Dormant if alive != PlantState::Dead => {
                    (usual, PlantState::Dormant)
                }
                PresentationState::Mature => (M, alive),
                // The old growth stands dead, and new growth rises among it.
                PresentationState::Recovering if order % 3 == 0 => (M, PlantState::Dead),
                PresentationState::Recovering if order < 10 => (Y, alive),
                PresentationState::Recovering => (S, alive),
                _ => (usual, alive),
            };
            PlantView {
                id: 100 + u32::from(order) * 7,
                species,
                stage: grown,
                state: look,
                bloom: false,
                scorched: false,
                order,
            }
        })
        .collect()
}

fn vigor(health: u8, offset: i8) -> u8 {
    let vigor = i16::from(health) + i16::from(offset);
    u8::try_from(vigor.clamp(0, i16::from(FULL))).unwrap_or(0)
}

fn residents(knobs: &Knobs, whole: PresentationState) -> Vec<ResidentView> {
    let present = 1 + usize::from(knobs.health() / 20);
    let resting = matches!(whole, PresentationState::Dormant | PresentationState::Dead);
    let energy = if resting || knobs.activity() < MID_ENERGY_FROM {
        Energy::Low
    } else if knobs.activity() < HIGH_ENERGY_FROM {
        Energy::Mid
    } else {
        Energy::High
    };
    (0_u32..)
        .zip(DWELLERS.iter().zip(RECOVERING_STAGES))
        .take(present.min(DWELLERS.len()))
        .map(|(i, (&species, young))| {
            let stage = match whole {
                PresentationState::Recovering => young,
                PresentationState::Dead if i == 1 => LifeStage::Dying,
                _ => LifeStage::Adult,
            };
            ResidentView {
                id: 30 + i,
                species,
                stage,
                energy,
            }
        })
        .collect()
}

/// One monument per full year, the oldest first, each aged by the years since it was raised.
/// The oldest carries a mark for every other year.
fn monuments(age_years: u8) -> Vec<MonumentView> {
    (0..age_years.min(MAX_MONUMENTS))
        .map(|k| {
            let age = age_years - k - 1;
            MonumentView {
                id: 10 + u32::from(k),
                age: match age {
                    0..2 => MonumentAge::Young,
                    2..5 => MonumentAge::Lichened,
                    _ => MonumentAge::Ancient,
                },
                marks: if k == 0 {
                    (age_years.saturating_sub(1) / 2).min(terrarium_truth::scene::MAX_MARKS)
                } else {
                    0
                },
            }
        })
        .collect()
}

/// A fossil for every three years, at most one of each form.
fn fossils(age_years: u8) -> Vec<FossilView> {
    (0_u32..)
        .zip(FOSSIL_FORMS)
        .take(usize::from(age_years / 3))
        .map(|(i, form)| FossilView { id: 20 + i, form })
        .collect()
}

/// The named states, one per key look: the presets' names, so a synthetic picture can sit
/// beside its preset. `--demo` plays them in this order.
pub const NAMED: [(&str, Knobs); 7] = [
    (
        "thriving",
        Knobs {
            light: 85,
            soil: 85,
            rain: 140,
            wind: 140,
            ..Knobs::typical(85)
        },
    ),
    (
        "mature",
        Knobs {
            light: 85,
            soil: 80,
            rain: 30,
            wind: 30,
            age_years: 6,
            ..Knobs::typical(85)
        },
    ),
    (
        "struggling-low-light",
        Knobs {
            light: 35,
            soil: 75,
            rain: 120,
            wind: 110,
            weather: Weather::Overcast,
            shift: Light::Dusk,
            ..Knobs::typical(35)
        },
    ),
    (
        "struggling-poor-soil",
        Knobs {
            light: 75,
            soil: 30,
            rain: 90,
            wind: 80,
            weather: Weather::Overcast,
            ..Knobs::typical(30)
        },
    ),
    (
        "dormant",
        Knobs {
            light: 70,
            soil: 65,
            rain: 0,
            wind: 0,
            idle_days: 45,
            ..Knobs::typical(65)
        },
    ),
    (
        "dead",
        Knobs {
            light: 15,
            soil: 20,
            rain: 0,
            wind: 0,
            weather: Weather::Haze,
            idle_days: 420,
            age_years: 7,
            ..Knobs::typical(15)
        },
    ),
    (
        "recovering",
        Knobs {
            light: 75,
            soil: 70,
            rain: 160,
            wind: 120,
            shift: Light::Sunburst,
            recovering: true,
            age_years: 4,
            ..Knobs::typical(70)
        },
    ),
];

/// The named state's knobs, or `None` for a name not in [`NAMED`].
pub fn named(name: &str) -> Option<Knobs> {
    NAMED
        .iter()
        .find(|(known, _)| *known == name)
        .map(|&(_, knobs)| knobs)
}

/// The gallery's health rows, healthiest first, and its activity columns, stillest first;
/// `None` is a repo untouched for two months.
pub const GALLERY_HEALTH: [u8; 6] = [90, 75, 60, 40, 25, 10];
pub const GALLERY_ACTIVITY: [Option<u8>; 5] = [None, Some(10), Some(25), Some(50), Some(85)];
/// The days an inactive column has gone without a commit.
const GALLERY_IDLE_DAYS: u16 = 60;
/// The gallery's terrarium is four years old: monuments and a fossil, as most repos would have.
const GALLERY_AGE: u8 = 4;

/// One state of the health × activity grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub row: usize,
    pub column: usize,
    pub health: u8,
    pub activity: Option<u8>,
    pub knobs: Knobs,
}

/// The 30 states of the health × activity grid, row by row. Light and soil stand equal, so soil
/// limits and low vigor wilts; the debug window shows pale. Recovering is a path through time
/// rather than a point on the grid, so the named states hold it.
pub fn gallery() -> Vec<Cell> {
    let mut cells = Vec::with_capacity(GALLERY_HEALTH.len() * GALLERY_ACTIVITY.len());
    for (row, &health) in GALLERY_HEALTH.iter().enumerate() {
        for (column, &activity) in GALLERY_ACTIVITY.iter().enumerate() {
            let mut knobs = Knobs {
                age_years: GALLERY_AGE,
                ..Knobs::typical(health)
            };
            if let Some(activity) = activity {
                knobs.set_activity(activity);
            } else {
                knobs.set_activity(0);
                knobs.idle_days = GALLERY_IDLE_DAYS;
            }
            cells.push(Cell {
                row,
                column,
                health,
                activity,
                knobs,
            });
        }
    }
    cells
}

/// The knobs' terrarium composed from nothing and settled, with the presets' salt and date, at
/// `palette_mix` (noon by day, midnight by night), dimmed or not, its label reading `label`.
pub fn frame(
    art: &Art,
    table: &ArtTable,
    knobs: &Knobs,
    label: &str,
    palette_mix: u8,
    dim: bool,
) -> Frame {
    frame_in(art, table, knobs, label, palette_mix, dim, Viewport::SCENE)
}

/// As [`frame`], in `viewport`.
pub fn frame_in(
    art: &Art,
    table: &ArtTable,
    knobs: &Knobs,
    label: &str,
    palette_mix: u8,
    dim: bool,
    viewport: Viewport,
) -> Frame {
    let mut composer = Composer::new(table, preset::SALT);
    composer.set_viewport(viewport);
    let overlay = OverlayInput {
        dim,
        ..OverlayInput::new(
            vec![label.to_owned()],
            preset::clock(preset::DATE, palette_mix),
        )
    };
    composer.update(&scene(knobs), &overlay, Millis(0));
    let scenery = composer.scenery(art, table);
    composer.frame(art, table, &scenery, palette_mix, composer.settled_at())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Truth's state rules written out again, independently of `Knobs::state`.
    fn expected_state(health: u8, activity: u8, idle_days: u16, recovering: bool) -> &'static str {
        let healthy = health >= 45;
        match (idle_days >= 30, recovering, healthy, activity >= 35) {
            (true, _, true, _) => "Dormant",
            (true, _, false, _) => "Dead",
            (false, true, _, _) => "Recovering",
            (false, false, false, _) => "Struggling",
            (false, false, true, true) => "Thriving",
            (false, false, true, false) => "Mature",
        }
    }

    #[test]
    fn the_state_follows_the_rules_in_their_order() {
        for light in (0..=100).step_by(5) {
            for soil in (0..=100).step_by(5) {
                for ratio in (0..=400).step_by(20) {
                    for idle_days in [0, 29, 30, 400] {
                        for recovering in [false, true] {
                            let knobs = Knobs {
                                light,
                                soil,
                                rain: ratio,
                                wind: ratio,
                                idle_days,
                                recovering,
                                ..Knobs::typical(0)
                            };
                            let activity = u8::try_from(u32::from(ratio).min(200) / 2).unwrap();
                            assert_eq!(knobs.activity(), activity);
                            assert_eq!(
                                knobs.state().name(),
                                expected_state(light.min(soil), activity, idle_days, recovering),
                                "{knobs:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_typical_day_is_half_active() {
        assert_eq!(Knobs::typical(80).activity(), 50);
        let mut knobs = Knobs::typical(80);
        knobs.set_activity(85);
        assert_eq!(knobs.activity(), 85);
        knobs.set_health(30);
        assert_eq!((knobs.light, knobs.soil, knobs.health()), (30, 30, 30));
    }

    fn thriving(knobs: &Knobs) -> usize {
        scene(knobs)
            .plants
            .iter()
            .filter(|plant| plant.state == PlantState::Thriving)
            .count()
    }

    #[test]
    fn more_light_or_soil_never_means_fewer_thriving_plants() {
        for base in [
            Knobs::typical(0),
            NAMED[0].1,
            NAMED[1].1,
            NAMED[4].1,
            NAMED[6].1,
        ] {
            for light in (0..100).step_by(5) {
                for soil in (0..100).step_by(5) {
                    let knobs = Knobs {
                        light,
                        soil,
                        ..base
                    };
                    let more_light = Knobs {
                        light: light + 5,
                        ..knobs
                    };
                    let more_soil = Knobs {
                        soil: soil + 5,
                        ..knobs
                    };
                    assert!(thriving(&more_light) >= thriving(&knobs), "{more_light:?}");
                    assert!(thriving(&more_soil) >= thriving(&knobs), "{more_soil:?}");
                }
            }
        }
    }

    #[test]
    fn low_vigor_is_pale_when_light_limits_and_wilting_when_soil_does() {
        let looks = |knobs: &Knobs| -> Vec<PlantState> {
            scene(knobs)
                .plants
                .iter()
                .map(|plant| plant.state)
                .collect()
        };
        let low_light = looks(&named("struggling-low-light").unwrap());
        let poor_soil = looks(&named("struggling-poor-soil").unwrap());
        assert!(low_light.contains(&PlantState::Pale));
        assert!(!low_light.contains(&PlantState::Wilting));
        assert!(poor_soil.contains(&PlantState::Wilting));
        assert!(!poor_soil.contains(&PlantState::Pale));
    }

    #[test]
    fn every_named_and_gallery_scene_is_valid() {
        for (name, knobs) in NAMED {
            let scene = scene(&knobs);
            assert_eq!(scene.validate(), Ok(()), "{name}");
            let state = scene.presentation.state.name().to_ascii_lowercase();
            assert!(name.starts_with(&state), "{name} is {state}");
        }
        for cell in gallery() {
            assert_eq!(scene(&cell.knobs).validate(), Ok(()), "{cell:?}");
        }
        for kind in EventKind::ALL {
            let mut with = scene(&NAMED[0].1);
            set_event(&mut with, *kind);
            assert_eq!(with.validate(), Ok(()), "{kind:?}");
        }
    }

    #[test]
    fn any_scene_can_take_any_event() {
        for name in crate::preset::NAMES {
            for kind in EventKind::ALL {
                let mut scene = crate::preset::scene(name).unwrap();
                let fossils = scene.fossils.len();
                set_event(&mut scene, *kind);
                assert_eq!(scene.validate(), Ok(()), "{name}, {kind:?}");
                assert_eq!(scene.event.map(|event| event.kind), Some(*kind));
                let revealed = usize::from(*kind == EventKind::Sinkhole);
                assert_eq!(scene.fossils.len(), fossils + revealed);
            }
        }
    }

    #[test]
    fn the_named_states_show_all_six() {
        let mut states: Vec<_> = NAMED.iter().map(|(_, k)| k.state()).collect();
        states.sort();
        states.dedup();
        assert_eq!(states.len(), 6);
        assert_eq!(named("lush"), None);
    }

    #[test]
    fn the_gallery_is_six_health_rows_by_five_activity_columns() {
        let cells = gallery();
        assert_eq!(cells.len(), 30);
        for cell in &cells {
            assert_eq!(cell.knobs.health(), cell.health);
            match cell.activity {
                Some(activity) => {
                    assert_eq!(cell.knobs.activity(), activity);
                    assert!(!cell.knobs.inactive());
                }
                None => assert!(cell.knobs.inactive()),
            }
        }
        let mut states: Vec<_> = cells.iter().map(|cell| cell.knobs.state()).collect();
        states.sort();
        states.dedup();
        // Every state but Recovering, which is a path rather than a point.
        assert_eq!(states.len(), 5);
        assert!(!states.contains(&PresentationState::Recovering));
    }

    #[test]
    fn every_row_keeps_its_id_whatever_the_knobs() {
        let ids = |knobs: &Knobs| -> Vec<(u32, PlantSpecies)> {
            scene(knobs)
                .plants
                .iter()
                .map(|plant| (plant.id, plant.species))
                .collect()
        };
        let all = ids(&Knobs::typical(100));
        assert_eq!(all.len(), 20);
        for (_, knobs) in NAMED {
            let some = ids(&knobs);
            assert_eq!(some[..], all[..some.len()], "{knobs:?}");
        }
    }

    #[test]
    fn an_idle_terrarium_rests_then_mists_or_gathers_dust() {
        let dormant = |idle_days| {
            scene(&Knobs {
                idle_days,
                ..Knobs::typical(70)
            })
        };
        let dormant_scene = dormant(45);
        assert_eq!(dormant_scene.presentation.state, PresentationState::Dormant);
        assert!(
            dormant_scene
                .plants
                .iter()
                .all(|plant| plant.state == PlantState::Dormant)
        );
        assert!(
            dormant_scene
                .residents
                .iter()
                .all(|resident| resident.energy == Energy::Low)
        );
        assert_eq!(
            [29, 30, 59, 60, 119, 120].map(|days| dormant(days).condensation),
            [0, 1, 1, 2, 2, 3]
        );
        let dead = |idle_days| {
            scene(&Knobs {
                idle_days,
                ..Knobs::typical(20)
            })
        };
        assert_eq!(dead(60).presentation.state, PresentationState::Dead);
        assert_eq!(
            [394, 395, 570, 760].map(|days| dead(days).dust),
            [0, 1, 2, 3]
        );
    }
}
