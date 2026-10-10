//! The composer: from the `SceneInput`, the `OverlayInput`, the install salt and the clock to the
//! draws of a frame. It depicts the states truth gives it and never judges health
//! itself.
//!
//! Each update places the plants, weeds and filler ([`crate::place`]) and gives monuments and
//! fossils their spots, then moves every subject's looks toward what the scene holds. A subject
//! is anything drawn that can change: a plant, a filler, the weeds in a cell, a monument, a
//! fossil, a piece of litter or fungus, a decorative tuft, the weather's backdrop, a cloud, the sun,
//! the moon, a star, the light, a glass film, or the label's text. Each has up to five parts
//! (a plant's stem, foliage, bloom and glow; a monument's stone, three marks and glow), and
//! each part keeps its looks oldest first, the newest being the one it is becoming.
//!
//! Nothing ever cuts (I-15, constraint 7):
//! - **A subject arrives** with every part rising from nothing.
//! - **A subject leaves** with every part falling from wherever it stands.
//! - **A part changes look** by its art's rule ([`Restyle`]). Opaque art raises the new look
//!   on top while every older look holds, and only then lets the older ones fall, so the sky
//!   never shows through a change. Translucent and glowing art crossfades, so two films or
//!   two glows never stack into a denser one. A part whose look is unchanged does not move:
//!   a plant turning pale fades its foliage, never its stem.
//! - **An update** only starts tweens from where their values stand, so the frame at the
//!   moment of an update is the frame just before it.
//!
//! The fixed rest of the world, which never changes while the app runs, is the
//! [`Scenery`].

use std::cell::OnceCell;
use std::collections::BTreeMap;

use terrarium_truth::PresentationState;
use terrarium_truth::scene::{
    EventKind, FossilForm, GroundMark, GrowthStage, LifeStage, Light, MonumentAge, PlantSpecies,
    PlantState, PlantView, ResidentSpecies, ResidentView, SceneInput, Weather,
};

use crate::anim;
use crate::blend::{mix, scale};
use crate::catalog::{Art, Flags, SpriteId};
use crate::clock::{DayNight, LocalClock};
use crate::drift::{self, SOIL_MS_PER_PX, SOIL_PERIOD};
use crate::frame::{
    Area, Draw, Frame, GROUND_Y, NATIVE_HEIGHT, NATIVE_WIDTH, PLANT_GROUND_Y, Shear, Viewport,
};
use crate::motion::{Clock, DIM_EVERY, Eras, Shaken, Snapshot, motion_turn, turn, turn_start};
use crate::order::{KeyedDraw, Place, Subject, merged};
use crate::panel::{Hover, Landmark};
use crate::place::{Bands, FillForm, Placement, WeedForm, assign_spots, place};
use crate::plant::{self, shows_bloom, shows_glow, sways};
use crate::resident::{self, Enter, Resident, Segment, TRANSITION_MS, TurnPlan, Way};
use crate::rules::dressing;
use crate::salt::{InstallSalt, Purpose};
use crate::sky::{self, CloudSize, MoonLook, moon_phase};
use crate::slots::{self, Layout};
use crate::sway;
use crate::text::{Label, fit};
use crate::tween::durations::{
    APPEAR_MS, CLOCK_FIX_MS, DIM_DOWN_MS, DIM_UP_MS, FILL_MS, LABEL_CYCLE_MS, LABEL_FADE_MS,
    RESTYLE_MS, SKY_MS, VANISH_MS,
};
use crate::tween::{Millis, Tween};
use crate::variants::{ArtTable, GlassFilm, GroundEventArt, PlantArt, SkyEventArt};
use crate::world::{self, Kind, LABEL_X, LABEL_Y, Scenery, WorldLayout};

mod event;
mod ground;
mod hover;

const SHOWN: u8 = u8::MAX;
const HALF_RESTYLE: u32 = RESTYLE_MS / 2;
/// How many turns past the running one the next motion may lie: a cell gusts once in every
/// [`sway::GUST_EVERY`] motion turns, and in dim mode one turn in [`DIM_EVERY`] moves. Once a
/// turn's gust is over, the plant's next is that many turns on.
const MOTION_AHEAD: u64 = sway::GUST_EVERY * DIM_EVERY;
const FRAME_WIDTH: i32 = NATIVE_WIDTH.cast_signed();
const FRAME_HEIGHT: i32 = NATIVE_HEIGHT.cast_signed();
const GROUND: i32 = GROUND_Y.cast_signed();

/// What the composer is given beside the scene: not truth, but the app's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayInput {
    /// The lines the label cycles through, already worded by the app's copy module (I-8). The
    /// composer only fits them to the plate.
    pub label: Vec<String>,
    /// The local wall time at the update: its date for the moon, its time of day for day and
    /// night. Between updates the composer carries it forward on its own clock.
    pub clock: LocalClock,
    /// When the day and the night begin.
    pub day_night: DayNight,
    /// Whether the machine has gone without input past the dim timeout.
    pub dim: bool,
    /// The soil panel and a hover label, as the pointer has them.
    pub hover: Hover,
}

impl OverlayInput {
    /// The label's lines at a local time, with the default day and night and no dimming.
    pub fn new(label: Vec<String>, clock: LocalClock) -> OverlayInput {
        OverlayInput {
            label,
            clock,
            day_night: DayNight::default(),
            dim: false,
            hover: Hover::default(),
        }
    }
}

/// How far dim mode darkens the frame: to 40% of every colour value, 255 × 0.6.
pub const DIM: u8 = 153;

/// Which litter a scene drops: amber leaves while anything lives, bleached dead stems once
/// the terrarium is dead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LitterSet {
    Amber,
    DeadStems,
}

/// What one look of a part shows. A piece is the depicted state, and never carries a choice
/// of the salt's: its sprite is looked up only when drawing, where the salt picks between
/// equivalent sprites.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Piece {
    Stem {
        species: PlantSpecies,
        stage: GrowthStage,
    },
    /// The state's foliage, or the combined image of a seedling.
    Foliage {
        species: PlantSpecies,
        stage: GrowthStage,
        state: PlantState,
    },
    Bloom {
        species: PlantSpecies,
    },
    /// A thriving plant's night glow, at the centre of its foliage.
    PlantGlow {
        species: PlantSpecies,
        stage: GrowthStage,
    },
    /// A filler's ground tuft.
    Tuft,
    /// A filler's dead-stem litter.
    DeadStems,
    /// A decorative tuft: dressing between the slots, never filler.
    DressingTuft,
    Weed(WeedForm),
    Litter(LitterSet),
    Fungus,
    FungusGlow,
    Stone(MonumentAge),
    /// A carved mark on a stone of this age; the age places it on the stone's face.
    Mark {
        age: MonumentAge,
    },
    LichenGlow(MonumentAge),
    Fossil(FossilForm),
    /// A mark an event left in the ground, settled.
    Ground(GroundMark),
    Weather(Weather),
    Cloud(CloudSize),
    Sun,
    Moon(MoonLook),
    Star,
    Light(Light),
    /// A film at a level, 1 to `MAX_GLASS_LEVEL`.
    Film(GlassFilm, u8),
    /// A resident as truth has it; which frame it shows is its turn's to say.
    Resident {
        species: ResidentSpecies,
        stage: LifeStage,
    },
    /// An event's choreography while it plays; it is never a look.
    Event(EventKind),
}

/// How a part changes from one look to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restyle {
    /// The new look rises on top while the older ones hold, then they fall: for opaque art.
    Cover,
    /// The new look rises while the older ones fall: for translucent and glowing art.
    Crossfade,
}

impl Piece {
    /// The art's rule for changing look: art that is not opaque and normal crossfades.
    pub const fn restyle(self) -> Restyle {
        match self {
            Piece::PlantGlow { .. }
            | Piece::FungusGlow
            | Piece::LichenGlow(_)
            | Piece::Light(_)
            | Piece::Film(..) => Restyle::Crossfade,
            _ => Restyle::Cover,
        }
    }

    /// The sprite the piece shows for `subject`'s `part`; `None` for the stem of a seedling,
    /// a young stone's glow, neutral light or a film past the last level, none of which a
    /// subject is given.
    pub fn sprite(
        self,
        table: &ArtTable,
        salt: InstallSalt,
        subject: Subject,
        part: u8,
    ) -> Option<SpriteId> {
        let world = table.world();
        let item = item(subject);
        let filler = |sprites: &[SpriteId]| {
            let n = u64::try_from(sprites.len()).unwrap_or(1);
            let pick = salt.below(Purpose::FillVariant, item, n);
            sprites.get(usize::try_from(pick).ok()?).copied()
        };
        let pick = |kind: Kind, sprites: &[SpriteId]| world::pick(salt, kind, item, sprites);
        match self {
            Piece::Stem { species, stage } => table.stem(species, stage),
            Piece::Foliage {
                species,
                stage,
                state,
            } => Some(match table.plant(species, stage, state) {
                PlantArt::Parts { foliage, .. } => foliage,
                PlantArt::Seedling(combined) => combined,
            }),
            Piece::Bloom { species } => Some(table.bloom(species)),
            Piece::PlantGlow { .. } => Some(world.plant_glow),
            Piece::Tuft => filler(table.tufts()),
            Piece::DeadStems => filler(table.dead_stems()),
            Piece::DressingTuft => pick(Kind::Dressing, table.tufts()),
            Piece::Weed(WeedForm::Single) => pick(Kind::Weed, &world.weeds),
            Piece::Weed(WeedForm::Patch) => pick(Kind::Weed, &world.weed_patches),
            Piece::Litter(LitterSet::Amber) => pick(Kind::Litter, &world.amber_litter),
            Piece::Litter(LitterSet::DeadStems) => pick(Kind::Litter, table.dead_stems()),
            Piece::Fungus | Piece::FungusGlow => {
                let (body, glow) = world::pick(salt, Kind::Fungus, item, &world.fungus)?;
                Some(if self == Piece::Fungus { body } else { glow })
            }
            Piece::Stone(age) => table.monument(age).first().copied(),
            Piece::LichenGlow(age) => table.monument(age).get(1).copied(),
            Piece::Mark { .. } => {
                // Each mark of each monument is its own choice of design.
                let mark = item * 4 + u64::from(part);
                world::pick(salt, Kind::Mark, mark, table.carved_marks())
            }
            Piece::Fossil(form) => Some(table.fossil(form)),
            Piece::Ground(GroundMark::FreshSoil) => {
                pick(Kind::Ground, table.ground_mark(GroundMark::FreshSoil))
            }
            Piece::Ground(mark) => table.ground_mark(mark).first().copied(),
            Piece::Weather(weather) => table.weather(weather).first().copied(),
            Piece::Cloud(size) => pick(Kind::Cloud, world.clouds(size)),
            Piece::Sun => Some(world.sun),
            Piece::Moon(look) => world
                .moons
                .get(usize::from(look.sprite).checked_sub(1)?)
                .copied(),
            Piece::Star => pick(Kind::Star, &world.stars),
            Piece::Light(light) => table.light(light),
            Piece::Film(film, level) => table.glass(film, level),
            Piece::Resident { .. } | Piece::Event(_) => None,
        }
    }
}

/// The number a subject's salted choices are drawn for: its cell, its place or its id.
fn item(subject: Subject) -> u64 {
    match subject {
        Subject::Weed { cell }
        | Subject::Fill { cell }
        | Subject::Plant { cell, .. }
        | Subject::Scorch { cell, .. }
        | Subject::Burn(cell)
        | Subject::Sprinkle(cell)
        | Subject::Mark { at: cell, .. } => u64::from(cell),
        Subject::Cloud(i)
        | Subject::Star(i)
        | Subject::Dressing(i)
        | Subject::Litter(i)
        | Subject::Fungus(i)
        | Subject::Leaf(i) => u64::from(i),
        Subject::Monument { id, .. }
        | Subject::Fossil { id, .. }
        | Subject::Resident { id, .. } => u64::from(id),
        Subject::Scenery(i) => u64::from(i),
        Subject::Film(_)
        | Subject::Weather
        | Subject::Sun
        | Subject::Moon
        | Subject::Front
        | Subject::Bolt
        | Subject::Rain
        | Subject::Light
        | Subject::Label
        | Subject::Soil
        | Subject::Sweep
        | Subject::Tag { .. }
        | Subject::Panel => 0,
    }
}

/// The parts of a subject, bottom to top.
const PARTS: usize = 5;
const STEM: usize = 0;
const FOLIAGE: usize = 1;
/// The foliage's part, as a draw's key counts it.
const FOLIAGE_PART: u8 = 1;
const BLOOM: usize = 2;
const PLANT_GLOW: usize = 3;
const STONE: usize = 0;
/// A monument's marks are parts 1 to 3, over the stone and under its glow.
const FIRST_MARK: usize = 1;
const LICHEN_GLOW: usize = 4;

type Pieces = [Option<Piece>; PARTS];

fn plant_pieces(view: &PlantView) -> Pieces {
    let PlantView {
        species,
        stage,
        state,
        bloom,
        ..
    } = *view;
    let mut pieces = [None; PARTS];
    if stage != GrowthStage::Seedling {
        pieces[STEM] = Some(Piece::Stem { species, stage });
    }
    pieces[FOLIAGE] = Some(Piece::Foliage {
        species,
        stage,
        state,
    });
    if shows_bloom(stage, state, bloom) {
        pieces[BLOOM] = Some(Piece::Bloom { species });
    }
    if shows_glow(stage, state) {
        pieces[PLANT_GLOW] = Some(Piece::PlantGlow { species, stage });
    }
    pieces
}

fn fill_pieces(form: FillForm) -> Pieces {
    let mut pieces = [None; PARTS];
    match form {
        FillForm::Tuft => pieces[FOLIAGE] = Some(Piece::Tuft),
        FillForm::DeadStems => pieces[FOLIAGE] = Some(Piece::DeadStems),
        FillForm::Plant { species, state } => {
            pieces[STEM] = Some(Piece::Stem {
                species,
                stage: FillForm::STAGE,
            });
            pieces[FOLIAGE] = Some(Piece::Foliage {
                species,
                stage: FillForm::STAGE,
                state,
            });
        }
    }
    pieces
}

fn one(piece: Piece) -> Pieces {
    let mut pieces = [None; PARTS];
    pieces[0] = Some(piece);
    pieces
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Look {
    piece: Piece,
    opacity: Tween,
    /// The sky as it stood when a wipe began, which the wipe covers.
    behind: bool,
}

/// A subject's looks, per part, oldest first. Only the newest look of a part ever rises.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Entry {
    parts: [Vec<Look>; PARTS],
}

impl Entry {
    /// Whether the subject is on its way in or standing, rather than leaving.
    fn shown(&self) -> bool {
        self.looks().any(|look| look.opacity.target() == SHOWN)
    }

    fn looks(&self) -> impl Iterator<Item = &Look> {
        self.parts.iter().flatten()
    }

    /// Moves each part toward its piece from its start, rising over `rise`; a part with none
    /// falls over `change` once the others have risen.
    fn show(&mut self, pieces: Pieces, starts: [Millis; PARTS], rise: u32, change: u32) {
        for ((looks, piece), start) in self.parts.iter_mut().zip(pieces).zip(starts) {
            match piece {
                Some(piece) => raise(looks, piece, start, rise, change),
                None => lower(looks, start, start.plus(rise), change),
            }
        }
    }

    /// Whether every look of `parts` holds still from `now` until it is gone by `until`, or for
    /// good: a change can wait until then without cutting any look that moves now.
    fn still(&self, parts: std::ops::Range<usize>, now: Millis, until: Millis) -> bool {
        self.parts[parts].iter().flatten().all(|look| {
            let tween = look.opacity;
            tween.end() <= now || (tween.target() == 0 && tween.end() <= until)
        })
    }

