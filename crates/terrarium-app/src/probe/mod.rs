//! `--probe pinning`: the strip with a recorder attached. It walks the tester through the
//! manual checklist on stdout, reads each answer as a line from stdin, and rewrites a JSON and
//! a text report after every step. Piping lines into it drives it without a person.

pub mod report;
pub mod script;
pub mod tools;

use std::io;
use std::path::PathBuf;
use std::time::Instant;

use terrarium_platform::geometry::Edge;
use winit::event_loop::EventLoopProxy;

use crate::copy;
use crate::hover::NativePoint;
use crate::monitor::{MonitorId, MonitorInfo};
use crate::pinning::{LevelMode, TaskbarState};
use crate::strip::UserEvent;
use report::{EndReason, ProbeEvent, Report, StepReport, Timed};
use script::Step;

/// What the strip does after an answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Advance {
    /// The next step runs so.
    Next(Setup),
    /// The last step was answered and the report is final.
    Done,
}

/// How a step runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setup {
    pub level: LevelMode,
    pub edge: Edge,
    /// The tool checks run as the step opens.
    pub tools: bool,
}

/// What the probe learns once the strip is up.
pub struct Launch {
    pub os: String,
    pub separate_spaces: Option<bool>,
    pub monitors: Vec<MonitorInfo>,
    pub chosen: Option<MonitorId>,
    pub taskbar: Option<TaskbarState>,
}

pub struct Probe {
    started: Instant,
    steps: Vec<Step>,
    current: usize,
    frames_at_step_start: u64,
    report: Report,
    dir: PathBuf,
    /// Where the report goes if `dir` cannot be written.
    fallback: PathBuf,
    /// Tells apart runs started in the same second.
    run_id: u32,
}

impl Probe {
    /// Starts reading answers from stdin and prints where the report will be written: the
    /// directory the tester ran it from, where they will look for it.
    pub fn start(proxy: EventLoopProxy<UserEvent>) -> Self {
        read_answers(proxy);
        let started_unix = u64::try_from(crate::now_secs()).unwrap_or(0);
        let probe = Self::new(
            std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir()),
            std::env::temp_dir(),
            started_unix,
            std::process::id(),
        );
        println!(
            "{}",
            copy::probe_intro(&probe.json_path(), &probe.text_path())
        );
        probe
    }

    fn new(dir: PathBuf, fallback: PathBuf, started_unix: u64, run_id: u32) -> Self {
        let steps = script::pinning();
        let first = StepReport::new(&steps[0].id, steps[0].level);
        Self {
            started: Instant::now(),
            steps,
            current: 0,
            frames_at_step_start: 0,
            report: Report {
                probe: "pinning",
                version: env!("CARGO_PKG_VERSION"),
                os: String::new(),
                started_unix,
                separate_spaces: None,
                monitors: Vec::new(),
                chosen: None,
                taskbar: None,
                strip: None,
                steps: vec![first],
                ended: None,
            },
            dir,
            fallback,
            run_id,
        }
    }

    /// The level the current step runs at.
    pub fn level(&self) -> LevelMode {
        self.steps[self.current].level
    }

    /// Until the report is final.
    pub fn is_open(&self) -> bool {
        self.report.ended.is_none()
    }

    pub fn launched(&mut self, launch: Launch) {
        self.report.os = launch.os;
        self.report.separate_spaces = launch.separate_spaces;
        self.report.monitors = launch.monitors;
        self.report.chosen = launch.chosen;
        self.report.taskbar = launch.taskbar;
        println!("{}", copy::probe_header(&self.report));
        self.print_step();
        self.write();
    }

    /// Records `event` in the open step. A placement also becomes the report's strip, so the
    /// header shows the latest one.
    pub fn record(&mut self, event: ProbeEvent) {
        if let ProbeEvent::Placed { strip, .. } = &event {
            self.report.strip = Some(*strip);
        }
        let ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        println!("{}", copy::probe_echo(ms, &event));
        if let Some(step) = self.open_step() {
            step.push(Timed { ms, event });
        }
    }

    pub fn moved(&mut self, point: NativePoint, app_active: Option<bool>) {
        if let Some(step) = self.open_step() {
            step.moves.record(point, app_active);
        }
    }

    /// Closes the current step with the tester's note and opens the next one.
    pub fn answer(&mut self, note: &str, frames: u64) -> Advance {
        self.close_step(note, frames);
        self.current += 1;
        let Some(step) = self.steps.get(self.current) else {
            self.end(EndReason::Completed, frames);
            return Advance::Done;
        };
        let setup = Setup {
            level: step.level,
            edge: step.edge,
            tools: step.tools,
        };
        let id = step.id.clone();
        self.report.steps.push(StepReport::new(&id, setup.level));
        self.frames_at_step_start = frames;
        self.print_step();
        self.write();
        Advance::Next(setup)
    }

    /// Writes the final report once, whatever ended the run.
    pub fn end(&mut self, reason: EndReason, frames: u64) {
        if self.report.ended.is_some() {
            return;
        }
        if reason != EndReason::Completed {
            self.close_step("", frames);
        }
        self.report.ended = Some(reason);
        self.write();
        println!(
            "{}",
            copy::probe_written(&self.json_path(), &self.text_path())
        );
    }

    fn open_step(&mut self) -> Option<&mut StepReport> {
        if self.report.ended.is_some() {
            return None;
        }
        self.report.steps.last_mut()
    }

    fn close_step(&mut self, note: &str, frames: u64) {
        let since = self.frames_at_step_start;
        if let Some(step) = self.open_step() {
            if !note.trim().is_empty() {
                note.trim().clone_into(&mut step.note);
            }
            step.frames = frames.saturating_sub(since);
        }
    }

    fn print_step(&self) {
        let step = &self.steps[self.current];
        println!("{}", copy::probe_step(self.current, self.steps.len(), step));
    }

    fn file_name(&self, extension: &str) -> String {
        copy::probe_file(self.report.started_unix, self.run_id, extension)
    }

    fn json_path(&self) -> PathBuf {
        self.dir.join(self.file_name("json"))
    }

    fn text_path(&self) -> PathBuf {
        self.dir.join(self.file_name("txt"))
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
        let json = self.report.to_json().map_err(io::Error::other)?;
        std::fs::write(self.json_path(), json)?;
        std::fs::write(self.text_path(), copy::probe_report_text(&self.report))
    }
}

