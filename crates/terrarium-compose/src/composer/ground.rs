//! The foreground event's choreography on the ground, and the marks events leave in it.
//!
//! - **Marks.** Truth keeps the settled crack and sinkhole for 30 days or until rain after a
//!   drought, and fresh soil for 14. Each lies on the ground line for as long as the scene
//!   carries it, coming and going as the ground's cover does. Where it lies is read once, from
//!   the scene before the one that first carries it: fresh soil on the cells a burn took the
//!   weeds from; a sinkhole over its fossil, which takes the free spot nearest the plant the
//!   hole swallowed; a crack at a salted place. A fossil a rewrite took leaves its hole where it
//!   lay. Which cell or spot is never the salt's, so every install depicts the same marks.
//! - **Burn-off** plays over every weed drawn when it begins: its frames step the weeds out and
//!   the fresh soil in by the art's keys at each frame's start, and its ash clears. A sprinkle of
//!   flowers then comes and goes on each patch of fresh soil (the art's own tag). The
//!   burned cells stay bare until the show is over.
//! - **Scorch** follows lightning's bolt on the plant it struck, if the art lets that plant
//!   take one: the scorch rises over the foliage, the foliage goes behind it, then the plant's
//!   foliage as truth now has it comes back under it and the scorch falls. It never heals the
//!   plant by itself. The bloom and the night glow are hidden while the scorch shows.
//! - **Flowering** opens the blooms truth authorizes one plant at a time across the strip from
//!   the event's side, over a minute.
//! - **The crack and the sinkhole** open in their three frames into the settled ones truth
//!   keeps. An earthquake opens the sinkholes of the fossils it took with its crack.
//!
//! Every ground piece stands in the scene's columns, so a wider viewport moves it with the
//! scene. Authored frames and the art's keyed steps show and go as drawn (I-11); everything else
//! tweens.

use std::collections::BTreeMap;

use terrarium_truth::scene::{
    GroundMark, GrowthStage, PlantSpecies, PlantState, PlantView, SceneInput, Side,
};

use super::{
    BLOOM, Composer, FOLIAGE, FOLIAGE_PART, GROUND, Lit, Look, PARTS, PLANT_GLOW, Piece, Pieces,
    SHOWN,
};
use crate::anim;
use crate::blend::scale;
use crate::catalog::{Art, Envelope, SpriteId};
use crate::frame::{Draw, PLANT_GROUND_Y};
use crate::order::{KeyedDraw, Place, Subject};
use crate::place::weed_walk;
use crate::plant;
use crate::slots;
use crate::sway;
use crate::tween::durations::APPEAR_MS;
use crate::tween::{Millis, Tween};
use crate::variants::ArtTable;
use crate::world::{self, Kind};

/// A flowering opens its blooms across this long.
const FLOWERING_MS: u32 = 60_000;
/// Fresh soil with no cells kept (a restart, or a burn with no weeds drawn) lies on this many of
/// the cells weeds take first.
const FRESH_FALLBACK: usize = 2;
/// How many moments a search for the next visible change of a scorched plant looks through:
/// more than its tweens' steps together.
const SEARCH: usize = 2048;

/// What a ground choreography keeps beside its show, from its cue.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Stage {
    /// The weeds a burn takes, each look as it was drawn at the cue.
    burnt: Vec<Burnt>,
    /// The cells a burn clears, which stay bare until its show is over.
    bare: Vec<u8>,
    bare_until: Millis,
    /// The fresh soil a burn leaves: its mark, and the cells it lies on with the look each
    /// steps in by.
    fresh: Option<(u32, Vec<(u8, u16)>)>,
    /// When that fresh soil's own look takes over from the show's keys.
    whole: Millis,
    /// When the scene stopped holding that fresh soil before its look took over, and when the
    /// show ends: the show fades it, and the flowers on it, out between.
    left: Option<(Millis, Millis)>,
    /// The marks a show opens into their settled frames, each with the look that settles.
    settling: Vec<(Subject, GroundMark, u16)>,
    /// When each plant a flowering opens blooms.
    blooms: BTreeMap<Subject, Millis>,
}

/// One look of a weed a burn takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Burnt {
    cell: u8,
    look: u16,
    piece: Piece,
    value: u8,
}

/// Lightning's scorch on the plant it struck, from the bolt's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Scorched {
    id: u32,
    cell: u8,
    species: PlantSpecies,
    start: Millis,
    envelope: Envelope,
}

impl Scorched {
    fn plant(self) -> Subject {
        Subject::Plant {
            id: self.id,
            cell: self.cell,
        }
    }

    /// The enter, hold and exit, and the exit's two halves.
    fn spans(self) -> (u32, u32, u32, u32) {
        let Envelope {
            enter_ms,
            hold_ms,
            exit_ms,
        } = self.envelope;
        let half = exit_ms / 2;
        (
            u32::from(enter_ms),
            u32::from(hold_ms),
            u32::from(half),
            u32::from(exit_ms - half),
        )
    }

    pub(super) fn end(self) -> Millis {
        let (enter, hold, first, second) = self.spans();
        self.start.plus(enter + hold + first + second)
    }

    /// The scorch rises over the foliage, and falls once the foliage is back under it.
    fn scorch(self) -> [Tween; 2] {
        let (enter, hold, first, second) = self.spans();
        [
            Tween::new(0, SHOWN, self.start, enter),
            Tween::new(SHOWN, 0, self.start.plus(enter + hold + first), second),
        ]
    }

    /// The foliage goes behind the scorch through its hold, and comes back under it first.
    fn foliage(self) -> [Tween; 2] {
        let (enter, hold, first, _) = self.spans();
        [
            Tween::new(SHOWN, 0, self.start.plus(enter), hold),
            Tween::new(0, SHOWN, self.start.plus(enter + hold), first),
        ]
    }

    /// The bloom and the night glow go as the scorch rises and come back as it falls.
    fn bloom(self) -> [Tween; 2] {
        let (enter, hold, first, second) = self.spans();
        [
            Tween::new(SHOWN, 0, self.start, enter),
            Tween::new(0, SHOWN, self.start.plus(enter + hold + first), second),
        ]
    }

    fn rising_then_falling([rise, fall]: [Tween; 2], now: Millis) -> u8 {
        rise.value(now).min(fall.value(now))
    }

    fn falling_then_rising([fall, rise]: [Tween; 2], now: Millis) -> u8 {
        fall.value(now).max(rise.value(now))
    }
}

/// The spot a hole whose fossil has none lies at: one of its id's, the same on every install.
fn spot_of(fossil: u32) -> u8 {
    u8::try_from(usize::try_from(fossil).unwrap_or(0) % world::FOSSILS).unwrap_or(0)
}

/// An art key in thousandths as an opacity.
fn permille(value: u16) -> u8 {
    u8::try_from((u32::from(value) * u32::from(SHOWN) + 500) / 1000).unwrap_or(SHOWN)
}

/// The earliest of each tween's next step after `at`.
fn next_of(tweens: &[Tween], at: Millis) -> Option<Millis> {
    tweens.iter().filter_map(|tween| tween.next_step(at)).min()
}

/// The first moment after `now` at which `seen` differs from what it is at `now`, looking only
/// at the moments `next` names from each moment on.
fn first_seen(
    now: Millis,
    seen: impl Fn(Millis) -> u8,
    next: impl Fn(Millis) -> Option<Millis>,
) -> Option<Millis> {
    let before = seen(now);
    let mut at = now;
    for _ in 0..SEARCH {
        at = next(at)?;
        if seen(at) != before {
            return Some(at);
        }
    }
    None
}

impl Composer {
    /// The cells whose weeds are drawn at `now`.
    fn weeds_drawn(&self, now: Millis) -> Vec<u8> {
        self.entries
            .iter()
            .filter_map(|(subject, entry)| match *subject {
                Subject::Weed { cell } if entry.looks().any(|l| l.opacity.value(now) > 0) => {
                    Some(cell)
                }
                _ => None,
            })
            .collect()
    }

    /// The cell of the living plant the scene no longer holds, the lowest id of several: the
    /// one a sinkhole first carried in this scene swallowed, as truth swallows only a living
    /// plant (a dead one leaves at a day's close).
    fn swallowed(&self, input: &SceneInput) -> Option<u8> {
        self.placement
            .plants
            .iter()
            .filter(|(id, placed)| {
                placed.view.state != PlantState::Dead && !input.plants.iter().any(|p| p.id == **id)
            })
            .map(|(_, placed)| placed.cell)
            .next()
    }

