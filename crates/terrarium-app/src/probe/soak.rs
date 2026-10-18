//! `--probe soak`: the strip as it runs every day, measured for a while (N-01..N-03). Every ten seconds it reads what the OS counts against the process (its processor
//! time, and its memory as each OS counts it fairly) beside what the strip counts itself: frames
//! by cause, compositions, the loop's wakes and the time spent covered. It writes a JSON report
//! for `cargo xtask soak` to judge, and the same in words, every five minutes and at the end;
//! neither holds a path.
//!
//! The strip runs on the repository `--repo` names, connected in a data directory the soak
//! owns and deletes, so a person's own terrarium is never touched; without one, on the
//! thriving sample. Dim mode is held off: a dimmed strip moves a quarter as much, so the bright
//! strip is the cost to measure.

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use serde::Serialize;
use terrarium_compose::scale::StripScale;
use terrarium_platform::geometry::Edge;
use terrarium_platform::memory::{self, Memory};
use terrarium_store::DataDir;

use crate::cli::SoakArgs;
use crate::copy;
use crate::instance::{Claim, Instance};
use crate::live::Launch;
use crate::present::Backend;
use crate::probe::report::ScaleReport;
use crate::strip::Mode;

/// The probe's name, in its report and its files' names.
const KIND: &str = "soak";

/// How often the soak reads the OS's counts.
const SAMPLE_EVERY: Duration = Duration::from_secs(10);
/// How often the report is rewritten, so a run cut short keeps most of what it measured.
const WRITE_EVERY: Duration = Duration::from_mins(5);
/// The start the averages leave out: connecting, the first replay, the caches filling. A soak
/// shorter than three of these leaves out its first third instead.
const WARM_UP: Duration = Duration::from_mins(5);

/// How a soak runs, as the command line asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setup {
    pub minutes: u32,
    pub presenter: Option<Backend>,
    pub edge: Option<Edge>,
}

/// Runs the soak `args` ask for, and ends the process when it has.
pub fn run(args: SoakArgs) -> ExitCode {
    let setup = Setup {
        minutes: args.minutes,
        presenter: args.presenter,
        edge: args.edge,
    };
    let Some(repo) = args.repo else {
        return crate::run(&Mode::Soak(setup), None);
    };
    // Deleted when the soak ends, with the terrarium it held.
    let dir = match tempfile::Builder::new()
        .prefix("repo-terrarium-soak-")
        .tempdir()
    {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("{}", copy::soak_data_unavailable(&err));
            return ExitCode::FAILURE;
        }
    };
    let data = DataDir::at(dir.path());
    let instance = match Instance::claim(&data) {
        Ok(Claim::First(instance)) => instance,
        Ok(Claim::Running) => {
            eprintln!(
                "{}",
                copy::soak_data_unavailable(&io::Error::other("locked"))
            );
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("{}", copy::soak_data_unavailable(&err));
            return ExitCode::FAILURE;
        }
    };
    crate::run(
        &Mode::Soak(setup),
        Some(Launch {
            data,
            repo: Some(repo),
            instance,
        }),
    )
}

/// What the strip shows while it is measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SoakScene {
    /// The repository `--repo` named, through the engine.
    Repository,
    /// The thriving sample, with no repository and no engine.
    Sample,
}

/// Why the soak ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SoakEnd {
    /// It ran its length.
    Done,
    /// The app was quit, or its window closed, first.
    Quit,
}

/// What the strip counts of itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Frames presented: for a change of pixels, and again unchanged.
    pub frames: (u64, u64),
    pub busiest_second: u32,
    pub compositions: u64,
}

/// One reading, `ms` after the soak started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Sample {
    pub ms: u64,
    /// Processor time used since the process started.
    pub cpu_ms: Option<u64>,
    pub resident: Option<u64>,
    /// macOS.
    pub footprint: Option<u64>,
    /// Windows.
    pub private: Option<u64>,
    pub frames_changed: u64,
    pub frames_again: u64,
    pub compositions: u64,
    pub wakes: u64,
    /// Time spent fully covered since the soak started.
    pub covered_ms: u64,
}

