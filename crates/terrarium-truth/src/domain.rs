//! Truth's domain types: readings and climate, habitat, the facts read from git, the
//! observations recorded about a repository, and the presentation classes.

use serde::{Deserialize, Serialize};

use crate::canon::{Canonical, DumpError, Reader, Writer, canonical_enum, canonical_struct};
use crate::fixed::Q16;
use crate::identity::CommitId;
use crate::time::Tick;

/// One climate input, or the fact that it is unknown.
///
/// Unknown is a state, never a number (I-3): a missing tool must not look like decay. So there
/// is no `Default`, and no code can conjure a zero reading:
///
/// ```compile_fail
/// let reading: terrarium_truth::Reading = Default::default();
/// ```
///
/// A reading is built explicitly instead:
///
/// ```
/// let reading = terrarium_truth::Reading::Unknown;
/// assert_eq!(reading.known_value(), None);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reading {
    Unknown,
    Known {
        value: Q16,
        /// The default tip has moved past the reading's commit and no newer report exists.
        /// A stale value is held, never decayed.
        stale: bool,
        since_tick: Tick,
    },
}

impl Reading {
    pub const fn known_value(self) -> Option<Q16> {
        match self {
            Reading::Unknown => None,
            Reading::Known { value, .. } => Some(value),
        }
    }
}

impl Canonical for Reading {
    fn write(&self, w: &mut Writer) {
        match self {
            Reading::Unknown => w.leaf("Unknown"),
            Reading::Known {
                value,
                stale,
                since_tick,
            } => {
                w.leaf("Known");
                w.field("value", value);
                w.field("stale", stale);
                w.field("since_tick", since_tick);
            }
        }
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        match r.leaf()? {
            "Unknown" => Ok(Reading::Unknown),
            "Known" => Ok(Reading::Known {
                value: r.field("value")?,
                stale: r.field("stale")?,
                since_tick: r.field("since_tick")?,
            }),
            other => Err(r.bad_value(other)),
        }
    }
}

/// The five climates. Light and soil are health; rain and wind are tempo; weather is the test
/// signal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Climate {
    pub light: Reading,
    pub soil: Reading,
    pub rain: Reading,
    pub wind: Reading,
    pub weather: WeatherReading,
}

canonical_struct!(Climate {
    light,
    soil,
    rain,
    wind,
    weather
});

/// The weather: `state` is 1 for passing and 0 for failing, and `source` is where it came from.
/// No source at all is haze.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherReading {
    pub state: Reading,
    pub source: Option<WeatherSource>,
}

canonical_struct!(WeatherReading { state, source });

/// Where a weather reading came from: a CI run on a branch, or local test results.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WeatherSource {
    Ci { branch: String },
    Local,
}

impl WeatherSource {
    /// The `source` column's text: `ci:<branch>` or `local`. It is stored data, so it never
    /// changes.
    pub fn to_column(&self) -> String {
        match self {
            WeatherSource::Ci { branch } => format!("ci:{branch}"),
            WeatherSource::Local => "local".to_owned(),
        }
    }

    /// The inverse of [`WeatherSource::to_column`]. An empty branch is not a source.
    pub fn from_column(text: &str) -> Option<WeatherSource> {
        match text.strip_prefix("ci:") {
            Some("") => None,
            Some(branch) => Some(WeatherSource::Ci {
                branch: branch.to_owned(),
            }),
            None => (text == "local").then_some(WeatherSource::Local),
        }
    }
}

impl Canonical for WeatherSource {
    fn write(&self, w: &mut Writer) {
        match self {
            WeatherSource::Ci { branch } => {
                w.leaf("Ci");
                w.field("branch", branch);
            }
            WeatherSource::Local => w.leaf("Local"),
        }
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        match r.leaf()? {
            "Ci" => Ok(WeatherSource::Ci {
                branch: r.field("branch")?,
            }),
            "Local" => Ok(WeatherSource::Local),
            other => Err(r.bad_value(other)),
        }
    }
}

/// The habitat: `terrain` is the running count of text lines, `age` the time since the root
/// commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Habitat {
    pub terrain: u64,
    pub age: Tick,
}

canonical_struct!(Habitat { terrain, age });

