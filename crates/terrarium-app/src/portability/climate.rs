//! The climate export: the presentation's timeline and each finished day's climate, as JSON for
//! anyone who builds on them. Format version 1 is described in the crate's README.
//!
//! The days are written as the replay reaches them, one line each, so nothing is held but the
//! day at hand; the timeline follows, once the replay has ended. Fixed-point values are
//! written as the exact numbers they stand for, and an unknown one as `null`, never 0 (I-3).

use std::io::{self, Write};

use serde::Serialize;
use terrarium_compose::sky::LocalDate;
use terrarium_engine::timelapse::{Day, Lapse};
use terrarium_truth::domain::{ActivityClass, HealthClass, WeatherReading, WeatherSource};
use terrarium_truth::timeline::TimelineEntry;
use terrarium_truth::{PresentationState, Q16, Reading, ReplayError, RepoSeed, SIM_VERSION};

pub const FORMAT: &str = "repo-terrarium-climate";
pub const FORMAT_VERSION: u32 = 1;

/// Why the climate could not be written.
#[derive(Debug)]
pub enum ClimateError {
    Replay(ReplayError),
    Io(io::Error),
}

impl From<ReplayError> for ClimateError {
    fn from(err: ReplayError) -> ClimateError {
        ClimateError::Replay(err)
    }
}

impl From<io::Error> for ClimateError {
    fn from(err: io::Error) -> ClimateError {
        ClimateError::Io(err)
    }
}

impl From<serde_json::Error> for ClimateError {
    fn from(err: serde_json::Error) -> ClimateError {
        ClimateError::Io(err.into())
    }
}

#[derive(Serialize)]
struct Header {
    format: &'static str,
    format_version: u32,
    seed: String,
    sim_version: u32,
}

/// A day's climate as its close left it.
#[derive(Serialize)]
struct DayLine {
    /// The UTC day.
    date: String,
    /// Light and soil smoothed, as health reads them, and whether the reading behind each is
    /// stale.
    light: Option<Held>,
    soil: Option<Held>,
    /// The soil's components, as last reported.
    dead_code: Option<f64>,
    complexity: Option<f64>,
    todo_density: Option<f64>,
    /// Rain and wind against the repository's own pace: 1 is its usual.
    rain: f64,
    wind: f64,
    health: f64,
    activity: f64,
    /// The default tip's text lines.
    terrain: u64,
    weather: WeatherLine,
}

#[derive(Serialize)]
struct Held {
    value: f64,
    stale: bool,
}

#[derive(Serialize)]
struct WeatherLine {
    /// `pass` or `fail`; `null` while unknown.
    state: Option<&'static str>,
    /// `ci:<branch>` or `local`; `null` for haze.
    source: Option<String>,
    stale: Option<bool>,
}

/// A change of the presentation, or of either class behind it.
#[derive(Serialize)]
struct Change {
    /// The UTC hour it began.
    at: String,
    state: &'static str,
    health: &'static str,
    activity: &'static str,
}

/// Writes the climate of `lapse`, the terrarium of `seed`, to `out`. Returns the days written.
pub fn write(lapse: &Lapse, seed: RepoSeed, out: &mut dyn Write) -> Result<u64, ClimateError> {
    let header = serde_json::to_string(&Header {
        format: FORMAT,
        format_version: FORMAT_VERSION,
        seed: seed.to_hex(),
        sim_version: SIM_VERSION.0,
    })?;
    let open = header.strip_suffix('}').expect("an object");
    writeln!(out, "{open},\"days\":[")?;
    let mut days = 0_u64;
    let timeline = lapse.days(&mut |day: Day<'_>| -> Result<(), ClimateError> {
        if days > 0 {
            out.write_all(b",\n")?;
        }
        serde_json::to_writer(&mut *out, &day_line(&day))?;
        days += 1;
        Ok(())
    })?;
    out.write_all(b"\n],\"timeline\":[\n")?;
    for (i, entry) in timeline.iter().enumerate() {
        if i > 0 {
            out.write_all(b",\n")?;
        }
        serde_json::to_writer(&mut *out, &change(lapse, *entry))?;
    }
    out.write_all(b"\n]}\n")?;
    out.flush()?;
    Ok(days)
}

fn day_line(day: &Day<'_>) -> DayLine {
    let record = day.record;
    let climate = &day.report.climate;
    let held = |smoothed: Option<Q16>, reading: Reading| {
        smoothed.map(|value| Held {
            value: number(value),
            stale: matches!(reading, Reading::Known { stale: true, .. }),
        })
    };
    DayLine {
        date: iso_date(LocalDate::from_days_since_epoch(day.day)),
        light: held(record.light, climate.light),
        soil: held(record.soil, climate.soil),
        dead_code: record.dead_code.map(number),
        complexity: record.complexity.map(number),
        todo_density: record.todo_density.map(number),
        rain: number(record.rain),
        wind: number(record.wind),
        health: number(record.health),
        activity: number(record.activity),
        terrain: record.terrain,
        weather: weather(&climate.weather),
    }
}

fn weather(weather: &WeatherReading) -> WeatherLine {
    let (word, held) = match weather.state {
        Reading::Unknown => (None, None),
        Reading::Known { value, stale, .. } => (
            Some(if value == Q16::ONE { "pass" } else { "fail" }),
            Some(stale),
        ),
    };
    WeatherLine {
        state: word,
        source: weather.source.as_ref().map(WeatherSource::to_column),
        stale: held,
    }
}

