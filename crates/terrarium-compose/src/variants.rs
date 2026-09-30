//! The variant table: every `SceneInput` variant and the art that depicts it (`xtask assets`,
//! check 6).
//!
//! A variant name is the art's structured key in Pascal case (`TallGrass` is `tall-grass`), so
//! plants and residents are found by their structured fields, never by splitting IDs. The rest
//! of the art carries no structured fields, so those variants name their sprite IDs outright.
//! Where the art and the enums are not one to one, the mapping is decided here and the art is
//! left as delivered:
//!
//! - `LifeStage::Egg` is the art's `egg`, the woodlouse's `brood` or the earthworm's `cocoon`.
//! - `LifeStage::Pupa` exists only where `ResidentSpecies::has_pupa` holds. That is checked
//!   against the art both ways.
//! - `LifeStage::Dying` is the adult `death` routine.
//! - `GroundMark::Sinkhole` is the settled frame, `tmp.ground.sinkhole.f04`.
//! - Carved marks have four designs for three placement points.
//! - `Wind` has no tier sprites: the tiers differ in sway amplitude, and a breeze or a gale
//!   uses the leaf art.
//! - `EventKind` goes through the events manifest's `reusedActiveIds` groups plus the sprites
//!   tagged with the event. Storm and Clearing share the storm front and body.
//! - The art tags the flower sprinkles `clearing-burn`, so they belong to `ClearingBurn` here.
//!
//! Some enums have no art by design; [`NO_ART`] lists them with the reason.

use std::fmt;

use terrarium_truth::scene::{
    EventKind, FossilForm, GroundMark, GrowthStage, LifeStage, Light, MAX_GLASS_LEVEL, MAX_MARKS,
    MonumentAge, PlantSpecies, PlantState, ResidentSpecies, Weather, Wind,
};

use crate::catalog::{AnimationId, Catalog, Sprite, SpriteId};

/// Art a variant needs that the catalog lacks, or holds more than once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingArt {
    pub variant: String,
    pub wanted: String,
}

impl fmt::Display for MissingArt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.variant, self.wanted)
    }
}

/// The enums that have no sprites, and why.
pub const NO_ART: &[(&str, &str)] = &[
    (
        "Energy",
        "a resident's energy shows in which routine it plays, not in its own art",
    ),
    ("Strength", "an event's strength scales its choreography"),
    (
        "PresentationState",
        "the presentation state is the whole scene, told by plants, sky and glass",
    ),
    (
        "Side",
        "the side the storm front enters from; the art is mirrored",
    ),
    ("Light::Neutral", "neutral light draws no overlay"),
    ("Wind::Calm", "still air draws no leaves and no sway"),
];

/// A Pascal-case variant name as the art's kebab-case key.
pub fn art_key(name: &str) -> String {
    let mut key = String::with_capacity(name.len() + 2);
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            key.push('-');
        }
        key.push(c.to_ascii_lowercase());
    }
    key
}

/// How a plant in a given stage and state is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlantArt {
    /// Seedlings are one combined image.
    Seedling(SpriteId),
    /// The shared stem, then the state's foliage at the same ground point. Dead foliage
    /// carries its bleached override of the stem.
    Parts { stem: SpriteId, foliage: SpriteId },
}

pub fn plant(
    catalog: &Catalog,
    species: PlantSpecies,
    growth: GrowthStage,
    health: PlantState,
) -> Result<PlantArt, MissingArt> {
    let variant = format!(
        "PlantSpecies::{} × GrowthStage::{} × PlantState::{}",
        species.name(),
        growth.name(),
        health.name()
    );
    let keys = PlantKeys {
        species: art_key(species.name()),
        growth: art_key(growth.name()),
        health: art_key(health.name()),
    };
    let find = |what: &str, part: &str, with_health: bool| {
        one(catalog, &variant, what, |sprite| {
            keys.matches(sprite, part, with_health)
        })
    };
    if growth == GrowthStage::Seedling {
        return find("a combined seedling", "combined", true).map(PlantArt::Seedling);
    }
    let stem = find("a stem", "stem", false)?;
    let foliage = find("a foliage", "foliage", true)?;
    Ok(PlantArt::Parts { stem, foliage })
}

