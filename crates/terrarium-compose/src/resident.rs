//! Residents: where each lives, and what it does in each turn.
//!
//! A resident lives about a home in one of six stretches of the strip, beside a vigorous plant
//! when one grows there, where it feeds. Every turn begins and ends with it resting at home,
//! so where it stands is a pure function of the turn and never of what came before. In a turn
//! it is chosen to move, it walks out and back (a firefly may fly, a springtail hop), resting
//! between; otherwise it rests, breathing in its idle, feed or sleep loop as authored.
//!
//! At most three residents move in a turn, and at most one of them flies or signals: the art's
//! budget. Which three rotates turn by turn; each one's energy and the scene's state decide
//! whether it uses its turn. Night and day, storms and lifecycle steps change what a resident
//! rests in; the change plays at the start of the next turn, through the art's own hand-off
//! where it has one (a hatch, a withdrawal into a shell) and otherwise as a cover: the new pose
//! rises over the old, then the old falls. Nothing here reads the salt.

use std::collections::BTreeMap;

use terrarium_truth::PresentationState;
use terrarium_truth::scene::{Energy, LifeStage, ResidentSpecies, ResidentView, Weather};

use crate::anim::{cycle_ms, length_ms};
use crate::catalog::{AnimationId, Catalog, Next};
use crate::motion::{Snapshot, motion_hash, motion_turn, turn_start};
use crate::tween::Millis;
use crate::variants::ArtTable;

/// The most residents moving at once, and of those, flying or signalling.
pub const MOST_MOVING: usize = 3;
pub const MOST_FLYING: usize = 1;
/// How far from home a resident goes, and how long it may take each way.
pub const ROAM_PX: i32 = 48;
const WAY_MS: u64 = 8000;
/// The art's visibility and placement transition.
pub const TRANSITION_MS: u32 = 1200;
/// Two residents of one species keep apart by up to this much more than the art's offsets.
const ID_PHASE_MS: u64 = 1000;

/// A resident as a turn reads it: what truth says it is, and where it lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resident {
    pub view: ResidentView,
    /// Its stretch of the strip, one of six.
    pub spot: u8,
    /// The ground point it rests at and returns to.
    pub home: i32,
    /// The cell of the vigorous plant its home is beside, where it feeds; `None` for its
    /// stretch's own place. Whether a resident moved home is told by this, never by `home`,
    /// which the salt places.
    pub plant: Option<u8>,
    /// Which way its stretch has more room: 1 to the right, −1 to the left. It goes out that
    /// way and comes back.
    pub out: i32,
}

/// Which way a resident faces: out from home, or back toward it, which is how it rests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Way {
    Out,
    In,
}

/// How a segment takes over from the one before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Enter {
    /// The art's own hand-off, or a loop starting from a pose it shares: no fade.
    Straight,
    /// The new pose rises over the old for half the transition, then the old falls.
    Cover,
    /// The home moved: the old pose fades out where it stood as the new fades in at home.
    Moved,
}

/// One stretch of a turn: a routine from `start`, counting its time from `anchor`, facing
/// `way`, and setting out `from` pixels out from home.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub routine: AnimationId,
    pub start: Millis,
    pub anchor: Millis,
    pub way: Way,
    pub from: i32,
    pub enter: Enter,
    /// A rest held on its first frame: a juvenile sheltering or asleep has no art of its own.
    pub still: bool,
}

/// What one resident does in one turn, from the turn's start: never empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnPlan {
    pub id: u32,
    pub segments: Vec<Segment>,
}

/// What a resident rests in: a routine, perhaps held still.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rest {
    routine: AnimationId,
    still: bool,
}

/// When the species keeps its active hours: the firefly signals only by night; every other
/// resident is about by day and by night.
const fn awake(species: ResidentSpecies, night: bool) -> bool {
    match species {
        ResidentSpecies::Firefly => night,
        ResidentSpecies::Snail
        | ResidentSpecies::Ladybird
        | ResidentSpecies::Springtail
        | ResidentSpecies::Beetle
        | ResidentSpecies::Woodlouse
        | ResidentSpecies::Earthworm
        | ResidentSpecies::Harvestman => true,
    }
}

