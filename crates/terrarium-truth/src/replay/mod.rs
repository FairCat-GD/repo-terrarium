//! Replay: the tick loop that turns git history and recorded observations into the climate, the
//! habitat and the presentation timeline.
//!
//! Ticks apply in order, and within one tick:
//! 1. a UTC day that has ended closes, and the smoothed climates step;
//! 2. the tick's commits apply, in the git reader's order, which puts parents first (truth
//!    cannot re-derive it: two commits of one tick may have their ids either way round);
//! 3. the tick's observations apply, ordered by their merge key (`recorded_at`, kind, source,
//!    head commit), which is the same on every install, unlike row ids;
//! 4. the weather and the presentation are evaluated.
//!
//! A light or soil report applies at its own tick, or at its commit's tick if that is later: a
//! report taken at a commit dated in a later hour waits for it, and an older report applied
//! late never replaces a newer one.
//!
//! The state after a tick depends only on the inputs at or before it. That is what lets a cached
//! state resume: a cache holds the state before the hour still open, with `next_tick`
//! as its tick, and may resume while every commit and observation that is new since is at or
//! after `next_tick`. The live view is a copy of it advanced through the current tick. A resume
//! is given every observation, not only those above the cache's high-water id: one recorded
//! earlier in the open hour has not been applied yet. Resuming checks, by a digest of each, that
//! the commits and observations before `next_tick` are exactly the ones the state consumed, and
//! refuses otherwise. It can refuse even when every new input is at or after `next_tick`: a
//! newly read commit can move an earlier report, taken at it, to its own tick. On a refusal,
//! replay from the start.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::canon::{Canonical, DumpError, Reader, Writer, canonical_struct, from_hex, to_hex};
use crate::climate::readings::{Latest, Readings};
use crate::climate::rhythm::Rhythm;
use crate::climate::weather::WeatherState;
use crate::climate::{self, ClimateReport, DailyClimate, SoilCauses, Trends};
use crate::domain::{
    Climate, CommitFacts, Habitat, Observation, ObservationKind, ObservationValue, Reading,
};
use crate::fixed::{Q16, Q32};
use crate::habitat::HabitatState;
use crate::identity::{CommitId, RepoSeed, SIM_VERSION, SimVersion};
use crate::time::{SECONDS_PER_TICK, TICKS_PER_DAY, Tick, TickGrid};
use crate::timeline::{Moment, PresentationTimeline};
use crate::tuning::{
    DAILY_RECORDS, INACTIVE_TICKS, NEUTRAL, PASS_FROM, RECOVERY_WINDOW_TICKS, SECONDS_PER_DAY,
    SETTLING_TICKS,
};

/// What a replay is of. A state resumes only on the basis it was built on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Basis {
    pub sim_version: SimVersion,
    pub seed: RepoSeed,
    pub grid: TickGrid,
    /// The default branch recorded at connect.
    pub default_branch: String,
}

canonical_struct!(Basis {
    sim_version,
    seed,
    grid,
    default_branch
});

