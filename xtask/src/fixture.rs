//! `cargo xtask fixture`: the scripted arc repository (`terrarium_adapters::fixture::arc`) and
//! the observations recorded alongside it.
//!
//! - `cargo xtask fixture <dir>` writes the arc to `<dir>/repo` and its observations to
//!   `<dir>/observations.jsonl`, for running the app or a replay against it.
//! - `cargo xtask fixture --bless` rewrites the committed `tests/fixtures/arc/observations.jsonl`,
//!   which golden replays and the engine's tests import. A test fails while it differs from a
//!   fresh build.
//!
//! The observations are written through a temporary store and its own export, so the file is
//! exactly the store's JSON Lines (format 1). Their values follow the arc's phases: coverage and
//! dead code healthy while it thrives, falling as it struggles, absent while it is dead, and
//! climbing back as it recovers; weather from local test runs at first and CI on `main` later,
//! with failing stretches.

use std::path::Path;
use std::process::ExitCode;

use sha2::{Digest, Sha256};
use terrarium_adapters::fixture::{self, Arc, Phase};
use terrarium_store::raw::{
    CoverageFormat, CoverageRaw, DeadCodeRaw, Shape, WeatherRaw, WeatherState,
};
use terrarium_store::{Contributor, DataDir, NewTerrarium, ObservationRecord, RepoLocation, Store};
use terrarium_truth::domain::{ObservationHead, WeatherSource};
use terrarium_truth::{CommitId, ObservationKind, Q16, RepoSeed};

/// The committed observations, relative to the workspace root.
pub const COMMITTED: &str = "tests/fixtures/arc/observations.jsonl";

const DAY: i64 = 86_400;
const HOUR: i64 = 3_600;
const MINUTE: i64 = 60;

/// A fictional repository's fictional files, as observations name them.
const COVERAGE_FILES: [&str; 3] = ["src/meadow.rs", "src/pond/frog.rs", "src/ground/loam.rs"];
const DEAD_CODE_FILES: [&str; 2] = ["src/meadow.rs", "src/trellis/vine.rs"];

pub fn run(out: &Path) -> ExitCode {
    match write(out) {
        Ok(()) => {
            println!(
                "fixture: the arc is in {} and its observations in {}",
                out.join("repo").display(),
                out.join("observations.jsonl").display()
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("fixture: {err}");
            ExitCode::FAILURE
        }
    }
}

pub fn bless(root: &Path) -> ExitCode {
    let result = (|| -> Result<(String, String), String> {
        let fresh = build()?;
        let path = root.join(COMMITTED);
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, &fresh).map_err(|e| e.to_string())?;
        Ok((old, fresh))
    })();
    match result {
        Ok((old, fresh)) if old == fresh => {
            println!("fixture: {COMMITTED} is unchanged");
            ExitCode::SUCCESS
        }
        Ok((old, fresh)) => {
            println!(
                "fixture: rewrote {COMMITTED}: {} lines, was {}",
                fresh.lines().count(),
                old.lines().count()
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("fixture: {err}");
            ExitCode::FAILURE
        }
    }
}

fn write(out: &Path) -> Result<(), String> {
    if out.join("repo").exists() {
        return Err(format!("{} already exists", out.join("repo").display()));
    }
    let arc = fixture::arc(&out.join("repo")).map_err(|e| e.to_string())?;
    let text = export(&arc)?;
    std::fs::write(out.join("observations.jsonl"), text).map_err(|e| e.to_string())
}

/// The arc's observations as the store exports them, building the arc in a temporary
/// directory.
pub fn build() -> Result<String, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let arc = fixture::arc(&dir.path().join("repo")).map_err(|e| e.to_string())?;
    export(&arc)
}

/// The arc's root commit, which names its terrarium.
pub fn seed(arc: &Arc) -> RepoSeed {
    RepoSeed::from_root_commit(&arc.commits[0].id)
}

