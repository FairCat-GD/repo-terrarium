//! Observation export and import as JSON Lines: how a person moves a terrarium's history to
//! another machine. The format is described in this crate's README.

use std::fmt;
use std::io::{BufRead, Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use terrarium_truth::canon::{from_hex, to_hex};
use terrarium_truth::domain::{ObservationHead, WeatherSource};
use terrarium_truth::{CommitId, ObservationKind, Q16, RepoSeed, SIM_VERSION, SimVersion};

use crate::observations::{COLUMNS, find_by_key, insert, read_row, source_from_column};
use crate::{
    Contributor, Invalid, ObservationRecord, Store, StoreError, StoredObservation, TerrariumId,
};

/// The header's `format`.
pub const FORMAT: &str = "repo-terrarium-observations";
/// The header's `format_version`: the only one this version reads and writes.
pub const FORMAT_VERSION: u32 = 1;
/// The longest line an import reads, in bytes. The largest `raw`, escaped, and twenty
/// contributors fit with room to spare.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    format: String,
    format_version: u32,
    seed: String,
    sim_version: u32,
}

/// One observation per line: every column in schema order but `terrarium_id`, which the
/// header's seed stands for. BLOBs are lowercase hex; `raw` is its stored text.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Line {
    id: i64,
    kind: String,
    adapter_version: u32,
    recorded_at: i64,
    head_commit: String,
    head_branch: Option<String>,
    at_default_tip: bool,
    source: String,
    report_mtime: Option<i64>,
    content_hash: String,
    raw: String,
    normalized: Option<i32>,
    confidence: i32,
    contributors: Vec<Contributor>,
}

impl Line {
    fn from_stored(stored: &StoredObservation) -> Line {
        let r = &stored.record;
        Line {
            id: stored.id.get(),
            kind: r.kind.as_column().to_owned(),
            adapter_version: r.adapter_version,
            recorded_at: r.recorded_at,
            head_commit: r.head.commit.as_str().to_owned(),
            head_branch: r.head.branch.clone(),
            at_default_tip: r.head.at_default_tip,
            source: r.source_column(),
            report_mtime: r.report_mtime,
            content_hash: to_hex(&r.content_hash),
            raw: r.raw.clone(),
            normalized: r.normalized.map(Q16::raw),
            confidence: r.confidence.raw(),
            contributors: r.contributors.clone(),
        }
    }

    /// The line as a record to merge. Its `id` belongs to the exporting install and is not kept.
    fn into_record(self) -> Result<ObservationRecord, LineProblem> {
        let record = ObservationRecord {
            kind: ObservationKind::from_column(&self.kind).ok_or(LineProblem::BadColumn("kind"))?,
            adapter_version: self.adapter_version,
            recorded_at: self.recorded_at,
            head: ObservationHead {
                commit: CommitId::from_hex(&self.head_commit)
                    .ok_or(LineProblem::BadColumn("head_commit"))?,
                branch: self.head_branch,
                at_default_tip: self.at_default_tip,
            },
            source: source_from_column(&self.source).map_err(LineProblem::BadColumn)?,
            report_mtime: self.report_mtime,
            content_hash: from_hex(&self.content_hash)
                .and_then(|bytes| bytes.try_into().ok())
                .ok_or(LineProblem::BadColumn("content_hash"))?,
            raw: self.raw,
            normalized: self.normalized.map(Q16::from_raw),
            confidence: Q16::from_raw(self.confidence),
            contributors: self.contributors,
        };
        record.check().map_err(LineProblem::Invalid)?;
        Ok(record)
    }
}

/// What an import did. Nothing in the file is lost silently: every line is inserted, a
/// duplicate, or a conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportReport {
    /// The sim version the file was exported under. Observations do not depend on it, so any
    /// version imports.
    pub sim_version: SimVersion,
    pub inserted: u64,
    /// Lines equal to a stored observation in every column.
    pub duplicates: u64,
    /// Lines whose merge key is stored with other values; the stored row is kept.
    pub conflicts: Vec<Conflict>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The line in the file, the header being line 1.
    pub line: u64,
    pub kind: ObservationKind,
    pub source: Option<WeatherSource>,
    pub head_commit: CommitId,
    pub recorded_at: i64,
    /// The columns whose values differ, in schema order.
    pub columns: Vec<&'static str>,
}

