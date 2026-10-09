//! The pinning probe's words. The probe is a tester's tool, so its steps and its README give
//! instructions, in the same plain tone as the rest of `copy`: the scan for banned words leaves
//! this file out, and the scan for exclamation marks does not.

use std::fmt::{Display, Write as _};
use std::path::Path;

use super::{BINARY_NAME, VERSION};
use crate::hover::NativePoint;
use crate::pinning::{AppBarPlaced, LevelMode, OsSnapshot, PxRect};
use crate::probe::report::{Activity, EndReason, MoveSummary, ProbeEvent, Report, StepReport};
use crate::probe::script::{Phase, Step};

/// The key that ends input in the terminal the probe runs in.
#[cfg(windows)]
const END_OF_INPUT: &str = "Ctrl-Z then Enter";
#[cfg(not(windows))]
const END_OF_INPUT: &str = "Control-D";

/// One step of the probe's walk-through.
pub struct StepText {
    pub key: &'static str,
    pub text: &'static str,
}

#[cfg(target_os = "macos")]
pub const PROBE_LAUNCH: StepText = StepText {
    key: "launch",
    text: "The strip should now lie along the bottom edge of the display marked with * above, \
           sky over a band of soil, without any app having lost focus.",
};

#[cfg(windows)]
pub const PROBE_LAUNCH: StepText = StepText {
    key: "launch",
    text: "The strip should now lie along the bottom of the display marked with * above, just \
           above the taskbar if the taskbar is on that display: sky over a band of soil. No \
           app should have lost focus. If this run follows a taskkill, first note whether a \
           maximised window stopped short of the bottom before this run started.",
};

/// Run once at each level: phase A at desktop-icon + 1, phase B at always-on-bottom.
#[cfg(target_os = "macos")]
pub const PROBE_LEVEL_STEPS: &[StepText] = &[
    StepText {
        key: "hover-clicks",
        text: "Click a window of another app so that app is frontmost. Move the pointer onto \
               the strip and off again, left-click the strip, then right-click it and dismiss \
               the menu with Esc. Note whether the other app lost focus.",
    },
    StepText {
        key: "mission-control",
        text: "Open Mission Control and close it. Then use Show Desktop (spread thumb and three \
               fingers on the trackpad, or press F11) and undo it. Note where the strip was \
               each time.",
    },
    StepText {
        key: "stage-manager",
        text: "Turn Stage Manager on from Control Center and look at the strip, then turn Stage \
               Manager off again.",
    },
    StepText {
        key: "wallpaper-click",
        text: "In System Settings > Desktop & Dock, set \"Click wallpaper to reveal desktop\" to \
               Always. Click empty wallpaper, then click the strip. Put the setting back as it \
               was.",
    },
    StepText {
        key: "hide-others",
        text: "In another app, press Option-Command-H (Hide Others). Note whether the strip \
               stayed, then bring the hidden apps back.",
    },
];

/// Run once, at the default level.
#[cfg(target_os = "macos")]
pub const PROBE_ONCE_STEPS: &[StepText] = &[
    StepText {
        key: "new-space",
        text: "Open Mission Control, add a Space with the + button, and switch to it. Note \
               whether the strip is there, then switch back and remove that Space.",
    },
    StepText {
        key: "zoomed-window",
        text: "On the strip's display, zoom an ordinary window to fill the screen (Option-click \
               its green button). Note what covers what, then restore the window.",
    },
    StepText {
        key: "fullscreen-other-display",
        text: "With a second display: put an app in full screen on the display without the \
               strip and note whether the strip stays. With one display, press Enter.",
    },
    StepText {
        key: "dock",
        text: "With the Dock at the bottom of the strip's display, move the pointer to the very \
               bottom edge. Note whether the Dock appears over the soil band.",
    },
    StepText {
        key: "hotkey",
        text: "Press Control-Option-T. The strip should come above your windows for 10 seconds, \
               then go back below them. Note whether any app lost focus.",
    },
    StepText {
        key: "cmd-tab-dock",
        text: "Press Command-Tab, then look at the Dock. Note whether Repo Terrarium appears in \
               either.",
    },
    StepText {
        key: "idle-cpu",
        text: "Open Activity Monitor and find repo-terrarium. Keep the pointer off the strip for \
               30 seconds and note its % CPU. The report counts the frames drawn during this \
               step; a still strip draws none.",
    },
    StepText {
        key: "resolution",
        text: "In System Settings > Displays, change the resolution of the strip's display, then \
               change it back. Note whether the strip refitted both times.",
    },
    StepText {
        key: "sleep-wake",
        text: "Optional: put the Mac to sleep, wake it, and note whether the strip is in place. \
               To skip, press Enter.",
    },
    StepText {
        key: "external-display",
        text: "If an external display is connected, note which display the strip chose. \
               Otherwise press Enter.",
    },
    StepText {
        key: "separate-spaces",
        text: "The header shows this run's \"Displays have separate Spaces\" setting. The other \
               setting takes effect only after logging out, so it needs a second run of this \
               probe, which can be ended early with Control-D.",
    },
    StepText {
        key: "preferred-level",
        text: "Which level worked better: desktop-icon + 1 (phases A and C) or always-on-bottom \
               (phase B)? Type the choice and the reason.",
    },
];