fn export(arc: &Arc) -> Result<String, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut store = Store::open(&DataDir::at(dir.path())).map_err(|e| e.to_string())?;
    let terrarium = store
        .create_terrarium(&NewTerrarium {
            seed: seed(arc),
            repo: RepoLocation {
                path: "/fixtures/arc".to_owned(),
                name: "arc".into(),
                default_branch: arc.default_branch.into(),
            },
            install_salt: [0; 16],
            connected_at: fixture::ARC_START_SECS,
        })
        .map_err(|e| e.to_string())?;
    for record in observations(arc) {
        store
            .append_observation(terrarium, &record)
            .map_err(|e| format!("{e} ({:?} at {})", record.kind, record.recorded_at))?;
    }
    let mut out = Vec::new();
    store
        .export_observations(terrarium, &mut out)
        .map_err(|e| e.to_string())?;
    String::from_utf8(out).map_err(|e| e.to_string())
}

/// Thousandths to Q16.
fn q16(thousandths: i64) -> Q16 {
    Q16::from_ratio(i32::try_from(thousandths).unwrap_or(1000), 1000)
}

/// Linear from `from` to `to` (thousandths) across the days `start..end`, at `day`.
fn ramp(day: i64, start: i64, end: i64, from: i64, to: i64) -> i64 {
    from + (to - from) * (day - start).clamp(0, end - start) / (end - start).max(1)
}

/// What one observation records, before its head and hashes are filled in.
struct Reading {
    kind: ObservationKind,
    at: i64,
    raw: String,
    normalized: Option<Q16>,
    source: Option<WeatherSource>,
    from_file: bool,
    contributors: Vec<Contributor>,
}

/// The arc's day on which `phase` begins.
fn first_day(phase: Phase) -> i64 {
    (phase.start_secs() - fixture::ARC_START_SECS) / DAY
}

/// Coverage on day `day` of the arc, in thousandths.
fn coverage(day: i64) -> Option<i64> {
    let (struggling, dead, recovering) = (
        first_day(Phase::Struggling),
        first_day(Phase::Dead),
        first_day(Phase::Recovering),
    );
    match Phase::at(fixture::ARC_START_SECS + day * DAY) {
        Phase::Thriving => Some(820 - (day % 3) * 5),
        Phase::Struggling => Some(ramp(day, struggling, dead, 780, 380)),
        Phase::Dead => None,
        Phase::Recovering => Some(ramp(day, recovering, recovering + 120, 420, 820)),
    }
}

/// Dead code's (unused, exported) items on day `day`.
fn dead_code(day: i64) -> Option<(u32, u32)> {
    let exported = 120 + u32::try_from(day / 7).unwrap_or(0);
    let unused = match Phase::at(fixture::ARC_START_SECS + day * DAY) {
        Phase::Thriving => 3 + u32::try_from(day % 4).unwrap_or(0),
        Phase::Struggling => {
            6 + u32::try_from((day - first_day(Phase::Struggling)) / 5).unwrap_or(0)
        }
        Phase::Dead => return None,
        // A clearing on day 640: from about 40 unused items down to 12.
        Phase::Recovering if day < 640 => 40,
        Phase::Recovering => 12 - u32::try_from(((day - 640) / 30).min(4)).unwrap_or(0),
    };
    Some((unused, exported))
}

/// Weather on day `day`: its source and state.
fn weather(day: i64) -> Option<(WeatherSource, WeatherState)> {
    let ci = WeatherSource::Ci {
        branch: "main".into(),
    };
    Some(match Phase::at(fixture::ARC_START_SECS + day * DAY) {
        Phase::Thriving => (WeatherSource::Local, WeatherState::Pass),
        Phase::Dead => return None,
        // Failing from day 301 through day 311, then a single failure and an unknown run.
        Phase::Struggling if (301..=311).contains(&day) || day == 357 => (ci, WeatherState::Fail),
        Phase::Struggling if day == 385 => (ci, WeatherState::Unknown),
        Phase::Struggling | Phase::Recovering => (ci, WeatherState::Pass),
    })
}