/// Why an import was refused. A refused import writes nothing.
#[derive(Debug)]
pub enum ImportError {
    Read(std::io::Error),
    /// The first line is not this format's header.
    NotAnExport,
    /// A `format_version` this version does not read.
    FormatVersion(u64),
    /// The file holds another terrarium's observations.
    SeedMismatch {
        file: RepoSeed,
        terrarium: RepoSeed,
    },
    Line {
        number: u64,
        problem: LineProblem,
    },
    Store(StoreError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineProblem {
    /// Longer than [`MAX_LINE_BYTES`].
    TooLong,
    /// Not UTF-8, not JSON, or not this format's fields, with the parser's message.
    Malformed(String),
    /// A column whose text does not decode.
    BadColumn(&'static str),
    /// An observation the store never holds.
    Invalid(Invalid),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::Read(err) => write!(f, "reading the file: {err}"),
            ImportError::NotAnExport => f.write_str("not an observations export"),
            ImportError::FormatVersion(v) => write!(f, "format version {v} is not readable"),
            ImportError::SeedMismatch { file, terrarium } => write!(
                f,
                "the file's seed {} is not this terrarium's {}",
                file.to_hex(),
                terrarium.to_hex()
            ),
            ImportError::Line { number, problem } => write!(f, "line {number}: {problem}"),
            ImportError::Store(err) => err.fmt(f),
        }
    }
}

impl fmt::Display for LineProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LineProblem::TooLong => write!(f, "longer than {MAX_LINE_BYTES} bytes"),
            LineProblem::Malformed(why) => f.write_str(why),
            LineProblem::BadColumn(column) => write!(f, "{column} does not decode"),
            LineProblem::Invalid(why) => why.fmt(f),
        }
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ImportError::Read(err) => Some(err),
            ImportError::Store(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ImportError {
    fn from(err: std::io::Error) -> ImportError {
        ImportError::Read(err)
    }
}

impl From<StoreError> for ImportError {
    fn from(err: StoreError) -> ImportError {
        ImportError::Store(err)
    }
}

impl From<rusqlite::Error> for ImportError {
    fn from(err: rusqlite::Error) -> ImportError {
        ImportError::Store(err.into())
    }
}

enum Next {
    End,
    Line(String),
    Problem(LineProblem),
}

/// The next line without its ending, read at most [`MAX_LINE_BYTES`] at a time so an endless
/// line cannot exhaust memory.
fn next_line(input: &mut impl BufRead) -> std::io::Result<Next> {
    let mut buf = Vec::new();
    let limit = u64::try_from(MAX_LINE_BYTES + 1).unwrap_or(u64::MAX);
    input.by_ref().take(limit).read_until(b'\n', &mut buf)?;
    if buf.is_empty() {
        return Ok(Next::End);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    } else if buf.len() > MAX_LINE_BYTES {
        return Ok(Next::Problem(LineProblem::TooLong));
    }
    Ok(match String::from_utf8(buf) {
        Ok(text) => Next::Line(text),
        Err(_) => Next::Problem(LineProblem::Malformed("not UTF-8".to_owned())),
    })
}

/// Checks the header against the terrarium being imported into and returns its sim version.
fn read_header(text: &str, seed: &RepoSeed) -> Result<SimVersion, ImportError> {
    let value: Value = serde_json::from_str(text).map_err(|_| ImportError::NotAnExport)?;
    if value.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(ImportError::NotAnExport);
    }
    match value.get("format_version").and_then(Value::as_u64) {
        Some(v) if v == u64::from(FORMAT_VERSION) => {}
        Some(v) => return Err(ImportError::FormatVersion(v)),
        None => return Err(ImportError::NotAnExport),
    }
    let header_problem = |problem| ImportError::Line { number: 1, problem };
    let header: Header = serde_json::from_value(value)
        .map_err(|err| header_problem(LineProblem::Malformed(err.to_string())))?;
    let file = RepoSeed::from_hex(&header.seed)
        .ok_or_else(|| header_problem(LineProblem::BadColumn("seed")))?;
    if file != *seed {
        return Err(ImportError::SeedMismatch {
            file,
            terrarium: *seed,
        });
    }
    Ok(SimVersion(header.sim_version))
}

