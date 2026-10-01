//! The composer: from the `SceneInput`, the install salt and the clock to the draws of a frame.
//! It depicts the states truth gives it and never judges health itself.
//!
//! Each update places the plants and filler ([`crate::place`]), then moves every subject's
//! looks toward that placement. A subject is a plant or filler in a cell. Each has three parts:
//! the stem, the foliage (or a combined seedling, or the filler's one image) and the bloom.
//! Each part keeps its looks oldest first, and the newest is the one it is becoming.
//!
//! Nothing ever cuts (I-15, constraint 7):
//! - **A subject arrives** with every part rising from nothing.
//! - **A subject leaves** with every part falling from wherever it stands.
//! - **A part changes look** with the new look rising on top for half the restyle while every
//!   older look holds, and only then do the older ones fall. No moment shows less of the plant
//!   than the moment before, so the sky never shows through a change. A part whose look is
//!   unchanged does not move: a plant turning pale fades its foliage, never its stem.
//! - **An update** only starts tweens from where their values stand, so the frame at the
//!   moment of an update is the frame just before it.

use std::collections::BTreeMap;

use terrarium_truth::scene::{GrowthStage, PlantSpecies, PlantState, PlantView, SceneInput};

use crate::catalog::{Art, SpriteId};
use crate::frame::{Draw, Frame, PLANT_GROUND_Y};
use crate::order::{KeyedDraw, Subject, in_order};
use crate::place::{Bands, FillForm, Placement, place};
use crate::plant::shows_bloom;
use crate::salt::{InstallSalt, Purpose};
use crate::slots::Layout;
use crate::tween::durations::{APPEAR_MS, FILL_MS, RESTYLE_MS, VANISH_MS};
use crate::tween::{Millis, Tween};
use crate::variants::{ArtTable, PlantArt};

const SHOWN: u8 = u8::MAX;
const HALF_RESTYLE: u32 = RESTYLE_MS / 2;

/// What one look of a part shows. A piece is the depicted state; its sprite is looked up only
/// when drawing.
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
    /// A ground tuft; which of the equivalent tufts is the salt's choice for the cell.
    Tuft,
    /// Dead-stem litter; which of them is the salt's choice for the cell.
    DeadStems,
}

impl Piece {
    fn sprite(self, table: &ArtTable, salt: InstallSalt, cell: u8) -> SpriteId {
        let variant = |sprites: &[SpriteId]| {
            let n = u64::try_from(sprites.len()).unwrap_or(1);
            let pick = salt.below(Purpose::FillVariant, u64::from(cell), n);
            sprites[usize::try_from(pick).unwrap_or(0)]
        };
        match self {
            Piece::Stem { species, stage } => {
                match table.plant(species, stage, PlantState::Thriving) {
                    PlantArt::Parts { stem, .. } => stem,
                    PlantArt::Seedling(combined) => combined,
                }
            }
            Piece::Foliage {
                species,
                stage,
                state,
            } => match table.plant(species, stage, state) {
                PlantArt::Parts { foliage, .. } => foliage,
                PlantArt::Seedling(combined) => combined,
            },
            Piece::Bloom { species } => table.bloom(species),
            Piece::Tuft => variant(table.tufts()),
            Piece::DeadStems => variant(table.dead_stems()),
        }
    }
}

/// The parts of a subject, bottom to top.
const PARTS: usize = 3;
const STEM: usize = 0;
const FOLIAGE: usize = 1;
const BLOOM: usize = 2;

fn plant_pieces(view: &PlantView) -> [Option<Piece>; PARTS] {
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
    pieces
}

