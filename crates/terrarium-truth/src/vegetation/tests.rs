use proptest::prelude::*;

use super::*;
use crate::test_support::ecology::{
    Ledger, START, any_state, conditions, day, q, reading, seed, thriving,
};
use crate::test_support::proptest_config;

/// `days` closes from day 1 on, each `with` that day's conditions changed.
fn run(
    vegetation: &mut Vegetation,
    seed: &RepoSeed,
    days: std::ops::RangeInclusive<u32>,
    with: impl Fn(&mut Conditions),
) {
    for n in days {
        let mut today = day(n);
        with(&mut today);
        vegetation.close(seed, &today);
    }
}

fn looks(vegetation: &Vegetation) -> Vec<PlantState> {
    vegetation
        .views(PresentationState::Thriving, Tick::new(START), None)
        .iter()
        .map(|view| view.state)
        .collect()
}

fn plant(slot: u16, vigor: Q16, limit: Limit) -> Plant {
    Plant {
        id: u32::from(slot) + 1,
        species: PlantSpecies::Clover,
        slot,
        born: Tick::ZERO,
        vigor,
        limit,
        stage: GrowthStage::Mature,
        grown: 40,
        low_since: None,
        dead_since: None,
        bloom_until: None,
    }
}

// Expected pools and draws below come from an independent Python model of the stream
// derivation, SplitMix64, PCG32 and Lemire's ranges (checked against `streams_are_known`).

#[test]
fn the_pool_is_known() {
    use PlantSpecies::*;
    let pool =
        |byte_seed: RepoSeed| -> Vec<PlantSpecies> { Vegetation::new(&byte_seed).pool().collect() };
    assert_eq!(
        pool(seed(0)),
        [
            Birch, Rowan, Fern, Bramble, Foxglove, MossMound, Clover, Thistle
        ]
    );
    assert_eq!(
        pool(seed(7)),
        [
            Birch,
            Rowan,
            TallGrass,
            Foxglove,
            Reed,
            MossMound,
            WildflowerCluster,
            Thistle
        ]
    );
    let mut counting = [0; 32];
    for (byte, value) in counting.iter_mut().zip(0..) {
        *byte = value;
    }
    assert_eq!(
        pool(RepoSeed::from_bytes(counting)),
        [
            Oak,
            Birch,
            Fern,
            TallGrass,
            Foxglove,
            MossMound,
            WildflowerCluster,
            Clover
        ]
    );
    let fresh = Vegetation::new(&seed(7));
    assert!(fresh.seed_bank.values().all(|&n| n == SEED_BANK_START));
    assert_eq!(fresh.next_id, 1);
}

#[test]
fn germination_is_known() {
    use PlantSpecies::*;
    let seed = seed(7);
    let mut vegetation = Vegetation::new(&seed);
    // Six plants of capacity, one seedling a day (⌈free / 7⌉), at the closes 24 hours apart
    // from tick 24, as the model has them.
    for n in 1..=6 {
        let today = Conditions {
            tick: Tick::new(24 * n),
            terrain: 0,
            ..day(0)
        };
        vegetation.close(&seed, &today);
    }
    let planted: Vec<(u16, u32, PlantSpecies)> = vegetation
        .plants
        .iter()
        .map(|p| (p.slot, p.id, p.species))
        .collect();
    assert_eq!(
        planted,
        [
            (0, 1, MossMound),
            (1, 2, Foxglove),
            (2, 3, Birch),
            (3, 4, WildflowerCluster),
            (4, 5, Foxglove),
            (5, 6, MossMound)
        ]
    );
    let bank: Vec<u32> = vegetation.seed_bank.values().copied().collect();
    // Birch, Rowan, TallGrass, Foxglove, Reed, MossMound, WildflowerCluster, Thistle.
    assert_eq!(bank, [3, 4, 4, 2, 4, 2, 3, 4]);
}

