//! The vegetation: plants, weeds, litter, fungus and the seed bank, stepped once per UTC day,
//! when the day closes.
//!
//! - **The pool.** The seed picks 8 of the 12 species: 2 of the 3 far, 3 of the 5 mid and 3 of
//!   the 4 near. Their seed bank never falls below 2 seeds each, so any state can recover.
//! - **Slots.** Terrain sets the capacity: `P` plants and `W` weeds. Truth numbers its own slots
//!   `0..P`, each in a layer by a fixed pattern of six (one far, two mid, three near), and a
//!   plant keeps its slot for life. The composer maps them onto its own cells, so nothing here
//!   depends on how many it has. Weeds take capacity but no slot.
//! - **Vigor.** Each plant's vigor moves toward a target at a pace set by rain (tempo, never
//!   health). The target is health scaled by 4/3, health being the lower of light and soil, an
//!   unknown one counting as neutral; the lower one is the limiting factor, which decides
//!   whether a failing plant is pale (light) or wilting (soil).
//! - **Life.** Seedlings germinate from the seed bank into free slots, while the repository is
//!   not inactive and a seedling would not be failing from the start. Plants grow from seedling
//!   to young to mature with the days they grow. A plant failing for 21 days dies, stands for 30,
//!   then falls into litter, which turns to fungus after 14 days and is gone 60 after that.
//! - **Weeds** germinate on poor soil before seedlings take the free capacity, and wither as it
//!   recovers.
//!
//! The step reads only its state, the day's climate and the tick, and its random draws come from
//! a stream keyed by that tick, so a cached state resumes exactly as a full replay goes on.
//! Ids count up and are never reused.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::canon::{canonical_enum, canonical_struct, count_of};
use crate::climate;
use crate::domain::PresentationState;
use crate::fixed::Q16;
use crate::habitat::{plant_capacity, weed_capacity};
use crate::identity::RepoSeed;
use crate::rng::{Pcg32, StreamTag};
use crate::scene::{GrowthStage, PlantSpecies, PlantState, PlantView};
use crate::time::{TICKS_PER_DAY, Tick};
use crate::tuning::{
    DEAD_STANDING_DAYS, DYING_DAYS, FAILING_BELOW, FAR_STAGE_DAYS, FUNGUS_DAYS, GROWTH_RATE,
    LITTER_DAYS, MID_STAGE_DAYS, NEAR_STAGE_DAYS, NEUTRAL, SEED_BANK_CAP, SEED_BANK_FLOOR,
    SEED_BANK_START, TEMPO_MAX, TEMPO_MIN, THRIVING_FROM, VIGOR_PER_HEALTH, WEEDS_BELOW,
    WEEDS_WITHER_FROM,
};

/// The strip's three planes of plants, back to front.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Far,
    Mid,
    Near,
}

impl Layer {
    /// The species that grow in this layer, of which the pool takes 2, 3 and 3.
    const fn species(self) -> &'static [PlantSpecies] {
        match self {
            Layer::Far => &[PlantSpecies::Oak, PlantSpecies::Birch, PlantSpecies::Rowan],
            Layer::Mid => &[
                PlantSpecies::Fern,
                PlantSpecies::TallGrass,
                PlantSpecies::Bramble,
                PlantSpecies::Foxglove,
                PlantSpecies::Reed,
            ],
            Layer::Near => &[
                PlantSpecies::MossMound,
                PlantSpecies::WildflowerCluster,
                PlantSpecies::Clover,
                PlantSpecies::Thistle,
            ],
        }
    }

    const fn picked(self) -> u32 {
        match self {
            Layer::Far => 2,
            Layer::Mid | Layer::Near => 3,
        }
    }

    /// Days grown to young, then to mature.
    const fn stage_days(self) -> [u32; 2] {
        match self {
            Layer::Far => FAR_STAGE_DAYS,
            Layer::Mid => MID_STAGE_DAYS,
            Layer::Near => NEAR_STAGE_DAYS,
        }
    }

    pub fn of(species: PlantSpecies) -> Layer {
        [Layer::Far, Layer::Mid, Layer::Near]
            .into_iter()
            .find(|layer| layer.species().contains(&species))
            .unwrap_or(Layer::Near)
    }

    /// The layer of truth's slot `slot`: one far, two mid and three near in every six, spread so
    /// that a small terrarium has all three.
    pub const fn of_slot(slot: u16) -> Layer {
        match slot % 6 {
            2 => Layer::Far,
            1 | 4 => Layer::Mid,
            _ => Layer::Near,
        }
    }
}

