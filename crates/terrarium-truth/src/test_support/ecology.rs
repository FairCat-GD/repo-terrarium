//! The vegetation's test helpers, shared by its own tests and the whole ecology's: a day's
//! conditions, generated climates and states, and the ledger every close is checked against.

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use proptest::sample::select;

use crate::fixed::Q16;
use crate::habitat::{plant_capacity, weed_capacity};
use crate::identity::RepoSeed;
use crate::scene::{GrowthStage, PlantSpecies};
use crate::time::{TICKS_PER_DAY, Tick};
use crate::tuning::{SEED_BANK_CAP, SEED_BANK_FLOOR, THRIVING_FROM};
use crate::vegetation::{Conditions, Limit, Plant, Vegetation};

/// A day well into a terrarium's life, so generated ages fit before it.
pub const START: u32 = 500 * TICKS_PER_DAY;

pub fn q(percent: i32) -> Q16 {
    Q16::from_ratio(percent, 100)
}

pub fn seed(byte: u8) -> RepoSeed {
    RepoSeed::from_bytes([byte; 32])
}

/// Day `day`'s close, after `START`, with no light or soil tool, the usual rain and 1,000
/// lines (30 plants, 2 residents), while the repository is active.
pub fn day(day: u32) -> Conditions {
    Conditions {
        tick: Tick::new(START + day * TICKS_PER_DAY),
        light: None,
        soil: None,
        rain: Q16::ONE,
        terrain: 1_000,
        inactive: false,
    }
}

/// Every slot of the capacity holds a living plant at thriving vigor, and no weed is left.
pub fn thriving(vegetation: &Vegetation, terrain: u64) -> bool {
    vegetation.weeds.is_empty()
        && u32::try_from(vegetation.plants.len()) == Ok(plant_capacity(terrain))
        && vegetation
            .plants
            .iter()
            .all(|plant| plant.alive() && plant.vigor >= THRIVING_FROM)
}

pub fn reading() -> impl Strategy<Value = Option<Q16>> {
    prop::option::of((0_i32..=65_536).prop_map(Q16::from_raw))
}

pub fn conditions() -> impl Strategy<Value = (Option<Q16>, Option<Q16>, Q16, u64, bool)> {
    (
        reading(),
        reading(),
        (0_i32..=4 << 16).prop_map(Q16::from_raw),
        select(vec![0_u64, 500, 3_000, 15_000, 40_000, 200_000, 1_000_000]),
        // Inactive about one day in seven.
        (0_u8..7).prop_map(|n| n == 0),
    )
}

/// Any state the sim could hold: plants in some of the capacity's slots with any vigor, stage,
/// failing or dead run, weeds within what is left, litter, fungus and a bank above its floor.
pub fn any_state() -> impl Strategy<Value = (u64, Vegetation)> {
    let plant = (
        any::<bool>(),
        0_usize..12,
        0_i32..=65_536,
        0_u32..3,
        0_u32..80,
        prop::option::of(0_u32..40),
        prop::option::of(0_u32..40),
        any::<bool>(),
    );
    (
        0_u64..100_000,
        any::<u8>(),
        prop::collection::vec(plant, 72),
        0_usize..24,
        prop::collection::vec(0_u32..30, 0..20),
        prop::collection::vec(0_u32..80, 0..20),
        prop::collection::vec(SEED_BANK_FLOOR..=SEED_BANK_CAP, 8),
    )
        .prop_map(|(terrain, byte, plants, weeds, litter, fungus, bank)| {
            let mut vegetation = Vegetation::new(&seed(byte));
            let capacity = usize::try_from(plant_capacity(terrain)).expect("small");
            let ago = |days: u32| Tick::new(START - days * TICKS_PER_DAY);
            for ((slot, (occupied, species, vigor, stage, grown, low, dead, light)), id) in
                (0_u16..).zip(plants).take(capacity).zip(1..)
            {
                if !occupied {
                    continue;
                }
                vegetation.plants.push(Plant {
                    id,
                    species: PlantSpecies::ALL[species],
                    slot,
                    born: ago(grown + 1),
                    vigor: Q16::from_raw(vigor),
                    limit: if light { Limit::Light } else { Limit::Soil },
                    stage: GrowthStage::ALL[usize::try_from(stage).expect("small")],
                    grown,
                    low_since: low.map(ago),
                    dead_since: dead.map(ago),
                    bloom_until: None,
                });
            }
            vegetation.next_id = 73;
            let room = capacity - vegetation.plants.len();
            let weeds = weeds
                .min(room)
                .min(usize::try_from(weed_capacity(plant_capacity(terrain))).expect("small"));
            vegetation.weeds = vec![ago(3); weeds];
            let mut litter: Vec<Tick> = litter.into_iter().map(ago).collect();
            litter.sort();
            vegetation.litter = litter;
            let mut fungus: Vec<Tick> = fungus.into_iter().map(ago).collect();
            fungus.sort();
            vegetation.fungus = fungus;
            for (seeds, count) in vegetation.seed_bank.values_mut().zip(bank) {
                *seeds = count;
            }
            (terrain, vegetation)
        })
}

/// Checks what a close must leave, against what the closes before it left: populations within
/// capacity, slots in range and taken once, and each id unique, on one plant for life, and
/// never back once gone.
#[derive(Default)]
pub struct Ledger {
    /// Each id's species, slot and germination.
    seen: BTreeMap<u32, (PlantSpecies, u16, Tick)>,
    gone: BTreeSet<u32>,
}

impl Ledger {
    pub fn check(&mut self, vegetation: &Vegetation, terrain: u64) -> Result<(), TestCaseError> {
        let plants = u32::try_from(vegetation.plants.len()).expect("small");
        let weeds = u32::try_from(vegetation.weeds.len()).expect("small");
        let capacity = plant_capacity(terrain);
        prop_assert!(plants <= capacity, "{} plants for {}", plants, capacity);
        prop_assert!(weeds <= weed_capacity(capacity));
        prop_assert!(plants + weeds <= capacity);
        let slots: Vec<u16> = vegetation.plants.iter().map(|p| p.slot).collect();
        prop_assert!(
            slots.windows(2).all(|pair| pair[0] < pair[1]),
            "{:?}",
            slots
        );
        prop_assert!(slots.iter().all(|&slot| u32::from(slot) < capacity));
        prop_assert!(vegetation.seed_bank.values().all(|&n| n >= SEED_BANK_FLOOR));
        let now: BTreeSet<u32> = vegetation.plants.iter().map(|p| p.id).collect();
        prop_assert_eq!(now.len(), vegetation.plants.len());
        for plant in &vegetation.plants {
            prop_assert!(plant.id < vegetation.next_id);
            prop_assert!(!self.gone.contains(&plant.id), "id {} came back", plant.id);
            let identity = (plant.species, plant.slot, plant.born);
            prop_assert_eq!(*self.seen.entry(plant.id).or_insert(identity), identity);
        }
        self.gone
            .extend(self.seen.keys().filter(|id| !now.contains(id)).copied());
        Ok(())
    }
}