/// What truth needs from one commit, read by the git adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitFacts {
    pub id: CommitId,
    /// The committer time, clamped monotonic along topological order.
    pub tick: Tick,
    pub parents: u32,
    pub on_default_first_parent: bool,
    /// Lines added and removed against the first parent, over text files.
    pub added: u64,
    pub removed: u64,
    /// Directories the commit deletes entirely, with the lines they held. A rename is not a
    /// deletion.
    pub deleted_dirs: Vec<DeletedDir>,
    pub renames: Vec<Rename>,
    /// Tags pointing at the commit.
    pub tags: Vec<TagFact>,
}

canonical_struct!(CommitFacts {
    id,
    tick,
    parents,
    on_default_first_parent,
    added,
    removed,
    deleted_dirs,
    renames,
    tags,
});

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletedDir {
    pub path: String,
    pub lines: u64,
}

canonical_struct!(DeletedDir { path, lines });

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rename {
    pub from: String,
    pub to: String,
}

canonical_struct!(Rename { from, to });

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagFact {
    pub name: String,
    /// The tag's date.
    pub tick: Tick,
}

canonical_struct!(TagFact { name, tick });

/// The observation kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ObservationKind {
    Coverage,
    DeadCode,
    Complexity,
    TodoDensity,
    Weather,
    RefRewrite,
}

canonical_enum!(ObservationKind {
    Coverage,
    DeadCode,
    Complexity,
    TodoDensity,
    Weather,
    RefRewrite,
});

impl ObservationKind {
    /// The `kind` column's text. It is stored data, so it never changes.
    pub const fn as_column(self) -> &'static str {
        match self {
            ObservationKind::Coverage => "coverage",
            ObservationKind::DeadCode => "dead_code",
            ObservationKind::Complexity => "complexity",
            ObservationKind::TodoDensity => "todo_density",
            ObservationKind::Weather => "weather",
            ObservationKind::RefRewrite => "ref_rewrite",
        }
    }

    pub fn from_column(text: &str) -> Option<ObservationKind> {
        ObservationKind::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_column() == text)
    }

    /// A light or soil reading, as against weather and a history rewrite.
    pub const fn is_reading(self) -> bool {
        !matches!(self, ObservationKind::Weather | ObservationKind::RefRewrite)
    }
}

/// The kind-specific part of an observation: what truth reads beyond the normalized value. Each
/// variant's doc says what `normalized` holds for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObservationValue {
    /// `normalized`: the line coverage ratio.
    Coverage,
    /// `normalized`: `1 − min(1, unused_items / max(1, exported_items))`.
    DeadCode {
        unused_items: u32,
        exported_items: u32,
    },
    /// `normalized`: `1 − clamp((mean_cyclomatic − 1) / 14)`.
    Complexity,
    /// `normalized`: `1 − clamp(todos_per_kloc / 20)`.
    TodoDensity,
    /// `normalized`: 1 for pass, 0 for fail, absent for unknown.
    Weather { source: WeatherSource },
    /// A non-fast-forward move of the default branch. `normalized` is absent. `rewritten` is
    /// how many commits it took off the default branch's first-parent chain, and `fossils` the
    /// commits of the fossils shown before it that it took off: both `None` and empty when the
    /// record predates them or could not tell.
    RefRewrite {
        reference: String,
        old: CommitId,
        new: CommitId,
        rewritten: Option<u32>,
        fossils: Vec<CommitId>,
    },
}

impl ObservationValue {
    pub const fn kind(&self) -> ObservationKind {
        match self {
            ObservationValue::Coverage => ObservationKind::Coverage,
            ObservationValue::DeadCode { .. } => ObservationKind::DeadCode,
            ObservationValue::Complexity => ObservationKind::Complexity,
            ObservationValue::TodoDensity => ObservationKind::TodoDensity,
            ObservationValue::Weather { .. } => ObservationKind::Weather,
            ObservationValue::RefRewrite { .. } => ObservationKind::RefRewrite,
        }
    }

    /// The weather's source; no other kind has one.
    pub const fn source(&self) -> Option<&WeatherSource> {
        match self {
            ObservationValue::Weather { source } => Some(source),
            _ => None,
        }
    }
}

