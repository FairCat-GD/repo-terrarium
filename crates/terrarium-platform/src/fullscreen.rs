//! When the strip steps back for a full-screen app on its monitor, as `ManagedShell`'s
//! full-screen helper decides it: the foreground window, measured against the monitors, with
//! the desktop, cloaked and click-through windows left out. `ABN_FULLSCREENAPP` and the shell
//! hook only say when to look. Pure, so it is tested on every OS; the `windows` module reads
//! the windows and moves the strip.

use crate::appbar::{
    HSHELL_FULLSCREENENTER, HSHELL_FULLSCREENEXIT, HSHELL_HIGHBIT, HSHELL_MONITORCHANGED,
    HSHELL_WINDOWACTIVATED, HSHELL_WINDOWDESTROYED, PixelRect, WS_BORDER, WS_CAPTION,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WS_THICKFRAME,
};

/// Why the strip looks again: what the shell hook or the `AppBar` said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    /// A window was activated.
    Activated,
    /// A window the shell takes as full screen was activated.
    RudeActivated,
    Destroyed,
    /// A window moved to another monitor.
    MonitorChanged,
    /// A window entered full screen where it was (the shell's undocumented code).
    Entered,
    Left,
    /// `ABN_FULLSCREENAPP`: some app opened (`true`) or closed full screen, on some monitor.
    AppBar(bool),
}

/// The hint a shell hook message's `wParam` gives, if it is one the strip looks again for.
pub fn shell_hint(wparam: usize) -> Option<Hint> {
    let rude = wparam & HSHELL_HIGHBIT != 0;
    match (wparam & !HSHELL_HIGHBIT, rude) {
        (HSHELL_WINDOWACTIVATED, false) => Some(Hint::Activated),
        (HSHELL_WINDOWACTIVATED, true) => Some(Hint::RudeActivated),
        (HSHELL_WINDOWDESTROYED, _) => Some(Hint::Destroyed),
        (HSHELL_MONITORCHANGED, _) => Some(Hint::MonitorChanged),
        (HSHELL_FULLSCREENENTER, _) => Some(Hint::Entered),
        (HSHELL_FULLSCREENEXIT, _) => Some(Hint::Left),
        _ => None,
    }
}

/// What the strip reads of a window to judge it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts, as Windows reports them"
)]
pub struct WindowFacts {
    pub handle: isize,
    /// It belongs to this process: the strip's own windows never count.
    pub own_process: bool,
    pub visible: bool,
    pub iconic: bool,
    /// DWM hides it: another virtual desktop's window, or a suspended app's.
    pub cloaked: bool,
    /// It set the `NonRudeHWND` property, asking never to be taken as full screen (the
    /// desktop does).
    pub non_rude: bool,
    pub class: String,
    pub style: u32,
    pub ex_style: u32,
    /// Its window rectangle.
    pub window: PixelRect,
    /// Its client area on screen, when it could be read.
    pub client: Option<PixelRect>,
    /// The `rcMonitor` of the monitor it is on, when it could be read.
    pub monitor: Option<PixelRect>,
}

impl WindowFacts {
    pub fn topmost(&self) -> bool {
        self.ex_style & WS_EX_TOPMOST != 0
    }
}

/// The rectangle a window is judged by: its client area when it has a frame or a border a
/// maximised window pushes off screen, otherwise its window.
pub fn effective_rect(facts: &WindowFacts) -> PixelRect {
    let has = |bits: u32| facts.style & bits == bits;
    let framed = has(WS_CAPTION | WS_THICKFRAME)
        || has(WS_POPUP | WS_THICKFRAME)
        || has(WS_POPUP | WS_BORDER);
    match facts.client {
        Some(client) if framed => client,
        _ => facts.window,
    }
}

/// The desktop's windows, which cover a monitor and are never an app.
const DESKTOP_CLASSES: &[&str] = &["Progman", "WorkerW"];

/// Whether a window may be taken as full screen at all.
pub fn may_be_full_screen(facts: &WindowFacts) -> bool {
    let click_through = facts.ex_style & WS_EX_LAYERED != 0
        && facts.ex_style & (WS_EX_TRANSPARENT | WS_EX_NOACTIVATE) != 0;
    !facts.own_process
        && facts.visible
        && !facts.iconic
        && !facts.cloaked
        && !facts.non_rude
        && !click_through
        && !DESKTOP_CLASSES.contains(&facts.class.as_str())
}

