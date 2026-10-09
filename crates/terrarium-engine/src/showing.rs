//! Which event a scene plays: one foreground event at a time, as arbitration among events that
//! come close together. The live view, the day-by-day replay and the debug window's simulation
//! all choose it here.

use std::collections::BTreeSet;

use terrarium_truth::events::{Candidate, arbitrate};
use terrarium_truth::scene::EventKind;
use terrarium_truth::{SceneInput, SimState, Tick, project_showing};

/// How long a playing event stays in the scenes after the hour it was first shown in: through
/// the next hour, long enough for any burst of snapshots to reach the composer.
const PLAY_HOLD_TICKS: u32 = 1;

/// What the scenes projected so far have played.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Showing {
    /// The event instances the state last projected held: what a later scene has already had
    /// the chance to play. `None` before anything was projected and with no cached state.
    seen: Option<BTreeSet<(Tick, EventKind)>>,
    /// The event the scene plays, and the tick it was first shown at. It stays in the scenes
    /// through the hour after that, so a burst of snapshots never drops it, unless an unseen
    /// event outranks it; the composer plays each instance once.
    playing: Option<((Tick, EventKind), Tick)>,
}

impl Showing {
    /// From the instances a cached state held, which it has had the chance to play.
    pub fn after(seen: Option<BTreeSet<(Tick, EventKind)>>) -> Showing {
        Showing {
            seen,
            playing: None,
        }
    }

    /// The state's scene. It plays the best event the state last projected did not hold (with
    /// none projected, the newest tick's best), unless the one already playing is still within
    /// its hold and outranks it.
    pub fn scene(&mut self, state: &SimState) -> SceneInput {
        let now = state.now();
        let events = &state.events;
        let held = self
            .playing
            .filter(|&(_, shown)| now.hours_since(shown) <= PLAY_HOLD_TICKS)
            .and_then(|(key, _)| events.find(key));
        let fresh = match &self.seen {
            Some(seen) => events.unseen(seen),
            None => events.current(now),
        };
        if let Some(fresh) = fresh {
            let winner = arbitrate(held.into_iter().chain([fresh]));
            if winner.map(Candidate::key) == Some(fresh.key()) {
                self.playing = Some((fresh.key(), now));
            }
        }
        let playing = self
            .playing
            .filter(|&(_, shown)| now.hours_since(shown) <= PLAY_HOLD_TICKS)
            .and_then(|(key, _)| events.find(key));
        if playing.is_none() {
            self.playing = None;
        }
        self.seen = Some(events.keys());
        project_showing(state, playing)
    }
}