#[test]
fn the_target_is_health_by_four_thirds_and_names_the_lower_climate() {
    // No tools: neutral health, thriving plants.
    assert_eq!(
        vigor_target(None, None),
        (q(50).saturating_mul(VIGOR_PER_HEALTH), Limit::Soil)
    );
    assert!(vigor_target(None, None).0 >= THRIVING_FROM);
    // The unhealthy threshold is the thriving one.
    assert!(vigor_target(Some(q(45)), Some(q(90))).0 >= THRIVING_FROM);
    assert!(vigor_target(Some(q(44)), Some(q(90))).0 < THRIVING_FROM);
    assert_eq!(vigor_target(Some(q(40)), None).1, Limit::Light);
    assert_eq!(vigor_target(None, Some(q(40))).1, Limit::Soil);
    assert_eq!(vigor_target(Some(q(60)), Some(q(60))).1, Limit::Soil);
    assert!(vigor_target(Some(q(26)), Some(q(90))).0 < FAILING_BELOW);
    assert!(vigor_target(Some(q(27)), Some(q(90))).0 >= FAILING_BELOW);
    assert_eq!(vigor_target(Some(q(90)), Some(q(80))).0, Q16::ONE);
    // Tempo is the rain ratio, held within [1/4, 2].
    assert_eq!(daily_rate(Q16::ZERO), daily_rate(TEMPO_MIN));
    assert_eq!(daily_rate(Q16::from_int(9)), daily_rate(TEMPO_MAX));
    assert_eq!(daily_rate(Q16::ONE), GROWTH_RATE);
}

#[test]
fn a_repository_without_tools_grows_a_full_thriving_terrarium() {
    let seed = seed(3);
    let mut vegetation = Vegetation::new(&seed);
    run(&mut vegetation, &seed, 1..=60, |_| {});
    assert!(thriving(&vegetation, 1_000), "{vegetation:#?}");
    // The last seedlings germinated on day 2 or so; trees take 60 days to mature.
    run(&mut vegetation, &seed, 61..=70, |_| {});
    assert!(
        vegetation
            .plants
            .iter()
            .all(|p| p.stage == GrowthStage::Mature)
    );
    // Slots run one far, two mid and three near in every six.
    let layers: Vec<Layer> = vegetation
        .plants
        .iter()
        .map(|p| Layer::of(p.species))
        .collect();
    assert_eq!(layers, (0..10).map(Layer::of_slot).collect::<Vec<_>>());
    assert_eq!(
        (0..6)
            .filter(|&slot| Layer::of_slot(slot) == Layer::Far)
            .count(),
        1
    );
    assert_eq!(
        (0..6)
            .filter(|&slot| Layer::of_slot(slot) == Layer::Mid)
            .count(),
        2
    );
}

#[test]
fn the_look_follows_vigor_its_limit_and_dormancy() {
    use PresentationState::{Dead, Dormant, Struggling};
    let thriving = plant(0, THRIVING_FROM, Limit::Light);
    let pale = plant(1, THRIVING_FROM.saturating_sub(Q16::EPSILON), Limit::Light);
    let wilting = plant(2, q(10), Limit::Soil);
    let dead = Plant {
        dead_since: Some(Tick::ZERO),
        ..plant(3, Q16::ONE, Limit::Soil)
    };
    assert_eq!(thriving.look(Struggling), PlantState::Thriving);
    assert_eq!(pale.look(Struggling), PlantState::Pale);
    assert_eq!(wilting.look(Struggling), PlantState::Wilting);
    assert_eq!(dead.look(Struggling), PlantState::Dead);
    // A dead terrarium's living plants keep their own look.
    assert_eq!(pale.look(Dead), PlantState::Pale);
    assert_eq!(thriving.look(Dormant), PlantState::Dormant);
    assert_eq!(wilting.look(Dormant), PlantState::Dormant);
    assert_eq!(dead.look(Dormant), PlantState::Dead);
}

#[test]
fn a_bloom_shows_only_on_a_mature_thriving_plant_until_it_ends() {
    let until = Tick::new(100);
    let blooming = |stage, vigor| Vegetation {
        plants: vec![Plant {
            stage,
            bloom_until: Some(until),
            ..plant(0, vigor, Limit::Soil)
        }],
        ..Vegetation::new(&seed(1))
    };
    let bloom = |vegetation: Vegetation, presentation, now| {
        vegetation.views(presentation, Tick::new(now), None)[0].bloom
    };
    let thriving = PresentationState::Thriving;
    assert!(bloom(blooming(GrowthStage::Mature, Q16::ONE), thriving, 99));
    assert!(!bloom(
        blooming(GrowthStage::Mature, Q16::ONE),
        thriving,
        100
    ));
    assert!(!bloom(blooming(GrowthStage::Young, Q16::ONE), thriving, 99));
    assert!(!bloom(blooming(GrowthStage::Mature, q(50)), thriving, 99));
    assert!(!bloom(
        blooming(GrowthStage::Mature, Q16::ONE),
        PresentationState::Dormant,
        99
    ));
    let view = blooming(GrowthStage::Mature, Q16::ONE).views(thriving, Tick::ZERO, None)[0];
    assert_eq!((view.id, view.order, view.scorched), (1, 0, false));
}