fn fill_pieces(form: FillForm) -> [Option<Piece>; PARTS] {
    match form {
        FillForm::Tuft => [None, Some(Piece::Tuft), None],
        FillForm::DeadStems => [None, Some(Piece::DeadStems), None],
        FillForm::Plant { species, state } => [
            Some(Piece::Stem {
                species,
                stage: FillForm::STAGE,
            }),
            Some(Piece::Foliage {
                species,
                stage: FillForm::STAGE,
                state,
            }),
            None,
        ],
    }
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

    /// Moves each part toward its piece, rising over `rise`; a part with none falls once the
    /// others have risen.
    fn show(&mut self, pieces: [Option<Piece>; PARTS], now: Millis, rise: u32) {
        for (looks, piece) in self.parts.iter_mut().zip(pieces) {
            match piece {
                Some(piece) => raise(looks, piece, now, rise),
                None => lower(looks, now, now.plus(rise), HALF_RESTYLE),
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

/// Brings `piece` to the top of a part, then holds every older look until it has risen and
/// lets them fall after, so nothing beneath ever shows through.
fn raise(looks: &mut Vec<Look>, piece: Piece, now: Millis, rise: u32) {
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
    let risen = now.plus(rise);
    let older = looks.len() - 1;
    for look in &mut looks[..older] {
        look.opacity = Tween::new(look.opacity.value(now), 0, risen, HALF_RESTYLE);
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
    placement: Placement,
    entries: BTreeMap<Subject, Entry>,
}

impl Composer {
    /// An empty terrarium: the first update fades everything in.
    pub fn new(table: &ArtTable, salt: InstallSalt) -> Composer {
        Composer {
            salt,
            bands: Bands::new(table),
            layout: Layout::new(salt),
            placement: Placement::default(),
            entries: BTreeMap::new(),
        }
    }

    pub fn salt(&self) -> InstallSalt {
        self.salt
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn placement(&self) -> &Placement {
        &self.placement
    }

    /// Moves the depiction toward `input` from `now`. The frame at `now` does not change.
    pub fn update(&mut self, input: &SceneInput, now: Millis) {
        for entry in self.entries.values_mut() {
            entry.prune(now);
        }
        self.entries
            .retain(|_, entry| entry.looks().next().is_some());
        self.placement = place(&self.placement, input, &self.bands);
        let views: BTreeMap<u32, &PlantView> = input.plants.iter().map(|p| (p.id, p)).collect();
        let mut wanted: BTreeMap<Subject, [Option<Piece>; PARTS]> = BTreeMap::new();
        for (&id, placed) in &self.placement.plants {
            if let Some(view) = views.get(&id) {
                let subject = Subject::Plant {
                    id,
                    cell: placed.cell,
                };
                wanted.insert(subject, plant_pieces(view));
            }
        }
        for (&cell, &form) in &self.placement.fill {
            wanted.insert(Subject::Fill { cell }, fill_pieces(form));
        }
        for (subject, entry) in &mut self.entries {
            if !wanted.contains_key(subject) {
                entry.hide(now, leave_ms(*subject));
            }
        }
        for (subject, pieces) in wanted {
            let entry = self.entries.entry(subject).or_default();
            let rise = if entry.shown() {
                HALF_RESTYLE
            } else {
                arrive_ms(subject)
            };
            entry.show(pieces, now, rise);
        }
    }

    /// When the last tween ends; from then on, every frame is the same until the next update.
    pub fn settled_at(&self) -> Millis {
        self.entries
            .values()
            .flat_map(Entry::looks)
            .map(|look| look.opacity.end())
            .max()
            .unwrap_or_default()
    }

    /// Every visible look at `now`, as depicted states in subject order.
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

    /// The plant and filler draws at `now`, each keyed by its place in the depth order, in
    /// that order.
    pub fn draws(&self, art: &Art, table: &ArtTable, now: Millis) -> Vec<KeyedDraw> {
        let catalog = &art.catalog;
        let mut keyed = Vec::new();
        for (&subject, entry) in &self.entries {
            let Some(cell) = subject.cell() else {
                continue;
            };
            let x = self.layout.slot(cell).x;
            for (part, looks) in (0_u8..).zip(&entry.parts) {
                for (index, look) in (0_u16..).zip(looks) {
                    let sprite = look.piece.sprite(table, self.salt, cell);
                    let draw = Draw::anchored(catalog, sprite, x, PLANT_GROUND_Y);
                    let opacity = scale(draw.opacity, look.opacity.value(now));
                    if opacity > 0 {
                        let draw = Draw { opacity, ..draw };
                        keyed.push(KeyedDraw::new(catalog, subject, part, index, draw));
                    }
                }
            }
        }
        keyed.sort_by_key(|keyed| keyed.key);
        keyed
    }

    /// The whole frame at `now`: the composer's draws with the fixed scenery, in depth order.
    pub fn frame(
        &self,
        art: &Art,
        table: &ArtTable,
        label: &str,
        palette_mix: u8,
        now: Millis,
    ) -> Frame {
        let mut keyed = self.draws(art, table, now);
        keyed.extend(
            (0_u16..)
                .zip(crate::sample::scenery(art, label))
                .map(|(i, draw)| KeyedDraw::new(&art.catalog, Subject::Scenery(i), 0, 0, draw)),
        );
        Frame {
            draws: in_order(keyed),
            palette_mix,
            dim: 0,
        }
    }
}

fn arrive_ms(subject: Subject) -> u32 {
    match subject {
        Subject::Plant { .. } => APPEAR_MS,
        Subject::Fill { .. } | Subject::Scenery(_) => FILL_MS,
    }
}

fn leave_ms(subject: Subject) -> u32 {
    match subject {
        Subject::Plant { .. } => VANISH_MS,
        Subject::Fill { .. } | Subject::Scenery(_) => FILL_MS,
    }
}

/// A sprite's own opacity scaled by a look's, rounded to nearest.
fn scale(own: u8, look: u8) -> u8 {
    u8::try_from((u16::from(own) * u16::from(look) + 127) / 255).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset;

    fn table() -> ArtTable {
        ArtTable::new(&Art::embedded().catalog).unwrap()
    }

    fn scene(name: &str) -> SceneInput {
        preset::scene(name).unwrap()
    }

    fn visible(composer: &Composer, table: &ArtTable, now: Millis) -> Vec<Draw> {
        composer
            .draws(Art::embedded(), table, now)
            .into_iter()
            .map(|keyed| keyed.draw)
            .collect()
    }

    #[test]
    fn the_first_update_fades_everything_in_from_nothing() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        composer.update(&scene("thriving"), Millis(1000));
        assert!(visible(&composer, &table, Millis(1000)).is_empty());
        let middle = visible(&composer, &table, Millis(3000));
        assert!(!middle.is_empty() && middle.iter().all(|d| d.opacity < u8::MAX));
        let settled = composer.settled_at();
        assert_eq!(settled, Millis(1000 + u64::from(APPEAR_MS)));
        assert!(
            visible(&composer, &table, settled)
                .iter()
                .all(|d| d.opacity == u8::MAX)
        );
    }

    #[test]
    fn an_update_never_changes_the_frame_at_its_own_moment() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        let mut now = Millis(0);
        for (name, after) in [
            ("thriving", 2500),
            ("struggling-poor-soil", 1700),
            ("dead", 3000),
            ("recovering", 1),
            ("thriving", 9000),
            ("dormant", 4100),
        ] {
            let before = visible(&composer, &table, now);
            composer.update(&scene(name), now);
            assert_eq!(visible(&composer, &table, now), before, "{name}");
            now = now.plus(after);
        }
    }

    #[test]
    fn a_restyle_holds_the_old_look_until_the_new_one_has_risen() {
        let mut looks = vec![Look {
            piece: Piece::Tuft,
            opacity: Tween::at(SHOWN),
        }];
        raise(&mut looks, Piece::DeadStems, Millis(100), HALF_RESTYLE);
        let [old, new] = [looks[0], looks[1]];
        let risen = 100 + u64::from(HALF_RESTYLE);
        assert_eq!(old.opacity.value(Millis(risen)), SHOWN);
        assert_eq!(new.opacity.value(Millis(risen)), SHOWN);
        assert_eq!(
            old.opacity.value(Millis(risen + u64::from(HALF_RESTYLE))),
            0
        );
        // A third look mid-way holds both older ones where they stand.
        raise(&mut looks, Piece::Tuft, Millis(1100), HALF_RESTYLE);
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
    fn a_pale_turn_fades_the_foliage_and_never_the_stem() {
        let table = table();
        let mut composer = Composer::new(&table, preset::SALT);
        let thriving = scene("mature");
        composer.update(&thriving, Millis(0));
        let settled = composer.settled_at();
        let mut pale = thriving.clone();
        for plant in &mut pale.plants {
            plant.state = PlantState::Pale;
        }
        composer.update(&pale, settled);
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
        composer.update(&scene("thriving"), Millis(0));
        let full = plants(&composer);
        let mut empty = scene("thriving");
        empty.plants.clear();
        composer.update(&empty, composer.settled_at());
        // The plants are leaving; none is pruned until it has gone.
        assert_eq!(plants(&composer), full);
        composer.update(&empty, composer.settled_at().plus(1));
        assert_eq!(plants(&composer), 0);
        assert!(composer.entries.values().all(Entry::shown));
    }

    #[test]
    fn the_tuft_variant_is_the_salt_s_choice_and_never_changes() {
        let table = table();
        let tufts = table.tufts();
        let chosen: Vec<SpriteId> = (0..30)
            .map(|cell| Piece::Tuft.sprite(&table, preset::SALT, cell))
            .collect();
        assert!(chosen.iter().all(|id| tufts.contains(id)));
        assert!(chosen.iter().any(|&id| id != chosen[0]));
        assert_eq!(chosen[4], Piece::Tuft.sprite(&table, preset::SALT, 4));
    }
}
