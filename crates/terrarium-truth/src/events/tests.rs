use super::*;
use crate::domain::{CommitFacts, DeletedDir, Observation, Rename, TagFact};
use crate::landmarks::fossil_id;
use crate::replay::{Prepared, SimState};
use crate::scene::GrowthStage;
use crate::test_support::{
    basis, ci, commit, coverage, daily_commits, dead_code, id, rewrite, weather,
};
use crate::{Q16, project, project_showing};

use EventKind::{
    Clearing, ClearingBurn, Dusk, Earthquake, Flowering, Lightning, RainAfterDrought, Sinkhole,
    Storm, Sunburst, Windstorm,
};
use Strength::{High, Low, Mid};

fn seed() -> RepoSeed {
    RepoSeed::from_bytes([7; 32])
}

fn replay(commits: &[CommitFacts], observations: &[Observation], through: u32) -> SimState {
    let prepared = Prepared::new(basis(), commits, observations).expect("valid inputs");
    let mut state = SimState::new(basis());
    state
        .advance_through(&prepared, Tick::new(through))
        .expect("advances");
    state
}

/// Every candidate of a replay through `through`, as (tick, kind, strength).
fn candidates(
    commits: &[CommitFacts],
    observations: &[Observation],
    through: u32,
) -> Vec<(u32, EventKind, Strength)> {
    let prepared = Prepared::new(basis(), commits, observations).expect("valid inputs");
    crate::dump(&prepared, Tick::new(through))
        .expect("dumps")
        .events
        .iter()
        .map(|e| (e.started_tick.hours(), e.kind, e.strength))
        .collect()
}

fn found(kind: EventKind, tick: u32, strength: Strength) -> Qualified {
    Qualified {
        kind,
        tick: Tick::new(tick),
        strength,
        fossil: (kind == Sinkhole).then_some(1),
        target: None,
    }
}

// The event core.

#[test]
fn a_release_tag_is_an_ascii_version_number() {
    for (name, major) in [
        ("v1.0.0", Some(1)),
        ("1.0.0", Some(1)),
        ("v2", Some(2)),
        ("0.9", Some(0)),
        ("v10.4.1.7", Some(10)),
        ("2024.1", Some(2_024)),
        ("v99999999999999999999999", Some(u64::MAX)),
        ("v1.0.0-rc1", None),
        ("V1.0", None),
        ("v", None),
        ("", None),
        ("1..2", None),
        ("1.", None),
        (".1", None),
        ("vv1", None),
        ("release-1", None),
        ("v1.0 ", None),
        ("v١.٠", None),
        ("v1.²", None),
    ] {
        assert_eq!(release(name), major, "{name:?}");
    }
}

#[test]
fn strength_reads_its_bands_from_their_lower_edges() {
    assert_eq!(strength(2_u32, [3, 10]), Low);
    assert_eq!(strength(3_u32, [3, 10]), Mid);
    assert_eq!(strength(9_u32, [3, 10]), Mid);
    assert_eq!(strength(10_u32, [3, 10]), High);
}

#[test]
fn a_cooldown_runs_from_every_qualification_and_a_suppressed_one_still_counts() {
    let seed = seed();
    let mut events = Events::default();
    // Storm's cooldown is two hours.
    assert!(events.qualify(&seed, found(Storm, 10, Mid)));
    assert!(!events.qualify(&seed, found(Storm, 11, Mid)));
    // The suppressed storm at 11 restarted the cooldown: 12 is still inside it.
    assert!(!events.qualify(&seed, found(Storm, 12, Mid)));
    assert!(events.qualify(&seed, found(Storm, 14, Mid)));
    // Another kind has its own.
    assert!(events.qualify(&seed, found(Clearing, 14, Mid)));
    assert_eq!(
        events.tally.get(&Storm),
        Some(&Tally {
            qualified: 4,
            eligible: 2
        })
    );
    let ticks: Vec<u32> = events
        .recent
        .iter()
        .map(|c| c.event.started_tick.hours())
        .collect();
    assert_eq!(ticks, [10, 14, 14]);
}

