//! `--demo`: the synthetic climate's named states in turn, a quarter of an hour each, so every
//! look can be watched on the strip without a repository.
//!
//! Each stage eases in from the one before over its first five minutes, so plants change one
//! by one and residents settle, then holds for ten. Weather, light and the idle days switch at
//! once halfway through the ease, as truth's would within a tick. A storm passes during the
//! low-light stage, so hiding and emerging can be seen. The cycle counts from launch.

use terrarium_compose::synthetic::{self, Knobs};
use terrarium_truth::scene::Weather;

/// How long each stage lasts, and how long it takes to ease in.
pub const STAGE_MS: u64 = 15 * 60_000;
pub const EASE_MS: u64 = 5 * 60_000;
/// When the discrete knobs switch, within the ease.
pub const SWITCH_MS: u64 = EASE_MS / 2;
/// One pass through every stage.
pub const LOOP_MS: u64 = STAGE_MS * synthetic::NAMED.len() as u64;
/// The storm passes through the low-light stage, after it has settled.
const STORM_STAGE: &str = "struggling-low-light";
const STORM_FROM_MS: u64 = 6 * 60_000;
const STORM_UNTIL_MS: u64 = 10 * 60_000;

/// The stage at `elapsed` milliseconds since launch, by name.
#[cfg_attr(not(feature = "dev"), allow(dead_code))]
pub fn stage_at(elapsed: u64) -> &'static str {
    synthetic::NAMED[stage_index(elapsed)].0
}

fn stage_index(elapsed: u64) -> usize {
    usize::try_from((elapsed % LOOP_MS) / STAGE_MS).unwrap_or(0)
}

/// The climate `elapsed` milliseconds after launch.
pub fn knobs_at(elapsed: u64) -> Knobs {
    let index = stage_index(elapsed);
    let within = elapsed % STAGE_MS;
    let (name, to) = synthetic::NAMED[index];
    // The very first stage starts settled: there is no stage before it to ease from.
    let from = if elapsed < STAGE_MS {
        to
    } else {
        synthetic::NAMED[(index + synthetic::NAMED.len() - 1) % synthetic::NAMED.len()].1
    };
    let progress = within.min(EASE_MS);
    let blend = |from: u16, to: u16| -> u16 {
        let (from, to) = (u64::from(from), u64::from(to));
        let value = if to >= from {
            from + (to - from) * progress / EASE_MS
        } else {
            from - (from - to) * progress / EASE_MS
        };
        u16::try_from(value).unwrap_or(u16::MAX)
    };
    let percent = |from: u8, to: u8| u8::try_from(blend(from.into(), to.into())).unwrap_or(u8::MAX);
    let discrete = if within < SWITCH_MS { from } else { to };
    let storm = name == STORM_STAGE && (STORM_FROM_MS..STORM_UNTIL_MS).contains(&within);
    Knobs {
        light: percent(from.light, to.light),
        soil: percent(from.soil, to.soil),
        rain: blend(from.rain, to.rain),
        wind: blend(from.wind, to.wind),
        weather: if storm {
            Weather::Storm
        } else {
            discrete.weather
        },
        ..discrete
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: u64 = 60_000;

    #[test]
    fn the_demo_starts_settled_in_thriving_and_loops_every_seven_stages() {
        assert_eq!(knobs_at(0), synthetic::named("thriving").unwrap());
        assert_eq!(stage_at(0), "thriving");
        assert_eq!(LOOP_MS, 105 * MINUTE);
        assert_eq!(knobs_at(LOOP_MS + EASE_MS), knobs_at(EASE_MS));
        let order: Vec<&str> = (0..7).map(|i| stage_at(i * STAGE_MS)).collect();
        assert_eq!(
            order,
            [
                "thriving",
                "mature",
                "struggling-low-light",
                "struggling-poor-soil",
                "dormant",
                "dead",
                "recovering"
            ]
        );
    }

    #[test]
    fn every_stage_holds_its_named_state_once_eased_in() {
        for pass in 0..2 {
            for (i, (name, knobs)) in synthetic::NAMED.iter().enumerate() {
                let start = pass * LOOP_MS + u64::try_from(i).unwrap() * STAGE_MS;
                for minute in 5..15 {
                    let at = start + minute * MINUTE;
                    let mut held = knobs_at(at);
                    if held.weather == Weather::Storm {
                        assert_eq!(*name, STORM_STAGE);
                        assert!((6..10).contains(&minute), "{name} storm at {minute}");
                        held.weather = knobs.weather;
                    }
                    assert_eq!(held, *knobs, "{name}, minute {minute}");
                }
            }
        }
    }

    #[test]
    fn the_state_shows_through_each_stage() {
        for (i, (name, knobs)) in synthetic::NAMED.iter().enumerate() {
            let held = knobs_at(LOOP_MS + u64::try_from(i).unwrap() * STAGE_MS + EASE_MS);
            assert_eq!(held.state(), knobs.state(), "{name}");
        }
    }

    #[test]
    fn the_climate_eases_minute_by_minute_and_switches_only_at_its_moments() {
        // Two loops, a second apart: each continuous knob moves at most a fifth of its stage's
        // change a minute, and the discrete ones only at the switch or the storm's ends.
        let mut before = knobs_at(0);
        for second in 1..=2 * LOOP_MS / 1000 {
            let at = second * 1000;
            let now = knobs_at(at);
            let within = at % STAGE_MS;
            let index = stage_index(at);
            let to = synthetic::NAMED[index].1;
            let from =
                synthetic::NAMED[(index + synthetic::NAMED.len() - 1) % synthetic::NAMED.len()].1;
            let step = |a: u16, b: u16, from: u16, to: u16| {
                let most = u64::from(from.abs_diff(to)) * 1000 / EASE_MS + 1;
                assert!(u64::from(a.abs_diff(b)) <= most, "{at}: {a} to {b}");
            };
            step(
                before.light.into(),
                now.light.into(),
                from.light.into(),
                to.light.into(),
            );
            step(
                before.soil.into(),
                now.soil.into(),
                from.soil.into(),
                to.soil.into(),
            );
            step(before.rain, now.rain, from.rain, to.rain);
            step(before.wind, now.wind, from.wind, to.wind);
            let switched = (
                before.weather,
                before.shift,
                before.idle_days,
                before.recovering,
                before.age_years,
            ) != (
                now.weather,
                now.shift,
                now.idle_days,
                now.recovering,
                now.age_years,
            );
            if switched {
                assert!(
                    [SWITCH_MS, STORM_FROM_MS, STORM_UNTIL_MS].contains(&within),
                    "a switch at {at} ({within} into its stage)"
                );
            }
            before = now;
        }
    }
}