    /// Reads where the scene's new marks lie from the scene before it, which the composer
    /// still depicts: the weeds drawn, the plant gone. It runs before the update places
    /// anything.
    pub(super) fn remember_marks(&mut self, input: &SceneInput, now: Millis) {
        let marks = &input.ground_marks;
        let weeds = self.weeds_drawn(now);
        for mark in marks.iter().filter(|m| m.kind == GroundMark::FreshSoil) {
            self.fresh.entry(mark.id).or_insert_with(|| {
                if weeds.is_empty() {
                    weed_walk().take(FRESH_FALLBACK).collect()
                } else {
                    weeds.clone()
                }
            });
        }
        self.fresh.retain(|id, _| {
            marks
                .iter()
                .any(|m| m.id == *id && m.kind == GroundMark::FreshSoil)
        });
        let present = |id: u32| input.fossils.iter().any(|f| f.id == id);
        let mut used: Vec<u8> = self
            .fossils
            .iter()
            .filter(|&(&id, _)| present(id))
            .map(|(_, &spot)| spot)
            .collect();
        let swallowed = self.swallowed(input);
        for fossil in marks.iter().filter_map(|m| m.fossil) {
            if self.sunk.contains_key(&fossil) {
                continue;
            }
            if let Some(&spot) = self.fossils.get(&fossil) {
                self.sunk.insert(fossil, spot);
            } else if !present(fossil) {
                self.sunk.insert(fossil, spot_of(fossil));
            } else if let Some(cell) = swallowed {
                // A fossil arriving with the hole that swallowed a plant lies as near it as a
                // free spot allows, by the places the salt does not move.
                let x = slots::centre(usize::from(cell));
                let nearest = (0..world::FOSSILS)
                    .filter_map(|spot| u8::try_from(spot).ok())
                    .filter(|spot| !used.contains(spot))
                    .min_by_key(|&spot| {
                        ((world::fossil_centre(usize::from(spot)) - x).abs(), spot)
                    });
                if let Some(spot) = nearest {
                    self.sunk.insert(fossil, spot);
                    used.push(spot);
                }
            }
        }
    }

    /// The spots of the scene's fossils as they stand, with the spot a sinkhole chose for a
    /// fossil arriving with it, for `assign_spots` to keep.
    pub(super) fn fossil_claims(&self, input: &SceneInput) -> BTreeMap<u32, u8> {
        let mut claims: BTreeMap<u32, u8> = self
            .fossils
            .iter()
            .filter(|(id, _)| input.fossils.iter().any(|f| f.id == **id))
            .map(|(&id, &spot)| (id, spot))
            .collect();
        for fossil in &input.fossils {
            if let Some(&spot) = self.sunk.get(&fossil.id)
                && !claims.contains_key(&fossil.id)
                && !claims.values().any(|&taken| taken == spot)
            {
                claims.insert(fossil.id, spot);
            }
        }
        claims
    }

    /// Keeps the spot of each fossil a sinkhole names, once spots are assigned, while a mark
    /// names it: the spot its fossil was given, so the hole always stands over it; for a fossil
    /// gone, the spot it last had; for one no spot was free for, one of its id's.
    pub(super) fn settle_sunk(&mut self, input: &SceneInput) {
        let named: Vec<u32> = input.ground_marks.iter().filter_map(|m| m.fossil).collect();
        for &fossil in &named {
            match self.fossils.get(&fossil) {
                Some(&spot) => {
                    self.sunk.insert(fossil, spot);
                }
                None => {
                    self.sunk.entry(fossil).or_insert_with(|| spot_of(fossil));
                }
            }
        }
        self.sunk.retain(|fossil, _| named.contains(fossil));
    }

    /// Every mark the scene holds, where it lies.
    pub(super) fn marks(&self, input: &SceneInput) -> Vec<(Subject, GroundMark)> {
        let mut out = Vec::new();
        for mark in &input.ground_marks {
            let at = |at: u8| Subject::Mark { id: mark.id, at };
            match mark.kind {
                GroundMark::Crack => out.push((at(0), mark.kind)),
                GroundMark::Sinkhole => {
                    if let Some(&spot) = mark.fossil.and_then(|f| self.sunk.get(&f)) {
                        out.push((at(spot), mark.kind));
                    }
                }
                GroundMark::FreshSoil => {
                    for &cell in self.fresh.get(&mark.id).into_iter().flatten() {
                        out.push((at(cell), mark.kind));
                    }
                }
            }
        }
        out
    }

    /// The x a mark is drawn at: a crack's left edge, a sinkhole's middle over its fossil, fresh
    /// soil's middle in its cell.
    pub(super) fn mark_x(
        &self,
        art: &Art,
        table: &ArtTable,
        kind: GroundMark,
        id: u32,
        at: u8,
    ) -> i32 {
        let catalog = &art.catalog;
        let width = |sprite: SpriteId| i32::from(catalog.sprite(sprite).rect.w);
        match kind {
            GroundMark::Crack => {
                let crack = table.ground_mark(kind).first().copied().map_or(0, width);
                self.world.crack(id, crack)
            }
            GroundMark::Sinkhole => {
                let widest = terrarium_truth::scene::FossilForm::ALL
                    .iter()
                    .map(|&form| width(table.fossil(form)))
                    .max()
                    .unwrap_or(0);
                self.world.fossil(at) + widest / 2
            }
            GroundMark::FreshSoil => self.layout.slot(at).x,
        }
    }

    /// A mark's draw, anchored on the ground line, with the point it is keyed at.
    pub(super) fn mark_draw(
        &self,
        art: &Art,
        table: &ArtTable,
        piece: Piece,
        (id, at): (u32, u8),
        sprite: SpriteId,
    ) -> Option<((i32, i32), Draw)> {
        let Piece::Ground(kind) = piece else {
            return None;
        };
        let x = self.mark_x(art, table, kind, id, at);
        Some(((x, GROUND), Draw::anchored(&art.catalog, sprite, x, GROUND)))
    }

    /// When each part of `subject` starts toward what the scene holds from `start`: a burned
    /// cell's weeds and filler once the burn's show is over, a flowering plant's bloom at its
    /// turn. Never later than `start` where a look of the part still moves at `now`.
    pub(super) fn starts(&self, subject: Subject, start: Millis, now: Millis) -> [Millis; PARTS] {
        let mut starts = [start; PARTS];
        let still = |parts: std::ops::Range<usize>, until: Millis| {
            self.entries
                .get(&subject)
                .is_none_or(|entry| entry.still(parts, now, until))
        };
        let stage = &self.stage;
        if let Subject::Weed { cell } | Subject::Fill { cell } = subject
            && stage.bare.contains(&cell)
            && stage.bare_until > start
            && still(0..PARTS, stage.bare_until)
        {
            starts = [stage.bare_until; PARTS];
        }
        if let Some(&at) = stage.blooms.get(&subject)
            && at > start
            && still(BLOOM..BLOOM + 1, at)
        {
            starts[BLOOM] = at;
        }
        starts
    }

