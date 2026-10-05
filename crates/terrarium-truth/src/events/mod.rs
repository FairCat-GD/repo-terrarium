//! Events: what qualified, what may play, and the marks events leave in the ground.
//!
//! - **Qualifying.** A detector in the replay qualifies an event at a tick, with its strength.
//!   Its sim effects apply then, whether it plays or not. Each kind's cooldown runs from the
//!   last time it qualified: a qualification inside it is suppressed, and starts it again.
//! - **Candidates.** A qualification outside its cooldown is a candidate to play, remembered for
//!   [`EVENT_RECALL_TICKS`]. Arbitration is a choice among candidates, never stored: the lowest
//!   rank, then the higher strength, then the newer. The scene plays the best of the current
//!   tick ([`Events::current`]); after a gap, the engine plays the best the terrarium had not
//!   shown before it ([`Events::unseen`]). An instance is its (tick, kind); every cooldown is at
//!   least a tick, so no two candidates share one.
//! - **Side.** Drawn from the seed, keyed by the tick and the kind. A clearing leaves by the side
//!   its storm came in from; with no storm since the weather in effect last switched, it draws
//!   its own.
//! - **Ground marks.** The settled crack and sinkhole stay until the next rain after drought or
//!   for 30 days, fresh soil for 14; at most 3, the oldest leaving first. A mark's id is hashed
//!   from what made it, so a replay after a history rewrite never gives one mark another's id.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::canon::canonical_struct;
use crate::identity::RepoSeed;
use crate::rng::{Pcg32, StreamTag, digest_u64, low_u32};
use crate::scene::{ActiveEvent, EventKind, GroundMark, GroundMarkView, Side, Strength};
use crate::time::{TICKS_PER_DAY, Tick};
use crate::tuning::{
    EVENT_RECALL_TICKS, FRESH_SOIL_DAYS, MARKS_MAX, SETTLED_MARK_DAYS, event_cooldown, event_rank,
};

/// Every cooldown is at least a tick, so a kind qualifies as a candidate at most once a tick.
const _: () = {
    let mut i = 0;
    while i < EventKind::ALL.len() {
        assert!(event_cooldown(EventKind::ALL[i]) >= 1);
        i += 1;
    }
};

/// The strength of `value` against bands `[mid_from, high_from]`.
pub fn strength<T: Ord + Copy>(value: T, [mid, high]: [T; 2]) -> Strength {
    if value >= high {
        Strength::High
    } else if value >= mid {
        Strength::Mid
    } else {
        Strength::Low
    }
}

/// A release tag's leading number: `name` matches `^v?\d+(\.\d+)*$`, digits ASCII only, so
/// `v1.0.0` and `2` are releases and `v1.0.0-rc1`, `V1` and `1..2` are not. A leading number
/// too long for `u64` saturates.
pub fn release(name: &str) -> Option<u64> {
    let numbers = name.strip_prefix('v').unwrap_or(name);
    let mut parts = numbers.split('.');
    let leading = parts.next()?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !digits(leading) || !parts.all(digits) {
        return None;
    }
    Some(leading.bytes().fold(0_u64, |n, b| {
        n.saturating_mul(10).saturating_add(u64::from(b - b'0'))
    }))
}

/// An event that may play.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub event: ActiveEvent,
    /// The plant it acts on: the one lightning scorches, or the one a sinkhole swallows.
    pub target: Option<u32>,
}

canonical_struct!(Candidate { event, target });

impl Candidate {
    /// Its instance: no two candidates share one.
    pub const fn key(&self) -> (Tick, EventKind) {
        (self.event.started_tick, self.event.kind)
    }

    /// What arbitration minimises: rank, then strength (higher first), then age (newer first).
    fn order(&self) -> (u8, std::cmp::Reverse<Strength>, std::cmp::Reverse<Tick>) {
        (
            event_rank(self.event.kind),
            std::cmp::Reverse(self.event.strength),
            std::cmp::Reverse(self.event.started_tick),
        )
    }
}

/// The candidate that wins arbitration among `candidates`.
pub fn arbitrate<'a>(candidates: impl Iterator<Item = &'a Candidate>) -> Option<&'a Candidate> {
    candidates.min_by_key(|candidate| candidate.order())
}

/// A mark an event leaves in the ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub id: u32,
    pub kind: GroundMark,
    /// The tick it was made.
    pub since: Tick,
    /// The fossil a sinkhole names.
    pub fossil: Option<u32>,
}

canonical_struct!(Mark {
    id,
    kind,
    since,
    fossil
});

/// How often a kind qualified, and how often it was a candidate to play.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tally {
    pub qualified: u32,
    pub eligible: u32,
}

canonical_struct!(Tally {
    qualified,
    eligible
});

/// What a detector found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Qualified {
    pub kind: EventKind,
    pub tick: Tick,
    pub strength: Strength,
    pub fossil: Option<u32>,
    pub target: Option<u32>,
}

/// The terrarium's events ("event queue").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Events {
    /// When each kind last qualified: its cooldown runs from there.
    pub last: BTreeMap<EventKind, Tick>,
    /// The candidates of the last `EVENT_RECALL_TICKS`, oldest first.
    pub recent: Vec<Candidate>,
    /// Running totals per kind.
    pub tally: BTreeMap<EventKind, Tally>,
    /// The ground marks, oldest first.
    pub marks: Vec<Mark>,
    /// The side the storm now in the sky came in from: set by a storm, and cleared by the
    /// clearing that ends it or by a switch of the weather in effect, after which no front of
    /// this failure entered.
    pub storm_side: Option<Side>,
    /// The unused items of the newest dead-code report taken at the default tip, while its value
    /// is known.
    pub unused: Option<u32>,
    /// The highest leading number among the release tags that have flowered.
    pub major: Option<u64>,
}

