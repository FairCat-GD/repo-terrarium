//! The residents: the terrarium's cast, stepped once per UTC day, right after the vegetation,
//! when the day closes.
//!
//! - **The pool.** The seed picks five of the eight species, among the casts with a species
//!   about by day, one about by night and at most one that flies. Under the residents' hours
//!   and the art as delivered, every five qualify; the rule is applied all the same.
//! - **Capacity.** Terrain sets how many residents the terrarium holds, `R`, eggs included.
//! - **Food.** Energy moves toward what the plants give, at the pace rain sets (tempo, never
//!   health): their vigor, summed over at least half the plant capacity, so a full terrarium
//!   feeds at its mean vigor and a sparse or dying one less. It is the whole terrarium's food,
//!   because the composer, not truth, places residents beside plants. Decomposers also feed on
//!   decay (dead plants, litter, fungus): enough to stay, never enough to breed.
//! - **Life.** Each resident lives through its species' stages, egg to juvenile to pupa (where
//!   the species has one) to adult, counting only the days the repository is not inactive, so
//!   a dormant terrarium keeps its sleeping cast. An adult that has lived its days dies: it is
//!   `Dying` for one day, then gone. A resident whose energy stays low for 14 days leaves (an
//!   egg or pupa fails), with no death routine.
//! - **Births and returns.** A well-fed adult lays an egg every 14 days at the usual pace,
//!   while there is room and its kind is not already two. While the plants feed well, arrivals
//!   bank, up to `R`, and fill free room with adults of the pool's least-present species; they
//!   are gone the day the plants stop feeding well, so a cast that left trickles back.
//!
//! The step reads only its state, the day's climate, the vegetation and the tick, and its random
//! draws come from a stream keyed by that tick, so a cached state resumes exactly as a full
//! replay goes on. Ids count up and are never reused.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::canon::canonical_struct;
use crate::fixed::Q16;
use crate::habitat::{plant_capacity, resident_capacity};
use crate::identity::RepoSeed;
use crate::rng::{Pcg32, StreamTag};
use crate::scene::{Energy, LifeStage, ResidentSpecies, ResidentView};
use crate::time::{TICKS_PER_DAY, Tick};
use crate::tuning::{
    ARRIVAL_DAYS, BROOD_DAYS, DETRITUS_FOOD, ENERGY_HIGH_FROM, ENERGY_LOW_BELOW, FOOD_SHARE,
    KIN_MAX, RESIDENT_POOL, RETREAT_DAYS, life_days,
};
use crate::vegetation::{Conditions, Vegetation, approach, daily_rate, tempo};

/// One resident.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resident {
    /// Stable for the resident's life, and never given to another.
    pub id: u32,
    pub species: ResidentSpecies,
    pub stage: LifeStage,
    /// The close it was laid at, or arrived at.
    pub since: Tick,
    /// Days in its stage while the repository was not inactive.
    pub stage_days: u32,
    pub energy: Q16,
    /// The first close of its current run of low energy, if it is in one.
    pub low_since: Option<Tick>,
    /// An adult's days toward its next egg, at the usual pace: `BROOD_DAYS` is an egg.
    pub brood: Q16,
}

canonical_struct!(Resident {
    id,
    species,
    stage,
    since,
    stage_days,
    energy,
    low_since,
    brood,
});

impl Resident {
    /// Alive and not dying: what counts toward its kind.
    pub fn living(&self) -> bool {
        self.stage != LifeStage::Dying
    }

    /// One more active day in its stage, and the next stage once its species' days are done.
    fn age(&mut self) {
        let [egg, juvenile, pupa, adult] = life_days(self.species);
        let (days, next) = match self.stage {
            LifeStage::Egg => (egg, LifeStage::Juvenile),
            LifeStage::Juvenile if self.species.has_pupa() => (juvenile, LifeStage::Pupa),
            LifeStage::Juvenile => (juvenile, LifeStage::Adult),
            LifeStage::Pupa => (pupa, LifeStage::Adult),
            LifeStage::Adult => (adult, LifeStage::Dying),
            LifeStage::Dying => return,
        };
        self.stage_days = self.stage_days.saturating_add(1);
        if self.stage_days >= days {
            self.stage = next;
            self.stage_days = 0;
        }
    }
}

/// The terrarium's residents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fauna {
    /// The id the next egg or arrival takes.
    pub next_id: u32,
    /// The five species this terrarium hosts, in species order.
    pub pool: Vec<ResidentSpecies>,
    /// Ordered by id.
    pub residents: Vec<Resident>,
    /// Days of arrivals banked while the plants feed well, at the usual pace, up to
    /// `ARRIVAL_DAYS` for each resident the terrarium holds: each arrival spends `ARRIVAL_DAYS`.
    pub arrivals: Q16,
    /// Running totals: residents that arrived, eggs laid, residents that died of old age, and
    /// residents that left (low energy, or no room).
    pub arrived: u32,
    pub laid: u32,
    pub died: u32,
    pub left: u32,
}

canonical_struct!(Fauna {
    next_id,
    pool,
    residents,
    arrivals,
    arrived,
    laid,
    died,
    left,
});