    /// Stages a burn over the weeds drawn at `now`, from `start`: their looks give way to the
    /// show's, and the scene's new fresh soil steps in on their cells. Returns when the show is
    /// over, or `None` where no weed is drawn.
    pub(super) fn cue_burn(
        &mut self,
        input: &SceneInput,
        now: Millis,
        start: Millis,
    ) -> Option<Millis> {
        let cells = self.weeds_drawn(now);
        if cells.is_empty() {
            return None;
        }
        let mut stage = Stage::default();
        let handed = Millis(start.0.saturating_sub(1));
        for &cell in &cells {
            let Some(entry) = self.entries.get_mut(&Subject::Weed { cell }) else {
                continue;
            };
            for (look, drawn) in (0_u16..).zip(&mut entry.parts[0]) {
                let value = drawn.opacity.value(now);
                if value > 0 {
                    stage.burnt.push(Burnt {
                        cell,
                        look,
                        piece: drawn.piece,
                        value,
                    });
                    drawn.opacity = Tween::new(value, 0, handed, 1);
                }
            }
        }
        let art = &self.ground_art;
        let whole = Millis(start.0 + art.fresh_from_ms);
        let fresh = input
            .ground_marks
            .iter()
            .filter(|m| m.kind == GroundMark::FreshSoil)
            .find(|m| {
                !self
                    .entries
                    .keys()
                    .any(|s| matches!(s, Subject::Mark { id, .. } if *id == m.id))
            });
        let mut end = Millis(start.0 + art.burnoff_ms);
        if let Some(mark) = fresh {
            let mut lying = Vec::new();
            for &cell in self.fresh.get(&mark.id).into_iter().flatten() {
                let looks = &mut self
                    .entries
                    .entry(Subject::Mark {
                        id: mark.id,
                        at: cell,
                    })
                    .or_default()
                    .parts[0];
                lying.push((cell, u16::try_from(looks.len()).unwrap_or(u16::MAX)));
                looks.push(Look {
                    piece: Piece::Ground(GroundMark::FreshSoil),
                    opacity: Tween::new(0, SHOWN, Millis(whole.0.saturating_sub(1)), 1),
                    behind: false,
                });
            }
            let Envelope {
                enter_ms,
                hold_ms,
                exit_ms,
            } = art.sprinkle;
            end = end.plus(u32::from(enter_ms) + u32::from(hold_ms) + u32::from(exit_ms));
            stage.fresh = Some((mark.id, lying));
            stage.whole = whole;
        }
        stage.bare = cells;
        stage.bare_until = end;
        self.stage = stage;
        Some(end)
    }

    /// Stages a flowering from `start`: each plant the scene newly blooms opens at its turn,
    /// across the strip from `side`. Returns when the last has opened, or `None` where none
    /// blooms anew.
    pub(super) fn cue_bloom(
        &mut self,
        wanted: &BTreeMap<Subject, Pieces>,
        start: Millis,
        side: Side,
    ) -> Option<Millis> {
        let mut opening: Vec<(i32, Subject)> = wanted
            .iter()
            .filter_map(|(&subject, pieces)| {
                let Subject::Plant { cell, .. } = subject else {
                    return None;
                };
                pieces[BLOOM]?;
                let open = self.entries.get(&subject).is_some_and(|entry| {
                    entry.parts[BLOOM]
                        .iter()
                        .any(|look| look.opacity.target() == SHOWN)
                });
                (!open).then(|| (slots::centre(usize::from(cell)), subject))
            })
            .collect();
        if opening.is_empty() {
            return None;
        }
        opening.sort_unstable();
        if side == Side::Right {
            opening.reverse();
        }
        let every = FLOWERING_MS / u32::try_from(opening.len()).unwrap_or(1).max(1);
        let mut stage = Stage::default();
        let mut last = start;
        for (i, (_, subject)) in (0_u32..).zip(opening) {
            last = start.plus(i * every);
            stage.blooms.insert(subject, last);
        }
        self.stage = stage;
        Some(last.plus(APPEAR_MS))
    }

    /// Stages the marks of `kinds` new in the scene to open from `start` into their settled
    /// frames. Returns when the last has settled, or `None` where none is new.
    pub(super) fn cue_settling(
        &mut self,
        wanted: &BTreeMap<Subject, Pieces>,
        now: Millis,
        start: Millis,
        kinds: &[GroundMark],
    ) -> Option<Millis> {
        let new: Vec<(Subject, GroundMark)> = wanted
            .iter()
            .filter_map(|(&subject, pieces)| match pieces[0] {
                Some(Piece::Ground(kind)) if kinds.contains(&kind) => Some((subject, kind)),
                _ => None,
            })
            .filter(|(subject, _)| {
                self.entries.get(subject).is_none_or(|entry| {
                    entry
                        .looks()
                        .all(|look| look.opacity.target() == 0 && look.opacity.value(now) == 0)
                })
            })
            .collect();
        if new.is_empty() {
            return None;
        }
        let mut stage = Stage::default();
        let mut end = start;
        for (subject, kind) in new {
            let Some((_, settles_ms)) = self.opening(kind) else {
                continue;
            };
            let settles = Millis(start.0 + settles_ms);
            let looks = &mut self.entries.entry(subject).or_default().parts[0];
            stage.settling.push((
                subject,
                kind,
                u16::try_from(looks.len()).unwrap_or(u16::MAX),
            ));
            looks.push(Look {
                piece: Piece::Ground(kind),
                opacity: Tween::new(0, SHOWN, Millis(settles.0.saturating_sub(1)), 1),
                behind: false,
            });
            end = end.max(settles);
        }
        self.stage = stage;
        Some(end)
    }

    /// What a ground show plays on first: the first cell a burn burns, the first mark that
    /// opens.
    pub(super) fn stage_subject(&self) -> Option<Subject> {
        let burn = self.stage.bare.first().map(|&cell| Subject::Burn(cell));
        burn.or_else(|| self.stage.settling.first().map(|&(subject, ..)| subject))
    }

    /// The sequence a mark opens in, and how far into it its settled frame takes over; fresh
    /// soil opens in none.
    fn opening(&self, kind: GroundMark) -> Option<(crate::catalog::AnimationId, u64)> {
        let art = &self.ground_art;
        match kind {
            GroundMark::Crack => Some((art.crack, art.crack_settles_ms)),
            GroundMark::Sinkhole => Some((art.sinkhole, art.sinkhole_settles_ms)),
            GroundMark::FreshSoil => None,
        }
    }

    /// The scorch lightning leaves on the plant it struck, from `start`, if the art lets that
    /// plant take one.
    pub(super) fn scorch_for(
        &self,
        struck: Option<(u8, PlantView)>,
        start: Millis,
    ) -> Option<Scorched> {
        let (cell, view) = struck?;
        let art = self.ground_art.scorches.get(&view.species)?;
        art.takes(view.stage, view.state).then_some(Scorched {
            id: view.id,
            cell,
            species: view.species,
            start,
            envelope: art.envelope,
        })
    }

    /// How much of a part of `subject` the scorch lets show at `now`: all of it, but for the
    /// struck plant's foliage, bloom and glow while the scorch plays.
    pub(super) fn veil(&self, subject: Subject, part: u8, now: Millis) -> u8 {
        let Some(scorched) = self.scorched().filter(|s| s.plant() == subject) else {
            return SHOWN;
        };
        match usize::from(part) {
            FOLIAGE => Scorched::falling_then_rising(scorched.foliage(), now),
            BLOOM | PLANT_GLOW => Scorched::falling_then_rising(scorched.bloom(), now),
            _ => SHOWN,
        }
    }

    /// The burn at `now`: its frames over each burned cell, the weeds and the fresh soil at the
    /// art's keys, and the sprinkles once the ash has cleared.
    pub(super) fn push_burn(
        &self,
        art: &Art,
        table: &ArtTable,
        start: Millis,
        palette_mix: u8,
        now: Millis,
        out: &mut Vec<KeyedDraw>,
    ) {
        if now < start {
            return;
        }
        let catalog = &art.catalog;
        let ground = &self.ground_art;
        let elapsed = now.0 - start.0;
        let lit = |value| Lit { value, palette_mix };
        let pose = anim::pose(catalog, catalog.animation(ground.burnoff), elapsed);
        if !pose.done {
            for &cell in &self.stage.bare {
                let x = self.layout.slot(cell).x;
                let draw = Draw::anchored(catalog, pose.frame, x, PLANT_GROUND_Y);
                if let Some(draw) = lit(SHOWN).apply(art, draw) {
                    // Under the light, like the sky's overlays, and by x among themselves.
                    out.push(KeyedDraw::at(
                        Place::new(Subject::Burn(cell), 0, 0),
                        (x, -1),
                        draw,
                    ));
                }
            }
            if let Some(keys) = catalog.sprite(pose.frame).burnoff {
                let weeds = permille(keys.weed_permille);
                for burnt in self.stage.burnt.iter().filter(|_| weeds > 0) {
                    let place = Place::new(Subject::Weed { cell: burnt.cell }, 0, burnt.look);
                    let value = scale(burnt.value, weeds);
                    self.push(art, table, place, burnt.piece, lit(value), now, out);
                }
            }
        }
        // The fresh soil steps in by the art's keys until its own look takes over, or, where
        // the scene stopped holding it, fades out from where the keys have it.
        let key = if pose.done {
            SHOWN
        } else {
            catalog
                .sprite(pose.frame)
                .burnoff
                .map_or(0, |keys| permille(keys.fresh_soil_permille))
        };
        let leaving = self.leaving().map(|tween| tween.value(now));
        let fresh = match leaving {
            Some(leaving) => scale(key, leaving),
            None if !pose.done && elapsed < ground.fresh_from_ms => key,
            None => 0,
        };
        if fresh > 0
            && let Some((id, lying)) = &self.stage.fresh
        {
            for &(cell, look) in lying {
                let place = Place::new(Subject::Mark { id: *id, at: cell }, 0, look);
                let piece = Piece::Ground(GroundMark::FreshSoil);
                self.push(art, table, place, piece, lit(fresh), now, out);
            }
        }
        let value = scale(self.sprinkled(start, now), leaving.unwrap_or(SHOWN));
        if value == 0 {
            return;
        }
        if let Some((_, lying)) = &self.stage.fresh {
            for &(cell, _) in lying {
                let item = (1 << 8) | u64::from(cell);
                let Some(sprite) = world::pick(self.salt, Kind::Ground, item, &ground.sprinkles)
                else {
                    continue;
                };
                let x = self.layout.slot(cell).x;
                let draw = Draw::anchored(catalog, sprite, x, PLANT_GROUND_Y);
                if let Some(draw) = lit(value).apply(art, draw) {
                    let place = Place::new(Subject::Sprinkle(cell), 0, 0);
                    out.push(KeyedDraw::at(place, (x, -1), draw));
                }
            }
        }
    }

