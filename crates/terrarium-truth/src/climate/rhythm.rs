//! Rain and wind: the repository's tempo, counted per UTC day.
//!
//! Rain is `log2(1 + commits)` of each day and wind `log2(1 + changed lines)`, both over the
//! commits that are not merges: a merge's first-parent diff repeats its branch's commits, which
//! are counted already. Each is smoothed with the 14-day EMA, and read relative to the median
//! of its own smoothed level over the last 90 days, never below the floor of one commit a week.
//! Taking the median of the smoothed level, not of the raw days, keeps a steady rhythm at ratio
//! 1 whatever its shape: a weekday-only repository does not read as slower every weekend, and
//! one that commits on fewer than half its days does not have a baseline of zero.
//!
//! A long silence is left out: a closed day without a sample enters the baseline only while
//! fewer than [`DROUGHT_DAYS`] such days came right before it, and the first day with a sample
//! after that many restarts the average from it, as the first day ever does. Otherwise a return
//! after months would be measured against a baseline sunk to its floor and read as a gale for
//! weeks. Each tempo counts its own days without a sample: rain a day with no commit that is
//! not a merge, wind a day with no changed line. The count is the tempo's own, not rain after
//! drought's, which reads hours since any commit; the two may differ by a day around midnight
//! or a merge-only day.
//!
//! The closed days' changed lines are also kept, for the windstorm: a day whose lines are at
//! least three times the median of the active days among the 90 before it. That reads a "daily
//! wind ratio ≥ 3" in lines rather than in the logarithm the wind is smoothed in, where a ratio
//! of 3 would take a day of millions of lines, yet a silence that leaves the smoothed baseline
//! at its floor would read as one for weeks.

use serde::{Deserialize, Serialize};

use crate::canon::canonical_struct;
use crate::domain::CommitFacts;
use crate::fixed::{Q16, Q32, log2_int};
use crate::keep_newest;
use crate::tuning::{
    BASELINE_DAYS, BASELINE_FLOOR, DROUGHT_DAYS, SMOOTHING_ALPHA, WINDSTORM_ACTIVE_DAYS,
    WINDSTORM_DAYS, WINDSTORM_RATIO,
};

/// One tempo, rain or wind: its smoothed level, the recent levels its baseline is taken from,
/// and the silence it is in. Kept in `Q32`, because a `Q16` average stops a few steps short of
/// a steady input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pace {
    /// The smoothed daily sample; `None` until the first day closes.
    pub level: Option<Q32>,
    /// The level after each of the last closes the baseline counts, oldest first, at most
    /// [`BASELINE_DAYS`].
    pub recent: Vec<Q32>,
    /// The closed days in a row without a sample, up to the last close.
    pub silent: u32,
}

canonical_struct!(Pace {
    level,
    recent,
    silent
});

