//! `SceneInput`, the composer's only truth-derived input.
//!
//! It is frozen: any change is an amendment and a `SIM_VERSION` bump. Every enum mirrors the art,
//! and each variant is the art's structured key in Pascal case (`tall-grass` is `TallGrass`,
//! `beetle` is `Beetle`), with two deliberate merges: `LifeStage::Egg` covers the egg, brood
//! and cocoon stages, and `LifeStage::Dying` is the adult `death` routine.
//!
//! Variant order is part of the postcard encoding, so a new variant is only ever appended.

use serde::{Deserialize, Serialize};

use crate::canon::{canonical_enum, canonical_struct};
use crate::domain::PresentationState;
use crate::time::Tick;

/// The most carved marks a monument holds: the stone has three placement points.
pub const MAX_MARKS: u8 = 3;
/// The highest condensation and dust level: the glass art has levels 1 to 3.
pub const MAX_GLASS_LEVEL: u8 = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneInput {
    pub presentation: Presentation,
    pub plants: Vec<PlantView>,
    /// Ground cover, as counts; the composer merges weeds into patches above its cap.
    pub weeds: u8,
    pub litter: u8,
    pub fungus: u8,
    pub residents: Vec<ResidentView>,
    pub monuments: Vec<MonumentView>,
    pub fossils: Vec<FossilView>,
    /// Marks an event leaves in the ground that outlast it, until truth clears them.
    pub ground_marks: Vec<GroundMarkView>,
    pub weather: Weather,
    pub light: Light,
    pub wind: Wind,
    /// 0 for none, up to `MAX_GLASS_LEVEL`.
    pub condensation: u8,
    /// 0 for none, up to `MAX_GLASS_LEVEL`.
    pub dust: u8,
    /// The one foreground event, while it plays.
    pub event: Option<ActiveEvent>,
}