    /// Notes, after an update, that the scene stopped holding a burn's fresh soil before its
    /// look took over from the show's keys: that look was lowered from nothing, so the show
    /// fades the soil out instead. Once the look has taken over, it fades as any look does.
    pub(super) fn note_left(&mut self, now: Millis) {
        let Some(end) = self
            .show_end()
            .filter(|&end| now < end && now < self.stage.whole)
        else {
            return;
        };
        let Some((id, lying)) = &self.stage.fresh else {
            return;
        };
        let held = lying.iter().any(|&(cell, _)| {
            self.entries
                .get(&Subject::Mark { id: *id, at: cell })
                .is_some_and(|entry| entry.looks().any(|look| look.opacity.target() == SHOWN))
        });
        if !held && self.stage.left.is_none() {
            self.stage.left = Some((now, end));
        }
    }

    /// How the fresh soil a scene stopped holding mid-burn fades out.
    fn leaving(&self) -> Option<Tween> {
        let (from, until) = self.stage.left?;
        let ms = u32::try_from(until.0.saturating_sub(from.0)).unwrap_or(u32::MAX);
        Some(Tween::new(SHOWN, 0, from, ms))
    }

    /// The sprinkles' envelope, from when the burn's ash clears.
    fn sprinkle_tweens(&self, start: Millis) -> [Tween; 2] {
        let ground = &self.ground_art;
        let from = Millis(start.0 + ground.burnoff_ms);
        let Envelope {
            enter_ms,
            hold_ms,
            exit_ms,
        } = ground.sprinkle;
        [
            Tween::new(0, SHOWN, from, u32::from(enter_ms)),
            Tween::new(
                SHOWN,
                0,
                from.plus(u32::from(enter_ms) + u32::from(hold_ms)),
                u32::from(exit_ms),
            ),
        ]
    }

    /// How much of the sprinkles shows at `now`: none without fresh soil.
    fn sprinkled(&self, start: Millis, now: Millis) -> u8 {
        if self.stage.fresh.is_none() {
            return 0;
        }
        let [rise, fall] = self.sprinkle_tweens(start);
        rise.value(now).min(fall.value(now))
    }

    /// The marks a show opens, in the frame each shows at `now`, until it settles.
    pub(super) fn push_settling(
        &self,
        art: &Art,
        table: &ArtTable,
        start: Millis,
        palette_mix: u8,
        now: Millis,
        out: &mut Vec<KeyedDraw>,
    ) {
        if now < start {
            return;
        }
        let catalog = &art.catalog;
        let elapsed = now.0 - start.0;
        for &(subject, kind, look) in &self.stage.settling {
            let Subject::Mark { id, at } = subject else {
                continue;
            };
            let Some((sequence, settles_ms)) = self.opening(kind) else {
                continue;
            };
            if elapsed >= settles_ms {
                continue;
            }
            let frame = anim::pose(catalog, catalog.animation(sequence), elapsed).frame;
            let x = self.mark_x(art, table, kind, id, at);
            let draw = Draw::anchored(catalog, frame, x, GROUND);
            let lit = Lit {
                value: SHOWN,
                palette_mix,
            };
            if let Some(draw) = lit.apply(art, draw) {
                out.push(KeyedDraw::at(
                    Place::new(subject, 0, look),
                    (x, GROUND),
                    draw,
                ));
            }
        }
    }

    /// The scorch at `now`, standing in the struck plant's cell and leaning with it.
    pub(super) fn push_scorch(
        &self,
        art: &Art,
        table: &ArtTable,
        scorched: Scorched,
        palette_mix: u8,
        now: Millis,
        out: &mut Vec<KeyedDraw>,
    ) {
        let value = Scorched::rising_then_falling(scorched.scorch(), now);
        let Some(scorch) = self.ground_art.scorches.get(&scorched.species) else {
            return;
        };
        if value == 0 {
            return;
        }
        let catalog = &art.catalog;
        let sprite = catalog.sprite(scorch.foliage);
        let x = self.layout.slot(scorched.cell).x;
        let y = plant::ground_y(scorched.species, GrowthStage::Mature);
        let layer = table.plant_layer(scorched.species);
        let (ax, ay) = (i32::from(sprite.anchor.x), i32::from(sprite.anchor.y));
        let crown = self.crown(scorched.plant(), now);
        let shear = sprite
            .shear_pivot
            .map_or(crate::frame::Shear::NONE, |pivot| {
                sway::shear(pivot.y, crown)
            });
        let draw = Draw {
            shear,
            ..Draw::in_layer(catalog, scorch.foliage, layer, x - ax, y - ay)
        };
        let lit = Lit { value, palette_mix };
        if let Some(draw) = lit.apply(art, draw) {
            let place = Place::new(
                Subject::Scorch {
                    id: scorched.id,
                    cell: scorched.cell,
                },
                FOLIAGE_PART,
                0,
            );
            out.push(KeyedDraw::placed(catalog, place, draw));
        }
    }

    /// The first moment after `now` at which a burn's draws change: its start, a frame's, the
    /// ash clearing, a step of the sprinkles.
    pub(super) fn burn_next(&self, art: &Art, start: Millis, now: Millis) -> Option<Millis> {
        if now < start {
            return Some(start);
        }
        let catalog = &art.catalog;
        let frame =
            anim::next_frame_at(catalog.animation(self.ground_art.burnoff), now.0 - start.0)
                .map(|ms| Millis(start.0 + ms));
        let sprinkles = self
            .stage
            .fresh
            .as_ref()
            .and_then(|_| next_of(&self.sprinkle_tweens(start), now));
        let leaving = self.leaving().and_then(|tween| tween.next_step(now));
        [frame, sprinkles, leaving].into_iter().flatten().min()
    }

    /// The first moment after `now` at which a mark the show opens shows its next frame, or
    /// its last frame gives way: to its settled look, or to nothing where the scene no longer
    /// holds the mark.
    pub(super) fn settling_next(&self, art: &Art, start: Millis, now: Millis) -> Option<Millis> {
        if now < start {
            return Some(start);
        }
        let catalog = &art.catalog;
        let elapsed = now.0 - start.0;
        self.stage
            .settling
            .iter()
            .filter_map(|&(_, kind, _)| {
                let (sequence, settles_ms) = self.opening(kind)?;
                let sequence = catalog.animation(sequence);
                let ms = anim::next_frame_at(sequence, elapsed)?;
                (ms <= settles_ms).then_some(Millis(start.0 + ms))
            })
            .min()
    }