/// How much of its chances to move a resident takes, in quarters: by its energy.
const fn energy_quarters(energy: Energy) -> u64 {
    match energy {
        Energy::High => 4,
        Energy::Mid => 2,
        Energy::Low => 1,
    }
}

/// How lively the scene lets its residents be, in quarters: calm when mature, very low activity
/// when dead, asleep when dormant.
const fn state_quarters(state: PresentationState) -> u64 {
    match state {
        PresentationState::Thriving
        | PresentationState::Recovering
        | PresentationState::Struggling => 4,
        PresentationState::Mature => 2,
        PresentationState::Dead => 1,
        PresentationState::Dormant => 0,
    }
}

/// The art's routines for one resident.
struct Routines<'a> {
    catalog: &'a Catalog,
    table: &'a ArtTable,
    species: ResidentSpecies,
}

impl Routines<'_> {
    /// The routine named `name` in `stage`, by its structured key.
    fn get(&self, stage: LifeStage, name: &str) -> Option<AnimationId> {
        self.table
            .resident(self.species, stage)
            .iter()
            .copied()
            .find(|&id| self.catalog.animation(id).keys.routine.as_deref() == Some(name))
    }

    /// The first of `names` the species has as an adult.
    fn adult(&self, names: &[&str]) -> Option<AnimationId> {
        names
            .iter()
            .find_map(|name| self.get(LifeStage::Adult, name))
    }

    fn next(&self, id: AnimationId) -> Option<AnimationId> {
        match self.catalog.animation(id).next {
            Next::Handoff(next) => Some(next),
            _ => None,
        }
    }

    fn length(&self, id: AnimationId) -> u64 {
        let animation = self.catalog.animation(id);
        length_ms(animation).unwrap_or_else(|| cycle_ms(animation))
    }
}

/// What the scene is for a resident in one turn.
#[derive(Clone, Copy, Debug)]
struct Conditions {
    state: PresentationState,
    storm: bool,
    night: bool,
}

impl Conditions {
    fn of(snapshot: &Snapshot, w: u64) -> Conditions {
        Conditions {
            state: snapshot.state,
            storm: snapshot.weather == Weather::Storm,
            night: snapshot.clock.night(turn_start(w)),
        }
    }

    fn asleep(self, species: ResidentSpecies) -> bool {
        self.state == PresentationState::Dormant || !awake(species, self.night)
    }
}

/// What a resident rests in under these conditions; `None` if the art lacks it, which
/// `cargo xtask assets` rules out (check 6).
fn rest(art: &Routines, resident: &Resident, at: Conditions) -> Option<Rest> {
    let stage = resident.view.stage;
    let held = |name: &str| {
        art.get(stage, name).map(|routine| Rest {
            routine,
            still: false,
        })
    };
    match stage {
        LifeStage::Egg => {
            let name = crate::variants::art_life_stage(art.species, stage)?;
            held(name)
        }
        LifeStage::Pupa => held("pupa"),
        LifeStage::Dying => art.get(LifeStage::Dying, "death").map(|routine| Rest {
            routine,
            still: false,
        }),
        LifeStage::Juvenile => {
            let sheltering = at.storm || at.asleep(art.species);
            let name = if resident.plant.is_some() && !sheltering {
                "feed"
            } else {
                "idle"
            };
            art.get(stage, name).map(|routine| Rest {
                routine,
                still: sheltering,
            })
        }
        LifeStage::Adult => {
            let name = if at.storm {
                "hide"
            } else if at.asleep(art.species) {
                "sleep"
            } else if resident.plant.is_some() {
                "feed"
            } else {
                "idle"
            };
            held(name)
        }
    }
}

/// A resident's place in the turn's rotation, and its share of the turn's motion.
fn phase(catalog: &Catalog, resident: &Resident) -> u64 {
    let key = crate::variants::art_key(resident.view.species.name());
    let species = catalog
        .residents
        .phase_offsets_ms
        .iter()
        .find(|(name, _)| *name == key)
        .map_or(0, |&(_, ms)| u64::from(ms));
    species + u64::from(resident.view.id) * 397 % ID_PHASE_MS
}