/// A plant's structured fields, as the art spells them.
struct PlantKeys {
    species: String,
    growth: String,
    health: String,
}

impl PlantKeys {
    /// Whether a sprite is this plant's `part`. A stem has no state, so it is found without
    /// one.
    fn matches(&self, sprite: &Sprite, part: &str, with_health: bool) -> bool {
        let keys = &sprite.keys;
        let health = with_health.then_some(self.health.as_str());
        keys.event.is_none()
            && keys.part.as_deref() == Some(part)
            && keys.species.as_deref() == Some(self.species.as_str())
            && keys.stage.as_deref() == Some(self.growth.as_str())
            && keys.state.as_deref() == health
    }
}

/// The bloom overlay, drawn only on a mature, thriving plant whose `bloom` truth set.
pub fn bloom(catalog: &Catalog, species: PlantSpecies) -> Result<SpriteId, MissingArt> {
    let sp = art_key(species.name());
    one(
        catalog,
        &format!("PlantSpecies::{} bloom", species.name()),
        "a mature bloom",
        |sprite| {
            let keys = &sprite.keys;
            keys.event.is_none()
                && keys.part.as_deref() == Some("bloom")
                && keys.species.as_deref() == Some(sp.as_str())
                && keys.stage.as_deref() == Some("mature")
        },
    )
}

/// The scorched foliage that stands in for a mature plant's while lightning's scorch plays.
pub fn scorch(catalog: &Catalog, species: PlantSpecies) -> Result<SpriteId, MissingArt> {
    let sp = art_key(species.name());
    one(
        catalog,
        &format!("PlantSpecies::{} scorched", species.name()),
        "a lightning scorch with its eligibility",
        |sprite| {
            let keys = &sprite.keys;
            sprite.scorch.is_some()
                && keys.event.as_deref() == Some("lightning")
                && keys.part.as_deref() == Some("foliage")
                && keys.species.as_deref() == Some(sp.as_str())
                && keys.stage.as_deref() == Some("mature")
        },
    )
}

/// The art's life stage for a resident's, or `None` for a pupa the species does not have.
pub fn art_life_stage(species: ResidentSpecies, stage: LifeStage) -> Option<&'static str> {
    match stage {
        LifeStage::Egg => Some(match species {
            ResidentSpecies::Woodlouse => "brood",
            ResidentSpecies::Earthworm => "cocoon",
            _ => "egg",
        }),
        LifeStage::Juvenile => Some("juvenile"),
        LifeStage::Pupa => species.has_pupa().then_some("pupa"),
        LifeStage::Adult | LifeStage::Dying => Some("adult"),
    }
}

/// The routines a resident may play in a stage, sorted by name. `Dying` is the death routine
/// alone, held until truth removes the resident.
pub fn resident(
    catalog: &Catalog,
    species: ResidentSpecies,
    stage: LifeStage,
) -> Result<Vec<AnimationId>, MissingArt> {
    let variant = format!(
        "ResidentSpecies::{} × LifeStage::{}",
        species.name(),
        stage.name()
    );
    let Some(life_stage) = art_life_stage(species, stage) else {
        return Err(MissingArt {
            variant,
            wanted: "a pupa stage, which this species does not have".into(),
        });
    };
    let sp = art_key(species.name());
    let routines: Vec<AnimationId> = animations(catalog, &sp, life_stage)
        .filter(|&id| {
            stage != LifeStage::Dying
                || catalog.animation(id).keys.routine.as_deref() == Some("death")
        })
        .collect();
    let expected = if stage == LifeStage::Dying {
        "one adult death routine"
    } else {
        "routines"
    };
    match routines.len() {
        0 => Err(MissingArt {
            variant,
            wanted: format!("{expected} for {sp} {life_stage}"),
        }),
        n if n > 1 && stage == LifeStage::Dying => Err(MissingArt {
            variant,
            wanted: format!("one death routine, not {n}"),
        }),
        _ => Ok(routines),
    }
}

