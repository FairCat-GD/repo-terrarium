//! `cargo xtask soak` (N-01..N-03): the release app run for an hour on a busy repository,
//! measured by its own `--probe soak`, and judged.
//!
//! - **The repository** is the synthetic history (3,000 files, so the strip holds every plant
//!   and resident it can) ending an hour before the run, so the strip is active and thriving.
//!   The harness owns it, adds a commit to it every ten minutes so the engine refreshes as it
//!   would in a working day, and deletes it after.
//! - **The app** is the release build, copied out first so a build during the hour cannot
//!   replace it. Its soak connects the repository in a data directory of its own.
//! - **The judgement** is the stricter budget on Apple silicon (under 0.5% of one core) and
//!   N-01's (1%) elsewhere, N-02's memory (80 MB once settled, 150 MB at the peak, in the
//!   measure each OS counts fairly), N-03's 15 frames in any second, and a strip kept in view:
//!   a report covered for more than 5% of the hour measured too little to judge.
//!   `--report <file>` judges a report from another machine, such as a Windows soak, against
//!   N-01.
//!
//! Nothing should be built while it runs: the processor time of the same work changes with the
//! cores it lands on.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;
use terrarium_adapters::fixture::{Edit, FixtureRepo, Stamp, synthetic_until};

use crate::{release, sweep};

/// The busy repository's commits.
const COMMITS: usize = 5_000;
/// The engine's work in a working day: a commit this often.
const COMMIT_EVERY: Duration = Duration::from_mins(10);

/// N-01's processor budget, as a percentage of one core, and the stricter one on Apple silicon.
const CPU_PERCENT: f64 = 1.0;
const CPU_PERCENT_APPLE: f64 = 0.5;
/// N-02, in decimal megabytes, the stricter reading.
const STEADY_BYTES: u64 = 80_000_000;
const PEAK_BYTES: u64 = 150_000_000;
/// N-03.
const FRAMES_PER_SECOND: u32 = 15;
/// More of the hour covered than this, and too little of it was measured.
const COVERED_PERCENT: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presenter {
    Default,
    Layer,
    Softbuffer,
    Wgpu,
    /// One app each, at once: softbuffer on the bottom edge, wgpu on the top.
    Both,
}

#[derive(Debug, PartialEq)]
struct Options {
    minutes: u32,
    presenter: Presenter,
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    if let [flag, file] = args
        && flag == "--report"
    {
        return judge_file(Path::new(file), CPU_PERCENT);
    }
    let options = match options(args) {
        Ok(options) => options,
        Err(err) => {
            println!("soak: {err}");
            return ExitCode::FAILURE;
        }
    };
    match soak(root, &options) {
        Ok(passed) if passed => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(err) => {
            println!("soak: {err}");
            ExitCode::FAILURE
        }
    }
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        minutes: 60,
        presenter: Presenter::Default,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let value = args.next().ok_or_else(|| format!("{arg} needs a value"))?;
        match arg.as_str() {
            "--minutes" => {
                options.minutes = value
                    .parse()
                    .ok()
                    .filter(|minutes| (1..=24 * 60).contains(minutes))
                    .ok_or_else(|| format!("'{value}' is not a length from 1 to 1440 minutes"))?;
            }
            "--presenter" => {
                options.presenter = match value.as_str() {
                    "layer" => Presenter::Layer,
                    "softbuffer" => Presenter::Softbuffer,
                    "wgpu" => Presenter::Wgpu,
                    "both" => Presenter::Both,
                    _ => {
                        return Err(format!(
                            "'{value}' is no presenter: layer, softbuffer, wgpu or both"
                        ));
                    }
                };
            }
            _ => return Err(format!("unexpected argument '{arg}'")),
        }
    }
    Ok(options)
}

/// One app soaking.
struct Run {
    name: &'static str,
    out: PathBuf,
    child: Child,
}