#[test]
fn a_failing_plant_dies_after_three_weeks_stands_a_month_and_rots() {
    let seed = seed(5);
    let mut vegetation = Vegetation::new(&seed);
    run(&mut vegetation, &seed, 1..=60, |_| {});
    let planted = vegetation.plants.len();
    // Coverage collapses: the target falls to 0.133, and nothing germinates. Every plant has
    // the same vigor by now, so they fail together; record the day each step completes.
    let (mut low, mut died, mut fell, mut rotted, mut cleared) = (None, None, None, None, None);
    for n in 61..=260 {
        let mut today = day(n);
        today.light = Some(q(10));
        vegetation.close(&seed, &today);
        let alive = vegetation.plants.iter().filter(|p| p.alive()).count();
        let failing = vegetation.plants.iter().all(|p| p.low_since.is_some());
        let mark = |step: &mut Option<u32>, reached: bool| {
            if step.is_none() && reached {
                *step = Some(n);
            }
        };
        mark(&mut low, failing && !vegetation.plants.is_empty());
        mark(&mut died, alive == 0);
        mark(&mut fell, vegetation.litter.len() == planted);
        mark(&mut rotted, vegetation.fungus.len() == planted);
        mark(
            &mut cleared,
            rotted.is_some() && vegetation.fungus.is_empty(),
        );
    }
    let low = low.expect("the plants fail");
    let steps = [died, fell, rotted, cleared].map(|step| step.map(|n| n - low));
    let expected = [
        DYING_DAYS,
        DYING_DAYS + DEAD_STANDING_DAYS,
        DYING_DAYS + DEAD_STANDING_DAYS + LITTER_DAYS,
        DYING_DAYS + DEAD_STANDING_DAYS + LITTER_DAYS + FUNGUS_DAYS,
    ];
    assert_eq!(steps, expected.map(Some));
    // Nothing is left but the seed bank.
    assert!(vegetation.plants.is_empty() && vegetation.litter.is_empty());
    assert!(vegetation.fungus.is_empty() && vegetation.weeds.is_empty());
}

#[test]
fn vigor_reaches_a_target_at_a_bands_edge() {
    // The slowest pace and a target exactly at thriving: a plant just below still gets there,
    // as a seedling germinated at that target starts there.
    let mut almost = plant(
        0,
        THRIVING_FROM.saturating_sub(Q16::from_raw(20)),
        Limit::Light,
    );
    let slowest = daily_rate(Q16::ZERO);
    for n in 1..=20 {
        almost.grow(&day(n), THRIVING_FROM, Limit::Light, slowest);
    }
    assert_eq!(almost.vigor, THRIVING_FROM);
    assert_eq!(
        almost.look(PresentationState::Thriving),
        PlantState::Thriving
    );
    // And from above it settles on the target too, never past it.
    let mut above = plant(
        1,
        THRIVING_FROM.saturating_add(Q16::from_raw(20)),
        Limit::Light,
    );
    for n in 1..=20 {
        above.grow(&day(n), THRIVING_FROM, Limit::Light, slowest);
    }
    assert_eq!(above.vigor, THRIVING_FROM);
}

#[test]
fn a_failing_plant_that_recovers_in_time_lives() {
    let seed = seed(5);
    let mut vegetation = Vegetation::new(&seed);
    run(&mut vegetation, &seed, 1..=60, |_| {});
    run(&mut vegetation, &seed, 61..=80, |today| {
        today.light = Some(q(10));
    });
    assert!(vegetation.plants.iter().all(|p| p.low_since.is_some()));
    run(&mut vegetation, &seed, 81..=140, |_| {});
    assert!(thriving(&vegetation, 1_000));
    assert!(vegetation.litter.is_empty());
}