#[test]
fn arbitration_takes_the_lowest_rank_then_the_stronger_then_the_newer() {
    let seed = seed();
    let mut events = Events::default();
    events.qualify(&seed, found(Clearing, 5, Mid));
    events.qualify(&seed, found(Windstorm, 5, High));
    events.qualify(&seed, found(Dusk, 5, High));
    assert_eq!(
        events.current(Tick::new(5)).map(|c| c.event.kind),
        Some(Windstorm)
    );
    events.qualify(&seed, found(Earthquake, 5, Low));
    assert_eq!(
        events.current(Tick::new(5)).map(|c| c.event.kind),
        Some(Earthquake)
    );
    // Sunburst and dusk share a rank: the stronger wins, then the newer.
    let mut sky = Events::default();
    sky.qualify(&seed, found(Sunburst, 7, Low));
    sky.qualify(&seed, found(Dusk, 7, Mid));
    assert_eq!(sky.current(Tick::new(7)).map(|c| c.event.kind), Some(Dusk));
    let mut later = Events::default();
    later.qualify(&seed, found(Dusk, 7, Mid));
    later.qualify(&seed, found(Sunburst, 20, Mid));
    let none = BTreeSet::new();
    assert_eq!(later.unseen(&none).map(|c| c.event.kind), Some(Sunburst));
    // Only what started in the tick plays in it.
    assert_eq!(later.current(Tick::new(8)), None);
}

#[test]
fn a_catch_up_plays_the_best_unseen_candidate_once_and_forgets_after_a_week() {
    let seed = seed();
    let mut events = Events::default();
    events.qualify(&seed, found(Flowering, 100, Mid));
    let shown = events.keys();
    events.qualify(&seed, found(Storm, 110, Mid));
    events.qualify(&seed, found(Windstorm, 120, High));
    // The flowering was shown; of the rest, the storm outranks the windstorm.
    assert_eq!(
        events.unseen(&shown).map(Candidate::key),
        Some((Tick::new(110), Storm))
    );
    // Once everything was shown, nothing plays.
    assert_eq!(events.unseen(&events.keys()), None);
    assert_eq!(
        events
            .find((Tick::new(100), Flowering))
            .map(|c| c.event.kind),
        Some(Flowering)
    );
    // A week after it started, a candidate is forgotten; an hour before, it is not.
    events.begin(Tick::new(100 + EVENT_RECALL_TICKS - 1));
    assert_eq!(events.recent.len(), 3);
    events.begin(Tick::new(100 + EVENT_RECALL_TICKS));
    assert_eq!(events.recent.len(), 2);
}

#[test]
fn marks_settle_expire_cap_and_wash_away() {
    let mut events = Events::default();
    events.mark(GroundMark::Crack, Tick::new(10), None);
    events.mark(GroundMark::Crack, Tick::new(10), None);
    assert_eq!(events.marks.len(), 1, "a mark made twice in a tick is one");
    events.mark(GroundMark::FreshSoil, Tick::new(20), None);
    // Fresh soil lasts 14 days, the crack 30.
    events.begin(Tick::new(20 + 14 * 24 - 1));
    assert_eq!(events.marks.len(), 2);
    events.begin(Tick::new(20 + 14 * 24));
    assert_eq!(events.marks.len(), 1);
    events.begin(Tick::new(10 + 30 * 24));
    assert!(events.marks.is_empty());
    // At most three, the oldest leaving first.
    for tick in 1..=4 {
        events.mark(GroundMark::Sinkhole, Tick::new(tick), Some(tick));
    }
    let fossils: Vec<Option<u32>> = events.marks.iter().map(|m| m.fossil).collect();
    assert_eq!(fossils, [Some(2), Some(3), Some(4)]);
    events.mark(GroundMark::FreshSoil, Tick::new(5), None);
    events.clear_settled();
    let kinds: Vec<GroundMark> = events.marks.iter().map(|m| m.kind).collect();
    assert_eq!(kinds, [GroundMark::FreshSoil]);
    assert_eq!(
        events.mark_views(),
        [GroundMarkView {
            id: mark_id(GroundMark::FreshSoil, Tick::new(5), None),
            kind: GroundMark::FreshSoil,
            fossil: None,
        }]
    );
}

// Expected draws and ids below come from an independent Python model of SHA-256, the stream
// derivation, SplitMix64, PCG32 and Lemire's ranges.