/// The stone of a monument's age, and its night lichen glow once it has lichen.
pub fn monument(catalog: &Catalog, age: MonumentAge) -> Result<Vec<SpriteId>, MissingArt> {
    let ids: &[&str] = match age {
        MonumentAge::Young => &["tmp.monument.stone.young"],
        MonumentAge::Lichened => &[
            "tmp.monument.stone.lichened",
            "tmp.monument.lichen-glow.lichened",
        ],
        MonumentAge::Ancient => &[
            "tmp.monument.stone.ancient",
            "tmp.monument.lichen-glow.ancient",
        ],
    };
    named(catalog, &format!("MonumentAge::{}", age.name()), ids)
}

/// The carved-mark designs: four of them, for the stone's three placement points.
pub const CARVED_MARKS: [&str; 4] = [
    "tmp.monument.carved-mark.01",
    "tmp.monument.carved-mark.02",
    "tmp.monument.carved-mark.03",
    "tmp.monument.carved-mark.04",
];

pub fn carved_marks(catalog: &Catalog) -> Result<Vec<SpriteId>, MissingArt> {
    named(catalog, "MonumentView.marks", &CARVED_MARKS)
}

pub fn fossil(catalog: &Catalog, form: FossilForm) -> Result<SpriteId, MissingArt> {
    let id = match form {
        FossilForm::Shell => "tmp.fossil.shell",
        FossilForm::Bone => "tmp.fossil.bone",
        FossilForm::LeafPrint => "tmp.fossil.leaf-print",
    };
    named(catalog, &format!("FossilForm::{}", form.name()), &[id]).map(|ids| ids[0])
}

/// A mark that outlasts its event: the settled crack, the settled sinkhole, or either fresh
/// soil patch.
pub fn ground_mark(catalog: &Catalog, kind: GroundMark) -> Result<Vec<SpriteId>, MissingArt> {
    let ids: &[&str] = match kind {
        GroundMark::Crack => &["tmp.ground.crack.settled"],
        GroundMark::Sinkhole => &["tmp.ground.sinkhole.f04"],
        GroundMark::FreshSoil => &["tmp.soil.fresh-patch.01", "tmp.soil.fresh-patch.02"],
    };
    named(catalog, &format!("GroundMark::{}", kind.name()), ids)
}

/// The two films the glass can carry, each in levels 1 to `MAX_GLASS_LEVEL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlassFilm {
    Condensation,
    Dust,
}

const CONDENSATION: [&str; 3] = [
    "tmp.glass.condensation.1",
    "tmp.glass.condensation.2",
    "tmp.glass.condensation.3",
];
const DUST: [&str; 3] = ["tmp.glass.dust.1", "tmp.glass.dust.2", "tmp.glass.dust.3"];

/// The film at a level; level 0 draws nothing.
pub fn glass(
    catalog: &Catalog,
    film: GlassFilm,
    level: u8,
) -> Result<Option<SpriteId>, MissingArt> {
    if level == 0 {
        return Ok(None);
    }
    let variant = format!("{film:?} {level}");
    let ids = match film {
        GlassFilm::Condensation => &CONDENSATION,
        GlassFilm::Dust => &DUST,
    };
    match ids.get(usize::from(level) - 1) {
        Some(id) => named(catalog, &variant, &[id]).map(|ids| Some(ids[0])),
        None => Err(MissingArt {
            variant,
            wanted: "a film for this level".into(),
        }),
    }
}