/// Which of light and soil holds a plant back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Limit {
    Light,
    Soil,
}

canonical_enum!(Limit { Light, Soil });

/// One plant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plant {
    /// Stable for the plant's life, and never given to another.
    pub id: u32,
    pub species: PlantSpecies,
    /// Truth's slot, `0..P`, kept for life.
    pub slot: u16,
    /// The close it germinated at.
    pub born: Tick,
    pub vigor: Q16,
    /// The limiting factor at the last close.
    pub limit: Limit,
    pub stage: GrowthStage,
    /// Days grown: at vigor 0.35 or more, while the repository was not inactive.
    pub grown: u32,
    /// The first close of the current failing run, if it is failing.
    pub low_since: Option<Tick>,
    /// When it died; it stands until it falls.
    pub dead_since: Option<Tick>,
    /// The tick a flowering event's bloom ends.
    pub bloom_until: Option<Tick>,
}

canonical_struct!(Plant {
    id,
    species,
    slot,
    born,
    vigor,
    limit,
    stage,
    grown,
    low_since,
    dead_since,
    bloom_until,
});

impl Plant {
    pub const fn alive(&self) -> bool {
        self.dead_since.is_none()
    }

    /// Its look, while the terrarium as a whole shows `presentation`.
    pub fn look(&self, presentation: PresentationState) -> PlantState {
        if !self.alive() {
            PlantState::Dead
        } else if presentation == PresentationState::Dormant {
            PlantState::Dormant
        } else if self.vigor >= THRIVING_FROM {
            PlantState::Thriving
        } else {
            match self.limit {
                Limit::Light => PlantState::Pale,
                Limit::Soil => PlantState::Wilting,
            }
        }
    }

    fn grow(&mut self, today: &Conditions, target: Q16, limit: Limit, rate: Q16) {
        if !self.alive() {
            return;
        }
        self.vigor = approach(self.vigor, target, rate);
        self.limit = limit;
        if self.vigor < FAILING_BELOW {
            let since = *self.low_since.get_or_insert(today.tick);
            if today.tick.hours_since(since) >= DYING_DAYS * TICKS_PER_DAY {
                self.dead_since = Some(today.tick);
            }
            return;
        }
        self.low_since = None;
        if today.inactive {
            return;
        }
        self.grown = self.grown.saturating_add(1);
        let [young, mature] = Layer::of(self.species).stage_days();
        let stage = if self.grown >= mature {
            GrowthStage::Mature
        } else if self.grown >= young {
            GrowthStage::Young
        } else {
            GrowthStage::Seedling
        };
        self.stage = self.stage.max(stage);
    }
}

/// What one day's close reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conditions {
    /// The close: the first tick of the new UTC day.
    pub tick: Tick,
    /// Smoothed light and soil; `None` while unknown.
    pub light: Option<Q16>,
    pub soil: Option<Q16>,
    /// The rain ratio: 1 is the repository's usual pace.
    pub rain: Q16,
    /// Lines of text at the default tip.
    pub terrain: u64,
    /// Thirty days or more without a commit on any branch.
    pub inactive: bool,
}

/// What a plant grows toward, and what limits it: health scaled by 4/3, health being the lower
/// of light and soil with an unknown one at neutral (I-3). Light limits when it is the lower;
/// soil otherwise.
pub fn vigor_target(light: Option<Q16>, soil: Option<Q16>) -> (Q16, Limit) {
    let limit = if light.unwrap_or(NEUTRAL) < soil.unwrap_or(NEUTRAL) {
        Limit::Light
    } else {
        Limit::Soil
    };
    let target = climate::health(light, soil)
        .saturating_mul(VIGOR_PER_HEALTH)
        .clamp01();
    (target, limit)
}