impl Canonical for ObservationValue {
    fn write(&self, w: &mut Writer) {
        w.leaf(self.kind().name());
        match self {
            ObservationValue::Coverage
            | ObservationValue::Complexity
            | ObservationValue::TodoDensity => {}
            ObservationValue::DeadCode {
                unused_items,
                exported_items,
            } => {
                w.field("unused_items", unused_items);
                w.field("exported_items", exported_items);
            }
            ObservationValue::Weather { source } => w.field("source", source),
            ObservationValue::RefRewrite {
                reference,
                old,
                new,
                rewritten,
                fossils,
            } => {
                w.field("reference", reference);
                w.field("old", old);
                w.field("new", new);
                w.field("rewritten", rewritten);
                w.field("fossils", fossils);
            }
        }
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        let text = r.leaf()?;
        let kind = ObservationKind::from_name(text).ok_or_else(|| r.bad_value(text))?;
        Ok(match kind {
            ObservationKind::Coverage => ObservationValue::Coverage,
            ObservationKind::DeadCode => ObservationValue::DeadCode {
                unused_items: r.field("unused_items")?,
                exported_items: r.field("exported_items")?,
            },
            ObservationKind::Complexity => ObservationValue::Complexity,
            ObservationKind::TodoDensity => ObservationValue::TodoDensity,
            ObservationKind::Weather => ObservationValue::Weather {
                source: r.field("source")?,
            },
            ObservationKind::RefRewrite => ObservationValue::RefRewrite {
                reference: r.field("reference")?,
                old: r.field("old")?,
                new: r.field("new")?,
                rewritten: r.field("rewritten")?,
                fossils: r.field("fossils")?,
            },
        })
    }
}

/// One recorded observation, as replay reads it (I-4). The store's other columns (the raw
/// report, its hash and mtime, the contributors) serve the store and the soil panel, not truth.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    /// The store's row id. It is install-local, so replay never orders by it: same-tick
    /// observations follow their merge key (`recorded_at`, `kind`, `source`, `head_commit`).
    pub id: i64,
    /// Unix seconds, UTC.
    pub recorded_at: i64,
    pub head: ObservationHead,
    /// Health-oriented, 1 best. `None` is unknown.
    pub normalized: Option<Q16>,
    pub confidence: Q16,
    pub value: ObservationValue,
}

canonical_struct!(Observation {
    id,
    recorded_at,
    head,
    normalized,
    confidence,
    value,
});

impl Observation {
    pub fn merge_key(&self) -> MergeKey<'_> {
        merge_key(
            self.recorded_at,
            self.value.kind(),
            self.value.source(),
            &self.head.commit,
        )
    }
}

/// What orders same-tick observations, and what no two of a terrarium's share: `recorded_at`,
/// the `kind` column's text, the `source` column's text and the head commit. The kind compares
/// as its text, as the store's unique key holds it.
pub type MergeKey<'a> = (i64, &'static str, String, &'a str);

pub fn merge_key<'a>(
    recorded_at: i64,
    kind: ObservationKind,
    source: Option<&WeatherSource>,
    head_commit: &'a CommitId,
) -> MergeKey<'a> {
    (
        recorded_at,
        kind.as_column(),
        source_column(source),
        head_commit.as_str(),
    )
}

/// The `source` column's text: the weather's source, and empty for every other kind.
pub fn source_column(source: Option<&WeatherSource>) -> String {
    source.map_or_else(String::new, WeatherSource::to_column)
}

/// Where HEAD was when an observation was recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationHead {
    pub commit: CommitId,
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    pub at_default_tip: bool,
}

canonical_struct!(ObservationHead {
    commit,
    branch,
    at_default_tip
});

/// What the terrarium shows as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PresentationState {
    Thriving,
    Struggling,
    Mature,
    Dormant,
    Dead,
    Recovering,
}

canonical_enum!(PresentationState {
    Thriving,
    Struggling,
    Mature,
    Dormant,
    Dead,
    Recovering,
});

/// Health with hysteresis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum HealthClass {
    Healthy,
    Unhealthy,
}

canonical_enum!(HealthClass { Healthy, Unhealthy });

/// Activity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ActivityClass {
    Inactive,
    Active,
    Quiet,
}

