//! The whole ecology's properties: plants and residents stepped together, as replay steps them,
//! through generated climates and from generated states.

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use proptest::sample::select;

use crate::fauna::{Fauna, Resident, close_living};
use crate::fixed::Q16;
use crate::habitat::resident_capacity;
use crate::scene::{LifeStage, ResidentSpecies};
use crate::test_support::ecology::{Ledger, START, any_state, conditions, day, q, seed, thriving};
use crate::test_support::proptest_config;
use crate::time::{TICKS_PER_DAY, Tick};
use crate::tuning::{BROOD_DAYS, ENERGY_HIGH_FROM, ENERGY_LOW_BELOW, life_days};

/// Whether a resident of `species` may go from `before` to `after` in one close.
fn allowed(species: ResidentSpecies, before: LifeStage, after: LifeStage) -> bool {
    use LifeStage::{Adult, Dying, Egg, Juvenile, Pupa};
    let grown = if species.has_pupa() { Pupa } else { Adult };
    before == after
        || matches!(
            (before, after),
            (Egg, Juvenile) | (Pupa, Adult) | (Adult, Dying)
        )
        || (before == Juvenile && after == grown)
}

/// Checks what a close must leave of the cast, against what the closes before it left: no more
/// residents than the capacity, ids unique, on one species for life and never back once gone,
/// every stage step one its species takes, the dying gone at the next close, and every egg and
/// arrival well fed.
#[derive(Default)]
struct Cast {
    /// Each id's species and stage at the last close.
    seen: BTreeMap<u32, (ResidentSpecies, LifeStage)>,
    gone: BTreeSet<u32>,
}

impl Cast {
    /// A ledger that starts from `fauna` as it stands.
    fn of(fauna: &Fauna) -> Cast {
        Cast {
            seen: fauna
                .residents
                .iter()
                .map(|r| (r.id, (r.species, r.stage)))
                .collect(),
            gone: BTreeSet::new(),
        }
    }

    fn check(&mut self, fauna: &Fauna, terrain: u64) -> Result<(), TestCaseError> {
        let residents = u32::try_from(fauna.residents.len()).expect("small");
        let capacity = resident_capacity(terrain);
        prop_assert!(
            residents <= capacity,
            "{} residents for {}",
            residents,
            capacity
        );
        let ids: Vec<u32> = fauna.residents.iter().map(|r| r.id).collect();
        prop_assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "{:?}", ids);
        for resident in &fauna.residents {
            prop_assert!(resident.id < fauna.next_id);
            prop_assert!(
                !self.gone.contains(&resident.id),
                "id {} came back",
                resident.id
            );
            prop_assert!(resident.stage != LifeStage::Pupa || resident.species.has_pupa());
            match self.seen.get(&resident.id) {
                None => {
                    prop_assert!(matches!(resident.stage, LifeStage::Egg | LifeStage::Adult));
                    prop_assert!(resident.energy >= ENERGY_HIGH_FROM, "{:?}", resident);
                    prop_assert!(fauna.pool.contains(&resident.species));
                }
                Some(&(species, before)) => {
                    prop_assert_eq!(species, resident.species);
                    prop_assert!(before != LifeStage::Dying, "id {} still dying", resident.id);
                    prop_assert!(
                        allowed(species, before, resident.stage),
                        "{:?} {:?} to {:?}",
                        species,
                        before,
                        resident.stage
                    );
                }
            }
        }
        let now: BTreeSet<u32> = ids.into_iter().collect();
        self.gone
            .extend(self.seen.keys().filter(|id| !now.contains(id)).copied());
        self.seen = fauna
            .residents
            .iter()
            .map(|r| (r.id, (r.species, r.stage)))
            .collect();
        Ok(())
    }
}

/// Any cast the sim could hold on `terrain` lines: up to the capacity, of the pool's species,
/// at any stage its species has, any energy, any brood, in or out of a run of low energy, with
/// some arrivals banked.
fn any_cast(terrain: u64) -> impl Strategy<Value = Fauna> {
    let resident = (
        0_usize..5,
        select(LifeStage::ALL),
        0_u32..400,
        0_i32..=65_536,
        prop::option::of(0_u32..20),
        0_i32..=14,
    );
    (
        any::<u8>(),
        prop::collection::vec(resident, 0..=6),
        0_i32..=12,
    )
        .prop_map(move |(byte, residents, arrivals)| {
            let mut fauna = Fauna::new(&seed(byte));
            let capacity = usize::try_from(resident_capacity(terrain)).expect("small");
            let ago = |days: u32| Tick::new(START - days * TICKS_PER_DAY);
            for ((pick, stage, days, energy, low, brood), id) in
                residents.into_iter().take(capacity).zip(1..)
            {
                let species = fauna.pool[pick];
                let stage = if stage == LifeStage::Pupa && !species.has_pupa() {
                    LifeStage::Adult
                } else {
                    stage
                };
                let length = match stage {
                    LifeStage::Egg => life_days(species)[0],
                    LifeStage::Juvenile => life_days(species)[1],
                    LifeStage::Pupa => life_days(species)[2],
                    LifeStage::Adult => life_days(species)[3],
                    LifeStage::Dying => 1,
                };
                fauna.residents.push(Resident {
                    id,
                    species,
                    stage,
                    since: ago(days),
                    stage_days: days % length,
                    energy: Q16::from_raw(energy),
                    low_since: low.map(ago),
                    brood: Q16::from_int(brood).min(BROOD_DAYS),
                });
            }
            fauna.next_id = 7;
            fauna.arrivals = Q16::from_int(arrivals);
            fauna
        })
}