#[test]
fn pale_and_wilting_name_the_limiting_climate() {
    let seed = seed(9);
    let mut vegetation = Vegetation::new(&seed);
    run(&mut vegetation, &seed, 1..=60, |_| {});
    run(&mut vegetation, &seed, 61..=90, |today| {
        today.light = Some(q(35));
        today.soil = Some(q(80));
    });
    assert!(
        looks(&vegetation)
            .iter()
            .all(|&look| look == PlantState::Pale)
    );
    run(&mut vegetation, &seed, 91..=120, |today| {
        today.light = Some(q(80));
        today.soil = Some(q(35));
    });
    assert!(
        looks(&vegetation)
            .iter()
            .all(|&look| look == PlantState::Wilting)
    );
}

#[test]
fn weeds_take_poor_soil_and_wither_as_it_recovers() {
    let seed = seed(2);
    let mut vegetation = Vegetation::new(&seed);
    // Poor soil from the start: a weed a day, before the seedlings, up to W = 3 for P = 10.
    run(&mut vegetation, &seed, 1..=20, |today| {
        today.soil = Some(q(30));
    });
    assert_eq!(vegetation.weeds.len(), 3);
    assert_eq!(vegetation.plants.len(), 7);
    // Middling soil holds them.
    run(&mut vegetation, &seed, 21..=40, |today| {
        today.soil = Some(q(50));
    });
    assert_eq!(vegetation.weeds.len(), 3);
    // Good or unknown soil withers one a day, and seedlings take the room.
    run(&mut vegetation, &seed, 41..=42, |today| {
        today.soil = Some(q(60));
    });
    assert_eq!(vegetation.weeds.len(), 1);
    run(&mut vegetation, &seed, 43..=60, |_| {});
    assert!(thriving(&vegetation, 1_000));
    // With one slot's room left on poor soil, the weed takes it before a seedling can.
    let mut crowded = Vegetation {
        plants: (0..9)
            .map(|slot| plant(slot, Q16::ONE, Limit::Soil))
            .collect(),
        next_id: 10,
        ..Vegetation::new(&seed)
    };
    run(&mut crowded, &seed, 1..=1, |today| {
        today.soil = Some(q(30));
    });
    assert_eq!((crowded.plants.len(), crowded.weeds.len()), (9, 1));
}

#[test]
fn a_species_with_thriving_mature_plants_sheds_one_seed_a_day_up_to_the_cap() {
    let seed = seed(0);
    // Six slots, all taken, so nothing germinates: four mature thriving clovers, one young
    // thistle and one pale moss mound.
    let mut plants: Vec<Plant> = (0..4)
        .map(|slot| plant(slot, Q16::ONE, Limit::Soil))
        .collect();
    plants.push(Plant {
        species: PlantSpecies::Thistle,
        stage: GrowthStage::Young,
        grown: 0,
        ..plant(4, Q16::ONE, Limit::Soil)
    });
    plants.push(Plant {
        species: PlantSpecies::MossMound,
        ..plant(5, q(40), Limit::Light)
    });
    let mut vegetation = Vegetation {
        plants,
        next_id: 7,
        ..Vegetation::new(&seed)
    };
    let bank = |vegetation: &Vegetation, species| vegetation.seed_bank[&species];
    let terrain = |today: &mut Conditions| today.terrain = 0;
    run(&mut vegetation, &seed, 1..=1, terrain);
    assert_eq!(bank(&vegetation, PlantSpecies::Clover), SEED_BANK_START + 1);
    assert_eq!(bank(&vegetation, PlantSpecies::Thistle), SEED_BANK_START);
    assert_eq!(bank(&vegetation, PlantSpecies::MossMound), SEED_BANK_START);
    run(&mut vegetation, &seed, 2..=30, terrain);
    assert_eq!(bank(&vegetation, PlantSpecies::Clover), SEED_BANK_CAP);
}

