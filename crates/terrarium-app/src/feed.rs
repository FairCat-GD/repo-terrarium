//! What the living strip shows, and when it looks again: a connected repository's terrarium as
//! the engine sends it, the thriving sample when none is connected, or `--demo`'s cycle; always
//! on the local clock and dimmed after a while without input. Dev builds add the debug window's
//! choices.
//!
//! The feeder is pure: the strip reads the OS (`Os::read`) and passes it in with the time, so
//! it is driven the same way in tests.

use std::time::Duration;

use terrarium_compose::clock::{DAY_MS, LocalClock};
use terrarium_compose::composer::OverlayInput;
use terrarium_compose::preset;
use terrarium_compose::sky::LocalDate;
use terrarium_compose::synthetic;
#[cfg(feature = "dev")]
use terrarium_compose::synthetic::Knobs;
use terrarium_compose::tween::Millis;
use terrarium_platform::clock::LocalNow;
use terrarium_truth::scene::SceneInput;

use crate::{copy, demo, idle};

/// How often the strip takes the local clock again, though nothing else changed: the
/// composer carries the time on by itself, and this bounds how far it can drift.
const CLOCK_EVERY_MS: u32 = 3_600_000;
/// How often `--demo` asks its cycle for the climate.
const DEMO_EVERY_MS: u32 = 60_000;

/// What the strip shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Feed {
    /// A connected repository: the scene and the label the engine's updates make.
    Live,
    /// The thriving sample, when no repository is connected.
    Thriving,
    /// `--demo`'s cycle through the named states.
    Demo,
    /// A preset, by name (dev builds, from the debug window).
    #[cfg(feature = "dev")]
    Preset(&'static str),
    /// The synthetic climate at these knobs (dev builds, from the debug window).
    #[cfg(feature = "dev")]
    Synthetic(Knobs),
}

/// What the strip reads from the OS each time it looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Os {
    pub clock: Option<LocalClock>,
    pub idle: Option<Duration>,
}

impl Os {
    pub fn read() -> Os {
        Os {
            clock: terrarium_platform::clock::local_now().map(local_clock),
            idle: terrarium_platform::idle::since_input(),
        }
    }
}

/// The OS's local time as the composer counts it.
pub fn local_clock(now: LocalNow) -> LocalClock {
    LocalClock::new(LocalDate::new(now.year, now.month, now.day), now.ms_of_day)
}

/// The debug window's say over the strip (dev builds).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Overrides {
    /// A fixed time of day, in milliseconds after midnight, instead of the OS's.
    pub time_of_day: Option<u32>,
    /// Dimmed or not whatever the idle time, instead of after it.
    pub dim: Option<bool>,
    /// The foreground event, which nothing draws yet.
    pub event: Option<terrarium_truth::scene::EventKind>,
}

pub struct Feeder {
    feed: Feed,
    /// When the strip started, from which `--demo`'s cycle counts.
    started: Millis,
    overrides: Overrides,
    scene: SceneInput,
    /// The connected repository's latest scene and label lines, kept while the debug window's
    /// choice is shown instead.
    live: Option<(SceneInput, Vec<String>)>,
    /// The clock last given, and when: carried on from there if the OS cannot say.
    clock: LocalClock,
    clock_at: Millis,
    idle_dim: bool,
    /// When the feed next asks for its scene, takes the clock, and checks the idle time.
    scene_due: Millis,
    clock_due: Millis,
    idle_due: Millis,
}

impl Feeder {
    pub fn new(feed: Feed, now: Millis, os: &Os) -> Feeder {
        let bare = preset::bare(terrarium_truth::PresentationState::Thriving, Vec::new());
        Feeder::starting(feed, bare, None, now, os)
    }

    /// The live feed, from the scene and label it starts with.
    pub fn live(scene: SceneInput, label: Vec<String>, now: Millis, os: &Os) -> Feeder {
        Feeder::starting(Feed::Live, scene.clone(), Some((scene, label)), now, os)
    }

