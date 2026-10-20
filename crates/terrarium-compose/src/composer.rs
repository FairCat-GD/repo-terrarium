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
    PlantState, PlantView, ResidentSpecies, SceneInput, Weather,
};

use crate::blend::{mix, scale};
use crate::catalog::{Art, Flags, SpriteId};
use crate::clock::{DayNight, LocalClock};
use crate::drift::{self, SOIL_MS_PER_PX, SOIL_PERIOD};
use crate::frame::{
    Area, Draw, Frame, GROUND_Y, NATIVE_HEIGHT, NATIVE_WIDTH, PLANT_GROUND_Y, Shear, Viewport,
};
use crate::motion::{Clock, DIM_EVERY, Eras, Snapshot, motion_turn, turn, turn_start};
use crate::order::{KeyedDraw, Place, Subject, merged};
use crate::panel::{Hover, Landmark};
use crate::place::{Bands, FillForm, Placement, WeedForm, assign_spots, place};
use crate::plant::{self, shows_bloom, shows_glow, sways};
use crate::resident::{self, Resident};
use crate::rules::dressing;
use crate::salt::{InstallSalt, Purpose};
use crate::sky::{self, CloudSize, MoonLook, moon_phase};
use crate::slots::Layout;
use crate::sway;
use crate::tween::durations::{
    APPEAR_MS, DIM_DOWN_MS, DIM_UP_MS, FILL_MS, RESTYLE_MS, SKY_MS, VANISH_MS,
};
use crate::tween::{Millis, Tween};
use crate::variants::{ArtTable, GlassFilm, GroundEventArt, PlantArt, SkyEventArt};
use crate::world::{self, Kind, Scenery, WorldLayout};

mod event;
mod ground;
mod hover;
mod label;
mod residents;
mod time;

use label::LabelLook;
use residents::{Shown, plan_next};
use time::{MixFix, SceneClock};

const SHOWN: u8 = u8::MAX;
const HALF_RESTYLE: u32 = RESTYLE_MS / 2;
/// How many turns past the running one the next motion may lie: a cell gusts once in every
/// [`sway::GUST_EVERY`] motion turns, and in dim mode one turn in [`DIM_EVERY`] moves. Once a
/// turn's gust is over, the plant's next is that many turns on.
const MOTION_AHEAD: u64 = sway::GUST_EVERY * DIM_EVERY;
/// How many turns past the running one a resident's next move is looked for: five motion
/// turns, dim mode's four turns to each. Building a turn's plans is the costly part of the
/// search, so the gusts' longer horizon is searched without them.
const RESIDENTS_AHEAD: u64 = 5 * DIM_EVERY;
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
    /// Whether the scene stands still: from the update that pauses it until the one that does
    /// not, nothing in it moves, while the label, dim mode and the hover layer go on.
    pub paused: bool,
    /// Whether this is a time-lapse's composer: its scene runs [`TIME_LAPSE_PACE`] times faster
    /// than the clock it is given, so its events play in miniature, and its label crossfades as
    /// much faster.
    pub time_lapse: bool,
    /// The soil panel and a hover label, as the pointer has them.
    pub hover: Hover,
}