/// Windows has one level, the topmost `AppBar`, so every step runs once. The last one ends the
/// process the way a crash does, so it must stay last.
#[cfg(windows)]
pub const PROBE_ONCE_STEPS: &[StepText] = &[
    StepText {
        key: "hover-clicks",
        text: "Click a window of another app so that app has focus. Move the pointer onto the \
               strip and off again, left-click the strip, then right-click it and dismiss the \
               menu with Esc. Note whether the other app lost focus after the left-click, and \
               after the right-click.",
    },
    StepText {
        key: "maximize",
        text: "On the strip's display, maximise an ordinary window. Note whether it stops at \
               the top of the strip or covers it, then restore the window.",
    },
    StepText {
        key: "alt-tab-taskbar",
        text: "Hold Alt and press Tab. Note whether Repo Terrarium is among the windows shown, \
               then release. Then look along the taskbar and note whether it has a Repo \
               Terrarium button.",
    },
    StepText {
        key: "win-d",
        text: "Press Win+D to show the desktop. Note whether the strip stays visible, then press \
               Win+D again to bring the windows back.",
    },
    StepText {
        key: "fullscreen",
        text: "On the strip's display, play a video full screen, or press F11 in a browser. \
               Note whether the strip covers the bottom of it, then leave full screen. This \
               build keeps the strip on top on purpose; the report records what Windows says.",
    },
    StepText {
        key: "display-scale",
        text: "In Settings > System > Display, change the scale of the strip's display (for \
               example from 100% to 125%), then change it back. Note whether the strip refitted \
               both times.",
    },
    StepText {
        key: "hotkey",
        text: "Press Ctrl+Alt+T. The strip is already above other windows, so nothing should \
               change. Note whether any app lost focus.",
    },
    StepText {
        key: "idle-cpu",
        text: "Open Task Manager and find repo-terrarium. Keep the pointer off the strip for 30 \
               seconds and note its CPU. The report counts the frames drawn during this step; a \
               still strip draws none.",
    },
    StepText {
        key: "second-display",
        text: "If a second display is connected, note which display the strip chose. \
               Otherwise press Enter.",
    },
    StepText {
        key: "taskkill",
        text: "Last step. Open a second terminal and run: taskkill /f /im repo-terrarium.exe \
               It ends this probe the way a crash would, so this report stops here. Then \
               maximise a window on the strip's display and see whether it stops short of the \
               bottom, and start the probe again as the README describes.",
    },
];

pub fn level_name(mode: LevelMode) -> &'static str {
    match mode {
        #[cfg(target_os = "macos")]
        LevelMode::DesktopIconPlusOne => "desktop-icon + 1",
        #[cfg(target_os = "macos")]
        LevelMode::AlwaysOnBottom => "always-on-bottom",
        #[cfg(windows)]
        LevelMode::TopmostAppBar => "topmost AppBar",
    }
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Launch => "launch",
        #[cfg(target_os = "macos")]
        Phase::A => "phase A",
        #[cfg(target_os = "macos")]
        Phase::B => "phase B",
        Phase::C => "phase C",
    }
}

pub fn probe_intro(json: &Path, text: &Path) -> String {
    format!(
        "{BINARY_NAME} {VERSION}: pinning probe\n\
         Each step describes something to try with the strip. Press Enter when it is done; to \
         record a note, type it first. {END_OF_INPUT} ends the probe early. The report is \
         rewritten after every step:\n  {}\n  {}",
        json.display(),
        text.display()
    )
}