    fn starting(
        feed: Feed,
        scene: SceneInput,
        live: Option<(SceneInput, Vec<String>)>,
        now: Millis,
        os: &Os,
    ) -> Feeder {
        let mut feeder = Feeder {
            feed,
            started: now,
            overrides: Overrides::default(),
            live,
            scene,
            clock: os.clock.unwrap_or(LocalClock::noon(preset::DATE)),
            clock_at: now,
            idle_dim: false,
            scene_due: now,
            clock_due: now,
            idle_due: now,
        };
        feeder.tick(now, os, true);
        feeder
    }

    /// The scene to show.
    pub fn scene(&self) -> &SceneInput {
        &self.scene
    }

    /// The overlay to show it with: the label, the local clock at the last look, and dim.
    pub fn overlay(&self) -> OverlayInput {
        OverlayInput {
            dim: self.overrides.dim.unwrap_or(self.idle_dim),
            ..OverlayInput::new(self.label(), self.clock)
        }
    }

    /// The label's lines: the connected repository's while it is shown, the sample's otherwise.
    fn label(&self) -> Vec<String> {
        match (&self.feed, &self.live) {
            (Feed::Live, Some((_, label))) => label.clone(),
            _ => vec![copy::sample_label().to_owned()],
        }
    }

    /// Takes the engine's latest `scene` and `label` at `now`; the strip is then updated with
    /// `scene()` and `overlay()`. While the debug window's choice is shown, it keeps the scene.
    pub fn show(&mut self, scene: &SceneInput, label: Vec<String>, now: Millis, os: &Os) {
        self.live = Some((scene.clone(), label));
        self.tick(now, os, true);
    }

    /// When the feed next wants to look.
    pub fn due(&self) -> Millis {
        self.scene_due.min(self.clock_due).min(self.idle_due)
    }

    /// Brings the feed to `now` with what the OS says. True when the strip must be updated
    /// with `scene()` and `overlay()`. `force` looks at everything at once, as after a wake,
    /// when the carried clock has fallen behind (on macOS the monotonic clock stops in sleep).
    pub fn tick(&mut self, now: Millis, os: &Os, force: bool) -> bool {
        let mut changed = force;
        if force || now >= self.idle_due {
            let (dim, look_again) = idle::check(os.idle);
            self.idle_due = now.plus(u32::try_from(look_again.as_millis()).unwrap_or(u32::MAX));
            changed |= dim != self.idle_dim;
            self.idle_dim = dim;
        }
        if force || now >= self.scene_due {
            self.scene_due = now.plus(self.scene_every());
            let scene = self.scene_at(now);
            changed |= scene != self.scene;
            self.scene = scene;
        }
        changed |= now >= self.clock_due;
        if changed {
            // Every update takes the clock afresh: the composer counts on from the one it is
            // given at the moment it is given it.
            self.clock = self.clock_now(now, os);
            self.clock_at = now;
            self.clock_due = now.plus(CLOCK_EVERY_MS);
        }
        changed
    }

    /// Shows `feed` with the debug window's `overrides` from `now`; the strip updates at once.
    /// Choosing the demo again starts its cycle again.
    #[cfg(feature = "dev")]
    pub fn choose(&mut self, feed: Feed, overrides: Overrides, now: Millis, os: &Os) {
        if feed == Feed::Demo && self.feed != Feed::Demo {
            self.started = now;
        }
        self.feed = feed;
        self.overrides = overrides;
        self.tick(now, os, true);
    }

    #[cfg(feature = "dev")]
    pub fn feed(&self) -> &Feed {
        &self.feed
    }

    #[cfg(feature = "dev")]
    pub fn overrides(&self) -> Overrides {
        self.overrides
    }