impl Basis {
    /// A basis under this build's `SIM_VERSION`.
    pub fn new(seed: RepoSeed, grid: TickGrid, default_branch: &str) -> Basis {
        Basis {
            sim_version: SIM_VERSION,
            seed,
            grid,
            default_branch: default_branch.to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// A commit's tick is earlier than the one before it, so the commits are not in the git
    /// reader's order.
    CommitsOutOfOrder { commit: CommitId },
    /// Two observations share a merge key.
    DuplicateObservation {
        recorded_at: i64,
        kind: ObservationKind,
    },
    /// The state was built on another seed, grid, default branch or `SIM_VERSION`.
    BasisChanged,
    /// The inputs before the state's next tick are not the ones it consumed: replay from the
    /// start.
    NeedsFullReplay,
}

/// The tick an observation recorded at `recorded_at` applies at: one recorded before tick 0
/// applies at tick 0, and one past the grid's last tick never does.
pub fn observation_tick(grid: TickGrid, recorded_at: i64) -> Option<Tick> {
    if recorded_at < grid.origin_secs() {
        Some(Tick::ZERO)
    } else {
        grid.tick_at(recorded_at)
    }
}

/// The UTC day `tick` falls in, as days since 1970-01-01.
pub fn utc_day(grid: TickGrid, tick: Tick) -> i64 {
    grid.start_secs(tick).div_euclid(SECONDS_PER_DAY)
}

/// The first tick after tick 0 that begins a UTC day: the first close.
pub fn first_close(grid: TickGrid) -> Tick {
    let into_day = grid.origin_secs().rem_euclid(SECONDS_PER_DAY) / SECONDS_PER_TICK;
    Tick::new(TICKS_PER_DAY - u32::try_from(into_day).unwrap_or(0))
}

/// The inputs of one replay, checked and ordered once, to advance any number of states.
#[derive(Clone, Debug)]
pub struct Prepared<'a> {
    basis: Basis,
    commits: &'a [CommitFacts],
    commit_ticks: BTreeMap<&'a CommitId, Tick>,
    /// Each with the tick it applies at, ordered by that tick, then by merge key.
    observations: Vec<(Tick, &'a Observation)>,
    /// `commit_digests[n]` is the digest of the first `n` commits; likewise for observations.
    commit_digests: Vec<InputDigest>,
    observation_digests: Vec<InputDigest>,
}

/// A chained SHA-256 over a run of inputs, in order: what a resume compares to know the inputs
/// it consumed are still the ones before its next tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDigest([u8; 32]);

impl InputDigest {
    /// The digest of no inputs.
    pub const EMPTY: InputDigest = InputDigest([0; 32]);

    fn chain<T: Serialize>(self, item: &T) -> InputDigest {
        let bytes = postcard::to_allocvec(item).unwrap_or_default();
        InputDigest(
            Sha256::new()
                .chain_update(self.0)
                .chain_update(bytes)
                .finalize()
                .into(),
        )
    }

    /// The digest after each prefix of `items`: `n + 1` digests for `n` items.
    fn prefixes<T: Serialize>(items: impl Iterator<Item = T>) -> Vec<InputDigest> {
        let mut digests = vec![InputDigest::EMPTY];
        let mut digest = InputDigest::EMPTY;
        for item in items {
            digest = digest.chain(&item);
            digests.push(digest);
        }
        digests
    }
}

impl Canonical for InputDigest {
    fn write(&self, w: &mut Writer) {
        w.leaf(&to_hex(&self.0));
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, DumpError> {
        let text = r.leaf()?;
        from_hex(text)
            .and_then(|bytes| bytes.try_into().ok())
            .map(InputDigest)
            .ok_or_else(|| r.bad_value(text))
    }
}

/// The tick a report applies at: its own, or its commit's if that is later, for light and soil
/// reports taken at the default tip.
fn applies_at(
    commit_ticks: &BTreeMap<&CommitId, Tick>,
    observation: &Observation,
    own: Tick,
) -> Tick {
    let reading = !matches!(
        observation.value,
        ObservationValue::Weather { .. } | ObservationValue::RefRewrite { .. }
    );
    match commit_ticks.get(&observation.head.commit) {
        Some(&at) if reading && observation.head.at_default_tip => at.max(own),
        _ => own,
    }
}

type MergeKey<'a> = (i64, &'static str, String, &'a str);

fn merge_key(observation: &Observation) -> MergeKey<'_> {
    let source = match &observation.value {
        ObservationValue::Weather { source } => source.to_column(),
        _ => String::new(),
    };
    (
        observation.recorded_at,
        observation.value.kind().as_column(),
        source,
        observation.head.commit.as_str(),
    )
}

impl<'a> Prepared<'a> {
    /// `commits` in the git reader's order; `observations` in any order.
    pub fn new(
        basis: Basis,
        commits: &'a [CommitFacts],
        observations: &'a [Observation],
    ) -> Result<Prepared<'a>, ReplayError> {
        let out_of_order = commits.windows(2).find_map(|pair| match pair {
            [before, after] if after.tick < before.tick => Some(after.id.clone()),
            _ => None,
        });
        if let Some(commit) = out_of_order {
            return Err(ReplayError::CommitsOutOfOrder { commit });
        }
        let mut keyed: Vec<(MergeKey<'a>, Tick, &'a Observation)> = observations
            .iter()
            .filter_map(|o| {
                Some((
                    merge_key(o),
                    observation_tick(basis.grid, o.recorded_at)?,
                    o,
                ))
            })
            .collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        let duplicate = keyed.windows(2).find_map(|pair| match pair {
            [before, after] if before.0 == after.0 => Some(after.2),
            _ => None,
        });
        if let Some(observation) = duplicate {
            return Err(ReplayError::DuplicateObservation {
                recorded_at: observation.recorded_at,
                kind: observation.value.kind(),
            });
        }
        let commit_ticks: BTreeMap<&CommitId, Tick> =
            commits.iter().map(|c| (&c.id, c.tick)).collect();
        let mut observations: Vec<(Tick, &Observation)> = keyed
            .into_iter()
            .map(|(_, own, o)| (applies_at(&commit_ticks, o, own), o))
            .collect();
        // Stable: within a tick, the merge key's order stands.
        observations.sort_by_key(|&(tick, _)| tick);
        // The row id is install-local, so it is left out of the digest.
        let observation_digests = InputDigest::prefixes(observations.iter().map(|(_, o)| {
            (
                &o.recorded_at,
                &o.head,
                &o.normalized,
                &o.confidence,
                &o.value,
            )
        }));
        Ok(Prepared {
            commit_digests: InputDigest::prefixes(commits.iter()),
            observation_digests,
            commit_ticks,
            observations,
            basis,
            commits,
        })
    }

    pub fn basis(&self) -> &Basis {
        &self.basis
    }

    /// Whether `commit` is in the history and applies at or before `tick`.
    fn applied_by(&self, commit: &CommitId, tick: Tick) -> bool {
        self.commit_ticks.get(commit).is_some_and(|&at| at <= tick)
    }

    /// The first `commits` commits and `observations` observations, as a state records them.
    fn consumed(&self, commits: usize, observations: usize) -> Consumed {
        let digest = |digests: &[InputDigest], n: usize| {
            digests.get(n).copied().unwrap_or(InputDigest::EMPTY)
        };
        Consumed {
            commits: u64::try_from(commits).unwrap_or(u64::MAX),
            observations: u64::try_from(observations).unwrap_or(u64::MAX),
            commit_digest: digest(&self.commit_digests, commits),
            observation_digest: digest(&self.observation_digests, observations),
        }
    }
}

/// The commits and observations the applied ticks held: how many, and their digests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Consumed {
    pub commits: u64,
    pub observations: u64,
    pub commit_digest: InputDigest,
    pub observation_digest: InputDigest,
}

