//! The composer: from the `SceneInput`, the `OverlayInput`, the install salt and the clock to the
//! draws of a frame. It depicts the states truth gives it and never judges health
//! itself.
//!
//! Each update places the plants, weeds and filler ([`crate::place`]) and gives monuments and
//! fossils their spots, then moves every subject's looks toward what the scene holds. A subject
//! is anything drawn that can change: a plant, a filler, the weeds in a cell, a monument, a
//! fossil, a piece of litter or fungus, a decorative tuft, the weather band, a cloud, the sun,
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

use std::collections::BTreeMap;

use terrarium_truth::PresentationState;
use terrarium_truth::scene::{
    FossilForm, GrowthStage, Light, MonumentAge, PlantSpecies, PlantState, PlantView, SceneInput,
    Weather,
};

use crate::catalog::{Art, Flags, SpriteId};
use crate::frame::{Draw, Frame, NATIVE_HEIGHT, NATIVE_WIDTH, PLANT_GROUND_Y};
use crate::order::{KeyedDraw, Place, Subject, merged};
use crate::place::{Bands, FillForm, Placement, WeedForm, assign_spots, place};
use crate::plant::shows_bloom;
use crate::rules::dressing;
use crate::salt::{InstallSalt, Purpose};
use crate::sky::{self, CloudSize, LocalDate, MoonLook, moon_phase};
use crate::slots::Layout;
use crate::text::{Label, fit};
use crate::tween::durations::{
    APPEAR_MS, FILL_MS, LABEL_CYCLE_MS, LABEL_FADE_MS, RESTYLE_MS, SKY_MS, VANISH_MS,
};
use crate::tween::{Millis, Tween};
use crate::variants::{ArtTable, GlassFilm, PlantArt};
use crate::world::{self, Kind, LABEL_X, LABEL_Y, Scenery, WorldLayout};

const SHOWN: u8 = u8::MAX;
const HALF_RESTYLE: u32 = RESTYLE_MS / 2;
const FRAME_WIDTH: i32 = NATIVE_WIDTH.cast_signed();
const FRAME_HEIGHT: i32 = NATIVE_HEIGHT.cast_signed();

/// What the composer is given beside the scene: not truth, but the app's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayInput {
    /// The lines the label cycles through, already worded by the app's copy module (I-8). The
    /// composer only fits them to the plate.
    pub label: Vec<String>,
    /// The local date, for the moon.
    pub date: LocalDate,
}

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
    Weather(Weather),
    Cloud(CloudSize),
    Sun,
    Moon(MoonLook),
    Star,
    Light(Light),
    /// A film at a level, 1 to `MAX_GLASS_LEVEL`.
    Film(GlassFilm, u8),
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
        }
    }
}

/// The number a subject's salted choices are drawn for: its cell, its place or its id.
fn item(subject: Subject) -> u64 {
    match subject {
        Subject::Weed { cell } | Subject::Fill { cell } | Subject::Plant { cell, .. } => {
            u64::from(cell)
        }
        Subject::Cloud(i)
        | Subject::Star(i)
        | Subject::Dressing(i)
        | Subject::Litter(i)
        | Subject::Fungus(i) => u64::from(i),
        Subject::Monument { id, .. } | Subject::Fossil { id, .. } => u64::from(id),
        Subject::Scenery(i) => u64::from(i),
        Subject::Film(_)
        | Subject::Weather
        | Subject::Sun
        | Subject::Moon
        | Subject::Light
        | Subject::Label => 0,
    }
}

/// The parts of a subject, bottom to top.
const PARTS: usize = 5;
const STEM: usize = 0;
const FOLIAGE: usize = 1;
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

/// Whether a plant glows at night: only young and mature thriving plants. A glow is a sign of
/// health, so nothing else ever shows one (I-7).
pub fn shows_glow(growth: GrowthStage, health: PlantState) -> bool {
    growth != GrowthStage::Seedling && health == PlantState::Thriving
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

    /// Moves each part toward its piece, rising over `rise`; a part with none falls over
    /// `change` once the others have risen.
    fn show(&mut self, pieces: Pieces, now: Millis, rise: u32, change: u32) {
        for (looks, piece) in self.parts.iter_mut().zip(pieces) {
            match piece {
                Some(piece) => raise(looks, piece, now, rise, change),
                None => lower(looks, now, now.plus(rise), change),
            }
        }
    }

    fn hide(&mut self, now: Millis, ms: u32) {
        for looks in &mut self.parts {
            lower(looks, now, now, ms);
        }
    }

    /// Drops every look that has finished falling. Each was invisible, so no frame changes.
    fn prune(&mut self, now: Millis) {
        for looks in &mut self.parts {
            looks.retain(|look| look.opacity.target() == SHOWN || now < look.opacity.end());
        }
    }
}