pub fn probe_header(report: &Report) -> String {
    let mut out = format!("\nOS: {}", report.os);
    // A macOS setting; Windows has no Spaces, so it has no line.
    if report.separate_spaces.is_some() {
        let _ = write!(
            out,
            "\nDisplays have separate Spaces: {}",
            yes_no(report.separate_spaces)
        );
    }
    out.push_str("\nDisplays:");
    for monitor in &report.monitors {
        let chosen = if report.chosen.as_ref() == Some(&monitor.id) {
            "*"
        } else {
            " "
        };
        let primary = if monitor.primary { ", primary" } else { "" };
        let _ = write!(
            out,
            "\n {chosen} {} [{}]: {}x{} px, scale {}{primary}; {}",
            monitor.name,
            monitor.id.0,
            monitor.physical_width,
            monitor.physical_height,
            monitor.scale,
            monitor.detail
        );
    }
    if let Some(strip) = &report.strip {
        let _ = write!(
            out,
            "\nStrip: {}x scale, {}x{} px, image from x = {} px",
            strip.scale, strip.width, strip.height, strip.image_x
        );
    }
    out
}

pub fn probe_step(index: usize, total: usize, step: &Step) -> String {
    format!(
        "\n[{} of {}] {}, {}: {}\n{}\n(Enter when done; type a note first to record one.)",
        index + 1,
        total,
        phase_name(step.phase),
        level_name(step.level),
        step.id,
        step.text
    )
}

pub fn probe_echo(ms: u64, event: &ProbeEvent) -> String {
    format!("  {ms:>8} ms  {}", probe_event(event))
}

fn probe_event(event: &ProbeEvent) -> String {
    match event {
        ProbeEvent::Placed {
            monitor,
            strip,
            frame,
        } => {
            format!(
                "placed on [{}]: {}x scale, {}x{} px; frame at ({}, {}), {} x {} {}",
                monitor.0,
                strip.scale,
                strip.width,
                strip.height,
                frame.x,
                frame.y,
                frame.width,
                frame.height,
                frame.unit
            ) + &frame.appbar.as_ref().map_or_else(String::new, appbar_text)
        }
        ProbeEvent::Level { mode, value } => {
            format!("level {}: {}", level_name(*mode), level_value(*value))
        }
        ProbeEvent::Raised { value } => format!("raised: {}", level_value(*value)),
        ProbeEvent::Restored { mode, value } => {
            format!("restored to {}: {}", level_name(*mode), level_value(*value))
        }
        ProbeEvent::PointerEntered(activity) => {
            format!("pointer entered; {}", activity_text(activity))
        }
        ProbeEvent::PointerLeft(activity) => format!("pointer left; {}", activity_text(activity)),
        ProbeEvent::Click {
            button,
            point,
            activity,
        } => format!(
            "{button} click at {}; {}",
            point.map_or_else(|| "an unknown point".to_owned(), point_text),
            activity_text(activity)
        ),
        ProbeEvent::Focused { focused: true } => "the strip window became key".to_owned(),
        ProbeEvent::Focused { focused: false } => "the strip window is not key".to_owned(),
        ProbeEvent::AppActive { at, active } => {
            format!("at {at}, this app is {}", active_text(*active))
        }
        ProbeEvent::System { what } => format!("system: {what}"),
        ProbeEvent::ScaleFactor { factor } => format!("scale factor now {factor}"),
        ProbeEvent::Resized { width, height } => format!("resized to {width}x{height} px"),
        ProbeEvent::Occluded { occluded: true } => "fully covered".to_owned(),
        ProbeEvent::Occluded { occluded: false } => "visible again".to_owned(),
        ProbeEvent::HotkeyUnavailable { error } => format!("no raise hotkey: {error}"),
        ProbeEvent::Os(snapshot) => os_text(snapshot),
    }
}

pub fn rect_text(rect: &PxRect) -> String {
    format!(
        "({}, {})-({}, {})",
        rect.left, rect.top, rect.right, rect.bottom
    )
}

fn maybe_rect(rect: Option<&PxRect>) -> String {
    rect.map_or_else(|| "unknown".to_owned(), rect_text)
}

fn appbar_text(appbar: &AppBarPlaced) -> String {
    let mut out = format!(
        "\n            {}: monitor {}, work area {} before and {} after; proposed {}, queried {}, \
         granted {}, window {}",
        if appbar.reserved {
            "edge reserved"
        } else {
            "edge not reserved, a plain window"
        },
        rect_text(&appbar.monitor),
        rect_text(&appbar.work_before),
        maybe_rect(appbar.work_after.as_ref()),
        rect_text(&appbar.proposal),
        maybe_rect(appbar.queried.as_ref()),
        rect_text(&appbar.set),
        maybe_rect(appbar.window.as_ref()),
    );
    if appbar.unchanged {
        out.push_str("; unchanged, nothing sent");
    }
    if appbar.size_mismatch {
        out.push_str("; the granted band is not the strip's size");
    }
    if let Some(stale) = &appbar.stale {
        let _ = write!(
            out,
            "\n            old reservation: {} ({} taskbars explain a work area of {}){}",
            if stale.suspected { "suspected" } else { "none" },
            stale.taskbars,
            rect_text(&stale.expected),
            if stale.other_instance {
                "; another strip is running, so nothing was cleared".to_owned()
            } else if stale.cleared {
                format!(
                    "; cleared, work area now {}",
                    maybe_rect(stale.work_after.as_ref())
                )
            } else {
                String::new()
            }
        );
    }
    out
}

