//! The debug window's simulation (dev builds): truth's own replay over a history made up as it
//! goes, so the climate sliders drive the real sim rather than the synthetic stand-in.
//!
//! Each simulated day brings what a repository would: commits by the rain slider and changed
//! lines by the wind slider, a coverage report reading the light slider, a dead-code report
//! reading the soil slider (so many unused of 100 exported items), and a local test run passing,
//! failing or unknown by the weather. The sim keeps them as truth keeps a repository's history
//! and observations, all in memory, and steps its state through each day's close, as a cached
//! state resumes: nothing new is ever earlier than the state's next tick. The scene plays
//! the event the live view would choose.
//!
//! It starts with 120 days at the usual pace, so rain and wind have a baseline to be measured
//! against: a slider at 200% reads as twice the usual once the 14-day average has caught up.
//! Three actions do what no slider does: tag a release, delete a 600-line directory, rewrite the
//! default branch.

use terrarium_engine::showing::Showing;
use terrarium_truth::domain::{
    DeletedDir, ObservationHead, ObservationValue, TagFact, WeatherSource,
};
use terrarium_truth::replay::first_close;
use terrarium_truth::scene::Weather;
use terrarium_truth::time::TICKS_PER_DAY;
use terrarium_truth::{
    Basis, CommitFacts, CommitId, Observation, Prepared, Q16, RepoSeed, SceneInput, SimState, Tick,
    TickGrid,
};

/// The sim's first hour: a UTC midnight, so each day closes 24 ticks on.
const ORIGIN_SECS: i64 = 1_704_067_200;
const SECONDS_PER_TICK: i64 = 3600;
/// Days at the usual pace before the sliders take over.
const WARM_UP_DAYS: u32 = 120;
/// The usual pace: commits and changed lines a day at 100%.
const USUAL_COMMITS: u32 = 4;
const USUAL_LINES: u64 = 120;
/// What a deleted directory held: enough to leave a fossil (500 lines).
const DELETED_LINES: u64 = 600;
/// The hours a day's commits start at, and when its reports are taken and a rewrite happens.
const FIRST_COMMIT_HOUR: u32 = 9;
const COMMIT_HOURS: u32 = 12;
const REWRITE_HOUR: u32 = 21;
const REPORT_HOUR: u32 = 22;
const DEFAULT_BRANCH: &str = "main";

/// What the sliders and the action buttons say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Coverage, 0–100.
    pub light: u8,
    /// Soil health, 0–100: the share of exported items in use.
    pub soil: u8,
    /// Commits and changed lines, as a percentage of the usual pace, 0–400.
    pub rain: u16,
    pub wind: u16,
    /// Clear passes the tests, storm fails them, haze reports nothing known.
    pub weather: Weather,
    /// Simulated days a second; 0 holds.
    pub speed: u8,
    /// How many times each action has been asked for: each count past what the sim has done
    /// is done on the next day.
    pub releases: u32,
    pub deletions: u32,
    pub rewrites: u32,
    /// Starting over makes a new sim.
    pub restarts: u32,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            light: 70,
            soil: 70,
            rain: 100,
            wind: 100,
            weather: Weather::Clear,
            speed: 1,
            releases: 0,
            deletions: 0,
            rewrites: 0,
            restarts: 0,
        }
    }
}

/// The weathers the sim can report: overcast has no test result that reads as it.
pub const WEATHERS: [Weather; 3] = [Weather::Clear, Weather::Storm, Weather::Haze];

/// An in-memory terrarium driven by [`Settings`].
pub struct Sandbox {
    basis: Basis,
    commits: Vec<CommitFacts>,
    observations: Vec<Observation>,
    state: SimState,
    showing: Showing,
    /// The next day to make.
    day: u32,
    /// The actions done so far, and the restart it began at.
    done: (u32, u32, u32),
    restarts: u32,
    scene: SceneInput,
}

impl Sandbox {
    /// A new sim: its warm-up at the usual pace, in the settings' light, soil and weather.
    pub fn new(settings: &Settings) -> Sandbox {
        let grid = TickGrid::from_origin(ORIGIN_SECS).expect("the origin is a whole hour");
        let root = commit_id(0);
        let basis = Basis::new(RepoSeed::from_root_commit(&root), grid, DEFAULT_BRANCH);
        let mut sandbox = Sandbox {
            state: SimState::new(basis.clone()),
            basis,
            commits: Vec::new(),
            observations: Vec::new(),
            showing: Showing::default(),
            day: 0,
            done: (settings.releases, settings.deletions, settings.rewrites),
            restarts: settings.restarts,
            scene: terrarium_compose::preset::bare(
                terrarium_truth::PresentationState::Thriving,
                Vec::new(),
            ),
        };
        let usual = Settings {
            rain: 100,
            wind: 100,
            ..*settings
        };
        for _ in 0..WARM_UP_DAYS {
            sandbox.make_day(&usual);
        }
        sandbox.advance();
        sandbox
    }