#[test]
fn a_shrinking_terrain_drops_the_highest_slots_and_the_newest_weeds() {
    let seed = seed(4);
    let weeds: Vec<Tick> = (1..=5).map(|n| Tick::new(START - n * 24)).rev().collect();
    let mut vegetation = Vegetation {
        plants: [0, 1, 12, 13]
            .into_iter()
            .map(|slot| plant(slot, Q16::ONE, Limit::Soil))
            .collect(),
        weeds: weeds.clone(),
        next_id: 15,
        ..Vegetation::new(&seed)
    };
    // 1 KLOC: P = 10, W = 3. Middling soil holds the weeds that are left.
    run(&mut vegetation, &seed, 1..=1, |today| {
        today.soil = Some(q(50));
    });
    let slots: Vec<u16> = vegetation.plants.iter().map(|p| p.slot).collect();
    // Slots 12 and 13 fell; one seedling took the lowest free slot (⌈5 / 7⌉).
    assert_eq!(slots, [0, 1, 2]);
    assert_eq!(vegetation.litter, [day(1).tick; 2]);
    assert_eq!(vegetation.weeds, weeds[..3]);
}

#[test]
fn inactivity_stops_germination_and_growth_but_not_decline() {
    let seed = seed(6);
    let mut vegetation = Vegetation::new(&seed);
    run(&mut vegetation, &seed, 1..=3, |_| {});
    let planted = vegetation.plants.clone();
    run(&mut vegetation, &seed, 4..=40, |today| {
        today.inactive = true;
    });
    let ids: Vec<u32> = vegetation.plants.iter().map(|p| p.id).collect();
    assert_eq!(ids, planted.iter().map(|p| p.id).collect::<Vec<_>>());
    assert!(
        vegetation
            .plants
            .iter()
            .zip(&planted)
            .all(|(now, then)| now.grown == then.grown && now.stage == then.stage)
    );
    // Poor health still kills while inactive.
    run(&mut vegetation, &seed, 41..=100, |today| {
        today.inactive = true;
        today.light = Some(q(10));
    });
    assert!(vegetation.plants.iter().all(|p| !p.alive()));
}

// The property tests: monotonicity, conservation, recovery (N-11) and stable ids.

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn raising_light_or_soil_never_lowers_the_target(
        light in reading(),
        soil in reading(),
        raise in 1_i32..=65_536,
        raise_soil in any::<bool>(),
        vigor in 0_i32..=65_536,
        rate in 0_i32..=65_536,
    ) {
        let raised = |value: Option<Q16>| {
            Some(value.unwrap_or(NEUTRAL).saturating_add(Q16::from_raw(raise)).min(Q16::ONE))
        };
        let (higher_light, higher_soil) = if raise_soil {
            (light, raised(soil))
        } else {
            (raised(light), soil)
        };
        // Raising a known reading, or making an unknown one known above neutral.
        let (before, _) = vigor_target(light, soil);
        let (after, _) = vigor_target(higher_light, higher_soil);
        prop_assert!(after >= before, "{:?} then {:?}", before, after);
        // And a higher target never leaves a plant with less vigor.
        let (rate, mut low, mut high) = (Q16::from_raw(rate), plant(0, Q16::from_raw(vigor), Limit::Soil), plant(0, Q16::from_raw(vigor), Limit::Soil));
        let today = day(1);
        low.grow(&today, before, Limit::Soil, rate);
        high.grow(&today, after, Limit::Soil, rate);
        prop_assert!(high.vigor >= low.vigor);
    }

    #[test]
    fn populations_stay_within_capacity_and_ids_stay_put(
        byte in any::<u8>(),
        days in prop::collection::vec(conditions(), 1..150),
    ) {
        let seed = seed(byte);
        let mut vegetation = Vegetation::new(&seed);
        let mut ledger = Ledger::default();
        for (n, (light, soil, rain, terrain, inactive)) in (1..).zip(days) {
            let today = Conditions { light, soil, rain, terrain, inactive, ..day(n) };
            vegetation.close(&seed, &today);
            ledger.check(&vegetation, terrain)?;
        }
    }

    #[test]
    fn any_state_recovers_within_60_days_of_good_climate(
        (terrain, mut vegetation) in any_state(),
        byte in any::<u8>(),
        days in prop::collection::vec((70_i32..=100, 70_i32..=100, 70_i32..=300), 60),
    ) {
        let seed = seed(byte);
        let mut ledger = Ledger::default();
        ledger.check(&vegetation, terrain)?;
        for (n, (light, soil, rain)) in (1..).zip(days) {
            let today = Conditions {
                light: Some(q(light)),
                soil: Some(q(soil)),
                rain: q(rain),
                terrain,
                ..day(n)
            };
            vegetation.close(&seed, &today);
            ledger.check(&vegetation, terrain)?;
        }
        prop_assert!(thriving(&vegetation, terrain), "{:#?}", vegetation);
    }
}