    fn hide(&mut self, start: Millis, ms: u32) {
        for looks in &mut self.parts {
            lower(looks, start, start, ms);
        }
    }

    /// Drops every look that has finished falling. Each was invisible, so no frame changes.
    fn prune(&mut self, now: Millis) {
        for looks in &mut self.parts {
            looks.retain(|look| look.opacity.target() == SHOWN || now < look.opacity.end());
        }
    }
}

/// Brings `piece` to the top of a part over `rise` from `start`. The older looks then go by
/// the piece's rule: under [`Restyle::Cover`] they hold until it has risen and fall over
/// `change` after, so nothing beneath ever shows through; under [`Restyle::Crossfade`] they
/// fall while it rises. A look already gone by `start` is left to go: a change that waits for
/// a wipe to pass never brings back the sky the wipe covered.
fn raise(looks: &mut Vec<Look>, piece: Piece, start: Millis, rise: u32, change: u32) {
    let gone = |look: &Look| look.opacity.target() == 0 && look.opacity.end() <= start;
    match looks.last_mut() {
        Some(top) if top.piece == piece && !gone(top) => {
            if top.opacity.target() == SHOWN {
                return;
            }
            top.opacity = Tween::new(top.opacity.value(start), SHOWN, start, rise);
        }
        _ => looks.push(Look {
            piece,
            opacity: Tween::new(0, SHOWN, start, rise),
            behind: false,
        }),
    }
    let older = looks.len() - 1;
    for look in looks[..older].iter_mut().filter(|look| !gone(look)) {
        let from = look.opacity.value(start);
        look.opacity = match piece.restyle() {
            Restyle::Cover => Tween::new(from, 0, start.plus(rise), change),
            Restyle::Crossfade => Tween::new(from, 0, start, rise),
        };
    }
}

/// Lets a part's shown look fall from where it stands at `at`, from `start` over `ms`. Looks
/// already falling keep their course.
fn lower(looks: &mut [Look], at: Millis, start: Millis, ms: u32) {
    for look in looks {
        if look.opacity.target() == SHOWN {
            look.opacity = Tween::new(look.opacity.value(at), 0, start, ms);
        }
    }
}

/// How long a subject takes to arrive, to leave, and to bring a new look up.
#[derive(Clone, Copy, Debug)]
struct Timing {
    arrive: u32,
    leave: u32,
    change: u32,
}

/// The label's text: one set of lines, cycling from when it arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LabelLook {
    /// Fitted to the plate once, when the set arrives.
    lines: Vec<String>,
    since: Millis,
    opacity: Tween,
}

impl LabelLook {
    /// The lines showing at `now`, each with its opacity: one line, or two while the cycle
    /// crossfades from one to the next.
    fn showing(&self, now: Millis) -> Vec<(usize, u8)> {
        let n = u64::try_from(self.lines.len()).unwrap_or(0);
        if n <= 1 {
            return if n == 1 { vec![(0, SHOWN)] } else { Vec::new() };
        }
        let cycle = u64::from(LABEL_CYCLE_MS);
        let elapsed = now.0.saturating_sub(self.since.0);
        let step = elapsed / cycle;
        let index = |step: u64| usize::try_from(step % n).unwrap_or(0);
        if step == 0 {
            return vec![(0, SHOWN)];
        }
        let started = Millis(self.since.0 + step * cycle);
        let rising = Tween::new(0, SHOWN, started, LABEL_FADE_MS).value(now);
        if rising == SHOWN {
            vec![(index(step), SHOWN)]
        } else {
            vec![(index(step - 1), SHOWN - rising), (index(step), rising)]
        }
    }

    /// When the lines showing next change after `now`: a step of the crossfade running, or
    /// the first step of the next one. `None` for a set of one line, which never cycles.
    fn next_change(&self, now: Millis) -> Option<Millis> {
        if self.lines.len() <= 1 {
            return None;
        }
        let cycle = u64::from(LABEL_CYCLE_MS);
        let step = now.0.saturating_sub(self.since.0) / cycle;
        let fade =
            |step: u64| Tween::new(0, SHOWN, Millis(self.since.0 + step * cycle), LABEL_FADE_MS);
        let running = (step > 0).then(|| fade(step).next_step(now)).flatten();
        let next = Millis(self.since.0 + (step + 1) * cycle);
        running.or_else(|| fade(step + 1).next_step(next))
    }
}

/// One look as drawn: the depicted state, without where the salt put it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Depicted {
    pub subject: Subject,
    pub part: u8,
    pub piece: Piece,
    pub opacity: u8,
}

/// The composer's depiction state for one terrarium on one install.
#[derive(Clone, Debug)]
pub struct Composer {
    salt: InstallSalt,
    bands: Bands,
    layout: Layout,
    world: WorldLayout,
    light_ms: u32,
    label_capacity: usize,
    sweep_width: i32,
    /// The columns it composes for; the scene stands in their middle.
    viewport: Viewport,
    placement: Placement,
    monuments: BTreeMap<u32, u8>,
    fossils: BTreeMap<u32, u8>,
    entries: BTreeMap<Subject, Entry>,
    label: Vec<LabelLook>,
    /// The local clock of the last update, and when on the composer's clock it was read.
    clock: Option<(LocalClock, Millis)>,
    day_night: DayNight,
    /// An update whose clock disagreed with the time carried forward eases the palette from
    /// where it stood to the new time's.
    mix_fix: Option<MixFix>,
    dim: Tween,
    /// What each turn of motion reads of the scene.
    eras: Eras,
    /// Each resident's stretch of the strip, by id, kept while it stays.
    resident_spots: BTreeMap<u32, u8>,
    /// Each resident drawn or fading, by id and stretch: one that comes back in another
    /// stretch fades out where it was as it fades in where it is.
    residents: BTreeMap<(u32, u8), Shown>,
    /// When the changes of rest an update brought at the next turn are over.
    residents_settle: Millis,
    /// What the sky's events play.
    sky: SkyEventArt,
    /// The event instances begun or passed over lately, oldest first: each plays once, even
    /// when another came between.
    played: Vec<event::Instance>,
    /// The storm front's curtain, read from the art the first time a wipe needs it.
    curtain: OnceCell<event::Curtain>,
    /// The event's choreography, playing or played.
    show: Option<event::Show>,
    /// What the ground's events play.
    ground_art: GroundEventArt,
    /// What a ground choreography keeps beside its show.
    stage: ground::Stage,
    /// The cells each patch of fresh soil lies on, by its mark: the cells a burn took the weeds
    /// from.
    fresh: BTreeMap<u32, Vec<u8>>,
    /// The spot of each fossil a sinkhole names, by the fossil, while a mark names it: a fossil
    /// a rewrite took leaves its hole where it lay.
    sunk: BTreeMap<u32, u8>,
    /// The soil panel and the hover labels.
    hover: hover::HoverLayer,
}

/// A resident as drawn: how visible it is, what it was when last in the scene, and, through
/// the turn it arrived in, what it arrived as and the scene it arrived with, which is how it
/// rests until the next turn reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Shown {
    opacity: Tween,
    last: Resident,
    arrival: Option<(u64, Resident, Snapshot)>,
}

/// The palette mix an update found standing, and how far the new clock's mix has taken over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MixFix {
    from: u8,
    weight: Tween,
}