pub fn weather(catalog: &Catalog, weather: Weather) -> Result<Vec<SpriteId>, MissingArt> {
    let ids: &[&str] = match weather {
        Weather::Clear => &["tmp.sky.clear.tile"],
        Weather::Overcast => &["tmp.sky.overcast.tile"],
        Weather::Storm => &["tmp.sky.storm.body", "tmp.sky.storm.front"],
        Weather::Haze => &["tmp.sky.haze.tile"],
    };
    named(catalog, &format!("Weather::{}", weather.name()), ids)
}

/// Dusk multiplies and sunburst adds over the scene; neutral light has no overlay.
pub fn light(catalog: &Catalog, light: Light) -> Result<Option<SpriteId>, MissingArt> {
    let id = match light {
        Light::Neutral => return Ok(None),
        Light::Dusk => "tmp.sky.light.dusk",
        Light::Sunburst => "tmp.sky.light.sunburst",
    };
    named(catalog, &format!("Light::{}", light.name()), &[id]).map(|ids| Some(ids[0]))
}

/// A breeze and a gale share the leaf art and differ in sway amplitude; calm has neither.
pub fn wind(catalog: &Catalog, wind: Wind) -> Result<Vec<SpriteId>, MissingArt> {
    match wind {
        Wind::Calm => Ok(Vec::new()),
        Wind::Breezy | Wind::Gale => group(catalog, &format!("Wind::{}", wind.name()), "wind", &[]),
    }
}

/// An event's art: the reused groups it plays, the members it picks from them, and the
/// sprites tagged with it.
struct EventArt {
    groups: &'static [&'static str],
    only: &'static [&'static str],
    tag: Option<&'static str>,
}

const fn groups(groups: &'static [&'static str]) -> EventArt {
    EventArt {
        groups,
        only: &[],
        tag: None,
    }
}

const fn event_art(kind: EventKind) -> EventArt {
    match kind {
        EventKind::Storm | EventKind::Clearing => groups(&["stormAndClearing"]),
        EventKind::Lightning => EventArt {
            groups: &["lightning"],
            only: &[],
            tag: Some("lightning"),
        },
        EventKind::Dusk => EventArt {
            groups: &["light"],
            only: &["tmp.sky.light.dusk"],
            tag: None,
        },
        EventKind::Sunburst => EventArt {
            groups: &["light"],
            only: &["tmp.sky.light.sunburst"],
            tag: None,
        },
        EventKind::ClearingBurn => EventArt {
            groups: &["freshSoil"],
            only: &[],
            tag: Some("clearing-burn"),
        },
        EventKind::Flowering => groups(&["bloom"]),
        EventKind::Windstorm => groups(&["wind"]),
        EventKind::RainAfterDrought => groups(&["rain"]),
        EventKind::Earthquake => groups(&["earthquake"]),
        EventKind::Sinkhole => groups(&["sinkhole", "fossils"]),
    }
}