/// What the soak measured, past its warm-up.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    /// The span the averages cover: from the end of the warm-up to the last sample.
    pub measured_secs: u64,
    pub warm_up_secs: u64,
    /// Processor time over wall time across the span, as a percentage of one core.
    pub cpu_percent: Option<f64>,
    /// Processor time used by the end of the warm-up, connecting included.
    pub warm_up_cpu_secs: Option<f64>,
    /// The most each measure held at any sample across the span.
    pub resident_steady: Option<u64>,
    pub footprint_steady: Option<u64>,
    pub private_steady: Option<u64>,
    /// The most each was ever, as the OS counts it over the process's life.
    pub resident_peak: Option<u64>,
    pub footprint_peak: Option<u64>,
    pub private_peak: Option<u64>,
    /// Frames presented across the span: changed, and the same again because the OS or the
    /// strip's own move asked.
    pub frames_changed: u64,
    pub frames_again: u64,
    /// Changed frames a second.
    pub frames_per_second: f64,
    /// The most changed frames presented in any one second of the whole run.
    pub busiest_second: u32,
    pub compositions_per_second: f64,
    pub wakes_per_minute: f64,
    /// The span's share spent fully covered, as a percentage.
    pub covered_percent: f64,
}

#[derive(Debug, Serialize)]
pub struct SoakReport {
    pub probe: &'static str,
    pub version: &'static str,
    pub os: String,
    pub started_unix: u64,
    pub minutes: u32,
    pub scene: SoakScene,
    /// The presenter the strip used, once its window opened.
    pub presenter: Option<&'static str>,
    pub strip: Option<ScaleReport>,
    /// Whether the GPU's pixels equal the CPU's for the first frame (wgpu only).
    pub pixels_equal: Option<bool>,
    pub samples: Vec<Sample>,
    pub summary: Option<Summary>,
    pub ended: Option<SoakEnd>,
}

pub struct Soak {
    started: Instant,
    length: Duration,
    warm_up: Duration,
    next_sample: Instant,
    next_write: Instant,
    dir: PathBuf,
    fallback: PathBuf,
    run_id: u32,
    covered: Duration,
    covered_since: Option<Instant>,
    wakes: u64,
    pixels_checked: bool,
    report: SoakReport,
}

