//! Observations (I-4): what adapters record about a repository, appended and never changed. The
//! triggers in migration 1 refuse every update and every delete outside the guarded path; this
//! module only ever inserts.

use rusqlite::{Connection, OptionalExtension, Row, Statement, params};
use serde::{Deserialize, Serialize};
use terrarium_truth::domain::{Observation, ObservationHead, WeatherSource, source_column};
use terrarium_truth::{CommitId, ObservationKind, Q16};

use crate::error::is_unique_violation;
use crate::{Invalid, Store, StoreError, TerrariumId, raw};

pub use terrarium_truth::domain::MAX_CONTRIBUTORS;
/// The largest `raw` the store accepts, in bytes. Adapters keep theirs well below it.
pub const MAX_RAW_BYTES: usize = 64 * 1024;

/// An observation's row id. It is local to this install: an import appends its rows after the
/// ones already stored, so replay orders by the merge key instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObservationId(i64);

impl ObservationId {
    pub const fn new(id: i64) -> ObservationId {
        ObservationId(id)
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

/// A file that weighs in a reading, with its weight, shown in the soil panel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contributor {
    pub path: String,
    pub value: i64,
}

/// Every column of an observation but its id and terrarium: what an adapter records, and what
/// the store gives back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationRecord {
    pub kind: ObservationKind,
    pub adapter_version: u32,
    /// Unix seconds, UTC.
    pub recorded_at: i64,
    /// Where HEAD was.
    pub head: ObservationHead,
    /// Set exactly for weather; every other kind stores `''`.
    pub source: Option<WeatherSource>,
    /// Unix seconds; `None` when the reading did not come from a file.
    pub report_mtime: Option<i64>,
    /// SHA-256 of the raw report bytes.
    pub content_hash: [u8; 32],
    /// Kind-specific JSON, at most [`MAX_RAW_BYTES`].
    pub raw: String,
    /// Health-oriented, 1 best; `None` is unknown, never zero (I-3).
    pub normalized: Option<Q16>,
    pub confidence: Q16,
    pub contributors: Vec<Contributor>,
}

impl ObservationRecord {
    /// The rules every stored observation keeps, checked before append and import alike.
    pub fn check(&self) -> Result<(), Invalid> {
        match (self.kind, &self.source) {
            (ObservationKind::Weather, None) => return Err(Invalid::WeatherWithoutSource),
            (ObservationKind::Weather, Some(source)) => {
                if WeatherSource::from_column(&source.to_column()).as_ref() != Some(source) {
                    return Err(Invalid::BadSource);
                }
            }
            (_, Some(_)) => return Err(Invalid::SourceWithoutWeather),
            (_, None) => {}
        }
        let unit = |q: Q16| (Q16::ZERO..=Q16::ONE).contains(&q);
        if !self.normalized.is_none_or(unit) {
            return Err(Invalid::OutOfRange("normalized"));
        }
        if !unit(self.confidence) {
            return Err(Invalid::OutOfRange("confidence"));
        }
        if self.contributors.len() > MAX_CONTRIBUTORS {
            return Err(Invalid::TooManyContributors(self.contributors.len()));
        }
        if self.raw.len() > MAX_RAW_BYTES {
            return Err(Invalid::RawTooLarge(self.raw.len()));
        }
        let json = raw::parse(self.kind, &self.raw)?;
        if self.kind == ObservationKind::RefRewrite && self.normalized.is_some() {
            return Err(Invalid::NormalizedOnRefRewrite);
        }
        raw::from_json(self.kind, self.source.as_ref(), json).map(|_| ())
    }

    /// The `source` column's text: `''` for every kind but weather.
    pub(crate) fn source_column(&self) -> String {
        source_column(self.source.as_ref())
    }
}

/// An observation as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredObservation {
    pub id: ObservationId,
    pub terrarium: TerrariumId,
    pub record: ObservationRecord,
}