/// Whether a cast of these species may share a terrarium: one about by day, one about by night,
/// and at most one that flies.
pub fn admissible(cast: &[ResidentSpecies]) -> bool {
    cast.iter().any(|species| species.about_by_day())
        && cast.iter().any(|species| species.about_by_night())
        && cast.iter().filter(|species| species.flies()).count() <= 1
}

/// Every pool the rules admit: each set of five species, ordered by its bitmask over
/// `ResidentSpecies::ALL`, each in species order.
pub fn admissible_pools() -> Vec<Vec<ResidentSpecies>> {
    let species = u32::try_from(ResidentSpecies::ALL.len()).unwrap_or(0);
    (0_u32..1 << species)
        .filter(|mask| mask.count_ones() == RESIDENT_POOL)
        .map(|mask| {
            ResidentSpecies::ALL
                .iter()
                .zip(0_u32..)
                .filter(|&(_, bit)| (mask >> bit) & 1 == 1)
                .map(|(&species, _)| species)
                .collect::<Vec<_>>()
        })
        .filter(|pool| admissible(pool))
        .collect()
}

/// What the plants give: the living plants' vigor, summed over at least `⌈P / 2⌉` of them.
pub fn plant_food(vegetation: &Vegetation, capacity: u32) -> Q16 {
    let (living, vigor) = vegetation
        .plants
        .iter()
        .filter(|plant| plant.alive())
        .fold((0_u32, Q16::ZERO), |(n, sum), plant| {
            (n.saturating_add(1), sum.saturating_add(plant.vigor))
        });
    let over = living.max(capacity.div_ceil(FOOD_SHARE)).max(1);
    vigor
        .saturating_div(Q16::from_int(i32::try_from(over).unwrap_or(i32::MAX)))
        .clamp01()
}

/// Whether anything decays for decomposers to feed on: a dead plant, litter or fungus.
pub fn decaying(vegetation: &Vegetation) -> bool {
    vegetation.plants.iter().any(|plant| !plant.alive())
        || !vegetation.litter.is_empty()
        || !vegetation.fungus.is_empty()
}

/// Energy as the composer is shown it: vigor's bands.
pub fn energy_band(energy: Q16) -> Energy {
    if energy >= ENERGY_HIGH_FROM {
        Energy::High
    } else if energy < ENERGY_LOW_BELOW {
        Energy::Low
    } else {
        Energy::Mid
    }
}

/// One day's close of everything that lives: the vegetation, then the residents, who feed on
/// what it left.
pub(crate) fn close_living(
    seed: &RepoSeed,
    today: &Conditions,
    vegetation: &mut Vegetation,
    fauna: &mut Fauna,
) {
    vegetation.close(seed, today);
    fauna.close(seed, today, vegetation);
}

/// What the day's close feeds each species.
struct Food {
    plants: Q16,
    decay: bool,
}

impl Food {
    fn of(&self, species: ResidentSpecies) -> Q16 {
        if species.decomposer() && self.decay {
            self.plants.max(DETRITUS_FOOD)
        } else {
            self.plants
        }
    }
}

fn count<T>(items: impl Iterator<Item = T>) -> u32 {
    u32::try_from(items.count()).unwrap_or(u32::MAX)
}

impl Fauna {
    /// No residents yet, and a pool of the species `seed` picks.
    pub fn new(seed: &RepoSeed) -> Fauna {
        let pools = admissible_pools();
        let picked = u32::try_from(pools.len())
            .ok()
            .and_then(NonZeroU32::new)
            .map(|bound| Pcg32::stream(seed, StreamTag::ResidentPool, 0).below(bound))
            .and_then(|index| usize::try_from(index).ok());
        Fauna {
            next_id: 1,
            pool: picked
                .and_then(|index| pools.get(index).cloned())
                .unwrap_or_default(),
            residents: Vec::new(),
            arrivals: Q16::ZERO,
            arrived: 0,
            laid: 0,
            died: 0,
            left: 0,
        }
    }

    /// One day's close, after the vegetation's.
    pub fn close(&mut self, seed: &RepoSeed, today: &Conditions, vegetation: &Vegetation) {
        let capacity = resident_capacity(today.terrain);
        let food = Food {
            plants: plant_food(vegetation, plant_capacity(today.terrain)),
            decay: decaying(vegetation),
        };
        self.bury_the_dead();
        self.make_room(capacity);
        self.feed(today, &food);
        if !today.inactive {
            for resident in &mut self.residents {
                resident.age();
            }
            self.lay(today, capacity);
        }
        self.arrive(seed, today, capacity, &food);
    }

    /// Residents that were dying at the last close are gone.
    fn bury_the_dead(&mut self) {
        let before = self.residents.len();
        self.residents.retain(Resident::living);
        self.died = self.died.saturating_add(gone(before, self.residents.len()));
    }

    /// Residents beyond the capacity leave, the newest first.
    fn make_room(&mut self, capacity: u32) {
        let before = self.residents.len();
        self.residents
            .truncate(usize::try_from(capacity).unwrap_or(usize::MAX));
        self.left = self.left.saturating_add(gone(before, self.residents.len()));
    }