fn change(lapse: &Lapse, entry: TimelineEntry) -> Change {
    let secs = lapse.tick_secs(entry.tick);
    Change {
        at: format!(
            "{}T{:02}:00:00Z",
            iso_date(LocalDate::from_days_since_epoch(secs.div_euclid(86_400))),
            secs.rem_euclid(86_400) / 3600
        ),
        state: match entry.state {
            PresentationState::Thriving => "thriving",
            PresentationState::Struggling => "struggling",
            PresentationState::Mature => "mature",
            PresentationState::Dormant => "dormant",
            PresentationState::Dead => "dead",
            PresentationState::Recovering => "recovering",
        },
        health: match entry.health {
            HealthClass::Healthy => "healthy",
            HealthClass::Unhealthy => "unhealthy",
        },
        activity: match entry.activity {
            ActivityClass::Inactive => "inactive",
            ActivityClass::Active => "active",
            ActivityClass::Quiet => "quiet",
        },
    }
}

/// A Q16.16 value as the number it stands for: exact, since every one is a binary fraction
/// that an `f64` holds whole.
fn number(value: Q16) -> f64 {
    f64::from(value.raw()) / 65_536.0
}

fn iso_date(date: LocalDate) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use terrarium_adapters::fixture::{self, Edit, FixtureRepo, Stamp};
    use terrarium_engine::headless::{Request, lapse};
    use terrarium_store::DataDir;
    use terrarium_truth::canon::to_hex;

    use super::*;
    use crate::timelapse::tests::arc;

    fn written(lapse: &Lapse) -> (u64, Vec<u8>) {
        let mut out = Vec::new();
        let days = write(lapse, RepoSeed::from_bytes([1; 32]), &mut out).unwrap();
        (days, out)
    }

    /// Every fraction written in `text`, read as written: `serde_json`'s own reading is not
    /// always the nearest `f64`, and a reader's may be.
    fn fractions(text: &str) -> Vec<(String, f64)> {
        text.split([',', ':', '{', '}', '[', ']'])
            .filter(|token| token.contains('.'))
            .filter_map(|token| Some((token.to_owned(), token.parse().ok()?)))
            .collect()
    }

    #[test]
    fn the_arc_s_climate_reads_back_a_day_a_line_then_its_timeline() {
        let (_dir, lapse) = arc();
        let closes = lapse.closes();
        let (days, out) = written(&lapse);
        assert_eq!(days, u64::try_from(closes.len()).unwrap());
        let text = std::str::from_utf8(&out).unwrap();
        let value: Value = serde_json::from_str(text).unwrap();
        assert_eq!(value["format"], FORMAT);
        assert_eq!(value["format_version"], FORMAT_VERSION);
        assert_eq!(value["seed"], "01".repeat(32));
        assert_eq!(value["sim_version"], SIM_VERSION.0);
        let lines = value["days"].as_array().unwrap();
        assert_eq!(lines.len(), closes.len());
        let date = |close| iso_date(LocalDate::from_days_since_epoch(lapse.day(close)));
        assert_eq!(lines[0]["date"], date(closes[0]));
        assert_eq!(
            lines[lines.len() - 1]["date"],
            date(*closes.last().unwrap())
        );
        // Unknown is null, never 0: light is unknown until the first coverage report.
        assert!(lines[0]["light"].is_null());
        let known = lines
            .iter()
            .filter(|day| day["light"]["value"].is_number())
            .count();
        assert!(known > 0 && known < lines.len());
        assert!(lines.iter().any(|day| day["weather"]["state"] == "fail"));
        assert!(lines.iter().any(|day| day["weather"]["source"] == "local"));
        // Each fraction is an exact Q16.16, a whole count of 65536ths, written so it reads
        // back as itself.
        let (_, days_text) = text.split_once("\"days\":[").unwrap();
        let (days_text, _) = days_text.split_once("],\"timeline\"").unwrap();
        let all = fractions(days_text);
        assert!(all.len() > lines.len() * 5, "{}", all.len());
        for (token, n) in all {
            assert!((n * 65_536.0).fract().abs() < f64::EPSILON, "{token}");
        }
        let timeline = value["timeline"].as_array().unwrap();
        assert!(timeline.len() > 3);
        assert_eq!(timeline[0]["state"], "thriving");
        assert!(timeline.iter().any(|change| change["state"] == "dead"));
        assert!(timeline[0]["at"].as_str().unwrap().ends_with(":00:00Z"));
        // A day a line, between the header's line and the timeline's.
        assert_eq!(text.lines().count(), lines.len() + timeline.len() + 3);
        assert_eq!(written(&lapse).1, out, "the same bytes every time");
        insta::assert_snapshot!(
            "arc-climate",
            format!(
                "{} bytes, SHA-256 {}",
                out.len(),
                to_hex(&Sha256::digest(&out))
            )
        );
    }

    #[test]
    fn a_history_with_no_day_finished_has_no_day_and_one_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = FixtureRepo::init(&dir.path().join("repo")).unwrap();
        repo.commit(
            "main",
            Stamp::at(fixture::ARC_START_SECS + 3600),
            "a note",
            &[Edit::write("notes.txt", "a line\n")],
        )
        .unwrap();
        let lapse = lapse(Request {
            repo: repo.path(),
            data: &DataDir::at(dir.path().join("data")),
            observations: None,
            at_secs: fixture::ARC_START_SECS + 7200,
            install_salt: [0; 16],
        })
        .unwrap();
        let (days, out) = written(&lapse);
        assert_eq!(days, 0);
        let value: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["days"], Value::Array(vec![]));
        assert!(value["timeline"].as_array().unwrap().len() <= 1);
    }
}