impl StoredObservation {
    /// The observation as replay reads it (I-4): its kind-specific value comes from `raw` and,
    /// for weather, the source column. Every stored row converts, because
    /// [`ObservationRecord::check`] refused any that would not; one that still fails is
    /// [`StoreError::Corrupt`].
    pub fn to_observation(&self) -> Result<Observation, StoreError> {
        let record = &self.record;
        let value = raw::value(record.kind, record.source.as_ref(), &record.raw)
            .map_err(|_| StoreError::Corrupt("raw"))?;
        Ok(Observation {
            id: self.id.get(),
            recorded_at: record.recorded_at,
            head: record.head.clone(),
            normalized: record.normalized,
            confidence: record.confidence,
            value,
        })
    }
}

/// The columns in schema order, as [`Row`]s are read by [`read_row`].
pub(crate) const COLUMNS: &str = "id, terrarium_id, kind, adapter_version, recorded_at, \
     head_commit, head_branch, at_default_tip, source, report_mtime, content_hash, raw, \
     normalized, confidence, contributors";

/// A row's values as SQLite holds them, before they are decoded into truth's types.
pub(crate) struct RawRow {
    id: i64,
    terrarium_id: i64,
    kind: String,
    adapter_version: i64,
    recorded_at: i64,
    head_commit: String,
    head_branch: Option<String>,
    at_default_tip: bool,
    source: String,
    report_mtime: Option<i64>,
    content_hash: Vec<u8>,
    raw: String,
    normalized: Option<i64>,
    confidence: i64,
    contributors: String,
}

pub(crate) fn read_row(row: &Row<'_>) -> rusqlite::Result<RawRow> {
    Ok(RawRow {
        id: row.get(0)?,
        terrarium_id: row.get(1)?,
        kind: row.get(2)?,
        adapter_version: row.get(3)?,
        recorded_at: row.get(4)?,
        head_commit: row.get(5)?,
        head_branch: row.get(6)?,
        at_default_tip: row.get(7)?,
        source: row.get(8)?,
        report_mtime: row.get(9)?,
        content_hash: row.get(10)?,
        raw: row.get(11)?,
        normalized: row.get(12)?,
        confidence: row.get(13)?,
        contributors: row.get(14)?,
    })
}

/// The inverse of [`ObservationRecord::source_column`]: `''` is no source, anything else must be
/// a weather source. The error names the column, for a text that is neither.
pub(crate) fn source_from_column(text: &str) -> Result<Option<WeatherSource>, &'static str> {
    if text.is_empty() {
        Ok(None)
    } else {
        WeatherSource::from_column(text).map(Some).ok_or("source")
    }
}

fn q16(value: i64, column: &'static str) -> Result<Q16, StoreError> {
    i32::try_from(value)
        .map(Q16::from_raw)
        .map_err(|_| StoreError::Corrupt(column))
}

impl RawRow {
    pub(crate) fn decode(self) -> Result<StoredObservation, StoreError> {
        let record = ObservationRecord {
            kind: ObservationKind::from_column(&self.kind).ok_or(StoreError::Corrupt("kind"))?,
            adapter_version: u32::try_from(self.adapter_version)
                .map_err(|_| StoreError::Corrupt("adapter_version"))?,
            recorded_at: self.recorded_at,
            head: ObservationHead {
                commit: CommitId::from_hex(&self.head_commit)
                    .ok_or(StoreError::Corrupt("head_commit"))?,
                branch: self.head_branch,
                at_default_tip: self.at_default_tip,
            },
            source: source_from_column(&self.source).map_err(StoreError::Corrupt)?,
            report_mtime: self.report_mtime,
            content_hash: self
                .content_hash
                .try_into()
                .map_err(|_| StoreError::Corrupt("content_hash"))?,
            raw: self.raw,
            normalized: self.normalized.map(|n| q16(n, "normalized")).transpose()?,
            confidence: q16(self.confidence, "confidence")?,
            contributors: serde_json::from_str(&self.contributors)
                .map_err(|_| StoreError::Corrupt("contributors"))?,
        };
        Ok(StoredObservation {
            id: ObservationId(self.id),
            terrarium: TerrariumId::new(self.terrarium_id),
            record,
        })
    }
}

/// A record that passed [`ObservationRecord::check`]: the only kind [`Inserter`] writes.
pub(crate) struct Checked<'r>(&'r ObservationRecord);

impl ObservationRecord {
    pub(crate) fn checked(&self) -> Result<Checked<'_>, Invalid> {
        self.check()?;
        Ok(Checked(self))
    }
}