    /// The first moment after `now` at which the scorch, or anything of the struck plant it
    /// hides, shows otherwise.
    pub(super) fn scorch_next(
        &self,
        art: &Art,
        table: &ArtTable,
        scorched: Scorched,
        now: Millis,
    ) -> Option<Millis> {
        let mut next = None;
        let mut take = |moment: Option<Millis>| {
            if let Some(moment) = moment {
                next = Some(next.map_or(moment, |earliest: Millis| earliest.min(moment)));
            }
        };
        let scorch = scorched.scorch();
        take(first_seen(
            now,
            |at| Scorched::rising_then_falling(scorch, at),
            |at| next_of(&scorch, at),
        ));
        let subject = scorched.plant();
        let Some(entry) = self.entries.get(&subject) else {
            return next;
        };
        for part in [FOLIAGE, BLOOM, PLANT_GLOW] {
            let veil = if part == FOLIAGE {
                scorched.foliage()
            } else {
                scorched.bloom()
            };
            let part_u8 = u8::try_from(part).unwrap_or(0);
            for look in &entry.parts[part] {
                let Some(sprite) = look.piece.sprite(table, self.salt, subject, part_u8) else {
                    continue;
                };
                let seen = |at: Millis| {
                    let value = scale(
                        look.opacity.value(at),
                        Scorched::falling_then_rising(veil, at),
                    );
                    self.drawn(art, sprite, value, at)
                };
                let tweens = [look.opacity, veil[0], veil[1]];
                take(first_seen(now, seen, |at| next_of(&tweens, at)));
            }
        }
        next
    }

    /// The opacity `sprite` is drawn at, at a look's `value`, by the palette at `now`.
    fn drawn(&self, art: &Art, sprite: SpriteId, value: u8, now: Millis) -> u8 {
        let lit = Lit {
            value,
            palette_mix: self.mix_at(now),
        };
        lit.apply(art, Draw::new(&art.catalog, sprite, 0, 0))
            .map_or(0, |draw| draw.opacity)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use terrarium_truth::Tick;
    use terrarium_truth::scene::{EventKind, FossilForm, FossilView, GroundMarkView, PlantState};

    use super::super::OverlayInput;
    use super::*;
    use crate::clock::LocalClock;
    use crate::frame::Viewport;
    use crate::motion::{turn, turn_start};
    use crate::preset;
    use crate::synthetic;

    fn art() -> &'static Art<'static> {
        Art::embedded()
    }

    fn table() -> &'static ArtTable {
        ArtTable::embedded()
    }

    fn overlay(night: bool) -> OverlayInput {
        let clock = if night {
            LocalClock::midnight(preset::DATE)
        } else {
            LocalClock::noon(preset::DATE)
        };
        OverlayInput::new(Vec::new(), clock)
    }

    fn poor() -> SceneInput {
        preset::scene("struggling-poor-soil").unwrap()
    }

    fn thriving() -> SceneInput {
        preset::scene("thriving").unwrap()
    }

    /// `before` with `kind` on it as truth brings it, from `side`.
    fn with(before: &SceneInput, kind: EventKind, side: Side) -> SceneInput {
        let mut after = before.clone();
        synthetic::set_event(&mut after, kind, Tick::new(5));
        after.event.as_mut().unwrap().side = side;
        after
    }

    /// A composer settled on `before` in `viewport`, then given `after`; and when.
    fn played_in(
        viewport: Viewport,
        before: &SceneInput,
        after: &SceneInput,
    ) -> (Composer, Millis) {
        let mut composer = Composer::new(table(), preset::SALT);
        composer.set_viewport(viewport);
        composer.update(before, &overlay(false), Millis(0));
        let now = composer.settled_at();
        composer.update(after, &overlay(false), now);
        (composer, now)
    }

    fn played(before: &SceneInput, after: &SceneInput) -> (Composer, Millis) {
        played_in(Viewport::SCENE, before, after)
    }

    fn of(composer: &Composer, now: Millis, wanted: impl Fn(Subject) -> bool) -> Vec<KeyedDraw> {
        composer
            .draws(art(), table(), 0, now)
            .into_iter()
            .filter(|keyed| wanted(keyed.key.subject))
            .collect()
    }