fn contributors(paths: &[&str], day: i64) -> Vec<Contributor> {
    paths
        .iter()
        .zip(1..)
        .map(|(path, n)| Contributor {
            path: (*path).to_owned(),
            value: (day * n) % 37 + n,
        })
        .collect()
}

fn readings(day: i64) -> Vec<Reading> {
    let start = fixture::ARC_START_SECS + day * DAY;
    let weekday = day % 7;
    let mut out = Vec::new();
    if weekday == 2
        && let Some(thousandths) = coverage(day)
    {
        let lines_found = 1_000 + u64::try_from(day * 3).unwrap_or(0);
        let lines_hit = lines_found * u64::try_from(thousandths).unwrap_or(0) / 1_000;
        out.push(Reading {
            kind: ObservationKind::Coverage,
            at: start + 17 * HOUR + 30 * MINUTE,
            raw: CoverageRaw {
                format: CoverageFormat::Lcov,
                lines_found,
                lines_hit,
            }
            .to_raw(),
            normalized: Some(Q16::from_ratio(
                i32::try_from(lines_hit).unwrap_or(0),
                i32::try_from(lines_found).unwrap_or(1),
            )),
            source: None,
            from_file: true,
            contributors: contributors(&COVERAGE_FILES, day),
        });
    }
    if weekday == 4
        && let Some((unused, exported)) = dead_code(day)
    {
        let share = Q16::from_ratio(
            i32::try_from(unused.min(exported)).unwrap_or(0),
            i32::try_from(exported.max(1)).unwrap_or(1),
        );
        out.push(Reading {
            kind: ObservationKind::DeadCode,
            at: start + 17 * HOUR + 45 * MINUTE,
            raw: DeadCodeRaw {
                unused_items: unused,
                exported_items: exported,
            }
            .to_raw(),
            normalized: Some(Q16::ONE.saturating_sub(share)),
            source: None,
            from_file: true,
            contributors: contributors(&DEAD_CODE_FILES, day),
        });
    }
    if matches!(weekday, 0 | 3)
        && let Some((source, state)) = weather(day)
    {
        let from_file = source == WeatherSource::Local;
        out.push(Reading {
            kind: ObservationKind::Weather,
            at: start + 18 * HOUR,
            raw: WeatherRaw { state }.to_raw(),
            normalized: match state {
                WeatherState::Pass => Some(Q16::ONE),
                WeatherState::Fail => Some(Q16::ZERO),
                WeatherState::Unknown => None,
            },
            source: Some(source),
            from_file,
            contributors: Vec::new(),
        });
    }
    out
}

fn record(
    reading: Reading,
    commit: CommitId,
    branch: &str,
    at_default_tip: bool,
) -> ObservationRecord {
    ObservationRecord {
        kind: reading.kind,
        adapter_version: 1,
        recorded_at: reading.at,
        head: ObservationHead {
            commit,
            branch: Some(branch.to_owned()),
            at_default_tip,
        },
        source: reading.source,
        report_mtime: reading.from_file.then_some(reading.at - 30),
        content_hash: Sha256::digest(reading.raw.as_bytes()).into(),
        raw: reading.raw,
        normalized: reading.normalized,
        confidence: Q16::ONE,
        contributors: reading.contributors,
    }
}