/// The one way observations are written, by append and by import, prepared once so an import
/// does not compile it per line. Never `INSERT OR REPLACE`, which would replace a row (and
/// which the triggers refuse).
pub(crate) struct Inserter<'c>(Statement<'c>);

impl<'c> Inserter<'c> {
    pub(crate) fn new(conn: &'c Connection) -> rusqlite::Result<Inserter<'c>> {
        conn.prepare(
            "INSERT INTO observations (terrarium_id, kind, adapter_version, recorded_at, \
             head_commit, head_branch, at_default_tip, source, report_mtime, content_hash, raw, \
             normalized, confidence, contributors) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )
        .map(Inserter)
    }

    pub(crate) fn insert(
        &mut self,
        terrarium: TerrariumId,
        Checked(record): Checked<'_>,
    ) -> Result<ObservationId, StoreError> {
        let contributors = serde_json::to_string(&record.contributors)?;
        let inserted = self.0.insert(params![
            terrarium.get(),
            record.kind.as_column(),
            record.adapter_version,
            record.recorded_at,
            record.head.commit.as_str(),
            record.head.branch,
            record.head.at_default_tip,
            record.source_column(),
            record.report_mtime,
            record.content_hash,
            record.raw,
            record.normalized.map(Q16::raw),
            record.confidence.raw(),
            contributors,
        ]);
        match inserted {
            Ok(id) => Ok(ObservationId(id)),
            Err(err) if is_unique_violation(&err) => Err(StoreError::AlreadyRecorded),
            Err(err) => Err(err.into()),
        }
    }
}

/// Finds the stored observation with a record's merge key (`kind`, `source`, `head_commit`,
/// `recorded_at`), prepared once for an import's every line.
pub(crate) struct KeyLookup<'c>(Statement<'c>);

impl<'c> KeyLookup<'c> {
    pub(crate) fn new(conn: &'c Connection) -> rusqlite::Result<KeyLookup<'c>> {
        conn.prepare(&format!(
            "SELECT {COLUMNS} FROM observations WHERE terrarium_id = ?1 AND kind = ?2 \
             AND source = ?3 AND head_commit = ?4 AND recorded_at = ?5"
        ))
        .map(KeyLookup)
    }

    pub(crate) fn find(
        &mut self,
        terrarium: TerrariumId,
        record: &ObservationRecord,
    ) -> Result<Option<StoredObservation>, StoreError> {
        self.0
            .query_row(
                params![
                    terrarium.get(),
                    record.kind.as_column(),
                    record.source_column(),
                    record.head.commit.as_str(),
                    record.recorded_at,
                ],
                read_row,
            )
            .optional()?
            .map(RawRow::decode)
            .transpose()
    }
}

impl Store {
    /// Appends one observation. One with the same merge key is [`StoreError::AlreadyRecorded`]
    /// and changes nothing.
    pub fn append_observation(
        &mut self,
        terrarium: TerrariumId,
        record: &ObservationRecord,
    ) -> Result<ObservationId, StoreError> {
        let checked = record.checked()?;
        Inserter::new(&self.conn)?.insert(terrarium, checked)
    }

    /// A terrarium's observations in id order, of one kind or all. The engine loads them once,
    /// at connect.
    pub fn observations(
        &self,
        terrarium: TerrariumId,
        kind: Option<ObservationKind>,
    ) -> Result<Vec<StoredObservation>, StoreError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM observations WHERE terrarium_id = ?1 \
             AND (?2 IS NULL OR kind = ?2) ORDER BY id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![terrarium.get(), kind.map(ObservationKind::as_column)],
            read_row,
        )?;
        rows.map(|row| row?.decode()).collect()
    }