/// The plans of every resident in turn `w`, by id: `now` is the scene the turn reads,
/// `before` the one the turn before read. Residents with no rest the art can draw are left
/// out.
pub fn plans(
    catalog: &Catalog,
    table: &ArtTable,
    w: u64,
    before: Option<&Snapshot>,
    now: &Snapshot,
    arrivals: &Arrivals,
) -> Vec<TurnPlan> {
    let start = turn_start(w);
    let here = Conditions::of(now, w);
    // How each rested through the turn before, and rests in this one. One that arrived then
    // rested as the scene it arrived with had it ([`resting`]).
    let rests: Vec<(&Resident, Rest, Rest, bool)> = now
        .residents
        .iter()
        .filter_map(|resident| {
            let art = routines(catalog, table, resident);
            let (record, scene) = earlier(resident, before, arrivals).unwrap_or((resident, now));
            let earlier = (record, Conditions::of(scene, w.saturating_sub(1)));
            // Read in the earlier record's own art: an id that became another species fades
            // from the one to the other, like a move of home.
            let then = rest(&routines(catalog, table, earlier.0), earlier.0, earlier.1)?;
            let moved = earlier.0.plant != resident.plant
                || earlier.0.view.species != resident.view.species;
            Some((resident, then, rest(&art, resident, here)?, moved))
        })
        .collect();
    // The turn's travellers rotate among those awake and settled, so none of the three
    // places goes to one asleep, hidden or changing.
    let ready: Vec<u32> = rests
        .iter()
        .filter(|&&(resident, then, rest_now, moved)| {
            !moved && then == rest_now && lively(catalog, rest_now) && may_move(resident, now, w)
        })
        .map(|(resident, ..)| resident.view.id)
        .collect();
    let movers = movers(&ready, now, w);
    let mut flyer_taken = false;
    let mut plans = Vec::new();
    for (resident, rest_then, rest_now, moved) in rests {
        let art = routines(catalog, table, resident);
        let phase = phase(catalog, resident);
        let begin = Millis(start.0 + phase);
        let mut plan = Plan::new(&art, start, phase);
        plan.rest(rest_then, start, Enter::Straight);
        if moved || rest_then != rest_now {
            plan.change(rest_then, rest_now, begin, moved);
        } else if movers.contains(&resident.view.id) {
            let flight = !flyer_taken && airborne(resident, here, w);
            flyer_taken |= flight;
            plan.trip(resident, rest_now, begin, flight, w);
        } else {
            plan.pause(resident, rest_now, begin, w);
        }
        plans.push(TurnPlan {
            id: resident.view.id,
            segments: plan.segments,
        });
    }
    plans
}

fn routines<'a>(catalog: &'a Catalog, table: &'a ArtTable, resident: &Resident) -> Routines<'a> {
    Routines {
        catalog,
        table,
        species: resident.view.species,
    }
}

/// Whether a rest is a lively one, idle or feeding, from which a resident may set out or do
/// something: not asleep, sheltering, hidden, or in a stage that holds.
fn lively(catalog: &Catalog, rest: Rest) -> bool {
    let routine = catalog.animation(rest.routine).keys.routine.as_deref();
    !rest.still && matches!(routine, Some("idle" | "feed"))
}

/// A turn `w` at rest, as `scene` has the resident in turn `read`: one that arrived in the turn
/// rests as the scene it arrived with has it, and one fading out after leaving rests as it did
/// in the turn before.
pub fn resting(
    catalog: &Catalog,
    table: &ArtTable,
    w: u64,
    resident: &Resident,
    scene: &Snapshot,
    read: u64,
) -> Option<TurnPlan> {
    let art = routines(catalog, table, resident);
    let rest = rest(&art, resident, Conditions::of(scene, read))?;
    let start = turn_start(w);
    let mut plan = Plan::new(&art, start, phase(catalog, resident));
    plan.rest(rest, start, Enter::Straight);
    Some(TurnPlan {
        id: resident.view.id,
        segments: plan.segments,
    })
}

/// Whether a resident's trip this turn may take the one airborne place: a firefly by night,
/// to fly or signal, or now and then a springtail, to hop.
fn airborne(resident: &Resident, at: Conditions, w: u64) -> bool {
    resident.view.stage == LifeStage::Adult
        && match resident.view.species {
            ResidentSpecies::Firefly => at.night,
            ResidentSpecies::Springtail => {
                motion_hash(u64::from(resident.view.id), w).is_multiple_of(4)
            }
            _ => false,
        }
}