fn soak(root: &Path, options: &Options) -> Result<bool, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let built = Command::new(&cargo)
        .args(release::BUILD)
        .current_dir(root)
        .status()
        .map_err(|err| format!("the release build could not start: {err}"))?;
    if !built.success() {
        return Err("the release build failed".to_owned());
    }
    let name = format!("repo-terrarium{}", std::env::consts::EXE_SUFFIX);
    let work = tempfile::Builder::new()
        .prefix("repo-terrarium-xtask-soak-")
        .tempdir()
        .map_err(|err| format!("no temporary directory: {err}"))?;
    let app = work.path().join(&name);
    fs::copy(sweep::target_dir(root).join("release").join(&name), &app)
        .map_err(|err| format!("the release binary could not be copied: {err}"))?;
    let repo_dir = work.path().join("repo");
    println!("soak: writing the busy repository ({COMMITS} commits)");
    let mut repo = synthetic_until(&repo_dir, COMMITS, terrarium_engine::wall_secs() - 3_600)
        .map_err(|err| format!("the repository could not be written: {err}"))?;

    let mut runs = start(&app, &repo_dir, work.path(), options)?;
    println!(
        "soak: {} running for {} minutes; keep the strip in view and build nothing",
        runs.iter()
            .map(|run| run.name)
            .collect::<Vec<_>>()
            .join(" and "),
        options.minutes
    );
    tend(&mut runs, &mut repo)?;

    let results = sweep::target_dir(root).join("soak");
    fs::create_dir_all(&results).map_err(|err| format!("{}: {err}", results.display()))?;
    let mut passed = true;
    for run in &runs {
        let report = keep_reports(&run.out, &results)?;
        println!("\n==> {} ({})", run.name, report.display());
        passed &= judge_file(&report, cpu_budget()) == ExitCode::SUCCESS;
    }
    Ok(passed)
}

/// Starts the app `options` ask for: one, or one on each edge with each presenter.
fn start(app: &Path, repo_dir: &Path, work: &Path, options: &Options) -> Result<Vec<Run>, String> {
    let plan: &[(&'static str, Option<&str>, Option<&str>)] = match options.presenter {
        Presenter::Default => &[("default", None, None)],
        Presenter::Layer => &[("layer", Some("layer"), None)],
        Presenter::Softbuffer => &[("softbuffer", Some("softbuffer"), None)],
        Presenter::Wgpu => &[("wgpu", Some("wgpu"), None)],
        Presenter::Both => &[
            ("softbuffer", Some("softbuffer"), Some("bottom")),
            ("wgpu", Some("wgpu"), Some("top")),
        ],
    };
    let mut runs = Vec::new();
    for &(label, presenter, edge) in plan {
        let out = work.join(format!("out-{label}"));
        fs::create_dir_all(&out).map_err(|err| format!("{}: {err}", out.display()))?;
        let mut command = Command::new(app);
        command
            .args(["--probe", "soak", "--minutes", &options.minutes.to_string()])
            .arg("--repo")
            .arg(repo_dir)
            .current_dir(&out)
            .stdin(Stdio::null());
        if let Some(presenter) = presenter {
            command.args(["--presenter", presenter]);
        }
        if let Some(edge) = edge {
            command.args(["--edge", edge]);
        }
        let child = command
            .spawn()
            .map_err(|err| format!("the app could not start: {err}"))?;
        runs.push(Run {
            name: label,
            out,
            child,
        });
    }
    Ok(runs)
}

/// Waits for every run to end, committing to `repo` every [`COMMIT_EVERY`] meanwhile.
fn tend(runs: &mut [Run], repo: &mut FixtureRepo) -> Result<(), String> {
    let started = Instant::now();
    let mut commits = 0_u32;
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let mut running = 0;
        for run in runs.iter_mut() {
            match run.child.try_wait() {
                Ok(None) => running += 1,
                Ok(Some(_)) => {}
                Err(err) => {
                    return Err(format!(
                        "the {} app could not be waited for: {err}",
                        run.name
                    ));
                }
            }
        }
        if running == 0 {
            return Ok(());
        }
        if started.elapsed() >= COMMIT_EVERY * (commits + 1) {
            commits += 1;
            let body = format!("pub const TENDED_{commits}: u32 = {commits};\n");
            repo.commit(
                "main",
                Stamp::at(terrarium_engine::wall_secs()),
                "tend the soak",
                &[Edit::Write(
                    format!("soak/s{commits}.rs"),
                    body.into_bytes(),
                )],
            )
            .map_err(|err| format!("a commit could not be written: {err}"))?;
        }
    }
}

/// Copies the reports a run wrote into `results`; the JSON one's new path.
fn keep_reports(out: &Path, results: &Path) -> Result<PathBuf, String> {
    let mut json = None;
    for entry in fs::read_dir(out).map_err(|err| format!("{}: {err}", out.display()))? {
        let path = entry.map_err(|err| err.to_string())?.path();
        let Some(file) = path.file_name() else {
            continue;
        };
        let kept = results.join(file);
        fs::copy(&path, &kept).map_err(|err| format!("{}: {err}", path.display()))?;
        if path.extension().is_some_and(|ext| ext == "json") {
            json = Some(kept);
        }
    }
    json.ok_or_else(|| format!("no report in {}", out.display()))
}

