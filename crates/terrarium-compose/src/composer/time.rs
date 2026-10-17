//! The scene's clock, which a pause holds and a time-lapse runs faster, and the palette's time
//! of day, eased over when a new clock disagrees.

use crate::blend::mix;
use crate::tween::durations::CLOCK_FIX_MS;
use crate::tween::{Millis, Tween};

use super::{Composer, OverlayInput, SHOWN, TIME_LAPSE_PACE};

/// The scene's own clock: the composer's, held through a pause and run [`TIME_LAPSE_PACE`]
/// times faster in a time-lapse. The label, dim mode and the hover layer keep the composer's
/// clock; everything else is the scene's, so a resume carries on from the very picture the
/// pause held.
///
/// It is anchored where its rate last changed and runs at that rate from there; before the
/// anchor it runs at the composer's own pace, which is how a pause has always left it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SceneClock {
    /// The composer's moment at the anchor.
    pub(super) real: Millis,
    /// The scene's moment at the anchor.
    pub(super) scene: Millis,
    /// Scene milliseconds per composer millisecond from the anchor: 0 while paused.
    pub(super) rate: u64,
}

impl Default for SceneClock {
    fn default() -> SceneClock {
        SceneClock {
            real: Millis(0),
            scene: Millis(0),
            rate: 1,
        }
    }
}

impl SceneClock {
    /// Pauses, resumes, or runs at the time-lapse's pace from `now`; the scene's moment at
    /// `now` stays where it was.
    pub(super) fn set(&mut self, paused: bool, time_lapse: bool, now: Millis) {
        let rate = if paused {
            0
        } else if time_lapse {
            u64::from(TIME_LAPSE_PACE)
        } else {
            1
        };
        if rate != self.rate {
            *self = SceneClock {
                real: now,
                scene: self.scene(now),
                rate,
            };
        }
    }

    /// The scene's moment at the composer's `now`.
    pub(super) fn scene(self, now: Millis) -> Millis {
        match now.0.checked_sub(self.real.0) {
            Some(after) => Millis(self.scene.0.saturating_add(after.saturating_mul(self.rate))),
            None => Millis(self.scene.0.saturating_sub(self.real.0 - now.0)),
        }
    }

    /// The composer's first moment at which the scene has reached `scene`, while the scene
    /// runs; a paused scene reaches no moment.
    pub(super) fn real(self, scene: Millis) -> Option<Millis> {
        if self.rate == 0 {
            return None;
        }
        Some(match scene.0.checked_sub(self.scene.0) {
            Some(after) => Millis(self.real.0.saturating_add(after.div_ceil(self.rate))),
            None => Millis(self.real.0.saturating_sub(self.scene.0 - scene.0)),
        })
    }
}

/// The palette mix an update found standing, and how far the new clock's mix has taken over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MixFix {
    pub(super) from: u8,
    pub(super) weight: Tween,
}

impl Composer {
    /// Takes the overlay's clock and day and night times. Where they put the palette
    /// somewhere other than it stands, it eases over rather than jumping: after a sleep, a
    /// change of time zone or the first update.
    pub(super) fn update_clock(&mut self, overlay: &OverlayInput, now: Millis) {
        let before = self.mix_at(now);
        self.clock = Some((overlay.clock, now));
        self.day_night = overlay.day_night;
        let after = self.clock_mix(now);
        self.mix_fix = (before != after).then(|| MixFix {
            from: before,
            weight: Tween::new(0, SHOWN, now, CLOCK_FIX_MS),
        });
    }

    /// The local time of day at `now`, carried forward from the last update's clock.
    pub(super) fn ms_of_day(&self, now: Millis) -> Option<u32> {
        let (clock, at) = self.clock?;
        Some(clock.advanced_by(now.0.saturating_sub(at.0)).ms_of_day)
    }

    /// The palette mix the clock alone gives at `now`: day until a clock arrives.
    pub(super) fn clock_mix(&self, now: Millis) -> u8 {
        self.ms_of_day(now).map_or(0, |ms| self.day_night.mix(ms))
    }

    /// The palette mix at `now`, 0 by day and 255 by night. It is the scene's, and a pause
    /// holds it.
    pub fn palette_mix(&self, now: Millis) -> u8 {
        self.mix_at(self.pause.scene(now))
    }

    /// The palette mix at the scene's moment `now`.
    pub(super) fn mix_at(&self, now: Millis) -> u8 {
        let by_clock = self.clock_mix(now);
        match self.mix_fix {
            Some(fix) => mix(fix.from, by_clock, fix.weight.value(now)),
            None => by_clock,
        }
    }
}