impl Soak {
    /// Starts measuring at `now`, and prints where the report will be written: the directory
    /// the soak was run from.
    pub fn start(setup: Setup, scene: SoakScene, now: Instant) -> Soak {
        let length = Duration::from_secs(u64::from(setup.minutes) * 60);
        let soak = Soak {
            started: now,
            length,
            warm_up: WARM_UP.min(length / 3),
            next_sample: now + SAMPLE_EVERY,
            next_write: now + WRITE_EVERY,
            dir: std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir()),
            fallback: std::env::temp_dir(),
            run_id: std::process::id(),
            covered: Duration::ZERO,
            covered_since: None,
            wakes: 0,
            pixels_checked: false,
            report: SoakReport {
                probe: KIND,
                version: env!("CARGO_PKG_VERSION"),
                os: crate::pinning::os_version(),
                started_unix: u64::try_from(crate::now_secs()).unwrap_or(0),
                minutes: setup.minutes,
                scene,
                presenter: None,
                strip: None,
                pixels_equal: None,
                samples: Vec::new(),
                summary: None,
                ended: None,
            },
        };
        println!(
            "{}",
            copy::soak_intro(setup.minutes, &soak.json_path(), &soak.text_path())
        );
        soak
    }

    pub fn woke(&mut self) {
        self.wakes += 1;
    }

    /// The strip was covered or uncovered at `now`.
    pub fn covered(&mut self, covered: bool, now: Instant) {
        match (covered, self.covered_since) {
            (true, None) => self.covered_since = Some(now),
            (false, Some(since)) => {
                self.covered += now.saturating_duration_since(since);
                self.covered_since = None;
            }
            _ => {}
        }
    }

    /// The strip's window was placed at `scale`, shown through `backend`.
    pub fn placed(&mut self, scale: &StripScale, backend: Backend) {
        self.report.strip = Some(ScaleReport::from(scale));
        self.report.presenter = Some(copy::presenter_name(backend));
    }

    /// Whether the presenter's pixels are still to be checked: once, at the first frame.
    pub fn wants_pixels(&self) -> bool {
        !self.pixels_checked
    }

    /// Whether the presenter's pixels equal the CPU upscale's: none where it cannot say.
    pub fn pixels(&mut self, equal: Option<bool>) {
        self.pixels_checked = true;
        self.report.pixels_equal = equal;
    }

    /// When the soak next wants the loop awake; none once it has ended.
    pub fn due(&self) -> Option<Instant> {
        self.report
            .ended
            .is_none()
            .then(|| self.next_sample.min(self.started + self.length))
    }

    /// The loop woke at `now`, with the strip counting `counts`: a sample if one is due. True
    /// once the soak has run its length and its report is final.
    pub fn sample(&mut self, now: Instant, counts: Counts) -> bool {
        if self.report.ended.is_some() {
            return true;
        }
        let end = self.started + self.length;
        if now < self.next_sample && now < end {
            return false;
        }
        self.read(now, counts);
        while self.next_sample <= now {
            self.next_sample += SAMPLE_EVERY;
        }
        if now >= end {
            self.finish(counts, SoakEnd::Done);
            return true;
        }
        if now >= self.next_write {
            self.next_write = now + WRITE_EVERY;
            self.write();
        }
        false
    }

    /// The app is ending at `now`: the report is made final, unless the soak already ended.
    pub fn end(&mut self, now: Instant, counts: Counts) {
        if self.report.ended.is_some() {
            return;
        }
        self.read(now, counts);
        self.finish(counts, SoakEnd::Quit);
    }

    fn read(&mut self, now: Instant, counts: Counts) {
        let millis = |duration: Duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        let memory = memory::usage();
        let covered = self.covered
            + self
                .covered_since
                .map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
        self.report.samples.push(Sample {
            ms: millis(now.saturating_duration_since(self.started)),
            cpu_ms: terrarium_platform::cpu::time().map(millis),
            resident: memory.map(|memory| memory.resident),
            footprint: memory.and_then(|memory| memory.footprint),
            private: memory.and_then(|memory| memory.private),
            frames_changed: counts.frames.0,
            frames_again: counts.frames.1,
            compositions: counts.compositions,
            wakes: self.wakes,
            covered_ms: millis(covered),
        });
    }

    fn finish(&mut self, counts: Counts, ended: SoakEnd) {
        self.report.summary = summarize(
            &self.report.samples,
            self.warm_up,
            memory::usage(),
            counts.busiest_second,
        );
        self.report.ended = Some(ended);
        self.write();
        println!(
            "{}",
            copy::probe_written(&self.json_path(), &self.text_path())
        );
    }

    fn json_path(&self) -> PathBuf {
        self.dir.join(self.file_name("json"))
    }

    fn text_path(&self) -> PathBuf {
        self.dir.join(self.file_name("txt"))
    }

    fn file_name(&self, extension: &str) -> String {
        copy::probe_file(KIND, self.report.started_unix, self.run_id, extension)
    }

    /// Moves to the fallback directory once if the current one cannot be written.
    fn write(&mut self) {
        let Err(err) = self.write_files() else {
            return;
        };
        eprintln!("{}", copy::probe_write_failed(&self.dir, &err));
        if self.dir != self.fallback {
            self.dir = self.fallback.clone();
            if let Err(err) = self.write_files() {
                eprintln!("{}", copy::probe_write_failed(&self.dir, &err));
            }
        }
    }

    fn write_files(&self) -> io::Result<()> {
        let json = serde_json::to_string_pretty(&self.report).map_err(io::Error::other)?;
        std::fs::write(self.json_path(), json)?;
        std::fs::write(self.text_path(), copy::soak_report_text(&self.report))
    }
}

