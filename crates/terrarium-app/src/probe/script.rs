//! The pinning probe's steps, in order. The texts live in the copy module.

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
    pub text: String,
}

pub fn pinning() -> Vec<Step> {
    let step = |prefix: &str, phase, level, item: &StepText| Step {
        id: format!("{prefix}{}", item.key),
        phase,
        level,
        text: copy::with_hotkey(item.text),
    };
    let mut steps = vec![step("", Phase::Launch, DEFAULT_LEVEL, &copy::PROBE_LAUNCH)];
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
    steps.extend(
        copy::PROBE_ONCE_STEPS
            .iter()
            .map(|item| step("c.", Phase::C, DEFAULT_LEVEL, item)),
    );
    steps
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::hotkey::{Platform, Which};

    #[test]
    fn step_ids_are_unique_and_every_step_has_text() {
        let steps = pinning();
        let ids: BTreeSet<&str> = steps.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids.len(), steps.len());
        assert!(steps.iter().all(|s| !s.text.trim().is_empty()));
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
