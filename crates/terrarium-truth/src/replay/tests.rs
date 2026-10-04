use std::collections::BTreeSet;

use proptest::prelude::*;

use super::*;
use crate::canon::{canonical_dump, from_canonical_dump};
use crate::climate::weather::{ChangeAt, WeatherChange};
use crate::domain::{ObservationHead, WeatherSource};
use crate::test_support::{
    HOUR, ORIGIN, basis, commit, coverage, grid, id, observation, proptest_config,
};

const THROUGH: u32 = 1_300;

fn replay(commits: &[CommitFacts], observations: &[Observation], through: u32) -> SimState {
    let prepared = Prepared::new(basis(), commits, observations).expect("valid inputs");
    let mut state = SimState::new(basis());
    state
        .advance_through(&prepared, Tick::new(through))
        .expect("advances");
    state
}

/// The record each midnight close leaves, through `through`.
fn closes(
    commits: &[CommitFacts],
    observations: &[Observation],
    through: u32,
) -> Vec<DailyClimate> {
    let prepared = Prepared::new(basis(), commits, observations).expect("valid inputs");
    let mut state = SimState::new(basis());
    let mut records = Vec::new();
    for close in (24..=through).step_by(24) {
        state
            .advance_through(&prepared, Tick::new(close))
            .expect("advances");
        records.extend(state.daily.last().cloned());
    }
    records
}

fn weather(
    source: WeatherSource,
    hours: u32,
    head: u32,
    branch: &str,
    pass: Option<bool>,
) -> Observation {
    let mut observation = observation(
        ObservationValue::Weather { source },
        hours,
        head,
        pass.map(|pass| if pass { Q16::ONE } else { Q16::ZERO }),
    );
    observation.head.branch = Some(branch.to_owned());
    observation.head.at_default_tip = branch == "main";
    observation
}

fn ci(branch: &str) -> WeatherSource {
    WeatherSource::Ci {
        branch: branch.to_owned(),
    }
}

/// `per_day[d]` commits on day `d`, from 09:00, each adding ten lines.
fn daily_commits(per_day: &[u32]) -> Vec<CommitFacts> {
    let mut commits = Vec::new();
    for (day, &count) in (0_u32..).zip(per_day) {
        for k in 0..count {
            let n = u32::try_from(commits.len()).expect("few") + 1;
            commits.push(commit(n, day * 24 + 9 + k));
        }
    }
    commits
}

#[test]
fn cold_start_is_half_active_until_the_baselines_settle() {
    // Ten quiet days, then a busy stretch: once settled, the busier pace reads as busy.
    let per_day: Vec<u32> = (0..20).map(|day| if day < 10 { 1 } else { 5 }).collect();
    let commits = daily_commits(&per_day);
    let prepared = Prepared::new(basis(), &commits, &[]).expect("valid");
    let mut state = SimState::new(basis());
    for (through, days_left) in [(0, 14), (100, 10), (335, 1)] {
        state
            .advance_through(&prepared, Tick::new(through))
            .expect("advances");
        let report = state.report();
        assert_eq!(report.activity, Q16::HALF, "at {through}");
        assert_eq!(report.settling_days_left, Some(days_left), "at {through}");
    }
    assert!(
        state
            .daily
            .iter()
            .all(|r| (r.rain, r.wind, r.activity) == (Q16::ONE, Q16::ONE, Q16::HALF))
    );
    state
        .advance_through(&prepared, Tick::new(SETTLING_TICKS))
        .expect("advances");
    let report = state.report();
    assert_eq!(report.settling_days_left, None);
    assert!(report.activity > Q16::HALF, "{:?}", report.activity);
}

#[test]
fn rain_and_wind_match_an_independent_model() {
    // Expected values from an exact integer model of the rain and wind pipeline, with log2 from
    // 80-digit decimals, written independently of this code.
    let commits = daily_commits(&[1, 0, 3, 0, 0, 2, 5, 1, 0, 0, 4, 2, 0, 1, 3, 0, 2, 6, 1, 0]);
    let state = replay(&commits, &[], 20 * 24);
    assert_eq!(state.rhythm.rain.level, Some(Q32::from_raw(4_592_453_673)));
    assert_eq!(state.rhythm.wind.level, Some(Q32::from_raw(12_208_340_222)));
    let record = state.daily.last().expect("a close");
    assert_eq!(record.rain.raw(), 69_458);
    assert_eq!(record.wind.raw(), 65_564);
    assert_eq!(record.activity.raw(), 34_242);
}

