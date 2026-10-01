//! The animation player: where an authored routine stands a given time after it began. It is a
//! pure function of the elapsed time, so the frame rate never changes how fast anything plays,
//! and a frame that was never drawn still moves its creature.
//!
//! A frame's `rootDelta` is world movement, applied when the frame's hold ends: once per
//! elapsed frame, whether or not it was rendered. A looping routine repeats; a finite one plays
//! its count and then holds its last frame, which is where a death, an egg or a hide rests.

use crate::catalog::{Animation, Catalog, SpriteId};

/// Where a routine stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pose {
    pub frame: SpriteId,
    /// The frame's place in the routine.
    pub index: usize,
    /// How far the routine has carried its creature, facing right: the `rootDelta.x` of every
    /// frame whose hold has ended.
    pub dx: i32,
    /// A finite routine has played out and holds its last frame.
    pub done: bool,
}

/// One pass through a routine's frames, in milliseconds.
pub fn cycle_ms(anim: &Animation) -> u64 {
    anim.durations_ms.iter().map(|&ms| u64::from(ms)).sum()
}

/// How long a finite routine plays before it holds; `None` for a loop.
pub fn length_ms(anim: &Animation) -> Option<u64> {
    (!anim.looping).then(|| cycle_ms(anim) * u64::from(anim.play_count.max(1)))
}

/// A frame's own world movement.
fn root_dx(catalog: &Catalog, frame: SpriteId) -> i32 {
    catalog
        .sprite(frame)
        .motion
        .as_ref()
        .map_or(0, |motion| i32::from(motion.root_delta.x))
}

/// The routine `elapsed_ms` after it began.
///
/// # Panics
/// If the routine has no frames, which `cargo xtask assets` rules out (check 5).
pub fn pose(catalog: &Catalog, anim: &Animation, elapsed_ms: u64) -> Pose {
    let cycle = cycle_ms(anim).max(1);
    let last = anim.frames.len() - 1;
    let cycle_dx = i32::from(anim.cycle_displacement);
    if let Some(length) = length_ms(anim)
        && elapsed_ms >= length
    {
        let plays = i32::from(anim.play_count.max(1));
        return Pose {
            frame: anim.frames[last],
            index: last,
            dx: cycle_dx * plays,
            done: true,
        };
    }
    let cycles = i32::try_from(elapsed_ms / cycle).unwrap_or(i32::MAX);
    let mut within = elapsed_ms % cycle;
    let mut dx = cycle_dx.saturating_mul(cycles);
    for (index, (&frame, &ms)) in anim.frames.iter().zip(&anim.durations_ms).enumerate() {
        let ms = u64::from(ms);
        if within < ms || index == last {
            return Pose {
                frame,
                index,
                dx,
                done: false,
            };
        }
        within -= ms;
        dx += root_dx(catalog, frame);
    }
    unreachable!("the last frame always returns")
}