impl Composer {
    /// An empty terrarium: the first update fades everything in.
    pub fn new(table: &ArtTable, salt: InstallSalt) -> Composer {
        Composer {
            salt,
            bands: Bands::new(table),
            layout: Layout::new(salt),
            world: WorldLayout::new(salt, table.world()),
            light_ms: table.world().light_ms,
            label_capacity: table.world().label_capacity,
            sweep_width: table.world().sweep_width,
            viewport: Viewport::SCENE,
            placement: Placement::default(),
            monuments: BTreeMap::new(),
            fossils: BTreeMap::new(),
            entries: BTreeMap::new(),
            label: Vec::new(),
            clock: None,
            day_night: DayNight::default(),
            mix_fix: None,
            dim: Tween::at(0),
            eras: Eras::default(),
            resident_spots: BTreeMap::new(),
            residents: BTreeMap::new(),
            residents_settle: Millis(0),
            sky: table.sky_events().clone(),
            played: Vec::new(),
            curtain: OnceCell::new(),
            show: None,
            ground_art: table.ground_events().clone(),
            stage: ground::Stage::default(),
            fresh: BTreeMap::new(),
            sunk: BTreeMap::new(),
            hover: hover::HoverLayer::default(),
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn placement(&self) -> &Placement {
        &self.placement
    }

    /// Each monument's spot, by id.
    pub fn monument_spots(&self) -> &BTreeMap<u32, u8> {
        &self.monuments
    }

    /// Each fossil's spot, by id.
    pub fn fossil_spots(&self) -> &BTreeMap<u32, u8> {
        &self.fossils
    }

    /// The fixed scenery of this install, from the composer's own salt.
    pub fn scenery(&self, art: &Art, table: &ArtTable) -> Scenery {
        Scenery::new(art, table, &self.world, self.viewport)
    }

    /// Moves the depiction toward `input` and `overlay` from `now`. The frame at `now` does
    /// not change.
    pub fn update(&mut self, input: &SceneInput, overlay: &OverlayInput, now: Millis) {
        for entry in self.entries.values_mut() {
            entry.prune(now);
        }
        self.entries
            .retain(|_, entry| entry.looks().next().is_some());
        // Where the scene's marks lie is read from the scene before this one: the weeds a burn
        // took, the plant a sinkhole swallowed.
        self.remember_marks(input, now);
        self.placement = place(&self.placement, input, &self.bands);
        self.monuments = assign_spots(
            &self.monuments,
            input.monuments.iter().map(|m| m.id),
            world::MONUMENTS,
        );
        self.fossils = assign_spots(
            &self.fossil_claims(input),
            input.fossils.iter().map(|f| f.id),
            world::FOSSILS,
        );
        self.settle_sunk(input);
        let wanted = self.wanted(input, overlay);
        // A change of the sky waits for a wipe crossing to pass; one the event begins is the
        // change itself.
        let sky_from = self.sky_from(now);
        let wipe = self.cue(input, &wanted, now);
        if let Some(wipe) = wipe {
            self.wipe_sky(&wanted, now, wipe.end());
        }
        let start = |subject: Subject| if sky(subject) { sky_from } else { now };
        let moved = |subject: Subject| wipe.is_none() || !sky(subject);
        for (subject, entry) in &mut self.entries {
            if !wanted.contains_key(subject) && moved(*subject) {
                entry.hide(start(*subject), timing(*subject, self.light_ms).leave);
            }
        }
        for (subject, pieces) in wanted.into_iter().filter(|(s, _)| moved(*s)) {
            let timing = timing(subject, self.light_ms);
            let starts = self.starts(subject, start(subject), now);
            let entry = self.entries.entry(subject).or_default();
            let rise = if entry.shown() {
                timing.change
            } else {
                timing.arrive
            };
            entry.show(pieces, starts, rise, timing.change);
        }
        self.note_left(now);
        self.update_label(&overlay.label, now);
        self.hover.set(&overlay.hover, now);
        self.update_clock(overlay, now);
        let residents = self.update_residents(input, now);
        let snapshot = self.snapshot(input, overlay, now, residents);
        let running = turn(now);
        self.note_arrivals(&snapshot, running);
        if resident::pending(self.eras.at(running), &snapshot, running, &self.arrivals()) {
            let settles = turn_start(running + 1).plus(resident::CHANGE_SETTLES_MS);
            self.residents_settle = self.residents_settle.max(settles);
        }
        // Residents an earthquake sends into hiding come out the turn after.
        if let Some(shaken) = snapshot.shaken.filter(|_| !snapshot.residents.is_empty())
            && shaken.turn > running
        {
            let settles = turn_start(shaken.turn + 1).plus(resident::CHANGE_SETTLES_MS);
            self.residents_settle = self.residents_settle.max(settles);
        }
        self.eras.record(now, snapshot);
        let dim = if overlay.dim { DIM } else { 0 };
        if self.dim.target() != dim {
            let ms = if overlay.dim { DIM_DOWN_MS } else { DIM_UP_MS };
            self.dim = Tween::new(self.dim.value(now), dim, now, ms);
        }
    }

    /// Gives each resident its stretch and home, fades in the arrivals and fades out those
    /// that left; returns the residents drawn, by id.
    fn update_residents(&mut self, input: &SceneInput, now: Millis) -> Vec<Resident> {
        // A resident still fading out keeps its stretch, so one that comes back finds it
        // again and no newcomer stands in its place while it goes; but never at the cost of a
        // resident of the scene going undrawn, which would hide it until the next update.
        let present: Vec<u32> = input.residents.iter().map(|r| r.id).collect();
        let fading: Vec<u32> = self
            .residents
            .iter()
            .filter(|(_, shown)| now < shown.opacity.end() || shown.opacity.target() == SHOWN)
            .map(|(&(id, _), _)| id)
            .collect();
        let held = assign_spots(
            &self.resident_spots,
            present.iter().chain(&fading).copied(),
            world::RESIDENTS,
        );
        let all_drawn = present
            .iter()
            .take(world::RESIDENTS)
            .all(|id| held.contains_key(id));
        self.resident_spots = if all_drawn {
            held
        } else {
            assign_spots(&self.resident_spots, present, world::RESIDENTS)
        };
        let mut records: Vec<Resident> = input
            .residents
            .iter()
            .filter_map(|view| {
                let spot = *self.resident_spots.get(&view.id)?;
                Some(self.home(*view, spot))
            })
            .collect();
        records.sort_by_key(|record| record.view.id);
        for (&key, shown) in &mut self.residents {
            if shown.opacity.target() == SHOWN
                && !records.iter().any(|r| (r.view.id, r.spot) == key)
            {
                shown.opacity = Tween::new(shown.opacity.value(now), 0, now, TRANSITION_MS);
            }
        }
        self.residents
            .retain(|_, shown| shown.opacity.target() == SHOWN || now < shown.opacity.end());
        for record in &records {
            let key = (record.view.id, record.spot);
            let shown = self.residents.entry(key).or_insert(Shown {
                opacity: Tween::new(0, 0, now, 0),
                last: *record,
                arrival: None,
            });
            shown.last = *record;
            if shown.opacity.target() != SHOWN {
                shown.opacity = Tween::new(shown.opacity.value(now), SHOWN, now, TRANSITION_MS);
            }
        }
        records
    }

    /// What each resident arrived as, with the scene it arrived with, by id and stretch.
    fn arrivals(&self) -> resident::Arrivals<'_> {
        self.residents
            .iter()
            .filter_map(|(&key, shown)| {
                let (_, record, scene) = shown.arrival.as_ref()?;
                Some((key, (record, scene)))
            })
            .collect()
    }

    /// Freezes what each resident the running turn does not read arrived as, with the scene
    /// it arrived with: a later update in the same turn changes it only from the next.
    fn note_arrivals(&mut self, snapshot: &Snapshot, running: u64) {
        let read = self.eras.at(running);
        for record in &snapshot.residents {
            let key = (record.view.id, record.spot);
            let in_turn =
                read.is_some_and(|era| era.residents.iter().any(|r| (r.view.id, r.spot) == key));
            if let Some(shown) = self.residents.get_mut(&key)
                && !in_turn
                && shown
                    .arrival
                    .as_ref()
                    .is_none_or(|&(turn, ..)| turn != running)
            {
                shown.arrival = Some((running, *record, snapshot.clone()));
            }
        }
    }

    /// Where a resident lives in its stretch: beside the first vigorous plant there, in cell
    /// order, where it feeds; otherwise at the stretch's own place. Which plant is in which
    /// stretch, and which way there is more room, go by the cells' places before the salt
    /// moves them, so only where it stands is the salt's.
    fn home(&self, view: ResidentView, spot: u8) -> Resident {
        let (from, to) = world::resident_stretch(usize::from(spot));
        let vigorous = self
            .placement
            .plants
            .values()
            .filter(|placed| {
                placed.view.stage != GrowthStage::Seedling
                    && placed.view.state == PlantState::Thriving
                    && (from..to).contains(&slots::centre(usize::from(placed.cell)))
            })
            .map(|placed| placed.cell)
            .min();
        let (home, base) = match vigorous {
            Some(cell) => (self.layout.slot(cell).x, slots::centre(usize::from(cell))),
            None => (self.world.resident(spot), i32::midpoint(from, to)),
        };
        let (left, right) = (base - from, to - base);
        let out = if right > left || (right == left && base < FRAME_WIDTH / 2) {
            1
        } else {
            -1
        };
        Resident {
            view,
            spot,
            home,
            plant: vigorous,
            out,
        }
    }

    /// What the turns after `now` read of this scene.
    fn snapshot(
        &self,
        input: &SceneInput,
        overlay: &OverlayInput,
        now: Millis,
        residents: Vec<Resident>,
    ) -> Snapshot {
        let swaying = self
            .placement
            .plants
            .iter()
            .filter(|(_, placed)| sways(placed.view.stage, placed.view.state))
            .map(|(&id, placed)| Subject::Plant {
                id,
                cell: placed.cell,
            })
            .collect();
        Snapshot {
            state: input.presentation.state,
            weather: input.weather,
            wind: event::wind(input),
            dim: overlay.dim,
            clock: Clock {
                local: overlay.clock,
                at: now,
                day_night: overlay.day_night,
            },
            swaying,
            residents,
            shaken: self.shaken(input, now),
        }
    }

    /// The earthquake the scene carries, and the turn it shakes: the one after the update that
    /// first carried it, kept while later updates carry it on.
    fn shaken(&self, input: &SceneInput, now: Millis) -> Option<Shaken> {
        let event = input
            .event
            .filter(|event| event.kind == EventKind::Earthquake)?;
        let carried = self
            .eras
            .latest()
            .and_then(|snapshot| snapshot.shaken)
            .filter(|shaken| shaken.instance == event.started_tick);
        Some(carried.unwrap_or(Shaken {
            instance: event.started_tick,
            turn: turn(now) + 1,
        }))
    }

    /// What every resident does in turn `w`, by id. It never reads the salt (I-2).
    pub fn turn_plans(&self, art: &Art, table: &ArtTable, w: u64) -> Vec<TurnPlan> {
        self.resident_plans(art, table, w)
            .into_values()
            .map(|(plan, _)| plan)
            .collect()
    }

    /// Every resident's plan in turn `w`: those the turn reads, and those that arrived or
    /// left in it, resting.
    fn resident_plans(
        &self,
        art: &Art,
        table: &ArtTable,
        w: u64,
    ) -> BTreeMap<(u32, u8), (TurnPlan, Resident)> {
        let catalog = &art.catalog;
        let mut plans: BTreeMap<(u32, u8), (TurnPlan, Resident)> = BTreeMap::new();
        let before = self.eras.at(w.saturating_sub(1)).filter(|_| w > 0);
        let arrivals = self.arrivals();
        if let Some(now) = self.eras.at(w) {
            for plan in resident::plans(catalog, table, w, before, now, &arrivals) {
                if let Some(record) = now.residents.iter().find(|r| r.view.id == plan.id) {
                    plans.insert((plan.id, record.spot), (plan, *record));
                }
            }
        }
        for (&key, shown) in &self.residents {
            if plans.contains_key(&key) {
                continue;
            }
            // An arrival rests as it arrived; one fading out after leaving rests as it did in
            // the turn before.
            let arrived = shown
                .arrival
                .as_ref()
                .filter(|&&(turn, ..)| turn == w)
                .map(|(_, record, scene)| (record, scene, w));
            let plan = arrived
                .or_else(|| {
                    let (record, scene) = resident::earlier(&shown.last, before, &arrivals)?;
                    Some((record, scene, w.saturating_sub(1)))
                })
                .and_then(|(record, scene, read)| {
                    Some((
                        resident::resting(catalog, table, w, record, scene, read)?,
                        *record,
                    ))
                });
            if let Some(plan) = plan {
                plans.insert(key, plan);
            }
        }
        plans
    }

    /// How many pixels downwind a plant's crown leans at `now`: only in a turn it gusts in.
    fn crown(&self, subject: Subject, now: Millis) -> i32 {
        let w = turn(now);
        let Some(snapshot) = self.eras.at(w) else {
            return 0;
        };
        let (Some(cell), Some(j)) = (subject.cell(), motion_turn(w, snapshot.dim)) else {
            return 0;
        };
        if !snapshot.swaying.contains(&subject) || !sway::gusts(cell, j) {
            return 0;
        }
        sway::crown(snapshot.wind, cell, now.0 - turn_start(w).0)
    }

    /// How a look of a plant leans at `now`: its foliage about the foliage's pivot, and a
    /// bloom with the foliage it sits on, row for row. Nothing else leans.
    fn lean(
        &self,
        art: &Art,
        table: &ArtTable,
        subject: Subject,
        piece: Piece,
        now: Millis,
    ) -> Shear {
        let crown = self.crown(subject, now);
        if crown == 0 {
            return Shear::NONE;
        }
        let catalog = &art.catalog;
        let pivot = |foliage: SpriteId| catalog.sprite(foliage).shear_pivot.map(|p| p.y);
        match piece {
            Piece::Foliage { .. } => piece
                .sprite(table, self.salt, subject, FOLIAGE_PART)
                .and_then(pivot)
                .map_or(Shear::NONE, |pivot| sway::shear(pivot, crown)),
            Piece::Bloom { species } => {
                let PlantArt::Parts { foliage, .. } =
                    table.plant(species, GrowthStage::Mature, PlantState::Thriving)
                else {
                    return Shear::NONE;
                };
                let Some(host) = pivot(foliage) else {
                    return Shear::NONE;
                };
                // Both stand on the same ground point: the foliage's pivot row, counted from
                // the bloom's top.
                let bloom = catalog.sprite(table.bloom(species)).anchor.y;
                let leaning = sway::shear(host, crown);
                Shear {
                    pivot_y: bloom - catalog.sprite(foliage).anchor.y + host,
                    slope: leaning.slope,
                }
            }
            _ => Shear::NONE,
        }
    }

    /// Takes the overlay's clock and day and night times. Where they put the palette
    /// somewhere other than it stands, it eases over rather than jumping: after a sleep, a
    /// change of time zone or the first update.
    fn update_clock(&mut self, overlay: &OverlayInput, now: Millis) {
        let before = self.palette_mix(now);
        self.clock = Some((overlay.clock, now));
        self.day_night = overlay.day_night;
        let after = self.clock_mix(now);
        self.mix_fix = (before != after).then(|| MixFix {
            from: before,
            weight: Tween::new(0, SHOWN, now, CLOCK_FIX_MS),
        });
    }

    /// The local time of day at `now`, carried forward from the last update's clock.
    fn ms_of_day(&self, now: Millis) -> Option<u32> {
        let (clock, at) = self.clock?;
        Some(clock.advanced_by(now.0.saturating_sub(at.0)).ms_of_day)
    }

    /// The palette mix the clock alone gives at `now`: day until a clock arrives.
    fn clock_mix(&self, now: Millis) -> u8 {
        self.ms_of_day(now).map_or(0, |ms| self.day_night.mix(ms))
    }

    /// The palette mix at `now`, 0 by day and 255 by night.
    pub fn palette_mix(&self, now: Millis) -> u8 {
        let by_clock = self.clock_mix(now);
        match self.mix_fix {
            Some(fix) => mix(fix.from, by_clock, fix.weight.value(now)),
            None => by_clock,
        }
    }

    /// How far the frame is dimmed at `now`: 0, or [`DIM`] in dim mode, with fades between.
    pub fn dim(&self, now: Millis) -> u8 {
        self.dim.value(now)
    }

    /// Every subject the scene holds, with the pieces of its parts.
    fn wanted(&self, input: &SceneInput, overlay: &OverlayInput) -> BTreeMap<Subject, Pieces> {
        let mut wanted: BTreeMap<Subject, Pieces> = BTreeMap::new();
        let state = input.presentation.state;
        let quiet = dressing(state);
        for (&id, placed) in &self.placement.plants {
            let subject = Subject::Plant {
                id,
                cell: placed.cell,
            };
            wanted.insert(subject, plant_pieces(&placed.view));
        }
        for (&cell, &form) in &self.placement.fill {
            wanted.insert(Subject::Fill { cell }, fill_pieces(form));
        }
        for (&cell, &form) in &self.placement.weeds {
            wanted.insert(Subject::Weed { cell }, one(Piece::Weed(form)));
        }
        if quiet.tufts {
            for i in places(world::DRESSING) {
                wanted.insert(Subject::Dressing(i), one(Piece::DressingTuft));
            }
        }
        let litter = if state == PresentationState::Dead {
            LitterSet::DeadStems
        } else {
            LitterSet::Amber
        };
        for i in places(world::LITTER).take(usize::from(input.litter)) {
            wanted.insert(Subject::Litter(i), one(Piece::Litter(litter)));
        }
        for i in places(world::FUNGUS).take(usize::from(input.fungus)) {
            let mut pieces = one(Piece::Fungus);
            pieces[1] = Some(Piece::FungusGlow);
            wanted.insert(Subject::Fungus(i), pieces);
        }
        for monument in &input.monuments {
            let Some(&spot) = self.monuments.get(&monument.id) else {
                continue;
            };
            let age = monument.age;
            let mut pieces = [None; PARTS];
            pieces[STONE] = Some(Piece::Stone(age));
            let marks = usize::from(monument.marks).min(LICHEN_GLOW - FIRST_MARK);
            for part in &mut pieces[FIRST_MARK..FIRST_MARK + marks] {
                *part = Some(Piece::Mark { age });
            }
            if age != MonumentAge::Young {
                pieces[LICHEN_GLOW] = Some(Piece::LichenGlow(age));
            }
            wanted.insert(
                Subject::Monument {
                    id: monument.id,
                    spot,
                },
                pieces,
            );
        }
        for fossil in &input.fossils {
            if let Some(&spot) = self.fossils.get(&fossil.id) {
                let subject = Subject::Fossil {
                    id: fossil.id,
                    spot,
                };
                wanted.insert(subject, one(Piece::Fossil(fossil.form)));
            }
        }
        for (subject, kind) in self.marks(input) {
            wanted.insert(subject, one(Piece::Ground(kind)));
        }
        let weather = input.weather;
        wanted.insert(Subject::Weather, one(Piece::Weather(weather)));
        for (i, &size) in (0_u8..).zip(sky::clouds(weather).iter().take(quiet.clouds)) {
            wanted.insert(Subject::Cloud(i), one(Piece::Cloud(size)));
        }
        if sky::shows_sun_and_moon(weather) {
            if quiet.sun {
                wanted.insert(Subject::Sun, one(Piece::Sun));
            }
            if let Some(look) = moon_phase(overlay.clock.date).look() {
                wanted.insert(Subject::Moon, one(Piece::Moon(look)));
            }
        }
        if sky::shows_stars(weather) && quiet.stars {
            for i in places(world::STARS) {
                wanted.insert(Subject::Star(i), one(Piece::Star));
            }
        }
        if input.light != Light::Neutral {
            wanted.insert(Subject::Light, one(Piece::Light(input.light)));
        }
        for (film, level) in [
            (GlassFilm::Condensation, input.condensation),
            (GlassFilm::Dust, input.dust),
        ] {
            if level > 0 {
                wanted.insert(Subject::Film(film), one(Piece::Film(film, level)));
            }
        }
        wanted
    }

    /// Moves only the hover layer toward `hover` from `now`, as [`Composer::update`] does with
    /// the rest: a pointer's move never touches the scene. The frame at `now` does not change.
    pub fn hover(&mut self, hover: &Hover, now: Millis) {
        self.hover.set(hover, now);
    }

    /// The monument or fossil standing at `(x, y)` in the viewport at `now`, if any: its newest
    /// look's sprite, its whole box.
    pub fn landmark_at(
        &self,
        art: &Art,
        table: &ArtTable,
        x: i32,
        y: i32,
        now: Millis,
    ) -> Option<Landmark> {
        self.landmarks(art, table, now)
            .into_iter()
            .rev()
            .find(|placed| placed.standing && crate::panel::contains(placed.area, x, y))
            .map(|placed| placed.landmark)
    }

    /// Each look of each monument and fossil at `now`: its spot, where its sprite lies in the
    /// viewport, where a hover label over it stands, and how visible it is. A label is drawn
    /// over each look and fades with it, so when a stone changes age its label crossfades with
    /// it rather than moving.
    fn landmarks(&self, art: &Art, table: &ArtTable, now: Millis) -> Vec<hover::Placed> {
        let catalog = &art.catalog;
        let inset = self.viewport.inset();
        let mut out = Vec::new();
        for (&subject, entry) in &self.entries {
            let (landmark, spot) = match subject {
                Subject::Monument { id, spot } => (Landmark::Monument(id), spot),
                Subject::Fossil { id, spot } => (Landmark::Fossil(id), spot),
                _ => continue,
            };
            let looks = &entry.parts[0];
            for (index, look) in (0_u16..).zip(looks) {
                let Some(sprite) = look.piece.sprite(table, self.salt, subject, 0) else {
                    continue;
                };
                let rect = catalog.sprite(sprite).rect;
                let draw = match landmark {
                    Landmark::Monument(_) => {
                        Draw::anchored(catalog, sprite, self.world.monument(spot), PLANT_GROUND_Y)
                    }
                    Landmark::Fossil(_) => {
                        let y = FRAME_HEIGHT - i32::from(rect.h);
                        Draw::new(catalog, sprite, self.world.fossil(spot), y)
                    }
                };
                let area = Area {
                    x: draw.x + inset,
                    y: draw.y,
                    w: i32::from(rect.w),
                    h: i32::from(rect.h),
                };
                let (centre, top) = match landmark {
                    Landmark::Monument(_) => (self.world.monument(spot) + inset, area.y),
                    Landmark::Fossil(_) => (area.x + area.w / 2, GROUND),
                };
                let newest = usize::from(index) + 1 == looks.len();
                out.push(hover::Placed {
                    landmark,
                    spot,
                    look: index,
                    area,
                    centre,
                    top,
                    opacity: look.opacity.value(now),
                    standing: newest && look.opacity.target() == SHOWN,
                });
            }
        }
        out
    }

    /// Whether line `index` of the label's lines showing at `now` has had its turn on the plate:
    /// the cycle has moved on from it at least once. A set of one line never moves on, and a
    /// new set starts its cycle again.
    pub fn label_step_done(&self, index: usize, now: Millis) -> bool {
        let Some(look) = self
            .label
            .last()
            .filter(|look| look.opacity.target() == SHOWN)
        else {
            return false;
        };
        if look.lines.len() <= 1 || index >= look.lines.len() {
            return false;
        }
        let steps = u64::try_from(index).map_or(u64::MAX, |index| index.saturating_add(1));
        now.0.saturating_sub(look.since.0) >= steps.saturating_mul(u64::from(LABEL_CYCLE_MS))
    }

    /// Raises a new set of label lines when they change, crossfading from the old. The lines
    /// are fitted to the plate here, once, rather than on every frame.
    fn update_label(&mut self, lines: &[String], now: Millis) {
        let lines: Vec<String> = lines
            .iter()
            .map(|line| fit(line, self.label_capacity))
            .collect();
        self.label
            .retain(|look| look.opacity.target() == SHOWN || now < look.opacity.end());
        let current = self
            .label
            .last()
            .filter(|look| look.opacity.target() == SHOWN);
        if current.is_some_and(|look| look.lines == lines)
            || (current.is_none() && lines.is_empty())
        {
            return;
        }
        for look in &mut self.label {
            if look.opacity.target() == SHOWN {
                look.opacity = Tween::new(look.opacity.value(now), 0, now, LABEL_FADE_MS);
            }
        }
        if !lines.is_empty() {
            self.label.push(LabelLook {
                lines,
                since: now,
                opacity: Tween::new(0, SHOWN, now, LABEL_FADE_MS),
            });
        }
    }

    /// The first moment after `now` at which the frame may change, or `None` if nothing will
    /// until the next update. Every source of change answers here; the frame is the same at
    /// every moment before it (I-10).
    pub fn next_change(&self, art: &Art, table: &ArtTable, now: Millis) -> Option<Millis> {
        let mut next = Earliest::default();
        // While the storm front crosses, a step of the sky that its cut hides before and after
        // changes nothing; the front's own steps look again.
        let crossing = self.crossing(now).is_some();
        let seen = |subject: Subject, step: Millis, looks: &[(Place, &Look)]| {
            !crossing
                || !sky(subject)
                || [now, step].iter().any(|&at| {
                    looks
                        .iter()
                        .any(|&(place, look)| !self.covered(art, table, place, look, at))
                })
        };
        for (&subject, entry) in &self.entries {
            for (part, looks) in (0_u8..).zip(&entry.parts) {
                for (index, look) in (0_u16..).zip(looks) {
                    let Some(step) = look.opacity.next_step(now) else {
                        continue;
                    };
                    // Art drawn at its own opacity, the same by day and night, shows a step
                    // of its look's only where the scaled opacity moves.
                    let own = look
                        .piece
                        .sprite(table, self.salt, subject, part)
                        .map(|sprite| art.catalog.sprite(sprite))
                        .filter(|own| {
                            own.night_opacity.is_none()
                                && !own.flags.contains(Flags::DAY_ONLY)
                                && !own.flags.contains(Flags::NIGHT_ONLY)
                        });
                    let step = match own {
                        Some(own) => look
                            .opacity
                            .next_step_by(now, |value| scale(own.opacity, value)),
                        None => Some(step),
                    };
                    let place = Place::new(subject, part, index);
                    next.take(step.filter(|&step| seen(subject, step, &[(place, look)])));
                }
            }
            if let Subject::Cloud(i) = subject {
                let shown: Vec<(Place, &Look)> = (0_u16..)
                    .zip(&entry.parts[0])
                    .filter(|(_, look)| look.opacity.value(now) > 0)
                    .map(|(index, look)| (Place::new(subject, 0, index), look))
                    .collect();
                let widest = shown
                    .iter()
                    .filter_map(|(_, look)| look.piece.sprite(table, self.salt, subject, 0))
                    .map(|sprite| i32::from(art.catalog.sprite(sprite).rect.w))
                    .max();
                let drift = self.world.cloud(i);
                let viewport = self.viewport_width();
                let step = widest.map(|width| drift.next_seen_step(now, width, viewport));
                next.take(step.filter(|&step| seen(subject, step, &shown)));
            }
        }
        next.take(Some(drift::next_step(now, SOIL_MS_PER_PX)));
        next.take(Some(drift::sweep_next(
            self.salt,
            now,
            self.sweep_width,
            self.viewport_width(),
        )));
        for look in &self.label {
            next.take(look.opacity.next_step(now));
            if look.opacity.value(now) > 0 || look.opacity.target() == SHOWN {
                next.take(look.next_change(now));
            }
        }
        if let Some(fix) = self.mix_fix {
            next.take(fix.weight.next_step(now));
        }
        if let Some(ms) = self.ms_of_day(now) {
            next.take(self.day_night.next_change(ms).map(|ahead| now.plus(ahead)));
        }
        next.take(self.dim.next_step(now));
        for shown in self.residents.values() {
            next.take(shown.opacity.next_step(now));
        }
        next.take(self.motion_change(art, table, now));
        next.take(self.show_next(art, table, now));
        next.take(self.hover.next_change(art, self.viewport_width(), now));
        next.0
    }

    /// The first moment after `now` at which a turn's motion changes the frame. Turns to come
    /// are searched until one moves, as far as the longest wait for a gust.
    fn motion_change(&self, art: &Art, table: &ArtTable, now: Millis) -> Option<Millis> {
        let running = turn(now);
        for w in running..=running + MOTION_AHEAD {
            let start = turn_start(w);
            // Into the running turn; any moment of a turn to come is after `now`.
            let since = now.0.saturating_sub(start.0);
            let at = |ms: u64| Millis(start.0 + ms);
            let mut next = Earliest::default();
            if let Some(snapshot) = self.eras.at(w)
                && let Some(j) = motion_turn(w, snapshot.dim)
            {
                for subject in &snapshot.swaying {
                    if let Some(cell) = subject.cell()
                        && sway::gusts(cell, j)
                    {
                        next.take(sway::next_move(snapshot.wind, cell, since).map(at));
                    }
                }
            }
            let end = turn_start(w + 1);
            for (plan, _) in self.resident_plans(art, table, w).values() {
                next.take(
                    plan_next(&art.catalog, table, plan, now, start).filter(|&moment| moment < end),
                );
            }
            // A turn to come begins with each resident where its plan puts it, which need not
            // be where the turn before left it: another place, facing or routine.
            if start > now && self.residents_change_at(art, table, start) {
                next.take(Some(start));
            }
            if next.0.is_some() {
                return next.0;
            }
        }
        None
    }

    /// Whether any resident is drawn otherwise at `at` than just before it.
    fn residents_change_at(&self, art: &Art, table: &ArtTable, at: Millis) -> bool {
        let residents = |now: Millis| {
            let mut out = Vec::new();
            self.push_residents(art, table, self.palette_mix(now), now, &mut out);
            out
        };
        residents(Millis(at.0.saturating_sub(1))) != residents(at)
    }

    /// When the last tween ends, and the changes of rest an update brought are over. From
    /// then on only motion moves until the next update: drift, the label's cycle, sway and the
    /// residents' turns.
    pub fn settled_at(&self) -> Millis {
        self.entries
            .values()
            .flat_map(Entry::looks)
            .map(|look| look.opacity)
            .chain(self.label.iter().map(|look| look.opacity))
            .chain(self.mix_fix.map(|fix| fix.weight))
            .chain([self.dim])
            .chain(self.residents.values().map(|shown| shown.opacity))
            .map(Tween::end)
            .chain([self.residents_settle, self.hover.end()])
            .chain(self.show_end())
            .max()
            .unwrap_or_default()
    }

    /// Every visible look at `now`, as depicted states in subject order. The label's text,
    /// which reads no salt, is not among them.
    pub fn depicted(&self, now: Millis) -> Vec<Depicted> {
        let mut depicted = Vec::new();
        for (&(id, spot), shown) in &self.residents {
            let opacity = shown.opacity.value(now);
            if opacity > 0 {
                let view = shown.last.view;
                depicted.push(Depicted {
                    subject: Subject::Resident { id, spot },
                    part: 0,
                    piece: Piece::Resident {
                        species: view.species,
                        stage: view.stage,
                    },
                    opacity,
                });
            }
        }
        for (&subject, entry) in &self.entries {
            for (part, looks) in (0_u8..).zip(&entry.parts) {
                let veil = self.veil(subject, part, now);
                for look in looks {
                    let opacity = scale(look.opacity.value(now), veil);
                    if opacity > 0 {
                        depicted.push(Depicted {
                            subject,
                            part,
                            piece: look.piece,
                            opacity,
                        });
                    }
                }
            }
        }
        depicted.extend(self.show_depicted(now));
        depicted
    }

    /// The composer's draws at `now` and `palette_mix`, each keyed by its place in the depth
    /// order, in that order. The scenery is not among them.
    pub fn draws(
        &self,
        art: &Art,
        table: &ArtTable,
        palette_mix: u8,
        now: Millis,
    ) -> Vec<KeyedDraw> {
        let mut keyed = Vec::new();
        // While a wipe crosses, which of the sky's draws are behind it.
        let crossing = self.crossing(now).is_some();
        let mut sky_draws = Vec::new();
        for (&subject, entry) in &self.entries {
            for (part, looks) in (0_u8..).zip(&entry.parts) {
                let veil = self.veil(subject, part, now);
                for (index, look) in (0_u16..).zip(looks) {
                    let value = scale(look.opacity.value(now), veil);
                    if value == 0 {
                        continue;
                    }
                    let place = Place::new(subject, part, index);
                    let lit = Lit { value, palette_mix };
                    let from = keyed.len();
                    self.push(art, table, place, look.piece, lit, now, &mut keyed);
                    if crossing && sky(subject) {
                        sky_draws.extend((from..keyed.len()).map(|i| (i, look.behind)));
                    }
                }
            }
        }
        self.push_label(art, now, &mut keyed);
        self.push_drifts(art, table, now, &mut keyed);
        self.push_residents(art, table, palette_mix, now, &mut keyed);
        self.push_ground(art, table, palette_mix, now, &mut keyed);
        // Only a label needs to know where the landmarks stand.
        let landmarks = if self.hover.has_tags() {
            self.landmarks(art, table, now)
        } else {
            Vec::new()
        };
        self.hover
            .push(art, &landmarks, self.viewport_width(), now, &mut keyed);
        // The scene stands in the middle of the viewport; everything else is laid out across
        // the viewport already.
        let inset = self.viewport.inset();
        for keyed in keyed
            .iter_mut()
            .filter(|keyed| keyed.key.subject.in_scene())
        {
            keyed.key.x += inset;
            keyed.draw.x += inset;
            if let Some(clip) = &mut keyed.draw.clip {
                clip.x += inset;
            }
        }
        self.push_show(art, table, palette_mix, now, &mut keyed, &sky_draws);
        keyed.sort_by_key(|keyed| keyed.key);
        keyed
    }

    /// Composes for `viewport` from now on; its scenery is [`Composer::scenery`]'s anew.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
        self.refit_show();
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    fn viewport_width(&self) -> i32 {
        i32::try_from(self.viewport.width()).unwrap_or(FRAME_WIDTH)
    }

    /// Every resident at `now`: its turn's pose, a cover or a move over the one before while
    /// it plays, and a firefly's lantern while it signals.
    fn push_residents(
        &self,
        art: &Art,
        table: &ArtTable,
        palette_mix: u8,
        now: Millis,
        out: &mut Vec<KeyedDraw>,
    ) {
        let w = turn(now);
        let plans = self.resident_plans(art, table, w);
        let before = self.eras.at(w.saturating_sub(1));
        let arrivals = self.arrivals();
        for (&(id, spot), shown) in &self.residents {
            let visible = shown.opacity.value(now);
            let Some((plan, record)) = plans.get(&(id, spot)).filter(|_| visible > 0) else {
                continue;
            };
            let subject = Subject::Resident { id, spot };
            // Where it stood the turn before, for a move's pose fading out there: in the scene
            // that turn read, or as it arrived.
            let old_home =
                resident::earlier(record, before, &arrivals).map_or(record.home, |(r, _)| r.home);
            // Until it moves, it stands where it stood.
            let moves = plan.segments.iter().position(|s| s.enter == Enter::Moved);
            for (look, index, segment, opacity) in poses(plan, now) {
                let home = if moves.is_some_and(|moves| index < moves) {
                    old_home
                } else {
                    record.home
                };
                let lit = Lit {
                    value: scale(opacity, visible),
                    palette_mix,
                };
                push_creature(
                    art,
                    table,
                    place_of(subject, look),
                    &segment,
                    home,
                    record.out,
                    lit,
                    now,
                    out,
                );
            }
        }
    }

    /// The soil band at its drift, and the reflection while one crosses the glass.
    fn push_drifts(&self, art: &Art, table: &ArtTable, now: Millis, out: &mut Vec<KeyedDraw>) {
        let catalog = &art.catalog;
        let offset = drift::offset(now, SOIL_MS_PER_PX, SOIL_PERIOD);
        let width = self.viewport_width();
        for (part, item, draw) in world::soil(catalog, width, offset) {
            // Keyed at the band's origin, so the tiles paint before anything lying on them.
            let place = Place::new(Subject::Soil, part, 0).item(item);
            out.push(KeyedDraw::at(place, (0, GROUND), draw));
        }
        let world = table.world();
        if let Some(x) = drift::sweep_x(self.salt, now, world.sweep_width, width) {
            let draw = Draw::new(catalog, world.sweep, x, 0);
            // Faded, but never to nothing while it is named a step: each step it takes shows.
            let fade = drift::sweep_fade(x, world.sweep_width, width);
            let draw = Draw {
                opacity: scale(draw.opacity, fade).max(1),
                ..draw
            };
            // Keyed past the frame's far corner: it paints over all the rest of the glass.
            let place = Place::new(Subject::Sweep, 0, 0);
            out.push(KeyedDraw::at(place, (width, FRAME_HEIGHT), draw));
        }
    }

    /// One look's draws, positioned for its subject.
    #[expect(
        clippy::too_many_arguments,
        reason = "one look's draws need where it is, what it is and how lit, each its own"
    )]
    fn push(
        &self,
        art: &Art,
        table: &ArtTable,
        place: Place,
        piece: Piece,
        lit: Lit,
        now: Millis,
        out: &mut Vec<KeyedDraw>,
    ) {
        let catalog = &art.catalog;
        let Some(sprite) = piece.sprite(table, self.salt, place.subject, place.part) else {
            return;
        };
        let rect = catalog.sprite(sprite).rect;
        let mut push = |place: Place, at: Option<(i32, i32)>, draw: Draw| {
            let Some(draw) = lit.apply(art, draw) else {
                return;
            };
            out.push(match at {
                Some(point) => KeyedDraw::at(place, point, draw),
                None => KeyedDraw::placed(catalog, place, draw),
            });
        };
        let ground = |x| Draw::anchored(catalog, sprite, x, PLANT_GROUND_Y);
        let at = |(x, y)| Draw::new(catalog, sprite, x, y);
        match place.subject {
            Subject::Weather => {
                // The backdrop is the whole scene, held still at its place: a new weather
                // covers the old one everywhere.
                let backdrop = world::fixed(art, sprite, self.viewport);
                push(place, Some((0, 0)), backdrop);
            }
            Subject::Cloud(i) => {
                // Wholly out of sight, a cloud is not drawn, so its drift there changes nothing.
                let drift = self.world.cloud(i);
                let viewport = self.viewport_width();
                let x = drift.x(now, viewport);
                if x + i32::from(rect.w) > 0 && x < viewport {
                    push(place, None, Draw::new(catalog, sprite, x, drift.y));
                }
            }
            Subject::Sun => push(place, None, at(self.world.sun())),
            Subject::Moon => {
                let flip_x = matches!(piece, Piece::Moon(look) if look.mirrored);
                let draw = at(self.world.moon());
                push(place, None, Draw { flip_x, ..draw });
            }
            Subject::Star(i) => push(place, None, at(self.world.star(i))),
            Subject::Light | Subject::Film(_) => {
                let draws = match place.subject {
                    Subject::Light => light(catalog, sprite, self.viewport),
                    _ => film(catalog, sprite, self.viewport),
                };
                for (item, draw) in (0_u16..).zip(draws) {
                    push(place.item(item), None, draw);
                }
            }
            Subject::Monument { spot, .. } => {
                let x = self.world.monument(spot);
                let draw = match piece {
                    Piece::Mark { age } => mark(catalog, table, age, place.part, sprite, x),
                    _ => Some(ground(x)),
                };
                // Every part keyed at the monument's ground point, so the marks paint over
                // their stone rather than sorting under it.
                if let Some(draw) = draw {
                    push(place, Some((x, PLANT_GROUND_Y)), draw);
                }
            }
            Subject::Fossil { spot, .. } => {
                let (x, y) = (self.world.fossil(spot), FRAME_HEIGHT - i32::from(rect.h));
                push(place, None, Draw::new(catalog, sprite, x, y));
            }
            Subject::Mark { id, at } => {
                // Keyed at the ground line, after the soil's tiles, so it lies on them.
                if let Some((point, draw)) = self.mark_draw(art, table, piece, (id, at), sprite) {
                    push(place, Some(point), draw);
                }
            }
            Subject::Plant { cell, .. } | Subject::Fill { cell } | Subject::Weed { cell } => {
                let x = self.layout.slot(cell).x;
                let draw = match piece {
                    Piece::PlantGlow { species, stage } => {
                        let (cx, cy) = foliage_centre(art, table, species, stage, x);
                        Draw::anchored(catalog, sprite, cx, cy)
                    }
                    Piece::Stem { .. } | Piece::Foliage { .. } | Piece::Bloom { .. } => {
                        Draw::anchored(catalog, sprite, x, standing(piece))
                    }
                    _ => ground(x),
                };
                let shear = self.lean(art, table, place.subject, piece, now);
                push(place, None, Draw { shear, ..draw });
            }
            Subject::Dressing(i) => push(place, None, ground(self.world.dressing(i))),
            Subject::Litter(i) => push(place, None, ground(self.world.litter(i))),
            Subject::Fungus(i) => push(place, None, ground(self.world.fungus(i))),
            Subject::Scenery(_)
            | Subject::Label
            | Subject::Tag { .. }
            | Subject::Panel
            | Subject::Soil
            | Subject::Sweep
            | Subject::Resident { .. }
            | Subject::Front
            | Subject::Bolt
            | Subject::Rain
            | Subject::Leaf(_)
            | Subject::Scorch { .. }
            | Subject::Burn(_)
            | Subject::Sprinkle(_) => {}
        }
    }

    /// The label's text, over the plate the scenery draws.
    fn push_label(&self, art: &Art, now: Millis, out: &mut Vec<KeyedDraw>) {
        let catalog = &art.catalog;
        let Some(label) = Label::new(catalog) else {
            return;
        };
        for (index, look) in (0_u16..).zip(&self.label) {
            let value = look.opacity.value(now);
            if value == 0 {
                continue;
            }
            for (line, line_value) in look.showing(now) {
                let opacity = scale(value, line_value);
                let Some(text) = look.lines.get(line) else {
                    continue;
                };
                let part = u8::try_from(line).unwrap_or(u8::MAX);
                for (item, glyph) in (0_u16..).zip(label.fitted_draws(text, LABEL_X, LABEL_Y)) {
                    let opacity = scale(glyph.opacity, opacity);
                    if opacity == 0 {
                        continue;
                    }
                    let place = Place::new(Subject::Label, part, index).item(item);
                    out.push(KeyedDraw::placed(catalog, place, Draw { opacity, ..glyph }));
                }
            }
        }
    }

    /// The whole frame at `now`: the composer's draws merged with the fixed scenery, in depth
    /// order.
    pub fn frame(
        &self,
        art: &Art,
        table: &ArtTable,
        scenery: &Scenery,
        palette_mix: u8,
        now: Millis,
    ) -> Frame {
        debug_assert_eq!(
            scenery.viewport(),
            self.viewport,
            "scenery of another viewport"
        );
        Frame {
            draws: merged(&self.draws(art, table, palette_mix, now), scenery.keyed()),
            palette_mix,
            viewport: self.viewport,
            dim: self.dim(now),
        }
    }
}