/// How long after a turn begins any change of rest it starts is over: the latest phase, the
/// longest hand-off (an eclosion) and a cover after it.
pub const CHANGE_SETTLES_MS: u32 = 8000;

/// Whether the turn after `w` will change how some resident rests or where it lives, given
/// the scene turn `w` reads (`None` before the first) and what the residents not in it arrived
/// with: a lifecycle step, a new home or species, a storm coming or going, dormancy, or day
/// turning to night. A change of energy alone changes no rest. It reads no art, so it may answer yes where in the end nothing changes,
/// never the other way.
pub fn pending(before: Option<&Snapshot>, now: &Snapshot, w: u64, arrivals: &Arrivals) -> bool {
    let conditions = |snapshot: &Snapshot, w: u64| {
        (
            snapshot.state == PresentationState::Dormant,
            snapshot.weather == Weather::Storm,
            snapshot.clock.night(turn_start(w)),
        )
    };
    let after = conditions(now, w + 1);
    let what = |r: &Resident| (r.view.species, r.view.stage, r.plant);
    now.residents.iter().any(|resident| {
        earlier(resident, before, arrivals).is_some_and(|(earlier, scene)| {
            conditions(scene, w) != after || what(earlier) != what(resident)
        })
    })
}

/// What residents arrived as, with the scene they arrived with, by id and stretch.
pub type Arrivals<'a> = BTreeMap<(u32, u8), (&'a Resident, &'a Snapshot)>;

/// How a resident stood in the turn before, in the same stretch, and the scene that turn read
/// it in: the record `before` holds, or else what it arrived as, with the scene it arrived
/// with. `None` for a resident first seen now, or first seen in this stretch: one that comes
/// back in another stretch is another to draw.
pub fn earlier<'a>(
    resident: &Resident,
    before: Option<&'a Snapshot>,
    arrivals: &Arrivals<'a>,
) -> Option<(&'a Resident, &'a Snapshot)> {
    let (id, spot) = (resident.view.id, resident.spot);
    before
        .and_then(|scene| {
            let record = scene
                .residents
                .iter()
                .find(|r| (r.view.id, r.spot) == (id, spot))?;
            Some((record, scene))
        })
        .or_else(|| arrivals.get(&(id, spot)).copied())
}

/// The residents that travel in turn `w`, of those `ready` to: three in rotation, or all
/// when there are three or fewer. In dim mode only every fourth turn moves.
fn movers(ready: &[u32], now: &Snapshot, w: u64) -> Vec<u32> {
    let n = ready.len();
    let Some(j) = motion_turn(w, now.dim) else {
        return Vec::new();
    };
    if n <= MOST_MOVING {
        return ready.to_vec();
    }
    let first = usize::try_from(j % u64::try_from(n).unwrap_or(1)).unwrap_or(0);
    (0..MOST_MOVING)
        .map(|k| ready[(first * MOST_MOVING + k) % n])
        .collect()
}

/// Whether a chosen resident uses its turn: its energy and the scene's liveliness, in
/// sixteenths, against the turn's draw for it.
fn may_move(resident: &Resident, now: &Snapshot, w: u64) -> bool {
    let share = energy_quarters(resident.view.energy) * state_quarters(now.state);
    motion_hash(u64::from(resident.view.id), w) % 16 < share
}

/// A turn's segments, built in order.
struct Plan<'a> {
    art: &'a Routines<'a>,
    start: Millis,
    /// Rest loops count their time from the resident's phase, on every turn alike, so a loop
    /// runs on unbroken across turns while the rest is the same.
    rest_anchor: Millis,
    segments: Vec<Segment>,
}

