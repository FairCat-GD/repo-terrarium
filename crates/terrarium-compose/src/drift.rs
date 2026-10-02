//! The slow motions that keep any region of an OLED strip from holding still (N-12): the sky
//! band drifting right, the soil's texture drifting a pixel every few minutes, and a faint
//! reflection sweeping across the glass every four to seven minutes. Each is a pure function of
//! the composer's clock, and each stands at its start for the first six seconds, while a fresh
//! composer fades the scene in.

use crate::catalog::{Art, SpriteId};
use crate::salt::{InstallSalt, Purpose};
use crate::tween::Millis;

/// The sky band moves a pixel to the right this often: slower than any cloud, as the farther
/// layer (constraint 5, "the sky band only drifts").
pub const BAND_MS_PER_PX: u64 = 12_000;
/// The soil's texture moves a pixel to the right this often.
pub const SOIL_MS_PER_PX: u64 = 180_000;
/// The soil's texture repeats every three tiles: two of one variant, then one of the other.
pub const SOIL_PERIOD: i32 = 24;
/// Each sweep has a slot of its own, and starts at a salted moment in the slot's first
/// [`SWEEP_JITTER_MS`], so sweeps come 4 to 7 minutes apart. There is none in the first slot.
pub const SWEEP_SLOT_MS: u64 = 330_000;
pub const SWEEP_JITTER_MS: u64 = 90_000;
/// A sweep moves [`SWEEP_STEP_PX`] pixels right every [`SWEEP_STEP_MS`].
pub const SWEEP_STEP_MS: u64 = 100;
pub const SWEEP_STEP_PX: i32 = 2;

const WIDTH: i32 = crate::frame::NATIVE_WIDTH.cast_signed();

/// How far a pattern `period` pixels wide has drifted right at `now`, moving a pixel every
/// `ms_per_px`: in `0..period`.
pub fn offset(now: Millis, ms_per_px: u64, period: i32) -> i32 {
    let moved = now.0 / ms_per_px.max(1);
    let period = u64::try_from(period.max(1)).unwrap_or(1);
    i32::try_from(moved % period).unwrap_or(0)
}

/// The columns of a tiled pattern `step` pixels wide, drifted `offset` pixels right, that fall
/// inside `0..width`: each column's index and left edge. Column `lead` stands at the left edge
/// before any drift. A column keeps its index as the pattern drifts, so a tile moves a pixel
/// at a time and only enters or leaves at an edge of the frame.
pub fn columns(offset: i32, step: i32, lead: i32, width: i32) -> Vec<(u16, i32)> {
    let step = step.max(1);
    let last = width / step + lead + 1;
    (0..=last)
        .map(|column| (column, offset + (column - lead) * step))
        .filter(|&(_, x)| x + step > 0 && x < width)
        .filter_map(|(column, x)| Some((u16::try_from(column).ok()?, x)))
        .collect()
}

/// Whether a tiled sprite is the same in every column: drifting it changes no pixel, so it is
/// held still.
pub fn plain(art: &Art, tile: SpriteId) -> bool {
    art.rows(tile)
        .all(|row| row.iter().all(|&index| Some(&index) == row.first()))
}

/// When a drift moving a pixel every `ms_per_px` next moves after `now`.
pub fn next_step(now: Millis, ms_per_px: u64) -> Millis {
    let ms = ms_per_px.max(1);
    Millis((now.0 / ms + 1).saturating_mul(ms))
}

/// The salted moment the sweep of `slot` starts. Slot 0 has none.
fn sweep_start(salt: InstallSalt, slot: u64) -> Option<Millis> {
    if slot == 0 {
        return None;
    }
    let jitter = salt.below(Purpose::Sweep, slot, SWEEP_JITTER_MS);
    Some(Millis(
        slot.saturating_mul(SWEEP_SLOT_MS).saturating_add(jitter),
    ))
}

/// The sweep's left edge at `now`, while a sweep of a reflection `width` pixels wide crosses
/// the frame; `None` between sweeps. It enters from the left a step at a time and leaves
/// once wholly past the right edge.
pub fn sweep_x(salt: InstallSalt, now: Millis, width: i32) -> Option<i32> {
    let start = sweep_start(salt, now.0 / SWEEP_SLOT_MS)?;
    let elapsed = now.0.checked_sub(start.0)?;
    let steps = i32::try_from(elapsed / SWEEP_STEP_MS).unwrap_or(i32::MAX);
    let x = steps
        .saturating_add(1)
        .saturating_mul(SWEEP_STEP_PX)
        .saturating_sub(width);
    (x < WIDTH).then_some(x)
}