canonical_struct!(SceneInput {
    presentation,
    plants,
    weeds,
    litter,
    fungus,
    residents,
    monuments,
    fossils,
    ground_marks,
    weather,
    light,
    wind,
    condensation,
    dust,
    event,
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presentation {
    pub state: PresentationState,
    pub since_tick: Tick,
}

canonical_struct!(Presentation { state, since_tick });

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlantView {
    /// Stable across ticks.
    pub id: u32,
    pub species: PlantSpecies,
    pub stage: GrowthStage,
    pub state: PlantState,
    /// Authorized by the sim; drawn only on a mature, thriving plant.
    pub bloom: bool,
    /// The lightning target.
    pub scorched: bool,
    /// Truth's order, for slot assignment.
    pub order: u16,
}

canonical_struct!(PlantView {
    id,
    species,
    stage,
    state,
    bloom,
    scorched,
    order,
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PlantSpecies {
    Oak,
    Birch,
    Rowan,
    Fern,
    TallGrass,
    Bramble,
    Foxglove,
    Reed,
    MossMound,
    WildflowerCluster,
    Clover,
    Thistle,
}

canonical_enum!(PlantSpecies {
    Oak,
    Birch,
    Rowan,
    Fern,
    TallGrass,
    Bramble,
    Foxglove,
    Reed,
    MossMound,
    WildflowerCluster,
    Clover,
    Thistle,
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum GrowthStage {
    Seedling,
    Young,
    Mature,
}

canonical_enum!(GrowthStage {
    Seedling,
    Young,
    Mature
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PlantState {
    Thriving,
    Pale,
    Wilting,
    Dormant,
    Dead,
}

canonical_enum!(PlantState {
    Thriving,
    Pale,
    Wilting,
    Dormant,
    Dead,
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResidentView {
    pub id: u32,
    pub species: ResidentSpecies,
    pub stage: LifeStage,
    pub energy: Energy,
}

canonical_struct!(ResidentView {
    id,
    species,
    stage,
    energy
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ResidentSpecies {
    Snail,
    Ladybird,
    Beetle,
    Firefly,
    Woodlouse,
    Springtail,
    Earthworm,
    Harvestman,
}

canonical_enum!(ResidentSpecies {
    Snail,
    Ladybird,
    Beetle,
    Firefly,
    Woodlouse,
    Springtail,
    Earthworm,
    Harvestman,
});

impl ResidentSpecies {
    /// Whether the species has a pupa stage in the art: the ladybird, beetle and firefly do.
    pub const fn has_pupa(self) -> bool {
        matches!(
            self,
            ResidentSpecies::Ladybird | ResidentSpecies::Beetle | ResidentSpecies::Firefly
        )
    }
}

/// A resident's life stage. `Egg` covers the egg, the woodlouse's brood and the earthworm's
/// cocoon; `Pupa` exists only for species where [`ResidentSpecies::has_pupa`] holds; `Dying`
/// is the adult death routine, held until truth removes the resident.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LifeStage {
    Egg,
    Juvenile,
    Pupa,
    Adult,
    Dying,
}

canonical_enum!(LifeStage {
    Egg,
    Juvenile,
    Pupa,
    Adult,
    Dying,
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Energy {
    Low,
    Mid,
    High,
}

canonical_enum!(Energy { Low, Mid, High });

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonumentView {
    pub id: u32,
    pub age: MonumentAge,
    /// Carved marks, up to `MAX_MARKS`.
    pub marks: u8,
}

canonical_struct!(MonumentView { id, age, marks });

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MonumentAge {
    Young,
    Lichened,
    Ancient,
}

canonical_enum!(MonumentAge {
    Young,
    Lichened,
    Ancient
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FossilView {
    pub id: u32,
    pub form: FossilForm,
}

canonical_struct!(FossilView { id, form });

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FossilForm {
    Shell,
    Bone,
    LeafPrint,
}

canonical_enum!(FossilForm {
    Shell,
    Bone,
    LeafPrint
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundMarkView {
    pub id: u32,
    pub kind: GroundMark,
    /// The fossil a sinkhole revealed; `None` for every other kind.
    pub fossil: Option<u32>,
}

canonical_struct!(GroundMarkView { id, kind, fossil });

/// A mark in the ground that outlasts its event: the settled earthquake crack, the settled
/// sinkhole, or the fresh soil a clearing burn leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum GroundMark {
    Crack,
    Sinkhole,
    FreshSoil,
}

canonical_enum!(GroundMark {
    Crack,
    Sinkhole,
    FreshSoil
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Weather {
    Clear,
    Overcast,
    Storm,
    Haze,
}

canonical_enum!(Weather {
    Clear,
    Overcast,
    Storm,
    Haze
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Light {
    Neutral,
    Dusk,
    Sunburst,
}

canonical_enum!(Light {
    Neutral,
    Dusk,
    Sunburst
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Wind {
    Calm,
    Breezy,
    Gale,
}

canonical_enum!(Wind { Calm, Breezy, Gale });

/// The foreground event while it plays. One instance is one (`kind`, `started_tick`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveEvent {
    pub kind: EventKind,
    pub started_tick: Tick,
    pub strength: Strength,
    /// Chosen by truth from the seed and the tick.
    pub side: Side,
    /// The fossil a sinkhole reveals; `None` for every other kind.
    pub fossil: Option<u32>,
}

canonical_struct!(ActiveEvent {
    kind,
    started_tick,
    strength,
    side,
    fossil,
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EventKind {
    Storm,
    Clearing,
    Lightning,
    Dusk,
    Sunburst,
    ClearingBurn,
    Flowering,
    Windstorm,
    RainAfterDrought,
    Earthquake,
    Sinkhole,
}

canonical_enum!(EventKind {
    Storm,
    Clearing,
    Lightning,
    Dusk,
    Sunburst,
    ClearingBurn,
    Flowering,
    Windstorm,
    RainAfterDrought,
    Earthquake,
    Sinkhole,
});

impl EventKind {
    /// The kind's stable id: its postcard encoding, fixed by declaration order.
    pub const fn id(self) -> u8 {
        match self {
            EventKind::Storm => 0,
            EventKind::Clearing => 1,
            EventKind::Lightning => 2,
            EventKind::Dusk => 3,
            EventKind::Sunburst => 4,
            EventKind::ClearingBurn => 5,
            EventKind::Flowering => 6,
            EventKind::Windstorm => 7,
            EventKind::RainAfterDrought => 8,
            EventKind::Earthquake => 9,
            EventKind::Sinkhole => 10,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Strength {
    Low,
    Mid,
    High,
}

canonical_enum!(Strength { Low, Mid, High });

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
}

canonical_enum!(Side { Left, Right });

/// A way a `SceneInput` breaks the contract. For developers only; no user sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneInputError {
    TooManyMarks {
        monument: u32,
        marks: u8,
    },
    CondensationTooHigh(u8),
    DustTooHigh(u8),
    PupaWithoutArt {
        resident: u32,
    },
    DuplicateId {
        list: &'static str,
        id: u32,
    },
    /// A sinkhole without its fossil, or a fossil on anything else.
    EventFossil,
    GroundMarkFossil {
        mark: u32,
    },
}

impl SceneInput {
    /// Checks the rules the types cannot hold on their own.
    pub fn validate(&self) -> Result<(), SceneInputError> {
        if self.condensation > MAX_GLASS_LEVEL {
            return Err(SceneInputError::CondensationTooHigh(self.condensation));
        }
        if self.dust > MAX_GLASS_LEVEL {
            return Err(SceneInputError::DustTooHigh(self.dust));
        }
        if let Some(monument) = self.monuments.iter().find(|m| m.marks > MAX_MARKS) {
            return Err(SceneInputError::TooManyMarks {
                monument: monument.id,
                marks: monument.marks,
            });
        }
        if let Some(resident) = self
            .residents
            .iter()
            .find(|r| r.stage == LifeStage::Pupa && !r.species.has_pupa())
        {
            return Err(SceneInputError::PupaWithoutArt {
                resident: resident.id,
            });
        }
        unique_ids("plants", self.plants.iter().map(|p| p.id))?;
        unique_ids("residents", self.residents.iter().map(|r| r.id))?;
        unique_ids("monuments", self.monuments.iter().map(|m| m.id))?;
        unique_ids("fossils", self.fossils.iter().map(|f| f.id))?;
        unique_ids("ground_marks", self.ground_marks.iter().map(|g| g.id))?;
        if let Some(event) = &self.event
            && (event.kind == EventKind::Sinkhole) != event.fossil.is_some()
        {
            return Err(SceneInputError::EventFossil);
        }
        if let Some(mark) = self
            .ground_marks
            .iter()
            .find(|g| (g.kind == GroundMark::Sinkhole) != g.fossil.is_some())
        {
            return Err(SceneInputError::GroundMarkFossil { mark: mark.id });
        }
        Ok(())
    }
}

fn unique_ids(list: &'static str, ids: impl Iterator<Item = u32>) -> Result<(), SceneInputError> {
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(SceneInputError::DuplicateId { list, id });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use proptest::prelude::*;
    use proptest::sample::select;

    use super::*;
    use crate::canon::{canonical_dump, canonical_sha256, from_canonical_dump, to_hex};
    use crate::identity::{SIM_VERSION, SimVersion};
    use crate::test_support::proptest_config;

    /// Keeps the first of each id, so lists meet the uniqueness rule.
    fn unique_by_id<T>(mut items: Vec<T>, id: impl Fn(&T) -> u32) -> Vec<T> {
        let mut seen = BTreeSet::new();
        items.retain(|item| seen.insert(id(item)));
        items
    }

    fn plant() -> impl Strategy<Value = PlantView> {
        (
            any::<u32>(),
            select(PlantSpecies::ALL),
            select(GrowthStage::ALL),
            select(PlantState::ALL),
            any::<bool>(),
            any::<bool>(),
            any::<u16>(),
        )
            .prop_map(
                |(id, species, stage, state, bloom, scorched, order)| PlantView {
                    id,
                    species,
                    stage,
                    state,
                    bloom,
                    scorched,
                    order,
                },
            )
    }

    fn resident() -> impl Strategy<Value = ResidentView> {
        (
            any::<u32>(),
            select(ResidentSpecies::ALL),
            select(LifeStage::ALL),
            select(Energy::ALL),
        )
            .prop_map(|(id, species, stage, energy)| ResidentView {
                id,
                species,
                // Only three species have a pupa in the art.
                stage: if stage == LifeStage::Pupa && !species.has_pupa() {
                    LifeStage::Adult
                } else {
                    stage
                },
                energy,
            })
    }

    fn ground_mark() -> impl Strategy<Value = GroundMarkView> {
        (any::<u32>(), select(GroundMark::ALL), any::<u32>()).prop_map(|(id, kind, fossil)| {
            GroundMarkView {
                id,
                kind,
                fossil: (kind == GroundMark::Sinkhole).then_some(fossil),
            }
        })
    }

    fn event() -> impl Strategy<Value = Option<ActiveEvent>> {
        proptest::option::of(
            (
                select(EventKind::ALL),
                any::<u32>(),
                select(Strength::ALL),
                select(Side::ALL),
                any::<u32>(),
            )
                .prop_map(|(kind, tick, strength, side, fossil)| ActiveEvent {
                    kind,
                    started_tick: Tick::new(tick),
                    strength,
                    side,
                    fossil: (kind == EventKind::Sinkhole).then_some(fossil),
                }),
        )
    }

    /// Valid `SceneInput`s across every variant of every enum.
    fn scene_input() -> impl Strategy<Value = SceneInput> {
        let living = (
            select(PresentationState::ALL),
            any::<u32>(),
            prop::collection::vec(plant(), 0..24),
            any::<(u8, u8, u8)>(),
            prop::collection::vec(resident(), 0..6),
        );
        let ground = (
            prop::collection::vec(
                (any::<u32>(), select(MonumentAge::ALL), 0..=MAX_MARKS),
                0..3,
            ),
            prop::collection::vec((any::<u32>(), select(FossilForm::ALL)), 0..5),
            prop::collection::vec(ground_mark(), 0..3),
        );
        let sky = (
            select(Weather::ALL),
            select(Light::ALL),
            select(Wind::ALL),
            0..=MAX_GLASS_LEVEL,
            0..=MAX_GLASS_LEVEL,
            event(),
        );
        (living, ground, sky).prop_map(
            |(
                (state, since, plants, (weeds, litter, fungus), residents),
                (monuments, fossils, ground_marks),
                (weather, light, wind, condensation, dust, event),
            )| SceneInput {
                presentation: Presentation {
                    state,
                    since_tick: Tick::new(since),
                },
                plants: unique_by_id(plants, |p| p.id),
                weeds,
                litter,
                fungus,
                residents: unique_by_id(residents, |r| r.id),
                monuments: unique_by_id(
                    monuments
                        .into_iter()
                        .map(|(id, age, marks)| MonumentView { id, age, marks })
                        .collect(),
                    |m| m.id,
                ),
                fossils: unique_by_id(
                    fossils
                        .into_iter()
                        .map(|(id, form)| FossilView { id, form })
                        .collect(),
                    |f| f.id,
                ),
                ground_marks: unique_by_id(ground_marks, |g| g.id),
                weather,
                light,
                wind,
                condensation,
                dust,
                event,
            },
        )
    }

    /// A small fixed scene whose encodings are pinned below.
    fn sample() -> SceneInput {
        SceneInput {
            presentation: Presentation {
                state: PresentationState::Recovering,
                since_tick: Tick::new(17_520),
            },
            plants: vec![
                PlantView {
                    id: 1,
                    species: PlantSpecies::Rowan,
                    stage: GrowthStage::Mature,
                    state: PlantState::Thriving,
                    bloom: true,
                    scorched: false,
                    order: 0,
                },
                PlantView {
                    id: 4,
                    species: PlantSpecies::WildflowerCluster,
                    stage: GrowthStage::Seedling,
                    state: PlantState::Pale,
                    bloom: false,
                    scorched: false,
                    order: 1,
                },
            ],
            weeds: 2,
            litter: 1,
            fungus: 0,
            residents: vec![ResidentView {
                id: 9,
                species: ResidentSpecies::Beetle,
                stage: LifeStage::Pupa,
                energy: Energy::Mid,
            }],
            monuments: vec![MonumentView {
                id: 2,
                age: MonumentAge::Lichened,
                marks: 3,
            }],
            fossils: vec![FossilView {
                id: 5,
                form: FossilForm::LeafPrint,
            }],
            ground_marks: vec![GroundMarkView {
                id: 6,
                kind: GroundMark::Sinkhole,
                fossil: Some(5),
            }],
            weather: Weather::Overcast,
            light: Light::Neutral,
            wind: Wind::Breezy,
            condensation: 0,
            dust: 0,
            event: Some(ActiveEvent {
                kind: EventKind::RainAfterDrought,
                started_tick: Tick::new(17_600),
                strength: Strength::High,
                side: Side::Right,
                fossil: None,
            }),
        }
    }

    const SAMPLE_DUMP: &str = "\
presentation.state = Recovering
presentation.since_tick = 17520
plants = [2]
plants[0].id = 1
plants[0].species = Rowan
plants[0].stage = Mature
plants[0].state = Thriving
plants[0].bloom = true
plants[0].scorched = false
plants[0].order = 0
plants[1].id = 4
plants[1].species = WildflowerCluster
plants[1].stage = Seedling
plants[1].state = Pale
plants[1].bloom = false
plants[1].scorched = false
plants[1].order = 1
weeds = 2
litter = 1
fungus = 0
residents = [1]
residents[0].id = 9
residents[0].species = Beetle
residents[0].stage = Pupa
residents[0].energy = Mid
monuments = [1]
monuments[0].id = 2
monuments[0].age = Lichened
monuments[0].marks = 3
fossils = [1]
fossils[0].id = 5
fossils[0].form = LeafPrint
ground_marks = [1]
ground_marks[0].id = 6
ground_marks[0].kind = Sinkhole
ground_marks[0].fossil.some = 5
weather = Overcast
light = Neutral
wind = Breezy
condensation = 0
dust = 0
event.some.kind = RainAfterDrought
event.some.started_tick = 17600
event.some.strength = High
event.some.side = Right
event.some.fossil = None
";

    #[test]
    fn the_sample_encodings_are_pinned() {
        // Both encodings are part of truth's output. If this fails, the format changed: bump
        // SIM_VERSION and re-pin, together.
        assert_eq!(SIM_VERSION, SimVersion(3));
        let scene = sample();
        assert_eq!(scene.validate(), Ok(()));
        assert_eq!(canonical_dump(&scene), SAMPLE_DUMP);
        assert_eq!(to_hex(&canonical_sha256(&scene)), SAMPLE_SHA256);
        let bytes = postcard::to_allocvec(&scene).expect("encodes");
        assert_eq!(to_hex(&bytes), SAMPLE_POSTCARD);
    }

    /// `shasum -a 256` of `SAMPLE_DUMP`.
    const SAMPLE_SHA256: &str = "4130942d026cd6ae01081c12dc687fd878b26366325d9b667ecb44c990027b00";
    /// Derived by hand from postcard's wire format: varints for `u16`, `u32`, lengths and variant
    /// indices, raw bytes for `u8` and `bool`, 0 or 1 before an option's value.
    const SAMPLE_POSTCARD: &str = "05f08801020102020001000004090001000001020100010902020101020103\
                                   010502010601010501000100000108c08901020100";

    /// Postcard writes a unit variant as its declaration index, so `ALL` in declaration order
    /// means `ALL[i]` encodes as `i`.
    fn assert_all_in_encoding_order<T: Serialize>(all: &[T]) {
        for (index, variant) in all.iter().enumerate() {
            let index = u8::try_from(index).expect("few variants");
            assert_eq!(postcard::to_allocvec(variant).expect("encodes"), [index]);
        }
    }

    #[test]
    fn every_all_is_in_encoding_order() {
        assert_all_in_encoding_order(PresentationState::ALL);
        assert_all_in_encoding_order(PlantSpecies::ALL);
        assert_all_in_encoding_order(GrowthStage::ALL);
        assert_all_in_encoding_order(PlantState::ALL);
        assert_all_in_encoding_order(ResidentSpecies::ALL);
        assert_all_in_encoding_order(LifeStage::ALL);
        assert_all_in_encoding_order(Energy::ALL);
        assert_all_in_encoding_order(MonumentAge::ALL);
        assert_all_in_encoding_order(FossilForm::ALL);
        assert_all_in_encoding_order(GroundMark::ALL);
        assert_all_in_encoding_order(Weather::ALL);
        assert_all_in_encoding_order(Light::ALL);
        assert_all_in_encoding_order(Wind::ALL);
        assert_all_in_encoding_order(EventKind::ALL);
        assert_all_in_encoding_order(Strength::ALL);
        assert_all_in_encoding_order(Side::ALL);
        assert_all_in_encoding_order(crate::domain::ObservationKind::ALL);
        assert_all_in_encoding_order(crate::domain::HealthClass::ALL);
        assert_all_in_encoding_order(crate::domain::ActivityClass::ALL);
    }

    #[test]
    fn event_kind_ids_are_pinned() {
        let ids: Vec<(&str, u8)> = EventKind::ALL.iter().map(|k| (k.name(), k.id())).collect();
        assert_eq!(
            ids,
            [
                ("Storm", 0),
                ("Clearing", 1),
                ("Lightning", 2),
                ("Dusk", 3),
                ("Sunburst", 4),
                ("ClearingBurn", 5),
                ("Flowering", 6),
                ("Windstorm", 7),
                ("RainAfterDrought", 8),
                ("Earthquake", 9),
                ("Sinkhole", 10),
            ]
        );
        for kind in EventKind::ALL {
            assert_eq!(postcard::to_allocvec(kind).expect("encodes"), [kind.id()]);
        }
    }

    #[test]
    fn enums_have_the_art_counts() {
        assert_eq!(PlantSpecies::ALL.len(), 12);
        assert_eq!(ResidentSpecies::ALL.len(), 8);
        assert_eq!(EventKind::ALL.len(), 11);
        let pupal: Vec<_> = ResidentSpecies::ALL
            .iter()
            .filter(|s| s.has_pupa())
            .map(|s| s.name())
            .collect();
        assert_eq!(pupal, ["Ladybird", "Beetle", "Firefly"]);
    }

    #[test]
    fn validate_refuses_each_broken_rule() {
        let broken = |change: fn(&mut SceneInput)| {
            let mut scene = sample();
            change(&mut scene);
            scene.validate()
        };
        assert_eq!(
            broken(|s| s.monuments[0].marks = MAX_MARKS + 1),
            Err(SceneInputError::TooManyMarks {
                monument: 2,
                marks: 4
            })
        );
        assert_eq!(
            broken(|s| s.condensation = 4),
            Err(SceneInputError::CondensationTooHigh(4))
        );
        assert_eq!(broken(|s| s.dust = 4), Err(SceneInputError::DustTooHigh(4)));
        assert_eq!(
            broken(|s| s.residents[0].species = ResidentSpecies::Snail),
            Err(SceneInputError::PupaWithoutArt { resident: 9 })
        );
        assert_eq!(
            broken(|s| s.plants[1].id = 1),
            Err(SceneInputError::DuplicateId {
                list: "plants",
                id: 1
            })
        );
        assert_eq!(
            broken(|s| s.event.as_mut().expect("sample has an event").fossil = Some(5)),
            Err(SceneInputError::EventFossil)
        );
        assert_eq!(
            broken(|s| s.event.as_mut().expect("sample has an event").kind = EventKind::Sinkhole),
            Err(SceneInputError::EventFossil)
        );
        assert_eq!(
            broken(|s| s.ground_marks[0].fossil = None),
            Err(SceneInputError::GroundMarkFossil { mark: 6 })
        );
        assert_eq!(
            broken(|s| s.ground_marks[0].kind = GroundMark::Crack),
            Err(SceneInputError::GroundMarkFossil { mark: 6 })
        );
    }

    proptest! {
        #![proptest_config(proptest_config())]

        #[test]
        fn scene_inputs_round_trip(scene in scene_input()) {
            prop_assert_eq!(scene.validate(), Ok(()));
            let bytes = postcard::to_allocvec(&scene).expect("encodes");
            prop_assert_eq!(&postcard::from_bytes::<SceneInput>(&bytes).expect("decodes"), &scene);
            let dump = canonical_dump(&scene);
            prop_assert_eq!(from_canonical_dump::<SceneInput>(&dump), Ok(scene));
        }
    }
}