/// The row a plant's part stands on: its plant's ground row.
fn standing(piece: Piece) -> i32 {
    match piece {
        Piece::Stem { species, stage } | Piece::Foliage { species, stage, .. } => {
            plant::ground_y(species, stage)
        }
        // Only a mature plant blooms.
        Piece::Bloom { species } => plant::ground_y(species, GrowthStage::Mature),
        _ => PLANT_GROUND_Y,
    }
}

/// A light across `viewport`. It is one colour across the scene, so a wider viewport takes a
/// second copy, cut where the first ends, and is one colour to both edges.
fn light(catalog: &crate::catalog::Catalog, sprite: SpriteId, viewport: Viewport) -> Vec<Draw> {
    let width = i32::try_from(viewport.width()).unwrap_or(FRAME_WIDTH);
    let own = i32::from(catalog.sprite(sprite).rect.w);
    let mut draws = vec![Draw::new(catalog, sprite, 0, 0)];
    if width > own {
        draws.push(Draw {
            clip: Some(beyond(own, width)),
            ..Draw::new(catalog, sprite, width - own, 0)
        });
    }
    draws
}

/// A glass film across `viewport`. It lies over the scene, and a wider viewport repeats its
/// droplets out to the edges, each copy cut where the film itself begins and ends.
fn film(catalog: &crate::catalog::Catalog, sprite: SpriteId, viewport: Viewport) -> Vec<Draw> {
    let width = i32::try_from(viewport.width()).unwrap_or(FRAME_WIDTH);
    let (inset, own) = (viewport.inset(), i32::from(catalog.sprite(sprite).rect.w));
    let copies = [
        (inset - own, beyond(0, inset)),
        (inset + own, beyond(inset + own, width)),
    ];
    let mut draws = vec![Draw::new(catalog, sprite, inset, 0)];
    draws.extend(
        copies
            .into_iter()
            .filter(|(_, clip)| clip.w > 0)
            .map(|(x, clip)| Draw {
                clip: Some(clip),
                ..Draw::new(catalog, sprite, x, 0)
            }),
    );
    draws
}