#[test]
fn lightning_dips_one_mature_plant_and_never_below_failing() {
    let seed = seed(7);
    let mut vegetation = Vegetation {
        plants: (0..5)
            .map(|slot| plant(slot, Q16::ONE, Limit::Soil))
            .collect(),
        ..Vegetation::new(&seed)
    };
    // The stream keyed by tick 72 draws index 4 of 5 (the Python model).
    let struck = vegetation.strike(&seed, Tick::new(72), q(15));
    assert_eq!(struck, Some(5));
    let vigor: Vec<Q16> = vegetation.plants.iter().map(|p| p.vigor).collect();
    assert_eq!(vigor, [Q16::ONE, Q16::ONE, Q16::ONE, Q16::ONE, q(85)]);
    // At 0.40 it falls to 0.35, no lower; below 0.35 it is left alone.
    let mut weak = Vegetation {
        plants: vec![plant(0, q(40), Limit::Soil)],
        ..Vegetation::new(&seed)
    };
    weak.strike(&seed, Tick::new(1), q(15));
    assert_eq!(weak.plants[0].vigor, FAILING_BELOW);
    weak.plants[0].vigor = q(20);
    weak.strike(&seed, Tick::new(2), q(15));
    assert_eq!(weak.plants[0].vigor, q(20));
    // Only mature living plants are struck.
    let mut young = Vegetation {
        plants: vec![Plant {
            stage: GrowthStage::Young,
            ..plant(0, Q16::ONE, Limit::Soil)
        }],
        ..Vegetation::new(&seed)
    };
    assert_eq!(young.strike(&seed, Tick::new(3), q(15)), None);
}

#[test]
fn a_sinkhole_swallows_one_living_plant_and_leaves_no_litter() {
    let seed = seed(7);
    let mut vegetation = Vegetation {
        plants: (0..7)
            .map(|slot| plant(slot, Q16::ONE, Limit::Soil))
            .collect(),
        ..Vegetation::new(&seed)
    };
    vegetation.plants[0].dead_since = Some(Tick::ZERO);
    // Six are living; commit 4's stream draws index 5 of 6 (the Python model): the plant in
    // slot 6. The dead one is never drawn.
    let key = crate::landmarks::commit_key(&crate::test_support::id(4));
    assert_eq!(vegetation.swallow(&seed, key), Some(7));
    assert_eq!(vegetation.plants.len(), 6);
    assert!(vegetation.litter.is_empty());
    let mut bare = Vegetation::new(&seed);
    assert_eq!(bare.swallow(&seed, key), None);
}

#[test]
fn a_flowering_blooms_the_vigorous_mature_plants_for_a_day() {
    let seed = seed(7);
    let mut vegetation = Vegetation {
        plants: vec![
            plant(0, Q16::ONE, Limit::Soil),
            plant(1, q(50), Limit::Soil),
            Plant {
                stage: GrowthStage::Young,
                ..plant(2, Q16::ONE, Limit::Soil)
            },
        ],
        ..Vegetation::new(&seed)
    };
    vegetation.bloom(Tick::new(124));
    let blooms: Vec<Option<Tick>> = vegetation.plants.iter().map(|p| p.bloom_until).collect();
    assert_eq!(blooms, [Some(Tick::new(124)), None, None]);
    // A bloom is never shortened.
    vegetation.bloom(Tick::new(110));
    assert_eq!(vegetation.plants[0].bloom_until, Some(Tick::new(124)));
}

#[test]
fn rain_after_drought_doubles_one_close_of_germination() {
    let seed = seed(4);
    let germinated = |boost: bool| {
        let mut vegetation = Vegetation {
            boost,
            ..Vegetation::new(&seed)
        };
        run(&mut vegetation, &seed, 1..=1, |_| {});
        assert!(!vegetation.boost, "spent at the close");
        vegetation.plants.len()
    };
    // 1 KLOC holds 10: a seventh of them, rounded up, is 2; doubled, 3.
    assert_eq!((germinated(false), germinated(true)), (2, 3));
}