    /// `--demo`'s stage at `now`, while the demo is shown.
    #[cfg(feature = "dev")]
    pub fn stage(&self, now: Millis) -> Option<&'static str> {
        (self.feed == Feed::Demo).then(|| demo::stage_at(now.0.saturating_sub(self.started.0)))
    }

    fn scene_every(&self) -> u32 {
        if self.feed == Feed::Demo {
            DEMO_EVERY_MS
        } else {
            CLOCK_EVERY_MS
        }
    }

    fn scene_at(&self, now: Millis) -> SceneInput {
        let mut scene = match &self.feed {
            Feed::Live => self
                .live
                .as_ref()
                .map_or_else(crate::live::unknown_scene, |(scene, _)| scene.clone()),
            Feed::Thriving => preset::scene("thriving").expect("the thriving preset"),
            Feed::Demo => synthetic::scene(&demo::knobs_at(now.0.saturating_sub(self.started.0))),
            #[cfg(feature = "dev")]
            Feed::Preset(name) => preset::scene(name).expect("a known preset"),
            #[cfg(feature = "dev")]
            Feed::Synthetic(knobs) => synthetic::scene(knobs),
        };
        if let Some(kind) = self.overrides.event {
            synthetic::set_event(&mut scene, kind);
        }
        scene
    }

    /// The local clock at `now`: the override's time of day, else the OS's, else the last
    /// clock carried on.
    fn clock_now(&self, now: Millis, os: &Os) -> LocalClock {
        let carried = self
            .clock
            .advanced_by(now.0.saturating_sub(self.clock_at.0));
        let clock = os.clock.unwrap_or(carried);
        match self.overrides.time_of_day {
            Some(ms_of_day) => LocalClock::new(clock.date, ms_of_day % DAY_MS),
            None => clock,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATE: LocalDate = LocalDate::new(2026, 10, 1);

    fn os(ms_of_day: u32, idle_mins: u64) -> Os {
        Os {
            clock: Some(LocalClock::new(DATE, ms_of_day)),
            idle: Some(Duration::from_mins(idle_mins)),
        }
    }

    #[test]
    fn a_plain_launch_shows_the_thriving_terrarium_at_the_local_time() {
        let feeder = Feeder::new(Feed::Thriving, Millis(0), &os(22 * 3_600_000, 0));
        assert_eq!(feeder.scene(), &preset::scene("thriving").unwrap());
        let overlay = feeder.overlay();
        assert_eq!(overlay.clock, LocalClock::new(DATE, 22 * 3_600_000));
        assert!(!overlay.dim);
        assert_eq!(overlay.label, [copy::sample_label()]);
    }

    #[test]
    fn the_clock_is_taken_again_every_hour_and_nothing_else_wakes_a_bright_strip() {
        let mut feeder = Feeder::new(Feed::Thriving, Millis(0), &os(0, 0));
        // The idle check is due when the idle time could first reach the timeout.
        assert_eq!(feeder.due(), Millis(30 * 60_000));
        assert!(!feeder.tick(Millis(30 * 60_000), &os(30 * 60_000, 10), false));
        assert_eq!(feeder.due(), Millis(50 * 60_000));
        assert!(!feeder.tick(Millis(50 * 60_000), &os(50 * 60_000, 0), false));
        assert_eq!(feeder.due(), Millis(60 * 60_000));
        // An hour on, the clock is taken again, though the scene is the same.
        assert!(feeder.tick(Millis(60 * 60_000), &os(60 * 60_000 + 5, 0), false));
        assert_eq!(feeder.overlay().clock.ms_of_day, 60 * 60_000 + 5);
    }

    #[test]
    fn thirty_idle_minutes_dim_the_strip_and_input_brightens_it() {
        let mut feeder = Feeder::new(Feed::Thriving, Millis(0), &os(0, 29));
        assert!(!feeder.overlay().dim);
        assert_eq!(feeder.due(), Millis(60_000));
        assert!(feeder.tick(Millis(60_000), &os(60_000, 30), false));
        assert!(feeder.overlay().dim);
        assert_eq!(feeder.due(), Millis(61_000));
        assert!(!feeder.tick(Millis(61_000), &os(61_000, 31), false));
        assert!(feeder.tick(
            Millis(62_000),
            &Os {
                idle: Some(Duration::ZERO),
                ..os(62_000, 0)
            },
            false
        ));
        assert!(!feeder.overlay().dim);
    }

    #[test]
    fn the_demo_plays_its_cycle_from_launch() {
        let start = Millis(5_000);
        let mut feeder = Feeder::new(Feed::Demo, start, &os(0, 0));
        assert_eq!(feeder.scene(), &synthetic::scene(&demo::knobs_at(0)));
        assert_eq!(feeder.due(), start.plus(DEMO_EVERY_MS));
        let mut updates = 0;
        for minute in 1..=105 {
            let now = start.plus(minute * 60_000);
            updates += usize::from(feeder.tick(now, &os(minute * 60_000, 0), false));
            let elapsed = u64::from(minute) * 60_000;
            assert_eq!(
                feeder.scene(),
                &synthetic::scene(&demo::knobs_at(elapsed)),
                "minute {minute}"
            );
        }
        // It updates the strip only when the climate moved, or the hour came round.
        assert!(updates < 105, "{updates} updates");
        assert!(updates > 7 * 3, "{updates} updates");
    }

    #[test]
    fn the_live_feed_shows_what_the_engine_sends() {
        let start = preset::bare(terrarium_truth::PresentationState::Mature, Vec::new());
        let mut feeder = Feeder::live(start.clone(), vec!["moss".into()], Millis(0), &os(0, 0));
        assert_eq!(feeder.scene(), &start);
        assert_eq!(feeder.overlay().label, ["moss"]);
        // Nothing but the clock and the idle time moves it on its own.
        assert!(!feeder.tick(Millis(60_000), &os(60_000, 0), false));
        let thriving = preset::scene("thriving").unwrap();
        let label = vec!["moss".to_owned(), "main - clear".to_owned()];
        feeder.show(&thriving, label.clone(), Millis(70_000), &os(70_000, 0));
        assert_eq!(feeder.scene(), &thriving);
        assert_eq!(feeder.overlay().label, label);
        assert_eq!(feeder.overlay().clock, LocalClock::new(DATE, 70_000));
        assert!(!feeder.tick(Millis(80_000), &os(80_000, 0), false));
        assert_eq!(feeder.scene(), &thriving);
    }

    #[cfg(feature = "dev")]
    #[test]
    fn a_debug_choice_keeps_its_own_label_and_the_live_one_comes_back() {
        let start = preset::scene("thriving").unwrap();
        let live = vec!["moss".to_owned(), "main - clear".to_owned()];
        let mut feeder = Feeder::live(start.clone(), live.clone(), Millis(0), &os(0, 0));
        feeder.choose(
            Feed::Preset("dead"),
            Overrides::default(),
            Millis(1),
            &os(1, 0),
        );
        let later = vec!["moss".to_owned(), "main - storm".to_owned()];
        feeder.show(&start, later.clone(), Millis(2), &os(2, 0));
        assert_eq!(feeder.scene(), &preset::scene("dead").unwrap());
        assert_eq!(feeder.overlay().label, [copy::sample_label()]);
        feeder.choose(Feed::Live, Overrides::default(), Millis(3), &os(3, 0));
        assert_eq!(feeder.scene(), &start);
        assert_eq!(feeder.overlay().label, later);
    }

    #[test]
    fn a_wake_takes_the_clock_at_once() {
        let mut feeder = Feeder::new(Feed::Thriving, Millis(0), &os(12 * 3_600_000, 0));
        // Ten minutes on the monotonic clock, but the machine slept through the evening.
        let woke = os(23 * 3_600_000, 0);
        assert!(!feeder.tick(Millis(600_000), &woke, false));
        assert!(feeder.tick(Millis(600_000), &woke, true));
        assert_eq!(
            feeder.overlay().clock,
            LocalClock::new(DATE, 23 * 3_600_000)
        );
    }

    #[test]
    fn without_the_os_clock_the_last_one_is_carried_on() {
        let mut feeder = Feeder::new(Feed::Thriving, Millis(0), &os(23 * 3_600_000, 0));
        let blind = Os {
            clock: None,
            idle: None,
        };
        assert!(feeder.tick(Millis(2 * 3_600_000), &blind, true));
        assert_eq!(feeder.overlay().clock, LocalClock::new(DATE, 3_600_000));
    }

    #[test]
    fn the_os_local_time_becomes_the_composer_s_clock() {
        let now = LocalNow {
            year: 2026,
            month: 10,
            day: 1,
            ms_of_day: 1234,
        };
        assert_eq!(local_clock(now), LocalClock::new(DATE, 1234));
    }
}