/// The slowest return good climate allows: 72 plants failing so long that they die at the
/// fourth close and stand a month, rain at 0.70 of the usual pace, and no resident left.
#[test]
fn the_slowest_return_still_fills_the_cast_within_60_days() {
    let terrain = 40_000;
    let seed = seed(3);
    let mut vegetation = crate::vegetation::Vegetation::new(&seed);
    let capacity = crate::habitat::plant_capacity(terrain);
    assert_eq!((capacity, resident_capacity(terrain)), (72, 6));
    let failing_since = Tick::new(START - 17 * TICKS_PER_DAY);
    vegetation.plants = (0..u16::try_from(capacity).expect("small"))
        .map(|slot| crate::vegetation::Plant {
            id: u32::from(slot) + 1,
            species: crate::scene::PlantSpecies::Clover,
            slot,
            born: Tick::ZERO,
            vigor: Q16::ZERO,
            limit: crate::vegetation::Limit::Soil,
            stage: crate::scene::GrowthStage::Mature,
            grown: 40,
            low_since: Some(failing_since),
            dead_since: None,
            bloom_until: None,
        })
        .collect();
    vegetation.next_id = capacity + 1;
    let mut fauna = Fauna::new(&seed);
    let mut died = None;
    let mut filled = None;
    for n in 1..=60 {
        let today = crate::vegetation::Conditions {
            light: Some(q(70)),
            soil: Some(q(70)),
            rain: q(70),
            terrain,
            ..day(n)
        };
        close_living(&seed, &today, &mut vegetation, &mut fauna);
        if died.is_none() && vegetation.plants.iter().all(|p| !p.alive()) {
            died = Some(n);
        }
        if filled.is_none() && fauna.residents.len() == 6 {
            filled = Some(n);
        }
    }
    assert_eq!(died, Some(4));
    // Plant food is good again from about the 37th close; the banked arrivals fill the cast at
    // the 54th, six days inside the 60.
    assert_eq!(filled, Some(54));
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn the_cast_stays_within_capacity_and_every_stage_step_is_valid(
        byte in any::<u8>(),
        days in prop::collection::vec(conditions(), 1..150),
    ) {
        let seed = seed(byte);
        let mut vegetation = crate::vegetation::Vegetation::new(&seed);
        let mut fauna = Fauna::new(&seed);
        let (mut plants, mut cast) = (Ledger::default(), Cast::default());
        for (n, (light, soil, rain, terrain, inactive)) in (1..).zip(days) {
            let today = crate::vegetation::Conditions {
                light, soil, rain, terrain, inactive, ..day(n)
            };
            close_living(&seed, &today, &mut vegetation, &mut fauna);
            plants.check(&vegetation, terrain)?;
            cast.check(&fauna, terrain)?;
        }
    }

    #[test]
    fn any_state_recovers_within_60_days_of_good_climate_residents_included(
        (terrain, mut vegetation, mut fauna) in any_state().prop_flat_map(|(terrain, vegetation)| {
            (Just(terrain), Just(vegetation), any_cast(terrain))
        }),
        byte in any::<u8>(),
        days in prop::collection::vec((70_i32..=100, 70_i32..=100, 70_i32..=300), 60),
    ) {
        let seed = seed(byte);
        let mut plants = Ledger::default();
        plants.check(&vegetation, terrain)?;
        let mut cast = Cast::of(&fauna);
        for (n, (light, soil, rain)) in (1..).zip(days) {
            let today = crate::vegetation::Conditions {
                light: Some(q(light)),
                soil: Some(q(soil)),
                rain: q(rain),
                terrain,
                ..day(n)
            };
            close_living(&seed, &today, &mut vegetation, &mut fauna);
            plants.check(&vegetation, terrain)?;
            cast.check(&fauna, terrain)?;
        }
        prop_assert!(thriving(&vegetation, terrain), "{:#?}", vegetation);
        let residents = u32::try_from(fauna.residents.len()).expect("small");
        prop_assert_eq!(residents, resident_capacity(terrain), "{:#?}", fauna);
        prop_assert!(
            fauna.residents.iter().all(|r| r.energy >= ENERGY_LOW_BELOW),
            "{:#?}",
            fauna
        );
    }
}