/// The full-height columns `from..to` of a viewport.
fn beyond(from: i32, to: i32) -> Area {
    Area {
        x: from,
        y: 0,
        w: to - from,
        h: FRAME_HEIGHT,
    }
}

/// The earliest of several moments, each perhaps none.
#[derive(Clone, Copy, Debug, Default)]
struct Earliest(Option<Millis>);

impl Earliest {
    fn take(&mut self, moment: Option<Millis>) {
        if let Some(moment) = moment {
            self.0 = Some(self.0.map_or(moment, |earliest| earliest.min(moment)));
        }
    }
}

/// One look of a resident: its frame, facing and standing where its segment has it, and
/// its lantern while the segment is a signal.
#[expect(
    clippy::too_many_arguments,
    reason = "a creature's look needs where it is, what it plays and how lit, each its own"
)]
fn push_creature(
    art: &Art,
    table: &ArtTable,
    place: Place,
    segment: &Segment,
    home: i32,
    out_way: i32,
    lit: Lit,
    now: Millis,
    out: &mut Vec<KeyedDraw>,
) {
    let catalog = &art.catalog;
    let animation = catalog.animation(segment.routine);
    let elapsed = if segment.still {
        0
    } else {
        now.0.saturating_sub(segment.anchor.0)
    };
    let pose = anim::pose(catalog, animation, elapsed);
    let (offset, facing_out) = match segment.way {
        Way::Out => (segment.from + pose.dx, true),
        Way::In => (segment.from - pose.dx, false),
    };
    let x = home + out_way * offset;
    let right = facing_out == (out_way > 0);
    let face = |draw: Draw| if right { draw } else { draw.mirrored(catalog) };
    let draw = face(Draw::anchored(catalog, pose.frame, x, PLANT_GROUND_Y));
    if let Some(draw) = lit.apply(art, draw) {
        out.push(KeyedDraw::placed(catalog, place, draw));
    }
    // A firefly's lantern plays in step with its signal, on the same anchor.
    if let Some(effect) = table.lantern(segment.routine) {
        let effect = catalog.animation(effect);
        let glow = anim::pose(catalog, effect, now.0.saturating_sub(segment.start.0));
        if !glow.done {
            let draw = face(Draw::anchored(catalog, glow.frame, x, PLANT_GROUND_Y));
            if let Some(draw) = lit.apply(art, draw) {
                let place = Place { part: 1, ..place };
                out.push(KeyedDraw::placed(catalog, place, draw));
            }
        }
    }
}

/// The looks of a resident's plan at `now`: the segment playing, and while it enters by a
/// cover or a move, the one before it under it, each with its look (0 under, 1 over), its
/// place in the plan, and its opacity.
fn poses(plan: &TurnPlan, now: Millis) -> Vec<(u16, usize, Segment, u8)> {
    let Some(i) = plan
        .segments
        .iter()
        .rposition(|segment| segment.start <= now)
    else {
        return plan
            .segments
            .first()
            .map(|&first| vec![(1, 0, first, SHOWN)])
            .unwrap_or_default();
    };
    let current = plan.segments[i];
    let Some((rise, fall)) = fades(&current) else {
        return vec![(1, i, current, SHOWN)];
    };
    let mut looks = vec![(1, i, current, rise.value(now))];
    if let Some(&under) = i.checked_sub(1).and_then(|j| plan.segments.get(j)) {
        let value = fall.value(now);
        if value > 0 {
            looks.insert(0, (0, i - 1, under, value));
        }
    }
    looks
}

/// The first moment after `after` at which a resident's plan for the turn starting at `start`
/// changes what it draws: a segment beginning (but not the first, which carries on from the
/// turn before), a frame of a look playing, a step of a cover or a move, or of a lantern.
fn plan_next(
    catalog: &crate::catalog::Catalog,
    table: &ArtTable,
    plan: &TurnPlan,
    after: Millis,
    start: Millis,
) -> Option<Millis> {
    let mut next = Earliest::default();
    for segment in plan.segments.iter().skip(1) {
        if segment.start > after {
            next.take(Some(segment.start));
        }
    }
    // For a turn to come, its first moments count: from just before it begins.
    let from = after.max(Millis(start.0.saturating_sub(1)));
    let at = from.max(start);
    for (_, _, segment, _) in poses(plan, at) {
        if !segment.still {
            let animation = catalog.animation(segment.routine);
            let elapsed = from.0.saturating_sub(segment.anchor.0);
            let frame = anim::next_frame_at(animation, elapsed);
            next.take(frame.map(|ms| Millis(segment.anchor.0 + ms).max(Millis(from.0 + 1))));
        }
        if let Some(effect) = table.lantern(segment.routine) {
            let effect = catalog.animation(effect);
            let elapsed = from.0.saturating_sub(segment.start.0);
            let glow = anim::next_frame_at(effect, elapsed);
            next.take(glow.map(|ms| Millis(segment.start.0 + ms).max(Millis(from.0 + 1))));
        }
    }
    if let Some((rise, fall)) = plan
        .segments
        .iter()
        .rfind(|segment| segment.start <= at)
        .and_then(fades)
    {
        next.take(rise.next_step(from));
        next.take(fall.next_step(from));
    }
    next.0.filter(|&moment| moment > after)
}

/// How a segment that does not enter straight takes over: its own pose rising and the one
/// before it falling. A cover holds the old pose until the new has risen, so nothing shows
/// through; a move fades one place into the other.
fn fades(segment: &Segment) -> Option<(Tween, Tween)> {
    let half = TRANSITION_MS / 2;
    match segment.enter {
        Enter::Straight => None,
        Enter::Cover => Some((
            Tween::new(0, SHOWN, segment.start, half),
            Tween::new(SHOWN, 0, segment.start.plus(half), half),
        )),
        Enter::Moved => Some((
            Tween::new(0, SHOWN, segment.start, TRANSITION_MS),
            Tween::new(SHOWN, 0, segment.start, TRANSITION_MS),
        )),
    }
}

/// Where a resident's look is keyed: the creature as part 0, its lantern as part 1.
fn place_of(subject: Subject, look: u16) -> Place {
    Place::new(subject, 0, look)
}

/// The places a kind of piece has, as subject indices.
fn places(n: usize) -> impl Iterator<Item = u8> {
    (0..n).filter_map(|i| u8::try_from(i).ok())
}

/// How long a subject takes to arrive, leave and change look.
fn timing(subject: Subject, light_ms: u32) -> Timing {
    let (arrive, leave, change) = match subject {
        Subject::Plant { .. } | Subject::Monument { .. } | Subject::Fossil { .. } => {
            (APPEAR_MS, VANISH_MS, HALF_RESTYLE)
        }
        Subject::Weather
        | Subject::Cloud(_)
        | Subject::Sun
        | Subject::Moon
        | Subject::Star(_)
        | Subject::Film(_) => (SKY_MS, SKY_MS, HALF_RESTYLE),
        Subject::Light => (light_ms, light_ms, light_ms),
        Subject::Fill { .. }
        | Subject::Weed { .. }
        | Subject::Dressing(_)
        | Subject::Litter(_)
        | Subject::Fungus(_)
        | Subject::Mark { .. }
        // Never entries: the label and the hover layer keep their own looks, the scenery never
        // changes, the soil and the sweep move with the clock alone, residents keep their own
        // looks, and an event's pieces are its choreography's. They are here only to keep the
        // match whole.
        | Subject::Label
        | Subject::Tag { .. }
        | Subject::Panel
        | Subject::Scenery(_)
        | Subject::Soil
        | Subject::Sweep
        | Subject::Resident { .. }
        | Subject::Front
        | Subject::Bolt
        | Subject::Rain
        | Subject::Leaf(_)
        | Subject::Scorch { .. }
        | Subject::Burn(_)
        | Subject::Sprinkle(_) => (FILL_MS, FILL_MS, HALF_RESTYLE),
    };
    Timing {
        arrive,
        leave,
        change,
    }
}

/// The sky's own subjects, which a wipe carries across: the weather's backdrop, the clouds,
/// the sun, the moon and the stars.
const fn sky(subject: Subject) -> bool {
    matches!(
        subject,
        Subject::Weather | Subject::Cloud(_) | Subject::Sun | Subject::Moon | Subject::Star(_)
    )
}

/// A carved mark on a stone of `age` standing at `x`: at its part's point on the stone's face,
/// counted from the stone's top-left.
fn mark(
    catalog: &crate::catalog::Catalog,
    table: &ArtTable,
    age: MonumentAge,
    part: u8,
    sprite: SpriteId,
    x: i32,
) -> Option<Draw> {
    let stone = catalog.sprite(*table.monument(age).first()?);
    let point = table
        .world()
        .mark_points
        .get(usize::from(part).checked_sub(FIRST_MARK)?)?;
    let left = x - i32::from(stone.anchor.x) + i32::from(point.x);
    let top = PLANT_GROUND_Y - i32::from(stone.anchor.y) + i32::from(point.y);
    Some(Draw::new(catalog, sprite, left, top))
}

/// The centre of a thriving plant's foliage, standing at `x`: where its night glow sits.
fn foliage_centre(
    art: &Art,
    table: &ArtTable,
    species: PlantSpecies,
    stage: GrowthStage,
    x: i32,
) -> (i32, i32) {
    let foliage = match table.plant(species, stage, PlantState::Thriving) {
        PlantArt::Parts { foliage, .. } => foliage,
        PlantArt::Seedling(combined) => combined,
    };
    let sprite = art.catalog.sprite(foliage);
    let left = x - i32::from(sprite.anchor.x);
    let top = plant::ground_y(species, stage) - i32::from(sprite.anchor.y);
    (
        left + i32::from(sprite.rect.w) / 2,
        top + i32::from(sprite.rect.h) / 2,
    )
}