impl<'a> Plan<'a> {
    fn new(art: &'a Routines<'a>, start: Millis, phase: u64) -> Plan<'a> {
        Plan {
            art,
            start,
            rest_anchor: Millis(phase),
            segments: Vec::new(),
        }
    }

    fn push(&mut self, routine: AnimationId, at: Millis, way: Way, from: i32, enter: Enter) {
        self.segments.push(Segment {
            routine,
            start: at,
            anchor: at,
            way,
            from,
            enter,
            still: false,
        });
    }

    /// Rests at home from `at`. A held routine (an egg, a hide, a death) has long since
    /// played out; a loop keeps the resident's phase.
    fn rest(&mut self, rest: Rest, at: Millis, enter: Enter) {
        let finite = length_ms(self.art.catalog.animation(rest.routine)).is_some();
        let fresh = enter != Enter::Straight || at != self.start;
        let anchor = match (finite, fresh) {
            (true, true) => at,
            (true, false) => Millis(0),
            (false, _) => self.rest_anchor,
        };
        self.segments.push(Segment {
            routine: rest.routine,
            start: at,
            anchor,
            way: Way::In,
            from: 0,
            enter,
            still: rest.still,
        });
    }

    /// Plays a finite routine from `at`, then follows its hand-off; returns when it ends and
    /// what it handed off to.
    fn play(
        &mut self,
        routine: AnimationId,
        at: Millis,
        way: Way,
        from: i32,
    ) -> (Millis, Option<AnimationId>) {
        self.push(routine, at, way, from, Enter::Straight);
        let end = Millis(at.0 + self.art.length(routine));
        (end, self.art.next(routine))
    }

    /// Ends a played routine in `rest`: straight on if the routine hands off to it, else the
    /// hand-off's pose covered by the rest.
    fn settle(&mut self, handed: Option<AnimationId>, rest: Rest, at: Millis) {
        if handed == Some(rest.routine) && !rest.still {
            self.rest(rest, at, Enter::Straight);
        } else {
            if let Some(handed) = handed {
                self.push(handed, at, Way::In, 0, Enter::Straight);
            }
            self.rest(rest, at, Enter::Cover);
        }
    }

    /// The rest changes, or the home moves: through the art's own steps where it has them.
    fn change(&mut self, then: Rest, now: Rest, at: Millis, moved: bool) {
        let art = self.art;
        let name = |id: AnimationId| art.catalog.animation(id).keys.routine.as_deref();
        let stage_of = |id: AnimationId| art.catalog.animation(id).keys.life_stage.as_deref();
        let step = if moved {
            None
        } else if name(now.routine) == Some("death") {
            self.push(now.routine, at, Way::In, 0, Enter::Straight);
            return;
        } else if stage_of(then.routine) != stage_of(now.routine) {
            // A hatch or an eclosion: the routine of the earlier stage that hands off to
            // the new stage.
            ["hatch", "eclose"].iter().find_map(|routine| {
                let earlier = stage_of_routine(art, then.routine)?;
                art.get(earlier, routine)
            })
        } else if name(now.routine) == Some("hide") {
            art.adult(&["withdraw", "curl", "retreat"])
        } else if name(then.routine) == Some("hide") {
            art.adult(&["emerge", "uncurl"])
        } else {
            None
        };
        match step {
            Some(step) => {
                let (end, handed) = self.play(step, at, Way::In, 0);
                self.settle(handed, now, end);
            }
            None => self.rest(now, at, if moved { Enter::Moved } else { Enter::Cover }),
        }
    }

    /// A turn out and back: walk, flight or hop out, rest, and come home; or a firefly's
    /// signal where it rests.
    fn trip(&mut self, resident: &Resident, home: Rest, at: Millis, flight: bool, w: u64) {
        let art = self.art;
        let stage = resident.view.stage;
        let draw = motion_hash(u64::from(resident.view.id), w) >> 8;
        let (Some(walk), Some(idle)) = (art.get(stage, "move"), art.get(stage, "idle")) else {
            self.pause(resident, home, at, w);
            return;
        };
        let pause = Rest {
            routine: idle,
            still: false,
        };
        // How many whole cycles of a routine moving `px` a cycle fit the way out.
        let cycles = |routine: AnimationId, room: u64| -> u64 {
            let animation = art.catalog.animation(routine);
            let px = u64::from(animation.cycle_displacement.unsigned_abs()).max(1);
            let most =
                (room / cycle_ms(animation).max(1)).min(u64::try_from(ROAM_PX).unwrap_or(0) / px);
            // Between half and all of the way, by the turn's draw.
            (most * (8 + draw % 9) / 16).max(1)
        };
        let firefly = resident.view.species == ResidentSpecies::Firefly;
        if flight
            && firefly
            && draw.is_multiple_of(2)
            && let Some(signal) = art.get(stage, "signal")
        {
            let (end, handed) = self.play(signal, at, Way::In, 0);
            self.settle(handed, home, end);
            return;
        }
        let (out, back) = if flight && firefly {
            (self.flight(Way::Out, 0, at, &cycles), None)
        } else if flight && let Some(hop) = art.get(stage, "hop") {
            let (end, _) = self.play(hop, at, Way::Out, 0);
            let px = i32::from(art.catalog.animation(hop).cycle_displacement);
            (Some((end, px)), Some(px))
        } else {
            let k = cycles(walk, WAY_MS);
            self.push(walk, at, Way::Out, 0, Enter::Straight);
            let animation = art.catalog.animation(walk);
            let px = i32::try_from(k).unwrap_or(0) * i32::from(animation.cycle_displacement);
            (Some((Millis(at.0 + k * cycle_ms(animation)), px)), Some(px))
        };
        let Some((there, far)) = out else {
            self.pause(resident, home, at, w);
            return;
        };
        // A rest out there, facing on, then the way back.
        self.push(pause.routine, there, Way::Out, far, Enter::Straight);
        let turn_back = Millis(there.0 + cycle_ms(art.catalog.animation(pause.routine)));
        let home_at = match back {
            None => self
                .flight(Way::In, far, turn_back, &cycles)
                .map(|(end, _)| end),
            Some(px) => {
                let animation = art.catalog.animation(walk);
                let step = i32::from(animation.cycle_displacement).max(1);
                let k = u64::try_from(px / step).unwrap_or(1);
                self.push(walk, turn_back, Way::In, far, Enter::Straight);
                Some(Millis(turn_back.0 + k * cycle_ms(animation)))
            }
        };
        if let Some(home_at) = home_at {
            let handed = art.next(walk).filter(|_| back.is_some()).or(Some(idle));
            self.settle(handed, home, home_at);
        }
    }

    /// A firefly's flight one way: takeoff, flight, landing. Returns when it lands and how far
    /// out it has come.
    fn flight(
        &mut self,
        way: Way,
        from: i32,
        at: Millis,
        cycles: &dyn Fn(AnimationId, u64) -> u64,
    ) -> Option<(Millis, i32)> {
        let art = self.art;
        let takeoff = art.adult(&["takeoff"])?;
        let flight = art.adult(&["flight"])?;
        let land = art.adult(&["land"])?;
        let both = art.length(takeoff) + art.length(land);
        let animation = art.catalog.animation(flight);
        let k = match way {
            Way::Out => cycles(flight, WAY_MS.saturating_sub(both)),
            Way::In => u64::try_from(from / i32::from(animation.cycle_displacement).max(1)).ok()?,
        };
        let px = i32::try_from(k).ok()? * i32::from(animation.cycle_displacement);
        let (lifted, _) = self.play(takeoff, at, way, from);
        self.push(flight, lifted, way, from, Enter::Straight);
        let landing = Millis(lifted.0 + k * cycle_ms(animation));
        let out = match way {
            Way::Out => from + px,
            Way::In => from - px,
        };
        let (landed, _) = self.play(land, landing, way, out);
        Some((landed, out))
    }

    /// A turn at rest: unchanged, but now and then a harvestman probes.
    fn pause(&mut self, resident: &Resident, home: Rest, at: Millis, w: u64) {
        let probing = resident.view.stage == LifeStage::Adult
            && lively(self.art.catalog, home)
            && motion_hash(u64::from(resident.view.id), w).is_multiple_of(3);
        if let Some(probe) = self.art.get(LifeStage::Adult, "probe").filter(|_| probing) {
            let (end, handed) = self.play(probe, at, Way::In, 0);
            self.settle(handed, home, end);
        }
    }
}

