use super::*;
use crate::scene::{GrowthStage, PlantSpecies};
use crate::test_support::ecology::{START, day, q, seed};
use crate::vegetation::{Limit, Plant};

use LifeStage::{Adult, Dying, Egg, Juvenile, Pupa};
use ResidentSpecies::{
    Beetle, Earthworm, Firefly, Harvestman, Ladybird, Snail, Springtail, Woodlouse,
};

/// Lines for one resident (and 6 plants), two (10 plants), three, four, and six (24 plants).
const ONE: u64 = 0;
const TWO: u64 = 1_000;
const THREE: u64 = 3_000;
const FOUR: u64 = 7_000;
const SIX: u64 = 31_000;

/// A terrarium of `terrain` lines whose every slot holds a mature plant at `vigor`: plant food
/// is `vigor`.
fn garden(terrain: u64, vigor: Q16) -> Vegetation {
    let capacity = u16::try_from(plant_capacity(terrain)).expect("small");
    Vegetation {
        plants: (0..capacity)
            .map(|slot| Plant {
                id: u32::from(slot) + 1,
                species: PlantSpecies::Clover,
                slot,
                born: Tick::ZERO,
                vigor,
                limit: Limit::Soil,
                stage: GrowthStage::Mature,
                grown: 40,
                low_since: None,
                dead_since: None,
                bloom_until: None,
            })
            .collect(),
        next_id: u32::from(capacity) + 1,
        ..Vegetation::new(&seed(1))
    }
}

/// No plants and nothing decaying: no food at all.
fn bare() -> Vegetation {
    Vegetation::new(&seed(1))
}

fn resident(id: u32, species: ResidentSpecies, stage: LifeStage, energy: Q16) -> Resident {
    Resident {
        id,
        species,
        stage,
        since: Tick::new(START),
        stage_days: 0,
        energy,
        low_since: None,
        brood: Q16::ZERO,
    }
}

/// The fauna `seed` picks, holding `residents`.
fn cast(seed: &RepoSeed, residents: Vec<Resident>) -> Fauna {
    Fauna {
        next_id: residents.iter().map(|r| r.id).max().unwrap_or(0) + 1,
        residents,
        ..Fauna::new(seed)
    }
}

/// Closes `days`, each on `terrain` lines and `vegetation`, with `with` changing that day's
/// conditions.
fn run(
    fauna: &mut Fauna,
    seed: &RepoSeed,
    vegetation: &Vegetation,
    terrain: u64,
    days: std::ops::RangeInclusive<u32>,
    with: impl Fn(&mut Conditions),
) {
    for n in days {
        let mut today = Conditions { terrain, ..day(n) };
        with(&mut today);
        fauna.close(seed, &today, vegetation);
    }
}

fn ids(fauna: &Fauna) -> Vec<u32> {
    fauna.residents.iter().map(|r| r.id).collect()
}

// Expected pools and draws below come from an independent Python model of the stream
// derivation, SplitMix64, PCG32 and Lemire's ranges (checked against `streams_are_known`).

#[test]
fn the_pool_is_known() {
    let pool = |byte_seed: RepoSeed| Fauna::new(&byte_seed).pool;
    assert_eq!(
        pool(seed(0)),
        [Snail, Beetle, Firefly, Earthworm, Harvestman]
    );
    assert_eq!(
        pool(seed(7)),
        [Snail, Beetle, Woodlouse, Springtail, Harvestman]
    );
    let mut counting = [0; 32];
    for (byte, value) in counting.iter_mut().zip(0..) {
        *byte = value;
    }
    assert_eq!(
        pool(RepoSeed::from_bytes(counting)),
        [Snail, Firefly, Woodlouse, Springtail, Harvestman]
    );
    let fresh = Fauna::new(&seed(7));
    assert!(fresh.residents.is_empty());
    assert_eq!((fresh.next_id, fresh.arrivals), (1, Q16::ZERO));
}

