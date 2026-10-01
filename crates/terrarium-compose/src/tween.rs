//! Tweens: every visible change eases between two values over seconds, and nothing ever cuts
//! (I-15, constraint 7). The clock is passed in; the composer never reads one.
//!
//! The arithmetic is integer only, so a tween gives the same value on every platform.

/// Milliseconds on the injected monotonic clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Millis(pub u64);

impl Millis {
    #[must_use]
    pub fn plus(self, ms: u32) -> Millis {
        Millis(self.0.saturating_add(u64::from(ms)))
    }
}

/// How long the composer's changes take (state transitions over 3–10 s).
pub mod durations {
    /// A plant fading in where nothing stood.
    pub const APPEAR_MS: u32 = 4000;
    /// A plant fading out.
    pub const VANISH_MS: u32 = 4000;
    /// A new look replacing an old one: it rises for the first half, then the old falls.
    pub const RESTYLE_MS: u32 = 6000;
    /// Filler fading in or out.
    pub const FILL_MS: u32 = 3000;
    /// The shortest of them, which bounds how fast any opacity moves.
    pub const SHORTEST_MS: u32 = RESTYLE_MS / 2;
}

/// One value easing from `from` to `to` with smoothstep, starting at `start` and lasting `ms`.
/// It holds `from` until `start`, so a tween can wait its turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tween {
    from: u8,
    to: u8,
    start: Millis,
    ms: u32,
}

/// 1.0 in the tween's fixed point.
const ONE: u64 = 1 << 16;

impl Tween {
    pub const fn new(from: u8, to: u8, start: Millis, ms: u32) -> Tween {
        Tween {
            from,
            to,
            start,
            ms,
        }
    }

    /// A value that is already where it is going.
    pub const fn at(value: u8) -> Tween {
        Tween::new(value, value, Millis(0), 0)
    }

    pub const fn target(self) -> u8 {
        self.to
    }

    /// When the value reaches its target and stops.
    pub fn end(self) -> Millis {
        self.start.plus(self.ms)
    }

    pub fn value(self, now: Millis) -> u8 {
        if now <= self.start {
            return self.from;
        }
        let elapsed = now.0 - self.start.0;
        if elapsed >= u64::from(self.ms) {
            return self.to;
        }
        // Progress and smoothstep, both in 16.16: s = t²(3 − 2t).
        let t = elapsed * ONE / u64::from(self.ms);
        let s = t * t * (3 * ONE - 2 * t) / (ONE * ONE);
        let from = i64::from(self.from);
        let span = i64::from(self.to) - from;
        let scaled = from * ONE.cast_signed() + span * s.cast_signed();
        // Rounded to nearest; the sum is never negative, so the shift floors correctly.
        u8::try_from((scaled + (ONE / 2).cast_signed()) >> 16).unwrap_or(self.to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tween_holds_before_its_start_and_lands_on_its_target() {
        let tween = Tween::new(40, 200, Millis(1000), 3000);
        assert_eq!(tween.value(Millis(0)), 40);
        assert_eq!(tween.value(Millis(1000)), 40);
        assert_eq!(tween.value(Millis(2500)), 120);
        assert_eq!(tween.value(Millis(4000)), 200);
        assert_eq!(tween.value(Millis(u64::MAX)), 200);
        assert_eq!(tween.end(), Millis(4000));
        assert_eq!(Tween::at(77).value(Millis(5)), 77);
    }

    #[test]
    fn every_millisecond_moves_at_most_one_step_and_never_backwards() {
        // Exhaustive over every duration the composer uses, rising and falling: the slowest
        // ramp moves at most one unit in a millisecond, so no frame can ever see a cut.
        for ms in [
            durations::APPEAR_MS,
            durations::VANISH_MS,
            durations::RESTYLE_MS / 2,
            durations::FILL_MS,
        ] {
            for (from, to) in [(0, 255), (255, 0), (17, 140), (200, 3)] {
                let tween = Tween::new(from, to, Millis(500), ms);
                let mut last = tween.value(Millis(0));
                for t in 1..=u64::from(ms) + 1000 {
                    let value = tween.value(Millis(t));
                    assert!(value.abs_diff(last) <= 1, "{from}→{to} over {ms} at {t}");
                    if to > from {
                        assert!(value >= last);
                    } else {
                        assert!(value <= last);
                    }
                    last = value;
                }
                assert_eq!(last, to);
            }
        }
    }

    #[test]
    fn smoothstep_eases_in_and_out() {
        let tween = Tween::new(0, 255, Millis(0), 1000);
        // Slow at the ends, fast in the middle, and symmetric about it.
        assert!(tween.value(Millis(100)) < 255 / 10);
        assert_eq!(tween.value(Millis(500)), 128);
        assert_eq!(tween.value(Millis(100)), 255 - tween.value(Millis(900)));
    }
}