impl Pace {
    /// Takes one closed day's sample. The first sample starts the average, so it never climbs
    /// from zero, and so does the first after a drought. A day without a sample still moves the
    /// average, but the baseline leaves it out once the silence is a drought.
    pub fn close(&mut self, sample: Q16) {
        let drought = self.silent >= DROUGHT_DAYS;
        let quiet = sample == Q16::ZERO;
        let sample = sample.to_q32();
        let level = match self.level {
            Some(level) if quiet || !drought => level.ema(sample, SMOOTHING_ALPHA),
            _ => sample,
        };
        self.level = Some(level);
        if !quiet || !drought {
            self.recent.push(level);
            keep_newest(&mut self.recent, BASELINE_DAYS);
        }
        self.silent = if quiet {
            self.silent.saturating_add(1)
        } else {
            0
        };
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
    /// The changed lines of each of the last `WINDSTORM_DAYS` closed days, oldest first.
    pub days: Vec<u64>,
}

canonical_struct!(Rhythm {
    open_day,
    commits,
    lines,
    rain,
    wind,
    days
});

impl Rhythm {
    pub fn new(open_day: i64) -> Rhythm {
        Rhythm {
            open_day,
            commits: 0,
            lines: 0,
            rain: Pace::default(),
            wind: Pace::default(),
            days: Vec::new(),
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

    /// Closes the open day into both tempos and opens `day`. Returns the closed day's lines
    /// over the usual active day's, rounded down, when they are at least `WINDSTORM_RATIO`
    /// times them.
    pub fn close(&mut self, day: i64) -> Option<u64> {
        self.rain.close(daily_sample(self.commits));
        self.wind.close(daily_sample(self.lines));
        let gust = gust(self.lines, &self.days);
        self.days.push(self.lines);
        keep_newest(&mut self.days, WINDSTORM_DAYS);
        self.commits = 0;
        self.lines = 0;
        self.open_day = day;
        gust
    }
}

/// `lines` over the median of the active (nonzero) days among `days`, rounded down, when it is
/// at least `WINDSTORM_RATIO` and `WINDSTORM_ACTIVE_DAYS` of them were active. An even count's
/// median is the mean of the middle two, compared without rounding: `2 × lines` against
/// `ratio × (a + b)`.
pub fn gust(lines: u64, days: &[u64]) -> Option<u64> {
    let mut active: Vec<u64> = days.iter().copied().filter(|&d| d > 0).collect();
    if active.len() < WINDSTORM_ACTIVE_DAYS {
        return None;
    }
    // By key, because a plain `sort` of integers trips `stable_sort_primitive`, whose fix,
    // `sort_unstable`, truth disallows. Equal counts are interchangeable either way.
    active.sort_by_key(|&lines| lines);
    let upper = active.len() / 2;
    let twice_median = if active.len() % 2 == 1 {
        active.get(upper)?.saturating_mul(2)
    } else {
        active
            .get(upper.checked_sub(1)?)?
            .saturating_add(*active.get(upper)?)
    };
    let ratio = lines.saturating_mul(2).checked_div(twice_median)?;
    (ratio >= WINDSTORM_RATIO).then_some(ratio)
}

/// `log2(1 + n)`.
pub(crate) fn daily_sample(n: u64) -> Q16 {
    log2_int(n.saturating_add(1)).unwrap_or(Q16::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::{ACTIVE_FROM, TEMPO_MAX};

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
        // The silent days past the first `DROUGHT_DAYS` leave the baseline.
        assert_eq!(pace.recent.len(), usize::try_from(DROUGHT_DAYS).unwrap());
        assert_eq!(pace.silent, 100);
        assert_eq!(pace.ratio(), Q32::ZERO);
        pace.close(daily_sample(1));
        // One commit after a drought restarts the average at log2(2), over a baseline of
        // silent levels, which sits at the floor.
        assert_eq!((pace.level, pace.silent), (Some(Q32::ONE), 0));
        assert_eq!(pace.ratio(), Q32::ONE.saturating_div(BASELINE_FLOOR));
    }

    /// The tempo as it was before droughts were left out: every closed day enters the
    /// baseline, and the average never restarts.
    #[derive(Default)]
    struct EveryDay {
        level: Option<Q32>,
        recent: Vec<Q32>,
    }

    impl EveryDay {
        fn close(&mut self, sample: Q16) {
            let sample = sample.to_q32();
            let level = self
                .level
                .map_or(sample, |level| level.ema(sample, SMOOTHING_ALPHA));
            self.level = Some(level);
            self.recent.push(level);
            keep_newest(&mut self.recent, BASELINE_DAYS);
        }

        fn ratio(&self) -> Q32 {
            let baseline = median(&self.recent).map_or(BASELINE_FLOOR, |m| m.max(BASELINE_FLOOR));
            self.level
                .map_or(Q32::ONE, |level| level.saturating_div(baseline))
        }
    }

    #[test]
    fn rhythms_without_a_drought_read_as_before() {
        // Five commits once a week, one a week, and three every third day.
        for (every, commits) in [(7, 5), (7, 1), (3, 3)] {
            let mut pace = Pace::default();
            let mut every_day = EveryDay::default();
            for day in 0..400 {
                let sample = daily_sample(if day % every == 0 { commits } else { 0 });
                pace.close(sample);
                every_day.close(sample);
                assert_eq!(pace.ratio(), every_day.ratio(), "every {every}, day {day}");
            }
        }
    }

    #[test]
    fn a_return_after_a_long_silence_reads_at_its_own_pace() {
        let commit = CommitFacts {
            id: crate::CommitId::from_hex(&"b".repeat(40)).expect("an id"),
            tick: crate::Tick::ZERO,
            parents: 1,
            on_default_first_parent: true,
            added: 30,
            removed: 10,
            deleted_dirs: Vec::new(),
            renames: Vec::new(),
            tags: Vec::new(),
        };
        let mut rhythm = Rhythm::new(0);
        let day = |rhythm: &mut Rhythm, open: i64, commits: bool| {
            if commits {
                rhythm.count(&commit);
                rhythm.count(&commit);
            }
            let _ = rhythm.close(open + 1);
            crate::climate::activity(rhythm.rain.ratio(), rhythm.wind.ratio())
        };
        // Two commits each weekday for 200 days, then 120 silent days.
        for open in 0..320 {
            let _ = day(&mut rhythm, open, open < 200 && open % 7 < 5);
        }
        // The same rhythm again: from the close of the first commit day back, the repository
        // reads as active, and never at the tempo's cap.
        let mut back = false;
        for open in 320..380 {
            let commits = open % 7 < 5;
            let activity = day(&mut rhythm, open, commits);
            back |= commits;
            assert!(rhythm.rain.ratio() < TEMPO_MAX.to_q32(), "day {open}");
            if back {
                assert!(activity >= ACTIVE_FROM, "day {open}: {activity:?}");
            }
        }
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
        let _ = rhythm.close(1);
        assert_eq!((rhythm.commits, rhythm.lines, rhythm.open_day), (0, 0, 1));
        assert_eq!(rhythm.rain.level, Some(Q32::ONE));
        assert_eq!(rhythm.wind.level, log2_int(15).map(Q16::to_q32));
    }

    #[test]
    fn a_gust_is_three_times_the_median_active_day() {
        // Seven active days of 10 lines, and idle days, which never count.
        let days = [10, 0, 10, 10, 0, 10, 10, 10, 10];
        assert_eq!(gust(29, &days), None);
        assert_eq!(gust(30, &days), Some(3));
        assert_eq!(gust(119, &days), Some(11));
        // Six active days are too few to tell what is usual.
        assert_eq!(gust(1_000, &days[..7]), None);
        // An even count's median is the mean of the middle two, unrounded: 10 and 13.
        let even = [10, 10, 10, 10, 13, 13, 13, 13];
        assert_eq!(gust(34, &even), None);
        assert_eq!(gust(35, &even), Some(3));
        // The day itself is not among the days it is compared with: after six usual days, a
        // seventh of any size has only six to be compared with.
        let mut first = Rhythm::new(0);
        for day in 1..=6 {
            first.lines = 10;
            assert_eq!(first.close(day), None);
        }
        first.lines = 1_000;
        assert_eq!(first.close(7), None);
        let mut rhythm = Rhythm::new(0);
        for day in 1..=7 {
            rhythm.lines = 10;
            assert_eq!(rhythm.close(day), None);
        }
        rhythm.lines = 60;
        assert_eq!(rhythm.close(8), Some(6));
        assert_eq!(rhythm.days.len(), 8);
        for day in 9..=200 {
            let _ = rhythm.close(day);
        }
        assert_eq!(rhythm.days.len(), WINDSTORM_DAYS);
    }
}