/// When the sweep next moves, appears or leaves after `now`.
pub fn sweep_next(salt: InstallSalt, now: Millis, width: i32) -> Millis {
    let slot = now.0 / SWEEP_SLOT_MS;
    if let Some(start) = sweep_start(salt, slot)
        && now >= start
        && sweep_x(salt, now, width).is_some()
    {
        let steps = (now.0 - start.0) / SWEEP_STEP_MS;
        return Millis(start.0 + (steps + 1) * SWEEP_STEP_MS);
    }
    // The next sweep's first step into the frame: this slot's if it is yet to come.
    sweep_start(salt, slot)
        .filter(|&start| start > now)
        .or_else(|| sweep_start(salt, slot + 1))
        .unwrap_or(Millis(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SALTS: [InstallSalt; 3] = [
        InstallSalt::new(*b"terrarium-preset"),
        InstallSalt::new([0; 16]),
        InstallSalt::new([0xa5; 16]),
    ];

    #[test]
    fn a_drift_moves_a_pixel_at_a_time_and_wraps_its_period() {
        assert_eq!(offset(Millis(0), BAND_MS_PER_PX, 16), 0);
        assert_eq!(offset(Millis(11_999), BAND_MS_PER_PX, 16), 0);
        assert_eq!(offset(Millis(12_000), BAND_MS_PER_PX, 16), 1);
        assert_eq!(offset(Millis(16 * 12_000), BAND_MS_PER_PX, 16), 0);
        assert_eq!(next_step(Millis(5), BAND_MS_PER_PX), Millis(12_000));
        assert_eq!(next_step(Millis(12_000), BAND_MS_PER_PX), Millis(24_000));
        // Still while a fresh composer fades the scene in.
        assert_eq!(offset(Millis(6000), SOIL_MS_PER_PX, SOIL_PERIOD), 0);
    }

    #[test]
    fn columns_cover_the_width_and_keep_their_index_as_they_drift() {
        let still = columns(0, 8, 3, 640);
        assert_eq!(still.first(), Some(&(3, 0)));
        assert_eq!(still.last(), Some(&(82, 632)));
        let drifted = columns(5, 8, 3, 640);
        assert_eq!(drifted.first(), Some(&(2, -3)));
        assert_eq!(drifted.last(), Some(&(82, 637)));
        for (column, x) in drifted {
            let before = still.iter().find(|&&(c, _)| c == column);
            assert!(before.is_none_or(|&(_, at)| x == at + 5));
        }
    }

    #[test]
    fn sweeps_come_four_to_seven_minutes_apart_and_cross_out_of_sight() {
        for salt in SALTS {
            let mut starts = Vec::new();
            let mut last: Option<i32> = None;
            for ms in (0..4 * 3_600_000).step_by(100) {
                let x = sweep_x(salt, Millis(ms), 24);
                match (last, x) {
                    (None, Some(x)) => {
                        assert_eq!(x, -24 + SWEEP_STEP_PX, "enters a step into sight");
                        starts.push(ms);
                    }
                    (Some(a), Some(b)) => assert!(b == a || b == a + SWEEP_STEP_PX, "{a} → {b}"),
                    (Some(a), None) => assert!(a + SWEEP_STEP_PX >= 640, "leaves at {a}"),
                    (None, None) => {}
                }
                last = x;
            }
            assert!(starts[0] >= SWEEP_SLOT_MS, "none in the first slot");
            for pair in starts.windows(2) {
                let gap = pair[1] - pair[0];
                assert!((240_000..=420_000).contains(&gap), "{gap}");
            }
        }
    }

    #[test]
    fn the_sweep_names_each_of_its_steps() {
        let salt = SALTS[0];
        let mut now = Millis(0);
        let mut moves = 0;
        loop {
            let next = sweep_next(salt, now, 24);
            assert!(next > now);
            if next >= Millis(2 * SWEEP_SLOT_MS) {
                break;
            }
            assert_eq!(
                sweep_x(salt, Millis(next.0 - 1), 24),
                sweep_x(salt, now, 24)
            );
            moves += usize::from(sweep_x(salt, next, 24) != sweep_x(salt, now, 24));
            now = next;
        }
        // One sweep: in at -22, 330 steps to 638, then out.
        assert_eq!(moves, 332);
    }
}