/// What `samples` say past `warm_up`: the averages from the first sample at or after it to the
/// last, the steady maxima over the same samples, and the peaks from `last`, the OS's counts at
/// the end. None with fewer than two samples to measure between.
#[expect(
    clippy::cast_precision_loss,
    reason = "counts and milliseconds of a day-long soak at most, far inside f64's exact range"
)]
pub fn summarize(
    samples: &[Sample],
    warm_up: Duration,
    last: Option<Memory>,
    busiest_second: u32,
) -> Option<Summary> {
    let warm_up_ms = u64::try_from(warm_up.as_millis()).unwrap_or(u64::MAX);
    let start = samples
        .iter()
        .position(|sample| sample.ms >= warm_up_ms)
        .unwrap_or(0);
    let span = &samples[start..];
    let (first, end) = (span.first()?, span.last()?);
    let ms = end.ms.checked_sub(first.ms).filter(|&ms| ms > 0)?;
    let secs = ms as f64 / 1000.0;
    let steady = |measure: fn(&Sample) -> Option<u64>| span.iter().filter_map(measure).max();
    let cpu = first.cpu_ms.zip(end.cpu_ms);
    let frames_changed = end.frames_changed.saturating_sub(first.frames_changed);
    let frames_again = end.frames_again.saturating_sub(first.frames_again);
    Some(Summary {
        measured_secs: ms / 1000,
        warm_up_secs: first.ms / 1000,
        cpu_percent: cpu.map(|(from, to)| to.saturating_sub(from) as f64 * 100.0 / ms as f64),
        warm_up_cpu_secs: first.cpu_ms.map(|cpu| cpu as f64 / 1000.0),
        resident_steady: steady(|sample| sample.resident),
        footprint_steady: steady(|sample| sample.footprint),
        private_steady: steady(|sample| sample.private),
        resident_peak: last.and_then(|memory| memory.resident_peak),
        footprint_peak: last.and_then(|memory| memory.footprint_peak),
        private_peak: last.and_then(|memory| memory.private_peak),
        frames_changed,
        frames_again,
        frames_per_second: frames_changed as f64 / secs,
        busiest_second,
        compositions_per_second: end.compositions.saturating_sub(first.compositions) as f64 / secs,
        wakes_per_minute: end.wakes.saturating_sub(first.wakes) as f64 * 60.0 / secs,
        covered_percent: end.covered_ms.saturating_sub(first.covered_ms) as f64 * 100.0 / ms as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(secs: u64, cpu_ms: u64, footprint: u64) -> Sample {
        Sample {
            ms: secs * 1000,
            cpu_ms: Some(cpu_ms),
            resident: Some(footprint + 10),
            footprint: Some(footprint),
            private: None,
            frames_changed: secs * 10,
            frames_again: 2,
            compositions: secs * 12,
            wakes: secs * 20,
            covered_ms: if secs > 400 { (secs - 400) * 1000 } else { 0 },
        }
    }

    #[test]
    fn the_averages_leave_the_warm_up_out_and_the_peaks_come_from_the_os() {
        // Connecting costs 20 s of processor time; then 3 ms a second, 0.3% of one core.
        let samples: Vec<Sample> = (1..=60)
            .map(|i| {
                let secs = i * 10;
                let cpu = 20_000 + secs * 3;
                // Memory settles after the warm-up's spike.
                let footprint = if secs < 300 { 90 << 20 } else { 40 << 20 };
                sample(secs, cpu, footprint)
            })
            .collect();
        let last = Memory {
            resident: 50 << 20,
            resident_peak: None,
            footprint: Some(40 << 20),
            footprint_peak: Some(95 << 20),
            private: None,
            private_peak: None,
        };
        let summary = summarize(&samples, WARM_UP, Some(last), 14).expect("a span");
        assert_eq!(summary.warm_up_secs, 300);
        assert_eq!(summary.measured_secs, 300);
        let cpu = summary.cpu_percent.expect("measured");
        assert!((cpu - 0.3).abs() < 1e-9, "{cpu}");
        assert_eq!(summary.warm_up_cpu_secs, Some(20.9));
        assert_eq!(summary.footprint_steady, Some(40 << 20));
        assert_eq!(summary.footprint_peak, Some(95 << 20));
        assert_eq!(summary.private_steady, None);
        assert_eq!(summary.frames_changed, 3000);
        assert_eq!(summary.frames_again, 0);
        assert!((summary.frames_per_second - 10.0).abs() < 1e-9);
        assert!((summary.compositions_per_second - 12.0).abs() < 1e-9);
        assert!((summary.wakes_per_minute - 1200.0).abs() < 1e-9);
        assert_eq!(summary.busiest_second, 14);
        // Covered from 400 s on: 200 of the 300 measured.
        assert!((summary.covered_percent - 200.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_soak_shorter_than_its_warm_up_measures_from_its_first_sample() {
        let samples = [sample(10, 100, 1), sample(20, 200, 2)];
        let summary = summarize(&samples, WARM_UP, None, 3).expect("a span");
        assert_eq!((summary.warm_up_secs, summary.measured_secs), (10, 10));
        assert_eq!(summary.footprint_peak, None);
        assert_eq!(summary.cpu_percent, Some(1.0));
    }

    #[test]
    fn one_sample_measures_nothing() {
        assert_eq!(summarize(&[sample(10, 1, 1)], WARM_UP, None, 0), None);
        assert_eq!(summarize(&[], WARM_UP, None, 0), None);
    }

    #[test]
    fn a_soak_samples_on_time_and_ends_at_its_length() {
        let start = Instant::now();
        let mut soak = Soak {
            started: start,
            length: Duration::from_secs(60),
            warm_up: Duration::from_secs(20),
            next_sample: start + SAMPLE_EVERY,
            next_write: start + WRITE_EVERY,
            dir: std::env::temp_dir(),
            fallback: std::env::temp_dir(),
            run_id: 0,
            covered: Duration::ZERO,
            covered_since: None,
            wakes: 0,
            pixels_checked: false,
            report: SoakReport {
                probe: KIND,
                version: env!("CARGO_PKG_VERSION"),
                os: String::new(),
                started_unix: 0,
                minutes: 1,
                scene: SoakScene::Sample,
                presenter: None,
                strip: None,
                pixels_equal: None,
                samples: Vec::new(),
                summary: None,
                ended: None,
            },
        };
        let at = |secs| start + Duration::from_secs(secs);
        assert_eq!(soak.due(), Some(at(10)));
        assert!(!soak.sample(at(5), Counts::default()));
        assert!(soak.report.samples.is_empty());
        soak.covered(true, at(8));
        assert!(!soak.sample(at(12), Counts::default()));
        assert_eq!(soak.due(), Some(at(20)));
        soak.covered(false, at(18));
        assert!(!soak.sample(at(31), Counts::default()));
        // Late wakes keep the grid: the next is at 40, not 41.
        assert_eq!(soak.due(), Some(at(40)));
        let covered: Vec<u64> = soak.report.samples.iter().map(|s| s.covered_ms).collect();
        assert_eq!(covered, [4_000, 10_000]);
        assert!(!soak.sample(at(50), Counts::default()));
        // Its length reached, it ends, and wants nothing more.
        let dir = tempfile::tempdir().expect("a temporary directory");
        soak.dir = dir.path().to_owned();
        soak.fallback = dir.path().to_owned();
        assert!(soak.sample(at(60), Counts::default()));
        assert_eq!(soak.report.ended, Some(SoakEnd::Done));
        assert_eq!(soak.due(), None);
        assert!(soak.report.summary.is_some());
        assert!(soak.json_path().exists() && soak.text_path().exists());
        // A quit afterwards changes nothing.
        soak.end(at(70), Counts::default());
        assert_eq!(soak.report.samples.len(), 4);
        assert_eq!(soak.report.ended, Some(SoakEnd::Done));
    }
}