canonical_struct!(Consumed {
    commits,
    observations,
    commit_digest,
    observation_digest,
});

/// Everything truth knows about a terrarium after its applied ticks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimState {
    pub basis: Basis,
    /// The first tick not yet applied.
    pub next_tick: Tick,
    pub consumed: Consumed,
    pub rhythm: Rhythm,
    pub readings: Readings,
    pub weather: WeatherState,
    pub habitat: HabitatState,
    /// The newest daily records, oldest first.
    pub daily: Vec<DailyClimate>,
    pub timeline: PresentationTimeline,
}

canonical_struct!(SimState {
    basis,
    next_tick,
    consumed,
    rhythm,
    readings,
    weather,
    habitat,
    daily,
    timeline,
});

impl SimState {
    /// The state before tick 0. Tick 0's day is already open, so the first close is at the next
    /// UTC midnight.
    pub fn new(basis: Basis) -> SimState {
        let open_day = utc_day(basis.grid, Tick::ZERO);
        SimState {
            basis,
            next_tick: Tick::ZERO,
            consumed: Consumed {
                commits: 0,
                observations: 0,
                commit_digest: InputDigest::EMPTY,
                observation_digest: InputDigest::EMPTY,
            },
            rhythm: Rhythm::new(open_day),
            readings: Readings::default(),
            weather: WeatherState::default(),
            habitat: HabitatState::default(),
            daily: Vec::new(),
            timeline: PresentationTimeline::default(),
        }
    }

    /// Applies every tick from `next_tick` through `through`. Nothing happens if `through` is
    /// already applied.
    pub fn advance_through(
        &mut self,
        prepared: &Prepared<'_>,
        through: Tick,
    ) -> Result<(), ReplayError> {
        if self.basis != prepared.basis {
            return Err(ReplayError::BasisChanged);
        }
        let mut commit = prepared
            .commits
            .partition_point(|c| c.tick < self.next_tick);
        let mut observation = prepared
            .observations
            .partition_point(|&(tick, _)| tick < self.next_tick);
        if self.consumed != prepared.consumed(commit, observation) {
            return Err(ReplayError::NeedsFullReplay);
        }
        if through < self.next_tick {
            return Ok(());
        }
        for hours in self.next_tick.hours()..=through.hours() {
            self.step(prepared, Tick::new(hours), &mut commit, &mut observation);
        }
        self.next_tick = through.saturating_add(1);
        self.consumed = prepared.consumed(commit, observation);
        Ok(())
    }

    fn step(
        &mut self,
        prepared: &Prepared<'_>,
        tick: Tick,
        commit: &mut usize,
        observation: &mut usize,
    ) {
        let day = utc_day(self.basis.grid, tick);
        if day != self.rhythm.open_day {
            self.close_day(tick, day);
        }
        while let Some(facts) = prepared.commits.get(*commit).filter(|c| c.tick == tick) {
            self.rhythm.count(facts);
            self.habitat.apply(facts);
            *commit += 1;
        }
        let mut observed = false;
        while let Some(&(_, recorded)) = prepared
            .observations
            .get(*observation)
            .filter(|&&(at, _)| at == tick)
        {
            self.observe(prepared, tick, recorded);
            observed = true;
            *observation += 1;
        }
        if observed {
            self.weather.evaluate(tick, &self.basis.default_branch);
        }
        self.evaluate(tick);
    }