#[test]
fn sides_and_ids_are_known() {
    use Side::{Left, Right};
    let seed = seed();
    let sides: Vec<Side> = [
        (Storm, 0),
        (Storm, 1),
        (Storm, 2),
        (Earthquake, 100),
        (Sinkhole, 100),
        (Flowering, 6_249),
        (Dusk, 9_999),
        (Windstorm, 24),
    ]
    .into_iter()
    .map(|(kind, tick)| side(&seed, kind, Tick::new(tick)))
    .collect();
    assert_eq!(sides, [Left, Left, Left, Right, Left, Right, Right, Right]);
    let fossils: Vec<u32> = (1..=3).map(|n| fossil_id(&id(n))).collect();
    assert_eq!(fossils, [903_053_662, 2_538_202_160, 2_849_461_840]);
    let marks = [
        mark_id(GroundMark::Crack, Tick::new(100), None),
        mark_id(GroundMark::Sinkhole, Tick::new(100), Some(7)),
        mark_id(GroundMark::FreshSoil, Tick::new(100), None),
        mark_id(GroundMark::Sinkhole, Tick::new(100), None),
    ];
    assert_eq!(
        marks,
        [3_604_166_259, 2_261_309_423, 3_076_604_223, 2_603_949_127]
    );
}

#[test]
fn a_clearing_leaves_by_the_side_its_storm_came_in() {
    let seed = seed();
    for storm_at in 0..16 {
        let mut events = Events::default();
        events.qualify(&seed, found(Storm, storm_at, Mid));
        events.qualify(&seed, found(Clearing, storm_at + 5, Mid));
        let sides: Vec<Side> = events.recent.iter().map(|c| c.event.side).collect();
        assert_eq!(sides[0], sides[1], "storm at {storm_at}");
    }
    // With no storm before it, a clearing draws its own.
    let mut events = Events::default();
    events.qualify(&seed, found(Clearing, 3, Mid));
    assert_eq!(
        events.recent[0].event.side,
        side(&seed, Clearing, Tick::new(3))
    );
}

// Each detector, on scripted inputs.

#[test]
fn a_rewrite_of_the_default_branch_is_an_earthquake_that_cracks_the_ground() {
    let commits = [commit(1, 0), commit(2, 5)];
    for (rewritten, expected) in [
        (None, Low),
        (Some(2), Low),
        (Some(3), Mid),
        (Some(10), High),
    ] {
        let observations = [rewrite(10, "refs/heads/main", rewritten, Vec::new())];
        assert_eq!(
            candidates(&commits, &observations, 20),
            [(10, Earthquake, expected)]
        );
    }
    let state = replay(
        &commits,
        &[rewrite(10, "refs/heads/main", None, Vec::new())],
        20,
    );
    let marks: Vec<(GroundMark, u32)> = state
        .events
        .marks
        .iter()
        .map(|m| (m.kind, m.since.hours()))
        .collect();
    assert_eq!(marks, [(GroundMark::Crack, 10)]);
    // Another branch's rewrite, or one named without its `refs/heads/`, shakes nothing.
    for reference in ["refs/heads/feature/fern", "main", "refs/tags/main"] {
        let observations = [rewrite(10, reference, Some(5), Vec::new())];
        assert!(
            candidates(&commits, &observations, 20).is_empty(),
            "{reference}"
        );
    }
}

#[test]
fn a_rewrite_that_took_a_shown_fossil_opens_a_sinkhole_naming_it() {
    let commits = [commit(1, 0), commit(2, 5)];
    // The recorded fossil's commit is gone from the history: its sinkhole names it.
    let gone = id(77);
    let observations = [rewrite(
        10,
        "refs/heads/main",
        Some(4),
        vec![gone.clone(), id(2)],
    )];
    let state = replay(&commits, &observations, 20);
    let sinkholes: Vec<Option<u32>> = state
        .events
        .recent
        .iter()
        .filter(|c| c.event.kind == Sinkhole)
        .map(|c| c.event.fossil)
        .collect();
    // `id(2)` is back on the default chain, so its fossil was never taken.
    assert_eq!(sinkholes, [Some(fossil_id(&gone))]);
    assert_eq!(
        state.events.recent[1].event.strength, Mid,
        "the rewrite's strength"
    );
    assert!(
        state
            .events
            .marks
            .iter()
            .any(|m| m.kind == GroundMark::Sinkhole && m.fossil == Some(fossil_id(&gone)))
    );
    // The earthquake outranks it.
    let scene = project(&replay(&commits, &observations, 10));
    assert_eq!(scene.event.map(|e| e.kind), Some(Earthquake));
    assert_eq!(scene.validate(), Ok(()));
}

