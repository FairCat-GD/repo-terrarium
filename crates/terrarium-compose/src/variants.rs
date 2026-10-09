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
//!   tagged with the event. Storm and Clearing share the storm front and body. The sky's
//!   events also resolve each piece in its role ([`SkyEventArt`]): the front's curtain and the
//!   sequences lightning, rain and the wind's leaves play.
//! - The art tags the flower sprinkles `clearing-burn`, so they belong to `ClearingBurn` here.
//!
//! Some enums have no art by design; [`NO_ART`] lists them with the reason.
//!
//! The table also holds what the composer draws beyond the variants: each species' plant layer,
//! which is its depth band, the filler that keeps the slots from standing empty (ground tufts
//! and dead-stem litter), and the world's own art ([`WorldArt`]). Check 6 also holds the world
//! to what the composer relies on: every glow is paired with the sprite it lights, and every
//! carved mark lists the same [`MAX_MARKS`] points on its stone.
//!
//! [`ArtTable`] resolves every variant once, so the composer reads maps instead of scanning the
//! catalog. [`missing`], the gaps it finds, is check 6 of `cargo xtask assets`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use terrarium_truth::scene::{
    EventKind, FossilForm, GroundMark, GrowthStage, LifeStage, Light, MAX_GLASS_LEVEL, MAX_MARKS,
    MonumentAge, PlantSpecies, PlantState, ResidentSpecies, Weather, Wind,
};

use crate::catalog::{AnimationId, Art, Catalog, Flags, Layer, Next, Point, Sprite, SpriteId};
use crate::sky::CloudSize;
use crate::text::Label;

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