#[test]
fn a_stale_reading_keeps_its_value() {
    let observations = [coverage(5, 1, Q16::from_ratio(3, 10))];
    let before = [commit(1, 0)];
    // The tip moves on with a merge, which adds neither rain nor wind.
    let mut merge = commit(2, 80);
    merge.parents = 2;
    let after = [commit(1, 0), merge];
    let fresh = replay(&before, &observations, 100);
    let stale = replay(&after, &observations, 100);
    let light = |state: &SimState| state.report().climate.light;
    assert_eq!(
        light(&fresh),
        Reading::Known {
            value: Q16::from_ratio(3, 10),
            stale: false,
            since_tick: Tick::new(5),
        }
    );
    assert_eq!(
        light(&stale),
        Reading::Known {
            value: Q16::from_ratio(3, 10),
            stale: true,
            since_tick: Tick::new(5),
        }
    );
    assert_eq!(fresh.daily, stale.daily);
    assert_eq!(fresh.timeline, stale.timeline);
}

#[test]
fn a_feature_branch_reading_is_never_applied() {
    let commits = [commit(1, 0), {
        let mut side = commit(2, 20);
        side.on_default_first_parent = false;
        side
    }];
    let main = [coverage(5, 1, Q16::from_ratio(8, 10))];
    let mut side = coverage(30, 2, Q16::from_ratio(1, 10));
    side.head.branch = Some("feature/fern".to_owned());
    side.head.at_default_tip = false;
    let with_side = [main[0].clone(), side];
    assert_eq!(
        closes(&commits, &main, 200),
        closes(&commits, &with_side, 200)
    );
    let report = replay(&commits, &with_side, 200).report();
    assert_eq!(
        report.climate.light.known_value(),
        Some(Q16::from_ratio(8, 10))
    );
}

#[test]
fn a_reading_waits_for_its_commit_and_never_outlives_a_rewrite() {
    let commits = [commit(1, 0), commit(2, 50)];
    // Its commit is not in the history at all: a rewrite removed it.
    let unreachable = [coverage(5, 9, Q16::ONE)];
    let state = replay(&commits, &unreachable, 100);
    assert_eq!(state.readings.coverage, None);
    assert_eq!(state.report().climate.light, Reading::Unknown);
    // Its commit is dated in a later hour: it applies when the commit does, dated as recorded.
    let early = [coverage(10, 2, Q16::ONE)];
    assert_eq!(replay(&commits, &early, 49).readings.coverage, None);
    let state = replay(&commits, &early, 50);
    let applied = state
        .readings
        .coverage
        .expect("applied at the commit's tick");
    assert_eq!(applied.tick, Tick::new(10));
    // A newer report at the old commit, applied in between, is not replaced by it.
    let newer = [coverage(10, 2, Q16::ONE), coverage(20, 1, Q16::HALF)];
    let state = replay(&commits, &newer, 100);
    assert_eq!(state.readings.raw_light(), Some(Q16::HALF));
}

#[test]
fn a_resume_refuses_a_rewrite_that_keeps_the_count() {
    let commits = [commit(1, 0), commit(2, 5)];
    let state = replay(&commits, &[], 10);
    let mut amended = commit(3, 5);
    amended.added = 99;
    let rewritten = [commit(1, 0), amended];
    let prepared = Prepared::new(basis(), &rewritten, &[]).expect("valid");
    let mut resumed = state.clone();
    assert_eq!(
        resumed.advance_through(&prepared, Tick::new(20)),
        Err(ReplayError::NeedsFullReplay)
    );
    assert_eq!(resumed, state);
}