/// The stricter budget holds on Apple silicon; N-01's elsewhere.
fn cpu_budget() -> f64 {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        CPU_PERCENT_APPLE
    } else {
        CPU_PERCENT
    }
}

/// The report as the judge reads it: the app writes more.
#[derive(Debug, Deserialize)]
struct Report {
    os: String,
    minutes: u32,
    presenter: Option<String>,
    pixels_equal: Option<bool>,
    summary: Option<Summary>,
    ended: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Summary {
    measured_secs: u64,
    cpu_percent: Option<f64>,
    resident_steady: Option<u64>,
    footprint_steady: Option<u64>,
    private_steady: Option<u64>,
    resident_peak: Option<u64>,
    footprint_peak: Option<u64>,
    private_peak: Option<u64>,
    frames_per_second: f64,
    busiest_second: u32,
    covered_percent: f64,
}

fn judge_file(path: &Path, cpu_budget: f64) -> ExitCode {
    let report = fs::read_to_string(path)
        .map_err(|err| err.to_string())
        .and_then(|text| serde_json::from_str::<Report>(&text).map_err(|err| err.to_string()));
    match report {
        Ok(report) => {
            let (passed, lines) = judge(&report, cpu_budget);
            for line in lines {
                println!("{line}");
            }
            if passed {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(err) => {
            println!("soak: {}: {err}", path.display());
            ExitCode::FAILURE
        }
    }
}

/// Whether `report` is within every budget, and a line for each.
fn judge(report: &Report, cpu_budget: f64) -> (bool, Vec<String>) {
    let mut lines = vec![format!(
        "{}, {} minutes, presenter {}",
        report.os,
        report.minutes,
        report.presenter.as_deref().unwrap_or("unknown")
    )];
    let mut passed = true;
    let mut check = |within: bool, line: String| {
        passed &= within;
        lines.push(format!(
            "  {line}: {}",
            if within { "ok" } else { "FAILED" }
        ));
    };
    check(
        report.ended.as_deref() == Some("done"),
        format!(
            "ended: {}",
            report.ended.as_deref().unwrap_or("still running")
        ),
    );
    if let Some(equal) = report.pixels_equal {
        check(equal, format!("GPU pixels equal the CPU's: {equal}"));
    }
    let Some(summary) = &report.summary else {
        check(false, "measured: nothing".to_owned());
        return (passed, lines);
    };
    check(
        summary.covered_percent <= COVERED_PERCENT,
        format!(
            "covered {:.1}% of {} s measured, at most {COVERED_PERCENT}%",
            summary.covered_percent, summary.measured_secs
        ),
    );
    match summary.cpu_percent {
        Some(cpu) => check(
            cpu < cpu_budget,
            format!("processor {cpu:.3}% of one core, under {cpu_budget}%"),
        ),
        None => check(false, "processor: not measured".to_owned()),
    }
    // macOS counts the footprint; Windows private bytes and the working set, both held to it.
    let measures: Vec<(&str, Option<u64>, Option<u64>)> = if summary.footprint_steady.is_some() {
        vec![(
            "footprint",
            summary.footprint_steady,
            summary.footprint_peak,
        )]
    } else {
        vec![
            (
                "private bytes",
                summary.private_steady,
                summary.private_peak,
            ),
            (
                "working set",
                summary.resident_steady,
                summary.resident_peak,
            ),
        ]
    };
    for (name, steady, peak) in measures {
        match steady {
            Some(steady) => check(
                steady < STEADY_BYTES,
                format!("{name} {} once settled, under 80 MB", megabytes(steady)),
            ),
            None => check(false, format!("{name}: not measured")),
        }
        if let Some(peak) = peak {
            check(
                peak < PEAK_BYTES,
                format!("{name} {} at its peak, under 150 MB", megabytes(peak)),
            );
        }
    }
    check(
        summary.busiest_second <= FRAMES_PER_SECOND,
        format!(
            "frames: {:.2} a second, at most {} in any second, at most {FRAMES_PER_SECOND}",
            summary.frames_per_second, summary.busiest_second
        ),
    );
    (passed, lines)
}

/// Decimal megabytes, to a tenth.
fn megabytes(bytes: u64) -> String {
    let tenths = bytes / 100_000;
    format!("{}.{} MB", tenths / 10, tenths % 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(summary: Summary) -> Report {
        Report {
            os: "macOS 15.4".to_owned(),
            minutes: 60,
            presenter: Some("wgpu".to_owned()),
            pixels_equal: Some(true),
            summary: Some(summary),
            ended: Some("done".to_owned()),
        }
    }

    fn within() -> Summary {
        Summary {
            measured_secs: 3_300,
            cpu_percent: Some(0.31),
            resident_steady: Some(70_000_000),
            footprint_steady: Some(42_000_000),
            footprint_peak: Some(61_000_000),
            frames_per_second: 9.8,
            busiest_second: 15,
            covered_percent: 0.4,
            ..Summary::default()
        }
    }

    fn passes(summary: Summary, cpu_budget: f64) -> bool {
        judge(&report(summary), cpu_budget).0
    }

    #[test]
    fn a_soak_within_every_budget_passes() {
        let (passed, lines) = judge(&report(within()), CPU_PERCENT_APPLE);
        assert!(passed, "{lines:#?}");
        assert!(lines.iter().skip(1).all(|line| line.ends_with(": ok")));
    }

    #[test]
    fn each_budget_fails_the_soak_on_its_own() {
        let over = [
            Summary {
                cpu_percent: Some(0.5),
                ..within()
            },
            Summary {
                cpu_percent: None,
                ..within()
            },
            Summary {
                footprint_steady: Some(80_000_000),
                ..within()
            },
            Summary {
                footprint_peak: Some(150_000_000),
                ..within()
            },
            Summary {
                busiest_second: 16,
                ..within()
            },
            Summary {
                covered_percent: 5.1,
                ..within()
            },
        ];
        for summary in over {
            assert!(!passes(summary, CPU_PERCENT_APPLE));
        }
        // The same processor time is within N-01's budget on another machine.
        let elsewhere = Summary {
            cpu_percent: Some(0.9),
            ..within()
        };
        assert!(passes(elsewhere, CPU_PERCENT));
    }

    #[test]
    fn windows_holds_private_bytes_and_the_working_set_to_the_budget() {
        let windows = Summary {
            footprint_steady: None,
            footprint_peak: None,
            private_steady: Some(30_000_000),
            private_peak: Some(40_000_000),
            resident_steady: Some(60_000_000),
            resident_peak: Some(70_000_000),
            ..within()
        };
        assert!(passes(Summary { ..windows }, CPU_PERCENT));
        let heavy = Summary {
            private_steady: Some(30_000_000),
            resident_steady: Some(129_000_000),
            footprint_steady: None,
            footprint_peak: None,
            ..within()
        };
        assert!(!passes(heavy, CPU_PERCENT));
        let unmeasured = Summary {
            footprint_steady: None,
            footprint_peak: None,
            ..within()
        };
        assert!(!passes(unmeasured, CPU_PERCENT));
    }

    #[test]
    fn a_soak_cut_short_or_with_unequal_pixels_fails() {
        let mut cut = report(within());
        cut.ended = Some("quit".to_owned());
        assert!(!judge(&cut, CPU_PERCENT_APPLE).0);
        let mut unequal = report(within());
        unequal.pixels_equal = Some(false);
        assert!(!judge(&unequal, CPU_PERCENT_APPLE).0);
        let mut unmeasured = report(within());
        unmeasured.summary = None;
        assert!(!judge(&unmeasured, CPU_PERCENT_APPLE).0);
    }

    #[test]
    fn the_options_are_a_length_and_a_presenter() {
        let args =
            |list: &[&str]| options(&list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert_eq!(
            args(&[]),
            Ok(Options {
                minutes: 60,
                presenter: Presenter::Default
            })
        );
        assert_eq!(
            args(&["--minutes", "10", "--presenter", "both"]),
            Ok(Options {
                minutes: 10,
                presenter: Presenter::Both
            })
        );
        assert!(args(&["--minutes", "0"]).is_err());
        assert!(args(&["--presenter", "gl"]).is_err());
        assert!(args(&["--minutes"]).is_err());
        assert!(args(&["--night", "1"]).is_err());
    }

    #[test]
    fn megabytes_are_decimal() {
        assert_eq!(megabytes(80_000_000), "80.0 MB");
        assert_eq!(megabytes(41_960_000), "41.9 MB");
    }
}