/// Thirty days of a commit a day, so a terrarium has grown, and `extra` after them.
fn grown(extra: &[CommitFacts]) -> Vec<CommitFacts> {
    let mut commits = daily_commits(&[1; 30]);
    commits.extend_from_slice(extra);
    commits
}

#[test]
fn a_large_deletion_opens_a_sinkhole_that_swallows_one_plant() {
    let deletion = CommitFacts {
        deleted_dirs: vec![DeletedDir {
            path: "src/legacy".to_owned(),
            lines: 2_500,
        }],
        ..commit(31, 30 * 24 + 9)
    };
    let before = replay(&grown(&[]), &[], 30 * 24 + 8);
    let commits = grown(&[deletion]);
    let state = replay(&commits, &[], 30 * 24 + 9);
    let candidate = state
        .events
        .current(Tick::new(30 * 24 + 9))
        .copied()
        .expect("a sinkhole");
    assert_eq!(candidate.event.kind, Sinkhole);
    assert_eq!(candidate.event.strength, Mid);
    assert_eq!(candidate.event.fossil, Some(fossil_id(&id(31))));
    let swallowed = candidate.target.expect("a plant");
    let ids = |s: &SimState| -> Vec<u32> { s.vegetation.plants.iter().map(|p| p.id).collect() };
    let mut expected = ids(&before);
    expected.retain(|&plant| plant != swallowed);
    assert_eq!(ids(&state), expected, "one plant gone, no other");
    assert_eq!(
        state.vegetation.litter, before.vegetation.litter,
        "no litter"
    );
    assert_eq!(
        state
            .events
            .marks
            .iter()
            .map(|m| (m.kind, m.fossil))
            .collect::<Vec<_>>(),
        [(GroundMark::Sinkhole, Some(fossil_id(&id(31))))]
    );
    // A rename is never a deletion.
    let rename = CommitFacts {
        renames: vec![Rename {
            from: "src/soil".to_owned(),
            to: "src/ground".to_owned(),
        }],
        ..commit(31, 30 * 24 + 9)
    };
    assert!(candidates(&grown(&[rename]), &[], 30 * 24 + 9).is_empty());
}

#[test]
fn lightning_strikes_each_day_of_failure_from_the_third_and_scorches_a_mature_plant() {
    let commits = daily_commits(&[1; 50]);
    let observations = [
        weather(ci("main"), 30 * 24, 30, "main", Some(true)),
        weather(ci("main"), 30 * 24 + 10, 31, "main", Some(false)),
        weather(ci("main"), 38 * 24 + 10, 38, "main", Some(true)),
    ];
    let fail = 30 * 24 + 10;
    let events = candidates(&commits, &observations, 40 * 24);
    assert_eq!(
        events,
        [
            (fail, Storm, Mid),
            (fail + 72, Lightning, Low),
            (fail + 96, Lightning, Mid),
            (fail + 120, Lightning, Mid),
            (fail + 144, Lightning, Mid),
            (fail + 168, Lightning, High),
            // At 8 days the pass arrives in the same tick: a clearing, no strike.
            (fail + 192, Clearing, Mid),
        ]
    );
    let state = replay(&commits, &observations, fail + 72);
    let scene = project(&state);
    assert_eq!(scene.event.map(|e| e.kind), Some(Lightning));
    let scorched: Vec<_> = scene.plants.iter().filter(|p| p.scorched).collect();
    assert_eq!(scorched.len(), 1);
    assert_eq!(scorched[0].stage, GrowthStage::Mature);
    // Shown as anything else, nothing is scorched.
    assert!(
        project_showing(&state, None)
            .plants
            .iter()
            .all(|p| !p.scorched)
    );
}

#[test]
fn lightning_spares_an_inactive_repository_and_a_switch_restarts_the_run() {
    // One commit, then CI failing from day 40: inactive since day 30, so no strike.
    let observations = [
        weather(ci("main"), 40 * 24, 1, "main", Some(true)),
        weather(ci("main"), 40 * 24 + 1, 1, "main", Some(false)),
    ];
    let quiet = candidates(&[commit(1, 0)], &observations, 50 * 24);
    assert_eq!(quiet, [(40 * 24 + 1, Storm, Mid)]);
    // The feature branch has failed since hour 2; HEAD moves onto it at hour 100. Its run
    // counts from the switch: the first strike is at 172, not at a multiple of a day from 2.
    let commits = daily_commits(&[1; 12]);
    let observations = [
        weather(ci("feature/fern"), 2, 1, "feature/fern", Some(false)),
        weather(ci("main"), 3, 1, "main", Some(true)),
        weather(ci("feature/fern"), 100, 4, "feature/fern", Some(false)),
    ];
    let events = candidates(&commits, &observations, 200);
    assert_eq!(events, [(172, Lightning, Low), (196, Lightning, Mid)]);
}

