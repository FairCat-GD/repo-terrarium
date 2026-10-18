//! Why the strip's next frame was asked for (I-10).
//!
//! The living strip's schedule asks for a frame only when it expects its pixels to change, and
//! such a frame is shown only if they did. Every other request (the window opened, placed,
//! resized or uncovered) shows the pixels whatever they are. winit folds every request made
//! before a redraw into one `RedrawRequested`, and on Windows folds the OS's own invalidation
//! (`WM_PAINT`) in with them, so the causes are kept together until that redraw takes them all.
//! A redraw nobody here asked for is the OS's own, and shows the pixels too.

/// The causes of the redraw on its way.
#[derive(Debug, Default)]
pub struct Redraws {
    scheduled: bool,
    forced: bool,
}

/// What a redraw does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Present {
    /// The pixels changed: show them, whole when the OS or the strip's own move asked too,
    /// otherwise only what changed.
    Changed { whole: bool },
    /// The pixels did not change, but the OS or the strip's own move asked: show them whole
    /// again.
    Again,
    /// Only the schedule asked, and nothing changed: show nothing.
    No,
}

impl Redraws {
    /// The living strip's schedule asked.
    pub fn schedule(&mut self) {
        self.scheduled = true;
    }

    /// The strip asked for itself: opened, placed, resized or uncovered.
    pub fn force(&mut self) {
        self.forced = true;
    }

    /// A redraw arrived with the pixels `changed` or not: what it does. It takes every cause
    /// asked for so far, so a scheduled redraw that never came (the strip was covered first)
    /// is answered by the next one, which the uncover forces.
    pub fn take(&mut self, changed: bool) -> Present {
        let scheduled = std::mem::take(&mut self.scheduled);
        let forced = std::mem::take(&mut self.forced);
        let whole = forced || !scheduled;
        match (changed, whole) {
            (true, whole) => Present::Changed { whole },
            (false, true) => Present::Again,
            (false, false) => Present::No,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schedule_alone_shows_only_a_change_and_only_what_changed() {
        let mut redraws = Redraws::default();
        redraws.schedule();
        assert_eq!(redraws.take(false), Present::No);
        redraws.schedule();
        assert_eq!(redraws.take(true), Present::Changed { whole: false });
    }

    #[test]
    fn a_move_folded_into_a_scheduled_redraw_still_shows_the_pixels_whole() {
        let mut redraws = Redraws::default();
        redraws.schedule();
        redraws.force();
        assert_eq!(redraws.take(false), Present::Again);
        redraws.schedule();
        redraws.force();
        assert_eq!(redraws.take(true), Present::Changed { whole: true });
        // The OS's own redraw, which nobody here asked for, shows them whole too.
        assert_eq!(redraws.take(false), Present::Again);
        assert_eq!(redraws.take(true), Present::Changed { whole: true });
    }

    #[test]
    fn a_scheduled_redraw_that_never_came_swallows_no_later_request() {
        let mut redraws = Redraws::default();
        redraws.schedule();
        // Covered before it came; uncovering forces the next one, which shows the pixels.
        redraws.force();
        assert_eq!(redraws.take(false), Present::Again);
        // Each redraw takes every cause: a schedule after it stands alone again.
        redraws.schedule();
        assert_eq!(redraws.take(false), Present::No);
    }
}
