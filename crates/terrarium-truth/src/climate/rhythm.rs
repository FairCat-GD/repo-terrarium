//! Rain and wind: the repository's tempo, counted per UTC day.
//!
//! Rain is `log2(1 + commits)` of each day and wind `log2(1 + changed lines)`, both over the
//! commits that are not merges: a merge's first-parent diff repeats its branch's commits, which
//! are counted already. Each is smoothed with the 14-day EMA, and read relative to the median
//! of its own smoothed level over the last 90 days, never below the floor of one commit a week.
//! Taking the median of the smoothed level, not of the raw days, keeps a steady rhythm at ratio
//! 1 whatever its shape: a weekday-only repository does not read as slower every weekend, and
//! one that commits on fewer than half its days does not have a baseline of zero.

use serde::{Deserialize, Serialize};

use crate::canon::canonical_struct;
use crate::domain::CommitFacts;
use crate::fixed::{Q16, Q32, log2_int};
use crate::tuning::{BASELINE_DAYS, BASELINE_FLOOR, SMOOTHING_ALPHA};

/// One tempo, rain or wind: its smoothed level, and the recent levels its baseline is taken
/// from. Kept in `Q32`, because a `Q16` average stops a few steps short of a steady input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pace {
    /// The smoothed daily sample; `None` until the first day closes.
    pub level: Option<Q32>,
    /// The level after each of the last closes, oldest first, at most [`BASELINE_DAYS`].
    pub recent: Vec<Q32>,
}

canonical_struct!(Pace { level, recent });

impl Pace {
    /// Takes one closed day's sample. The first sample starts the average, so it never climbs
    /// from zero.
    pub fn close(&mut self, sample: Q16) {
        let sample = sample.to_q32();
        let level = self
            .level
            .map_or(sample, |level| level.ema(sample, SMOOTHING_ALPHA));
        self.level = Some(level);
        self.recent.push(level);
        let excess = self.recent.len().saturating_sub(BASELINE_DAYS);
        self.recent.drain(..excess);
    }

    /// The level over the baseline: 1 is the repository's usual pace.
    pub fn ratio(&self) -> Q32 {
        let Some(level) = self.level else {
            return Q32::ONE;
        };
        let baseline = median(&self.recent).map_or(BASELINE_FLOOR, |m| m.max(BASELINE_FLOOR));
        level.saturating_div(baseline)
    }
}

/// The middle value, or the mean of the two middle values (rounded half away from zero) of an
/// even count. `None` for no values.
pub fn median(values: &[Q32]) -> Option<Q32> {
    let mut sorted = values.to_vec();
    sorted.sort();
    let upper = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        return sorted.get(upper).copied();
    }
    let low = *sorted.get(upper.checked_sub(1)?)?;
    let high = *sorted.get(upper)?;
    Some(low.saturating_add(high).saturating_mul(Q32::HALF))
}

/// The open day's tallies and the two tempos.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rhythm {
    /// The UTC day the tallies count, in days since 1970-01-01.
    pub open_day: i64,
    /// The open day's commits that are not merges, and the lines they changed.
    pub commits: u64,
    pub lines: u64,
    pub rain: Pace,
    pub wind: Pace,
}

canonical_struct!(Rhythm {
    open_day,
    commits,
    lines,
    rain,
    wind
});

impl Rhythm {
    pub fn new(open_day: i64) -> Rhythm {
        Rhythm {
            open_day,
            commits: 0,
            lines: 0,
            rain: Pace::default(),
            wind: Pace::default(),
        }
    }

    /// Counts a commit into the open day. Merges add nothing.
    pub fn count(&mut self, commit: &CommitFacts) {
        if commit.parents >= 2 {
            return;
        }
        self.commits = self.commits.saturating_add(1);
        self.lines = self
            .lines
            .saturating_add(commit.added.saturating_add(commit.removed));
    }

    /// Closes the open day into both tempos and opens `day`.
    pub fn close(&mut self, day: i64) {
        self.rain.close(daily_sample(self.commits));
        self.wind.close(daily_sample(self.lines));
        self.commits = 0;
        self.lines = 0;
        self.open_day = day;
    }
}

/// `log2(1 + n)`.
pub fn daily_sample(n: u64) -> Q16 {
    log2_int(n.saturating_add(1)).unwrap_or(Q16::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(raw: i64) -> Q32 {
        Q32::from_raw(raw)
    }

    #[test]
    fn the_median_takes_the_middle_or_the_mean_of_two() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[q(7)]), Some(q(7)));
        assert_eq!(median(&[q(9), q(1), q(5)]), Some(q(5)));
        assert_eq!(median(&[q(9), q(1), q(5), q(2)]), Some(q(4)));
        // (2 + 3) / 2 = 2.5 ULP, a tie, rounds away from zero.
        assert_eq!(median(&[q(3), q(2)]), Some(q(3)));
    }

    #[test]
    fn the_first_sample_starts_the_average() {
        let mut pace = Pace::default();
        assert_eq!(pace.ratio(), Q32::ONE);
        pace.close(Q16::from_int(2));
        assert_eq!(pace.level, Some(Q32::from_int(2)));
        // One level is its own median, so the ratio is 1.
        assert_eq!(pace.ratio(), Q32::ONE);
        pace.close(Q16::ZERO);
        assert_eq!(
            pace.level,
            Some(Q32::from_int(2).ema(Q32::ZERO, SMOOTHING_ALPHA))
        );
    }

    #[test]
    fn the_baseline_never_drops_below_the_floor() {
        let mut pace = Pace::default();
        for _ in 0..100 {
            pace.close(Q16::ZERO);
        }
        assert_eq!(pace.level, Some(Q32::ZERO));
        assert_eq!(pace.recent.len(), BASELINE_DAYS);
        assert_eq!(pace.ratio(), Q32::ZERO);
        pace.close(daily_sample(1));
        // One commit after a silence: 2/15 of log2(2) over the floor.
        let expected = Q32::from_ratio(2, 15).saturating_div(BASELINE_FLOOR);
        assert_eq!(pace.ratio(), expected);
    }

    #[test]
    fn a_steady_weekday_rhythm_reads_as_usual() {
        let mut pace = Pace::default();
        for day in 0..200 {
            let commits = if day % 7 < 5 { 2 } else { 0 };
            pace.close(daily_sample(commits));
        }
        let ratio = pace.ratio();
        assert!(
            Q32::from_ratio(8, 10) < ratio && ratio < Q32::from_ratio(12, 10),
            "{ratio:?}"
        );
    }

    #[test]
    fn merges_add_neither_rain_nor_wind() {
        let id = crate::CommitId::from_hex(&"a".repeat(40)).expect("an id");
        let mut commit = CommitFacts {
            id,
            tick: crate::Tick::ZERO,
            parents: 2,
            on_default_first_parent: true,
            added: 10,
            removed: 4,
            deleted_dirs: Vec::new(),
            renames: Vec::new(),
            tags: Vec::new(),
        };
        let mut rhythm = Rhythm::new(0);
        rhythm.count(&commit);
        assert_eq!((rhythm.commits, rhythm.lines), (0, 0));
        commit.parents = 1;
        rhythm.count(&commit);
        assert_eq!((rhythm.commits, rhythm.lines), (1, 14));
        rhythm.close(1);
        assert_eq!((rhythm.commits, rhythm.lines, rhythm.open_day), (0, 0, 1));
        assert_eq!(rhythm.rain.level, Some(Q32::ONE));
        assert_eq!(rhythm.wind.level, log2_int(15).map(Q16::to_q32));
    }
}