fn plant(
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
fn bloom(catalog: &Catalog, species: PlantSpecies) -> Result<SpriteId, MissingArt> {
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
fn scorch(catalog: &Catalog, species: PlantSpecies) -> Result<SpriteId, MissingArt> {
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
fn resident(
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

/// Each effect that plays in step with a routine, by the routine.
fn lanterns(catalog: &Catalog) -> BTreeMap<AnimationId, AnimationId> {
    (0..catalog.animations.len())
        .filter_map(AnimationId::from_index)
        .filter_map(|effect| Some((catalog.animation(effect).sync?, effect)))
        .collect()
}

/// The stone of a monument's age, and its night lichen glow once it has lichen.
fn monument(catalog: &Catalog, age: MonumentAge) -> Result<Vec<SpriteId>, MissingArt> {
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
    let found = named(catalog, &format!("MonumentAge::{}", age.name()), ids)?;
    if let [stone, glow] = found[..] {
        paired(catalog, glow, stone)?;
    }
    Ok(found)
}

/// The carved-mark designs: four of them, for the stone's three placement points.
pub const CARVED_MARKS: [&str; 4] = [
    "tmp.monument.carved-mark.01",
    "tmp.monument.carved-mark.02",
    "tmp.monument.carved-mark.03",
    "tmp.monument.carved-mark.04",
];

fn carved_marks(catalog: &Catalog) -> Result<Vec<SpriteId>, MissingArt> {
    named(catalog, "MonumentView.marks", &CARVED_MARKS)
}

fn fossil(catalog: &Catalog, form: FossilForm) -> Result<SpriteId, MissingArt> {
    let id = match form {
        FossilForm::Shell => "tmp.fossil.shell",
        FossilForm::Bone => "tmp.fossil.bone",
        FossilForm::LeafPrint => "tmp.fossil.leaf-print",
    };
    named(catalog, &format!("FossilForm::{}", form.name()), &[id]).map(|ids| ids[0])
}

/// A mark that outlasts its event: the settled crack, the settled sinkhole, or either fresh
/// soil patch.
fn ground_mark(catalog: &Catalog, kind: GroundMark) -> Result<Vec<SpriteId>, MissingArt> {
    let ids: &[&str] = match kind {
        GroundMark::Crack => &["tmp.ground.crack.settled"],
        GroundMark::Sinkhole => &["tmp.ground.sinkhole.f04"],
        GroundMark::FreshSoil => &["tmp.soil.fresh-patch.01", "tmp.soil.fresh-patch.02"],
    };
    named(catalog, &format!("GroundMark::{}", kind.name()), ids)
}

/// Every part of a species, from seedling to bloom, in one plant layer: its depth band.
fn plant_layer(
    catalog: &Catalog,
    species: PlantSpecies,
    sprites: &[SpriteId],
) -> Result<Layer, MissingArt> {
    let layers: Vec<Option<Layer>> = sprites.iter().map(|&id| catalog.sprite(id).layer).collect();
    match layers.first() {
        Some(&Some(layer @ (Layer::FarPlants | Layer::MidPlants | Layer::NearPlants)))
            if layers.iter().all(|&l| l == Some(layer)) =>
        {
            Ok(layer)
        }
        _ => Err(MissingArt {
            variant: format!("PlantSpecies::{}", species.name()),
            wanted: "every part in one plant layer".into(),
        }),
    }
}

/// The ground tufts: the lower vegetation that fills a living scene's empty slots.
pub const TUFTS: [&str; 4] = [
    "tmp.ground.tuft.01",
    "tmp.ground.tuft.02",
    "tmp.ground.tuft.03",
    "tmp.ground.tuft.04",
];

/// The dead-stem litter that fills a dead scene's empty slots when it has no species to show.
pub const DEAD_STEMS: [&str; 2] = ["tmp.litter.dead-stems.01", "tmp.litter.dead-stems.02"];

/// The world's own art, by name: what the composer draws around the plants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldArt {
    pub pebbles: Vec<SpriteId>,
    /// Top-left, top-right, bottom-left, bottom-right. The glass has no top or bottom edge.
    pub corners: [SpriteId; 4],
    pub sun: SpriteId,
    /// `tmp.sky.moon.phase.1` to `.4`.
    pub moons: [SpriteId; 4],
    pub stars: Vec<SpriteId>,
    /// The side of the sun and of every moon, which are squares of one size.
    pub body_side: i32,
    /// The side of every star, all squares of one size.
    pub star_side: i32,
    /// Small, medium and large, each a list of equivalent clouds.
    pub clouds: [Vec<SpriteId>; 3],
    /// The single weeds, all equivalent.
    pub weeds: Vec<SpriteId>,
    /// The merged patches that stand in for several weeds.
    pub weed_patches: Vec<SpriteId>,
    /// The amber litter of a living or dormant scene; a dead one's is the dead stems.
    pub amber_litter: Vec<SpriteId>,
    /// Each cap cluster with the night glow paired with it.
    pub fungus: Vec<(SpriteId, SpriteId)>,
    pub plant_glow: SpriteId,
    /// Where each carved mark may sit: its top-left in its stone's pixels.
    pub mark_points: Vec<Point>,
    /// How long the dusk and sunburst overlays take to fade, as the art sets it.
    pub light_ms: u32,
    /// How many characters a line of the label holds.
    pub label_capacity: usize,
    /// The faint reflection that sweeps across the glass, and its width.
    pub sweep: SpriteId,
    pub sweep_width: i32,
}

impl WorldArt {
    pub fn clouds(&self, size: CloudSize) -> &[SpriteId] {
        match size {
            CloudSize::Small => &self.clouds[0],
            CloudSize::Medium => &self.clouds[1],
            CloudSize::Large => &self.clouds[2],
        }
    }
}

const PEBBLES: [&str; 4] = [
    "tmp.ground.pebble.01",
    "tmp.ground.pebble.02",
    "tmp.ground.pebble.03",
    "tmp.ground.pebble.04",
];
const CORNERS: [&str; 4] = [
    "tmp.glass.corner.top-left",
    "tmp.glass.corner.top-right",
    "tmp.glass.corner.bottom-left",
    "tmp.glass.corner.bottom-right",
];
const MOONS: [&str; 4] = [
    "tmp.sky.moon.phase.1",
    "tmp.sky.moon.phase.2",
    "tmp.sky.moon.phase.3",
    "tmp.sky.moon.phase.4",
];
const STARS: [&str; 3] = ["tmp.sky.star.1", "tmp.sky.star.2", "tmp.sky.star.3"];
const CLOUDS: [&[&str]; 3] = [
    &[
        "tmp.sky.cloud.small.01",
        "tmp.sky.cloud.small.02",
        "tmp.sky.cloud.small.03",
    ],
    &[
        "tmp.sky.cloud.medium.01",
        "tmp.sky.cloud.medium.02",
        "tmp.sky.cloud.medium.03",
    ],
    &["tmp.sky.cloud.large.01", "tmp.sky.cloud.large.02"],
];
const WEEDS: [&str; 6] = [
    "tmp.weed.creeping.01",
    "tmp.weed.creeping.02",
    "tmp.weed.tall.01",
    "tmp.weed.tall.02",
    "tmp.weed.burr.01",
    "tmp.weed.burr.02",
];
const WEED_PATCHES: [&str; 2] = ["tmp.weed.patch.01", "tmp.weed.patch.02"];
const AMBER_LITTER: [&str; 4] = [
    "tmp.litter.leaf-pile.01",
    "tmp.litter.leaf-pile.02",
    "tmp.litter.fallen-leaf.01",
    "tmp.litter.fallen-leaf.02",
];
/// Each cap cluster and its glow. The bracket fungus waits for dead wood to grow on.
const FUNGUS: [(&str, &str); 2] = [
    (
        "tmp.fungus.cap-cluster.01",
        "tmp.fungus.cap-cluster.01.glow",
    ),
    (
        "tmp.fungus.cap-cluster.02",
        "tmp.fungus.cap-cluster.02.glow",
    ),
];

/// The world's art, or every gap in it.
fn world(catalog: &Catalog) -> Result<WorldArt, Vec<MissingArt>> {
    let mut gaps = Vec::new();
    let mut get = |what: &str, ids: &[&str]| match named(catalog, what, ids) {
        Ok(found) => found,
        Err(gap) => {
            gaps.push(gap);
            Vec::new()
        }
    };
    let pebbles = get("pebbles", &PEBBLES);
    let corners = get("the glass corners", &CORNERS);
    let sun = get("the sun", &["tmp.sky.sun"]);
    let moons = get("the moon", &MOONS);
    let stars = get("the stars", &STARS);
    let clouds = CLOUDS.map(|ids| get("the clouds", ids));
    let weeds = get("weeds", &WEEDS);
    let weed_patches = get("weed patches", &WEED_PATCHES);
    let amber_litter = get("amber litter", &AMBER_LITTER);
    let fungus_bodies = get("fungus", &FUNGUS.map(|(body, _)| body));
    let fungus_glows = get("fungus glows", &FUNGUS.map(|(_, glow)| glow));
    let plant_glow = get("the plant glow", &["tmp.plant.night-glow"]);
    let sweep = get("the reflection sweep", &["tmp.glass.reflection-sweep"]);
    let fungus: Vec<(SpriteId, SpriteId)> = fungus_bodies.into_iter().zip(fungus_glows).collect();
    for &(body, glow) in &fungus {
        if let Err(gap) = paired(catalog, glow, body) {
            gaps.push(gap);
        }
    }
    let light_ms = match light_ms(catalog) {
        Ok(ms) => ms,
        Err(gap) => {
            gaps.push(gap);
            0
        }
    };
    let mark_points = match mark_points(catalog) {
        Ok(points) => points,
        Err(gap) => {
            gaps.push(gap);
            Vec::new()
        }
    };
    let (Ok(corners), Ok(moons), [sun], [plant_glow], [sweep]) = (
        <[SpriteId; 4]>::try_from(corners),
        <[SpriteId; 4]>::try_from(moons),
        sun.as_slice(),
        plant_glow.as_slice(),
        sweep.as_slice(),
    ) else {
        return Err(gaps);
    };
    let body_side = side(
        catalog,
        "the sun and the moons",
        [*sun].iter().chain(&moons),
    );
    let star_side = side(catalog, "the stars", stars.iter());
    let label_capacity = Label::new(catalog).map(|label| label.capacity());
    if label_capacity.is_none() {
        gaps.push(MissingArt {
            variant: "the label".into(),
            wanted: "a plate with a text inset, and the body font".into(),
        });
    }
    let (body_side, star_side) = match (body_side, star_side) {
        (Ok(body), Ok(star)) => (body, star),
        (body, star) => {
            gaps.extend(body.err());
            gaps.extend(star.err());
            return Err(gaps);
        }
    };
    let Some(label_capacity) = label_capacity else {
        return Err(gaps);
    };
    if !gaps.is_empty() {
        return Err(gaps);
    }
    Ok(WorldArt {
        pebbles,
        corners,
        sun: *sun,
        moons,
        stars,
        body_side,
        star_side,
        clouds,
        weeds,
        weed_patches,
        amber_litter,
        fungus,
        plant_glow: *plant_glow,
        mark_points,
        light_ms,
        label_capacity,
        sweep: *sweep,
        sweep_width: i32::from(catalog.sprite(*sweep).rect.w),
    })
}

/// The side shared by sprites that must be squares of one size: the sky places them by it.
fn side<'a>(
    catalog: &Catalog,
    what: &str,
    mut ids: impl Iterator<Item = &'a SpriteId>,
) -> Result<i32, MissingArt> {
    let gap = || MissingArt {
        variant: what.into(),
        wanted: "squares of one size".into(),
    };
    let first = catalog.sprite(*ids.next().ok_or_else(gap)?).rect;
    let square = |rect: crate::catalog::Rect| rect.w == rect.h && rect.w == first.w;
    if square(first) && ids.all(|&id| square(catalog.sprite(id).rect)) {
        Ok(i32::from(first.w))
    } else {
        Err(gap())
    }
}

/// The light overlays' crossfade: one `transitionMs` for both. A missing overlay is the
/// `Light` variant's gap, reported there, not again here.
fn light_ms(catalog: &Catalog) -> Result<u32, MissingArt> {
    let mut ids = Vec::new();
    for kind in [Light::Dusk, Light::Sunburst] {
        let Ok(Some(id)) = light(catalog, kind) else {
            return Ok(0);
        };
        ids.push(id);
    }
    let ms: Vec<Option<u16>> = ids
        .iter()
        .map(|&id| catalog.sprite(id).transition_ms)
        .collect();
    match ms[..] {
        [Some(dusk), Some(sunburst)] if dusk == sunburst && dusk > 0 => Ok(u32::from(dusk)),
        _ => Err(MissingArt {
            variant: "Light".into(),
            wanted: "one transitionMs on both light overlays".into(),
        }),
    }
}

/// A night glow must be paired with the sprite it lights, since it is drawn where that sprite
/// stands.
fn paired(catalog: &Catalog, glow: SpriteId, host: SpriteId) -> Result<(), MissingArt> {
    if catalog.sprite(glow).paired_with == Some(host) {
        Ok(())
    } else {
        Err(MissingArt {
            variant: catalog.sprite(glow).id.clone(),
            wanted: format!("paired with `{}`", catalog.sprite(host).id),
        })
    }
}

/// The points every carved-mark design lists: exactly one per mark a stone can hold, the same
/// on each design. A missing design is the marks variant's gap, reported there, not again here.
fn mark_points(catalog: &Catalog) -> Result<Vec<Point>, MissingArt> {
    let Ok(marks) = carved_marks(catalog) else {
        return Ok(Vec::new());
    };
    let first = catalog.sprite(marks[0]).placement.clone();
    let agreed = marks
        .iter()
        .all(|&id| catalog.sprite(id).placement == first);
    if first.len() == usize::from(MAX_MARKS) && agreed {
        Ok(first)
    } else {
        Err(MissingArt {
            variant: "MonumentView.marks".into(),
            wanted: format!("{MAX_MARKS} placement points, the same on every design"),
        })
    }
}

/// The two films the glass can carry, each in levels 1 to `MAX_GLASS_LEVEL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
fn glass(catalog: &Catalog, film: GlassFilm, level: u8) -> Result<Option<SpriteId>, MissingArt> {
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

fn weather(catalog: &Catalog, weather: Weather) -> Result<Vec<SpriteId>, MissingArt> {
    let ids: &[&str] = match weather {
        Weather::Clear => &["tmp.sky.clear.tile"],
        Weather::Overcast => &["tmp.sky.overcast.tile"],
        Weather::Storm => &["tmp.sky.storm.body", "tmp.sky.storm.front"],
        Weather::Haze => &["tmp.sky.haze.tile"],
    };
    named(catalog, &format!("Weather::{}", weather.name()), ids)
}

/// Dusk multiplies and sunburst adds over the scene; neutral light has no overlay.
fn light(catalog: &Catalog, light: Light) -> Result<Option<SpriteId>, MissingArt> {
    let id = match light {
        Light::Neutral => return Ok(None),
        Light::Dusk => "tmp.sky.light.dusk",
        Light::Sunburst => "tmp.sky.light.sunburst",
    };
    named(catalog, &format!("Light::{}", light.name()), &[id]).map(|ids| Some(ids[0]))
}

/// A breeze and a gale share the leaf art and differ in sway amplitude; calm has neither.
fn wind(catalog: &Catalog, wind: Wind) -> Result<Vec<SpriteId>, MissingArt> {
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

/// What the sky's events play, each in its role: the storm's curtain and the authored sequences
/// of lightning, rain and the wind's leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkyEventArt {
    /// The storm front's curtain. It leads a storm in from the left, its open columns ahead;
    /// mirrored, from the right.
    pub front: SpriteId,
    /// The curtain's width, whose crossing paces a wipe.
    pub front_width: i32,
    /// Lightning's two frames, played once and cleared.
    pub lightning: AnimationId,
    /// How long lightning's frames last together.
    pub lightning_ms: u64,
    /// Rain after drought's streaks: a loop whose frames repeat across the scene.
    pub rain: AnimationId,
    /// The two leaves a windstorm carries, each a loop.
    pub leaves: [AnimationId; 2],
    /// The widest frame of either leaf, which a leaf crosses out of sight by.
    pub leaf_width: i32,
}

/// The sky events' art, or every gap in it: a sequence missing, or not shaped as it is played.
fn sky_events(catalog: &Catalog) -> Result<SkyEventArt, Vec<MissingArt>> {
    let mut gaps = Vec::new();
    let front = catalog.id("tmp.sky.storm.front");
    if front.is_none() {
        gaps.push(MissingArt {
            variant: "EventKind::Storm".into(),
            wanted: "`tmp.sky.storm.front`".into(),
        });
    }
    let mut sequence = |kind: EventKind, name: &str, looping: bool, tiled: bool| {
        let gap = |wanted: String| MissingArt {
            variant: format!("EventKind::{}", kind.name()),
            wanted,
        };
        let Some(id) = catalog.animation_id(name) else {
            gaps.push(gap(format!("the `{name}` sequence")));
            return None;
        };
        let animation = catalog.animation(id);
        let shaped = animation.looping == looping
            && (looping || animation.next == Next::Clear)
            && (!tiled
                || animation
                    .frames
                    .iter()
                    .all(|&frame| catalog.sprite(frame).flags.contains(Flags::TILE_X)));
        if !shaped {
            let how = match (looping, tiled) {
                (false, _) => "played once and cleared",
                (true, true) => "a loop of frames that repeat across the scene",
                (true, false) => "a loop",
            };
            gaps.push(gap(format!("`{name}` {how}")));
            return None;
        }
        Some(id)
    };
    let lightning = sequence(EventKind::Lightning, "lightning", false, false);
    let rain = sequence(EventKind::RainAfterDrought, "rain", true, true);
    let leaves = [
        sequence(EventKind::Windstorm, "wind-leaf-1", true, false),
        sequence(EventKind::Windstorm, "wind-leaf-2", true, false),
    ];
    match (front, lightning, rain, leaves) {
        (Some(front), Some(lightning), Some(rain), [Some(first), Some(second)]) => {
            Ok(SkyEventArt {
                front,
                front_width: i32::from(catalog.sprite(front).rect.w),
                lightning,
                lightning_ms: crate::anim::length_ms(catalog.animation(lightning))
                    .unwrap_or_default(),
                rain,
                leaves: [first, second],
                leaf_width: [first, second]
                    .iter()
                    .flat_map(|&leaf| &catalog.animation(leaf).frames)
                    .map(|&frame| i32::from(catalog.sprite(frame).rect.w))
                    .max()
                    .unwrap_or(0),
            })
        }
        _ => Err(gaps),
    }
}

/// Every sprite the event plays, sorted and without repeats.
fn event(catalog: &Catalog, kind: EventKind) -> Result<Vec<SpriteId>, MissingArt> {
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

/// Every variant's art, resolved from a catalog once, so the composer's lookups are map reads
/// rather than scans. Building it is check 6: it fails with every gap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtTable {
    plants: BTreeMap<(PlantSpecies, GrowthStage, PlantState), PlantArt>,
    blooms: BTreeMap<PlantSpecies, SpriteId>,
    plant_layers: BTreeMap<PlantSpecies, Layer>,
    scorches: BTreeMap<PlantSpecies, SpriteId>,
    residents: BTreeMap<(ResidentSpecies, LifeStage), Vec<AnimationId>>,
    /// Each effect that plays in step with a routine, by the routine: the firefly's lantern.
    lanterns: BTreeMap<AnimationId, AnimationId>,
    monuments: BTreeMap<MonumentAge, Vec<SpriteId>>,
    carved_marks: Vec<SpriteId>,
    fossils: BTreeMap<FossilForm, SpriteId>,
    ground_marks: BTreeMap<GroundMark, Vec<SpriteId>>,
    glass: BTreeMap<(GlassFilm, u8), SpriteId>,
    weather: BTreeMap<Weather, Vec<SpriteId>>,
    light: BTreeMap<Light, SpriteId>,
    wind: BTreeMap<Wind, Vec<SpriteId>>,
    events: BTreeMap<EventKind, Vec<SpriteId>>,
    tufts: Vec<SpriteId>,
    dead_stems: Vec<SpriteId>,
    world: Option<WorldArt>,
    sky_events: Option<SkyEventArt>,
}

impl ArtTable {
    /// A table with nothing resolved but the effects synced to routines.
    fn empty(catalog: &Catalog) -> ArtTable {
        ArtTable {
            plants: BTreeMap::new(),
            blooms: BTreeMap::new(),
            plant_layers: BTreeMap::new(),
            scorches: BTreeMap::new(),
            residents: BTreeMap::new(),
            lanterns: lanterns(catalog),
            monuments: BTreeMap::new(),
            carved_marks: Vec::new(),
            fossils: BTreeMap::new(),
            ground_marks: BTreeMap::new(),
            glass: BTreeMap::new(),
            weather: BTreeMap::new(),
            light: BTreeMap::new(),
            wind: BTreeMap::new(),
            events: BTreeMap::new(),
            tufts: Vec::new(),
            dead_stems: Vec::new(),
            world: None,
            sky_events: None,
        }
    }

    /// Every plant sprite of a species resolved so far, blooms included.
    fn parts_of(&self, species: PlantSpecies) -> Vec<SpriteId> {
        self.plants
            .iter()
            .filter(|((sp, _, _), _)| *sp == species)
            .flat_map(|(_, art)| match *art {
                PlantArt::Seedling(id) => vec![id],
                PlantArt::Parts { stem, foliage } => vec![stem, foliage],
            })
            .chain(self.blooms.get(&species).copied())
            .collect()
    }

    /// The embedded art's table, resolved once.
    ///
    /// # Panics
    /// If the embedded art leaves a variant without a sprite, which `cargo xtask verify` rules
    /// out.
    pub fn embedded() -> &'static ArtTable {
        static TABLE: OnceLock<ArtTable> = OnceLock::new();
        TABLE.get_or_init(|| {
            ArtTable::new(&Art::embedded().catalog).expect("the embedded art depicts every variant")
        })
    }

    /// Resolves every variant, or returns every gap. `cargo xtask assets` refuses a package
    /// that leaves any.
    pub fn new(catalog: &Catalog) -> Result<ArtTable, Vec<MissingArt>> {
        let mut gaps = Vec::new();
        let mut keep = |result: Result<(), MissingArt>| {
            if let Err(gap) = result {
                gaps.push(gap);
            }
        };
        let mut table = ArtTable::empty(catalog);
        for &species in PlantSpecies::ALL {
            for &growth in GrowthStage::ALL {
                for &health in PlantState::ALL {
                    keep(plant(catalog, species, growth, health).map(|art| {
                        table.plants.insert((species, growth, health), art);
                    }));
                }
            }
            keep(bloom(catalog, species).map(|id| {
                table.blooms.insert(species, id);
            }));
            keep(scorch(catalog, species).map(|id| {
                table.scorches.insert(species, id);
            }));
            let parts = table.parts_of(species);
            if !parts.is_empty() {
                keep(plant_layer(catalog, species, &parts).map(|layer| {
                    table.plant_layers.insert(species, layer);
                }));
            }
        }
        for &species in ResidentSpecies::ALL {
            for &stage in LifeStage::ALL {
                if stage == LifeStage::Pupa && !species.has_pupa() {
                    keep(no_pupa_art(catalog, species));
                } else {
                    keep(resident(catalog, species, stage).map(|routines| {
                        table.residents.insert((species, stage), routines);
                    }));
                }
            }
        }
        for &age in MonumentAge::ALL {
            keep(monument(catalog, age).map(|ids| {
                table.monuments.insert(age, ids);
            }));
        }
        keep(carved_marks(catalog).and_then(|marks| {
            if marks.len() < usize::from(MAX_MARKS) {
                return Err(MissingArt {
                    variant: "MonumentView.marks".into(),
                    wanted: format!("{MAX_MARKS} designs"),
                });
            }
            table.carved_marks = marks;
            Ok(())
        }));
        for &form in FossilForm::ALL {
            keep(fossil(catalog, form).map(|id| {
                table.fossils.insert(form, id);
            }));
        }
        for &kind in GroundMark::ALL {
            keep(ground_mark(catalog, kind).map(|ids| {
                table.ground_marks.insert(kind, ids);
            }));
        }
        for film in [GlassFilm::Condensation, GlassFilm::Dust] {
            for level in 1..=MAX_GLASS_LEVEL {
                keep(glass(catalog, film, level).map(|id| {
                    table.glass.extend(id.map(|id| ((film, level), id)));
                }));
            }
        }
        for &kind in Weather::ALL {
            keep(weather(catalog, kind).map(|ids| {
                table.weather.insert(kind, ids);
            }));
        }
        for &kind in Light::ALL {
            keep(light(catalog, kind).map(|id| table.light.extend(id.map(|id| (kind, id)))));
        }
        for &kind in Wind::ALL {
            keep(wind(catalog, kind).map(|ids| {
                table.wind.insert(kind, ids);
            }));
        }
        for &kind in EventKind::ALL {
            keep(event(catalog, kind).map(|ids| {
                table.events.insert(kind, ids);
            }));
        }
        keep(named(catalog, "filler tufts", &TUFTS).map(|ids| table.tufts = ids));
        keep(named(catalog, "filler dead stems", &DEAD_STEMS).map(|ids| table.dead_stems = ids));
        table.world = world(catalog).map_err(|found| gaps.extend(found)).ok();
        table.sky_events = sky_events(catalog).map_err(|found| gaps.extend(found)).ok();
        if gaps.is_empty() {
            Ok(table)
        } else {
            Err(gaps)
        }
    }

    /// How a plant in this stage and state is drawn. Every combination is present once `new`
    /// succeeded.
    pub fn plant(
        &self,
        species: PlantSpecies,
        growth: GrowthStage,
        health: PlantState,
    ) -> PlantArt {
        self.plants[&(species, growth, health)]
    }

    /// A young or mature plant's stem, or `None` for a seedling, whose one image carries its
    /// own. A stem is found without a state, so every state of a stage shares it.
    pub fn stem(&self, species: PlantSpecies, growth: GrowthStage) -> Option<SpriteId> {
        match self.plants.get(&(species, growth, PlantState::Thriving))? {
            PlantArt::Parts { stem, .. } => Some(*stem),
            PlantArt::Seedling(_) => None,
        }
    }

    /// The bloom overlay, drawn only on a mature, thriving plant whose `bloom` truth set.
    pub fn bloom(&self, species: PlantSpecies) -> SpriteId {
        self.blooms[&species]
    }

    /// The species' plant layer, shared by every part of it: its depth band.
    pub fn plant_layer(&self, species: PlantSpecies) -> Layer {
        self.plant_layers[&species]
    }

    /// The ground tufts that fill a living scene's empty slots, all equivalent.
    pub fn tufts(&self) -> &[SpriteId] {
        &self.tufts
    }

    /// The dead-stem litter that fills a dead scene's empty slots, all equivalent.
    pub fn dead_stems(&self) -> &[SpriteId] {
        &self.dead_stems
    }

    /// The world's own art.
    ///
    /// # Panics
    /// Never once `new` succeeded, which resolves it.
    pub fn world(&self) -> &WorldArt {
        self.world
            .as_ref()
            .expect("a table that built has its world art")
    }

    /// What the sky's events play.
    ///
    /// # Panics
    /// Never once `new` succeeded, which resolves it.
    pub fn sky_events(&self) -> &SkyEventArt {
        self.sky_events
            .as_ref()
            .expect("a table that built has its sky events' art")
    }

    /// The scorched foliage that stands in for a mature plant's while lightning's scorch plays.
    pub fn scorch(&self, species: PlantSpecies) -> SpriteId {
        self.scorches[&species]
    }

    /// The routines a resident may play in a stage, sorted by name; empty for a pupa the
    /// species does not have. `Dying` is the death routine alone.
    pub fn resident(&self, species: ResidentSpecies, stage: LifeStage) -> &[AnimationId] {
        self.residents
            .get(&(species, stage))
            .map_or(&[], Vec::as_slice)
    }

    /// The effect that plays in step with a routine, if it has one: a firefly's lantern with
    /// its signal.
    pub fn lantern(&self, routine: AnimationId) -> Option<AnimationId> {
        self.lanterns.get(&routine).copied()
    }

    /// The stone of a monument's age, and its night lichen glow once it has lichen.
    pub fn monument(&self, age: MonumentAge) -> &[SpriteId] {
        &self.monuments[&age]
    }

    /// The carved-mark designs: at least one per placement point.
    pub fn carved_marks(&self) -> &[SpriteId] {
        &self.carved_marks
    }

    pub fn fossil(&self, form: FossilForm) -> SpriteId {
        self.fossils[&form]
    }

    pub fn ground_mark(&self, kind: GroundMark) -> &[SpriteId] {
        &self.ground_marks[&kind]
    }

    /// The film at a level; level 0, or a level past `MAX_GLASS_LEVEL`, draws nothing.
    pub fn glass(&self, film: GlassFilm, level: u8) -> Option<SpriteId> {
        self.glass.get(&(film, level)).copied()
    }

    pub fn weather(&self, weather: Weather) -> &[SpriteId] {
        &self.weather[&weather]
    }

    /// Dusk's or sunburst's overlay; neutral light has none.
    pub fn light(&self, light: Light) -> Option<SpriteId> {
        self.light.get(&light).copied()
    }

    /// The leaf art for a breeze or a gale; calm has none.
    pub fn wind(&self, wind: Wind) -> &[SpriteId] {
        &self.wind[&wind]
    }

    /// Every sprite the event plays, sorted and without repeats.
    pub fn event(&self, kind: EventKind) -> &[SpriteId] {
        &self.events[&kind]
    }
}

/// Every gap between the `SceneInput` variants and the catalog: check 6.
pub fn missing(catalog: &Catalog) -> Vec<MissingArt> {
    ArtTable::new(catalog).err().unwrap_or_default()
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
    use crate::catalog::{Art, Flags};

    fn catalog() -> &'static Catalog {
        &Art::embedded().catalog
    }

    #[test]
    fn the_embedded_art_depicts_every_variant() {
        let gaps = missing(catalog());
        assert!(gaps.is_empty(), "art gaps:\n{}", lines(&gaps));
    }

    #[test]
    fn the_table_answers_every_variant_from_its_maps() {
        let table = ArtTable::new(catalog()).expect("the embedded art has no gaps");
        let oak = table.plant(PlantSpecies::Oak, GrowthStage::Mature, PlantState::Thriving);
        assert_eq!(
            oak,
            plant(
                catalog(),
                PlantSpecies::Oak,
                GrowthStage::Mature,
                PlantState::Thriving
            )
            .unwrap()
        );
        assert!(
            table
                .resident(ResidentSpecies::Snail, LifeStage::Pupa)
                .is_empty()
        );
        assert_eq!(
            table
                .resident(ResidentSpecies::Snail, LifeStage::Dying)
                .len(),
            1
        );
        assert_eq!(table.light(Light::Neutral), None);
        assert!(table.light(Light::Dusk).is_some());
        assert_eq!(table.glass(GlassFilm::Dust, 0), None);
        assert!(table.glass(GlassFilm::Dust, MAX_GLASS_LEVEL).is_some());
        assert_eq!(table.glass(GlassFilm::Dust, MAX_GLASS_LEVEL + 1), None);
        assert!(table.wind(Wind::Calm).is_empty());
        assert_eq!(
            table.event(EventKind::Storm),
            table.event(EventKind::Clearing)
        );
        assert_eq!(table.carved_marks().len(), CARVED_MARKS.len());
        assert_eq!(table.monument(MonumentAge::Ancient).len(), 2);
    }

    /// Each sky event's art in its role, and a sequence not shaped as it is played is a gap:
    /// rain whose frames do not repeat across the scene, lightning that loops.
    #[test]
    fn the_sky_choreography_art_resolves() {
        let art = sky_events(catalog()).expect("the embedded sky events' art");
        assert_eq!(catalog().sprite(art.front).id, "tmp.sky.storm.front");
        assert_eq!(
            ArtTable::new(catalog()).unwrap().weather(Weather::Storm)[1],
            art.front
        );
        let name = |id: AnimationId| catalog().animation(id).name.as_str();
        assert_eq!(
            [art.lightning, art.rain, art.leaves[0], art.leaves[1]].map(name),
            ["lightning", "rain", "wind-leaf-1", "wind-leaf-2"]
        );
        assert_eq!(catalog().animation(art.lightning).frames.len(), 2);

        let mut untiled = catalog().clone();
        let streak = untiled.animation(art.rain).frames[1];
        untiled.sprites[streak.index()].flags = Flags::default();
        let gaps = sky_events(&untiled).unwrap_err();
        assert_eq!(
            lines(&gaps),
            "EventKind::RainAfterDrought: `rain` a loop of frames that repeat across the scene"
        );

        let mut looping = catalog().clone();
        looping.animations[art.lightning.index()].looping = true;
        assert_eq!(
            lines(&sky_events(&looping).unwrap_err()),
            "EventKind::Lightning: `lightning` played once and cleared"
        );
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
    fn every_state_of_a_stage_shares_its_stem_and_a_seedling_has_none() {
        let table = ArtTable::new(catalog()).unwrap();
        for &species in PlantSpecies::ALL {
            assert_eq!(table.stem(species, GrowthStage::Seedling), None);
            for growth in [GrowthStage::Young, GrowthStage::Mature] {
                let stem = table.stem(species, growth).expect("a stem");
                for &health in PlantState::ALL {
                    let PlantArt::Parts { stem: own, .. } = table.plant(species, growth, health)
                    else {
                        panic!("{species:?} {growth:?} is drawn from parts");
                    };
                    assert_eq!(own, stem, "{species:?} {growth:?} {health:?}");
                }
            }
        }
    }

    #[test]
    fn each_species_stands_in_the_band_of_its_pool() {
        // The species pools: three far trees, five mid plants, four near ones.
        let table = ArtTable::new(catalog()).unwrap();
        let in_layer = |layer| {
            PlantSpecies::ALL
                .iter()
                .filter(|&&species| table.plant_layer(species) == layer)
                .map(|species| species.name())
                .collect::<Vec<_>>()
        };
        assert_eq!(in_layer(Layer::FarPlants), ["Oak", "Birch", "Rowan"]);
        assert_eq!(
            in_layer(Layer::MidPlants),
            ["Fern", "TallGrass", "Bramble", "Foxglove", "Reed"]
        );
        assert_eq!(
            in_layer(Layer::NearPlants),
            ["MossMound", "WildflowerCluster", "Clover", "Thistle"]
        );
    }

    #[test]
    fn a_species_split_across_layers_is_a_gap() {
        let mut broken = catalog().clone();
        let id = broken.id("tmp.plant.fern.bloom.mature").unwrap();
        broken.sprites[id.index()].layer = Some(Layer::NearPlants);
        assert_eq!(
            lines(&missing(&broken)),
            "PlantSpecies::Fern: every part in one plant layer"
        );
    }

    #[test]
    fn the_filler_is_tufts_and_dead_stems_from_the_ground_art() {
        let table = ArtTable::new(catalog()).unwrap();
        assert_eq!(table.tufts().len(), TUFTS.len());
        assert_eq!(table.dead_stems().len(), DEAD_STEMS.len());
        for &id in table.tufts() {
            assert_eq!(catalog().sprite(id).layer, Some(Layer::NearPlants));
        }
        for &id in table.dead_stems() {
            assert_eq!(catalog().sprite(id).layer, Some(Layer::GroundLitter));
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

    #[test]
    fn the_world_art_resolves_with_its_glows_paired_and_its_light_time() {
        let table = ArtTable::new(catalog()).unwrap();
        let world = table.world();
        assert_eq!(world.weeds.len(), 6);
        assert_eq!(world.weed_patches.len(), 2);
        assert_eq!(world.amber_litter.len(), 4);
        assert_eq!(world.fungus.len(), 2);
        assert_eq!(world.light_ms, 1200);
        for &(body, glow) in &world.fungus {
            assert_eq!(catalog().sprite(glow).paired_with, Some(body));
        }
        let flags = |id: SpriteId| catalog().sprite(id).flags;
        assert!(flags(world.sun).contains(Flags::DAY_ONLY));
        assert!(
            world
                .moons
                .iter()
                .all(|&id| flags(id).contains(Flags::NIGHT_ONLY))
        );
        assert!(
            world
                .stars
                .iter()
                .all(|&id| flags(id).contains(Flags::NIGHT_ONLY))
        );
        assert!(flags(world.plant_glow).contains(Flags::NIGHT_ONLY));
    }

    #[test]
    fn a_glow_that_lost_its_pairing_is_a_gap() {
        let mut broken = catalog().clone();
        let glow = broken.id("tmp.fungus.cap-cluster.02.glow").unwrap();
        broken.sprites[glow.index()].paired_with = None;
        assert_eq!(
            lines(&missing(&broken)),
            "tmp.fungus.cap-cluster.02.glow: paired with `tmp.fungus.cap-cluster.02`"
        );
        let mut broken = catalog().clone();
        let glow = broken.id("tmp.monument.lichen-glow.ancient").unwrap();
        let young = broken.id("tmp.monument.stone.young");
        broken.sprites[glow.index()].paired_with = young;
        assert_eq!(
            lines(&missing(&broken)),
            "tmp.monument.lichen-glow.ancient: paired with `tmp.monument.stone.ancient`"
        );
    }

    #[test]
    fn the_light_time_and_the_sky_sizes_are_one_gap_each() {
        let mut broken = catalog().clone();
        let dusk = broken.id("tmp.sky.light.dusk").unwrap();
        broken.sprites[dusk.index()].transition_ms = None;
        assert_eq!(
            lines(&missing(&broken)),
            "Light: one transitionMs on both light overlays"
        );
        let mut broken = catalog().clone();
        let moon = broken.id("tmp.sky.moon.phase.2").unwrap();
        broken.sprites[moon.index()].rect.w += 1;
        assert_eq!(
            lines(&missing(&broken)),
            "the sun and the moons: squares of one size"
        );
        let table = ArtTable::new(catalog()).unwrap();
        assert_eq!((table.world().body_side, table.world().star_side), (5, 3));
        assert_eq!(table.world().label_capacity, 18);
    }

    #[test]
    fn every_carved_mark_lists_the_three_prose_points() {
        let table = ArtTable::new(catalog()).unwrap();
        let p = |x, y| Point { x, y };
        assert_eq!(table.world().mark_points, [p(3, 6), p(3, 11), p(3, 16)]);
        let mut broken = catalog().clone();
        let mark = broken.id("tmp.monument.carved-mark.04").unwrap();
        broken.sprites[mark.index()].placement.pop();
        assert_eq!(
            lines(&missing(&broken)),
            "MonumentView.marks: 3 placement points, the same on every design"
        );
    }

    #[test]
    fn every_mark_lands_on_opaque_stone_at_every_point_and_age() {
        // The prose says to fit the marks onto the opaque stone face: each opaque mark pixel,
        // with the mark's top-left on a point, covers an opaque pixel of the stone.
        let art = Art::embedded();
        let table = ArtTable::new(catalog()).unwrap();
        let opaque = |id: SpriteId| -> Vec<Vec<bool>> {
            art.rows(id)
                .map(|row| row.iter().map(|&index| index != 0).collect())
                .collect()
        };
        for &age in MonumentAge::ALL {
            let stone = opaque(table.monument(age)[0]);
            for &mark in table.carved_marks() {
                let mark_pixels = opaque(mark);
                for point in &table.world().mark_points {
                    for (dy, row) in mark_pixels.iter().enumerate() {
                        for (dx, &inked) in row.iter().enumerate() {
                            let x = usize::try_from(point.x).unwrap() + dx;
                            let y = usize::try_from(point.y).unwrap() + dy;
                            assert!(
                                !inked || stone[y][x],
                                "{age:?} {} at {point:?}",
                                catalog().sprite(mark).id
                            );
                        }
                    }
                }
            }
        }
    }

    fn lines(gaps: &[MissingArt]) -> String {
        gaps.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
}