/// Every sprite the event plays, sorted and without repeats.
pub fn event(catalog: &Catalog, kind: EventKind) -> Result<Vec<SpriteId>, MissingArt> {
    let variant = format!("EventKind::{}", kind.name());
    let art = event_art(kind);
    let mut ids = Vec::new();
    for name in art.groups {
        ids.extend(group(catalog, &variant, name, art.only)?);
    }
    if let Some(tag) = art.tag {
        let tagged: Vec<SpriteId> = sprite_ids(catalog)
            .filter(|&id| catalog.sprite(id).keys.event.as_deref() == Some(tag))
            .collect();
        if tagged.is_empty() {
            return Err(MissingArt {
                variant,
                wanted: format!("sprites tagged `{tag}`"),
            });
        }
        ids.extend(tagged);
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// Every gap between the `SceneInput` variants and the catalog. Empty when the art depicts
/// all of them; `cargo xtask assets` refuses a package that leaves any.
pub fn missing(catalog: &Catalog) -> Vec<MissingArt> {
    let mut gaps = Vec::new();
    let mut note = |result: Result<(), MissingArt>| {
        if let Err(gap) = result {
            gaps.push(gap);
        }
    };
    for &species in PlantSpecies::ALL {
        for &stage in GrowthStage::ALL {
            for &state in PlantState::ALL {
                note(plant(catalog, species, stage, state).map(drop));
            }
        }
        note(bloom(catalog, species).map(drop));
        note(scorch(catalog, species).map(drop));
    }
    for &species in ResidentSpecies::ALL {
        for &stage in LifeStage::ALL {
            if stage == LifeStage::Pupa && !species.has_pupa() {
                note(no_pupa_art(catalog, species));
            } else {
                note(resident(catalog, species, stage).map(drop));
            }
        }
    }
    for &age in MonumentAge::ALL {
        note(monument(catalog, age).map(drop));
    }
    note(carved_marks(catalog).and_then(|marks| {
        if marks.len() >= usize::from(MAX_MARKS) {
            Ok(())
        } else {
            Err(MissingArt {
                variant: "MonumentView.marks".into(),
                wanted: format!("{MAX_MARKS} designs"),
            })
        }
    }));
    for &form in FossilForm::ALL {
        note(fossil(catalog, form).map(drop));
    }
    for &kind in GroundMark::ALL {
        note(ground_mark(catalog, kind).map(drop));
    }
    for film in [GlassFilm::Condensation, GlassFilm::Dust] {
        for level in 1..=MAX_GLASS_LEVEL {
            note(glass(catalog, film, level).map(drop));
        }
    }
    for &kind in Weather::ALL {
        note(weather(catalog, kind).map(drop));
    }
    for &kind in Light::ALL {
        note(light(catalog, kind).map(drop));
    }
    for &kind in Wind::ALL {
        note(wind(catalog, kind).map(drop));
    }
    for &kind in EventKind::ALL {
        note(event(catalog, kind).map(drop));
    }
    gaps
}

/// A species without a pupa has no pupa art, so `has_pupa` and the art agree both ways.
fn no_pupa_art(catalog: &Catalog, species: ResidentSpecies) -> Result<(), MissingArt> {
    let sp = art_key(species.name());
    if animations(catalog, &sp, "pupa").next().is_none() {
        Ok(())
    } else {
        Err(MissingArt {
            variant: format!("ResidentSpecies::{} × LifeStage::Pupa", species.name()),
            wanted: "no pupa art, since has_pupa is false".into(),
        })
    }
}

fn sprite_ids(catalog: &Catalog) -> impl Iterator<Item = SpriteId> + '_ {
    (0..catalog.sprites.len()).filter_map(SpriteId::from_index)
}

fn animations<'a>(
    catalog: &'a Catalog,
    species: &'a str,
    life_stage: &'a str,
) -> impl Iterator<Item = AnimationId> + 'a {
    (0..catalog.animations.len())
        .filter_map(AnimationId::from_index)
        .filter(move |&id| {
            let keys = &catalog.animation(id).keys;
            keys.species.as_deref() == Some(species)
                && keys.life_stage.as_deref() == Some(life_stage)
        })
}

/// The one sprite matching `wanted`: none, or more than one, is a gap.
fn one(
    catalog: &Catalog,
    variant: &str,
    what: &str,
    wanted: impl Fn(&Sprite) -> bool,
) -> Result<SpriteId, MissingArt> {
    let mut found = sprite_ids(catalog).filter(|&id| wanted(catalog.sprite(id)));
    match (found.next(), found.next()) {
        (Some(id), None) => Ok(id),
        (None, _) => Err(MissingArt {
            variant: variant.into(),
            wanted: what.into(),
        }),
        (Some(_), Some(_)) => Err(MissingArt {
            variant: variant.into(),
            wanted: format!("one {what}, not several"),
        }),
    }
}