/// How brightly a look is drawn: its own opacity, and the time of day for art that shows only
/// by day or only by night, or that has an opacity of its own by night.
#[derive(Clone, Copy, Debug)]
struct Lit {
    value: u8,
    palette_mix: u8,
}

impl Lit {
    /// The draw at the look's opacity, scaled down by day for night-only art and by night for
    /// day-only art, and eased to its night opacity with the palette; `None` once nothing of
    /// it shows.
    fn apply(self, art: &Art, draw: Draw) -> Option<Draw> {
        let sprite = art.catalog.sprite(draw.sprite);
        let flags = sprite.flags;
        let own = sprite.night_opacity.map_or(draw.opacity, |night| {
            mix(draw.opacity, night, self.palette_mix)
        });
        let mut opacity = scale(own, self.value);
        if flags.contains(Flags::NIGHT_ONLY) {
            opacity = scale(opacity, self.palette_mix);
        }
        if flags.contains(Flags::DAY_ONLY) {
            opacity = scale(opacity, u8::MAX - self.palette_mix);
        }
        (opacity > 0).then_some(Draw { opacity, ..draw })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use terrarium_truth::scene::Wind;

    use super::*;
    use crate::catalog::Layer;
    use crate::preset;

    fn table() -> ArtTable {
        ArtTable::new(&Art::embedded().catalog).unwrap()
    }

    fn scene(name: &str) -> SceneInput {
        preset::scene(name).unwrap()
    }

    fn overlay(lines: &[&str]) -> OverlayInput {
        OverlayInput::new(
            lines.iter().map(|line| (*line).to_owned()).collect(),
            LocalClock::noon(preset::DATE),
        )
    }

    /// The draws by day that come and go: every one but the soil's, which, like the scenery,
    /// is always there.
    fn visible(composer: &Composer, table: &ArtTable, now: Millis) -> Vec<Draw> {
        composer
            .draws(Art::embedded(), table, 0, now)
            .into_iter()
            .filter(|keyed| keyed.key.subject != Subject::Soil)
            .map(|keyed| keyed.draw)
            .collect()
    }

    fn visible_at(composer: &Composer, table: &ArtTable, mix: u8, now: Millis) -> Vec<Draw> {
        composer
            .draws(Art::embedded(), table, mix, now)
            .into_iter()
            .map(|keyed| keyed.draw)
            .collect()
    }

    fn of(
        composer: &Composer,
        table: &ArtTable,
        mix: u8,
        now: Millis,
        subject: Subject,
    ) -> Vec<KeyedDraw> {
        composer
            .draws(Art::embedded(), table, mix, now)
            .into_iter()
            .filter(|keyed| keyed.key.subject == subject)
            .collect()
    }

    #[test]
    fn the_first_update_fades_everything_in_from_nothing() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        // No label: its fade is the quickest, and would have finished by the middle.
        composer.update(&scene("thriving"), &overlay(&[]), Millis(1000));
        assert!(visible(&composer, &table, Millis(1000)).is_empty());
        // Within the residents' quicker fade too.
        let middle = visible(&composer, &table, Millis(1600));
        let art = Art::embedded();
        assert!(!middle.is_empty());
        assert!(
            middle
                .iter()
                .all(|d| d.opacity < art.catalog.sprite(d.sprite).opacity)
        );
        let settled = composer.settled_at();
        assert_eq!(settled, Millis(1000 + u64::from(SKY_MS)));
        assert!(
            visible(&composer, &table, settled)
                .iter()
                .all(|d| d.opacity == art.catalog.sprite(d.sprite).opacity)
        );
    }