/// The screen a full-screen window covers: exactly one monitor's rectangle, or exactly all of
/// them (with one monitor, the same). `None` when it is not full screen.
pub fn full_screen_on(facts: &WindowFacts, monitors: &[PixelRect]) -> Option<PixelRect> {
    if !may_be_full_screen(facts) {
        return None;
    }
    let rect = effective_rect(facts);
    if monitors.contains(&rect) {
        return Some(rect);
    }
    union(monitors).filter(|&all| rect == all)
}

fn union(rects: &[PixelRect]) -> Option<PixelRect> {
    rects.iter().copied().reduce(|a, b| PixelRect {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    })
}

/// Whether `inner` lies within `outer`.
fn within(inner: PixelRect, outer: PixelRect) -> bool {
    inner.left >= outer.left
        && inner.top >= outer.top
        && inner.right <= outer.right
        && inner.bottom <= outer.bottom
}

/// A full-screen app the strip knows of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tracked {
    pub handle: isize,
    /// The screen it covers.
    pub screen: PixelRect,
    /// Its screen is given over to it. An ordinary window in front of it on its screen makes it
    /// inactive, and the strip comes back there until the app is in front again.
    pub active: bool,
}

/// The most full-screen apps followed at once.
pub const TRACKED_MAX: usize = 8;

/// The full-screen apps after a look: `tracked` as they were, `foreground` the window in front
/// now, and `now` each tracked window as it is now (`None` once it is gone).
///
/// - One that left full screen, or closed, is dropped.
/// - One in front stays active, on whichever screen it now covers.
/// - One behind stays active while the window in front is on another screen, or is topmost on
///   its own (a volume flyout, a notification); an ordinary window in front of it on its
///   screen makes it inactive.
/// - An inactive one in front again is active again.
/// - The window in front, full screen and not followed yet, is added, active.
pub fn update(
    tracked: &[Tracked],
    foreground: Option<&WindowFacts>,
    now: &[(isize, Option<WindowFacts>)],
    monitors: &[PixelRect],
) -> Vec<Tracked> {
    let in_front = |handle| foreground.is_some_and(|f| f.handle == handle);
    let front_screen = foreground.and_then(|f| f.monitor);
    let front_topmost = foreground.is_some_and(WindowFacts::topmost);
    let mut next: Vec<Tracked> = tracked
        .iter()
        .filter_map(|app| {
            let facts = now.iter().find(|(h, _)| *h == app.handle)?.1.as_ref()?;
            let screen = full_screen_on(facts, monitors)?;
            let covered = front_screen.is_some_and(|front| within(front, screen));
            let active = if in_front(app.handle) {
                true
            } else {
                app.active && (!covered || front_topmost)
            };
            Some(Tracked {
                handle: app.handle,
                screen,
                active,
            })
        })
        .collect();
    if let Some(front) = foreground
        && !next.iter().any(|app| app.handle == front.handle)
        && next.len() < TRACKED_MAX
        && let Some(screen) = full_screen_on(front, monitors)
    {
        next.push(Tracked {
            handle: front.handle,
            screen,
            active: true,
        });
    }
    next
}