impl OverlayInput {
    /// The label's lines at a local time, with the default day and night, no dimming, and the
    /// scene running at the composer's pace.
    pub fn new(label: Vec<String>, clock: LocalClock) -> OverlayInput {
        OverlayInput {
            label,
            clock,
            day_night: DayNight::default(),
            dim: false,
            paused: false,
            time_lapse: false,
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
    /// The columns it composes for: the ground spreads across them, and the sky's bodies stand
    /// in their middle.
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
    /// Where the scene's clock stands against the composer's.
    pause: SceneClock,
}

/// How many times faster than the composer's clock the scene's runs in a time-lapse: every
/// tween, event, motion turn and drift.
pub const TIME_LAPSE_PACE: u32 = 8;

impl Composer {
    /// An empty terrarium: the first update fades everything in.
    pub fn new(table: &ArtTable, salt: InstallSalt) -> Composer {
        Composer {
            salt,
            bands: Bands::new(table),
            layout: Layout::new(salt, Viewport::SCENE),
            world: WorldLayout::new(salt, table.world(), Viewport::SCENE),
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
            pause: SceneClock::default(),
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
        Scenery::new(art, table, &self.world)
    }

    /// Moves the depiction toward `input` and `overlay` from `now`. The frame at `now` does
    /// not change. While the overlay pauses the scene, its changes wait for the resume.
    pub fn update(&mut self, input: &SceneInput, overlay: &OverlayInput, now: Millis) {
        self.pause.set(overlay.paused, overlay.time_lapse, now);
        let real = now;
        let now = self.pause.scene(real);
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
        self.update_label(&overlay.label, overlay.time_lapse, real);
        self.hover.set(&overlay.hover, real);
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
            self.dim = Tween::new(self.dim.value(real), dim, real, ms);
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
            after_rain: input.event.as_ref().is_some_and(|event| {
                matches!(
                    event.kind,
                    EventKind::RainAfterDrought | EventKind::Clearing
                )
            }),
        }
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
        self.landmarks(art, table, self.pause.scene(now))
            .into_iter()
            .rev()
            .find(|placed| placed.standing && crate::panel::contains(placed.area, x, y))
            .map(|placed| placed.landmark)
    }

    /// Each look of each monument and fossil at the scene's moment `now`: its spot, where its
    /// sprite lies in the viewport, where a hover label over it stands, and how visible it is. A label is drawn
    /// over each look and fades with it, so when a stone changes age its label crossfades with
    /// it rather than moving.
    fn landmarks(&self, art: &Art, table: &ArtTable, now: Millis) -> Vec<hover::Placed> {
        let catalog = &art.catalog;
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
                    x: draw.x,
                    y: draw.y,
                    w: i32::from(rect.w),
                    h: i32::from(rect.h),
                };
                let (centre, top) = match landmark {
                    Landmark::Monument(_) => (self.world.monument(spot), area.y),
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

    /// The first moment after `now` at which the frame may change, or `None` if nothing will
    /// until the next update. Every source of change answers here; the frame is the same at
    /// every moment before it (I-10). A paused scene changes nothing, so then only the label,
    /// dim mode and the hover layer answer.
    pub fn next_change(&self, art: &Art, table: &ArtTable, now: Millis) -> Option<Millis> {
        let mut next = Earliest::default();
        for look in &self.label {
            next.take(look.opacity.next_step(now));
            if look.opacity.value(now) > 0 || look.opacity.target() == SHOWN {
                next.take(look.next_change(now));
            }
        }
        next.take(self.dim.next_step(now));
        next.take(self.hover.next_change(art, self.viewport_width(), now));
        let scene = self.scene_change(art, table, self.pause.scene(now));
        next.take(scene.and_then(|scene| self.pause.real(scene)));
        next.0
    }

    /// The scene's first moment after its `now` at which it may change.
    fn scene_change(&self, art: &Art, table: &ArtTable, now: Millis) -> Option<Millis> {
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
        if let Some(fix) = self.mix_fix {
            next.take(fix.weight.next_step(now));
        }
        if let Some(ms) = self.ms_of_day(now) {
            next.take(self.day_night.next_change(ms).map(|ahead| now.plus(ahead)));
        }
        for shown in self.residents.values() {
            next.take(shown.opacity.next_step(now));
        }
        next.take(self.motion_change(art, table, now));
        next.take(self.show_next(art, table, now));
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
            if w <= running + RESIDENTS_AHEAD && !self.residents.is_empty() {
                let end = turn_start(w + 1);
                // A resident that has faded out is drawn no more, though it stays until the
                // next update prunes it, so its routine's frames change nothing.
                let shown_at = |key: &(u32, u8), moment: Millis| {
                    self.residents.get(key).is_some_and(|shown| {
                        shown.opacity.target() == SHOWN || moment < shown.opacity.end()
                    })
                };
                for (key, (plan, _)) in &self.resident_plans(art, table, w) {
                    next.take(
                        plan_next(&art.catalog, table, plan, now, start)
                            .filter(|&moment| moment < end && shown_at(key, moment)),
                    );
                }
                // A turn to come begins with each resident where its plan puts it, which need
                // not be where the turn before left it: another place, facing or routine.
                if start > now && self.residents_change_at(art, table, start) {
                    next.take(Some(start));
                }
            }
            if next.0.is_some() {
                return next.0;
            }
        }
        None
    }

    /// When the last tween ends, and the changes of rest an update brought are over. From
    /// then on only motion moves until the next update: drift, the label's cycle, sway and the
    /// residents' turns. While the scene is paused, its own changes wait for the resume, so
    /// only the label's, dim mode's and the hover layer's count.
    pub fn settled_at(&self) -> Millis {
        let overlay = self
            .label
            .iter()
            .map(|look| look.opacity)
            .chain([self.dim])
            .map(Tween::end)
            .chain([self.hover.end()])
            .max()
            .unwrap_or_default();
        let scene = self
            .entries
            .values()
            .flat_map(Entry::looks)
            .map(|look| look.opacity)
            .chain(self.mix_fix.map(|fix| fix.weight))
            .chain(self.residents.values().map(|shown| shown.opacity))
            .map(Tween::end)
            .chain([self.residents_settle])
            .chain(self.show_end())
            .max()
            .unwrap_or_default();
        self.pause
            .real(scene)
            .map_or(overlay, |scene| scene.max(overlay))
    }

    /// Every visible look at `now`, as depicted states in subject order. The label's text,
    /// which reads no salt, is not among them.
    pub fn depicted(&self, art: &Art, table: &ArtTable, now: Millis) -> Vec<Depicted> {
        let now = self.pause.scene(now);
        let mut depicted = Vec::new();
        for ((id, spot), _, record, opacity) in self.drawn_residents(art, table, now) {
            depicted.push(Depicted {
                subject: Subject::Resident { id, spot },
                part: 0,
                piece: Piece::Resident {
                    species: record.view.species,
                    stage: record.view.stage,
                },
                opacity,
            });
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
        let real = now;
        let now = self.pause.scene(real);
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
        self.push_label(art, real, &mut keyed);
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
            .push(art, &landmarks, self.viewport_width(), real, &mut keyed);
        // The sun, the moon and the stars stand in the middle of the viewport; everything else
        // is laid out across the viewport already.
        let inset = self.viewport.inset();
        for keyed in keyed.iter_mut().filter(|keyed| keyed.key.subject.centred()) {
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

    /// Composes for `viewport` from now on: the ground's places spread to its width, and its
    /// scenery is [`Composer::scenery`]'s anew.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
        self.layout = Layout::new(self.salt, viewport);
        self.world = self.world.in_viewport(viewport);
        self.refit_show();
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    fn viewport_width(&self) -> i32 {
        i32::try_from(self.viewport.width()).unwrap_or(FRAME_WIDTH)
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
mod tests;