    #[test]
    fn an_update_never_changes_the_frame_at_its_own_moment() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        let mut now = Millis(0);
        for (name, lines, after) in [
            ("thriving", &["a", "b"][..], 2500),
            ("struggling-poor-soil", &["a", "b"], 1700),
            ("dead", &["c"], 3000),
            ("recovering", &[], 1),
            ("struggling-low-light", &["d"], 700),
            ("thriving", &["d"], 9000),
            ("dormant", &["e", "f", "g"], 4100),
            ("mature", &["e"], 20_100),
        ] {
            for mix in [0, 255] {
                let before = visible_at(&composer, &table, mix, now);
                let mut copy = composer.clone();
                copy.update(&scene(name), &overlay(lines), now);
                assert_eq!(
                    visible_at(&copy, &table, mix, now),
                    before,
                    "{name} at {mix}"
                );
            }
            composer.update(&scene(name), &overlay(lines), now);
            now = now.plus(after);
        }
    }

    #[test]
    fn a_restyle_holds_the_old_look_until_the_new_one_has_risen() {
        let mut looks = vec![Look {
            piece: Piece::Tuft,
            opacity: Tween::at(SHOWN),
            behind: false,
        }];
        raise(
            &mut looks,
            Piece::DeadStems,
            Millis(100),
            HALF_RESTYLE,
            HALF_RESTYLE,
        );
        let [old, new] = [looks[0], looks[1]];
        let risen = 100 + u64::from(HALF_RESTYLE);
        assert_eq!(old.opacity.value(Millis(risen)), SHOWN);
        assert_eq!(new.opacity.value(Millis(risen)), SHOWN);
        assert_eq!(
            old.opacity.value(Millis(risen + u64::from(HALF_RESTYLE))),
            0
        );
        // A third look mid-way holds both older ones where they stand.
        raise(
            &mut looks,
            Piece::Tuft,
            Millis(1100),
            HALF_RESTYLE,
            HALF_RESTYLE,
        );
        let held = looks[1].opacity.value(Millis(1100));
        assert!(held > 0 && held < SHOWN);
        assert_eq!(
            looks[1]
                .opacity
                .value(Millis(1100 + u64::from(HALF_RESTYLE))),
            held
        );
        assert_eq!(
            looks[0]
                .opacity
                .value(Millis(1100 + u64::from(HALF_RESTYLE))),
            SHOWN
        );
    }

    /// A change that waits for a wipe to pass reads its looks as they will stand then: one
    /// gone by then stays gone, whatever it shows now.
    #[test]
    fn a_deferred_raise_keeps_a_look_already_falling_by_then() {
        let cloud = Piece::Cloud(CloudSize::Small);
        let covered = Tween::new(SHOWN, 0, Millis(9999), 1);
        let mut looks = vec![Look {
            piece: cloud,
            opacity: covered,
            behind: true,
        }];
        raise(&mut looks, cloud, Millis(10_000), SKY_MS, HALF_RESTYLE);
        // Not raised again from where it stands now: a new look rises from nothing then.
        assert_eq!(looks.len(), 2);
        assert_eq!(looks[0].opacity, covered);
        assert_eq!(looks[1].opacity.value(Millis(10_000)), 0);
        assert_eq!(looks[1].opacity.value(Millis(5000)), 0);
        // An older look gone by then keeps its course too.
        let mut looks = vec![
            Look {
                piece: Piece::Weather(Weather::Clear),
                opacity: covered,
                behind: true,
            },
            Look {
                piece: Piece::Weather(Weather::Storm),
                opacity: Tween::at(SHOWN),
                behind: false,
            },
        ];
        raise(
            &mut looks,
            Piece::Weather(Weather::Haze),
            Millis(10_000),
            HALF_RESTYLE,
            HALF_RESTYLE,
        );
        assert_eq!(looks[0].opacity, covered);
        assert_eq!(looks[1].opacity.value(Millis(9000)), SHOWN);
    }

    #[test]
    fn a_film_crossfades_so_two_levels_never_stack() {
        let film = |level| Piece::Film(GlassFilm::Condensation, level);
        let mut looks = vec![Look {
            piece: film(1),
            opacity: Tween::at(SHOWN),
            behind: false,
        }];
        raise(&mut looks, film(2), Millis(100), HALF_RESTYLE, HALF_RESTYLE);
        let mid = Millis(100 + u64::from(HALF_RESTYLE) / 2);
        let (old, new) = (looks[0].opacity.value(mid), looks[1].opacity.value(mid));
        // Falling while the new one rises: the two sum to about one look, never two.
        assert!(old < SHOWN && new < SHOWN);
        assert!(
            (u16::from(old) + u16::from(new)).abs_diff(255) <= 2,
            "{old} + {new}"
        );
        assert_eq!(looks[0].opacity.value(Millis(100)), SHOWN);
    }

    #[test]
    fn every_piece_restyles_by_its_art() {
        // Art that is not opaque and normal crossfades; the rest covers.
        let table = table();
        let catalog = &Art::embedded().catalog;
        let plant = Subject::Plant { id: 1, cell: 2 };
        let pieces = [
            (Piece::Tuft, Subject::Fill { cell: 0 }),
            (Piece::DressingTuft, Subject::Dressing(0)),
            (Piece::Weed(WeedForm::Patch), Subject::Weed { cell: 3 }),
            (Piece::Litter(LitterSet::Amber), Subject::Litter(0)),
            (Piece::Fungus, Subject::Fungus(0)),
            (Piece::FungusGlow, Subject::Fungus(0)),
            (
                Piece::Stone(MonumentAge::Ancient),
                Subject::Monument { id: 1, spot: 0 },
            ),
            (
                Piece::Mark {
                    age: MonumentAge::Young,
                },
                Subject::Monument { id: 1, spot: 0 },
            ),
            (
                Piece::LichenGlow(MonumentAge::Lichened),
                Subject::Monument { id: 1, spot: 0 },
            ),
            (
                Piece::Fossil(FossilForm::Bone),
                Subject::Fossil { id: 1, spot: 0 },
            ),
            (Piece::Weather(Weather::Overcast), Subject::Weather),
            (Piece::Cloud(CloudSize::Large), Subject::Cloud(0)),
            (Piece::Sun, Subject::Sun),
            (
                Piece::Moon(MoonLook {
                    sprite: 2,
                    mirrored: false,
                }),
                Subject::Moon,
            ),
            (Piece::Star, Subject::Star(0)),
            (Piece::Light(Light::Dusk), Subject::Light),
            (Piece::Light(Light::Sunburst), Subject::Light),
            (
                Piece::Film(GlassFilm::Dust, 2),
                Subject::Film(GlassFilm::Dust),
            ),
            (
                Piece::PlantGlow {
                    species: PlantSpecies::Oak,
                    stage: GrowthStage::Mature,
                },
                plant,
            ),
            (
                Piece::Foliage {
                    species: PlantSpecies::Oak,
                    stage: GrowthStage::Mature,
                    state: PlantState::Pale,
                },
                plant,
            ),
        ];
        for (piece, subject) in pieces {
            let sprite = piece.sprite(&table, preset::SALT, subject, 1).expect("art");
            let art = catalog.sprite(sprite);
            let translucent = art.blend != crate::catalog::Blend::Normal || art.opacity < 255;
            let expected = if translucent {
                Restyle::Crossfade
            } else {
                Restyle::Cover
            };
            assert_eq!(piece.restyle(), expected, "{piece:?}");
        }
    }

    #[test]
    fn a_pale_turn_fades_the_foliage_and_never_the_stem() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        let thriving = scene("mature");
        composer.update(&thriving, &overlay(&[]), Millis(0));
        let settled = composer.settled_at();
        let mut pale = thriving.clone();
        for plant in &mut pale.plants {
            plant.state = PlantState::Pale;
        }
        composer.update(&pale, &overlay(&[]), settled);
        let mid = settled.plus(HALF_RESTYLE / 2);
        let stems: Vec<Depicted> = composer
            .depicted(mid)
            .into_iter()
            .filter(|d| matches!(d.piece, Piece::Stem { .. }))
            .collect();
        assert!(!stems.is_empty());
        assert!(stems.iter().all(|d| d.opacity == SHOWN));
    }

    #[test]
    fn finished_looks_are_pruned_and_live_ones_kept() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        let plants = |composer: &Composer| {
            composer
                .entries
                .keys()
                .filter(|s| matches!(s, Subject::Plant { .. }))
                .count()
        };
        composer.update(&scene("thriving"), &overlay(&["a"]), Millis(0));
        let full = plants(&composer);
        let mut empty = scene("thriving");
        empty.plants.clear();
        composer.update(&empty, &overlay(&["a"]), composer.settled_at());
        // The plants are leaving; none is pruned until it has gone.
        assert_eq!(plants(&composer), full);
        composer.update(&empty, &overlay(&["a"]), composer.settled_at().plus(1));
        assert_eq!(plants(&composer), 0);
        assert!(composer.entries.values().all(Entry::shown));
    }

    #[test]
    fn the_tuft_variant_is_the_salt_s_choice_and_never_changes() {
        let table = table();
        let tufts = table.tufts();
        let chosen: Vec<SpriteId> = (0..30)
            .filter_map(|cell| Piece::Tuft.sprite(&table, preset::SALT, Subject::Fill { cell }, 1))
            .collect();
        assert!(chosen.iter().all(|id| tufts.contains(id)));
        assert!(chosen.iter().any(|&id| id != chosen[0]));
        assert_eq!(
            Some(chosen[4]),
            Piece::Tuft.sprite(&table, preset::SALT, Subject::Fill { cell: 4 }, 1)
        );
    }

    #[test]
    fn night_only_art_is_gone_by_day_and_day_only_art_by_night() {
        let table = table();
        let art = Art::embedded();
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&scene("mature"), &overlay(&["a"]), Millis(0));
        let settled = composer.settled_at();
        let flagged = |mix: u8, flag: Flags| {
            composer
                .draws(art, &table, mix, settled)
                .iter()
                .filter(|keyed| art.catalog.sprite(keyed.draw.sprite).flags.contains(flag))
                .count()
        };
        assert_eq!(flagged(0, Flags::NIGHT_ONLY), 0);
        assert!(flagged(255, Flags::NIGHT_ONLY) > 0);
        assert!(flagged(0, Flags::DAY_ONLY) > 0);
        assert_eq!(flagged(255, Flags::DAY_ONLY), 0);
        // Halfway, a glow shows at half its own opacity.
        let glow = table.world().plant_glow;
        let half = composer.draws(art, &table, 128, settled);
        let glows: Vec<u8> = half
            .iter()
            .filter(|keyed| keyed.draw.sprite == glow)
            .map(|keyed| keyed.draw.opacity)
            .collect();
        assert!(!glows.is_empty());
        assert!(
            glows
                .iter()
                .all(|&o| o == scale(art.catalog.sprite(glow).opacity, 128))
        );
    }

    #[test]
    fn a_plant_glows_only_while_young_or_mature_and_thriving() {
        for &stage in GrowthStage::ALL {
            for &state in PlantState::ALL {
                let glows = stage != GrowthStage::Seedling && state == PlantState::Thriving;
                assert_eq!(shows_glow(stage, state), glows, "{stage:?} {state:?}");
            }
        }
    }

    #[test]
    fn a_whole_weather_look_covers_the_old_one_in_every_column() {
        let table = table();
        for viewport in [Viewport::SCENE, Viewport::new(735)] {
            let mut composer = Composer::new(&table, preset::SALT);
            composer.set_viewport(viewport);
            let mut clear = preset::bare(PresentationState::Thriving, Vec::new());
            composer.update(&clear, &overlay(&[]), Millis(0));
            clear.weather = Weather::Overcast;
            let settled = composer.settled_at();
            composer.update(&clear, &overlay(&[]), settled);
            let mid = settled.plus(HALF_RESTYLE / 2);
            let looks = of(&composer, &table, 0, mid, Subject::Weather);
            // One draw a look, the new one on top, each across every column of the viewport.
            assert_eq!(
                looks.iter().map(|keyed| keyed.key.look).collect::<Vec<_>>(),
                [0, 1]
            );
            let width = i32::try_from(viewport.width()).unwrap();
            for keyed in looks {
                let w = i32::from(Art::embedded().catalog.sprite(keyed.draw.sprite).rect.w);
                assert!(
                    keyed.draw.x <= 0 && keyed.draw.x + w >= width,
                    "{viewport:?}"
                );
            }
        }
    }

    #[test]
    fn marks_paint_over_their_stone_at_the_prose_points() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        let mut input = preset::bare(PresentationState::Mature, Vec::new());
        input.monuments = vec![terrarium_truth::scene::MonumentView {
            id: 7,
            age: MonumentAge::Ancient,
            marks: 3,
        }];
        composer.update(&input, &overlay(&[]), Millis(0));
        let spot = composer.monument_spots()[&7];
        let draws = of(
            &composer,
            &table,
            255,
            composer.settled_at(),
            Subject::Monument { id: 7, spot },
        );
        let parts: Vec<u8> = draws.iter().map(|keyed| keyed.key.part).collect();
        assert_eq!(parts, [0, 1, 2, 3, 4], "stone, three marks, then the glow");
        let stone = draws[0].draw;
        let points = &table.world().mark_points;
        for (keyed, point) in draws[1..4].iter().zip(points) {
            assert_eq!(keyed.draw.layer, Layer::Monuments);
            assert_eq!(
                (keyed.draw.x - stone.x, keyed.draw.y - stone.y),
                (i32::from(point.x), i32::from(point.y))
            );
        }
    }

    #[test]
    fn a_landmark_is_found_where_it_is_drawn_and_nowhere_beside_it() {
        let table = table();
        let art = Art::embedded();
        let mut composer = Composer::new(&table, preset::SALT);
        composer.set_viewport(Viewport::new(700));
        let mut input = preset::bare(PresentationState::Mature, Vec::new());
        input.monuments = vec![terrarium_truth::scene::MonumentView {
            id: 7,
            age: MonumentAge::Young,
            marks: 0,
        }];
        input.fossils = vec![terrarium_truth::scene::FossilView {
            id: 9,
            form: FossilForm::Bone,
        }];
        composer.update(&input, &overlay(&[]), Millis(0));
        let now = composer.settled_at();
        let drawn = |subject: Subject| -> Area {
            let draw = of(&composer, &table, 0, now, subject)[0].draw;
            let rect = art.catalog.sprite(draw.sprite).rect;
            Area {
                x: draw.x,
                y: draw.y,
                w: i32::from(rect.w),
                h: i32::from(rect.h),
            }
        };
        let stone = drawn(Subject::Monument {
            id: 7,
            spot: composer.monument_spots()[&7],
        });
        let fossil = drawn(Subject::Fossil {
            id: 9,
            spot: composer.fossil_spots()[&9],
        });
        let at = |composer: &Composer, x, y| composer.landmark_at(art, &table, x, y, now);
        for (area, landmark) in [
            (stone, Landmark::Monument(7)),
            (fossil, Landmark::Fossil(9)),
        ] {
            let c = &composer;
            assert_eq!(at(c, area.x, area.y), Some(landmark));
            let corner = (area.x + area.w - 1, area.y + area.h - 1);
            assert_eq!(at(c, corner.0, corner.1), Some(landmark));
            assert_ne!(at(c, area.x - 1, area.y), Some(landmark));
            assert_ne!(at(c, area.x + area.w, corner.1), Some(landmark));
        }
        // Leaving, it is no longer pointed at.
        input.monuments.clear();
        composer.update(&input, &overlay(&[]), now);
        assert_eq!(at(&composer, stone.x, stone.y), None);
    }

    #[test]
    fn the_label_cycles_its_lines_and_crossfades_between_them() {
        let look = LabelLook {
            lines: vec!["one".into(), "two".into(), "three".into()],
            since: Millis(1000),
            opacity: Tween::at(SHOWN),
        };
        let cycle = u64::from(LABEL_CYCLE_MS);
        assert_eq!(look.showing(Millis(1000)), [(0, SHOWN)]);
        assert_eq!(look.showing(Millis(1000 + cycle - 1)), [(0, SHOWN)]);
        // The step to the next line: the old one still whole, the new one at nothing.
        assert_eq!(look.showing(Millis(1000 + cycle)), [(0, SHOWN), (1, 0)]);
        let mid = look.showing(Millis(1000 + cycle + u64::from(LABEL_FADE_MS) / 2));
        assert_eq!((mid[0].0, mid[1].0), (0, 1));
        assert_eq!(u16::from(mid[0].1) + u16::from(mid[1].1), 255);
        assert_eq!(
            look.showing(Millis(1000 + cycle + u64::from(LABEL_FADE_MS))),
            [(1, SHOWN)]
        );
        // After the last line, the first again.
        assert_eq!(
            look.showing(Millis(1000 + 3 * cycle + u64::from(LABEL_FADE_MS))),
            [(0, SHOWN)]
        );
        let single = LabelLook {
            lines: vec!["only".into()],
            ..look.clone()
        };
        assert_eq!(single.showing(Millis(1000 + 5 * cycle)), [(0, SHOWN)]);
    }

    #[test]
    fn a_label_line_s_turn_ends_when_the_cycle_moves_on_from_it() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        let input = preset::bare(PresentationState::Thriving, Vec::new());
        let cycle = u64::from(LABEL_CYCLE_MS);
        composer.update(
            &input,
            &overlay(&["moss", "main - clear", "sim updated"]),
            Millis(0),
        );
        assert!(!composer.label_step_done(2, Millis(3 * cycle - 1)));
        assert!(composer.label_step_done(2, Millis(3 * cycle)));
        assert!(composer.label_step_done(0, Millis(cycle)));
        assert!(
            !composer.label_step_done(3, Millis(10 * cycle)),
            "no such line"
        );
        // A new set starts again.
        composer.update(
            &input,
            &overlay(&["moss", "sim updated"]),
            Millis(3 * cycle),
        );
        assert!(!composer.label_step_done(1, Millis(4 * cycle)));
        assert!(composer.label_step_done(1, Millis(5 * cycle)));
        // One line never moves on.
        composer.update(&input, &overlay(&["moss"]), Millis(5 * cycle));
        assert!(!composer.label_step_done(0, Millis(50 * cycle)));
    }

    #[test]
    fn new_label_lines_crossfade_from_the_old_on_the_plate() {
        let table = table();
        let art = Art::embedded();
        let mut composer = Composer::new(&table, preset::SALT);
        let input = preset::bare(PresentationState::Thriving, Vec::new());
        composer.update(&input, &overlay(&["main - clear"]), Millis(0));
        let settled = composer.settled_at();
        let text = |composer: &Composer, now| of(composer, &table, 0, now, Subject::Label);
        let before = text(&composer, settled);
        assert_eq!(before.len(), "main - clear".len());
        let plate = world::label_area(&art.catalog).unwrap();
        for keyed in &before {
            let draw = keyed.draw;
            let rect = art.catalog.sprite(draw.sprite).rect;
            assert!(draw.x >= plate.x && draw.x + i32::from(rect.w) <= plate.x + plate.w);
            assert!(draw.y >= plate.y && draw.y + i32::from(rect.h) <= plate.y + plate.h);
            assert_eq!(draw.layer, Layer::Label);
        }
        composer.update(&input, &overlay(&["feature/a-very-long-branch"]), settled);
        let draws = |keyed: &[KeyedDraw]| keyed.iter().map(|k| k.draw).collect::<Vec<_>>();
        assert_eq!(draws(&text(&composer, settled)), draws(&before));
        let mid = text(&composer, settled.plus(LABEL_FADE_MS / 2));
        assert!(mid.iter().any(|keyed| keyed.key.look == 0));
        assert!(mid.iter().any(|keyed| keyed.key.look == 1));
        let after = text(&composer, settled.plus(LABEL_FADE_MS));
        // Fitted to the plate: 17 characters and a tilde.
        assert_eq!(after.len(), 18);
        assert!(
            after
                .iter()
                .all(|keyed| keyed.key.look == 1 && keyed.draw.opacity == SHOWN)
        );
    }

    #[test]
    fn the_palette_follows_the_clock_and_names_each_of_its_steps() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        let input = preset::bare(PresentationState::Thriving, Vec::new());
        let evening = OverlayInput::new(Vec::new(), LocalClock::new(preset::DATE, 19 * 3_600_000));
        composer.update(&input, &evening, Millis(0));
        assert_eq!(composer.palette_mix(Millis(0)), 0);
        // Carried forward on the composer's clock: night 45 minutes into the evening.
        assert_eq!(composer.palette_mix(Millis(45 * 60_000)), u8::MAX);
        let mut now = Millis(0);
        let mut steps = 0;
        while let Some(next) = composer.next_change(Art::embedded(), &table, now) {
            if next > Millis(46 * 60_000) {
                break;
            }
            // The scene fading in names its own changes too; the palette holds between them.
            let mix = composer.palette_mix(now);
            assert_eq!(composer.palette_mix(Millis(next.0 - 1)), mix, "{next:?}");
            steps += usize::from(composer.palette_mix(next) != mix);
            now = next;
        }
        assert_eq!(steps, 255, "every step of the mix is named");
    }

    #[test]
    fn a_clock_that_disagrees_eases_the_palette_over() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        let input = preset::bare(PresentationState::Thriving, Vec::new());
        let at = |ms| OverlayInput::new(Vec::new(), LocalClock::new(preset::DATE, ms));
        composer.update(&input, &at(12 * 3_600_000), Millis(0));
        // Woken at midnight: the palette eases to night rather than jumping.
        let woke = Millis(5000);
        composer.update(&input, &at(0), woke);
        assert_eq!(composer.palette_mix(woke), 0);
        let mut last = 0;
        for ms in (0..=CLOCK_FIX_MS).step_by(10) {
            let mix = composer.palette_mix(woke.plus(ms));
            assert!(mix >= last && mix - last <= 2, "{ms}: {last} → {mix}");
            last = mix;
        }
        assert_eq!(last, u8::MAX);
        assert_eq!(composer.settled_at(), woke.plus(CLOCK_FIX_MS));
    }

    #[test]
    fn dim_mode_settles_in_slowly_and_lifts_quickly() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        let input = preset::bare(PresentationState::Thriving, Vec::new());
        let noon = OverlayInput::new(Vec::new(), LocalClock::noon(preset::DATE));
        let dimmed = OverlayInput {
            dim: true,
            ..noon.clone()
        };
        composer.update(&input, &dimmed, Millis(0));
        assert_eq!(composer.dim(Millis(0)), 0);
        assert_eq!(composer.dim(Millis(u64::from(DIM_DOWN_MS))), DIM);
        let scenery = composer.scenery(Art::embedded(), &table);
        let frame = composer.frame(Art::embedded(), &table, &scenery, 0, Millis(20_000));
        assert_eq!(frame.dim, DIM);
        composer.update(&input, &noon, Millis(20_000));
        assert_eq!(composer.dim(Millis(20_000)), DIM);
        assert_eq!(composer.dim(Millis(20_000 + u64::from(DIM_UP_MS))), 0);
    }

    /// The plants whose foliage leans at `now`, and every leaning draw's part.
    fn leaning(composer: &Composer, table: &ArtTable, now: Millis) -> (Vec<Subject>, Vec<u8>) {
        let draws = composer.draws(Art::embedded(), table, 0, now);
        let leaning: Vec<&KeyedDraw> = draws
            .iter()
            .filter(|keyed| keyed.draw.shear != Shear::NONE)
            .collect();
        let mut subjects: Vec<Subject> = leaning.iter().map(|keyed| keyed.key.subject).collect();
        subjects.dedup();
        (
            subjects,
            leaning.iter().map(|keyed| keyed.key.part).collect(),
        )
    }

    #[test]
    fn a_gust_turns_ahead_is_named_however_long_the_frame_holds_before_it() {
        use crate::motion::TURN_MS;
        // One swaying fern under a storm: no clouds, sun or residents, and one label line, so
        // after a gust the frame holds for turns, until the plant's next gust.
        let art = Art::embedded();
        let table = table();
        let fern = PlantView {
            id: 1,
            species: PlantSpecies::Fern,
            stage: GrowthStage::Young,
            state: PlantState::Thriving,
            bloom: false,
            scorched: false,
            order: 0,
        };
        let mut scene = preset::bare(PresentationState::Thriving, vec![fern]);
        scene.weather = Weather::Storm;
        scene.wind = Wind::Breezy;
        for dim in [false, true] {
            let overlay = OverlayInput {
                dim,
                ..overlay(&["main"])
            };
            let mut composer = Composer::new(&table, preset::SALT);
            composer.update(&scene, &overlay, Millis(0));
            let scenery = composer.scenery(art, &table);
            let frame = |now: Millis| {
                let draws = composer.frame(art, &table, &scenery, composer.palette_mix(now), now);
                draws.draws
            };
            let mut moment = composer.settled_at();
            let mut longest = 0;
            // Past the longest wait between one cell's gusts, in either mode.
            while moment.0 < turn_start(MOTION_AHEAD + 2).0 {
                let next = composer.next_change(art, &table, moment).unwrap();
                let held = frame(moment);
                // A gust holds each pose at least 300 ms, so 100 ms steps see every one.
                for inside in (moment.0 + 1..next.0).step_by(100) {
                    assert!(
                        frame(Millis(inside)) == held,
                        "changed at {inside} before {next:?}"
                    );
                }
                longest = longest.max(next.0 - moment.0);
                moment = next;
            }
            assert!(
                longest > 2 * TURN_MS,
                "the frame never held for long: {longest} ms"
            );
        }
    }

    #[test]
    fn wind_sways_at_most_six_plants_their_foliage_and_blooms_only() {
        let table = table();
        let mut thriving = scene("thriving");
        for (wind, most) in [(Wind::Breezy, 1), (Wind::Gale, 2)] {
            thriving.wind = wind;
            let mut composer = Composer::new(&table, preset::SALT);
            composer.update(&thriving, &overlay(&[]), Millis(0));
            assert!(
                leaning(&composer, &table, Millis(20_000)).0.is_empty(),
                "the first turn is still"
            );
            let mut seen = BTreeSet::new();
            // Every cell gusts once in five turns.
            for ms in (24_000..144_000).step_by(100) {
                let (subjects, parts) = leaning(&composer, &table, Millis(ms));
                assert!(subjects.len() <= sway::MOST_SWAYING);
                assert!(
                    parts.iter().all(|&part| part == 1 || part == 2),
                    "{parts:?}"
                );
                for keyed in composer.draws(Art::embedded(), &table, 0, Millis(ms)) {
                    let top = keyed.draw.shear.offset(0);
                    if keyed.key.part == 1 {
                        assert!(top.abs() <= most, "{top}");
                    }
                }
                seen.extend(subjects);
            }
            let living: BTreeSet<Subject> = composer
                .placement()
                .plants
                .iter()
                .filter(|(_, placed)| sways(placed.view.stage, placed.view.state))
                .map(|(&id, placed)| Subject::Plant {
                    id,
                    cell: placed.cell,
                })
                .collect();
            assert!(living.len() > sway::MOST_SWAYING);
            assert_eq!(seen, living, "every living plant takes its turn");
        }
        thriving.wind = Wind::Calm;
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&thriving, &overlay(&[]), Millis(0));
        for ms in (24_000..72_000).step_by(100) {
            assert!(leaning(&composer, &table, Millis(ms)).0.is_empty());
        }
    }

    #[test]
    fn a_wind_change_waits_for_the_next_turn_and_dim_quarters_the_gusts() {
        let table = table();
        let mut scene = scene("thriving");
        scene.wind = Wind::Gale;
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&scene, &overlay(&[]), Millis(0));
        // Calm arrives mid-turn: the gusts of the running turn play out.
        let moving =
            (24_000..48_000).find(|&ms| !leaning(&composer, &table, Millis(ms)).0.is_empty());
        let moving = Millis(moving.expect("a gust in the first turn that moves"));
        let calm = SceneInput {
            wind: Wind::Calm,
            ..scene.clone()
        };
        composer.update(&calm, &overlay(&[]), moving);
        assert!(!leaning(&composer, &table, moving).0.is_empty());
        assert!((48_000..96_000).all(|ms| leaning(&composer, &table, Millis(ms)).0.is_empty()));
        // Dimmed, only every fourth turn gusts.
        let mut composer = Composer::new(&table, preset::SALT);
        let dimmed = OverlayInput {
            dim: true,
            ..overlay(&[])
        };
        composer.update(&scene, &dimmed, Millis(0));
        for w in 1..9_u64 {
            let gusting = (0..24_000).step_by(100).any(|ms| {
                !leaning(&composer, &table, Millis(w * 24_000 + ms))
                    .0
                    .is_empty()
            });
            assert_eq!(gusting, w % 4 == 0, "turn {w}");
        }
    }

    use terrarium_truth::scene::Energy;

    fn resident(id: u32, species: ResidentSpecies, stage: LifeStage) -> ResidentView {
        ResidentView {
            id,
            species,
            stage,
            energy: Energy::High,
        }
    }

    /// What a resident's draw shows: its art's life stage and routine, and where it stands.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Seen {
        stage: String,
        routine: String,
        x: i32,
    }

    impl Seen {
        fn name(&self) -> String {
            format!("{}.{}", self.stage, self.routine)
        }
    }

    /// Each resident's draws at `now`, by id, in draw order: the top look last.
    fn creatures(
        composer: &Composer,
        table: &ArtTable,
        mix: u8,
        now: Millis,
    ) -> BTreeMap<u32, Vec<Seen>> {
        let catalog = &Art::embedded().catalog;
        let mut creatures: BTreeMap<u32, Vec<Seen>> = BTreeMap::new();
        for keyed in composer.draws(Art::embedded(), table, mix, now) {
            if let Subject::Resident { id, .. } = keyed.key.subject
                && keyed.key.part == 0
            {
                let keys = &catalog.sprite(keyed.draw.sprite).keys;
                creatures.entry(id).or_default().push(Seen {
                    stage: keys.life_stage.clone().unwrap_or_default(),
                    routine: keys.state.clone().unwrap_or_default(),
                    x: keyed.key.x,
                });
            }
        }
        creatures
    }

    /// The residents walking, flying, hopping or signalling at `now`, and of those, in the air
    /// or signalling.
    fn moving(composer: &Composer, table: &ArtTable, mix: u8, now: Millis) -> (usize, usize) {
        let creatures = creatures(composer, table, mix, now);
        let doing = |names: &[&str]| {
            creatures
                .values()
                .filter(|looks| {
                    looks
                        .iter()
                        .any(|look| names.contains(&look.routine.as_str()))
                })
                .count()
        };
        (
            doing(&["move", "takeoff", "flight", "land", "hop", "signal"]),
            doing(&["takeoff", "flight", "land", "hop", "signal"]),
        )
    }

    /// The top look of each resident at every step of `range`, each change of it once.
    fn story(
        composer: &Composer,
        table: &ArtTable,
        mix: u8,
        range: std::ops::Range<u64>,
    ) -> BTreeMap<u32, Vec<String>> {
        let mut seen: BTreeMap<u32, Vec<String>> = BTreeMap::new();
        for ms in range.step_by(50) {
            for (id, looks) in creatures(composer, table, mix, Millis(ms)) {
                let Some(top) = looks.last().map(Seen::name) else {
                    continue;
                };
                let names = seen.entry(id).or_default();
                if names.last() != Some(&top) {
                    names.push(top);
                }
            }
        }
        seen
    }

    fn six() -> Vec<ResidentView> {
        use ResidentSpecies::{Beetle, Firefly, Harvestman, Ladybird, Snail, Springtail};
        [Snail, Ladybird, Beetle, Firefly, Springtail, Harvestman]
            .into_iter()
            .zip(1..)
            .map(|(species, id)| resident(id, species, LifeStage::Adult))
            .collect()
    }

    #[test]
    fn at_most_three_residents_move_and_one_flies_and_each_rests_at_home() {
        let table = table();
        for (clock, mix) in [
            (LocalClock::noon(preset::DATE), 0),
            (LocalClock::midnight(preset::DATE), 255),
        ] {
            let mut scene = scene("thriving");
            scene.residents = six();
            let mut composer = Composer::new(&table, preset::SALT);
            composer.update(&scene, &OverlayInput::new(Vec::new(), clock), Millis(0));
            let (mut most, mut flew, mut walked) = (0, false, BTreeSet::new());
            for ms in (24_000..240_000).step_by(83) {
                let (moving, flying) = moving(&composer, &table, mix, Millis(ms));
                assert!(moving <= resident::MOST_MOVING, "{moving} moving at {ms}");
                assert!(flying <= resident::MOST_FLYING, "{flying} flying at {ms}");
                most = most.max(moving);
                flew |= flying > 0;
                for (id, looks) in creatures(&composer, &table, mix, Millis(ms)) {
                    if looks.iter().any(|look| look.routine == "move") {
                        walked.insert(id);
                    }
                }
            }
            assert_eq!(most, resident::MOST_MOVING, "three move together at times");
            assert!(walked.len() >= 3, "{walked:?}");
            // By night the firefly takes to the air or signals; by day nothing flies but a
            // springtail's hop.
            if mix == 255 {
                assert!(flew);
            }
            // At every turn's start each resident stands at home.
            let homes = creatures(&composer, &table, mix, Millis(24_000));
            for w in 2..10 {
                let at = creatures(&composer, &table, mix, Millis(w * 24_000));
                for (id, looks) in &at {
                    assert_eq!(
                        looks.last().map(|look| look.x),
                        homes[id].last().map(|look| look.x),
                        "{id} in turn {w}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_change_of_energy_alone_settles_with_the_tweens() {
        let table = table();
        let mut scene = scene("thriving");
        scene.residents = six();
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&scene, &overlay(&[]), Millis(0));
        let settled = composer.settled_at();
        for resident in &mut scene.residents {
            resident.energy = Energy::Low;
        }
        composer.update(&scene, &overlay(&[]), Millis(30_000));
        assert_eq!(composer.settled_at(), settled, "nothing new to settle");
        // A step of stage plays at the next turn, and settling waits for it.
        scene.residents[0].stage = LifeStage::Dying;
        composer.update(&scene, &overlay(&[]), Millis(31_000));
        assert!(composer.settled_at() > Millis(48_000));
    }

    #[test]
    fn a_newcomer_is_drawn_though_one_leaving_still_fades() {
        let table = table();
        let mut scene = scene("thriving");
        scene.residents = six();
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&scene, &overlay(&[]), Millis(0));
        // One leaves as another arrives: six in the scene, a seventh still fading.
        scene.residents.pop();
        scene
            .residents
            .push(resident(9, ResidentSpecies::Woodlouse, LifeStage::Adult));
        composer.update(&scene, &overlay(&[]), Millis(30_000));
        let shown: BTreeSet<u32> = composer
            .depicted(Millis(30_600))
            .iter()
            .filter_map(|d| match d.subject {
                Subject::Resident { id, .. } => Some(id),
                _ => None,
            })
            .collect();
        assert!(shown.contains(&9), "the newcomer fades in: {shown:?}");
        assert!(shown.contains(&6), "the one leaving fades out");
    }

    #[test]
    fn of_many_fireflies_and_springtails_one_at_most_takes_to_the_air() {
        let table = table();
        let mut scene = scene("thriving");
        scene.residents = [
            ResidentSpecies::Firefly,
            ResidentSpecies::Firefly,
            ResidentSpecies::Springtail,
            ResidentSpecies::Firefly,
        ]
        .into_iter()
        .zip(1..)
        .map(|(species, id)| resident(id, species, LifeStage::Adult))
        .collect();
        let night = OverlayInput::new(Vec::new(), LocalClock::midnight(preset::DATE));
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&scene, &night, Millis(0));
        let mut flew = 0;
        for ms in (24_000..480_000).step_by(83) {
            let (_, flying) = moving(&composer, &table, 255, Millis(ms));
            assert!(
                flying <= resident::MOST_FLYING,
                "{flying} in the air at {ms}"
            );
            flew += flying;
        }
        assert!(flew > 0);
    }

    #[test]
    fn a_resident_hatches_ecloses_and_dies_through_its_own_art() {
        let table = table();
        let mut scene = scene("thriving");
        scene.residents = vec![
            resident(1, ResidentSpecies::Snail, LifeStage::Egg),
            resident(2, ResidentSpecies::Ladybird, LifeStage::Pupa),
            resident(3, ResidentSpecies::Beetle, LifeStage::Adult),
        ];
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&scene, &overlay(&[]), Millis(0));
        let at = |composer: &Composer, ms| creatures(composer, &table, 0, Millis(ms));
        assert_eq!(at(&composer, 30_000)[&1][0].name(), "egg.egg");
        assert_eq!(at(&composer, 30_000)[&2][0].name(), "pupa.pupa");
        // The next update steps each one on; the step plays at the next turn.
        scene.residents = vec![
            resident(1, ResidentSpecies::Snail, LifeStage::Juvenile),
            resident(2, ResidentSpecies::Ladybird, LifeStage::Adult),
            resident(3, ResidentSpecies::Beetle, LifeStage::Dying),
        ];
        composer.update(&scene, &overlay(&[]), Millis(30_000));
        assert_eq!(
            at(&composer, 47_000)[&1][0].name(),
            "egg.egg",
            "the running turn holds"
        );
        // The snail feeds at a plant's foot: hatching hands it to its idle, and the feed rises
        // over that.
        let seen = story(&composer, &table, 0, 48_000..72_000);
        assert_eq!(
            seen[&1],
            ["egg.egg", "egg.hatch", "juvenile.idle", "juvenile.feed"].map(String::from)
        );
        assert_eq!(
            seen[&2][..2],
            ["pupa.pupa", "pupa.eclose"].map(String::from)
        );
        assert!(seen[&2][2].starts_with("adult."), "{:?}", seen[&2]);
        assert_eq!(seen[&3].last().map(String::as_str), Some("adult.death"));
        let at_hatch = (48_000..72_000)
            .step_by(50)
            .flat_map(|ms| at(&composer, ms).remove(&1).unwrap_or_default())
            .map(|look| look.stage);
        assert!(
            at_hatch.into_iter().all(|stage| stage != "adult"),
            "juveniles never use adult art"
        );
        // Death holds its last frame until truth removes the resident.
        let held = at(&composer, 200_000)[&3].clone();
        assert_eq!(held, at(&composer, 300_000)[&3]);
        assert!(
            composer.settled_at() >= Millis(48_000),
            "the steps are part of settling"
        );
    }

    #[test]
    fn a_storm_sends_adults_into_hiding_and_juveniles_still() {
        let table = table();
        let mut scene = scene("thriving");
        scene.residents = vec![
            resident(1, ResidentSpecies::Snail, LifeStage::Adult),
            resident(2, ResidentSpecies::Woodlouse, LifeStage::Adult),
            resident(3, ResidentSpecies::Ladybird, LifeStage::Adult),
            resident(4, ResidentSpecies::Beetle, LifeStage::Juvenile),
        ];
        let night = OverlayInput::new(Vec::new(), LocalClock::midnight(preset::DATE));
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&scene, &night, Millis(0));
        scene.weather = Weather::Storm;
        composer.update(&scene, &night, Millis(30_000));
        let seen = story(&composer, &table, 255, 48_000..96_000);
        // The snail feeds at a plant's foot, and draws in from there.
        assert_eq!(
            seen[&1][1..],
            ["adult.withdraw", "adult.hide"].map(String::from)
        );
        assert_eq!(
            seen[&2][1..],
            ["adult.curl", "adult.hide"].map(String::from)
        );
        // The ladybird is about at midnight too: it stops feeding and hides.
        assert_eq!(seen[&3], ["adult.feed", "adult.hide"].map(String::from));
        // The young beetle stops feeding and shelters on its idle's first frame, never moving.
        assert_eq!(seen[&4].last().map(String::as_str), Some("juvenile.idle"));
        assert!(!seen[&4].contains(&"juvenile.move".to_owned()));
        let still = creatures(&composer, &table, 255, Millis(80_000))[&4].clone();
        assert_eq!(still, creatures(&composer, &table, 255, Millis(90_000))[&4]);
    }

    #[test]
    fn a_firefly_asleep_by_day_covers_its_sleep_with_its_hide_in_a_storm() {
        let table = table();
        let mut scene = scene("thriving");
        scene.residents = vec![resident(5, ResidentSpecies::Firefly, LifeStage::Adult)];
        let noon = OverlayInput::new(Vec::new(), LocalClock::noon(preset::DATE));
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&scene, &noon, Millis(0));
        scene.weather = Weather::Storm;
        composer.update(&scene, &noon, Millis(30_000));
        let seen = story(&composer, &table, 0, 48_000..96_000);
        assert_eq!(seen[&5], ["adult.sleep", "adult.hide"].map(String::from));
    }

    #[test]
    fn a_firefly_signals_by_night_with_its_lantern_and_never_by_day() {
        let table = table();
        let catalog = &Art::embedded().catalog;
        let mut scene = scene("thriving");
        scene.residents = vec![resident(7, ResidentSpecies::Firefly, LifeStage::Adult)];
        let lanterns = |clock: LocalClock, mix: u8| {
            let mut composer = Composer::new(&table, preset::SALT);
            composer.update(&scene, &OverlayInput::new(Vec::new(), clock), Millis(0));
            (24_000..600_000)
                .step_by(100)
                .filter(|&ms| {
                    composer
                        .draws(Art::embedded(), &table, mix, Millis(ms))
                        .iter()
                        .any(|keyed| {
                            keyed.key.part == 1
                                && catalog.sprite(keyed.draw.sprite).keys.state.as_deref()
                                    == Some("signal-glow")
                        })
                })
                .count()
        };
        assert!(lanterns(LocalClock::midnight(preset::DATE), 255) > 0);
        assert_eq!(lanterns(LocalClock::noon(preset::DATE), 255), 0);
    }

    #[test]
    fn the_moon_shows_its_phase_and_new_moon_shows_none() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        let input = preset::bare(PresentationState::Thriving, Vec::new());
        let at =
            |composer: &Composer| of(composer, &table, 255, composer.settled_at(), Subject::Moon);
        let date = |date| OverlayInput::new(Vec::new(), LocalClock::noon(date));
        composer.update(&input, &date(sky::LocalDate::new(2026, 3, 19)), Millis(0));
        assert!(at(&composer).is_empty());
        composer.update(
            &input,
            &date(sky::LocalDate::new(2026, 3, 15)),
            composer.settled_at(),
        );
        let waning = at(&composer);
        assert_eq!(waning.len(), 1);
        assert!(waning[0].draw.flip_x);
        assert_eq!(
            Some(waning[0].draw.sprite),
            Art::embedded().catalog.id("tmp.sky.moon.phase.3")
        );
    }
}