#[test]
fn every_pool_meets_the_cast_rules() {
    // Every resident but the firefly is about by day, all are about by night (by their hours),
    // and only the firefly flies (the art): so all 56 sets of five qualify. A trait change that
    // lets a pool break a rule fails here first.
    let pools = admissible_pools();
    assert_eq!(pools.len(), 56);
    assert!(pools.iter().all(|pool| pool.len() == 5 && admissible(pool)));
    let mut seen = pools.clone();
    seen.dedup();
    assert_eq!(seen.len(), 56);
    // The rules themselves: a day species, a night species, and at most one flyer.
    assert!(admissible(&[Firefly, Snail]));
    assert!(!admissible(&[Firefly]));
    assert!(!admissible(&[Firefly, Firefly, Snail]));
    let flyers: Vec<_> = ResidentSpecies::ALL
        .iter()
        .filter(|s| s.flies())
        .map(|s| s.name())
        .collect();
    assert_eq!(flyers, ["Firefly"]);
    let decomposers: Vec<_> = ResidentSpecies::ALL
        .iter()
        .filter(|s| s.decomposer())
        .map(|s| s.name())
        .collect();
    assert_eq!(decomposers, ["Woodlouse", "Springtail", "Earthworm"]);
}

#[test]
fn each_species_lives_through_its_own_stages() {
    for &species in ResidentSpecies::ALL {
        let mut life = resident(1, species, Egg, Q16::ONE);
        let mut changes = Vec::new();
        for active_day in 1..=800 {
            let before = life.stage;
            life.age();
            if life.stage != before {
                changes.push((active_day, life.stage));
            }
        }
        let [egg, juvenile, pupa, adult] = life_days(species);
        let mut expected = vec![(egg, Juvenile)];
        if species.has_pupa() {
            expected.push((egg + juvenile, Pupa));
            expected.push((egg + juvenile + pupa, Adult));
        } else {
            expected.push((egg + juvenile, Adult));
        }
        expected.push((egg + juvenile + pupa + adult, Dying));
        assert_eq!(changes, expected, "{}", species.name());
    }
}

#[test]
fn old_age_shows_dying_for_one_close_and_the_room_refills_at_once() {
    let seed = seed(7);
    let mut adult = resident(1, Snail, Adult, Q16::ONE);
    adult.stage_days = life_days(Snail)[3] - 1;
    let mut fauna = cast(&seed, vec![adult]);
    let food = garden(ONE, Q16::ONE);
    run(&mut fauna, &seed, &food, ONE, 1..=1, |_| {});
    assert_eq!(fauna.residents[0].stage, Dying);
    assert_eq!(fauna.died, 0);
    // The next close buries it, and the arrivals banked over the two days take its room.
    run(&mut fauna, &seed, &food, ONE, 2..=2, |_| {});
    assert_eq!((fauna.died, fauna.arrived), (1, 1));
    assert_eq!(ids(&fauna), [2]);
    assert_eq!(fauna.residents[0].stage, Adult);
    // The dead go while the repository is inactive too.
    let mut dying = cast(&seed, vec![resident(1, Snail, Dying, Q16::ONE)]);
    run(&mut dying, &seed, &food, ONE, 1..=1, |today| {
        today.inactive = true;
    });
    assert!(dying.residents.is_empty());
}

