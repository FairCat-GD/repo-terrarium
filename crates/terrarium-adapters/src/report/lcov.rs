//! lcov tracefiles (coverage, lcov first), read by this parser rather than a crate.
//!
//! Line coverage comes from `DA` records, merged across records and files the way `lcov -a`
//! merges them: per source file, the union of instrumented lines, a line hit when any record hit
//! it. `LF`/`LH` count only for a record with no `DA` line, since summaries cannot be merged.
//! Function, branch and MC/DC records, and any record type added later, are skipped.
//!
//! A record runs from `SF:` to `end_of_record`, and a file must end outside one. lcov has no
//! end-of-document marker, so a file cut between two records reads as the records before the
//! cut; one cut anywhere else is malformed.

use std::collections::BTreeMap;

use super::paths::Context;
use super::{Contributor, Loaded, Parsed, Reading, Why, count, hits, text};

/// One source file across every record that names it.
#[derive(Default)]
struct Source {
    /// Line number to whether any record hit it.
    lines: BTreeMap<u64, bool>,
    /// The largest `LF`/`LH` of the records that had no `DA`.
    summary: Option<(u64, u64)>,
    /// Where it lies in the repository, if it does.
    path: Option<String>,
}

impl Source {
    fn totals(&self) -> (u64, u64) {
        if self.lines.is_empty() {
            return self.summary.unwrap_or((0, 0));
        }
        let found = u64::try_from(self.lines.len()).unwrap_or(u64::MAX);
        let hit =
            u64::try_from(self.lines.values().filter(|&&hit| hit).count()).unwrap_or(u64::MAX);
        (found, hit)
    }
}

/// The record being read.
struct Record {
    /// The line its `SF:` stands on.
    line: u64,
    key: String,
    lines: BTreeMap<u64, bool>,
    found: Option<u64>,
    hit: Option<u64>,
}

pub(crate) fn parse(context: &Context, files: &[Loaded]) -> Parsed {
    let mut sources: BTreeMap<String, Source> = BTreeMap::new();
    for file in files {
        read_file(context, file, &mut sources)?;
    }
    let mut found = 0_u64;
    let mut hit = 0_u64;
    let mut contributors = Vec::new();
    for source in sources.values() {
        let (f, h) = source.totals();
        let h = h.min(f);
        found = found.saturating_add(f);
        hit = hit.saturating_add(h);
        if let Some(path) = &source.path {
            contributors.push(Contributor {
                path: path.clone(),
                value: i64::try_from(f - h).unwrap_or(i64::MAX),
            });
        }
    }
    if found == 0 {
        return Err(Why::Empty);
    }
    Ok((
        Reading::Coverage {
            lines_found: found,
            lines_hit: hit,
        },
        contributors,
    ))
}

fn read_file(
    context: &Context,
    file: &Loaded,
    sources: &mut BTreeMap<String, Source>,
) -> Result<(), Why> {
    let text = text(file)?;
    let mut record: Option<Record> = None;
    for (index, line) in text.split('\n').enumerate() {
        let line_no = u64::try_from(index).map_or(u64::MAX, |i| i.saturating_add(1));
        let malformed = || Why::Malformed {
            path: file.path.clone(),
            line: Some(line_no),
        };
        let line = line.strip_suffix('\r').unwrap_or(line).trim();
        if line.is_empty() {
            continue;
        }
        if line == "end_of_record" {
            let done = record.take().ok_or_else(malformed)?;
            close(context, done, sources);
            continue;
        }
        let Some((tag, value)) = line.split_once(':') else {
            // A record line always has a tag; a bare word other than `end_of_record` is a cut
            // or another format.
            return Err(malformed());
        };
        match (tag, record.as_mut()) {
            ("SF", None) => {
                if value.is_empty() {
                    return Err(malformed());
                }
                record = Some(Record {
                    line: line_no,
                    key: value.to_owned(),
                    lines: BTreeMap::new(),
                    found: None,
                    hit: None,
                });
            }
            ("DA", Some(open)) => {
                let mut fields = value.split(',');
                let number = fields.next().and_then(count).ok_or_else(malformed)?;
                let ran = fields.next().and_then(hits).ok_or_else(malformed)?;
                // A third field is an optional checksum; more is not lcov.
                if fields.nth(1).is_some() {
                    return Err(malformed());
                }
                let entry = open.lines.entry(number).or_insert(false);
                *entry |= ran;
            }
            ("LF", Some(open)) => open.found = Some(count(value).ok_or_else(malformed)?),
            ("LH", Some(open)) => open.hit = Some(count(value).ok_or_else(malformed)?),
            // A new file before the last one ended, or a line outside any file.
            ("SF", Some(_)) | ("DA" | "LF" | "LH", None) => return Err(malformed()),
            // TN, VER, FN, FNDA, FNF, FNH, FNL, FNA, BRDA, BRF, BRH, MCDC, MRF, MRH and anything
            // newer: not line coverage.
            _ => {}
        }
    }
    // A record still open is a file cut short: reading the records before it would record part
    // of a report as the whole.
    if let Some(open) = record {
        return Err(Why::Malformed {
            path: file.path.clone(),
            line: Some(open.line),
        });
    }
    Ok(())
}

fn close(context: &Context, record: Record, sources: &mut BTreeMap<String, Source>) {
    let path = context.relative(&record.key);
    // The same file named two ways (absolute and relative) is one source.
    let key = path.clone().unwrap_or(record.key);
    let source = sources.entry(key).or_default();
    source.path = path;
    if record.lines.is_empty() {
        if let (Some(found), Some(hit)) = (record.found, record.hit) {
            let (f, h) = source.summary.unwrap_or((0, 0));
            source.summary = Some((f.max(found), h.max(hit)));
        }
    } else {
        for (number, ran) in record.lines {
            *source.lines.entry(number).or_insert(false) |= ran;
        }
    }
}
