//! `project`: the replayed state as the composer's `SceneInput`.
//!
//! The presentation, the sky and the glass are read from the climate and the timeline:
//! - weather: a pass is clear, a failure a storm, and no source that can tell is haze;
//! - light: the light of a coverage change, dusk while light has fallen over the last week and
//!   sunburst while it has risen;
//! - wind: calm below 0.7 of the usual pace, breezy from 0.7, a gale from 3;
//! - glass: condensation while dormant and dust after a year dead.
//!
//! Vegetation is [`placeholder`]'s. Residents, monuments, fossils, ground cover, ground marks
//! and events stay empty: the sim that grows them does not exist yet.

pub mod placeholder;

use crate::climate::weather::InEffect;
use crate::climate::{Trend, Trends};
use crate::fixed::Q16;
use crate::replay::SimState;
use crate::scene::{Light, Presentation, SceneInput, Weather, Wind};
use crate::tuning::{BREEZY_FROM, GALE_FROM, NEUTRAL};

pub fn project(state: &SimState) -> SceneInput {
    let now = state.now();
    let (presentation, since_tick) = state.timeline.presentation();
    let record = state.daily.last();
    SceneInput {
        presentation: Presentation {
            state: presentation,
            since_tick,
        },
        plants: placeholder::plants(
            presentation,
            record.map_or(NEUTRAL, |r| r.health),
            record.and_then(|r| r.light),
            record.and_then(|r| r.soil),
        ),
        weeds: 0,
        litter: 0,
        fungus: 0,
        residents: Vec::new(),
        monuments: Vec::new(),
        fossils: Vec::new(),
        ground_marks: Vec::new(),
        weather: weather(state.weather.in_effect.as_ref()),
        light: match Trends::of(&state.daily).light {
            Some(Trend::Down) => Light::Dusk,
            Some(Trend::Up) => Light::Sunburst,
            Some(Trend::Flat) | None => Light::Neutral,
        },
        wind: wind(record.map_or(Q16::ONE, |r| r.wind)),
        condensation: state.timeline.condensation(now),
        dust: state.timeline.dust(now),
        event: None,
    }
}

fn weather(in_effect: Option<&InEffect>) -> Weather {
    match in_effect {
        Some(InEffect { pass: true, .. }) => Weather::Clear,
        Some(InEffect { pass: false, .. }) => Weather::Storm,
        None => Weather::Haze,
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
    }
}