fn parse_line(text: &str) -> Result<ObservationRecord, LineProblem> {
    serde_json::from_str::<Line>(text)
        .map_err(|err| LineProblem::Malformed(err.to_string()))?
        .into_record()
}

/// The columns, in schema order, whose values differ between two records with the same merge
/// key. Destructured without `..`, so a new column cannot be left out.
fn differing(stored: &ObservationRecord, file: &ObservationRecord) -> Vec<&'static str> {
    let ObservationRecord {
        kind: _,
        adapter_version,
        recorded_at: _,
        head:
            ObservationHead {
                commit: _,
                branch,
                at_default_tip,
            },
        source: _,
        report_mtime,
        content_hash,
        raw,
        normalized,
        confidence,
        contributors,
    } = stored;
    [
        (*adapter_version == file.adapter_version, "adapter_version"),
        (*branch == file.head.branch, "head_branch"),
        (
            *at_default_tip == file.head.at_default_tip,
            "at_default_tip",
        ),
        (*report_mtime == file.report_mtime, "report_mtime"),
        (*content_hash == file.content_hash, "content_hash"),
        (*raw == file.raw, "raw"),
        (*normalized == file.normalized, "normalized"),
        (*confidence == file.confidence, "confidence"),
        (*contributors == file.contributors, "contributors"),
    ]
    .into_iter()
    .filter_map(|(same, column)| (!same).then_some(column))
    .collect()
}

fn write_json_line(out: &mut impl Write, value: &impl Serialize) -> Result<(), StoreError> {
    serde_json::to_writer(&mut *out, value)?;
    out.write_all(b"\n")?;
    Ok(())
}

impl Store {
    /// Writes a terrarium's observations as JSON Lines, a header and then one line per
    /// observation in id order, streaming rows as it reads them. Returns how many it wrote.
    pub fn export_observations(
        &self,
        terrarium: TerrariumId,
        mut out: impl Write,
    ) -> Result<u64, StoreError> {
        let target = self
            .terrarium(terrarium)?
            .ok_or(StoreError::NoSuchTerrarium(terrarium))?;
        let header = Header {
            format: FORMAT.to_owned(),
            format_version: FORMAT_VERSION,
            seed: target.seed.to_hex(),
            sim_version: SIM_VERSION.0,
        };
        write_json_line(&mut out, &header)?;
        let sql = format!("SELECT {COLUMNS} FROM observations WHERE terrarium_id = ?1 ORDER BY id");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query([terrarium.get()])?;
        let mut written = 0;
        while let Some(row) = rows.next()? {
            let stored = read_row(row)?.decode()?;
            write_json_line(&mut out, &Line::from_stored(&stored))?;
            written += 1;
        }
        out.flush()?;
        Ok(written)
    }

