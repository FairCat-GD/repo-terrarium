//! The residents as drawn: each arriving, resting and leaving by its own fade, the routines a
//! motion turn plans for it, and its frames and lantern.

use std::collections::BTreeMap;

use terrarium_truth::scene::{EventKind, GrowthStage, PlantState, ResidentView, SceneInput};

use crate::anim;
use crate::blend::scale;
use crate::catalog::Art;
use crate::frame::{Draw, PLANT_GROUND_Y};
use crate::motion::{Shaken, Snapshot, turn};
use crate::order::{KeyedDraw, Place, Subject};
use crate::place::assign_spots;
use crate::resident::{self, Enter, Resident, Segment, TRANSITION_MS, TurnPlan, Way};
use crate::slots::{self};
use crate::tween::{Millis, Tween};
use crate::variants::ArtTable;
use crate::world::{self};

use super::{Composer, Earliest, FRAME_WIDTH, Lit, SHOWN};

/// A resident as drawn: how visible it is, what it was when last in the scene, and, through
/// the turn it arrived in, what it arrived as and the scene it arrived with, which is how it
/// rests until the next turn reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Shown {
    pub(super) opacity: Tween,
    pub(super) last: Resident,
    pub(super) arrival: Option<(u64, Resident, Snapshot)>,
}

/// One look of a resident: its frame, facing and standing where its segment has it, and
/// its lantern while the segment is a signal.
#[expect(
    clippy::too_many_arguments,
    reason = "a creature's look needs where it is, what it plays and how lit, each its own"
)]
pub(super) fn push_creature(
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
pub(super) fn poses(plan: &TurnPlan, now: Millis) -> Vec<(u16, usize, Segment, u8)> {
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
pub(super) fn plan_next(
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
pub(super) fn fades(segment: &Segment) -> Option<(Tween, Tween)> {
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

impl Composer {
    /// Gives each resident its stretch and home, fades in the arrivals and fades out those
    /// that left; returns the residents drawn, by id.
    pub(super) fn update_residents(&mut self, input: &SceneInput, now: Millis) -> Vec<Resident> {
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
    pub(super) fn arrivals(&self) -> resident::Arrivals<'_> {
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
    pub(super) fn note_arrivals(&mut self, snapshot: &Snapshot, running: u64) {
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
    pub(super) fn home(&self, view: ResidentView, spot: u8) -> Resident {
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

    /// The earthquake the scene carries, and the turn it shakes: the one after the update that
    /// first carried it, kept while later updates carry it on.
    pub(super) fn shaken(&self, input: &SceneInput, now: Millis) -> Option<Shaken> {
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
    pub(super) fn resident_plans(
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

    /// Whether any resident is drawn otherwise at `at` than just before it.
    pub(super) fn residents_change_at(&self, art: &Art, table: &ArtTable, at: Millis) -> bool {
        let residents = |now: Millis| {
            let mut out = Vec::new();
            self.push_residents(art, table, self.mix_at(now), now, &mut out);
            out
        };
        residents(Millis(at.0.saturating_sub(1))) != residents(at)
    }

    /// The residents drawn at `now`, by `(id, spot)`, with their turn's plan, the record it
    /// reads and how visible they are: shown above nothing and planned in the turn. Drawing
    /// and [`Composer::depicted`] both read it, so neither counts one the other leaves out.
    pub(super) fn drawn_residents(
        &self,
        art: &Art,
        table: &ArtTable,
        now: Millis,
    ) -> Vec<((u32, u8), TurnPlan, Resident, u8)> {
        let mut plans = self.resident_plans(art, table, turn(now));
        self.residents
            .iter()
            .filter_map(|(&key, shown)| {
                let visible = shown.opacity.value(now);
                let (plan, record) = plans.remove(&key).filter(|_| visible > 0)?;
                Some((key, plan, record, visible))
            })
            .collect()
    }

    /// Every resident at `now`: its turn's pose, a cover or a move over the one before while
    /// it plays, and a firefly's lantern while it signals.
    pub(super) fn push_residents(
        &self,
        art: &Art,
        table: &ArtTable,
        palette_mix: u8,
        now: Millis,
        out: &mut Vec<KeyedDraw>,
    ) {
        let before = self.eras.at(turn(now).saturating_sub(1));
        let arrivals = self.arrivals();
        for ((id, spot), plan, record, visible) in self.drawn_residents(art, table, now) {
            let (plan, record) = (&plan, &record);
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
                    Place::new(subject, 0, look),
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
}