/// Tempo: the rain ratio held within `[1/4, 2]`. A silent repository still changes, slowly, and
/// a burst cannot rush it; rain is pace, never health.
pub fn tempo(rain: Q16) -> Q16 {
    rain.clamp(TEMPO_MIN, TEMPO_MAX)
}

/// The share of the way to its target vigor moves in a day, and of free capacity that
/// germinates: `tempo / 7`. Residents' energy moves at the same pace.
pub fn daily_rate(rain: Q16) -> Q16 {
    tempo(rain).saturating_mul(GROWTH_RATE)
}

/// One day's step of `vigor` toward `target`: the share `rate` of the way, and at least one
/// epsilon. A rounded average alone stalls a few epsilons short, which would hold a plant whose
/// target is exactly a band's edge below it for good, while a seedling starts at the target.
pub(crate) fn approach(vigor: Q16, target: Q16, rate: Q16) -> Q16 {
    let next = vigor.ema(target, rate);
    if next != vigor {
        next
    } else if vigor < target {
        vigor.saturating_add(Q16::EPSILON)
    } else if vigor > target {
        vigor.saturating_sub(Q16::EPSILON)
    } else {
        vigor
    }
}

/// The terrarium's vegetation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vegetation {
    /// The id the next seedling takes.
    pub next_id: u32,
    /// Ordered by slot.
    pub plants: Vec<Plant>,
    /// When each weed germinated, oldest first.
    pub weeds: Vec<Tick>,
    /// When each piece of litter fell, oldest first.
    pub litter: Vec<Tick>,
    /// When each fungus grew, oldest first.
    pub fungus: Vec<Tick>,
    /// Seeds per pool species. Its keys are the pool.
    pub seed_bank: BTreeMap<PlantSpecies, u32>,
    /// A rain after drought doubles the next close's germination.
    pub boost: bool,
}

canonical_struct!(Vegetation {
    next_id,
    plants,
    weeds,
    litter,
    fungus,
    seed_bank,
    boost,
});

const fn days(n: u32) -> u32 {
    n.saturating_mul(TICKS_PER_DAY)
}

impl Vegetation {
    /// No plants yet, and a seed bank of the species `seed` picks.
    pub fn new(seed: &RepoSeed) -> Vegetation {
        let mut rng = Pcg32::stream(seed, StreamTag::SpeciesPool, 0);
        let mut seed_bank = BTreeMap::new();
        for layer in [Layer::Far, Layer::Mid, Layer::Near] {
            let mut left = layer.species().to_vec();
            for _ in 0..layer.picked() {
                let Some(bound) = u32::try_from(left.len()).ok().and_then(NonZeroU32::new) else {
                    break;
                };
                let picked = usize::try_from(rng.below(bound)).unwrap_or(0);
                seed_bank.insert(left.remove(picked), SEED_BANK_START);
            }
        }
        Vegetation {
            next_id: 1,
            plants: Vec::new(),
            weeds: Vec::new(),
            litter: Vec::new(),
            fungus: Vec::new(),
            seed_bank,
            boost: false,
        }
    }