#[test]
fn a_long_hunger_sends_a_resident_away_and_food_in_time_keeps_it() {
    let seed = seed(7);
    let mut fauna = cast(&seed, vec![resident(1, Snail, Adult, Q16::ONE)]);
    let mut low_from = None;
    let mut gone_at = None;
    for n in 1..=60 {
        run(&mut fauna, &seed, &bare(), ONE, n..=n, |_| {});
        match fauna.residents.first() {
            Some(snail) if snail.energy < ENERGY_LOW_BELOW => {
                low_from.get_or_insert(n);
            }
            Some(_) => {}
            None => {
                gone_at.get_or_insert(n);
            }
        }
    }
    let low_from = low_from.expect("it goes hungry");
    assert_eq!(gone_at, Some(low_from + RETREAT_DAYS));
    assert_eq!((fauna.left, fauna.died), (1, 0));
    // Low for a few days, then fed: it recovers in time and stays.
    let mut fed = cast(&seed, vec![resident(1, Snail, Adult, Q16::ONE)]);
    run(&mut fed, &seed, &bare(), ONE, 1..=low_from + 3, |_| {});
    assert!(fed.residents[0].low_since.is_some());
    let fed_from = low_from + 4;
    run(
        &mut fed,
        &seed,
        &garden(ONE, Q16::ONE),
        ONE,
        fed_from..=fed_from + 30,
        |_| {},
    );
    assert_eq!(ids(&fed), [1]);
    assert_eq!(fed.residents[0].low_since, None);
    assert_eq!(energy_band(fed.residents[0].energy), Energy::High);
}

#[test]
fn decomposers_live_on_decay_but_never_breed() {
    let seed = seed(7);
    // Every plant fallen to litter: nothing living to eat, something decaying.
    let decay = Vegetation {
        litter: vec![Tick::new(START)],
        ..bare()
    };
    assert!(decaying(&decay) && !decaying(&bare()));
    let standing = Vegetation {
        plants: garden(TWO, Q16::ONE)
            .plants
            .into_iter()
            .map(|plant| Plant {
                dead_since: Some(Tick::new(START)),
                ..plant
            })
            .collect(),
        ..bare()
    };
    assert!(decaying(&standing));
    assert_eq!(plant_food(&standing, plant_capacity(TWO)), Q16::ZERO);
    let mut fauna = cast(
        &seed,
        vec![
            resident(1, Woodlouse, Adult, Q16::ONE),
            resident(2, Snail, Adult, Q16::ONE),
        ],
    );
    run(&mut fauna, &seed, &decay, TWO, 1..=90, |_| {});
    // The snail left; the woodlouse holds at the decay's middling food, with no egg and no
    // arrival.
    assert_eq!(ids(&fauna), [1]);
    assert_eq!(energy_band(fauna.residents[0].energy), Energy::Mid);
    assert_eq!((fauna.laid, fauna.arrived, fauna.left), (0, 0, 1));
    // When the decay is gone, it goes hungry too.
    run(&mut fauna, &seed, &bare(), TWO, 91..=150, |_| {});
    assert!(fauna.residents.is_empty());
}

#[test]
fn a_ready_adult_lays_only_when_well_fed_with_room_and_few_kin() {
    let seed = seed(7);
    let ready = |id, species| Resident {
        brood: BROOD_DAYS,
        ..resident(id, species, Adult, Q16::ONE)
    };
    let food = garden(SIX, Q16::ONE);
    // Room and food: an egg of its kind on the first close, at its parent's energy.
    let mut laying = cast(&seed, vec![ready(1, Beetle)]);
    run(&mut laying, &seed, &food, SIX, 1..=1, |_| {});
    let egg = &laying.residents[1];
    assert_eq!((egg.id, egg.species, egg.stage), (2, Beetle, Egg));
    assert_eq!(egg.energy, laying.residents[0].energy);
    assert_eq!(laying.residents[0].brood, Q16::ZERO);
    assert_eq!(laying.laid, 1);
    // No room.
    let mut full = cast(&seed, vec![ready(1, Beetle)]);
    run(&mut full, &seed, &garden(ONE, Q16::ONE), ONE, 1..=1, |_| {});
    assert_eq!(full.laid, 0);
    // Its kind already two.
    let mut kin = cast(
        &seed,
        vec![ready(1, Beetle), resident(2, Beetle, Juvenile, Q16::ONE)],
    );
    run(&mut kin, &seed, &food, SIX, 1..=1, |_| {});
    assert_eq!(kin.laid, 0);
    // Not well fed.
    let mut hungry = cast(
        &seed,
        vec![Resident {
            energy: q(50),
            ..ready(1, Beetle)
        }],
    );
    run(&mut hungry, &seed, &garden(SIX, q(50)), SIX, 1..=1, |_| {});
    assert_eq!(hungry.laid, 0);
    // With room for one, the oldest ready adult lays.
    let mut two = cast(&seed, vec![ready(1, Snail), ready(2, Beetle)]);
    run(
        &mut two,
        &seed,
        &garden(THREE, Q16::ONE),
        THREE,
        1..=1,
        |_| {},
    );
    assert_eq!(two.residents[2].species, Snail);
    assert_eq!(two.residents[1].brood, BROOD_DAYS);
}