/// The full-screen app the strip on `strip_monitor` steps back behind, if any: an active one
/// covering that monitor.
pub fn yields(tracked: &[Tracked], strip_monitor: PixelRect) -> Option<isize> {
    tracked
        .iter()
        .find(|app| app.active && within(strip_monitor, app.screen))
        .map(|app| app.handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn rect(left: i32, top: i32, right: i32, bottom: i32) -> PixelRect {
        PixelRect {
            left,
            top,
            right,
            bottom,
        }
    }

    const LEFT: PixelRect = rect(0, 0, 2560, 1440);
    const RIGHT: PixelRect = rect(2560, 0, 4480, 1080);
    const BOTH: [PixelRect; 2] = [LEFT, RIGHT];

    /// A borderless window over `window`, on the monitor `monitor`.
    fn window(handle: isize, window: PixelRect, monitor: PixelRect) -> WindowFacts {
        WindowFacts {
            handle,
            own_process: false,
            visible: true,
            iconic: false,
            cloaked: false,
            non_rude: false,
            class: "GameWindow".to_owned(),
            style: WS_POPUP,
            ex_style: 0,
            window,
            client: Some(window),
            monitor: Some(monitor),
        }
    }

    fn game() -> WindowFacts {
        window(10, LEFT, LEFT)
    }

    #[test]
    fn a_window_covering_its_monitor_exactly_is_full_screen() {
        assert_eq!(full_screen_on(&game(), &BOTH), Some(LEFT));
        assert_eq!(
            full_screen_on(&window(11, RIGHT, RIGHT), &BOTH),
            Some(RIGHT)
        );
    }

    #[test]
    fn a_maximised_framed_window_is_judged_by_its_client_area() {
        // Maximised, its frame hangs off every side of the work area.
        let mut maximised = window(12, rect(-8, -8, 2568, 1400), LEFT);
        maximised.style = WS_CAPTION | WS_THICKFRAME;
        maximised.client = Some(rect(0, 23, 2560, 1392));
        assert_eq!(full_screen_on(&maximised, &BOTH), None);
        // A framed window whose client area is the monitor is full screen.
        maximised.client = Some(LEFT);
        assert_eq!(full_screen_on(&maximised, &BOTH), Some(LEFT));
        // Without a frame the window rectangle counts, whatever the client.
        let mut plain = window(13, LEFT, LEFT);
        plain.client = Some(rect(0, 0, 10, 10));
        assert_eq!(full_screen_on(&plain, &BOTH), Some(LEFT));
    }

    #[test]
    fn a_pixel_short_is_not_full_screen() {
        assert_eq!(
            full_screen_on(&window(14, rect(0, 0, 2560, 1439), LEFT), &BOTH),
            None
        );
    }

    #[test]
    fn spanning_every_monitor_counts_and_spanning_more_does_not() {
        let all = rect(0, 0, 4480, 1440);
        assert_eq!(full_screen_on(&window(15, all, LEFT), &BOTH), Some(all));
        assert_eq!(full_screen_on(&window(16, LEFT, LEFT), &[LEFT]), Some(LEFT));
        assert_eq!(full_screen_on(&window(17, all, LEFT), &[LEFT]), None);
        assert_eq!(full_screen_on(&window(18, LEFT, LEFT), &[]), None);
    }

    #[test]
    fn own_hidden_minimised_cloaked_and_opted_out_windows_never_count() {
        let cases: [fn(&mut WindowFacts); 5] = [
            |f| f.own_process = true,
            |f| f.visible = false,
            |f| f.iconic = true,
            |f| f.cloaked = true,
            |f| f.non_rude = true,
        ];
        for change in cases {
            let mut facts = game();
            change(&mut facts);
            assert_eq!(full_screen_on(&facts, &BOTH), None, "{facts:?}");
        }
    }

    #[test]
    fn the_desktop_and_click_through_overlays_never_count() {
        for class in DESKTOP_CLASSES {
            let mut desktop = game();
            desktop.class = (*class).to_owned();
            assert_eq!(full_screen_on(&desktop, &BOTH), None);
        }
        for extra in [WS_EX_TRANSPARENT, WS_EX_NOACTIVATE] {
            let mut overlay = game();
            overlay.ex_style = WS_EX_LAYERED | extra;
            assert_eq!(full_screen_on(&overlay, &BOTH), None);
        }
        // Layered alone is an ordinary window (many players are).
        let mut layered = game();
        layered.ex_style = WS_EX_LAYERED;
        assert_eq!(full_screen_on(&layered, &BOTH), Some(LEFT));
    }

    fn look(tracked: &[Tracked], front: &WindowFacts, now: &[WindowFacts]) -> Vec<Tracked> {
        let now: Vec<_> = now.iter().map(|f| (f.handle, Some(f.clone()))).collect();
        update(tracked, Some(front), &now, &BOTH)
    }

    #[test]
    fn the_full_screen_app_in_front_on_the_strips_monitor_is_stepped_back_for() {
        let tracked = look(&[], &game(), &[]);
        assert_eq!(
            tracked,
            [Tracked {
                handle: 10,
                screen: LEFT,
                active: true
            }]
        );
        assert_eq!(yields(&tracked, LEFT), Some(10));
    }

    #[test]
    fn one_on_another_monitor_is_not() {
        let tracked = look(&[], &window(11, RIGHT, RIGHT), &[]);
        assert_eq!(yields(&tracked, LEFT), None);
        assert_eq!(yields(&tracked, RIGHT), Some(11));
    }

    #[test]
    fn work_in_front_on_another_monitor_keeps_the_app_active() {
        let tracked = look(&[], &game(), &[]);
        let editor = window(20, rect(2600, 40, 4000, 1000), RIGHT);
        let tracked = look(&tracked, &editor, &[game()]);
        assert_eq!(yields(&tracked, LEFT), Some(10));
    }

    #[test]
    fn a_topmost_window_in_front_on_the_same_monitor_keeps_it_active() {
        let tracked = look(&[], &game(), &[]);
        let mut flyout = window(21, rect(2200, 1200, 2540, 1420), LEFT);
        flyout.ex_style = WS_EX_TOPMOST;
        let tracked = look(&tracked, &flyout, &[game()]);
        assert_eq!(yields(&tracked, LEFT), Some(10));
    }

    #[test]
    fn an_ordinary_window_or_the_desktop_in_front_on_the_same_monitor_makes_it_inactive() {
        let tracked = look(&[], &game(), &[]);
        let editor = window(22, rect(100, 100, 1200, 900), LEFT);
        let behind = look(&tracked, &editor, &[game()]);
        assert_eq!(yields(&behind, LEFT), None);
        assert_eq!(behind.len(), 1, "still followed, inactive");
        let mut desktop = window(23, LEFT, LEFT);
        desktop.class = "WorkerW".to_owned();
        assert_eq!(yields(&look(&tracked, &desktop, &[game()]), LEFT), None);
    }

    #[test]
    fn an_app_that_left_full_screen_or_closed_is_dropped() {
        let tracked = look(&[], &game(), &[]);
        let windowed = window(10, rect(200, 200, 1800, 1100), LEFT);
        assert_eq!(
            look(&tracked, &windowed, std::slice::from_ref(&windowed)),
            []
        );
        let editor = window(22, rect(100, 100, 1200, 900), LEFT);
        assert_eq!(update(&tracked, Some(&editor), &[(10, None)], &BOTH), []);
        let mut minimised = game();
        minimised.iconic = true;
        assert_eq!(look(&tracked, &editor, &[minimised]), []);
    }

    #[test]
    fn an_inactive_app_back_in_front_is_stepped_back_for_again() {
        let tracked = look(&[], &game(), &[]);
        let editor = window(22, rect(100, 100, 1200, 900), LEFT);
        let behind = look(&tracked, &editor, &[game()]);
        let again = look(&behind, &game(), &[game()]);
        assert_eq!(yields(&again, LEFT), Some(10));
        assert_eq!(again.len(), 1);
    }

    #[test]
    fn with_nothing_in_front_the_apps_stay_as_they_were() {
        let tracked = look(&[], &game(), &[]);
        let now = [(10, Some(game()))];
        assert_eq!(update(&tracked, None, &now, &BOTH), tracked);
    }

    #[test]
    fn the_apps_followed_are_bounded() {
        let mut tracked = Vec::new();
        for handle in 0..20 {
            let mut front = game();
            front.handle = handle;
            let now: Vec<_> = tracked
                .iter()
                .map(|app: &Tracked| {
                    let mut facts = game();
                    facts.handle = app.handle;
                    (app.handle, Some(facts))
                })
                .collect();
            tracked = update(&tracked, Some(&front), &now, &BOTH);
        }
        assert_eq!(tracked.len(), TRACKED_MAX);
    }

    #[test]
    fn shell_codes_give_their_hints_with_and_without_the_high_bit() {
        assert_eq!(shell_hint(4), Some(Hint::Activated));
        assert_eq!(shell_hint(0x8004), Some(Hint::RudeActivated));
        assert_eq!(shell_hint(2), Some(Hint::Destroyed));
        assert_eq!(shell_hint(16), Some(Hint::MonitorChanged));
        assert_eq!(shell_hint(53), Some(Hint::Entered));
        assert_eq!(shell_hint(54), Some(Hint::Left));
        assert_eq!(
            shell_hint(6),
            None,
            "a title redrawn says nothing of full screen"
        );
        assert_eq!(shell_hint(1), None);
    }
}