canonical_enum!(ActivityClass {
    Inactive,
    Active,
    Quiet
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::{canonical_dump, from_canonical_dump};

    fn commit(hex_digit: char) -> CommitId {
        CommitId::from_hex(&hex_digit.to_string().repeat(40)).expect("valid id")
    }

    fn round_trips<T>(value: &T)
    where
        T: Canonical + Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        assert_eq!(
            from_canonical_dump::<T>(&canonical_dump(value)).as_ref(),
            Ok(value)
        );
        let bytes = postcard::to_allocvec(value).expect("encodes");
        assert_eq!(&postcard::from_bytes::<T>(&bytes).expect("decodes"), value);
    }

    #[test]
    fn column_texts_are_pinned() {
        // Stored in `observations.kind` and `observations.source`; they never change.
        let kinds: Vec<&str> = ObservationKind::ALL.iter().map(|k| k.as_column()).collect();
        assert_eq!(
            kinds,
            [
                "coverage",
                "dead_code",
                "complexity",
                "todo_density",
                "weather",
                "ref_rewrite"
            ]
        );
        for kind in ObservationKind::ALL {
            assert_eq!(ObservationKind::from_column(kind.as_column()), Some(*kind));
        }
        assert_eq!(ObservationKind::from_column("Coverage"), None);
        let ci = WeatherSource::Ci {
            branch: "feature/soil".to_owned(),
        };
        assert_eq!(ci.to_column(), "ci:feature/soil");
        assert_eq!(WeatherSource::from_column("ci:feature/soil"), Some(ci));
        assert_eq!(WeatherSource::Local.to_column(), "local");
        assert_eq!(
            WeatherSource::from_column("local"),
            Some(WeatherSource::Local)
        );
        for not_a_source in ["", "ci:", "Local", "ci", "remote"] {
            assert_eq!(WeatherSource::from_column(not_a_source), None);
        }
    }

    #[test]
    fn domain_values_round_trip() {
        let known = Reading::Known {
            value: Q16::from_ratio(3, 4),
            stale: true,
            since_tick: Tick::new(40),
        };
        round_trips(&Climate {
            light: known,
            soil: Reading::Unknown,
            rain: known,
            wind: Reading::Unknown,
            weather: WeatherReading {
                state: known,
                source: Some(WeatherSource::Ci {
                    branch: "main".to_owned(),
                }),
            },
        });
        round_trips(&Habitat {
            terrain: 120_000,
            age: Tick::new(9_000),
        });
        round_trips(&CommitFacts {
            id: commit('a'),
            tick: Tick::new(12),
            parents: 2,
            on_default_first_parent: true,
            added: 40,
            removed: 7,
            deleted_dirs: vec![DeletedDir {
                path: "src/old".to_owned(),
                lines: 600,
            }],
            renames: vec![Rename {
                from: "a.rs".to_owned(),
                to: "b.rs".to_owned(),
            }],
            tags: vec![TagFact {
                name: "v1.0.0".to_owned(),
                tick: Tick::new(12),
            }],
        });
        let values = [
            ObservationValue::Coverage,
            ObservationValue::DeadCode {
                unused_items: 3,
                exported_items: 90,
            },
            ObservationValue::Complexity,
            ObservationValue::TodoDensity,
            ObservationValue::Weather {
                source: WeatherSource::Local,
            },
            ObservationValue::RefRewrite {
                reference: "refs/heads/main".to_owned(),
                old: commit('b'),
                new: commit('c'),
                rewritten: None,
                fossils: Vec::new(),
            },
            ObservationValue::RefRewrite {
                reference: "refs/heads/main".to_owned(),
                old: commit('b'),
                new: commit('c'),
                rewritten: Some(3),
                fossils: vec![commit('f'), commit('0')],
            },
        ];
        for (value, id) in values.into_iter().zip(1..) {
            round_trips(&Observation {
                id,
                recorded_at: 1_700_000_000,
                head: ObservationHead {
                    commit: commit('d'),
                    branch: None,
                    at_default_tip: false,
                },
                normalized: Some(Q16::ONE),
                confidence: Q16::HALF,
                value,
            });
        }
        for kind in ObservationKind::ALL {
            round_trips(kind);
        }
    }

    #[test]
    fn an_observation_value_names_its_kind() {
        let dump = canonical_dump(&Observation {
            id: 1,
            recorded_at: 0,
            head: ObservationHead {
                commit: commit('e'),
                branch: Some("main".to_owned()),
                at_default_tip: true,
            },
            normalized: None,
            confidence: Q16::ONE,
            value: ObservationValue::DeadCode {
                unused_items: 1,
                exported_items: 2,
            },
        });
        assert!(dump.ends_with(
            "normalized = None\nconfidence = 1\n\
             value = DeadCode\nvalue.unused_items = 1\nvalue.exported_items = 2\n"
        ));
        assert!(dump.contains("head.branch.some = \"main\"\n"));
    }
}