    /// The files that weighed in one of a terrarium's observations, heaviest first, as the
    /// soil panel lists them; none for a row of another terrarium or no row at all.
    pub fn contributors(
        &self,
        terrarium: TerrariumId,
        id: ObservationId,
    ) -> Result<Vec<Contributor>, StoreError> {
        let stored: Option<String> = self
            .conn
            .query_row(
                "SELECT contributors FROM observations WHERE terrarium_id = ?1 AND id = ?2",
                params![terrarium.get(), id.get()],
                |row| row.get(0),
            )
            .optional()?;
        stored.map_or(Ok(Vec::new()), |json| {
            serde_json::from_str(&json).map_err(|_| StoreError::Corrupt("contributors"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrarium_truth::domain::ObservationValue;

    use crate::test_support::{Fixture, commit, new_terrarium, record, weather};

    fn stored_text(fx: &Fixture, column: &str, id: ObservationId) -> String {
        fx.store
            .conn
            .query_row(
                &format!("SELECT {column} FROM observations WHERE id = ?1"),
                [id.get()],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn every_kind_round_trips_with_every_column() {
        let mut fx = Fixture::new();
        let id = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        let mut records: Vec<ObservationRecord> = ObservationKind::ALL
            .iter()
            .zip(100..)
            .map(|(&kind, at)| record(kind, at))
            .collect();
        records[0].head.branch = None;
        records[0].report_mtime = None;
        records[0].normalized = None;
        records[1].contributors.clear();
        let mut ids = Vec::new();
        for r in &records {
            ids.push(fx.store.append_observation(id, r).unwrap());
        }
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
        let stored = fx.store.observations(id, None).unwrap();
        let expected: Vec<StoredObservation> = ids
            .iter()
            .zip(records)
            .map(|(&id_, record)| StoredObservation {
                id: id_,
                terrarium: id,
                record,
            })
            .collect();
        assert_eq!(stored, expected);
        assert_eq!(stored_text(&fx, "source", ids[0]), "");
        assert_eq!(stored_text(&fx, "kind", ids[1]), "dead_code");
    }

    #[test]
    fn a_row_s_contributors_are_read_back_for_its_own_terrarium_only() {
        let mut fx = Fixture::new();
        let ours = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        let theirs = fx.store.create_terrarium(&new_terrarium(2)).unwrap();
        let with = record(ObservationKind::Coverage, 100);
        assert!(!with.contributors.is_empty());
        let mut without = record(ObservationKind::Coverage, 101);
        without.contributors.clear();
        let a = fx.store.append_observation(ours, &with).unwrap();
        let b = fx.store.append_observation(ours, &without).unwrap();
        assert_eq!(fx.store.contributors(ours, a).unwrap(), with.contributors);
        assert_eq!(fx.store.contributors(ours, b).unwrap(), []);
        assert_eq!(fx.store.contributors(theirs, a).unwrap(), []);
        let missing = ObservationId::new(b.get() + 100);
        assert_eq!(fx.store.contributors(ours, missing).unwrap(), []);
    }

    #[test]
    fn every_stored_kind_converts_to_what_replay_reads() {
        let mut fx = Fixture::new();
        let id = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        for (&kind, at) in ObservationKind::ALL.iter().zip(100..) {
            fx.store.append_observation(id, &record(kind, at)).unwrap();
        }
        let values: Vec<ObservationValue> = fx
            .store
            .observations(id, None)
            .unwrap()
            .iter()
            .map(|stored| {
                let observation = stored.to_observation().unwrap();
                assert_eq!(observation.id, stored.id.get());
                assert_eq!(observation.recorded_at, stored.record.recorded_at);
                assert_eq!(observation.head, stored.record.head);
                assert_eq!(observation.normalized, stored.record.normalized);
                assert_eq!(observation.confidence, stored.record.confidence);
                observation.value
            })
            .collect();
        assert_eq!(
            values,
            [
                ObservationValue::Coverage,
                ObservationValue::DeadCode {
                    unused_items: 4,
                    exported_items: 80
                },
                ObservationValue::Complexity,
                ObservationValue::TodoDensity,
                ObservationValue::Weather {
                    source: WeatherSource::Ci {
                        branch: "main".into()
                    }
                },
                ObservationValue::RefRewrite {
                    reference: "refs/heads/main".into(),
                    old: commit('b'),
                    new: commit('c'),
                    rewritten: None,
                    fossils: Vec::new(),
                    left: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn queries_filter_by_terrarium_and_kind() {
        let mut fx = Fixture::new();
        let one = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        let two = fx.store.create_terrarium(&new_terrarium(2)).unwrap();
        let a = fx
            .store
            .append_observation(one, &record(ObservationKind::Coverage, 100))
            .unwrap();
        let b = fx.store.append_observation(one, &weather(200)).unwrap();
        let c = fx
            .store
            .append_observation(one, &record(ObservationKind::Coverage, 300))
            .unwrap();
        fx.store
            .append_observation(two, &record(ObservationKind::Coverage, 100))
            .unwrap();
        let ids = |kind| -> Vec<ObservationId> {
            fx.store
                .observations(one, kind)
                .unwrap()
                .iter()
                .map(|o| o.id)
                .collect()
        };
        assert_eq!(ids(None), [a, b, c]);
        assert_eq!(ids(Some(ObservationKind::Coverage)), [a, c]);
        assert_eq!(ids(Some(ObservationKind::Weather)), [b]);
    }

    #[test]
    fn the_same_merge_key_is_recorded_once() {
        let mut fx = Fixture::new();
        let id = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        let first = record(ObservationKind::Coverage, 100);
        fx.store.append_observation(id, &first).unwrap();
        let mut again = first.clone();
        again.raw = r#"{"lines":3}"#.to_owned();
        assert!(matches!(
            fx.store.append_observation(id, &again),
            Err(StoreError::AlreadyRecorded)
        ));
        // Another source, commit or time is another observation.
        let mut later = first.clone();
        later.recorded_at += 1;
        fx.store.append_observation(id, &later).unwrap();
        let mut other_commit = first;
        other_commit.head.commit = commit('e');
        fx.store.append_observation(id, &other_commit).unwrap();
        let w = weather(100);
        fx.store.append_observation(id, &w).unwrap();
        let mut local = w;
        local.source = Some(WeatherSource::Local);
        fx.store.append_observation(id, &local).unwrap();
        assert_eq!(fx.store.observations(id, None).unwrap().len(), 5);
    }

    #[test]
    fn an_observation_needs_a_known_terrarium() {
        let mut fx = Fixture::new();
        let err = fx
            .store
            .append_observation(TerrariumId::new(4), &record(ObservationKind::Coverage, 1))
            .unwrap_err();
        assert!(err.to_string().contains("FOREIGN KEY"), "{err}");
    }

    #[test]
    fn invalid_observations_are_refused_and_nothing_is_written() {
        let mut fx = Fixture::new();
        let id = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        let base = record(ObservationKind::Coverage, 100);
        let with = |change: &dyn Fn(&mut ObservationRecord)| {
            let mut r = base.clone();
            change(&mut r);
            r
        };
        let cases = [
            (
                with(&|r| r.kind = ObservationKind::Weather),
                Invalid::WeatherWithoutSource,
            ),
            (
                with(&|r| r.source = Some(WeatherSource::Local)),
                Invalid::SourceWithoutWeather,
            ),
            (
                {
                    let mut w = weather(100);
                    w.source = Some(WeatherSource::Ci {
                        branch: String::new(),
                    });
                    w
                },
                Invalid::BadSource,
            ),
            (
                with(&|r| r.normalized = Some(Q16::from_raw(65_537))),
                Invalid::OutOfRange("normalized"),
            ),
            (
                with(&|r| r.normalized = Some(Q16::from_raw(-1))),
                Invalid::OutOfRange("normalized"),
            ),
            (
                with(&|r| r.confidence = Q16::from_raw(65_537)),
                Invalid::OutOfRange("confidence"),
            ),
            (
                with(&|r| {
                    r.contributors = (0..21)
                        .map(|i| Contributor {
                            path: format!("src/f{i}.rs"),
                            value: i,
                        })
                        .collect();
                }),
                Invalid::TooManyContributors(21),
            ),
            (
                with(&|r| r.raw = "{not json".to_owned()),
                Invalid::RawNotJson,
            ),
            (with(&|r| r.raw = String::new()), Invalid::RawNotJson),
            (
                with(&|r| r.raw = format!("\"{}\"", "x".repeat(MAX_RAW_BYTES))),
                Invalid::RawTooLarge(MAX_RAW_BYTES + 2),
            ),
            (
                with(&|r| r.kind = ObservationKind::DeadCode),
                Invalid::RawShape(ObservationKind::DeadCode),
            ),
            (
                with(&|r| {
                    r.kind = ObservationKind::RefRewrite;
                    r.normalized = None;
                }),
                Invalid::RawShape(ObservationKind::RefRewrite),
            ),
            (
                {
                    let mut rewrite = record(ObservationKind::RefRewrite, 100);
                    rewrite.normalized = Some(Q16::ZERO);
                    rewrite
                },
                Invalid::NormalizedOnRefRewrite,
            ),
        ];
        for (bad, why) in cases {
            assert_eq!(bad.check(), Err(why.clone()));
            assert!(
                matches!(fx.store.append_observation(id, &bad), Err(StoreError::Invalid(w)) if w == why)
            );
        }
        assert_eq!(fx.store.observations(id, None).unwrap(), vec![]);
        let mut edge = base;
        edge.normalized = Some(Q16::ONE);
        edge.confidence = Q16::ZERO;
        edge.contributors.clear();
        assert_eq!(edge.check(), Ok(()));
    }

    /// One stored observation, and the raw SQL that tries to change it.
    fn one_row() -> (Fixture, ObservationId) {
        let mut fx = Fixture::new();
        let terrarium = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        let id = fx
            .store
            .append_observation(terrarium, &record(ObservationKind::Coverage, 100))
            .unwrap();
        (fx, id)
    }

    fn assert_refused(fx: &Fixture, id: ObservationId, sql: &str) {
        let before = fx.store.observations(TerrariumId::new(1), None).unwrap();
        let err = fx.store.conn.execute(sql, [id.get()]).unwrap_err();
        assert!(
            err.to_string().contains("observations are append-only"),
            "{sql}: {err}"
        );
        assert_eq!(
            fx.store.observations(TerrariumId::new(1), None).unwrap(),
            before,
            "{sql}"
        );
    }

    /// Every column of row `?1` again, as the values for a new row: the same merge key, with
    /// the id left for SQLite to assign and the raw report changed.
    const SAME_KEY: &str = "SELECT NULL, terrarium_id, kind, adapter_version, recorded_at, \
         head_commit, head_branch, at_default_tip, source, report_mtime, content_hash, \
         '{\"replaced\":true}', normalized, confidence, contributors \
         FROM observations WHERE id = ?1";

    #[test]
    fn raw_is_json_by_its_syntax_whatever_its_numbers_hold() {
        // Past f64's range: valid JSON, which a parser that keeps nothing accepts.
        let big = r#"{"lines_found":1e400}"#;
        let mut coverage = record(ObservationKind::Coverage, 100);
        coverage.raw = big.to_owned();
        assert_eq!(coverage.check(), Ok(()));
        // A kind whose value is in `raw` cannot hold it, so its shape is wrong, not its JSON.
        let mut dead_code = record(ObservationKind::DeadCode, 100);
        dead_code.raw = big.to_owned();
        assert_eq!(
            dead_code.check(),
            Err(Invalid::RawShape(ObservationKind::DeadCode))
        );
    }

    #[test]
    fn an_update_is_refused() {
        let (fx, id) = one_row();
        assert_refused(&fx, id, "UPDATE observations SET raw = '{}' WHERE id = ?1");
        assert_refused(&fx, id, "UPDATE observations SET id = id WHERE id = ?1");
    }

    #[test]
    fn a_delete_is_refused() {
        let (fx, id) = one_row();
        assert_refused(&fx, id, "DELETE FROM observations WHERE id = ?1");
    }

    #[test]
    fn a_replace_is_refused() {
        // `INSERT OR REPLACE` deletes the row it collides with; with `recursive_triggers` ON
        // that delete fires the trigger. An upsert's update fires the update trigger.
        let (fx, id) = one_row();
        let insert = format!("INSERT INTO observations ({COLUMNS}) ");
        assert_refused(
            &fx,
            id,
            &format!(
                "{}{SAME_KEY}",
                insert.replace("INSERT", "INSERT OR REPLACE")
            ),
        );
        assert_refused(
            &fx,
            id,
            &format!("{}{SAME_KEY}", insert.replace("INSERT", "REPLACE")),
        );
        assert_refused(
            &fx,
            id,
            &format!("{insert}{SAME_KEY} ON CONFLICT DO UPDATE SET raw = excluded.raw"),
        );
        assert_refused(
            &fx,
            id,
            &format!(
                "{} SELECT * FROM observations WHERE id = ?1",
                insert.replace("INSERT", "INSERT OR REPLACE")
            ),
        );
    }
}