    /// The restart it began at.
    pub fn restarts(&self) -> u32 {
        self.restarts
    }

    /// Whether an action asked for is still to be done.
    pub fn pending(&self, settings: &Settings) -> bool {
        (settings.releases, settings.deletions, settings.rewrites) != self.done
    }

    /// Simulates `days` more days under `settings`; the scene is then the last day's.
    pub fn step(&mut self, settings: &Settings, days: u32) {
        for _ in 0..days {
            self.make_day(settings);
        }
        self.advance();
    }

    /// The scene at the close of the last day made.
    pub fn scene(&self) -> &SceneInput {
        &self.scene
    }

    /// How many days the sim has run, the warm-up's among them.
    pub fn day(&self) -> u32 {
        self.day
    }

    /// Brings the state through the close of the last day made, and projects its scene.
    fn advance(&mut self) {
        let close =
            Tick::new(first_close(self.basis.grid).hours() + (self.day - 1) * TICKS_PER_DAY);
        let prepared = Prepared::new(self.basis.clone(), &self.commits, &self.observations)
            .expect("the sim's inputs are in order");
        self.state
            .advance_through(&prepared, close)
            .expect("every new input is at or after the state's next tick");
        self.scene = self.showing.scene(&self.state);
    }

    /// Makes one day's commits and reports, and the actions asked for since the last.
    fn make_day(&mut self, settings: &Settings) {
        let day = self.day;
        let hour = |h: u32| Tick::new(day * TICKS_PER_DAY + h);
        let count = (u32::from(settings.rain) * USUAL_COMMITS + 50) / 100;
        let lines = u64::from(settings.wind) * USUAL_LINES / 100;
        let releases = settings.releases.saturating_sub(self.done.0);
        let deletions = settings.deletions.saturating_sub(self.done.1);
        // The root, and a release, needs a commit to stand on.
        let count = if self.commits.is_empty() || releases > 0 {
            count.max(1)
        } else {
            count
        };
        for k in 0..count {
            let added = lines / u64::from(count);
            let tags = (0..if k == 0 { releases } else { 0 })
                .map(|r| TagFact {
                    name: format!("v{}.0.0", self.done.0 + r + 1),
                    tick: hour(FIRST_COMMIT_HOUR),
                })
                .collect();
            self.commit(
                hour(FIRST_COMMIT_HOUR + k * COMMIT_HOURS / count),
                added,
                Vec::new(),
                tags,
            );
        }
        for d in 0..deletions {
            let dir = DeletedDir {
                path: format!("src/old-{}", self.done.1 + d + 1),
                lines: DELETED_LINES,
            };
            self.commit(
                hour(FIRST_COMMIT_HOUR + COMMIT_HOURS),
                0,
                vec![dir],
                Vec::new(),
            );
        }
        let rewrites = settings.rewrites.saturating_sub(self.done.2);
        for r in 0..rewrites {
            let old = self.tip();
            let new = commit_id(u64::from(u32::MAX) + u64::from(self.done.2 + r + 1));
            self.observe(
                hour(REWRITE_HOUR),
                r,
                None,
                ObservationValue::RefRewrite {
                    reference: format!("refs/heads/{DEFAULT_BRANCH}"),
                    old,
                    new,
                    rewritten: Some(3),
                    fossils: Vec::new(),
                },
            );
        }
        self.done = (settings.releases, settings.deletions, settings.rewrites);
        let ratio = |value: u8| Some(Q16::from_ratio(i32::from(value.min(100)), 100));
        self.observe(
            hour(REPORT_HOUR),
            0,
            ratio(settings.light),
            ObservationValue::Coverage,
        );
        self.observe(
            hour(REPORT_HOUR),
            1,
            ratio(settings.soil),
            ObservationValue::DeadCode {
                unused_items: u32::from(100 - settings.soil.min(100)),
                exported_items: 100,
            },
        );
        let pass = match settings.weather {
            Weather::Clear | Weather::Overcast => Some(Q16::ONE),
            Weather::Storm => Some(Q16::ZERO),
            Weather::Haze => None,
        };
        self.observe(
            hour(REPORT_HOUR),
            2,
            pass,
            ObservationValue::Weather {
                source: WeatherSource::Local,
            },
        );
        self.day += 1;
    }

    fn commit(
        &mut self,
        tick: Tick,
        added: u64,
        deleted_dirs: Vec<DeletedDir>,
        tags: Vec<TagFact>,
    ) {
        let removed = added / 3 + deleted_dirs.iter().map(|dir| dir.lines).sum::<u64>();
        self.commits.push(CommitFacts {
            id: commit_id(u64::try_from(self.commits.len()).unwrap_or(u64::MAX)),
            tick,
            parents: u32::from(!self.commits.is_empty()),
            on_default_first_parent: true,
            added,
            removed,
            deleted_dirs,
            renames: Vec::new(),
            tags,
        });
    }

