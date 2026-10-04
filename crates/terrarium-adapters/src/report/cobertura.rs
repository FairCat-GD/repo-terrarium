//! Cobertura XML (coverage, second to lcov), as istanbul, coverage.py, tarpaulin and llvm-cov
//! write it.
//!
//! Lines come from each class's own `<lines>`; the copies under `<methods>` repeat them. A line
//! counts once per file, hit when any class hit it, so several files merge as lcov's do. The
//! root's `lines-valid` and `lines-covered` are used only by a report with no line at all. Class
//! file names are read against each `<source>`.

use std::borrow::Cow;
use std::collections::BTreeMap;

use super::paths::Context;
use super::xml::{Step, attribute, walk};
use super::{Contributor, Loaded, Parsed, Reading, Why, count, hits};

#[derive(Default)]
struct Source {
    lines: BTreeMap<u64, bool>,
    path: Option<String>,
}

/// What one file holds, before its class file names are read against its sources (which a
/// tool may write anywhere in the document).
#[derive(Default)]
struct Document {
    sources: Vec<String>,
    /// Class file name to its lines.
    classes: BTreeMap<String, BTreeMap<u64, bool>>,
    root_totals: Option<(u64, u64)>,
    bad_line: bool,
}

pub(crate) fn parse(context: &Context, files: &[Loaded]) -> Parsed {
    let mut merged: BTreeMap<String, Source> = BTreeMap::new();
    let mut root_found = 0_u64;
    let mut root_hit = 0_u64;
    for file in files {
        let document = read_file(file)?;
        if let Some((found, hit)) = document.root_totals {
            root_found = root_found.saturating_add(found);
            root_hit = root_hit.saturating_add(hit.min(found));
        }
        for (name, lines) in document.classes {
            let path = resolve(context, &document.sources, &name, &file.path);
            let source = merged.entry(path.clone().unwrap_or(name)).or_default();
            source.path = path;
            for (number, ran) in lines {
                *source.lines.entry(number).or_insert(false) |= ran;
            }
        }
    }
    let mut found = 0_u64;
    let mut hit = 0_u64;
    let mut contributors = Vec::new();
    for source in merged.values() {
        let f = u64::try_from(source.lines.len()).unwrap_or(u64::MAX);
        let h =
            u64::try_from(source.lines.values().filter(|&&ran| ran).count()).unwrap_or(u64::MAX);
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
        (found, hit) = (root_found, root_hit);
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

/// The first reading of `name` against a source that lands inside the repository, or the name
/// as the report at `report` names it.
fn resolve(context: &Context, sources: &[String], name: &str, report: &str) -> Option<String> {
    sources
        .iter()
        .find_map(|source| context.relative_to(source, name))
        .or_else(|| context.locate(name, report))
}

fn read_file(file: &Loaded) -> Result<Document, Why> {
    let mut document = Document::default();
    let mut class: Option<String> = None;
    let mut root: Option<String> = None;
    walk(file, |around, step| match step {
        Step::Start { name, element } => {
            if around.is_empty() {
                root = Some(name.to_owned());
                let totals = attribute(element, "lines-valid")
                    .as_deref()
                    .and_then(count)
                    .zip(
                        attribute(element, "lines-covered")
                            .as_deref()
                            .and_then(count),
                    );
                document.root_totals = totals;
            }
            match (name, around.last().map(String::as_str)) {
                ("class", _) => {
                    class = attribute(element, "filename").map(Cow::into_owned);
                }
                // Only a class's own lines: `class > lines > line`.
                ("line", Some("lines"))
                    if around.len() >= 2 && around[around.len() - 2] == "class" =>
                {
                    let (Some(file_name), Some(number), Some(ran)) = (
                        class.as_ref(),
                        attribute(element, "number").as_deref().and_then(count),
                        attribute(element, "hits").as_deref().and_then(hits),
                    ) else {
                        document.bad_line = true;
                        return;
                    };
                    let lines = document.classes.entry(file_name.clone()).or_default();
                    *lines.entry(number).or_insert(false) |= ran;
                }
                _ => {}
            }
        }
        Step::End { name } => {
            if name == "class" {
                class = None;
            }
        }
        Step::Text(text) => {
            if around.last().map(String::as_str) == Some("source") {
                document.sources.push(text.trim().to_owned());
            }
        }
    })?;
    if root.as_deref() != Some("coverage") || document.bad_line {
        return Err(Why::Malformed {
            path: file.path.clone(),
            line: None,
        });
    }
    Ok(document)
}