#[test]
fn a_brood_takes_14_days_at_the_usual_pace() {
    let seed = seed(7);
    let food = garden(ONE, Q16::ONE);
    let days_to_brood = |rain: Q16| {
        let mut fauna = cast(&seed, vec![resident(1, Snail, Adult, Q16::ONE)]);
        (1..=100)
            .find(|&n| {
                run(&mut fauna, &seed, &food, ONE, n..=n, |today| {
                    today.rain = rain;
                });
                fauna.residents[0].brood >= BROOD_DAYS
            })
            .expect("a brood is due")
    };
    assert_eq!(days_to_brood(Q16::ONE), 14);
    assert_eq!(days_to_brood(Q16::from_int(5)), 7);
    assert_eq!(days_to_brood(Q16::ZERO), 56);
}

#[test]
fn arrivals_bring_the_least_present_species_every_two_days() {
    let seed = seed(7);
    let mut fauna = Fauna::new(&seed);
    let food = garden(SIX, Q16::ONE);
    let mut came = Vec::new();
    for n in 1..=14 {
        let before = fauna.residents.len();
        run(&mut fauna, &seed, &food, SIX, n..=n, |_| {});
        came.extend(
            fauna.residents[before..]
                .iter()
                .map(|r| (n, r.id, r.species)),
        );
    }
    assert_eq!(
        came,
        [
            (2, 1, Beetle),
            (4, 2, Snail),
            (6, 3, Springtail),
            (8, 4, Woodlouse),
            (10, 5, Harvestman),
            (12, 6, Beetle),
        ]
    );
    assert!(
        fauna
            .residents
            .iter()
            .all(|r| r.stage == Adult && r.energy == Q16::ONE)
    );
    // With the cast full, the bank fills, up to two days for each resident it holds.
    assert_eq!(fauna.arrivals, Q16::from_int(2));
    run(&mut fauna, &seed, &food, SIX, 15..=40, |_| {});
    assert_eq!(fauna.arrivals, Q16::from_int(12));
    // The day the plants stop feeding well, the bank is gone.
    run(&mut fauna, &seed, &garden(SIX, q(59)), SIX, 41..=41, |_| {});
    assert_eq!(fauna.arrivals, Q16::ZERO);
}

#[test]
fn a_late_double_death_at_a_full_cast_is_refilled_at_once() {
    let seed = seed(7);
    let old = |id, days_left: u32| Resident {
        stage_days: life_days(Snail)[3] - days_left,
        ..resident(id, Snail, Adult, Q16::ONE)
    };
    let mut fauna = Fauna {
        arrivals: Q16::from_int(12),
        ..cast(
            &seed,
            vec![
                old(1, 1),
                old(2, 2),
                resident(3, Beetle, Adult, Q16::ONE),
                resident(4, Beetle, Adult, Q16::ONE),
                resident(5, Woodlouse, Adult, Q16::ONE),
                resident(6, Woodlouse, Adult, Q16::ONE),
            ],
        )
    };
    let food = garden(SIX, Q16::ONE);
    for n in 1..=6 {
        run(&mut fauna, &seed, &food, SIX, n..=n, |_| {});
        assert_eq!(fauna.residents.len(), 6, "close {n}");
    }
    assert_eq!((fauna.died, fauna.arrived), (2, 2));
}

