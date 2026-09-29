//! What the probe writes: JSON for tools and text for people (the text is built in the copy
//! module). It is rewritten after every step, so an interrupted run keeps what it saw.

use serde::Serialize;
use terrarium_compose::scale::StripScale;

use crate::hover::NativePoint;
use crate::monitor::{MonitorId, MonitorInfo};
use crate::pinning::{LevelMode, Placed};

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
        frame: Placed,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    Completed,
    Quit,
    InputClosed,
    WindowClosed,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        let mut step = StepReport::new("a.hover-clicks", LevelMode::DesktopIconPlusOne);
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
        assert_eq!(step["level"], "desktop-icon-plus-one");
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
        let mut step = StepReport::new("c.idle-cpu", LevelMode::AlwaysOnBottom);
        for ms in 0..EVENTS_PER_STEP as u64 + 7 {
            step.push(Timed {
                ms,
                event: ProbeEvent::Focused { focused: true },
            });
        }
        assert_eq!(step.events.len(), EVENTS_PER_STEP);
        assert_eq!(step.events_dropped, 7);
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