/// Every observation of the arc, in the order recorded.
pub fn observations(arc: &Arc) -> Vec<ObservationRecord> {
    let days = (fixture::ARC_END_SECS - fixture::ARC_START_SECS) / DAY;
    let mut out = Vec::new();
    for day in 0..days {
        for reading in readings(day) {
            let Some(tip) = arc.main_tip_at(reading.at) else {
                continue;
            };
            let tip = tip.id.clone();
            out.push(record(reading, tip, "main", true));
        }
    }
    // Readings on side branches are recorded and shown, never applied.
    for (branch, day, thousandths) in [("feature/trellis", 140, 610), ("experiment/moss", 603, 450)]
    {
        let at = fixture::ARC_START_SECS + day * DAY + 21 * HOUR;
        let tip = arc
            .commits
            .iter()
            .filter(|c| c.branch == branch && c.secs <= at)
            .max_by_key(|c| c.secs);
        if let Some(tip) = tip {
            let lines_found = 1_000;
            let lines_hit = 1_000 * u64::try_from(thousandths).unwrap_or(0) / 1_000;
            let reading = Reading {
                kind: ObservationKind::Coverage,
                at,
                raw: CoverageRaw {
                    format: CoverageFormat::Lcov,
                    lines_found,
                    lines_hit,
                }
                .to_raw(),
                normalized: Some(q16(thousandths)),
                source: None,
                from_file: true,
                contributors: contributors(&COVERAGE_FILES[..1], day),
            };
            out.push(record(reading, tip.id.clone(), branch, false));
        }
    }
    out.sort_by_key(|r| r.recorded_at);
    out
}

#[cfg(test)]
mod tests {
    use terrarium_store::EXPORT_FORMAT_VERSION;

    use super::*;

    fn committed() -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        std::fs::read_to_string(root.join(COMMITTED)).expect("the committed observations")
    }

    #[test]
    fn the_committed_arc_observations_are_current() {
        let fresh = build().unwrap();
        let committed = committed();
        let (fresh_header, fresh_lines) = fresh.split_once('\n').unwrap();
        let (header, lines) = committed.split_once('\n').unwrap();
        // The header's `sim_version` is the build's, and an input need not follow it.
        let without_version = |line: &str| {
            let mut header: serde_json::Value = serde_json::from_str(line).unwrap();
            header.as_object_mut().unwrap().remove("sim_version");
            header
        };
        assert_eq!(without_version(header), without_version(fresh_header));
        assert!(
            lines == fresh_lines,
            "{COMMITTED} is stale: run `cargo xtask fixture --bless`"
        );
    }

    #[test]
    fn the_committed_observations_import_into_the_arcs_terrarium() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let arc = fixture::arc(&repo).unwrap();
        let mut store = Store::open(&DataDir::at(dir.path().join("data"))).unwrap();
        let terrarium = store
            .create_terrarium(&NewTerrarium {
                seed: seed(&arc),
                repo: RepoLocation {
                    path: repo.display().to_string(),
                    name: "arc".into(),
                    default_branch: "main".into(),
                },
                install_salt: [7; 16],
                connected_at: 0,
            })
            .unwrap();
        let committed = committed();
        let report = store
            .import_observations(terrarium, committed.as_bytes())
            .unwrap();
        assert!(report.conflicts.is_empty());
        assert_eq!(report.duplicates, 0);
        let lines = committed.lines().count() - 1;
        assert_eq!(usize::try_from(report.inserted).unwrap(), lines);
        let stored = store.observations(terrarium, None, None).unwrap();
        let kinds = |kind| stored.iter().filter(|o| o.record.kind == kind).count();
        assert!(kinds(ObservationKind::Coverage) > 40);
        assert!(kinds(ObservationKind::DeadCode) > 40);
        assert!(kinds(ObservationKind::Weather) > 80);
        let side_branch = stored
            .iter()
            .filter(|o| !o.record.head.at_default_tip)
            .count();
        assert_eq!(side_branch, 2);
        assert!(stored.iter().all(|o| o.to_observation().is_ok()));
        assert!(committed.starts_with(&format!(
            "{{\"format\":\"repo-terrarium-observations\",\"format_version\":{EXPORT_FORMAT_VERSION},\"seed\":\"{}\"",
            seed(&arc).to_hex()
        )));
    }
}