    /// Energy moves toward food; a resident low for `RETREAT_DAYS` leaves.
    fn feed(&mut self, today: &Conditions, food: &Food) {
        let rate = daily_rate(today.rain);
        let before = self.residents.len();
        self.residents.retain_mut(|resident| {
            resident.energy = approach(resident.energy, food.of(resident.species), rate);
            if resident.energy >= ENERGY_LOW_BELOW {
                resident.low_since = None;
                return true;
            }
            let since = *resident.low_since.get_or_insert(today.tick);
            today.tick.hours_since(since) < RETREAT_DAYS.saturating_mul(TICKS_PER_DAY)
        });
        self.left = self.left.saturating_add(gone(before, self.residents.len()));
    }

    /// Living members of each species.
    fn kin(&self) -> BTreeMap<ResidentSpecies, u32> {
        let mut kin = BTreeMap::new();
        for resident in self.residents.iter().filter(|r| r.living()) {
            *kin.entry(resident.species).or_insert(0) += 1;
        }
        kin
    }

    fn room(&self, capacity: u32) -> bool {
        count(self.residents.iter()) < capacity
    }

    /// Well-fed adults' broods grow, and those that are ready lay while there is room, the
    /// oldest first.
    fn lay(&mut self, today: &Conditions, capacity: u32) {
        let step = tempo(today.rain);
        for resident in &mut self.residents {
            if resident.stage == LifeStage::Adult && resident.energy >= ENERGY_HIGH_FROM {
                resident.brood = resident.brood.saturating_add(step).min(BROOD_DAYS);
            }
        }
        while self.room(capacity) {
            let kin = self.kin();
            let members = |species| kin.get(&species).copied().unwrap_or(0);
            let Some(parent) = self.residents.iter_mut().find(|r| {
                r.stage == LifeStage::Adult
                    && r.energy >= ENERGY_HIGH_FROM
                    && r.brood >= BROOD_DAYS
                    && members(r.species) < KIN_MAX
            }) else {
                break;
            };
            parent.brood = Q16::ZERO;
            let (species, energy) = (parent.species, parent.energy);
            self.join(species, LifeStage::Egg, energy, today.tick);
            self.laid = self.laid.saturating_add(1);
        }
    }

    /// Arrivals bank while the plants feed well and are gone when they do not; while the
    /// repository is active, banked arrivals fill free room with the pool's least-present
    /// species.
    fn arrive(&mut self, seed: &RepoSeed, today: &Conditions, capacity: u32, food: &Food) {
        if food.plants < ENERGY_HIGH_FROM {
            self.arrivals = Q16::ZERO;
            return;
        }
        if today.inactive {
            return;
        }
        let bank =
            Q16::from_int(i32::try_from(capacity).unwrap_or(i32::MAX)).saturating_mul(ARRIVAL_DAYS);
        self.arrivals = self.arrivals.saturating_add(tempo(today.rain)).min(bank);
        let mut rng = Pcg32::stream(
            seed,
            StreamTag::ResidentArrival,
            u64::from(today.tick.hours()),
        );
        while self.room(capacity) && self.arrivals >= ARRIVAL_DAYS {
            let kin = self.kin();
            let members = |species| kin.get(&species).copied().unwrap_or(0);
            let fewest = self
                .pool
                .iter()
                .map(|&s| members(s))
                .min()
                .unwrap_or(KIN_MAX);
            if fewest >= KIN_MAX {
                break;
            }
            let least: Vec<ResidentSpecies> = self
                .pool
                .iter()
                .copied()
                .filter(|&s| members(s) == fewest)
                .collect();
            let Some(species) = u32::try_from(least.len())
                .ok()
                .and_then(NonZeroU32::new)
                .and_then(|bound| usize::try_from(rng.below(bound)).ok())
                .and_then(|index| least.get(index).copied())
            else {
                break;
            };
            self.join(species, LifeStage::Adult, food.of(species), today.tick);
            self.arrivals = self.arrivals.saturating_sub(ARRIVAL_DAYS);
            self.arrived = self.arrived.saturating_add(1);
        }
    }

    /// A new resident, laid or arrived at `now`.
    fn join(&mut self, species: ResidentSpecies, stage: LifeStage, energy: Q16, now: Tick) {
        self.residents.push(Resident {
            id: self.next_id,
            species,
            stage,
            since: now,
            stage_days: 0,
            energy,
            low_since: None,
            brood: Q16::ZERO,
        });
        self.next_id = self.next_id.saturating_add(1);
    }

    /// The residents as the composer sees them, by id.
    pub fn views(&self) -> Vec<ResidentView> {
        self.residents
            .iter()
            .map(|resident| ResidentView {
                id: resident.id,
                species: resident.species,
                stage: resident.stage,
                energy: energy_band(resident.energy),
            })
            .collect()
    }
}

fn gone(before: usize, after: usize) -> u32 {
    u32::try_from(before.saturating_sub(after)).unwrap_or(u32::MAX)
}