fn os_text(snapshot: &OsSnapshot) -> String {
    let flag = |on: bool, name: &str| format!("{}{name}", if on { "" } else { "not " });
    let mut out = format!(
        "window: {}, {}, {}, {}, {}; work area {}",
        flag(snapshot.tool_window, "tool window"),
        flag(snapshot.no_activate, "no-activate"),
        flag(snapshot.app_window, "app window"),
        flag(snapshot.topmost, "topmost"),
        flag(snapshot.visible, "visible"),
        maybe_rect(snapshot.work_area.as_ref())
    );
    if !snapshot.messages.is_empty() {
        out.push_str("\n            messages (subclass/hook):");
        for count in &snapshot.messages {
            let _ = write!(out, " {} {}/{}", count.name, count.subclass, count.hook);
        }
    }
    out
}

fn level_value(value: Option<isize>) -> String {
    value.map_or_else(
        || "window level unknown".to_owned(),
        |v| format!("window level {v}"),
    )
}

fn active_text(active: Option<bool>) -> &'static str {
    match active {
        Some(true) => "active",
        Some(false) => "inactive",
        None => "of unknown activity",
    }
}

fn activity_text(activity: &Activity) -> String {
    format!(
        "this app {}, frontmost: {}",
        active_text(activity.app_active),
        activity.frontmost.as_deref().unwrap_or("unknown")
    )
}

fn point_text(point: NativePoint) -> String {
    match point {
        NativePoint::Scene { x, y } => format!("native ({x}, {y})"),
        NativePoint::Padding => "beside the scene".to_owned(),
    }
}

fn yes_no(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

/// A display's id in the report when macOS gives it no UUID.
#[cfg(target_os = "macos")]
pub fn display_id(number: u32) -> String {
    format!("display-{number}")
}

#[cfg(target_os = "macos")]
pub fn display_detail(
    number: u32,
    frame: &impl std::fmt::Debug,
    visible: &impl std::fmt::Debug,
) -> String {
    format!("display {number}, frame {frame:?}, visible {visible:?} (points)")
}

/// A monitor's id in the report when Windows gives it no name.
#[cfg(windows)]
pub fn monitor_id(index: usize) -> String {
    format!("monitor-{index}")
}

#[cfg(windows)]
pub fn monitor_detail(monitor: &PxRect, work: &PxRect) -> String {
    format!(
        "monitor {}, work area {} (px)",
        rect_text(monitor),
        rect_text(work)
    )
}

/// The unit a placement is reported in: points on macOS, pixels on Windows.
#[cfg(target_os = "macos")]
pub const PLACED_UNIT: &str = "pt";
#[cfg(windows)]
pub const PLACED_UNIT: &str = "px";

#[cfg(target_os = "macos")]
pub fn os_version(version: &str) -> String {
    format!("macOS {version}")
}

/// Windows' version as `major.minor.build`, or the bare name when it could not be read.
#[cfg(windows)]
pub fn os_version(version: Option<(u32, u32, u32)>) -> String {
    version.map_or_else(
        || "Windows".to_owned(),
        |(major, minor, build)| format!("Windows {major}.{minor}.{build}"),
    )
}

/// Where in the run the app's activation was read.
pub const AT_WINDOW_SHOWN: &str = "window shown";
pub const AT_FIRST_IDLE: &str = "first idle";

/// The report's file: the run's start and id tell two runs apart in one directory.
pub fn probe_file(started_unix: u64, run_id: u32, extension: &str) -> String {
    format!("{BINARY_NAME}-probe-pinning-{started_unix}-{run_id}.{extension}")
}

pub fn probe_written(json: &Path, text: &Path) -> String {
    format!(
        "\nThe report is written:\n  {}\n  {}",
        json.display(),
        text.display()
    )
}

pub fn probe_write_failed(dir: &Path, err: &impl Display) -> String {
    format!(
        "{BINARY_NAME}: the probe report could not be written to {}: {err}",
        dir.display()
    )
}

fn end_reason(reason: EndReason) -> &'static str {
    match reason {
        EndReason::Completed => "every step answered",
        EndReason::Quit => "quit from the menu",
        EndReason::InputClosed => "input closed",
        EndReason::WindowClosed => "window closed",
        EndReason::LaunchFailed => "the strip could not open",
    }
}

