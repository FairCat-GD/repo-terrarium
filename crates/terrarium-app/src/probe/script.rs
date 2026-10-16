//! The pinning probe's steps, in order. The texts live in the copy module.

use terrarium_platform::geometry::Edge;

use crate::copy::{self, StepText};
use crate::pinning::{DEFAULT_LEVEL, LevelMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Launch,
    /// Each level-sensitive item at desktop-icon + 1 (macOS, which has two levels to compare).
    #[cfg(target_os = "macos")]
    A,
    /// The same items at always-on-bottom.
    #[cfg(target_os = "macos")]
    B,
    /// Everything else, once, at the default level.
    C,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub id: String,
    pub phase: Phase,
    pub level: LevelMode,
    /// The edge the strip is on for this step: the top for the one step that moves it there,
    /// so the bar is registered afresh on a new edge, and the bottom otherwise.
    pub edge: Edge,
    /// The step runs the checks of the keychain, node and TLS (`probe::tools`).
    pub tools: bool,
    pub text: String,
}

fn step(prefix: &str, phase: Phase, level: LevelMode, item: &StepText) -> Step {
    Step {
        id: format!("{prefix}{}", item.key),
        phase,
        level,
        edge: if item.key == copy::TOP_EDGE_STEP {
            Edge::Top
        } else {
            Edge::Bottom
        },
        tools: item.key == copy::TOOLS_STEP,
        text: copy::with_hotkey(item.text),
    }
}

/// The launch, then each of `once`, at the default level.
fn launch_and_once(launch: &StepText, once: &[StepText]) -> (Step, Vec<Step>) {
    (
        step("", Phase::Launch, DEFAULT_LEVEL, launch),
        once.iter()
            .map(|item| step("c.", Phase::C, DEFAULT_LEVEL, item))
            .collect(),
    )
}

pub fn pinning() -> Vec<Step> {
    let (launch, once) = launch_and_once(&copy::PROBE_LAUNCH, copy::PROBE_ONCE_STEPS);
    let mut steps = vec![launch];
    #[cfg(target_os = "macos")]
    for (prefix, phase, level) in [
        ("a.", Phase::A, LevelMode::DesktopIconPlusOne),
        ("b.", Phase::B, LevelMode::AlwaysOnBottom),
    ] {
        steps.extend(
            copy::PROBE_LEVEL_STEPS
                .iter()
                .map(|item| step(prefix, phase, level, item)),
        );
    }
    steps.extend(once);
    steps
}

/// The Windows walk-through, built on any OS for its tests.
#[cfg(test)]
pub(crate) fn windows_steps() -> Vec<Step> {
    let (launch, once) =
        launch_and_once(&copy::WINDOWS_PROBE_LAUNCH, copy::WINDOWS_PROBE_ONCE_STEPS);
    std::iter::once(launch).chain(once).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::hotkey::{Platform, Which};

    #[test]
    fn step_ids_are_unique_and_every_step_has_text() {
        for steps in [pinning(), windows_steps()] {
            let ids: BTreeSet<&str> = steps.iter().map(|s| s.id.as_str()).collect();
            assert_eq!(ids.len(), steps.len());
            assert!(steps.iter().all(|s| !s.text.trim().is_empty()));
        }
    }

    #[test]
    fn the_windows_walk_through_opens_at_launch_and_ends_with_the_taskkill() {
        let steps = windows_steps();
        assert_eq!(steps.first().map(|s| s.phase), Some(Phase::Launch));
        assert_eq!(steps.last().map(|s| s.id.as_str()), Some("c.taskkill"));
        for id in [
            "c.peek",
            "c.fullscreen",
            "c.auto-hide",
            "c.explorer-restart",
            "c.hot-plug",
            "c.connect",
        ] {
            assert!(steps.iter().any(|s| s.id == id), "{id}");
        }
    }

    #[test]
    fn only_the_top_edge_step_moves_the_edge_and_the_next_one_returns_it() {
        for steps in [pinning(), windows_steps()] {
            let top: Vec<usize> = (0..steps.len())
                .filter(|&i| steps[i].edge == Edge::Top)
                .collect();
            assert!(top.len() <= 1, "{top:?}");
            if let Some(&i) = top.first() {
                assert_eq!(steps[i].id, format!("c.{}", copy::TOP_EDGE_STEP));
                assert_eq!(steps.get(i + 1).map(|s| s.edge), Some(Edge::Bottom));
            }
        }
        assert!(windows_steps().iter().any(|s| s.edge == Edge::Top));
    }

    #[test]
    fn the_tools_step_runs_on_both_platforms() {
        for steps in [pinning(), windows_steps()] {
            let tools: Vec<&str> = steps
                .iter()
                .filter(|s| s.tools)
                .map(|s| s.id.as_str())
                .collect();
            assert_eq!(tools, [format!("c.{}", copy::TOOLS_STEP)]);
        }
    }

    #[test]
    fn the_hotkey_step_names_the_keys_the_strip_registers() {
        let steps = pinning();
        let keys = copy::hotkey(Which::Raise.default_binding(), Platform::THIS);
        let hotkey = steps
            .iter()
            .find(|s| s.id == "c.hotkey")
            .expect("a hotkey step");
        assert!(
            hotkey.text.contains(&format!("Press {keys}.")),
            "{}",
            hotkey.text
        );
        assert!(steps.iter().all(|s| !s.text.contains('{')));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn phases_a_and_b_repeat_the_same_items_at_the_two_levels() {
        let steps = pinning();
        let items = |phase| {
            steps
                .iter()
                .filter(|s| s.phase == phase)
                .map(|s| (&s.id[2..], s.text.as_str(), s.level))
                .collect::<Vec<_>>()
        };
        let (a, b) = (items(Phase::A), items(Phase::B));
        assert!(!a.is_empty());
        assert_eq!(a.len(), b.len());
        for ((a_key, a_text, a_level), (b_key, b_text, b_level)) in a.into_iter().zip(b) {
            assert_eq!((a_key, a_text), (b_key, b_text));
            assert_eq!(a_level, LevelMode::DesktopIconPlusOne);
            assert_eq!(b_level, LevelMode::AlwaysOnBottom);
        }
    }

    /// macOS ends by asking which level worked better. Windows ends with the taskkill, which
    /// ends the process, so nothing can come after it.
    #[test]
    fn the_probe_opens_at_launch_and_ends_with_its_last_step() {
        let last = if cfg!(windows) {
            "c.taskkill"
        } else {
            "c.preferred-level"
        };
        let steps = pinning();
        assert_eq!(steps.first().map(|s| s.phase), Some(Phase::Launch));
        assert_eq!(steps.last().map(|s| s.id.as_str()), Some(last));
    }
}