#[test]
fn inactivity_pauses_ageing_and_births_but_not_leaving() {
    let seed = seed(7);
    let ready = Resident {
        brood: BROOD_DAYS.saturating_sub(Q16::ONE),
        stage_days: 3,
        ..resident(2, Beetle, Adult, Q16::ONE)
    };
    let mut fauna = cast(
        &seed,
        vec![resident(1, Snail, Egg, Q16::ONE), ready.clone()],
    );
    run(
        &mut fauna,
        &seed,
        &garden(SIX, Q16::ONE),
        SIX,
        1..=30,
        |today| {
            today.inactive = true;
        },
    );
    assert_eq!(fauna.residents[0].stage_days, 0);
    assert_eq!(fauna.residents[1], ready);
    assert_eq!((fauna.laid, fauna.arrived), (0, 0));
    assert_eq!(fauna.arrivals, Q16::ZERO);
    // A hungry resident still leaves.
    run(&mut fauna, &seed, &bare(), SIX, 31..=80, |today| {
        today.inactive = true;
    });
    assert!(fauna.residents.is_empty());
    assert_eq!(fauna.left, 2);
}

#[test]
fn a_shrinking_terrain_sends_the_newest_away() {
    let seed = seed(7);
    let mut fauna = cast(
        &seed,
        (1..=4)
            .map(|id| resident(id, Snail, Juvenile, Q16::ONE))
            .collect(),
    );
    run(
        &mut fauna,
        &seed,
        &garden(FOUR, Q16::ONE),
        FOUR,
        1..=1,
        |_| {},
    );
    assert_eq!(ids(&fauna), [1, 2, 3, 4]);
    run(
        &mut fauna,
        &seed,
        &garden(TWO, Q16::ONE),
        TWO,
        2..=2,
        |_| {},
    );
    assert_eq!(ids(&fauna), [1, 2]);
    assert_eq!(fauna.left, 2);
}

#[test]
fn food_is_the_plants_vigor_over_at_least_half_their_room() {
    // Ten plants of room: a full garden feeds at its vigor; three plants feed as if over five.
    assert_eq!(plant_food(&garden(TWO, q(80)), 10), q(80));
    let sparse = Vegetation {
        plants: garden(TWO, Q16::ONE).plants.into_iter().take(3).collect(),
        ..bare()
    };
    assert_eq!(plant_food(&sparse, 10), q(60));
    assert_eq!(plant_food(&bare(), 10), Q16::ZERO);
    // Energy reads in vigor's bands.
    assert_eq!(energy_band(ENERGY_HIGH_FROM), Energy::High);
    assert_eq!(
        energy_band(ENERGY_HIGH_FROM.saturating_sub(Q16::EPSILON)),
        Energy::Mid
    );
    assert_eq!(energy_band(ENERGY_LOW_BELOW), Energy::Mid);
    assert_eq!(
        energy_band(ENERGY_LOW_BELOW.saturating_sub(Q16::EPSILON)),
        Energy::Low
    );
    // A decomposer eats decay at no less than its middling food.
    let food = Food {
        plants: q(20),
        decay: true,
    };
    assert_eq!(
        (food.of(Earthworm), food.of(Ladybird)),
        (DETRITUS_FOOD, q(20))
    );
}

#[test]
fn views_follow_the_residents_by_id() {
    let seed = seed(7);
    let fauna = cast(
        &seed,
        vec![
            resident(3, Beetle, Pupa, q(70)),
            resident(9, Woodlouse, Egg, q(10)),
        ],
    );
    let views = fauna.views();
    assert_eq!(
        views,
        [
            ResidentView {
                id: 3,
                species: Beetle,
                stage: Pupa,
                energy: Energy::High,
            },
            ResidentView {
                id: 9,
                species: Woodlouse,
                stage: Egg,
                energy: Energy::Low,
            },
        ]
    );
}
