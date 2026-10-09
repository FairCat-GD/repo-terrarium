//! Day and night on the local clock: the palette crossfades from day to night when the evening
//! begins and back when the day does, over 45 minutes. Local time is the composer's, never
//! truth's; the app reads it and passes it in.

use crate::sky::LocalDate;
use crate::tween::{Millis, Tween};

/// Milliseconds in a local day.
pub const DAY_MS: u32 = 86_400_000;
/// How long the palette takes to cross from day to night, or back.
pub const CROSSFADE_MS: u32 = 45 * 60_000;
/// The palette mix from which residents keep night hours: past halfway to night.
pub const NIGHT_FROM: u8 = 128;

/// The local wall time: a date and the milliseconds since its midnight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalClock {
    pub date: LocalDate,
    pub ms_of_day: u32,
}

impl LocalClock {
    pub const fn new(date: LocalDate, ms_of_day: u32) -> LocalClock {
        LocalClock { date, ms_of_day }
    }

    /// Noon on `date`.
    pub const fn noon(date: LocalDate) -> LocalClock {
        LocalClock::new(date, DAY_MS / 2)
    }

    /// Midnight at the start of `date`.
    pub const fn midnight(date: LocalDate) -> LocalClock {
        LocalClock::new(date, 0)
    }

    /// The time of day `elapsed_ms` later, wrapping at midnight. The date stays as it is: a
    /// clock carried on between updates only keeps the time of day going.
    #[must_use]
    pub fn advanced_by(self, elapsed_ms: u64) -> LocalClock {
        let day = u64::from(DAY_MS);
        let ms = (u64::from(self.ms_of_day) + elapsed_ms % day) % day;
        LocalClock::new(self.date, u32::try_from(ms).unwrap_or(0))
    }
}

/// When the day and the night begin, in milliseconds after local midnight: 07:00 and 19:00
/// unless the settings say otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DayNight {
    pub day_starts: u32,
    pub night_starts: u32,
}

impl Default for DayNight {
    fn default() -> Self {
        DayNight {
            day_starts: 7 * 3_600_000,
            night_starts: 19 * 3_600_000,
        }
    }
}

impl DayNight {
    /// The palette mix at a local time: 0 by day, 255 by night, and a smoothstep for the 45
    /// minutes after each begins. A crossfade that starts before the last one has finished
    /// starts from where that one stands, so the mix never jumps.
    pub fn mix(self, ms_of_day: u32) -> u8 {
        let day = self.day_starts % DAY_MS;
        let night = self.night_starts % DAY_MS;
        if day == night {
            return 0;
        }
        let ms = ms_of_day % DAY_MS;
        let since = |start: u32| (ms + DAY_MS - start) % DAY_MS;
        let (latest, target, other) = if since(night) < since(day) {
            (night, u8::MAX, day)
        } else {
            (day, 0, night)
        };
        // Where the crossfade before this one, the other way, stood when this one began.
        let gap = (latest + DAY_MS - other) % DAY_MS;
        let previous = Tween::new(target, u8::MAX - target, Millis(0), CROSSFADE_MS);
        let from = previous.value(Millis(u64::from(gap)));
        Tween::new(from, target, Millis(0), CROSSFADE_MS).value(Millis(u64::from(since(latest))))
    }