/// Brings `piece` to the top of a part over `rise`. The older looks then go by the piece's
/// rule: under [`Restyle::Cover`] they hold until it has risen and fall over `change` after,
/// so nothing beneath ever shows through; under [`Restyle::Crossfade`] they fall while it
/// rises.
fn raise(looks: &mut Vec<Look>, piece: Piece, now: Millis, rise: u32, change: u32) {
    match looks.last_mut() {
        Some(top) if top.piece == piece => {
            if top.opacity.target() == SHOWN {
                return;
            }
            top.opacity = Tween::new(top.opacity.value(now), SHOWN, now, rise);
        }
        _ => looks.push(Look {
            piece,
            opacity: Tween::new(0, SHOWN, now, rise),
        }),
    }
    let older = looks.len() - 1;
    for look in &mut looks[..older] {
        let from = look.opacity.value(now);
        look.opacity = match piece.restyle() {
            Restyle::Cover => Tween::new(from, 0, now.plus(rise), change),
            Restyle::Crossfade => Tween::new(from, 0, now, rise),
        };
    }
}

/// Lets a part's shown look fall from where it stands, from `start` over `ms`. Looks already
/// falling keep their course.
fn lower(looks: &mut [Look], now: Millis, start: Millis, ms: u32) {
    for look in looks {
        if look.opacity.target() == SHOWN {
            look.opacity = Tween::new(look.opacity.value(now), 0, start, ms);
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
    placement: Placement,
    monuments: BTreeMap<u32, u8>,
    fossils: BTreeMap<u32, u8>,
    entries: BTreeMap<Subject, Entry>,
    label: Vec<LabelLook>,
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
            placement: Placement::default(),
            monuments: BTreeMap::new(),
            fossils: BTreeMap::new(),
            entries: BTreeMap::new(),
            label: Vec::new(),
        }
    }

    pub fn salt(&self) -> InstallSalt {
        self.salt
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn world(&self) -> &WorldLayout {
        &self.world
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
    /// not change.
    pub fn update(&mut self, input: &SceneInput, overlay: &OverlayInput, now: Millis) {
        for entry in self.entries.values_mut() {
            entry.prune(now);
        }
        self.entries
            .retain(|_, entry| entry.looks().next().is_some());
        self.placement = place(&self.placement, input, &self.bands);
        self.monuments = assign_spots(
            &self.monuments,
            input.monuments.iter().map(|m| m.id),
            world::MONUMENTS,
        );
        self.fossils = assign_spots(
            &self.fossils,
            input.fossils.iter().map(|f| f.id),
            world::FOSSILS,
        );
        let wanted = self.wanted(input, overlay);
        for (subject, entry) in &mut self.entries {
            if !wanted.contains_key(subject) {
                entry.hide(now, timing(*subject, self.light_ms).leave);
            }
        }
        for (subject, pieces) in wanted {
            let timing = timing(subject, self.light_ms);
            let entry = self.entries.entry(subject).or_default();
            let rise = if entry.shown() {
                timing.change
            } else {
                timing.arrive
            };
            entry.show(pieces, now, rise, timing.change);
        }
        self.update_label(&overlay.label, now);
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
        let weather = input.weather;
        wanted.insert(Subject::Weather, one(Piece::Weather(weather)));
        for (i, &size) in (0_u8..).zip(sky::clouds(weather).iter().take(quiet.clouds)) {
            wanted.insert(Subject::Cloud(i), one(Piece::Cloud(size)));
        }
        if sky::shows_sun_and_moon(weather) {
            if quiet.sun {
                wanted.insert(Subject::Sun, one(Piece::Sun));
            }
            if let Some(look) = moon_phase(overlay.date).look() {
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

    /// When the last tween ends. From then on only the clouds' drift and the label's cycle
    /// move until the next update.
    pub fn settled_at(&self) -> Millis {
        self.entries
            .values()
            .flat_map(Entry::looks)
            .map(|look| look.opacity)
            .chain(self.label.iter().map(|look| look.opacity))
            .map(Tween::end)
            .max()
            .unwrap_or_default()
    }

    /// Every visible look at `now`, as depicted states in subject order. The label's text,
    /// which reads no salt, is not among them.
    pub fn depicted(&self, now: Millis) -> Vec<Depicted> {
        let mut depicted = Vec::new();
        for (&subject, entry) in &self.entries {
            for (part, looks) in (0_u8..).zip(&entry.parts) {
                for look in looks {
                    let opacity = look.opacity.value(now);
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
        for (&subject, entry) in &self.entries {
            for (part, looks) in (0_u8..).zip(&entry.parts) {
                for (index, look) in (0_u16..).zip(looks) {
                    let value = look.opacity.value(now);
                    if value == 0 {
                        continue;
                    }
                    let place = Place::new(subject, part, index);
                    let lit = Lit { value, palette_mix };
                    self.push(art, table, place, look.piece, lit, now, &mut keyed);
                }
            }
        }
        self.push_label(art, now, &mut keyed);
        keyed.sort_by_key(|keyed| keyed.key);
        keyed
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
        match place.subject {
            Subject::Weather => {
                // A whole look paints together at the band's origin, so a new weather covers
                // the old in every column, whatever the two tiles' widths.
                let step = usize::from(rect.w).max(1);
                for (tile, x) in (0_u16..).zip((0..FRAME_WIDTH).step_by(step)) {
                    let draw = Draw::new(catalog, sprite, x, 0);
                    push(place.item(tile), Some((0, 0)), draw);
                }
            }
            Subject::Cloud(i) => {
                let drift = self.world.cloud(i);
                push(
                    place,
                    None,
                    Draw::new(catalog, sprite, drift.x(now), drift.y),
                );
            }
            Subject::Sun => {
                let (x, y) = self.world.sun();
                push(place, None, Draw::new(catalog, sprite, x, y));
            }
            Subject::Moon => {
                let (x, y) = self.world.moon();
                let mirrored = matches!(piece, Piece::Moon(look) if look.mirrored);
                let draw = Draw {
                    flip_x: mirrored,
                    ..Draw::new(catalog, sprite, x, y)
                };
                push(place, None, draw);
            }
            Subject::Star(i) => {
                let (x, y) = self.world.star(i);
                push(place, None, Draw::new(catalog, sprite, x, y));
            }
            Subject::Light | Subject::Film(_) => {
                push(place, None, Draw::new(catalog, sprite, 0, 0));
            }
            Subject::Monument { spot, .. } => {
                let x = self.world.monument(spot);
                let draw = match piece {
                    Piece::Mark { age } => {
                        let Some(stone) = table.monument(age).first() else {
                            return;
                        };
                        let stone = catalog.sprite(*stone);
                        let mark = usize::from(place.part) - FIRST_MARK;
                        let Some(point) = table.world().mark_points.get(mark) else {
                            return;
                        };
                        let left = x - i32::from(stone.anchor.x) + i32::from(point.x);
                        let top = PLANT_GROUND_Y - i32::from(stone.anchor.y) + i32::from(point.y);
                        Draw::new(catalog, sprite, left, top)
                    }
                    _ => ground(x),
                };
                // Every part keyed at the monument's ground point, so the marks paint over
                // their stone rather than sorting under it.
                push(place, Some((x, PLANT_GROUND_Y)), draw);
            }
            Subject::Fossil { spot, .. } => {
                let draw = Draw::new(
                    catalog,
                    sprite,
                    self.world.fossil(spot),
                    FRAME_HEIGHT - i32::from(rect.h),
                );
                push(place, None, draw);
            }
            Subject::Plant { cell, .. } | Subject::Fill { cell } | Subject::Weed { cell } => {
                let x = self.layout.slot(cell).x;
                let draw = match piece {
                    Piece::PlantGlow { species, stage } => {
                        let (cx, cy) = foliage_centre(art, table, species, stage, x);
                        Draw::anchored(catalog, sprite, cx, cy)
                    }
                    _ => ground(x),
                };
                push(place, None, draw);
            }
            Subject::Dressing(i) => push(place, None, ground(self.world.dressing(i))),
            Subject::Litter(i) => push(place, None, ground(self.world.litter(i))),
            Subject::Fungus(i) => push(place, None, ground(self.world.fungus(i))),
            Subject::Scenery(_) | Subject::Label => {}
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
                let opacity = times_opacity(value, line_value);
                let Some(text) = look.lines.get(line) else {
                    continue;
                };
                let part = u8::try_from(line).unwrap_or(u8::MAX);
                for (item, glyph) in (0_u16..).zip(label.fitted_draws(text, LABEL_X, LABEL_Y)) {
                    let opacity = times_opacity(glyph.opacity, opacity);
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
        Frame {
            draws: merged(&self.draws(art, table, palette_mix, now), scenery.keyed()),
            palette_mix,
            dim: 0,
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
        // Never entries: the label keeps its own looks, timed by `update_label`, and the
        // scenery never changes. They are here only to keep the match whole.
        | Subject::Label
        | Subject::Scenery(_) => (FILL_MS, FILL_MS, HALF_RESTYLE),
    };
    Timing {
        arrive,
        leave,
        change,
    }
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
    let top = PLANT_GROUND_Y - i32::from(sprite.anchor.y);
    (
        left + i32::from(sprite.rect.w) / 2,
        top + i32::from(sprite.rect.h) / 2,
    )
}

/// How brightly a look is drawn: its own opacity, and the time of day for art that shows only
/// by day or only by night.
#[derive(Clone, Copy, Debug)]
struct Lit {
    value: u8,
    palette_mix: u8,
}

impl Lit {
    /// The draw at the look's opacity, scaled down by day for night-only art and by night for
    /// day-only art; `None` once nothing of it shows.
    fn apply(self, art: &Art, draw: Draw) -> Option<Draw> {
        let flags = art.catalog.sprite(draw.sprite).flags;
        let mut opacity = times_opacity(draw.opacity, self.value);
        if flags.contains(Flags::NIGHT_ONLY) {
            opacity = times_opacity(opacity, self.palette_mix);
        }
        if flags.contains(Flags::DAY_ONLY) {
            opacity = times_opacity(opacity, u8::MAX - self.palette_mix);
        }
        (opacity > 0).then_some(Draw { opacity, ..draw })
    }
}

/// A sprite's own opacity times a look's, rounded to nearest.
fn times_opacity(own: u8, look: u8) -> u8 {
    u8::try_from((u16::from(own) * u16::from(look) + 127) / 255).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
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
        OverlayInput {
            label: lines.iter().map(|line| (*line).to_owned()).collect(),
            date: preset::DATE,
        }
    }

    fn visible(composer: &Composer, table: &ArtTable, now: Millis) -> Vec<Draw> {
        visible_at(composer, table, 0, now)
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
        let middle = visible(&composer, &table, Millis(3000));
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

    #[test]
    fn a_film_crossfades_so_two_levels_never_stack() {
        let film = |level| Piece::Film(GlassFilm::Condensation, level);
        let mut looks = vec![Look {
            piece: film(1),
            opacity: Tween::at(SHOWN),
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
                .all(|&o| o == times_opacity(art.catalog.sprite(glow).opacity, 128))
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
        // Clear tiles are 8 wide and overcast ones 16: keyed by x, they would interleave.
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        let mut clear = preset::bare(PresentationState::Thriving, Vec::new());
        composer.update(&clear, &overlay(&[]), Millis(0));
        clear.weather = Weather::Overcast;
        let settled = composer.settled_at();
        composer.update(&clear, &overlay(&[]), settled);
        let mid = settled.plus(HALF_RESTYLE / 2);
        let tiles = of(&composer, &table, 0, mid, Subject::Weather);
        let looks: Vec<u16> = tiles.iter().map(|keyed| keyed.key.look).collect();
        let first_new = looks
            .iter()
            .position(|&look| look == 1)
            .expect("the new weather");
        assert!(looks[..first_new].iter().all(|&look| look == 0));
        assert!(looks[first_new..].iter().all(|&look| look == 1));
        let width = |look| -> i32 {
            tiles
                .iter()
                .filter(|keyed| keyed.key.look == look)
                .map(|keyed| i32::from(Art::embedded().catalog.sprite(keyed.draw.sprite).rect.w))
                .sum()
        };
        assert_eq!((width(0), width(1)), (640, 640));
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
    fn the_moon_shows_its_phase_and_new_moon_shows_none() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        let input = preset::bare(PresentationState::Thriving, Vec::new());
        let at =
            |composer: &Composer| of(composer, &table, 255, composer.settled_at(), Subject::Moon);
        let date = |date| OverlayInput {
            label: Vec::new(),
            date,
        };
        composer.update(&input, &date(LocalDate::new(2026, 3, 19)), Millis(0));
        assert!(at(&composer).is_empty());
        composer.update(
            &input,
            &date(LocalDate::new(2026, 3, 15)),
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
