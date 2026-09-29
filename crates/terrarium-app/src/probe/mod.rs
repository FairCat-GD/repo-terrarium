//! `--probe pinning`: the strip with a recorder attached. It walks the tester through the
//! manual checklist on stdout, reads each answer as a line from stdin, and rewrites a JSON and
//! a text report after every step. Piping lines into it drives it without a person.

pub mod report;
pub mod script;

use std::io;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use terrarium_compose::scale::StripScale;
use winit::event_loop::EventLoopProxy;

use crate::copy;
use crate::hover::NativePoint;
use crate::monitor::{MonitorId, MonitorInfo};
use crate::pinning::LevelMode;
use crate::strip::UserEvent;
use report::{EndReason, ProbeEvent, Report, ScaleReport, StepReport, Timed};
use script::Step;

/// What the strip does after an answer.
pub enum Advance {
    /// The next step runs at this level.
    Next(LevelMode),
    /// The last step was answered and the report is final.
    Done,
}

/// What the probe learns once the strip is up.
pub struct Launch {
    pub os: String,
    pub separate_spaces: Option<bool>,
    pub monitors: Vec<MonitorInfo>,
    pub chosen: Option<MonitorId>,
    pub scale: Option<StripScale>,
}

pub struct Probe {
    started: Instant,
    steps: Vec<Step>,
    current: usize,
    frames_at_step_start: u64,
    report: Report,
    dir: PathBuf,
}

impl Probe {
    /// Starts reading answers from stdin and prints where the report will be written.
    pub fn start(proxy: EventLoopProxy<UserEvent>) -> Self {
        read_answers(proxy);
        let started_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let steps = script::pinning();
        let first = StepReport::new(&steps[0].id, steps[0].level);
        let probe = Self {
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
                strip: None,
                steps: vec![first],
                ended: None,
            },
            // The directory the tester ran it from, where they will look for it.
            dir: std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir()),
        };
        println!(
            "{}",
            copy::probe_intro(&probe.json_path(), &probe.text_path())
        );
        probe
    }

    /// The level the current step runs at.
    pub fn level(&self) -> LevelMode {
        self.steps[self.current].level
    }

    pub fn launched(&mut self, launch: Launch) {
        self.report.os = launch.os;
        self.report.separate_spaces = launch.separate_spaces;
        self.report.monitors = launch.monitors;
        self.report.chosen = launch.chosen;
        self.report.strip = launch.scale.as_ref().map(ScaleReport::from);
        println!("{}", copy::probe_header(&self.report));
        self.print_step();
        self.write();
    }

    pub fn record(&mut self, event: ProbeEvent) {
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
        let (id, level) = (step.id.clone(), step.level);
        self.report.steps.push(StepReport::new(&id, level));
        self.frames_at_step_start = frames;
        self.print_step();
        self.write();
        Advance::Next(level)
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
        format!(
            "repo-terrarium-probe-pinning-{}.{extension}",
            self.report.started_unix
        )
    }

    fn json_path(&self) -> PathBuf {
        self.dir.join(self.file_name("json"))
    }

    fn text_path(&self) -> PathBuf {
        self.dir.join(self.file_name("txt"))
    }

    /// Falls back to the temp directory once if the current one is not writable.
    fn write(&mut self) {
        let Err(err) = self.write_files() else {
            return;
        };
        eprintln!("{}", copy::probe_write_failed(&self.dir, &err));
        let temp = std::env::temp_dir();
        if self.dir != temp {
            self.dir = temp;
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