fn named(catalog: &Catalog, variant: &str, ids: &[&str]) -> Result<Vec<SpriteId>, MissingArt> {
    ids.iter()
        .map(|id| {
            catalog.id(id).ok_or_else(|| MissingArt {
                variant: variant.into(),
                wanted: format!("`{id}`"),
            })
        })
        .collect()
}

/// A reused group, or only the named members of it.
fn group(
    catalog: &Catalog,
    variant: &str,
    name: &str,
    only: &[&str],
) -> Result<Vec<SpriteId>, MissingArt> {
    let gap = |wanted: String| MissingArt {
        variant: variant.into(),
        wanted,
    };
    let Some(group) = catalog
        .event_group(name)
        .filter(|group| !group.sprites.is_empty())
    else {
        return Err(gap(format!("the reused group `{name}`")));
    };
    if only.is_empty() {
        return Ok(group.sprites.clone());
    }
    only.iter()
        .map(|id| {
            catalog
                .id(id)
                .filter(|sprite| group.sprites.contains(sprite))
                .ok_or_else(|| gap(format!("`{id}` in the reused group `{name}`")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Art;

    fn catalog() -> &'static Catalog {
        &Art::embedded().catalog
    }

    #[test]
    fn the_embedded_art_depicts_every_variant() {
        let gaps = missing(catalog());
        assert!(gaps.is_empty(), "art gaps:\n{}", lines(&gaps));
    }

    #[test]
    fn seedlings_are_combined_and_older_plants_are_parts() {
        for &species in PlantSpecies::ALL {
            for &state in PlantState::ALL {
                let seedling = plant(catalog(), species, GrowthStage::Seedling, state).unwrap();
                assert!(matches!(seedling, PlantArt::Seedling(_)));
                for stage in [GrowthStage::Young, GrowthStage::Mature] {
                    let PlantArt::Parts { stem, foliage } =
                        plant(catalog(), species, stage, state).unwrap()
                    else {
                        panic!("{species:?} {stage:?} is drawn from parts");
                    };
                    assert_eq!(catalog().sprite(stem).keys.part.as_deref(), Some("stem"));
                    assert_eq!(
                        catalog().sprite(foliage).keys.part.as_deref(),
                        Some("foliage")
                    );
                }
            }
        }
    }

    #[test]
    fn pupa_art_exists_exactly_where_has_pupa_holds() {
        let pupal: Vec<&str> = ResidentSpecies::ALL
            .iter()
            .filter(|species| resident(catalog(), **species, LifeStage::Pupa).is_ok())
            .map(|species| species.name())
            .collect();
        assert_eq!(pupal, ["Ladybird", "Beetle", "Firefly"]);
        for &species in ResidentSpecies::ALL.iter().filter(|s| !s.has_pupa()) {
            assert_eq!(no_pupa_art(catalog(), species), Ok(()));
        }
    }

    #[test]
    fn the_egg_is_a_brood_for_the_woodlouse_and_a_cocoon_for_the_earthworm() {
        let egg = |species| art_life_stage(species, LifeStage::Egg);
        assert_eq!(egg(ResidentSpecies::Woodlouse), Some("brood"));
        assert_eq!(egg(ResidentSpecies::Earthworm), Some("cocoon"));
        assert_eq!(egg(ResidentSpecies::Snail), Some("egg"));
        for &species in ResidentSpecies::ALL {
            let hatch = format!(
                "{}.{}.hatch",
                art_key(species.name()),
                egg(species).unwrap()
            );
            assert!(catalog().animation_id(&hatch).is_some(), "{hatch}");
        }
    }

    #[test]
    fn dying_is_the_adult_death_routine_alone() {
        for &species in ResidentSpecies::ALL {
            let dying = resident(catalog(), species, LifeStage::Dying).unwrap();
            let names: Vec<&str> = dying
                .iter()
                .map(|&id| catalog().animation(id).name.as_str())
                .collect();
            assert_eq!(names, [format!("{}.adult.death", art_key(species.name()))]);
        }
    }

    #[test]
    fn variant_names_become_the_arts_kebab_case_keys() {
        assert_eq!(art_key(PlantSpecies::TallGrass.name()), "tall-grass");
        assert_eq!(
            art_key(PlantSpecies::WildflowerCluster.name()),
            "wildflower-cluster"
        );
        assert_eq!(art_key(PlantSpecies::MossMound.name()), "moss-mound");
        assert_eq!(art_key(FossilForm::LeafPrint.name()), "leaf-print");
        assert_eq!(art_key(ResidentSpecies::Beetle.name()), "beetle");
        assert_eq!(art_key(GrowthStage::Seedling.name()), "seedling");
    }

    #[test]
    fn the_variants_without_art_draw_nothing() {
        assert_eq!(light(catalog(), Light::Neutral), Ok(None));
        assert_eq!(wind(catalog(), Wind::Calm), Ok(Vec::new()));
        assert_eq!(glass(catalog(), GlassFilm::Dust, 0), Ok(None));
        let named: Vec<&str> = NO_ART.iter().map(|(name, _)| *name).collect();
        for name in [
            "Energy",
            "Strength",
            "PresentationState",
            "Side",
            "Light::Neutral",
            "Wind::Calm",
        ] {
            assert!(named.contains(&name), "{name}");
        }
    }

    #[test]
    fn a_film_past_the_last_level_is_a_gap() {
        assert!(glass(catalog(), GlassFilm::Condensation, MAX_GLASS_LEVEL + 1).is_err());
        assert_eq!(CONDENSATION.len(), usize::from(MAX_GLASS_LEVEL));
        assert_eq!(DUST.len(), usize::from(MAX_GLASS_LEVEL));
    }

    #[test]
    fn storm_and_clearing_share_their_art_and_dusk_takes_only_its_own() {
        let storm = event(catalog(), EventKind::Storm).unwrap();
        assert_eq!(storm, event(catalog(), EventKind::Clearing).unwrap());
        let dusk = event(catalog(), EventKind::Dusk).unwrap();
        assert_eq!(dusk, [catalog().id("tmp.sky.light.dusk").unwrap()]);
        let lightning = event(catalog(), EventKind::Lightning).unwrap();
        assert!(
            lightning
                .iter()
                .any(|&id| catalog().sprite(id).scorch.is_some())
        );
    }

    #[test]
    fn an_event_takes_only_members_of_its_reused_group() {
        let mut broken = catalog().clone();
        let dusk = broken.id("tmp.sky.light.dusk").unwrap();
        let light = broken
            .events
            .iter_mut()
            .find(|group| group.name == "light")
            .unwrap();
        light.sprites.retain(|&id| id != dusk);
        let gaps = lines(&missing(&broken));
        assert_eq!(
            gaps,
            "EventKind::Dusk: `tmp.sky.light.dusk` in the reused group `light`"
        );
    }

    #[test]
    fn a_missing_foliage_is_a_gap_named_for_its_variant() {
        let mut broken = catalog().clone();
        let id = broken.id("tmp.plant.oak.foliage.young.pale").unwrap();
        broken.sprites[id.index()].keys.state = Some("pallid".into());
        let gaps = missing(&broken);
        assert_eq!(
            lines(&gaps),
            "PlantSpecies::Oak × GrowthStage::Young × PlantState::Pale: a foliage"
        );
    }

    #[test]
    fn pupa_art_for_a_species_without_one_is_a_gap() {
        let mut broken = catalog().clone();
        let hatch = broken.animation_id("woodlouse.brood.hatch").unwrap();
        broken.animations[hatch.index()].keys.life_stage = Some("pupa".into());
        let gaps = lines(&missing(&broken));
        assert!(
            gaps.contains("ResidentSpecies::Woodlouse × LifeStage::Pupa: no pupa art"),
            "{gaps}"
        );
    }

    fn lines(gaps: &[MissingArt]) -> String {
        gaps.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
}