/// The report for people: the header, then each step with its note, counts and events.
pub fn probe_report_text(report: &Report) -> String {
    let mut out = format!(
        "{BINARY_NAME} {} pinning probe, started at Unix time {}\n",
        report.version, report.started_unix
    );
    out.push_str(probe_header(report).trim_start());
    let _ = write!(
        out,
        "\nEnded: {}\n",
        report.ended.map_or("still running", end_reason)
    );
    for step in &report.steps {
        out.push_str(&step_text(step));
    }
    out
}

fn step_text(step: &StepReport) -> String {
    let mut out = format!(
        "\n{} ({})\n  note: {}\n  frames drawn: {}\n  pointer moves: {}\n",
        step.id,
        level_name(step.level),
        if step.note.is_empty() {
            "(none)"
        } else {
            &step.note
        },
        step.frames,
        moves_text(&step.moves)
    );
    for timed in &step.events {
        let _ = writeln!(out, "{}", probe_echo(timed.ms, &timed.event));
    }
    if step.events_dropped > 0 {
        let _ = writeln!(out, "  and {} more events", step.events_dropped);
    }
    out
}

fn moves_text(moves: &MoveSummary) -> String {
    let total = moves.while_active + moves.while_inactive + moves.while_unknown;
    if total == 0 {
        return "none".to_owned();
    }
    format!(
        "{total} ({} while active, {} while inactive, {} unknown), from {} to {}",
        moves.while_active,
        moves.while_inactive,
        moves.while_unknown,
        moves.first.map_or_else(String::new, point_text),
        moves.last.map_or_else(String::new, point_text)
    )
}

/// The Windows probe's instructions for testers, which `cargo xtask probe-zip` puts in the zip
/// as `README.txt` (`repo-terrarium --probe-readme` prints it). Plain text, wrapped at 80.
pub const PROBE_README: &str = "Repo Terrarium: the Windows pinning probe
==========================================

Thank you for testing. Repo Terrarium is a thin pixel-art strip that sits along
the bottom edge of a monitor. On Windows it reserves that edge, the way the
taskbar does, so maximised windows stop above it. Nobody on this project has a
Windows machine, so this probe is how we find out whether that works.

It takes about ten minutes. It changes nothing on your machine: there is no
installer, and it writes only its report files, into this folder.


Before you start
----------------

1. Extract the zip first (right-click it, then Extract All). Running it from
   inside the zip does not work.
2. Windows may say \"Windows protected your PC\", because the app is not signed.
   Choose \"More info\", then \"Run anyway\".
3. In this build the strip stays on top of full-screen video on its display.
   That is expected; the probe measures it.


Run 1: the walk-through
-----------------------

Double-click run-probe.cmd. A console window opens and the strip appears.

The console describes one thing to try at a time. Try it, then come back to the
console: type a short note if something is worth saying, and press Enter to go
on. Enter alone is fine too.

The last step asks you to end the probe from a second terminal with:

    taskkill /f /im repo-terrarium.exe

That ends it the way a crash would, on purpose. Then maximise a window on the
same display and look whether it stops short of the bottom, as if the strip
were still there.


Run 2 and run 3: two short checks
---------------------------------

Run 2. Double-click run-probe.cmd again. At the first step, type a note saying
whether the maximised window stopped short of the bottom before this run, then
look at the strip and press Enter. Now press Ctrl-C in the console. The probe
ends at once, and the console asks \"Terminate batch job (Y/N)?\": type Y and
press Enter.

Run 3. Double-click run-probe.cmd once more. At the first step, type a note
saying whether a maximised window still stopped short of the bottom after run 2,
then press Enter. End this run with Ctrl-Z and then Enter.


Sending the results
-------------------

Each run writes two files into this folder, named
repo-terrarium-probe-pinning-<numbers>.json and .txt. Please send all of the
repo-terrarium-probe- files back. They hold the display sizes, what Windows
reported about the strip, the names of window classes (never window titles),
and your notes. Nothing else.

If the strip ever gets stuck on screen, or the bottom of the screen stays
reserved after the probe has ended, signing out and back in clears it.
";
