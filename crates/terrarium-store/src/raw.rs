//! Each observation kind's `raw` JSON, and the part of it truth reads.
//!
//! Shapes are keyed by kind, not by `adapter_version`, and only ever gain optional keys: a key
//! once written keeps its meaning, so every stored row and every export stays readable. Unknown
//! keys are ignored. The store enforces only what truth reads, dead code's counts and a ref
//! rewrite's refs, so a row truth could not convert never enters the append-only table. The
//! other shapes are what adapters write, and are checked only as JSON.

use serde::de::IgnoredAny;
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

/// `weather`: the run's outcome, which `normalized` repeats (pass 1, fail 0, unknown null), and
/// what the run said beyond it, when it said: a test report's counts, or the exit code of a run
/// `record-tests` recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherRaw {
    pub state: WeatherState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    /// A test report's failure outside any test case: a suite that did not load, an unhandled
    /// error. Present only when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite_failed: Option<bool>,
}

impl WeatherRaw {
    /// An outcome with nothing more to say.
    pub const fn of(state: WeatherState) -> WeatherRaw {
        WeatherRaw {
            state,
            tests: None,
            failed: None,
            skipped: None,
            exit_code: None,
            suite_failed: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WeatherState {
    Pass,
    Fail,
    Unknown,
}

/// `ref_rewrite`: a default-branch ref moved from `old` to `new` without fast-forwarding.
/// `rewritten` counts the commits it took off the default branch's first-parent chain, and
/// `fossils` names the commits of the fossils shown before it that it took off; rows written
/// before either was recorded lack them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefRewriteRaw {
    #[serde(rename = "ref")]
    pub reference: String,
    pub old: CommitId,
    pub new: CommitId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewritten: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fossils: Vec<CommitId>,
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

/// The kind-specific value truth reads from `raw` (and, for weather, the source column). Only
/// the kinds whose value is in `raw` parse it.
pub(crate) fn value(
    kind: ObservationKind,
    source: Option<&WeatherSource>,
    raw: &str,
) -> Result<ObservationValue, Invalid> {
    let json = match kind {
        ObservationKind::DeadCode | ObservationKind::RefRewrite => {
            serde_json::from_str(raw).map_err(|_| Invalid::RawShape(kind))?
        }
        _ => serde_json::Value::Null,
    };
    from_json(kind, source, json)
}

/// `raw` parsed once, for [`ObservationRecord::check`](crate::ObservationRecord::check): its
/// syntax for every kind, and the value itself for the kinds whose value is in it. For the
/// others, any text a JSON parser accepts passes, numbers of any size included, so nothing is
/// built from it.
pub(crate) fn parse(kind: ObservationKind, raw: &str) -> Result<serde_json::Value, Invalid> {
    let is_json = |raw: &str| serde_json::from_str::<IgnoredAny>(raw).is_ok();
    match kind {
        ObservationKind::DeadCode | ObservationKind::RefRewrite => serde_json::from_str(raw)
            .map_err(|_| {
                if is_json(raw) {
                    Invalid::RawShape(kind)
                } else {
                    Invalid::RawNotJson
                }
            }),
        _ if is_json(raw) => Ok(serde_json::Value::Null),
        _ => Err(Invalid::RawNotJson),
    }
}

/// [`value`], from `raw` already parsed.
pub(crate) fn from_json(
    kind: ObservationKind,
    source: Option<&WeatherSource>,
    json: serde_json::Value,
) -> Result<ObservationValue, Invalid> {
    // serde would read a struct from a JSON array too; a shape is always an object.
    let object = || -> Result<serde_json::Value, Invalid> {
        match json {
            serde_json::Value::Object(map) => Ok(serde_json::Value::Object(map)),
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
                rewritten,
                fossils,
            } = serde_json::from_value(object()?).map_err(shape)?;
            ObservationValue::RefRewrite {
                reference,
                old,
                new,
                rewritten,
                fossils,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::commit;

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
        let weather = WeatherRaw::of(WeatherState::Unknown);
        assert_eq!(weather.to_raw(), r#"{"state":"unknown"}"#);
        let tests = WeatherRaw {
            tests: Some(12),
            failed: Some(1),
            skipped: Some(2),
            ..WeatherRaw::of(WeatherState::Fail)
        };
        assert_eq!(
            tests.to_raw(),
            r#"{"state":"fail","tests":12,"failed":1,"skipped":2}"#
        );
        let suite = WeatherRaw {
            tests: Some(3),
            failed: Some(0),
            skipped: Some(0),
            suite_failed: Some(true),
            ..WeatherRaw::of(WeatherState::Fail)
        };
        assert_eq!(
            suite.to_raw(),
            r#"{"state":"fail","tests":3,"failed":0,"skipped":0,"suite_failed":true}"#
        );
        let run = WeatherRaw {
            exit_code: Some(0),
            ..WeatherRaw::of(WeatherState::Pass)
        };
        assert_eq!(run.to_raw(), r#"{"state":"pass","exit_code":0}"#);
        let rewrite = RefRewriteRaw {
            reference: "refs/heads/main".into(),
            old: commit('a'),
            new: commit('b'),
            rewritten: None,
            fossils: Vec::new(),
        };
        assert_eq!(
            rewrite.to_raw(),
            format!(
                r#"{{"ref":"refs/heads/main","old":"{}","new":"{}"}}"#,
                "a".repeat(40),
                "b".repeat(40)
            )
        );
        let dropping = RefRewriteRaw {
            rewritten: Some(2),
            fossils: vec![commit('c')],
            ..rewrite
        };
        assert_eq!(
            dropping.to_raw(),
            format!(
                r#"{{"ref":"refs/heads/main","old":"{}","new":"{}","rewritten":2,"fossils":["{}"]}}"#,
                "a".repeat(40),
                "b".repeat(40),
                "c".repeat(40)
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
        // A row written before the counts were recorded reads with none.
        let old_row = format!(
            r#"{{"ref":"refs/heads/main","old":"{}","new":"{}"}}"#,
            "a".repeat(40),
            "b".repeat(40)
        );
        assert_eq!(
            value(ObservationKind::RefRewrite, None, &old_row),
            Ok(ObservationValue::RefRewrite {
                reference: "refs/heads/main".into(),
                old: commit('a'),
                new: commit('b'),
                rewritten: None,
                fossils: Vec::new(),
            })
        );
        let raw = RefRewriteRaw {
            reference: "refs/heads/main".into(),
            old: commit('a'),
            new: commit('b'),
            rewritten: Some(4),
            fossils: vec![commit('d'), commit('e')],
        }
        .to_raw();
        assert_eq!(
            value(ObservationKind::RefRewrite, None, &raw),
            Ok(ObservationValue::RefRewrite {
                reference: "refs/heads/main".into(),
                old: commit('a'),
                new: commit('b'),
                rewritten: Some(4),
                fossils: vec![commit('d'), commit('e')],
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
        let ids = format!(r#""old":"{}","new":"{}""#, "a".repeat(40), "b".repeat(40));
        for extra in [
            r#""fossils":["zz"]"#,
            r#""rewritten":-1"#,
            r#""fossils":"c""#,
        ] {
            let raw = format!(r#"{{"ref":"refs/heads/main",{ids},{extra}}}"#);
            assert_eq!(
                value(ObservationKind::RefRewrite, None, &raw),
                Err(Invalid::RawShape(ObservationKind::RefRewrite)),
                "{raw}"
            );
        }
    }
}