    /// Merges an export into a terrarium, in one transaction. It refuses, writing nothing, a
    /// file that is not an export, a format version it does not read, another terrarium's seed,
    /// or any line that does not decode into a valid observation. Otherwise each line is merged
    /// by (`kind`, `source`, `head_commit`, `recorded_at`): a new one is inserted, an identical
    /// one skipped, and a different one reported as a conflict, keeping the stored row.
    pub fn import_observations(
        &mut self,
        terrarium: TerrariumId,
        mut input: impl BufRead,
    ) -> Result<ImportReport, ImportError> {
        let target = self
            .terrarium(terrarium)?
            .ok_or(StoreError::NoSuchTerrarium(terrarium))?;
        let sim_version = match next_line(&mut input)? {
            Next::End => return Err(ImportError::NotAnExport),
            Next::Problem(problem) => return Err(ImportError::Line { number: 1, problem }),
            Next::Line(text) => read_header(&text, &target.seed)?,
        };
        let mut report = ImportReport {
            sim_version,
            inserted: 0,
            duplicates: 0,
            conflicts: Vec::new(),
        };
        let tx = self.conn.transaction()?;
        let mut number = 1;
        loop {
            number += 1;
            let text = match next_line(&mut input)? {
                Next::End => break,
                Next::Problem(problem) => return Err(ImportError::Line { number, problem }),
                Next::Line(text) => text,
            };
            let record =
                parse_line(&text).map_err(|problem| ImportError::Line { number, problem })?;
            match find_by_key(&tx, terrarium, &record)? {
                None => {
                    insert(&tx, terrarium, &record)?;
                    report.inserted += 1;
                }
                Some(stored) => {
                    let columns = differing(&stored.record, &record);
                    if columns.is_empty() {
                        report.duplicates += 1;
                    } else {
                        report.conflicts.push(Conflict {
                            line: number,
                            kind: record.kind,
                            source: record.source,
                            head_commit: record.head.commit,
                            recorded_at: record.recorded_at,
                            columns,
                        });
                    }
                }
            }
        }
        tx.commit()?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;
    use crate::MAX_CONTRIBUTORS;
    use crate::test_support::{
        Fixture, commit, new_terrarium, proptest_config, record, seed, weather,
    };

    fn export(fx: &Fixture, id: TerrariumId) -> Vec<u8> {
        let mut out = Vec::new();
        fx.store.export_observations(id, &mut out).unwrap();
        out
    }

    /// A store with one terrarium holding `records`.
    fn holding(records: &[ObservationRecord]) -> (Fixture, TerrariumId) {
        let mut fx = Fixture::new();
        let id = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        for r in records {
            fx.store.append_observation(id, r).unwrap();
        }
        (fx, id)
    }

    fn sample() -> Vec<ObservationRecord> {
        let mut records: Vec<ObservationRecord> = ObservationKind::ALL
            .iter()
            .zip(100..)
            .map(|(&kind, at)| record(kind, at))
            .collect();
        records[2].head.branch = None;
        records[2].report_mtime = None;
        records[2].normalized = None;
        records[3].contributors.clear();
        records.push(weather(900));
        records
    }

    fn count(fx: &Fixture) -> usize {
        fx.store
            .observations(TerrariumId::new(1), None, None)
            .unwrap()
            .len()
    }

    #[test]
    fn the_format_is_pinned() {
        // Golden fixtures and other installs read this text; it changes only with a new
        // `format_version`.
        let (fx, id) = holding(&[ObservationRecord {
            kind: ObservationKind::Weather,
            adapter_version: 2,
            recorded_at: 1_700_000_200,
            head: ObservationHead {
                commit: commit('d'),
                branch: Some("main".to_owned()),
                at_default_tip: true,
            },
            source: Some(WeatherSource::Ci {
                branch: "main".to_owned(),
            }),
            report_mtime: None,
            content_hash: [0xab; 32],
            raw: r#"{"state":"pass"}"#.to_owned(),
            normalized: Some(Q16::ONE),
            confidence: Q16::HALF,
            contributors: vec![Contributor {
                path: "tests/pond.rs".to_owned(),
                value: 1,
            }],
        }]);
        let text = String::from_utf8(export(&fx, id)).unwrap();
        let expected = format!(
            "{{\"format\":\"repo-terrarium-observations\",\"format_version\":1,\
             \"seed\":\"{seed}\",\"sim_version\":1}}\n\
             {{\"id\":1,\"kind\":\"weather\",\"adapter_version\":2,\"recorded_at\":1700000200,\
             \"head_commit\":\"{commit}\",\"head_branch\":\"main\",\"at_default_tip\":true,\
             \"source\":\"ci:main\",\"report_mtime\":null,\"content_hash\":\"{hash}\",\
             \"raw\":\"{{\\\"state\\\":\\\"pass\\\"}}\",\"normalized\":65536,\"confidence\":32768,\
             \"contributors\":[{{\"path\":\"tests/pond.rs\",\"value\":1}}]}}\n",
            seed = "01".repeat(32),
            commit = "d".repeat(40),
            hash = "ab".repeat(32),
        );
        assert_eq!(text, expected);
    }

    #[test]
    fn an_export_imports_into_a_new_install_and_exports_the_same_bytes() {
        let (source, id) = holding(&sample());
        let file = export(&source, id);

        let mut target = Fixture::new();
        let there = target.store.create_terrarium(&new_terrarium(1)).unwrap();
        let report = target.store.import_observations(there, &file[..]).unwrap();
        assert_eq!(
            report,
            ImportReport {
                sim_version: SIM_VERSION,
                inserted: 7,
                duplicates: 0,
                conflicts: vec![],
            }
        );
        assert_eq!(export(&target, there), file);
    }

    #[test]
    fn importing_an_export_into_its_own_terrarium_finds_only_duplicates() {
        let (mut fx, id) = holding(&sample());
        let file = export(&fx, id);
        let report = fx.store.import_observations(id, &file[..]).unwrap();
        assert_eq!((report.inserted, report.duplicates), (0, 7));
        assert!(report.conflicts.is_empty());
        assert_eq!(export(&fx, id), file);
    }

    #[test]
    fn a_conflict_keeps_the_stored_row_and_names_the_columns() {
        let mut changed = sample();
        changed[1].raw = r#"{"unused":4}"#.to_owned();
        changed[1].confidence = Q16::HALF;
        changed[1].head.at_default_tip = false;
        let extra = record(ObservationKind::Coverage, 5_000);
        changed.push(extra.clone());
        let (other, other_id) = holding(&changed);

        let (mut fx, id) = holding(&sample());
        let before = fx.store.observations(id, None, None).unwrap();
        let report = fx
            .store
            .import_observations(id, &export(&other, other_id)[..])
            .unwrap();
        assert_eq!((report.inserted, report.duplicates), (1, 6));
        assert_eq!(
            report.conflicts,
            vec![Conflict {
                line: 3,
                kind: ObservationKind::DeadCode,
                source: None,
                head_commit: commit('a'),
                recorded_at: 101,
                columns: vec!["at_default_tip", "raw", "confidence"],
            }]
        );
        let after = fx.store.observations(id, None, None).unwrap();
        assert_eq!(
            after[..before.len()],
            before[..],
            "the stored rows are kept"
        );
        assert_eq!(after.last().unwrap().record, extra);
    }

    #[test]
    fn another_terrariums_export_is_refused_and_nothing_is_written() {
        let (source, id) = holding(&sample());
        let file = export(&source, id);
        let mut fx = Fixture::new();
        let other = fx.store.create_terrarium(&new_terrarium(2)).unwrap();
        let err = fx.store.import_observations(other, &file[..]).unwrap_err();
        assert!(
            matches!(
                err,
                ImportError::SeedMismatch { file, terrarium } if file == seed(1) && terrarium == seed(2)
            ),
            "{err}"
        );
        assert_eq!(fx.store.observations(other, None, None).unwrap(), vec![]);
    }

    #[test]
    fn a_file_that_is_not_a_readable_export_is_refused() {
        let (mut fx, id) = holding(&[]);
        let header = |value: Value| format!("{value}\n");
        let good = json!({
            "format": FORMAT, "format_version": 1, "seed": seed(1).to_hex(), "sim_version": 1
        });
        let with = |key: &str, value: Value| {
            let mut h = good.clone();
            h[key] = value;
            header(h)
        };
        let mut extra = good.clone();
        extra["comment"] = json!("hello");
        // Each refusal, as the start of its `Debug` text.
        let cases = [
            (String::new(), "NotAnExport"),
            ("not json\n".to_owned(), "NotAnExport"),
            (with("format", json!("photos")), "NotAnExport"),
            (with("format_version", json!("1")), "NotAnExport"),
            (with("format_version", json!(2)), "FormatVersion(2)"),
            (
                with("seed", json!("XYZ")),
                "Line { number: 1, problem: BadColumn(\"seed\") }",
            ),
            (header(extra), "Line { number: 1, problem: Malformed("),
        ];
        for (file, expected) in cases {
            let err = fx
                .store
                .import_observations(id, file.as_bytes())
                .unwrap_err();
            assert!(
                format!("{err:?}").starts_with(expected),
                "{file:?}: {err:?}"
            );
        }
        assert!(matches!(
            fx.store
                .import_observations(TerrariumId::new(7), header(good).as_bytes())
                .unwrap_err(),
            ImportError::Store(StoreError::NoSuchTerrarium(_))
        ));
    }

    #[test]
    fn an_invalid_line_is_refused_by_number_and_nothing_is_written() {
        let (source, id) = holding(&[record(ObservationKind::Coverage, 1), weather(2)]);
        let file = String::from_utf8(export(&source, id)).unwrap();
        let lines: Vec<&str> = file.lines().collect();
        let (header, valid, weather_line) = (lines[0], lines[1], lines[2]);
        let edit = |line: &str, key: &str, value: Value| {
            let mut v: Value = serde_json::from_str(line).unwrap();
            if value.is_null() && key != "head_branch" {
                v.as_object_mut().unwrap().remove(key);
            } else {
                v[key] = value;
            }
            v.to_string()
        };
        let contributors: Vec<Value> = (0..21)
            .map(|i| json!({"path": format!("src/f{i}.rs"), "value": i}))
            .collect();
        let cases: Vec<(String, LineProblem)> = vec![
            ("{".to_owned(), LineProblem::Malformed(String::new())),
            (
                edit(valid, "extra", json!(1)),
                LineProblem::Malformed(String::new()),
            ),
            (
                edit(valid, "raw", Value::Null),
                LineProblem::Malformed(String::new()),
            ),
            (
                edit(valid, "kind", json!("sunshine")),
                LineProblem::BadColumn("kind"),
            ),
            (
                edit(valid, "head_commit", json!("XYZ")),
                LineProblem::BadColumn("head_commit"),
            ),
            (
                edit(valid, "content_hash", json!("abc")),
                LineProblem::BadColumn("content_hash"),
            ),
            (
                edit(valid, "source", json!("remote")),
                LineProblem::BadColumn("source"),
            ),
            (
                edit(weather_line, "source", json!("")),
                LineProblem::Invalid(Invalid::WeatherWithoutSource),
            ),
            (
                edit(valid, "normalized", json!(70_000)),
                LineProblem::Invalid(Invalid::OutOfRange("normalized")),
            ),
            (
                edit(valid, "contributors", Value::Array(contributors)),
                LineProblem::Invalid(Invalid::TooManyContributors(21)),
            ),
            (
                edit(valid, "raw", json!("{oops")),
                LineProblem::Invalid(Invalid::RawNotJson),
            ),
            ("x".repeat(MAX_LINE_BYTES + 1), LineProblem::TooLong),
        ];
        let mut fx = Fixture::new();
        let there = fx.store.create_terrarium(&new_terrarium(1)).unwrap();
        for (bad, expected) in cases {
            // A valid line first, so a refusal must undo a write.
            let text = format!("{header}\n{valid}\n{bad}\n{weather_line}\n");
            match fx
                .store
                .import_observations(there, text.as_bytes())
                .unwrap_err()
            {
                ImportError::Line { number, problem } => {
                    assert_eq!(number, 3, "{bad:.80}");
                    match (&problem, &expected) {
                        (LineProblem::Malformed(_), LineProblem::Malformed(_)) => {}
                        _ => assert_eq!(problem, expected, "{bad:.80}"),
                    }
                }
                other => panic!("{bad:.80}: {other}"),
            }
            assert_eq!(count(&fx), 0, "{bad:.80}");
        }
        let mut not_utf8 = format!("{header}\n").into_bytes();
        not_utf8.extend_from_slice(&[0xff, 0xfe, b'\n']);
        assert!(matches!(
            fx.store
                .import_observations(there, &not_utf8[..])
                .unwrap_err(),
            ImportError::Line {
                number: 2,
                problem: LineProblem::Malformed(_)
            }
        ));
        // The same file without the bad line imports, CRLF endings included.
        let good = format!("{header}\r\n{valid}\r\n{weather_line}");
        let report = fx
            .store
            .import_observations(there, good.as_bytes())
            .unwrap();
        assert_eq!(report.inserted, 2);
    }

    #[test]
    fn exporting_an_unknown_terrarium_fails() {
        let fx = Fixture::new();
        assert!(matches!(
            fx.store
                .export_observations(TerrariumId::new(3), Vec::new())
                .unwrap_err(),
            StoreError::NoSuchTerrarium(_)
        ));
    }

    fn text() -> impl Strategy<Value = String> {
        // Any characters, newlines, quotes and backslashes included.
        "(?s).{0,12}"
    }

    prop_compose! {
        fn any_record(at: i64)(
            kind in prop::sample::select(ObservationKind::ALL),
            ci_branch in proptest::option::of("(?s).{1,12}"),
            adapter_version in any::<u32>(),
            commit_digit in prop::sample::select(&b"0123456789abcdef"[..]),
            branch in proptest::option::of(text()),
            at_default_tip in any::<bool>(),
            report_mtime in proptest::option::of(any::<i64>()),
            content_hash in any::<[u8; 32]>(),
            raw_text in text(),
            normalized in proptest::option::of(0..=65_536_i32),
            confidence in 0..=65_536_i32,
            contributors in prop::collection::vec((text(), any::<i64>()), 0..=MAX_CONTRIBUTORS),
        ) -> ObservationRecord {
            ObservationRecord {
                kind,
                adapter_version,
                recorded_at: at,
                head: ObservationHead {
                    commit: CommitId::from_hex(&char::from(commit_digit).to_string().repeat(40))
                        .unwrap(),
                    branch,
                    at_default_tip,
                },
                source: (kind == ObservationKind::Weather).then(|| {
                    ci_branch.map_or(WeatherSource::Local, |branch| WeatherSource::Ci { branch })
                }),
                report_mtime,
                content_hash,
                raw: serde_json::to_string(&raw_text).unwrap(),
                normalized: normalized.map(Q16::from_raw),
                confidence: Q16::from_raw(confidence),
                contributors: contributors
                    .into_iter()
                    .map(|(path, value)| Contributor { path, value })
                    .collect(),
            }
        }
    }

    fn any_records() -> impl Strategy<Value = Vec<ObservationRecord>> {
        (0..6_i64).prop_flat_map(|n| (0..n).map(any_record).collect::<Vec<_>>())
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 32, ..proptest_config() })]

        #[test]
        fn any_observations_round_trip(records in any_records()) {
            let (source, id) = holding(&records);
            let file = export(&source, id);
            prop_assert_eq!(std::str::from_utf8(&file).unwrap().lines().count(), records.len() + 1);
            let mut target = Fixture::new();
            let there = target.store.create_terrarium(&new_terrarium(1)).unwrap();
            let report = target.store.import_observations(there, &file[..]).unwrap();
            prop_assert_eq!(report.inserted, u64::try_from(records.len()).unwrap());
            let stored: Vec<ObservationRecord> = target
                .store
                .observations(there, None, None)
                .unwrap()
                .into_iter()
                .map(|o| o.record)
                .collect();
            prop_assert_eq!(&stored, &records);
            prop_assert_eq!(export(&target, there), file);
        }
    }
}
