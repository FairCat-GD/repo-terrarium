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
    /// Filler, weeds, litter, fungus and the decorative tufts fading in or out.
    pub const FILL_MS: u32 = 3000;
    /// The weather band, a cloud, the sun, the moon, a star or a glass film fading in or out.
    pub const SKY_MS: u32 = 6000;
    /// The shortest change of a plant or the ground, which bounds how fast their opacity moves.
    /// The light's crossfade is the art's own and shorter, over a faint overlay, and the
    /// label's is its own.
    pub const SHORTEST_MS: u32 = RESTYLE_MS / 2;
    /// How long the label shows a line before the next.
    pub const LABEL_CYCLE_MS: u32 = 20_000;
    /// The label's crossfade between lines, and between one set of lines and the next.
    pub const LABEL_FADE_MS: u32 = 1500;
    /// The soil panel rising out of the soil band, or lowering into it.
    pub const RISE_MS: u32 = 300;
    /// A new page of the panel covering the one shown, then the old one falling.
    pub const PAGE_MS: u32 = 300;
    /// The panel's row pointer moving from one row to another.
    pub const POINTER_MS: u32 = 150;
    /// A hover label over a monument or a fossil fading in or out.
    pub const TAG_MS: u32 = 300;
    /// Dim mode settling in, slowly, once the machine has been idle.
    pub const DIM_DOWN_MS: u32 = 10_000;
    /// Dim mode lifting when someone is back: quick, but still a fade.
    pub const DIM_UP_MS: u32 = 3000;
    /// The palette easing to a clock that disagrees with the time carried forward.
    pub const CLOCK_FIX_MS: u32 = 6000;
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

    /// The first moment after `after` at which the value differs from its value at `after`,
    /// or `None` if it never will. The value only ever moves toward its target, so the moment
    /// is found by halving.
    pub fn next_step(self, after: Millis) -> Option<Millis> {
        self.next_step_by(after, |value| value)
    }

    /// As [`Tween::next_step`], for what `seen` makes of the value: the first moment it shows
    /// otherwise. `seen` must never turn back as the value moves on, as an opacity scaled by
    /// a sprite's own never does, so a step it hides is passed over.
    pub fn next_step_by(self, after: Millis, seen: impl Fn(u8) -> u8) -> Option<Millis> {
        let now = seen(self.value(after));
        let end = self.end();
        if after >= end || now == seen(self.to) {
            return None;
        }
        // What `end` shows is the target's, which differs from `now`: the step lies in
        // (lo, hi].
        let (mut lo, mut hi) = (after.0.max(self.start.0), end.0);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if seen(self.value(Millis(mid))) == now {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Some(Millis(hi))
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
        // Every duration of the scene's own fades and dim mode's, rising and falling: each
        // ramp moves at most one unit in a millisecond, so no frame can ever see a cut. The
        // hover layer's quicker fades and a time-lapse's label step further, and the continuity
        // property bounds them by their own pace.
        for ms in [
            durations::APPEAR_MS,
            durations::VANISH_MS,
            durations::RESTYLE_MS / 2,
            durations::FILL_MS,
            durations::SKY_MS,
            durations::LABEL_FADE_MS,
            durations::DIM_UP_MS,
            durations::DIM_DOWN_MS,
            durations::CLOCK_FIX_MS,
            // The light overlays' delivered crossfade.
            1200,
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
    fn the_next_step_is_the_first_millisecond_the_value_moves() {
        // Against stepping one millisecond at a time, from every moment of several tweens.
        for (from, to, start, ms) in [
            (0, 255, 500, durations::RESTYLE_MS / 2),
            (255, 0, 0, durations::LABEL_FADE_MS),
            (17, 140, 40, 1200),
            (200, 199, 10, 3000),
            (9, 9, 0, 400),
        ] {
            let tween = Tween::new(from, to, Millis(start), ms);
            let last = start + u64::from(ms) + 5;
            for t in 0..=last {
                let after = Millis(t);
                let expected = (t + 1..=last)
                    .find(|&s| tween.value(Millis(s)) != tween.value(after))
                    .map(Millis);
                assert_eq!(tween.next_step(after), expected, "{from}→{to} at {t}");
            }
        }
        assert_eq!(Tween::at(4).next_step(Millis(0)), None);
    }

    #[test]
    fn a_step_hidden_by_a_faint_opacity_is_passed_over() {
        // Against stepping one millisecond at a time, for faint and full art.
        for own in [26_u16, 34, 100, 255] {
            let seen = |value: u8| u8::try_from((u16::from(value) * own + 127) / 255).unwrap();
            for tween in [
                Tween::new(0, 255, Millis(10), 1200),
                Tween::new(255, 0, Millis(0), 3000),
            ] {
                let last = tween.end().0 + 5;
                for t in (0..=last).step_by(7) {
                    let after = Millis(t);
                    let expected = (t + 1..=last)
                        .find(|&s| seen(tween.value(Millis(s))) != seen(tween.value(after)))
                        .map(Millis);
                    assert_eq!(tween.next_step_by(after, seen), expected, "{own} at {t}");
                }
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
