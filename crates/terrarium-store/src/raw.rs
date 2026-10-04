//! Each observation kind's `raw` JSON, and the part of it truth reads.
//!
//! Shapes are keyed by kind, not by `adapter_version`, and only ever gain optional keys: a key
//! once written keeps its meaning, so every stored row and every export stays readable. Unknown
//! keys are ignored. The store enforces only what truth reads, dead code's counts and a ref
//! rewrite's refs, so a row truth could not convert never enters the append-only table. The
//! other shapes are what adapters write, and are checked only as JSON.

use serde::{Deserialize, Serialize};
use terrarium_truth::domain::{ObservationValue, WeatherSource};
use terrarium_truth::{CommitId, ObservationKind};

use crate::Invalid;

/// `coverage`: the line totals of a coverage report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageRaw {
    pub format: CoverageFormat,
    pub lines_found: u64,
    pub lines_hit: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverageFormat {
    Lcov,
    Cobertura,
}

/// `dead_code`: what `normalized` is computed from,
/// `1 − min(1, unused_items / max(1, exported_items))`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadCodeRaw {
    pub unused_items: u32,
    pub exported_items: u32,
}

/// `weather`: the run's outcome, which `normalized` repeats (pass 1, fail 0, unknown null).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherRaw {
    pub state: WeatherState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WeatherState {
    Pass,
    Fail,
    Unknown,
}

/// `ref_rewrite`: a default-branch ref moved from `old` to `new` without fast-forwarding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefRewriteRaw {
    #[serde(rename = "ref")]
    pub reference: String,
    pub old: CommitId,
    pub new: CommitId,
}

/// A raw shape: what an adapter writes into `raw`.
pub trait Shape: Serialize {
    /// The JSON text.
    ///
    /// # Panics
    ///
    /// Never: every shape is plain data, which always encodes.
    fn to_raw(&self) -> String {
        serde_json::to_string(self).expect("raw shapes always encode")
    }
}

impl Shape for CoverageRaw {}
impl Shape for DeadCodeRaw {}
impl Shape for WeatherRaw {}
impl Shape for RefRewriteRaw {}

/// The kind-specific value truth reads from `raw` (and, for weather, the source column).
pub(crate) fn value(
    kind: ObservationKind,
    source: Option<&WeatherSource>,
    raw: &str,
) -> Result<ObservationValue, Invalid> {
    // serde would read a struct from a JSON array too; a shape is always an object.
    let object = || -> Result<serde_json::Value, Invalid> {
        match serde_json::from_str(raw) {
            Ok(serde_json::Value::Object(map)) => Ok(serde_json::Value::Object(map)),
            _ => Err(Invalid::RawShape(kind)),
        }
    };
    let shape = |_| Invalid::RawShape(kind);
    Ok(match kind {
        ObservationKind::Coverage => ObservationValue::Coverage,
        ObservationKind::Complexity => ObservationValue::Complexity,
        ObservationKind::TodoDensity => ObservationValue::TodoDensity,
        ObservationKind::DeadCode => {
            let DeadCodeRaw {
                unused_items,
                exported_items,
            } = serde_json::from_value(object()?).map_err(shape)?;
            ObservationValue::DeadCode {
                unused_items,
                exported_items,
            }
        }
        ObservationKind::Weather => ObservationValue::Weather {
            source: source.cloned().ok_or(Invalid::WeatherWithoutSource)?,
        },
        ObservationKind::RefRewrite => {
            let RefRewriteRaw {
                reference,
                old,
                new,
            } = serde_json::from_value(object()?).map_err(shape)?;
            ObservationValue::RefRewrite {
                reference,
                old,
                new,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(digit: char) -> CommitId {
        CommitId::from_hex(&digit.to_string().repeat(40)).expect("an id")
    }

    #[test]
    fn the_shapes_are_pinned() {
        // Stored data: these texts never change.
        let coverage = CoverageRaw {
            format: CoverageFormat::Lcov,
            lines_found: 120,
            lines_hit: 90,
        };
        assert_eq!(
            coverage.to_raw(),
            r#"{"format":"lcov","lines_found":120,"lines_hit":90}"#
        );
        let dead = DeadCodeRaw {
            unused_items: 4,
            exported_items: 80,
        };
        assert_eq!(dead.to_raw(), r#"{"unused_items":4,"exported_items":80}"#);
        let weather = WeatherRaw {
            state: WeatherState::Unknown,
        };
        assert_eq!(weather.to_raw(), r#"{"state":"unknown"}"#);
        let rewrite = RefRewriteRaw {
            reference: "refs/heads/main".into(),
            old: commit('a'),
            new: commit('b'),
        };
        assert_eq!(
            rewrite.to_raw(),
            format!(
                r#"{{"ref":"refs/heads/main","old":"{}","new":"{}"}}"#,
                "a".repeat(40),
                "b".repeat(40)
            )
        );
    }

    #[test]
    fn truth_reads_what_it_needs_and_ignores_the_rest() {
        let dead = value(
            ObservationKind::DeadCode,
            None,
            r#"{"unused_items":4,"exported_items":80,"knip":"5.61.0"}"#,
        );
        assert_eq!(
            dead,
            Ok(ObservationValue::DeadCode {
                unused_items: 4,
                exported_items: 80
            })
        );
        let raw = RefRewriteRaw {
            reference: "refs/heads/main".into(),
            old: commit('a'),
            new: commit('b'),
        }
        .to_raw();
        assert_eq!(
            value(ObservationKind::RefRewrite, None, &raw),
            Ok(ObservationValue::RefRewrite {
                reference: "refs/heads/main".into(),
                old: commit('a'),
                new: commit('b')
            })
        );
        let local = WeatherSource::Local;
        assert_eq!(
            value(ObservationKind::Weather, Some(&local), "{}"),
            Ok(ObservationValue::Weather { source: local })
        );
        for kind in [
            ObservationKind::Coverage,
            ObservationKind::Complexity,
            ObservationKind::TodoDensity,
        ] {
            assert!(value(kind, None, r#"{"anything":[1,2]}"#).is_ok());
        }
    }

    #[test]
    fn a_shape_truth_cannot_read_is_refused() {
        for (kind, raw) in [
            (ObservationKind::DeadCode, r#"{"unused_items":4}"#),
            (
                ObservationKind::DeadCode,
                r#"{"unused_items":-1,"exported_items":2}"#,
            ),
            (ObservationKind::DeadCode, "[4, 80]"),
            (
                ObservationKind::RefRewrite,
                r#"{"ref":"main","old":"a","new":"b"}"#,
            ),
            (ObservationKind::RefRewrite, r#"{"reference":"main"}"#),
        ] {
            assert_eq!(
                value(kind, None, raw),
                Err(Invalid::RawShape(kind)),
                "{raw}"
            );
        }
    }
}