    fn name(keyed: &KeyedDraw) -> &'static str {
        &art().catalog.sprite(keyed.draw.sprite).id
    }

    fn opacities(draws: &[KeyedDraw]) -> BTreeSet<u8> {
        draws.iter().map(|keyed| keyed.draw.opacity).collect()
    }

    fn after(now: Millis, ms: u64) -> Millis {
        Millis(now.0 + ms)
    }

    #[test]
    fn burn_off_steps_the_weeds_out_and_the_fresh_soil_in_at_its_frame_starts() {
        let (composer, now) = played(&poor(), &with(&poor(), EventKind::ClearingBurn, Side::Left));
        let weeds = |ms| {
            of(&composer, after(now, ms), |s| {
                matches!(s, Subject::Weed { .. })
            })
        };
        let soil = |ms| {
            of(&composer, after(now, ms), |s| {
                matches!(s, Subject::Mark { .. })
            })
        };
        let burns = |ms| of(&composer, after(now, ms), |s| matches!(s, Subject::Burn(_)));
        let before = weeds(0);
        assert!(!before.is_empty());
        let keys = |draws: &[KeyedDraw]| -> Vec<_> { draws.iter().map(|k| k.key).collect() };
        let cells: BTreeSet<u8> = before.iter().filter_map(|k| k.key.subject.cell()).collect();
        assert!(
            burns(0).is_empty() && soil(0).is_empty(),
            "nothing at the update"
        );
        // Each frame from its start: the weeds at 1, 0.8, 0.35 and 0 of themselves, under the
        // keys they were drawn by, and the fresh soil at 0, 0, 0.4 and 1.
        for (ms, frame, weed, fresh) in [
            (1, "f01", Some(255), None),
            (180, "f01", Some(255), None),
            (181, "f02", Some(204), None),
            (401, "f03", Some(89), Some(102)),
            (641, "f04", None, Some(255)),
        ] {
            let burning = burns(ms);
            assert_eq!(burning.len(), cells.len(), "{ms}");
            assert!(burning.iter().all(|k| name(k).ends_with(frame)), "{ms}");
            let drawn = weeds(ms);
            match weed {
                Some(weed) => {
                    assert_eq!(keys(&drawn), keys(&before), "{ms}");
                    assert_eq!(opacities(&drawn), BTreeSet::from([weed]), "{ms}");
                }
                None => assert!(drawn.is_empty(), "{ms}"),
            }
            let lying = soil(ms);
            match fresh {
                Some(fresh) => {
                    assert_eq!(opacities(&lying), BTreeSet::from([fresh]), "{ms}");
                    let under: BTreeSet<u8> = lying
                        .iter()
                        .filter_map(|k| match k.key.subject {
                            Subject::Mark { at, .. } => Some(at),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(under, cells, "fresh soil on the burned cells");
                }
                None => assert!(lying.is_empty(), "{ms}"),
            }
        }
        // What plays is named by a cell it burns.
        let playing = composer.show_depicted(after(now, 1)).map(|d| d.subject);
        assert!(matches!(playing, Some(Subject::Burn(cell)) if cells.contains(&cell)));
        // The ash clears; the sprinkles come over 900 ms, hold and go over 1200; the soil stays.
        assert!(burns(941).is_empty());
        let sprinkles = |ms| {
            of(&composer, after(now, ms), |s| {
                matches!(s, Subject::Sprinkle(_))
            })
        };
        assert!(sprinkles(941).is_empty());
        let rising = opacities(&sprinkles(941 + 450));
        assert!(rising.iter().all(|&o| o > 0 && o < 255), "{rising:?}");
        assert_eq!(
            opacities(&sprinkles(941 + 900 + 300)),
            BTreeSet::from([255])
        );
        assert_eq!(sprinkles(941 + 900).len(), cells.len());
        assert!(sprinkles(941 + 2700).is_empty());
        assert_eq!(composer.show_end(), Some(after(now, 1 + 940 + 2700)));
        let settled = composer.settled_at();
        assert_eq!(
            opacities(&soil(settled.0 - now.0 + 60_000)),
            BTreeSet::from([255])
        );
    }

    #[test]
    fn fresh_soil_the_scene_drops_mid_burn_fades_out_rather_than_vanishing() {
        let burnt = with(&poor(), EventKind::ClearingBurn, Side::Left);
        let (mut composer, now) = played(&poor(), &burnt);
        let soil = |composer: &Composer, ms: u64| -> u8 {
            of(composer, after(now, ms), |s| {
                matches!(s, Subject::Mark { .. })
            })
            .iter()
            .map(|k| k.draw.opacity)
            .max()
            .unwrap_or(0)
        };
        // Truth drops it while it stands at 0.4 of itself.
        let mut dropped = burnt.clone();
        dropped.ground_marks.clear();
        let frame = composer.draws(art(), table(), 0, after(now, 500));
        composer.update(&dropped, &overlay(false), after(now, 500));
        assert_eq!(composer.draws(art(), table(), 0, after(now, 500)), frame);
        let mut last = soil(&composer, 500);
        assert_eq!(last, 102);
        let mut rose = false;
        for ms in (501..4000).step_by(7) {
            let next = soil(&composer, ms);
            // No step wider than the art's own, from 0.4 to 1.
            assert!(next.abs_diff(last) <= 153, "{ms}: {last} to {next}");
            rose |= next > last;
            last = next;
        }
        assert!(rose, "it still steps in by the keys");
        assert_eq!(soil(&composer, composer.settled_at().0 - now.0), 0);
    }

    #[test]
    fn a_burn_with_no_weeds_drawn_plays_nothing_and_its_soil_fades_in() {
        let (composer, now) = played(
            &thriving(),
            &with(&thriving(), EventKind::ClearingBurn, Side::Left),
        );
        assert_eq!(composer.show, None);
        assert!(
            of(&composer, after(now, 300), |s| matches!(
                s,
                Subject::Burn(_)
            ))
            .is_empty()
        );
        let soil = |ms| {
            of(&composer, after(now, ms), |s| {
                matches!(s, Subject::Mark { .. })
            })
        };
        let lying: Vec<u8> = soil(5000)
            .iter()
            .filter_map(|k| match k.key.subject {
                Subject::Mark { at, .. } => Some(at),
                _ => None,
            })
            .collect();
        assert_eq!(lying, weed_walk().take(FRESH_FALLBACK).collect::<Vec<_>>());
        let fading = opacities(&soil(1500));
        assert!(fading.iter().all(|&o| o > 0 && o < 255), "{fading:?}");
    }

    #[test]
    fn a_burned_cell_stays_bare_until_the_show_is_over() {
        // Few plants, so filler takes the cells the weeds leave.
        let mut sparse = preset::bare(terrarium_truth::PresentationState::Struggling, Vec::new());
        sparse.weeds = 4;
        let (composer, now) = played(&sparse, &with(&sparse, EventKind::ClearingBurn, Side::Left));
        let burned = composer.stage.bare.clone();
        let end = composer.show_end().unwrap();
        let in_burned = |at: Millis| {
            of(&composer, at, |s| match s {
                Subject::Fill { cell } | Subject::Weed { cell } => burned.contains(&cell),
                _ => false,
            })
            .into_iter()
            .filter(|keyed| keyed.key.look == 0 || !name(keyed).starts_with("tmp.weed."))
            .count()
        };
        // The burning weeds are the show's; nothing else stands in their cells until it ends.
        for ms in [2000, 3000, end.0 - now.0 - 1] {
            let filler = of(&composer, after(now, ms), |s| match s {
                Subject::Fill { cell } => burned.contains(&cell),
                _ => false,
            });
            assert!(filler.is_empty(), "{ms}");
        }
        assert!(
            in_burned(composer.settled_at()) > 0,
            "filler takes a burned cell once the show is over"
        );
    }

    /// The thriving terrarium struck by lightning on its first mature plant; the plant's id.
    fn struck(state: PlantState, bloom: bool) -> (Composer, Millis, Subject) {
        let mut before = thriving();
        let plant = before
            .plants
            .iter_mut()
            .find(|p| p.stage == GrowthStage::Mature)
            .unwrap();
        plant.state = state;
        plant.bloom = bloom;
        let id = plant.id;
        // Struck as truth names it, whether or not the art lets it take a scorch.
        let mut after = with(&before, EventKind::Lightning, Side::Left);
        for p in &mut after.plants {
            p.scorched = p.id == id;
        }
        let (composer, now) = played(&before, &after);
        let cell = composer.placement.plants[&id].cell;
        (composer, now, Subject::Plant { id, cell })
    }

    #[test]
    fn the_scorch_follows_the_bolt_and_hands_back_the_plant_s_own_foliage() {
        let (composer, now, plant) = struck(PlantState::Thriving, false);
        let scorch = |ms| {
            of(&composer, after(now, ms), |s| {
                matches!(s, Subject::Scorch { .. })
            })
        };
        let foliage = |ms| {
            of(&composer, after(now, ms), |s| s == plant)
                .into_iter()
                .filter(|k| k.key.part == FOLIAGE_PART)
                .map(|k| k.draw.opacity)
                .max()
                .unwrap_or(0)
        };
        // The bolt is over 333 ms in; the scorch enters over 600, holds 900, exits over 1200.
        let bolt = 1 + 332;
        assert!(scorch(bolt).is_empty());
        let entering = scorch(bolt + 300)[0].draw.opacity;
        assert!(entering > 0 && entering < 255);
        let held = scorch(bolt + 600);
        assert_eq!(held[0].draw.opacity, 255);
        assert!(name(&held[0]).starts_with("tmp.event.scorch."));
        assert_eq!(foliage(bolt + 600), 255, "the foliage holds under it");
        assert_eq!(foliage(bolt + 1500), 0, "and goes behind it");
        assert_eq!(foliage(bolt + 2100), 255, "then comes back under it");
        assert_eq!(scorch(bolt + 2100)[0].draw.opacity, 255);
        assert!(scorch(bolt + 2700).is_empty());
        assert_eq!(composer.show_end(), Some(after(now, bolt + 2700)));
        // In its plant's layer, keyed at its plant's ground point, over the plant.
        let plant_key = of(&composer, after(now, 1), |s| s == plant)
            .into_iter()
            .find(|k| k.key.part == FOLIAGE_PART)
            .unwrap()
            .key;
        let key = held[0].key;
        assert_eq!(
            (key.layer, key.y, key.x),
            (plant_key.layer, plant_key.y, plant_key.x)
        );
        assert!(key > plant_key);
    }

    #[test]
    fn a_scorch_gives_back_the_foliage_truth_has_by_then_never_a_healed_one() {
        let (mut composer, now, plant) = struck(PlantState::Thriving, false);
        let Subject::Plant { id, .. } = plant else {
            unreachable!()
        };
        // Mid-hold, truth has the plant wilting: that is what the scorch hands back.
        let mut wilting = with(&thriving(), EventKind::Lightning, Side::Left);
        for p in &mut wilting.plants {
            if p.id == id {
                p.state = PlantState::Wilting;
            }
        }
        composer.update(&wilting, &overlay(false), after(now, 1000));
        let settled = composer.settled_at();
        let shown: Vec<Piece> = composer
            .depicted(art(), table(), settled)
            .into_iter()
            .filter(|d| d.subject == plant && usize::from(d.part) == FOLIAGE)
            .map(|d| d.piece)
            .collect();
        assert!(
            matches!(
                shown[..],
                [Piece::Foliage {
                    state: PlantState::Wilting,
                    ..
                }]
            ),
            "{shown:?}"
        );
    }

    #[test]
    fn only_a_plant_the_art_lets_take_one_is_scorched() {
        let (composer, now, _) = struck(PlantState::Dead, false);
        assert!(composer.scorched().is_none());
        for ms in [400, 1000, 2000] {
            assert!(
                of(&composer, after(now, ms), |s| matches!(
                    s,
                    Subject::Scorch { .. }
                ))
                .is_empty()
            );
        }
        assert_eq!(composer.show_end(), Some(after(now, 1 + 332)));
        let (dormant, ..) = struck(PlantState::Dormant, false);
        assert!(dormant.scorched().is_some());
    }

    #[test]
    fn the_bloom_and_the_glow_hide_while_the_scorch_shows() {
        let (composer, now, plant) = struck(PlantState::Thriving, true);
        let part = |ms: u64, part: usize, mix: u8| {
            composer
                .draws(art(), table(), mix, after(now, ms))
                .into_iter()
                .filter(|k| k.key.subject == plant && usize::from(k.key.part) == part)
                .map(|k| k.draw.opacity)
                .max()
                .unwrap_or(0)
        };
        let bolt = 1 + 332;
        assert!(part(bolt, BLOOM, 0) > 0);
        for ms in [bolt + 600, bolt + 1500, bolt + 2100] {
            assert_eq!(part(ms, BLOOM, 0), 0, "{ms}");
        }
        assert!(part(bolt + 2700, BLOOM, 0) > 0);
        assert!(part(bolt, PLANT_GLOW, 255) > 0);
        assert_eq!(part(bolt + 1500, PLANT_GLOW, 255), 0);
    }

    #[test]
    fn flowering_opens_the_blooms_one_by_one_from_its_side() {
        let base = thriving();
        for side in [Side::Left, Side::Right] {
            let (composer, now) = played(&base, &with(&base, EventKind::Flowering, side));
            let opened = |subject: Subject| {
                (0..=70_000).step_by(100).find(|&ms| {
                    of(&composer, after(now, ms), |s| s == subject)
                        .iter()
                        .any(|k| usize::from(k.key.part) == BLOOM && k.draw.opacity > 0)
                })
            };
            let new: Vec<(i32, u64)> = base
                .plants
                .iter()
                .filter(|p| !p.bloom && p.stage == GrowthStage::Mature)
                .map(|p| {
                    let cell = composer.placement.plants[&p.id].cell;
                    let subject = Subject::Plant { id: p.id, cell };
                    (slots::centre(usize::from(cell)), opened(subject).unwrap())
                })
                .collect();
            assert!(new.len() > 2);
            let mut order = new.clone();
            order.sort_unstable();
            if side == Side::Right {
                order.reverse();
            }
            let times: Vec<u64> = order.iter().map(|&(_, at)| at).collect();
            assert!(
                times.windows(2).all(|pair| pair[0] < pair[1]),
                "{side:?}: {times:?}"
            );
            assert!(
                times[0] <= 100 && *times.last().unwrap() < 60_000,
                "{times:?}"
            );
        }
    }

    /// The marks drawn at `at`, by subject, with the sprite each shows.
    fn marks_at(composer: &Composer, at: Millis) -> Vec<(Subject, &'static str, u8)> {
        of(composer, at, |s| matches!(s, Subject::Mark { .. }))
            .iter()
            .map(|k| (k.key.subject, name(k), k.draw.opacity))
            .collect()
    }

    #[test]
    fn an_earthquake_opens_its_crack_in_three_frames_into_the_settled_one_that_stays() {
        let (composer, now) = played(&poor(), &with(&poor(), EventKind::Earthquake, Side::Left));
        assert!(marks_at(&composer, now).is_empty());
        let mut seen = Vec::new();
        for ms in [1, 167, 333, 499] {
            let marks = marks_at(&composer, after(now, ms));
            assert_eq!(marks.len(), 1, "{ms}");
            assert_eq!(marks[0].2, 255);
            seen.push(marks[0]);
        }
        let names: Vec<&str> = seen.iter().map(|m| m.1).collect();
        assert_eq!(
            names,
            [
                "tmp.ground.crack.f01",
                "tmp.ground.crack.f02",
                "tmp.ground.crack.f03",
                "tmp.ground.crack.settled"
            ]
        );
        assert!(seen.iter().all(|m| m.0 == seen[0].0), "one key throughout");
        let playing = composer.show_depicted(after(now, 1)).map(|d| d.subject);
        assert_eq!(
            playing,
            Some(seen[0].0),
            "what plays is named by the mark it opens"
        );
        let later = marks_at(&composer, after(composer.settled_at(), 600_000));
        assert_eq!(later, [(seen[0].0, "tmp.ground.crack.settled", 255)]);
        assert_eq!(composer.show_end(), Some(after(now, 1 + 498)));
    }

    #[test]
    fn residents_shelter_through_the_one_turn_an_earthquake_shakes() {
        let base = poor();
        let quake = with(&base, EventKind::Earthquake, Side::Left);
        let (mut composer, now) = played(&base, &quake);
        // Truth carries the instance on through the hour: the turn it shook stays the one.
        let shaken = turn(now) + 1;
        composer.update(&quake, &overlay(false), turn_start(shaken).plus(1000));
        let routines = |w: u64| -> Vec<String> {
            composer
                .turn_plans(art(), table(), w)
                .iter()
                .filter_map(|plan| plan.segments.last())
                .filter_map(|s| art().catalog.animation(s.routine).keys.routine.clone())
                .collect()
        };
        let hiding = |r: &String| matches!(r.as_str(), "hide" | "withdraw" | "curl" | "retreat");
        let adults = base
            .residents
            .iter()
            .filter(|r| r.stage == terrarium_truth::scene::LifeStage::Adult)
            .count();
        assert!(adults > 0);
        assert_eq!(
            routines(shaken).iter().filter(|r| hiding(r)).count(),
            adults
        );
        assert!(
            !routines(shaken + 1).iter().any(hiding),
            "out again the turn after"
        );
        assert!(composer.settled_at() >= turn_start(shaken + 1));
    }

    #[test]
    fn a_sinkhole_opens_over_its_fossil_as_near_the_plant_it_swallowed_as_a_spot_allows() {
        let base = thriving();
        let sunk = with(&base, EventKind::Sinkhole, Side::Left);
        let gone = base
            .plants
            .iter()
            .find(|p| !sunk.plants.iter().any(|q| q.id == p.id))
            .unwrap();
        let mut composer = Composer::new(table(), preset::SALT);
        composer.update(&base, &overlay(false), Millis(0));
        let now = composer.settled_at();
        let cell = composer.placement.plants[&gone.id].cell;
        composer.update(&sunk, &overlay(false), now);
        let fossil = sunk.event.unwrap().fossil.unwrap();
        let spot = composer.fossil_spots()[&fossil];
        let x = slots::centre(usize::from(cell));
        let nearest = (0..world::FOSSILS)
            .min_by_key(|&s| (world::fossil_centre(s) - x).abs())
            .unwrap();
        assert_eq!(usize::from(spot), nearest);
        let names: Vec<&str> = [1, 167, 333, 499]
            .iter()
            .map(|&ms| marks_at(&composer, after(now, ms))[0].1)
            .collect();
        assert_eq!(
            names,
            [
                "tmp.ground.sinkhole.f01",
                "tmp.ground.sinkhole.f02",
                "tmp.ground.sinkhole.f03",
                "tmp.ground.sinkhole.f04"
            ]
        );
        // Over its fossil: its anchor stands on the fossil spot's middle.
        let hole = of(&composer, after(now, 499), |s| {
            matches!(s, Subject::Mark { .. })
        })[0];
        assert_eq!(
            hole.key.subject,
            Subject::Mark {
                id: sunk.ground_marks[0].id,
                at: spot
            }
        );
        let middle = composer.mark_x(art(), table(), GroundMark::Sinkhole, 0, spot);
        assert_eq!(hole.key.x, middle);
        assert_eq!(middle, composer.world.fossil(spot) + 4);
    }

    #[test]
    fn a_hole_stands_over_its_fossil_though_a_fossil_left_that_spot_in_the_same_update() {
        let base = thriving();
        let sunk = with(&base, EventKind::Sinkhole, Side::Left);
        let gone = base
            .plants
            .iter()
            .find(|p| !sunk.plants.iter().any(|q| q.id == p.id))
            .unwrap()
            .id;
        let fresh = || {
            let mut composer = Composer::new(table(), preset::SALT);
            composer.update(&base, &overlay(false), Millis(0));
            composer
        };
        let cell = fresh().placement.plants[&gone].cell;
        let x = slots::centre(usize::from(cell));
        let nearest = (0..world::FOSSILS)
            .min_by_key(|&s| (world::fossil_centre(s) - x).abs())
            .unwrap();
        // A fossil the composer puts at that spot, which leaves as the hole opens.
        let leaving = (100..200)
            .find(|&id| {
                let mut with_fossil = base.clone();
                with_fossil.fossils.push(FossilView {
                    id,
                    form: FossilForm::Shell,
                });
                let mut composer = Composer::new(table(), preset::SALT);
                composer.update(&with_fossil, &overlay(false), Millis(0));
                usize::from(composer.fossil_spots()[&id]) == nearest
            })
            .expect("an id the spot goes to");
        let mut before = base.clone();
        before.fossils.push(FossilView {
            id: leaving,
            form: FossilForm::Shell,
        });
        let (composer, _) = played(&before, &sunk);
        let fossil = sunk.event.unwrap().fossil.unwrap();
        assert_eq!(composer.sunk[&fossil], composer.fossil_spots()[&fossil]);
        assert_eq!(usize::from(composer.sunk[&fossil]), nearest);
    }

    #[test]
    fn a_dead_plant_leaving_with_the_swallowed_one_is_not_taken_for_it() {
        let mut base = thriving();
        let sunk = with(&base, EventKind::Sinkhole, Side::Left);
        let swallowed = base
            .plants
            .iter()
            .find(|p| !sunk.plants.iter().any(|q| q.id == p.id))
            .unwrap()
            .id;
        // A dead plant with a lower id leaves at the same update.
        base.plants.push(PlantView {
            id: 1,
            species: PlantSpecies::Thistle,
            stage: GrowthStage::Mature,
            state: PlantState::Dead,
            bloom: false,
            scorched: false,
            order: 99,
        });
        let mut composer = Composer::new(table(), preset::SALT);
        composer.update(&base, &overlay(false), Millis(0));
        assert!(composer.placement.plants.contains_key(&1));
        assert_eq!(
            composer.swallowed(&sunk),
            Some(composer.placement.plants[&swallowed].cell)
        );
    }

    #[test]
    fn an_earthquake_opens_the_sinkholes_of_the_fossils_it_takes_where_they_lay() {
        let mut base = poor();
        let taken = base.fossils[0];
        let mut quake = with(&base, EventKind::Earthquake, Side::Left);
        quake.fossils.retain(|f| f.id != taken.id);
        quake.ground_marks.push(GroundMarkView {
            id: 77,
            kind: GroundMark::Sinkhole,
            fossil: Some(taken.id),
        });
        assert_eq!(quake.validate(), Ok(()));
        base.event = None;
        let (composer, now) = played(&base, &quake);
        let was = {
            let mut fresh = Composer::new(table(), preset::SALT);
            fresh.update(&base, &overlay(false), Millis(0));
            fresh.fossil_spots()[&taken.id]
        };
        let marks = marks_at(&composer, after(now, 1));
        let names: BTreeSet<&str> = marks.iter().map(|m| m.1).collect();
        assert_eq!(
            names,
            BTreeSet::from(["tmp.ground.crack.f01", "tmp.ground.sinkhole.f01"])
        );
        assert!(
            marks
                .iter()
                .any(|m| m.0 == Subject::Mark { id: 77, at: was })
        );
        // The fossil itself leaves from under it.
        let fossil = |at| {
            of(
                &composer,
                at,
                |s| matches!(s, Subject::Fossil { id, .. } if id == taken.id),
            )
        };
        assert!(!fossil(after(now, 1)).is_empty());
        assert!(fossil(composer.settled_at()).is_empty());
    }

    #[test]
    fn a_mark_with_no_show_fades_in_and_out_as_truth_keeps_it() {
        let base = thriving();
        let mut cracked = base.clone();
        cracked.ground_marks.push(GroundMarkView {
            id: 9,
            kind: GroundMark::Crack,
            fossil: None,
        });
        let (mut composer, now) = played(&base, &cracked);
        let fading = marks_at(&composer, after(now, 1500));
        assert!(fading[0].2 > 0 && fading[0].2 < 255, "{fading:?}");
        assert_eq!(fading[0].1, "tmp.ground.crack.settled");
        let settled = composer.settled_at();
        assert_eq!(marks_at(&composer, settled)[0].2, 255);
        composer.update(&base, &overlay(false), settled);
        let leaving = marks_at(&composer, after(settled, 1500));
        assert!(leaving[0].2 > 0 && leaving[0].2 < 255);
        assert!(marks_at(&composer, composer.settled_at()).is_empty());
    }

    #[test]
    fn fresh_soil_keeps_its_cells_and_a_new_composer_lays_it_where_weeds_go_first() {
        let burnt = with(&poor(), EventKind::ClearingBurn, Side::Left);
        let (mut composer, now) = played(&poor(), &burnt);
        let cells = |composer: &Composer, at| -> BTreeSet<u8> {
            of(composer, at, |s| matches!(s, Subject::Mark { .. }))
                .iter()
                .filter_map(|k| match k.key.subject {
                    Subject::Mark { at, .. } => Some(at),
                    _ => None,
                })
                .collect()
        };
        let lying = cells(&composer, composer.settled_at());
        composer.update(&burnt, &overlay(false), composer.settled_at());
        assert_eq!(cells(&composer, composer.settled_at()), lying);
        assert!(lying.len() > FRESH_FALLBACK);
        let mut restarted = Composer::new(table(), preset::SALT);
        restarted.update(&burnt, &overlay(false), now);
        let fallback: BTreeSet<u8> = weed_walk().take(FRESH_FALLBACK).collect();
        assert_eq!(cells(&restarted, restarted.settled_at()), fallback);
    }

    #[test]
    fn every_ground_choreography_s_update_leaves_the_frame_as_it_was() {
        for columns in [640, 840] {
            for (base, kind) in [
                (poor(), EventKind::ClearingBurn),
                (thriving(), EventKind::Lightning),
                (thriving(), EventKind::Flowering),
                (poor(), EventKind::Earthquake),
                (thriving(), EventKind::Sinkhole),
            ] {
                let mut composer = Composer::new(table(), preset::SALT);
                composer.set_viewport(Viewport::new(columns));
                composer.update(&base, &overlay(false), Millis(0));
                let now = composer.settled_at();
                let frame = composer.draws(art(), table(), 0, now);
                composer.update(&with(&base, kind, Side::Left), &overlay(false), now);
                assert_eq!(
                    composer.draws(art(), table(), 0, now),
                    frame,
                    "{kind:?} {columns}"
                );
            }
        }
    }

    #[test]
    fn a_burn_in_a_wide_viewport_stands_where_its_weeds_stood() {
        let viewport = Viewport::new(840);
        let (composer, now) = played_in(
            viewport,
            &poor(),
            &with(&poor(), EventKind::ClearingBurn, Side::Left),
        );
        let weeds = |at| of(&composer, at, |s| matches!(s, Subject::Weed { .. }));
        let at_cue: Vec<(i32, i32)> = weeds(now).iter().map(|k| (k.draw.x, k.draw.y)).collect();
        let burning: Vec<(i32, i32)> = weeds(after(now, 200))
            .iter()
            .map(|k| (k.draw.x, k.draw.y))
            .collect();
        assert_eq!(burning, at_cue);
        for burn in of(&composer, after(now, 200), |s| {
            matches!(s, Subject::Burn(_))
        }) {
            let Subject::Burn(cell) = burn.key.subject else {
                unreachable!()
            };
            let anchor = i32::from(art().catalog.sprite(burn.draw.sprite).anchor.x);
            let slot = composer.layout.slot(cell).x;
            assert_eq!(burn.draw.x + anchor, slot);
            assert_eq!(slot, viewport.spread(composer.layout.scene_x(cell)));
        }
    }

    #[test]
    fn every_ground_choreography_ends_by_settled_at() {
        for (base, kind) in [
            (poor(), EventKind::ClearingBurn),
            (thriving(), EventKind::Lightning),
            (thriving(), EventKind::Flowering),
            (poor(), EventKind::Earthquake),
            (thriving(), EventKind::Sinkhole),
        ] {
            let (composer, _) = played(&base, &with(&base, kind, Side::Right));
            let settled = composer.settled_at();
            assert!(
                composer.show_end().is_some_and(|end| end <= settled),
                "{kind:?}"
            );
            let pieces = of(&composer, settled, |s| {
                matches!(
                    s,
                    Subject::Scorch { .. } | Subject::Burn(_) | Subject::Sprinkle(_)
                )
            });
            assert!(pieces.is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn a_hole_s_spot_is_kept_while_its_mark_lasts_and_no_longer() {
        let mut base = thriving();
        base.fossils.push(FossilView {
            id: 40,
            form: FossilForm::Shell,
        });
        let sunk = with(&base, EventKind::Sinkhole, Side::Left);
        let (mut composer, _) = played(&base, &sunk);
        let fossil = sunk.event.unwrap().fossil.unwrap();
        assert_eq!(composer.sunk.keys().copied().collect::<Vec<_>>(), [fossil]);
        // Rain after a drought washes the hole away.
        let washed = with(&sunk, EventKind::RainAfterDrought, Side::Left);
        composer.update(&washed, &overlay(false), composer.settled_at());
        assert!(composer.sunk.is_empty());
    }
}