/// The first moment after `elapsed_ms` at which the routine shows another frame or moves its
/// creature, as an elapsed time; `None` once a finite routine holds.
pub fn next_frame_at(anim: &Animation, elapsed_ms: u64) -> Option<u64> {
    if length_ms(anim).is_some_and(|length| elapsed_ms >= length) {
        return None;
    }
    let cycle = cycle_ms(anim).max(1);
    let start = elapsed_ms - elapsed_ms % cycle;
    let mut end = start;
    for &ms in &anim.durations_ms {
        end += u64::from(ms);
        if end > elapsed_ms {
            return Some(end);
        }
    }
    Some(start + cycle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Art, Next};

    fn catalog() -> &'static Catalog {
        &Art::embedded().catalog
    }

    /// Every creature routine: the ones with a species, but not an effect attached to one.
    fn routines() -> impl Iterator<Item = &'static Animation> {
        catalog().animations.iter().filter(|anim| {
            anim.keys.species.is_some() && anim.keys.life_stage.is_some() && anim.sync.is_none()
        })
    }

    /// The movement written out frame by frame: each frame's delta once its hold has ended,
    /// stepping through the frames one at a time from the start.
    fn walked(anim: &Animation, elapsed: u64) -> (usize, i32) {
        let plays = if anim.looping {
            u64::MAX
        } else {
            u64::from(anim.play_count.max(1))
        };
        let (mut at, mut dx, mut index) = (0_u64, 0_i32, 0_usize);
        let mut played = 0;
        loop {
            for (i, (&frame, &ms)) in anim.frames.iter().zip(&anim.durations_ms).enumerate() {
                index = i;
                if at + u64::from(ms) > elapsed {
                    return (index, dx);
                }
                at += u64::from(ms);
                dx += root_dx(catalog(), frame);
            }
            played += 1;
            if played == plays {
                return (index, dx);
            }
        }
    }

    #[test]
    fn root_delta_applies_once_per_elapsed_frame_rendered_or_not() {
        for anim in routines() {
            let cycle = cycle_ms(anim);
            // Jumps of awkward lengths over three cycles, as frames come at no set rate.
            for elapsed in (0..3 * cycle + 400)
                .step_by(37)
                .chain([cycle, 2 * cycle - 1])
            {
                let pose = pose(catalog(), anim, elapsed);
                let (index, dx) = walked(anim, elapsed);
                assert_eq!(
                    (pose.index, pose.dx),
                    (index, dx),
                    "{} at {elapsed}",
                    anim.name
                );
                assert_eq!(pose.frame, anim.frames[index]);
            }
        }
        let walk = catalog().animation(catalog().animation_id("beetle.adult.move").unwrap());
        // 200, 167, 200, 167 ms, a pixel after each: two cycles and one frame, unrendered.
        assert_eq!(pose(catalog(), walk, 734 * 2 + 200).dx, 9);
        assert_eq!(pose(catalog(), walk, 734 * 2 + 199).dx, 8);
    }

    #[test]
    fn a_finite_routine_holds_its_last_frame_and_a_loop_repeats() {
        let id = |name| catalog().animation_id(name).unwrap();
        let death = catalog().animation(id("snail.adult.death"));
        let held = pose(catalog(), death, 10 * cycle_ms(death));
        assert!(held.done);
        assert_eq!(held.frame, *death.frames.last().unwrap());
        assert_eq!(death.next, Next::Hold);
        assert_eq!(next_frame_at(death, 10 * cycle_ms(death)), None);
        let idle = catalog().animation(id("snail.adult.idle"));
        let again = pose(catalog(), idle, cycle_ms(idle) * 5 + 1);
        assert!(!again.done);
        assert_eq!(again.index, 0);
    }

    #[test]
    fn the_next_frame_is_the_first_moment_the_pose_changes() {
        for anim in routines() {
            let horizon = 2 * cycle_ms(anim) + 50;
            let mut elapsed = 0;
            while let Some(next) = next_frame_at(anim, elapsed) {
                assert!(next > elapsed);
                let before = pose(catalog(), anim, elapsed);
                assert_eq!(pose(catalog(), anim, next - 1), before, "{}", anim.name);
                let after = pose(catalog(), anim, next);
                assert!(after != before || anim.frames.len() == 1, "{}", anim.name);
                if next > horizon {
                    break;
                }
                elapsed = next;
            }
        }
    }

    #[test]
    fn idle_feed_and_sleep_never_slide() {
        let resting = routines().filter(|anim| {
            matches!(
                anim.keys.routine.as_deref(),
                Some("idle" | "feed" | "sleep")
            )
        });
        let mut seen = 0;
        for anim in resting {
            for &frame in &anim.frames {
                assert_eq!(root_dx(catalog(), frame), 0, "{}", anim.name);
            }
            assert_eq!(pose(catalog(), anim, 7 * cycle_ms(anim) + 3).dx, 0);
            seen += 1;
        }
        assert_eq!(
            seen, 40,
            "idle, feed and sleep for each species, adult and juvenile"
        );
    }

    #[test]
    fn ground_frames_stand_on_the_anchor_row_and_only_flight_leaves_it() {
        for anim in routines() {
            let routine = anim.keys.routine.as_deref().unwrap_or("");
            for &frame in &anim.frames {
                let sprite = catalog().sprite(frame);
                let contacts = &sprite
                    .motion
                    .as_ref()
                    .expect("creature motion")
                    .ground_contacts;
                if contacts.is_empty() {
                    assert!(
                        matches!(routine, "takeoff" | "flight" | "land" | "hop"),
                        "{} is airborne",
                        sprite.id
                    );
                } else {
                    assert!(
                        contacts.iter().all(|c| c.y == sprite.anchor.y),
                        "{} stands off its anchor row",
                        sprite.id
                    );
                }
            }
        }
    }
}