    /// The default branch's tip: the last commit made.
    fn tip(&self) -> CommitId {
        self.commits
            .last()
            .map_or_else(|| commit_id(0), |commit| commit.id.clone())
    }

    /// Records a report taken at `tick`, the `nth` of that moment, at the tip.
    fn observe(&mut self, tick: Tick, nth: u32, normalized: Option<Q16>, value: ObservationValue) {
        let recorded_at = ORIGIN_SECS + i64::from(tick.hours()) * SECONDS_PER_TICK + i64::from(nth);
        self.observations.push(Observation {
            id: i64::try_from(self.observations.len()).unwrap_or(i64::MAX),
            recorded_at,
            head: ObservationHead {
                commit: self.tip(),
                branch: Some(DEFAULT_BRANCH.to_owned()),
                at_default_tip: true,
            },
            normalized,
            confidence: Q16::ONE,
            value,
        });
    }
}

/// The `n`th made-up commit's id.
fn commit_id(n: u64) -> CommitId {
    CommitId::from_hex(&format!("{n:040x}")).expect("forty hex digits")
}

#[cfg(test)]
mod tests {
    use terrarium_truth::PresentationState;
    use terrarium_truth::scene::EventKind;

    use super::*;

    /// The states and events the sim shows over `days` days under `settings`, a day a step.
    fn run(
        sandbox: &mut Sandbox,
        settings: &Settings,
        days: u32,
    ) -> (Vec<PresentationState>, Vec<EventKind>) {
        let mut states = Vec::new();
        let mut events = Vec::new();
        for _ in 0..days {
            sandbox.step(settings, 1);
            states.push(sandbox.scene().presentation.state);
            events.extend(sandbox.scene().event.map(|e| e.kind));
        }
        (states, events)
    }

    #[test]
    fn the_same_settings_make_the_same_terrarium() {
        let settings = Settings::default();
        let mut one = Sandbox::new(&settings);
        let mut two = Sandbox::new(&settings);
        one.step(&settings, 10);
        two.step(&settings, 10);
        assert_eq!(one.scene(), two.scene());
        assert_eq!(one.day(), WARM_UP_DAYS + 10);
        assert_eq!(one.scene().presentation.state, PresentationState::Thriving);
    }

    #[test]
    fn poor_soil_struggles_and_good_soil_brings_it_back() {
        let poor = Settings {
            soil: 20,
            ..Settings::default()
        };
        let mut sandbox = Sandbox::new(&Settings::default());
        let (states, _) = run(&mut sandbox, &poor, 40);
        assert_eq!(states.last(), Some(&PresentationState::Struggling));
        assert!(sandbox.scene().weeds > 0, "weeds take the poor soil");
        let good = Settings {
            light: 90,
            soil: 90,
            ..Settings::default()
        };
        let (states, events) = run(&mut sandbox, &good, 40);
        assert!(
            states.contains(&PresentationState::Recovering),
            "{states:?}"
        );
        assert!(events.contains(&EventKind::ClearingBurn), "{events:?}");
    }

    #[test]
    fn a_month_of_silence_goes_dormant_and_the_first_commit_back_rains() {
        let silent = Settings {
            rain: 0,
            wind: 0,
            ..Settings::default()
        };
        let mut sandbox = Sandbox::new(&Settings::default());
        let (states, _) = run(&mut sandbox, &silent, 35);
        assert_eq!(states.last(), Some(&PresentationState::Dormant));
        let (_, events) = run(&mut sandbox, &Settings::default(), 2);
        assert!(events.contains(&EventKind::RainAfterDrought), "{events:?}");
    }

    #[test]
    fn each_action_plays_its_event_and_each_failure_a_storm() {
        let mut settings = Settings::default();
        let mut sandbox = Sandbox::new(&settings);
        let mut played = |settings: &Settings| run(&mut sandbox, settings, 1).1;
        settings.releases += 1;
        assert_eq!(played(&settings), [EventKind::Flowering]);
        settings.deletions += 1;
        assert_eq!(played(&settings), [EventKind::Sinkhole]);
        settings.rewrites += 1;
        assert_eq!(played(&settings), [EventKind::Earthquake]);
        settings.weather = Weather::Storm;
        assert_eq!(played(&settings), [EventKind::Storm]);
        // Asked for once, each is done once.
        assert!(
            !played(&Settings {
                weather: Weather::Storm,
                ..settings
            })
            .contains(&EventKind::Flowering)
        );
    }
}