#[test]
fn a_source_switch_is_not_a_transition() {
    let commits = [commit(1, 0)];
    let observations = [
        weather(ci("main"), 1, 1, "main", Some(true)),
        weather(ci("feature/fern"), 2, 1, "feature/fern", Some(false)),
        weather(ci("main"), 3, 1, "main", Some(true)),
        weather(ci("main"), 4, 1, "main", Some(false)),
    ];
    let change = |through| replay(&commits, &observations, through).weather.last_change;
    assert_eq!(
        change(2),
        Some(ChangeAt {
            tick: Tick::new(2),
            change: WeatherChange::Switch {
                to: Some(ci("feature/fern"))
            }
        })
    );
    assert_eq!(
        change(3),
        Some(ChangeAt {
            tick: Tick::new(3),
            change: WeatherChange::Switch {
                to: Some(ci("main"))
            }
        })
    );
    assert_eq!(
        change(4),
        Some(ChangeAt {
            tick: Tick::new(4),
            change: WeatherChange::Transition {
                source: ci("main"),
                pass: false
            }
        })
    );
}

#[test]
fn the_first_close_is_the_next_midnight() {
    let commits = [commit(1, 0)];
    assert!(replay(&commits, &[], 23).daily.is_empty());
    let state = replay(&commits, &[], 24);
    assert_eq!(state.daily.len(), 1);
    assert_eq!(state.daily[0].tick, Tick::new(24));
    // From 09:00 UTC, the first day is 15 hours long.
    let nine = TickGrid::from_origin(ORIGIN + 9 * HOUR).expect("a whole hour");
    assert_eq!(first_close(nine), Tick::new(15));
    assert_eq!(first_close(grid()), Tick::new(24));
    let basis = Basis::new(RepoSeed::from_bytes([7; 32]), nine, "main");
    let prepared = Prepared::new(basis.clone(), &commits, &[]).expect("valid");
    let mut state = SimState::new(basis);
    state
        .advance_through(&prepared, Tick::new(14))
        .expect("advances");
    assert!(state.daily.is_empty());
    state
        .advance_through(&prepared, Tick::new(15))
        .expect("advances");
    assert_eq!(state.daily.len(), 1);
    assert_eq!(state.settling_days_left(), Some(14));
}

#[test]
fn same_tick_commits_keep_the_readers_order() {
    // The child's id sorts before its parent's; the reader puts the parent first anyway.
    let commits = [commit(9, 5), commit(1, 5)];
    let observations = [coverage(5, 1, Q16::from_ratio(7, 10))];
    let state = replay(&commits, &observations, 30);
    assert_eq!(state.habitat.default_tip, Some(id(1)));
    // The reading, recorded in the same hour as its commit, applies after it and is fresh.
    assert_eq!(
        state.report().climate.light,
        Reading::Known {
            value: Q16::from_ratio(7, 10),
            stale: false,
            since_tick: Tick::new(5),
        }
    );
}

#[test]
fn a_future_dated_commit_applies_when_reached() {
    let commits = [commit(1, 0), commit(2, 500)];
    let early = replay(&commits, &[], 499);
    assert_eq!(early.habitat().terrain, 10);
    assert_eq!(early.habitat.last_commit, Some(Tick::ZERO));
    assert_eq!(early.consumed.commits, 1);
    let late = replay(&commits, &[], 500);
    assert_eq!(late.habitat().terrain, 20);
    assert_eq!(late.habitat.last_commit, Some(Tick::new(500)));
    assert_eq!(late.habitat().age, Tick::new(500));
}

#[test]
fn observations_before_tick_zero_apply_at_it() {
    assert_eq!(observation_tick(grid(), ORIGIN - 1), Some(Tick::ZERO));
    assert_eq!(observation_tick(grid(), ORIGIN + HOUR), Some(Tick::new(1)));
    assert_eq!(observation_tick(grid(), i64::MAX), None);
    let mut early = coverage(0, 1, Q16::ONE);
    early.recorded_at = ORIGIN - 3 * HOUR;
    let state = replay(&[commit(1, 0)], &[early], 30);
    assert_eq!(
        state.readings.coverage.as_ref().map(|c| c.tick),
        Some(Tick::ZERO)
    );
}