#[test]
fn a_release_tag_flowers_at_its_own_tick_and_a_new_major_waits_for_a_stone() {
    let tag = |name: &str, tick: u32| TagFact {
        name: name.to_owned(),
        tick: Tick::new(tick),
    };
    let commits = [
        commit(1, 0),
        CommitFacts {
            tags: vec![tag("v0.9.0", 30), tag("nightly", 30)],
            ..commit(2, 30)
        },
        CommitFacts {
            tags: vec![tag("v1.0.0", 70), tag("1.0.0", 70), tag("v1.1.0-rc1", 70)],
            ..commit(3, 60)
        },
        CommitFacts {
            tags: vec![tag("2.0", 200), tag("v1.5.0", 230)],
            ..commit(4, 100)
        },
    ];
    let events = candidates(&commits, &[], 300);
    assert_eq!(
        events,
        [
            (30, Flowering, Mid),
            (70, Flowering, Mid),
            (200, Flowering, Mid),
            (230, Flowering, Mid)
        ]
    );
    // Majors 1 and 2 wait for a stone; 0.9 and the backported 1.5 carve nothing.
    let state = replay(&commits, &[], 300);
    assert_eq!(state.landmarks.pending, 2);
    assert_eq!(state.events.major, Some(2));
    // Two tags in one tick flower once; the cooldown suppresses the second.
    assert_eq!(
        state.events.tally.get(&Flowering),
        Some(&Tally {
            qualified: 5,
            eligible: 4
        })
    );
    // A cache taken between the tagged commit and its later tag resumes to the same state.
    let prefix: Vec<CommitFacts> = commits[..3].to_vec();
    let mut cached = SimState::new(basis());
    cached
        .advance_through(
            &Prepared::new(basis(), &prefix, &[]).expect("valid"),
            Tick::new(64),
        )
        .expect("advances");
    cached
        .advance_through(
            &Prepared::new(basis(), &commits, &[]).expect("valid"),
            Tick::new(300),
        )
        .expect("resumes");
    assert_eq!(cached, state);
}

#[test]
fn a_clearing_burn_needs_a_fifth_and_five_items_both_known() {
    let commits = daily_commits(&[1; 10]);
    let report =
        |day: u32, unused: u32, known: bool| dead_code(day * 24 + 10, day + 1, unused, known);
    let observations = [
        report(0, 40, true),
        // 10 fewer, 25%: a burn.
        report(1, 30, true),
        // 3 fewer, 10%: none.
        report(2, 27, true),
        // 7 fewer, 26%.
        report(3, 20, true),
        // Unknown, then known again: no comparison across the gap.
        report(4, 5, false),
        report(5, 1, true),
        // 1 to 0 is all of them, but fewer than five.
        report(6, 0, true),
    ];
    let events = candidates(&commits, &observations, 9 * 24);
    assert_eq!(
        events,
        [
            (24 + 10, ClearingBurn, Low),
            (3 * 24 + 10, ClearingBurn, Low)
        ]
    );
    let strong = [report(0, 50, true), report(1, 10, true)];
    assert_eq!(
        candidates(&commits, &strong, 3 * 24),
        [(24 + 10, ClearingBurn, High)]
    );
    let state = replay(&commits, &strong, 24 + 10);
    let kinds: Vec<GroundMark> = state.events.marks.iter().map(|m| m.kind).collect();
    assert_eq!(kinds, [GroundMark::FreshSoil]);
    // A feature branch's report is never taken.
    let mut feature = report(1, 10, true);
    feature.head.at_default_tip = false;
    assert!(candidates(&commits, &[report(0, 50, true), feature], 3 * 24).is_empty());
}

#[test]
fn coverage_that_moves_two_hundredths_is_sunburst_or_dusk() {
    let commits = daily_commits(&[1; 6]);
    let report =
        |day: u32, percent: i32| coverage(day * 24 + 10, day + 1, Q16::from_ratio(percent, 100));
    let observations = [
        report(0, 50),
        report(1, 53),
        report(2, 40),
        report(3, 41),
        report(4, 52),
    ];
    assert_eq!(
        candidates(&commits, &observations, 6 * 24),
        [
            (24 + 10, Sunburst, Low),
            (2 * 24 + 10, Dusk, High),
            (4 * 24 + 10, Sunburst, High),
        ]
    );
}

