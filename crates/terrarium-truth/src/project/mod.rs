//! `project`: the replayed state as the composer's `SceneInput`.
//!
//! The presentation, the sky and the glass are read from the climate and the timeline:
//! - weather: a pass is clear, a failure a storm, and no source that can tell is haze;
//! - light: the light of a coverage change, dusk for a week after a coverage report fell by
//!   0.02 or more from the one before it, and sunburst after one rose;
//! - wind: calm below 0.7 of the usual pace, breezy from 0.7, a gale from 3;
//! - glass: condensation while dormant and dust after a year dead.
//!
//! The plants, weeds, litter and fungus are the vegetation's
//! ([`crate::vegetation::Vegetation::views`]), the residents the fauna's
//! ([`crate::fauna::Fauna::views`]), the monuments and fossils the landmarks', and the ground
//! marks the events'. The event is the one playing: by default the best candidate of the newest
//! tick ([`project`]); the engine names another after a gap ([`project_showing`]). A playing
//! lightning scorches its plant.

use crate::climate::weather::InEffect;
use crate::events::Candidate;
use crate::fixed::Q16;
use crate::replay::SimState;
use crate::scene::{EventKind, Light, Presentation, SceneInput, Weather, Wind};
use crate::time::Tick;
use crate::tuning::{BREEZY_FROM, GALE_FROM, LIGHT_SHIFT_STEP, LIGHT_SHIFT_TICKS};

pub fn project(state: &SimState) -> SceneInput {
    project_showing(state, state.events.current(state.now()))
}

/// The scene with `playing` as its foreground event.
pub fn project_showing(state: &SimState, playing: Option<&Candidate>) -> SceneInput {
    let now = state.now();
    let (presentation, since_tick) = state.timeline.presentation();
    let scorched = playing
        .filter(|candidate| candidate.event.kind == EventKind::Lightning)
        .and_then(|candidate| candidate.target);
    let record = state.daily.last();
    SceneInput {
        presentation: Presentation {
            state: presentation,
            since_tick,
        },
        plants: state.vegetation.views(presentation, now, scorched),
        weeds: count(&state.vegetation.weeds),
        litter: count(&state.vegetation.litter),
        fungus: count(&state.vegetation.fungus),
        residents: state.fauna.views(),
        monuments: state.landmarks.monument_views(now),
        fossils: state.landmarks.fossil_views(),
        ground_marks: state.events.mark_views(),
        weather: weather(state.weather.in_effect.as_ref()),
        light: light(
            state
                .readings
                .coverage_change(Tick::new(now.hours().saturating_sub(LIGHT_SHIFT_TICKS))),
        ),
        wind: wind(record.map_or(Q16::ONE, |r| r.wind)),
        condensation: state.timeline.condensation(now),
        dust: state.timeline.dust(now),
        event: playing.map(|candidate| candidate.event),
    }
}

/// A count of ground cover, saturated to the scene's `u8`.
fn count<T>(items: &[T]) -> u8 {
    u8::try_from(items.len()).unwrap_or(u8::MAX)
}

fn weather(in_effect: Option<&InEffect>) -> Weather {
    match in_effect {
        Some(InEffect { pass: true, .. }) => Weather::Clear,
        Some(InEffect { pass: false, .. }) => Weather::Storm,
        None => Weather::Haze,
    }
}

fn light(coverage_change: Option<Q16>) -> Light {
    match coverage_change {
        Some(change) if change >= LIGHT_SHIFT_STEP => Light::Sunburst,
        Some(change) if change <= Q16::ZERO.saturating_sub(LIGHT_SHIFT_STEP) => Light::Dusk,
        _ => Light::Neutral,
    }
}

fn wind(ratio: Q16) -> Wind {
    if ratio >= GALE_FROM {
        Wind::Gale
    } else if ratio >= BREEZY_FROM {
        Wind::Breezy
    } else {
        Wind::Calm
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::WeatherSource;

    #[test]
    fn the_sky_reads_the_weather_and_the_wind() {
        let local = |pass| InEffect {
            source: WeatherSource::Local,
            pass,
        };
        assert_eq!(weather(Some(&local(true))), Weather::Clear);
        assert_eq!(weather(Some(&local(false))), Weather::Storm);
        assert_eq!(weather(None), Weather::Haze);
        assert_eq!(wind(Q16::ZERO), Wind::Calm);
        assert_eq!(wind(BREEZY_FROM.saturating_sub(Q16::EPSILON)), Wind::Calm);
        assert_eq!(wind(BREEZY_FROM), Wind::Breezy);
        assert_eq!(wind(Q16::ONE), Wind::Breezy);
        assert_eq!(wind(GALE_FROM), Wind::Gale);
        let step = LIGHT_SHIFT_STEP;
        assert_eq!(light(None), Light::Neutral);
        assert_eq!(light(Some(step)), Light::Sunburst);
        assert_eq!(
            light(Some(step.saturating_sub(Q16::EPSILON))),
            Light::Neutral
        );
        assert_eq!(light(Some(Q16::ZERO.saturating_sub(step))), Light::Dusk);
    }
}
