//! When the app draws (I-10): at the composer's next change, never more than 15 times a second,
//! and not at all while nothing changes. The cap holds for every composition, presented or not,
//! so a change too faint to show still costs no more than a frame would.

use crate::tween::Millis;

/// The least time between two frames: 15 fps at most, the manifests' ceiling.
pub const MIN_FRAME_MS: u64 = 67;

/// Paces frames: it remembers when the last one was composed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pacer {
    last: Option<Millis>,
}

impl Pacer {
    /// When to compose next, given when the picture next changes: then, or a fifteenth of a
    /// second after the last composition if that is later. `None` while nothing will change.
    pub fn wake(&self, change: Option<Millis>) -> Option<Millis> {
        let change = change?;
        Some(match self.last {
            Some(last) => change.max(Millis(last.0.saturating_add(MIN_FRAME_MS))),
            None => change,
        })
    }

    /// The picture was composed at `at`, whether or not it went to the screen.
    pub fn composed(&mut self, at: Millis) {
        self.last = Some(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_wait_for_a_change_and_come_at_most_15_a_second() {
        let mut pacer = Pacer::default();
        assert_eq!(pacer.wake(None), None);
        assert_eq!(pacer.wake(Some(Millis(5))), Some(Millis(5)));
        pacer.composed(Millis(1000));
        assert_eq!(pacer.wake(Some(Millis(1001))), Some(Millis(1067)));
        assert_eq!(pacer.wake(Some(Millis(1500))), Some(Millis(1500)));
        assert_eq!(pacer.wake(None), None);
        // A change every millisecond still draws no more than 15 frames in a second.
        let mut pacer = Pacer::default();
        let mut now = Millis(0);
        let mut frames = Vec::new();
        while let Some(at) = pacer.wake(Some(Millis(now.0 + 1))) {
            if at.0 >= 1000 {
                break;
            }
            frames.push(at);
            pacer.composed(at);
            now = at;
        }
        assert_eq!(frames.len(), 15);
    }
}