#[test]
fn malformed_inputs_are_refused() {
    let backwards = [commit(1, 5), commit(2, 4)];
    assert_eq!(
        Prepared::new(basis(), &backwards, &[]).map(|_| ()),
        Err(ReplayError::CommitsOutOfOrder { commit: id(2) })
    );
    let mut twin = coverage(3, 1, Q16::ONE);
    twin.id = 99;
    twin.normalized = Some(Q16::ZERO);
    let twins = [coverage(3, 1, Q16::ONE), twin];
    assert_eq!(
        Prepared::new(basis(), &[commit(1, 0)], &twins).map(|_| ()),
        Err(ReplayError::DuplicateObservation {
            recorded_at: ORIGIN + 3 * HOUR + 60,
            kind: ObservationKind::Coverage,
        })
    );
    let commits = [commit(1, 0)];
    let mut state = replay(&commits, &[], 10);
    let other = Basis::new(RepoSeed::from_bytes([7; 32]), grid(), "trunk");
    let elsewhere = Prepared::new(other, &commits, &[]).expect("valid");
    assert_eq!(
        state.advance_through(&elsewhere, Tick::new(20)),
        Err(ReplayError::BasisChanged)
    );
    // A reading, and a commit, from before the state's next tick, which it never saw.
    let late = [coverage(4, 1, Q16::ONE)];
    let grown = Prepared::new(basis(), &commits, &late).expect("valid");
    assert_eq!(
        state.advance_through(&grown, Tick::new(20)),
        Err(ReplayError::NeedsFullReplay)
    );
    let backdated = [commit(1, 0), commit(2, 5)];
    let grown = Prepared::new(basis(), &backdated, &[]).expect("valid");
    assert_eq!(
        state.advance_through(&grown, Tick::new(20)),
        Err(ReplayError::NeedsFullReplay)
    );
    // Neither refusal moved the state.
    assert_eq!(state, replay(&commits, &[], 10));
}

#[test]
fn the_state_round_trips_through_both_encodings() {
    let commits = daily_commits(&[2, 1, 0, 3]);
    let observations = [
        coverage(10, 1, Q16::from_ratio(3, 4)),
        weather(ci("main"), 11, 1, "main", Some(false)),
        weather(ci("feature/\"]x"), 12, 1, "feature/\"]x", Some(true)),
    ];
    let state = replay(&commits, &observations, 100);
    let bytes = postcard::to_allocvec(&state).expect("encodes");
    assert_eq!(
        postcard::from_bytes::<SimState>(&bytes).expect("decodes"),
        state
    );
    assert_eq!(
        from_canonical_dump(&canonical_dump(&state)),
        Ok(state.clone())
    );
    let prepared = Prepared::new(basis(), &commits, &observations).expect("valid");
    let dump = crate::dump(&prepared, Tick::new(100)).expect("dumps");
    assert_eq!(dump.state, state);
    assert!(canonical_dump(&dump).starts_with("sim_version = 2\n"));
    assert_eq!(
        from_canonical_dump(&canonical_dump(&dump)),
        Ok(dump.clone())
    );
    assert_eq!(dump.scene.validate(), Ok(()));
    assert_eq!(dump.samples.len(), 1);
}

// Generated histories for the property tests: up to 40 commits over about 1,200 hours, and up
// to 40 observations of every kind truth reads, some at commits the history lacks.