#[test]
fn the_first_commit_after_two_silent_weeks_is_rain_after_drought() {
    for (days, expected) in [
        (13, None),
        (14, Some(Low)),
        (40, Some(Mid)),
        (100, Some(High)),
    ] {
        let commits = [commit(1, 0), commit(2, days * 24)];
        let events = candidates(&commits, &[], days * 24 + 1);
        let rain: Vec<Strength> = events
            .iter()
            .filter(|e| e.1 == RainAfterDrought)
            .map(|e| e.2)
            .collect();
        assert_eq!(
            rain,
            expected.into_iter().collect::<Vec<_>>(),
            "{days} days"
        );
    }
    // It washes the settled crack away and boosts the next close.
    let commits = [commit(1, 0), commit(2, 20 * 24 + 3)];
    let observations = [rewrite(10, "refs/heads/main", None, Vec::new())];
    let before = replay(&commits, &observations, 20 * 24 + 2);
    assert_eq!(before.events.marks.len(), 1);
    let after = replay(&commits, &observations, 20 * 24 + 3);
    assert!(after.events.marks.is_empty());
    assert!(after.vegetation.boost);
    assert!(!replay(&commits, &observations, 21 * 24).vegetation.boost);
}

#[test]
fn a_day_of_three_times_the_usual_churn_is_a_windstorm_that_brings_litter() {
    // Ten lines a day for three weeks, then a hundred on day 20.
    let mut commits = daily_commits(&[1; 25]);
    commits[20].added = 100;
    let events = candidates(&commits, &[], 25 * 24);
    assert_eq!(events, [(21 * 24, Windstorm, Mid)]);
    let before = replay(&commits, &[], 21 * 24 - 1);
    let after = replay(&commits, &[], 21 * 24);
    assert_eq!(
        after.vegetation.litter.len(),
        before.vegetation.litter.len() + 2
    );
    // While the baselines settle, a gust is no windstorm.
    let mut early = daily_commits(&[1; 12]);
    early[10].added = 1_000;
    assert!(candidates(&early, &[], 12 * 24).is_empty());
}

#[test]
fn a_branch_switch_fires_nothing() {
    let commits = daily_commits(&[1; 6]);
    let mut feature_coverage = coverage(30, 2, Q16::from_ratio(10, 100));
    feature_coverage.head.at_default_tip = false;
    let observations = [
        coverage(10, 1, Q16::from_ratio(80, 100)),
        weather(ci("main"), 11, 1, "main", Some(true)),
        weather(ci("feature/fern"), 12, 1, "feature/fern", Some(false)),
        // HEAD moves to the feature branch: the sky turns, nothing fires.
        weather(ci("feature/fern"), 20, 2, "feature/fern", Some(false)),
        feature_coverage,
        // Back to main, which still passes.
        weather(ci("main"), 40, 2, "main", Some(true)),
        rewrite(50, "refs/heads/feature/fern", Some(9), Vec::new()),
    ];
    assert!(candidates(&commits, &observations, 6 * 24).is_empty());
}

#[test]
fn the_scene_plays_the_current_candidate_and_shows_the_marks() {
    let commits = [commit(1, 0), commit(2, 5)];
    let observations = [rewrite(10, "refs/heads/main", Some(12), Vec::new())];
    let at_quake = replay(&commits, &observations, 10);
    let scene = project(&at_quake);
    let event = scene.event.expect("the earthquake plays");
    assert_eq!(
        (event.kind, event.started_tick, event.strength, event.fossil),
        (Earthquake, Tick::new(10), High, None)
    );
    assert_eq!(scene.ground_marks.len(), 1);
    assert_eq!(scene.validate(), Ok(()));
    // An hour later the crack stays and nothing new plays.
    let later = project(&replay(&commits, &observations, 11));
    assert_eq!(later.event, None);
    assert_eq!(later.ground_marks, scene.ground_marks);
    // The engine keeps the earthquake showing through it.
    let held = replay(&commits, &observations, 11);
    let playing = held.events.find((Tick::new(10), Earthquake));
    assert_eq!(project_showing(&held, playing).event, Some(event));
}
