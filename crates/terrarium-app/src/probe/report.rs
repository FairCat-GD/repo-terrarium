//! What the probe writes: JSON for tools and text for people (the text is built in the copy
//! module). It is rewritten after every step, so an interrupted run keeps what it saw.

use serde::Serialize;
use terrarium_compose::scale::StripScale;

use crate::hover::NativePoint;
use crate::monitor::{MonitorId, MonitorInfo};
use crate::pinning::{
    FullscreenReport, LevelMode, OsSnapshot, Placed, TaskbarReport, TaskbarState,
};
use crate::probe::tools::ToolsReport;

/// Events kept per step; a step that produces more counts the rest as dropped.
pub const EVENTS_PER_STEP: usize = 200;

#[derive(Debug, Serialize)]
pub struct Report {
    pub probe: &'static str,
    pub version: &'static str,
    pub os: String,
    pub started_unix: u64,
    pub separate_spaces: Option<bool>,
    pub monitors: Vec<MonitorInfo>,
    pub chosen: Option<MonitorId>,
    /// The taskbar at launch (Windows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taskbar: Option<TaskbarState>,
    pub strip: Option<ScaleReport>,
    pub steps: Vec<StepReport>,
    pub ended: Option<EndReason>,
}

impl Report {
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ScaleReport {
    pub scale: u32,
    pub width: u32,
    pub height: u32,
    pub image_x: u32,
}

impl From<&StripScale> for ScaleReport {
    fn from(scale: &StripScale) -> Self {
        Self {
            scale: scale.scale,
            width: scale.width,
            height: scale.height,
            image_x: scale.image_x,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StepReport {
    pub id: String,
    pub level: LevelMode,
    pub note: String,
    /// Frames presented while this step was open.
    pub frames: u64,
    pub moves: MoveSummary,
    pub events: Vec<Timed>,
    pub events_dropped: u64,
}

impl StepReport {
    pub fn new(id: &str, level: LevelMode) -> Self {
        Self {
            id: id.to_owned(),
            level,
            note: String::new(),
            frames: 0,
            moves: MoveSummary::default(),
            events: Vec::new(),
            events_dropped: 0,
        }
    }

    pub fn push(&mut self, event: Timed) {
        if self.events.len() < EVENTS_PER_STEP {
            self.events.push(event);
        } else {
            self.events_dropped += 1;
        }
    }
}

/// Pointer moves are counted rather than logged: there can be hundreds a second.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct MoveSummary {
    pub while_active: u64,
    pub while_inactive: u64,
    pub while_unknown: u64,
    pub first: Option<NativePoint>,
    pub last: Option<NativePoint>,
}

impl MoveSummary {
    pub fn record(&mut self, point: NativePoint, app_active: Option<bool>) {
        match app_active {
            Some(true) => self.while_active += 1,
            Some(false) => self.while_inactive += 1,
            None => self.while_unknown += 1,
        }
        self.first.get_or_insert(point);
        self.last = Some(point);
    }
}

/// An event and when it happened, in milliseconds since the probe started.
#[derive(Debug, Clone, Serialize)]
pub struct Timed {
    pub ms: u64,
    #[serde(flatten)]
    pub event: ProbeEvent,
}

/// Whether this app was active, and which app was frontmost, when something happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Activity {
    pub app_active: Option<bool>,
    pub frontmost: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ProbeEvent {
    Placed {
        monitor: MonitorId,
        strip: ScaleReport,
        /// Boxed: with its `AppBar` record it is by far the largest event.
        frame: Box<Placed>,
    },
    Level {
        mode: LevelMode,
        value: Option<isize>,
    },
    Raised {
        value: Option<isize>,
    },
    Restored {
        mode: LevelMode,
        value: Option<isize>,
    },
    PointerEntered(Activity),
    PointerLeft(Activity),
    Click {
        button: String,
        point: Option<NativePoint>,
        #[serde(flatten)]
        activity: Activity,
    },
    Focused {
        focused: bool,
    },
    AppActive {
        at: &'static str,
        active: Option<bool>,
    },
    System {
        what: String,
    },
    ScaleFactor {
        factor: f64,
    },
    Resized {
        width: u32,
        height: u32,
    },
    Occluded {
        occluded: bool,
    },
    HotkeyUnavailable {
        error: String,
    },
    /// What the OS says about the strip (Windows): at launch and whenever a step closes.
    Os(OsSnapshot),
    /// What Explorer starting again did to the bar (Windows).
    Taskbar(TaskbarReport),
    /// The strip went behind a full-screen app, or came back (Windows).
    Fullscreen(FullscreenReport),
    /// What the tools step found.
    Tools(Box<ToolsReport>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    Completed,
    Quit,
    InputClosed,
    WindowClosed,
    /// The strip never opened; stderr says why.
    LaunchFailed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pinning::{AppBarPlaced, DEFAULT_LEVEL, MessageCount, PxRect};

    fn report() -> Report {
        let mut step = StepReport::new("a.hover-clicks", DEFAULT_LEVEL);
        step.note = "the other app kept focus".to_owned();
        step.push(Timed {
            ms: 1500,
            event: ProbeEvent::Click {
                button: "Right".to_owned(),
                point: Some(NativePoint::Scene { x: 12, y: 70 }),
                activity: Activity {
                    app_active: Some(false),
                    frontmost: Some("Example Editor".to_owned()),
                },
            },
        });
        step.moves.record(NativePoint::Padding, Some(false));
        Report {
            probe: "pinning",
            version: "0.0.0",
            os: "Example OS".to_owned(),
            started_unix: 1,
            separate_spaces: Some(true),
            monitors: Vec::new(),
            chosen: Some(MonitorId("ABC".to_owned())),
            taskbar: None,
            strip: None,
            steps: vec![step],
            ended: Some(EndReason::InputClosed),
        }
    }

    #[test]
    fn the_json_carries_the_steps_and_tags_each_event() {
        let json: serde_json::Value = serde_json::from_str(&report().to_json().unwrap()).unwrap();
        let step = &json["steps"][0];
        assert_eq!(step["id"], "a.hover-clicks");
        assert_eq!(step["level"], serde_json::to_value(DEFAULT_LEVEL).unwrap());
        assert_eq!(step["note"], "the other app kept focus");
        assert_eq!(step["events"][0]["event"], "click");
        assert_eq!(step["events"][0]["ms"], 1500);
        assert_eq!(step["events"][0]["app_active"], false);
        assert_eq!(step["events"][0]["point"]["scene"]["x"], 12);
        assert_eq!(step["moves"]["while_inactive"], 1);
        assert_eq!(json["ended"], "input-closed");
    }

    #[test]
    fn a_step_keeps_a_bounded_number_of_events() {
        let mut step = StepReport::new("c.idle-cpu", DEFAULT_LEVEL);
        for ms in 0..EVENTS_PER_STEP as u64 + 7 {
            step.push(Timed {
                ms,
                event: ProbeEvent::Focused { focused: true },
            });
        }
        assert_eq!(step.events.len(), EVENTS_PER_STEP);
        assert_eq!(step.events_dropped, 7);
    }

    const RECT: PxRect = PxRect {
        left: 0,
        top: 1096,
        right: 2560,
        bottom: 1392,
    };

    fn placed(appbar: Option<AppBarPlaced>) -> ProbeEvent {
        ProbeEvent::Placed {
            monitor: MonitorId("M".to_owned()),
            strip: ScaleReport {
                scale: 4,
                width: 2560,
                height: 296,
                image_x: 0,
            },
            frame: Box::new(Placed {
                x: 0.0,
                y: 1096.0,
                width: 2560.0,
                height: 296.0,
                unit: "px",
                appbar,
            }),
        }
    }

    #[test]
    fn a_placement_without_an_appbar_carries_no_appbar_field() {
        let json = serde_json::to_value(placed(None)).unwrap();
        assert_eq!(json["event"], "placed");
        assert!(json["frame"].get("appbar").is_none(), "{json}");
    }

    #[test]
    fn an_appbar_placement_carries_its_rectangles_and_leaves_out_an_absent_stale_check() {
        let appbar = AppBarPlaced {
            monitor: RECT,
            work_before: RECT,
            reserved: true,
            proposal: RECT,
            queried: Some(RECT),
            set: RECT,
            window: None,
            work_after: None,
            unchanged: false,
            size_mismatch: false,
            fresh: false,
            autohide_inset: 0,
            stale: None,
        };
        let json = serde_json::to_value(placed(Some(appbar))).unwrap();
        let appbar = &json["frame"]["appbar"];
        assert_eq!(appbar["set"]["top"], 1096);
        assert_eq!(appbar["reserved"], true);
        assert_eq!(appbar["window"], serde_json::Value::Null);
        assert!(appbar.get("stale").is_none(), "{appbar}");
    }

    #[test]
    fn a_taskbar_report_names_how_the_bar_was_asked_for() {
        use terrarium_platform::appbar::TaskbarAction;
        let event = |action| {
            serde_json::to_value(ProbeEvent::Taskbar(TaskbarReport {
                action,
                answered: Some(true),
                shell_hook: true,
            }))
            .unwrap()
        };
        assert_eq!(event(TaskbarAction::AskOnce)["action"], "ask-once");
        assert_eq!(
            event(TaskbarAction::RegisterAfresh)["action"],
            "register-afresh"
        );
        assert_eq!(event(TaskbarAction::Nothing)["action"], "nothing");
        assert_eq!(event(TaskbarAction::Nothing)["event"], "taskbar");
    }

    #[test]
    fn an_os_snapshot_is_tagged_and_lists_message_counts() {
        let event = ProbeEvent::Os(OsSnapshot {
            work_area: Some(RECT),
            tool_window: true,
            no_activate: true,
            app_window: false,
            topmost: true,
            visible: true,
            iconic: false,
            yielded: false,
            messages: vec![MessageCount {
                name: "WM_ACTIVATE",
                subclass: 2,
                hook: 0,
            }],
        });
        let json = serde_json::to_value(event).unwrap();
        assert_eq!(json["event"], "os");
        assert_eq!(json["messages"][0]["name"], "WM_ACTIVATE");
        assert_eq!(json["messages"][0]["subclass"], 2);
        assert_eq!(json["tool_window"], true);
    }

    #[test]
    fn moves_are_counted_by_activity_with_the_first_and_last_point() {
        let mut moves = MoveSummary::default();
        moves.record(NativePoint::Scene { x: 1, y: 2 }, Some(true));
        moves.record(NativePoint::Padding, None);
        moves.record(NativePoint::Scene { x: 3, y: 4 }, Some(false));
        assert_eq!(
            moves,
            MoveSummary {
                while_active: 1,
                while_inactive: 1,
                while_unknown: 1,
                first: Some(NativePoint::Scene { x: 1, y: 2 }),
                last: Some(NativePoint::Scene { x: 3, y: 4 }),
            }
        );
    }
}