type OwnedKey = (i64, &'static str, String, String);

fn owned_key(observation: &Observation) -> OwnedKey {
    let (recorded_at, kind, source, head) = merge_key(observation);
    (recorded_at, kind, source, head.to_owned())
}

fn generated_value(kind: u8) -> ObservationValue {
    match kind {
        0 => ObservationValue::Coverage,
        1 => ObservationValue::DeadCode {
            unused_items: 1,
            exported_items: 2,
        },
        2 => ObservationValue::Weather {
            source: WeatherSource::Local,
        },
        3 => ObservationValue::Weather { source: ci("main") },
        _ => ObservationValue::Weather {
            source: ci("feature/fern"),
        },
    }
}

fn generated() -> impl Strategy<Value = (Vec<CommitFacts>, Vec<Observation>)> {
    let commits = prop::collection::vec(
        (0_u32..60, 1_u32..=2, any::<bool>(), 0_u64..500, 0_u64..200),
        1..40,
    );
    let observations = prop::collection::vec(
        (
            (0_u32..THROUGH, 0_i64..3_600, 0_u8..5),
            (1_u32..45, 0_u8..3, any::<bool>()),
            prop::option::of(0_i32..=65_536),
        ),
        0..40,
    );
    (commits, observations).prop_map(|(commits, observations)| {
        let mut tick = 0;
        let commits = (1_u32..)
            .zip(commits)
            .map(|(n, (gap, parents, default, added, removed))| {
                if n > 1 {
                    tick += gap;
                }
                CommitFacts {
                    parents,
                    on_default_first_parent: n == 1 || default,
                    added,
                    removed,
                    ..commit(n, tick)
                }
            })
            .collect();
        let mut seen = BTreeSet::new();
        let observations = (0_i64..)
            .zip(observations)
            .map(
                |(row, ((hours, secs, kind), (head, branch, at_tip), raw))| {
                    let value = generated_value(kind);
                    let normalized = match value {
                        ObservationValue::Weather { .. } => {
                            raw.map(|raw| if raw >= 32_768 { Q16::ONE } else { Q16::ZERO })
                        }
                        _ => raw.map(Q16::from_raw),
                    };
                    let branch = match branch {
                        0 => Some("main".to_owned()),
                        1 => Some("feature/fern".to_owned()),
                        _ => None,
                    };
                    Observation {
                        id: row,
                        recorded_at: ORIGIN + i64::from(hours) * HOUR + secs,
                        head: ObservationHead {
                            commit: id(head),
                            branch,
                            at_default_tip: at_tip,
                        },
                        normalized,
                        confidence: Q16::ONE,
                        value,
                    }
                },
            )
            .filter(|o| seen.insert(owned_key(o)))
            .collect();
        (commits, observations)
    })
}

/// A month of daily commits, each with a coverage and a dead-code reading at its tip.
fn readings_month(
    coverage_values: &[i32],
    soil_values: &[i32],
) -> (Vec<CommitFacts>, Vec<Observation>) {
    let commits: Vec<CommitFacts> = (0..30).map(|day| commit(day + 1, day * 24 + 1)).collect();
    let mut observations = Vec::new();
    for (day, (&light, &soil)) in (0_u32..).zip(coverage_values.iter().zip(soil_values)) {
        observations.push(coverage(day * 24 + 2, day + 1, Q16::from_raw(light)));
        observations.push(observation(
            ObservationValue::DeadCode {
                unused_items: 1,
                exported_items: 2,
            },
            day * 24 + 3,
            day + 1,
            Some(Q16::from_raw(soil)),
        ));
    }
    (commits, observations)
}

fn values() -> impl Strategy<Value = Vec<i32>> {
    prop::collection::vec(0_i32..=65_536, 30)
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn resuming_a_cached_state_equals_a_full_replay(
        (commits, observations) in generated(),
        split in 0_u32..THROUGH,
        keep_late in any::<u64>(),
    ) {
        let full = replay(&commits, &observations, THROUGH);
        // What the cache saw: everything before the split, and some of what came after it.
        let late = |n: usize| (keep_late >> (n % 64)) & 1 == 1;
        let seen_commits: Vec<CommitFacts> = commits
            .iter()
            .enumerate()
            .filter(|(n, c)| c.tick.hours() < split || late(*n))
            .map(|(_, c)| c.clone())
            .collect();
        let tick_of = |o: &Observation| observation_tick(grid(), o.recorded_at).expect("on the grid");
        let seen_observations: Vec<Observation> = observations
            .iter()
            .enumerate()
            .filter(|(n, o)| tick_of(o).hours() < split || late(n + 7))
            .map(|(_, o)| o.clone())
            .collect();
        let mut cached = SimState::new(basis());
        if let Some(before) = split.checked_sub(1) {
            let seen = Prepared::new(basis(), &seen_commits, &seen_observations).expect("valid");
            cached.advance_through(&seen, Tick::new(before)).expect("advances");
        }
        let bytes = postcard::to_allocvec(&cached).expect("encodes");
        let mut resumed: SimState = postcard::from_bytes(&bytes).expect("decodes");
        let everything = Prepared::new(basis(), &commits, &observations).expect("valid");
        // A commit the cache had not read moves an earlier reading taken at it to its own tick,
        // and only then may the resume refuse.
        let moved = observations.iter().any(|o| {
            let late = commits
                .iter()
                .find(|c| c.id == o.head.commit)
                .is_some_and(|c| c.tick.hours() >= split && c.tick > tick_of(o));
            o.head.at_default_tip
                && tick_of(o).hours() < split
                && late
                && !matches!(o.value, ObservationValue::Weather { .. })
        });
        match resumed.advance_through(&everything, Tick::new(THROUGH)) {
            Ok(()) => prop_assert_eq!(&resumed, &full),
            Err(err) => {
                prop_assert_eq!(err, ReplayError::NeedsFullReplay);
                prop_assert!(moved, "refused with nothing moved");
            }
        }
        // An observation applied before the split that the cache never saw forces a full
        // replay. Weather applies at its own tick whatever its commit.
        let before_split = |o: &Observation| {
            matches!(o.value, ObservationValue::Weather { .. }) && tick_of(o).hours() < split
        };
        if let Some(missing) = observations.iter().position(before_split) {
            let mut fewer = observations.clone();
            fewer.remove(missing);
            let seen = Prepared::new(basis(), &commits, &fewer).expect("valid");
            let mut short = SimState::new(basis());
            short.advance_through(&seen, Tick::new(split - 1)).expect("advances");
            prop_assert_eq!(
                short.advance_through(&everything, Tick::new(THROUGH)),
                Err(ReplayError::NeedsFullReplay)
            );
        }
    }

    #[test]
    fn neither_input_order_nor_row_ids_matter(
        (commits, observations) in generated(),
        rotation in 0_usize..40,
    ) {
        let mut shuffled = observations.clone();
        shuffled.reverse();
        if !shuffled.is_empty() {
            let by = rotation % shuffled.len();
            shuffled.rotate_left(by);
        }
        for (observation, id) in shuffled.iter_mut().zip((1_000_i64..).step_by(7)) {
            observation.id = id;
        }
        prop_assert_eq!(
            replay(&commits, &observations, THROUGH),
            replay(&commits, &shuffled, THROUGH)
        );
    }

    #[test]
    fn raising_a_reading_never_lowers_health(
        light in values(),
        soil in values(),
        day in 0_usize..30,
        raise in 1_i32..=65_536,
        raise_soil in any::<bool>(),
    ) {
        let (commits, observations) = readings_month(&light, &soil);
        let (mut higher_light, mut higher_soil) = (light.clone(), soil.clone());
        let target = if raise_soil { &mut higher_soil } else { &mut higher_light };
        target[day] = target[day].saturating_add(raise).min(65_536);
        let (_, raised) = readings_month(&higher_light, &higher_soil);
        let before = closes(&commits, &observations, 31 * 24);
        let after = closes(&commits, &raised, 31 * 24);
        for (before, after) in before.iter().zip(&after) {
            prop_assert!(after.health >= before.health, "{:?} then {:?}", before, after);
        }
    }

    #[test]
    fn an_unknown_reading_never_lowers_health_below_neutral(
        light in values(),
        soil in values(),
        forget_soil in any::<bool>(),
    ) {
        let (commits, observations) = readings_month(&light, &soil);
        let kind = if forget_soil { ObservationKind::DeadCode } else { ObservationKind::Coverage };
        let forgotten: Vec<Observation> = observations
            .iter()
            .filter(|o| o.value.kind() != kind)
            .cloned()
            .collect();
        let before = closes(&commits, &observations, 31 * 24);
        let after = closes(&commits, &forgotten, 31 * 24);
        for (before, after) in before.iter().zip(&after) {
            prop_assert!(after.health >= before.health.min(NEUTRAL), "{:?} then {:?}", before, after);
        }
    }
}