    /// How long after `ms_of_day` the mix next changes, or `None` if it never does. The mix
    /// moves only in the 45 minutes after each start and one way in each, so the change is the
    /// first one in the crossfade running now, else the first in the next to begin.
    pub fn next_change(self, ms_of_day: u32) -> Option<u32> {
        let day = self.day_starts % DAY_MS;
        let night = self.night_starts % DAY_MS;
        if day == night {
            return None;
        }
        let ms = ms_of_day % DAY_MS;
        let now = self.mix(ms);
        let at = |ahead: u32| self.mix((ms + ahead) % DAY_MS);
        [day, night]
            .into_iter()
            .filter_map(|start| {
                let since = (ms + DAY_MS - start) % DAY_MS;
                let (lo, hi) = if since < CROSSFADE_MS {
                    (0, CROSSFADE_MS - since)
                } else {
                    let ahead = DAY_MS - since;
                    (ahead, ahead + CROSSFADE_MS)
                };
                if at(lo) != now || at(hi) == now {
                    return None;
                }
                // The value moves one way between `lo` and `hi`: halve to its first move.
                let (mut lo, mut hi) = (lo, hi);
                while hi - lo > 1 {
                    let mid = lo + (hi - lo) / 2;
                    if at(mid) == now {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                Some(hi)
            })
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u32 = 3_600_000;
    const MINUTE: u32 = 60_000;

    #[test]
    fn day_is_day_night_is_night_and_the_evening_crosses_over_45_minutes() {
        let times = DayNight::default();
        assert_eq!(times.mix(12 * HOUR), 0);
        assert_eq!(times.mix(0), 255);
        assert_eq!(times.mix(3 * HOUR), 255);
        assert_eq!(times.mix(19 * HOUR), 0, "the evening begins at 19:00");
        assert_eq!(times.mix(19 * HOUR + CROSSFADE_MS), 255);
        let half = times.mix(19 * HOUR + CROSSFADE_MS / 2);
        assert!((127..=128).contains(&half), "{half}");
        assert_eq!(times.mix(7 * HOUR), 255, "the morning begins at 07:00");
        assert_eq!(times.mix(7 * HOUR + CROSSFADE_MS), 0);
        assert_eq!(times.mix(DAY_MS + 12 * HOUR), 0, "a day later is the same");
    }

    #[test]
    fn the_mix_moves_at_most_one_step_a_second_and_never_backwards() {
        let times = DayNight::default();
        for (from, to, rising) in [(19 * HOUR, 20 * HOUR, true), (7 * HOUR, 8 * HOUR, false)] {
            let mut last = times.mix(from);
            for ms in (from..to).step_by(1000) {
                let mix = times.mix(ms);
                assert!(mix.abs_diff(last) <= 1, "{ms}");
                assert!(if rising { mix >= last } else { mix <= last });
                last = mix;
            }
        }
    }

    #[test]
    fn the_next_change_is_where_the_mix_first_moves() {
        let times = DayNight::default();
        for ms in [
            0,
            12 * HOUR,
            19 * HOUR - 1,
            19 * HOUR,
            19 * HOUR + 1,
            19 * HOUR + 7 * MINUTE + 333,
            19 * HOUR + CROSSFADE_MS - 1,
            7 * HOUR + 20 * MINUTE,
            DAY_MS - 1,
        ] {
            let now = times.mix(ms);
            let ahead = times.next_change(ms).expect("the mix moves every day");
            assert!(ahead > 0);
            assert_ne!(times.mix((ms + ahead) % DAY_MS), now, "{ms}");
            // Unchanged up to it, sampled a thousand times.
            for k in 0..1000_u32 {
                let step = u32::try_from(u64::from(ahead) * u64::from(k) / 1000).unwrap();
                assert_eq!(times.mix((ms + step) % DAY_MS), now, "{ms} + {step}");
            }
        }
        let equal = DayNight {
            day_starts: 0,
            night_starts: 0,
        };
        assert_eq!(equal.next_change(5), None);
    }

    #[test]
    fn a_crossfade_cut_short_carries_on_from_where_it_stood() {
        // Night only 20 minutes after day: the day never fully arrives, and nothing jumps.
        let times = DayNight {
            day_starts: 7 * HOUR,
            night_starts: 7 * HOUR + 20 * MINUTE,
        };
        let mut last = times.mix(6 * HOUR);
        for ms in (6 * HOUR..9 * HOUR).step_by(1000) {
            let mix = times.mix(ms);
            assert!(mix.abs_diff(last) <= 1, "{ms}: {last} → {mix}");
            last = mix;
        }
        let equal = DayNight {
            day_starts: HOUR,
            night_starts: HOUR,
        };
        assert_eq!(equal.mix(0), 0);
    }
}