canonical_struct!(Events {
    last,
    recent,
    tally,
    marks,
    storm_side,
    unused,
    major,
});

/// The side `kind` takes at `tick`.
pub fn side(seed: &RepoSeed, kind: EventKind, tick: Tick) -> Side {
    let key = (u64::from(tick.hours()) << 8) | u64::from(kind.id());
    let picked =
        NonZeroU32::new(2).map(|two| Pcg32::stream(seed, StreamTag::EventSide, key).below(two));
    if picked == Some(1) {
        Side::Right
    } else {
        Side::Left
    }
}

/// Separates mark ids from every other SHA-256 truth computes.
const MARK_DOMAIN: &[u8] = b"repo-terrarium ground mark v1\0";

/// A mark's id: hashed from its kind, the tick it was made and the fossil it names.
pub fn mark_id(kind: GroundMark, since: Tick, fossil: Option<u32>) -> u32 {
    let kind_byte: u8 = match kind {
        GroundMark::Crack => 0,
        GroundMark::Sinkhole => 1,
        GroundMark::FreshSoil => 2,
    };
    let digest: [u8; 32] = Sha256::new()
        .chain_update(MARK_DOMAIN)
        .chain_update([kind_byte, u8::from(fossil.is_some())])
        .chain_update(since.hours().to_le_bytes())
        .chain_update(fossil.unwrap_or(0).to_le_bytes())
        .finalize()
        .into();
    low_u32(digest_u64(&digest))
}

const fn days(n: u32) -> u32 {
    n.saturating_mul(TICKS_PER_DAY)
}

impl Events {
    /// The start of `tick`: candidates past their recall are forgotten, and marks past their
    /// time leave.
    pub fn begin(&mut self, tick: Tick) {
        let forgotten = self
            .recent
            .iter()
            .take_while(|c| tick.hours_since(c.event.started_tick) >= EVENT_RECALL_TICKS)
            .count();
        self.recent.drain(..forgotten);
        self.marks.retain(|mark| {
            let lasts = match mark.kind {
                GroundMark::Crack | GroundMark::Sinkhole => days(SETTLED_MARK_DAYS),
                GroundMark::FreshSoil => days(FRESH_SOIL_DAYS),
            };
            tick.hours_since(mark.since) < lasts
        });
    }

    /// Records a qualification, and makes it a candidate unless its kind is cooling down.
    /// Returns whether it is one.
    pub fn qualify(&mut self, seed: &RepoSeed, found: Qualified) -> bool {
        let Qualified {
            kind,
            tick,
            strength,
            fossil,
            target,
        } = found;
        let tally = self.tally.entry(kind).or_default();
        tally.qualified = tally.qualified.saturating_add(1);
        let eligible = self
            .last
            .get(&kind)
            .is_none_or(|&last| tick.hours_since(last) >= event_cooldown(kind));
        self.last.insert(kind, tick);
        let drawn = side(seed, kind, tick);
        let side = match kind {
            EventKind::Clearing => self.storm_side.take().unwrap_or(drawn),
            _ => drawn,
        };
        if kind == EventKind::Storm {
            self.storm_side = Some(side);
        }
        if eligible {
            tally.eligible = tally.eligible.saturating_add(1);
            self.recent.push(Candidate {
                event: ActiveEvent {
                    kind,
                    started_tick: tick,
                    strength,
                    side,
                    fossil,
                },
                target,
            });
        }
        eligible
    }

    /// Leaves a mark in the ground at `since`. A mark made twice in one tick is one mark.
    pub fn mark(&mut self, kind: GroundMark, since: Tick, fossil: Option<u32>) {
        let id = mark_id(kind, since, fossil);
        if self.marks.iter().any(|mark| mark.id == id) {
            return;
        }
        self.marks.push(Mark {
            id,
            kind,
            since,
            fossil,
        });
        let excess = self.marks.len().saturating_sub(MARKS_MAX);
        self.marks.drain(..excess);
    }

    /// Rain after drought clears the settled crack and sinkhole.
    pub fn clear_settled(&mut self) {
        self.marks.retain(|mark| mark.kind == GroundMark::FreshSoil);
    }

    /// The candidate the scene plays at `now`: the best of those that started then.
    pub fn current(&self, now: Tick) -> Option<&Candidate> {
        arbitrate(self.recent.iter().filter(|c| c.event.started_tick == now))
    }

    /// The best candidate whose instance is not among `seen`: what plays after a gap, a restart
    /// or a rewrite, once every effect in between has applied.
    pub fn unseen(&self, seen: &BTreeSet<(Tick, EventKind)>) -> Option<&Candidate> {
        arbitrate(self.recent.iter().filter(|c| !seen.contains(&c.key())))
    }

    /// The candidate with instance `key`, while it is remembered.
    pub fn find(&self, key: (Tick, EventKind)) -> Option<&Candidate> {
        self.recent.iter().find(|c| c.key() == key)
    }

    /// Every remembered instance.
    pub fn keys(&self) -> BTreeSet<(Tick, EventKind)> {
        self.recent.iter().map(Candidate::key).collect()
    }

    /// The marks as the composer sees them.
    pub fn mark_views(&self) -> Vec<GroundMarkView> {
        self.marks
            .iter()
            .map(|mark| GroundMarkView {
                id: mark.id,
                kind: mark.kind,
                fossil: mark.fossil,
            })
            .collect()
    }
}