    /// The pool: the eight species this terrarium grows.
    pub fn pool(&self) -> impl Iterator<Item = PlantSpecies> + '_ {
        self.seed_bank.keys().copied()
    }

    /// One day's close.
    pub fn close(&mut self, seed: &RepoSeed, today: &Conditions) {
        let plants = plant_capacity(today.terrain);
        let weeds = weed_capacity(plants);
        self.fall(today.tick, plants, weeds);
        self.decay(today.tick);
        let (target, limit) = vigor_target(today.light, today.soil);
        let rate = daily_rate(today.rain);
        let germination = if std::mem::take(&mut self.boost) {
            rate.saturating_mul(Q16::from_int(2)).clamp01()
        } else {
            rate
        };
        for plant in &mut self.plants {
            plant.grow(today, target, limit, rate);
        }
        if !today.inactive {
            self.shed_seeds();
        }
        self.tend_weeds(today, plants, weeds);
        if !today.inactive && target >= FAILING_BELOW {
            self.germinate(seed, today.tick, plants, target, limit, germination);
        }
    }

    /// Dead plants that have stood their time, and plants beyond the capacity, fall into litter;
    /// weeds beyond what is left go, the newest first.
    fn fall(&mut self, now: Tick, capacity: u32, weeds: u32) {
        let litter = &mut self.litter;
        self.plants.retain(|plant| {
            let stood = plant
                .dead_since
                .is_some_and(|dead| now.hours_since(dead) >= days(DEAD_STANDING_DAYS));
            let beyond = u32::from(plant.slot) >= capacity;
            if stood || beyond {
                litter.push(now);
            }
            !(stood || beyond)
        });
        let left = capacity.saturating_sub(self.plant_count());
        let kept = usize::try_from(left.min(weeds)).unwrap_or(0);
        self.weeds.truncate(kept);
    }

    /// Old litter turns to fungus, and old fungus is gone.
    fn decay(&mut self, now: Tick) {
        let rotted = self
            .litter
            .iter()
            .take_while(|&&fell| now.hours_since(fell) >= days(LITTER_DAYS))
            .count();
        self.litter.drain(..rotted);
        self.fungus.extend(std::iter::repeat_n(now, rotted));
        let gone = self
            .fungus
            .iter()
            .take_while(|&&grew| now.hours_since(grew) >= days(FUNGUS_DAYS))
            .count();
        self.fungus.drain(..gone);
    }

    /// Each species with a thriving mature plant adds one seed to the bank.
    fn shed_seeds(&mut self) {
        let shedding: BTreeSet<PlantSpecies> = self
            .plants
            .iter()
            .filter(|plant| {
                plant.alive() && plant.stage == GrowthStage::Mature && plant.vigor >= THRIVING_FROM
            })
            .map(|plant| plant.species)
            .collect();
        for species in shedding {
            if let Some(seeds) = self.seed_bank.get_mut(&species) {
                *seeds = seeds.saturating_add(1).min(SEED_BANK_CAP);
            }
        }
    }

    fn tend_weeds(&mut self, today: &Conditions, plants: u32, weeds: u32) {
        match today.soil {
            Some(soil) if soil < WEEDS_BELOW => {
                let room = self.plant_count().saturating_add(self.weed_count()) < plants;
                if room && self.weed_count() < weeds {
                    self.weeds.push(today.tick);
                }
            }
            Some(soil) if soil < WEEDS_WITHER_FROM => {}
            _ => {
                let withered = self.weeds.len().min(1);
                self.weeds.drain(..withered);
            }
        }
    }

    /// Seedlings take the lowest free slots: `⌈free × rate⌉` of them, each of a species of its
    /// slot's layer drawn by the seeds in the bank.
    fn germinate(
        &mut self,
        seed: &RepoSeed,
        now: Tick,
        capacity: u32,
        target: Q16,
        limit: Limit,
        rate: Q16,
    ) {
        let free = capacity
            .saturating_sub(self.plant_count())
            .saturating_sub(self.weed_count());
        let wanted = Q16::from_int(i32::try_from(free).unwrap_or(i32::MAX))
            .saturating_mul(rate)
            .ceil_to_int();
        let wanted = usize::try_from(wanted).unwrap_or(0);
        let taken: Vec<u16> = self.plants.iter().map(|plant| plant.slot).collect();
        let slots: Vec<u16> = (0..u16::try_from(capacity).unwrap_or(u16::MAX))
            .filter(|slot| !taken.contains(slot))
            .take(wanted)
            .collect();
        let mut rng = Pcg32::stream(seed, StreamTag::Germination, u64::from(now.hours()));
        for slot in slots {
            let Some(species) = self.draw(&mut rng, Layer::of_slot(slot)) else {
                continue;
            };
            if let Some(seeds) = self.seed_bank.get_mut(&species) {
                *seeds = seeds.saturating_sub(1).max(SEED_BANK_FLOOR);
            }
            self.plants.push(Plant {
                id: self.next_id,
                species,
                slot,
                born: now,
                vigor: target,
                limit,
                stage: GrowthStage::Seedling,
                grown: 0,
                low_since: None,
                dead_since: None,
                bloom_until: None,
            });
            self.next_id = self.next_id.saturating_add(1);
        }
        self.plants.sort_by_key(|plant| plant.slot);
    }

    /// A species of `layer` from the pool, each as likely as its seeds in the bank.
    fn draw(&self, rng: &mut Pcg32, layer: Layer) -> Option<PlantSpecies> {
        let stock: Vec<(PlantSpecies, u32)> = self
            .seed_bank
            .iter()
            .filter(|(species, _)| Layer::of(**species) == layer)
            .map(|(&species, &seeds)| (species, seeds))
            .collect();
        let total = stock
            .iter()
            .fold(0_u32, |sum, &(_, seeds)| sum.saturating_add(seeds));
        let mut pick = rng.below(NonZeroU32::new(total)?);
        for (species, seeds) in stock {
            if pick < seeds {
                return Some(species);
            }
            pick -= seeds;
        }
        None
    }

    fn plant_count(&self) -> u32 {
        count_of(self.plants.len())
    }

    fn weed_count(&self) -> u32 {
        count_of(self.weeds.len())
    }

    /// A flowering: every living mature plant at thriving vigor blooms until `until`.
    pub fn bloom(&mut self, until: Tick) {
        for plant in &mut self.plants {
            if plant.alive() && plant.stage == GrowthStage::Mature && plant.vigor >= THRIVING_FROM {
                plant.bloom_until = Some(plant.bloom_until.map_or(until, |was| was.max(until)));
            }
        }
    }

    /// A lightning strike: one living mature plant, drawn on a stream keyed by `now`, loses
    /// `dip` of its vigor, never below `FAILING_BELOW`. Returns its id.
    pub fn strike(&mut self, seed: &RepoSeed, now: Tick, dip: Q16) -> Option<u32> {
        let eligible: Vec<usize> = (0..self.plants.len())
            .filter(|&i| {
                self.plants
                    .get(i)
                    .is_some_and(|p| p.alive() && p.stage == GrowthStage::Mature)
            })
            .collect();
        let bound = u32::try_from(eligible.len())
            .ok()
            .and_then(NonZeroU32::new)?;
        let mut rng = Pcg32::stream(seed, StreamTag::Lightning, u64::from(now.hours()));
        let index = usize::try_from(rng.below(bound)).ok()?;
        let plant = self.plants.get_mut(*eligible.get(index)?)?;
        if plant.vigor > FAILING_BELOW {
            plant.vigor = plant.vigor.saturating_sub(dip).max(FAILING_BELOW);
        }
        Some(plant.id)
    }

    /// A sinkhole: one living plant, drawn on a stream keyed by `key`, is swallowed, leaving no
    /// litter. Returns its id.
    pub fn swallow(&mut self, seed: &RepoSeed, key: u64) -> Option<u32> {
        let living: Vec<usize> = (0..self.plants.len())
            .filter(|&i| self.plants.get(i).is_some_and(Plant::alive))
            .collect();
        let bound = u32::try_from(living.len()).ok().and_then(NonZeroU32::new)?;
        let mut rng = Pcg32::stream(seed, StreamTag::Sinkhole, key);
        let index = *living.get(usize::try_from(rng.below(bound)).ok()?)?;
        Some(self.plants.remove(index).id)
    }

    /// A clearing burn takes every weed.
    pub fn burn_weeds(&mut self) {
        self.weeds.clear();
    }

    /// A windstorm brings `pieces` of leaf litter.
    pub fn blow_litter(&mut self, now: Tick, pieces: usize) {
        self.litter.extend(std::iter::repeat_n(now, pieces));
    }

    /// The plants as the composer sees them, in slot order, while the terrarium shows
    /// `presentation` at `now`, with `scorched` the one the playing lightning strikes. A bloom
    /// shows only on a mature, thriving plant.
    pub fn views(
        &self,
        presentation: PresentationState,
        now: Tick,
        scorched: Option<u32>,
    ) -> Vec<PlantView> {
        self.plants
            .iter()
            .map(|plant| {
                let state = plant.look(presentation);
                PlantView {
                    id: plant.id,
                    species: plant.species,
                    stage: plant.stage,
                    state,
                    bloom: plant.bloom_until.is_some_and(|until| now < until)
                        && plant.stage == GrowthStage::Mature
                        && state == PlantState::Thriving,
                    scorched: scorched == Some(plant.id),
                    order: plant.slot,
                }
            })
            .collect()
    }
}
