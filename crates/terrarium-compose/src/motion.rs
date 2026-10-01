//! Motion in turns: the residents' walks and flights and the plants' sway happen in global
//! turns of [`TURN_MS`] on the composer's clock.
//!
//! A turn reads only what the scene was when it began. An update records the scene for the
//! turns after the one running ([`Eras`]), so a change never cuts a walk or a gust in
//! progress: it takes effect at the next turn. Everything a turn does is a pure function of
//! that record, the turn's index and the ids, so motion needs no history and the salt never
//! times it (I-2).

use std::collections::BTreeSet;

use terrarium_truth::PresentationState;
use terrarium_truth::scene::{Weather, Wind};

use crate::clock::{DayNight, LocalClock, NIGHT_FROM};
use crate::order::Subject;
use crate::resident::Resident;
use crate::tween::Millis;

/// One turn: long enough for a resident to walk out, rest and walk back, and a whole number of
/// every rest loop's cycle, so a loop runs on across turns unbroken.
pub const TURN_MS: u64 = 24_000;

/// The turn `now` falls in.
pub const fn turn(now: Millis) -> u64 {
    now.0 / TURN_MS
}

/// When turn `w` begins.
pub const fn turn_start(w: u64) -> Millis {
    Millis(w.saturating_mul(TURN_MS))
}

/// Whether turn `w` carries motion, and if so the count its rotations use. In dim mode only
/// every fourth turn moves (a quarter of the motion), and the rotations count those turns
/// alone, so every resident and plant still takes its turn.
pub const fn motion_turn(w: u64, dim: bool) -> Option<u64> {
    if !dim {
        Some(w)
    } else if w.is_multiple_of(4) {
        Some(w / 4)
    } else {
        None
    }
}

/// A number for one thing in one turn, spread over all 64 bits. It reads no salt: motion is
/// the same on every install (`SplitMix64`'s output function).
pub const fn motion_hash(id: u64, w: u64) -> u64 {
    let mut z = id
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(w.wrapping_add(1).wrapping_mul(0xd1b5_4a32_d192_ed03));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The local clock as an update gave it, to tell whether a turn begins by night.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    pub local: LocalClock,
    /// When on the composer's clock the local clock was read.
    pub at: Millis,
    pub day_night: DayNight,
}

impl Clock {
    /// Whether the palette is past halfway to night at `now`, on this clock carried forward.
    pub fn night(self, now: Millis) -> bool {
        let carried = u64::from(self.local.ms_of_day) + now.0.saturating_sub(self.at.0);
        let ms = u32::try_from(carried % u64::from(crate::clock::DAY_MS)).unwrap_or(0);
        self.day_night.mix(ms) >= NIGHT_FROM
    }
}

/// What a turn reads of the scene: the scene as the last update before the turn left it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub state: PresentationState,
    pub weather: Weather,
    pub wind: Wind,
    pub dim: bool,
    pub clock: Clock,
    /// The plants that may sway: living, past seedlings.
    pub swaying: BTreeSet<Subject>,
    /// The residents drawn, by id, each at its spot and home.
    pub residents: Vec<Resident>,
}

/// The scene each turn reads. An update stands from the turn after the one it arrives in;
/// the turn it arrives in keeps reading what it began with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Eras {
    /// Each record and the first turn it stands for, in turn order.
    eras: Vec<(u64, Snapshot)>,
}

impl Eras {
    /// Records the scene of an update at `now`.
    pub fn record(&mut self, now: Millis, snapshot: Snapshot) {
        let from = turn(now) + 1;
        if self.eras.last().is_some_and(|&(first, _)| first == from) {
            self.eras.pop();
        }
        self.eras.push((from, snapshot));
        // Only the turn running and the one before it are ever read again.
        let running = turn(now);
        while self.eras.len() > 2 && self.eras[1].0 <= running.saturating_sub(1) {
            self.eras.remove(0);
        }
    }

    /// The scene turn `w` reads, or `None` before the first update's turn has passed: that
    /// turn is still.
    pub fn at(&self, w: u64) -> Option<&Snapshot> {
        self.eras
            .iter()
            .rev()
            .find(|&&(first, _)| first <= w)
            .map(|(_, snapshot)| snapshot)
    }

    /// The first turn any record stands for.
    pub fn first(&self) -> Option<u64> {
        self.eras.first().map(|&(first, _)| first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::LocalDate;

    fn snapshot(wind: Wind) -> Snapshot {
        Snapshot {
            state: PresentationState::Thriving,
            weather: Weather::Clear,
            wind,
            dim: false,
            clock: Clock {
                local: LocalClock::noon(LocalDate::new(2026, 3, 3)),
                at: Millis(0),
                day_night: DayNight::default(),
            },
            swaying: BTreeSet::new(),
            residents: Vec::new(),
        }
    }

    #[test]
    fn an_update_stands_from_the_next_turn() {
        let mut eras = Eras::default();
        eras.record(Millis(1000), snapshot(Wind::Calm));
        assert_eq!(eras.at(0), None, "the first update's turn is still");
        assert_eq!(eras.at(1).map(|s| s.wind), Some(Wind::Calm));
        eras.record(Millis(TURN_MS * 3 + 5), snapshot(Wind::Gale));
        assert_eq!(
            eras.at(3).map(|s| s.wind),
            Some(Wind::Calm),
            "the running turn holds"
        );
        assert_eq!(eras.at(4).map(|s| s.wind), Some(Wind::Gale));
        // A second update in one turn replaces the first for the turns after.
        eras.record(Millis(TURN_MS * 3 + 9), snapshot(Wind::Breezy));
        assert_eq!(eras.at(4).map(|s| s.wind), Some(Wind::Breezy));
        assert_eq!(eras.at(3).map(|s| s.wind), Some(Wind::Calm));
        // Long after, only what can still be read is kept.
        for k in 10..20 {
            eras.record(Millis(TURN_MS * k), snapshot(Wind::Calm));
        }
        assert!(eras.eras.len() <= 3);
        assert!(eras.at(18).is_some() && eras.at(19).is_some());
    }

    #[test]
    fn dim_moves_one_turn_in_four_and_counts_only_those() {
        let turns: Vec<Option<u64>> = (0..9).map(|w| motion_turn(w, true)).collect();
        assert_eq!(
            turns,
            [
                Some(0),
                None,
                None,
                None,
                Some(1),
                None,
                None,
                None,
                Some(2)
            ]
        );
        assert_eq!(motion_turn(7, false), Some(7));
    }

    #[test]
    fn the_motion_hash_is_pinned_and_spreads() {
        // Pinned: every resident's walks and every gust hang on it.
        assert_eq!(motion_hash(0, 0), 0x8209_b480_faed_1b10);
        assert_eq!(motion_hash(7, 3), 0x986b_8f4b_bb8c_694e);
        let values: BTreeSet<u64> = (0..64).map(|w| motion_hash(7, w) % 16).collect();
        assert!(values.len() > 12, "{values:?}");
        assert_ne!(motion_hash(1, 2), motion_hash(2, 1));
    }
}