/// The stage a routine belongs to, as a `LifeStage`.
fn stage_of_routine(art: &Routines, routine: AnimationId) -> Option<LifeStage> {
    let stage = art.catalog.animation(routine).keys.life_stage.as_deref()?;
    match stage {
        "egg" | "brood" | "cocoon" => Some(LifeStage::Egg),
        "juvenile" => Some(LifeStage::Juvenile),
        "pupa" => Some(LifeStage::Pupa),
        "adult" => Some(LifeStage::Adult),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use terrarium_truth::scene::Wind;

    use super::*;
    use crate::catalog::Art;
    use crate::clock::{DayNight, LocalClock};
    use crate::motion::{Clock, TURN_MS};
    use crate::sky::LocalDate;

    fn scene(residents: Vec<Resident>, weather: Weather, hour: u32) -> Snapshot {
        Snapshot {
            state: PresentationState::Thriving,
            weather,
            wind: Wind::Calm,
            dim: false,
            clock: Clock {
                local: LocalClock::new(LocalDate::new(2026, 3, 3), hour * 3_600_000),
                at: Millis(0),
                day_night: DayNight::default(),
            },
            swaying: BTreeSet::new(),
            residents,
        }
    }

    fn resident(
        id: u32,
        species: ResidentSpecies,
        stage: LifeStage,
        plant: Option<u8>,
    ) -> Resident {
        Resident {
            view: ResidentView {
                id,
                species,
                stage,
                energy: Energy::High,
            },
            spot: u8::try_from(id % 6).unwrap(),
            home: 100,
            plant,
            out: if id.is_multiple_of(2) { 1 } else { -1 },
        }
    }

    /// Every species, as one stage then another, perhaps beside a plant then not.
    fn cast(
        then: LifeStage,
        now: LifeStage,
        plants: (Option<u8>, Option<u8>),
    ) -> (Vec<Resident>, Vec<Resident>) {
        let (mut before, mut after) = (Vec::new(), Vec::new());
        for (&species, id) in ResidentSpecies::ALL.iter().zip(1..) {
            let fits = |stage| stage != LifeStage::Pupa || species.has_pupa();
            if fits(then) && fits(now) {
                before.push(resident(id, species, then, plants.0));
                after.push(resident(id, species, now, plants.1));
            }
        }
        (before, after)
    }

    #[test]
    fn every_plan_settles_before_its_turn_ends() {
        use LifeStage::{Adult, Dying, Egg, Juvenile, Pupa};
        let catalog = &Art::embedded().catalog;
        let table = ArtTable::new(catalog).unwrap();
        let steps = [
            (Adult, Adult, (Some(3), Some(3))),
            (Juvenile, Juvenile, (None, None)),
            (Egg, Juvenile, (Some(3), Some(3))),
            (Pupa, Adult, (None, Some(3))),
            (Juvenile, Adult, (None, None)),
            (Adult, Dying, (Some(3), None)),
            (Adult, Adult, (Some(3), None)),
        ];
        let mut segments = 0;
        for (then, now, plants) in steps {
            let (before, after) = cast(then, now, plants);
            for (weathers, hour) in [
                ((Weather::Clear, Weather::Clear), 12),
                ((Weather::Clear, Weather::Clear), 0),
                ((Weather::Clear, Weather::Storm), 0),
                ((Weather::Storm, Weather::Clear), 12),
            ] {
                let before = scene(before.clone(), weathers.0, hour);
                let now = scene(after.clone(), weathers.1, hour);
                for w in 1..60 {
                    let end = turn_start(w + 1);
                    for plan in plans(catalog, &table, w, Some(&before), &now, &Arrivals::new()) {
                        let last = plan.segments.last().unwrap();
                        let routine = catalog.animation(last.routine);
                        let name = routine.keys.routine.as_deref().unwrap_or("");
                        assert!(
                            !matches!(name, "move" | "flight" | "takeoff" | "land" | "hop"),
                            "turn {w} ends {} on its way: {}",
                            plan.id,
                            routine.name
                        );
                        for segment in &plan.segments {
                            segments += 1;
                            let animation = catalog.animation(segment.routine);
                            let fades = if segment.enter == Enter::Straight {
                                0
                            } else {
                                TRANSITION_MS
                            };
                            let plays = if segment.anchor == segment.start {
                                length_ms(animation).unwrap_or(0)
                            } else {
                                0
                            };
                            let over = segment
                                .start
                                .plus(fades)
                                .max(Millis(segment.start.0 + plays));
                            assert!(
                                over <= end,
                                "{} in turn {w}: {} runs to {over:?}",
                                plan.id,
                                animation.name
                            );
                        }
                    }
                }
            }
        }
        assert!(segments > 10_000, "{segments}");
        assert_eq!(TURN_MS, 24_000);
    }
}