/// Each line on stdin answers a step; the end of input ends the probe.
fn read_answers(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        for line in io::stdin().lines() {
            let Ok(line) = line else { break };
            if proxy.send_event(UserEvent::ProbeLine(line)).is_err() {
                return;
            }
        }
        let _ = proxy.send_event(UserEvent::ProbeEnd);
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::pinning::Placed;
    use report::ScaleReport;

    fn probe(dir: &Path) -> Probe {
        Probe::new(dir.to_owned(), dir.to_owned(), 100, 7)
    }

    #[test]
    fn answers_advance_the_steps_and_count_frames_per_step() {
        let dir = tempfile::tempdir().unwrap();
        let mut probe = probe(dir.path());
        let setup = |step: &Step| Setup {
            level: step.level,
            edge: step.edge,
            tools: step.tools,
        };
        let second = setup(&probe.steps[1]);
        assert_eq!(probe.answer("first note", 5), Advance::Next(second));
        let third = setup(&probe.steps[2]);
        assert_eq!(probe.answer("  ", 12), Advance::Next(third));
        let steps = &probe.report.steps;
        assert_eq!(steps.len(), 3);
        assert_eq!((steps[0].note.as_str(), steps[0].frames), ("first note", 5));
        assert_eq!((steps[1].note.as_str(), steps[1].frames), ("", 7));
        assert_eq!(steps[2].id, probe.steps[2].id);
    }

    #[test]
    fn the_answer_says_each_step_s_edge_and_whether_its_tools_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut probe = probe(dir.path());
        probe.steps = script::windows_steps();
        probe.report.steps = vec![StepReport::new(&probe.steps[0].id, probe.steps[0].level)];
        let mut setups = Vec::new();
        while let Advance::Next(setup) = probe.answer("", 0) {
            setups.push((probe.steps[probe.current].id.clone(), setup));
        }
        let top: Vec<&str> = setups
            .iter()
            .filter(|(_, setup)| setup.edge == Edge::Top)
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(top, ["c.top-edge"]);
        let tools: Vec<&str> = setups
            .iter()
            .filter(|(_, setup)| setup.tools)
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(tools, ["c.tools"]);
    }

    #[test]
    fn the_last_answer_completes_the_report_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut probe = probe(dir.path());
        let total = probe.steps.len();
        for _ in 1..total {
            assert!(matches!(probe.answer("", 0), Advance::Next(_)));
        }
        assert_eq!(probe.answer("last", 3), Advance::Done);
        probe.end(EndReason::InputClosed, 9);
        assert_eq!(probe.report.ended, Some(EndReason::Completed));
        assert_eq!(probe.report.steps.len(), total);
        assert_eq!(probe.report.steps[total - 1].note, "last");
        let json = std::fs::read_to_string(probe.json_path()).expect("the report is written");
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(json["ended"], "completed");
    }

    #[test]
    fn ending_early_closes_the_open_step_and_stops_recording() {
        let dir = tempfile::tempdir().unwrap();
        let mut probe = probe(dir.path());
        probe.answer("", 2);
        assert!(probe.is_open());
        probe.end(EndReason::LaunchFailed, 10);
        assert!(!probe.is_open());
        probe.record(ProbeEvent::Focused { focused: true });
        let step = &probe.report.steps[1];
        assert_eq!(step.frames, 8);
        assert!(step.events.is_empty());
        assert_eq!(probe.report.ended, Some(EndReason::LaunchFailed));
    }

    #[test]
    fn an_unwritable_directory_falls_back_once() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing").join("nested");
        let mut probe = Probe::new(missing, dir.path().to_owned(), 100, 7);
        probe.end(EndReason::Quit, 0);
        assert_eq!(probe.dir, dir.path());
        assert!(probe.json_path().is_file());
        assert!(probe.text_path().is_file());
    }

    #[test]
    fn the_report_strip_follows_the_latest_placement() {
        let dir = tempfile::tempdir().unwrap();
        let mut probe = probe(dir.path());
        let placed = |scale: u32| ProbeEvent::Placed {
            monitor: MonitorId("M".to_owned()),
            strip: ScaleReport {
                scale,
                width: 2560,
                height: 74 * scale,
                image_x: 0,
            },
            frame: Box::new(Placed {
                x: 0.0,
                y: 0.0,
                width: 2560.0,
                height: f64::from(74 * scale),
                unit: "px",
                appbar: None,
            }),
        };
        assert_eq!(probe.report.strip, None);
        probe.record(placed(4));
        probe.record(placed(3));
        assert_eq!(probe.report.strip.map(|strip| strip.scale), Some(3));
    }

    #[test]
    fn runs_started_in_the_same_second_write_separate_files() {
        let dir = tempfile::tempdir().unwrap();
        let first = Probe::new(dir.path().to_owned(), dir.path().to_owned(), 100, 7);
        let second = Probe::new(dir.path().to_owned(), dir.path().to_owned(), 100, 8);
        assert_ne!(first.json_path(), second.json_path());
        assert_ne!(first.text_path(), second.text_path());
    }
}