    /// Light and soil take only reports recorded at the default tip whose commit is in the
    /// history. A report about a commit a rewrite removed is kept in the store but never
    /// applied.
    fn observe(&mut self, prepared: &Prepared<'_>, tick: Tick, observation: &Observation) {
        self.weather
            .note_head(observation.head.branch.as_deref(), observation.recorded_at);
        match &observation.value {
            ObservationValue::Weather { source } => {
                let pass = observation.normalized.map(|value| value >= PASS_FROM);
                self.weather.record(source, pass, tick);
            }
            ObservationValue::RefRewrite { .. } => {}
            value => {
                let head = &observation.head;
                if head.at_default_tip && prepared.applied_by(&head.commit, tick) {
                    let recorded = observation_tick(self.basis.grid, observation.recorded_at);
                    self.readings.record(
                        value.kind(),
                        Latest {
                            value: observation.normalized,
                            head: head.commit.clone(),
                            tick: recorded.unwrap_or(tick),
                            recorded_at: observation.recorded_at,
                        },
                    );
                }
            }
        }
    }

    fn close_day(&mut self, tick: Tick, day: i64) {
        self.rhythm.close(day);
        self.readings.close();
        let (rain, wind) = if tick.hours() < SETTLING_TICKS {
            (Q32::ONE, Q32::ONE)
        } else {
            (self.rhythm.rain.ratio(), self.rhythm.wind.ratio())
        };
        let light = self.readings.light.map(Q32::to_q16);
        let soil = self.readings.soil.map(Q32::to_q16);
        let raw = |latest: &Option<Latest>| latest.as_ref().and_then(|l| l.value);
        self.daily.push(DailyClimate {
            tick,
            light,
            soil,
            dead_code: raw(&self.readings.dead_code),
            complexity: raw(&self.readings.complexity),
            todo_density: raw(&self.readings.todo_density),
            rain: rain.to_q16(),
            wind: wind.to_q16(),
            health: climate::health(light, soil),
            activity: climate::activity(rain, wind),
        });
        let excess = self.daily.len().saturating_sub(DAILY_RECORDS);
        self.daily.drain(..excess);
    }

    fn evaluate(&mut self, tick: Tick) {
        let record = self.daily.last();
        let moment = Moment {
            tick,
            health: record.map_or(NEUTRAL, |r| r.health),
            health_before: tick
                .hours()
                .checked_sub(RECOVERY_WINDOW_TICKS)
                .map(|before| climate::health_at(&self.daily, Tick::new(before))),
            activity: record.map_or(Q16::HALF, |r| r.activity),
            inactive: tick.hours_since(self.habitat.last_commit.unwrap_or(Tick::ZERO))
                >= INACTIVE_TICKS,
        };
        self.timeline.evaluate(&moment);
    }

    /// The newest applied tick, or tick 0 before any.
    pub fn now(&self) -> Tick {
        Tick::new(self.next_tick.hours().saturating_sub(1))
    }

    pub fn habitat(&self) -> Habitat {
        self.habitat.at(self.now())
    }

    /// Days until rain and wind are read against the repository's own baseline: until the
    /// first close at or after 14 days of history. `None` once they are.
    pub fn settling_days_left(&self) -> Option<u32> {
        if self
            .daily
            .last()
            .is_some_and(|record| record.tick.hours() >= SETTLING_TICKS)
        {
            return None;
        }
        let first = first_close(self.basis.grid).hours();
        let settled =
            first + SETTLING_TICKS.saturating_sub(first).div_ceil(TICKS_PER_DAY) * TICKS_PER_DAY;
        Some(
            settled
                .saturating_sub(self.now().hours())
                .div_ceil(TICKS_PER_DAY),
        )
    }

    /// The climate as of the newest applied tick.
    pub fn report(&self) -> ClimateReport {
        let tip = self.habitat.default_tip.as_ref();
        let record = self.daily.last();
        let tempo = |ratio: Option<Q16>| Reading::Known {
            value: ratio.unwrap_or(Q16::ONE),
            stale: false,
            since_tick: record.map_or(Tick::ZERO, |r| r.tick),
        };
        ClimateReport {
            climate: Climate {
                light: self.readings.light_reading(tip),
                soil: self.readings.soil_reading(tip),
                rain: tempo(record.map(|r| r.rain)),
                wind: tempo(record.map(|r| r.wind)),
                weather: self.weather.reading(),
            },
            health: record.map_or(NEUTRAL, |r| r.health),
            activity: record.map_or(Q16::HALF, |r| r.activity),
            settling_days_left: self.settling_days_left(),
            trends: Trends::of(&self.daily),
            causes: SoilCauses {
                dead_code: Latest::reading(self.readings.dead_code.as_ref(), tip),
                complexity: Latest::reading(self.readings.complexity.as_ref(), tip),
                todo_density: Latest::reading(self.readings.todo_density.as_ref(), tip),
            },
        }
    }
}
