//! The settings the person chooses, typed as the app uses them, and kept in the store's
//! `settings` table under fixed keys, one JSON value each.
//!
//! Loading never fails: a key that is absent, or holds a value this version cannot read, gives
//! its default, so a settings row written by hand or by another version never stops the strip.
//! A change is one [`Pref`]: applied to the model the strip runs on, then written by
//! [`Writer`], a thread of its own, so the event loop never waits on the database.
//!
//! The GitHub token is no setting: [`Writer`] keeps it in the keychain, and it never reaches
//! the `settings` table, a [`Pref`] or an answer.

use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use terrarium_adapters::binding::{CiRepository, RepoBinding};
use terrarium_adapters::ci::{Keychain, KeychainError, Token};
use terrarium_adapters::discovery::{Enabled, Overrides};
use terrarium_compose::clock::DayNight;
use terrarium_compose::scale::SizeStep;
use terrarium_engine::lifecycle::{AdapterSettings, stored_adapters};
use terrarium_platform::geometry::Edge;
use terrarium_platform::login::{LoginError, LoginStatus};
use terrarium_store::identifiers::{
    SETTING_CI_GITHUB_REPO, SETTING_COMPLEXITY_ENABLED, SETTING_COVERAGE_ENABLED,
    SETTING_COVERAGE_REPORT_PATH, SETTING_CRASHLOG_OPT_IN, SETTING_DAYNIGHT_DAY_START,
    SETTING_DAYNIGHT_NIGHT_START, SETTING_DEAD_CODE_ENABLED, SETTING_DEAD_CODE_REPORT_PATH,
    SETTING_DIM_AFTER_MINUTES, SETTING_HOTKEYS_RAISE, SETTING_HOTKEYS_TIMELAPSE,
    SETTING_KNIP_RUN_OPT_IN, SETTING_LOGIN_OPEN_AT_LOGIN, SETTING_STRIP_EDGE,
    SETTING_STRIP_MONITOR, SETTING_STRIP_SIZE_STEP, SETTING_TODO_DENSITY_ENABLED,
    SETTING_WEATHER_ENABLED, SETTING_WEATHER_REPORT_PATH,
};
use terrarium_store::{DataDir, Store, StoreError};

use crate::hotkey::{Binding, Which};
use crate::login::LoginItem;
use crate::monitor::MonitorId;

/// How long without input before the strip dims, in minutes: the default, and the range
/// settings offer. Dim mode cannot be turned off; it is the strip's guard against burn-in.
pub const DIM_DEFAULT: u32 = 30;
pub const DIM_MIN: u32 = 5;
pub const DIM_MAX: u32 = 120;
pub const DIM_STEP: u32 = 5;
const MINUTES_PER_DAY: u16 = 24 * 60;

/// The dim timeout of `minutes`.
pub fn dim_after(minutes: u32) -> Duration {
    Duration::from_secs(u64::from(minutes) * 60)
}
const MS_PER_MINUTE: u32 = 60_000;

/// A time of day to the minute, stored as `"HH:MM"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeOfDay(u16);

impl TimeOfDay {
    pub const fn new(hour: u16, minute: u16) -> Option<TimeOfDay> {
        if hour < 24 && minute < 60 {
            Some(TimeOfDay(hour * 60 + minute))
        } else {
            None
        }
    }

    pub const fn hour(self) -> u16 {
        self.0 / 60
    }

    pub const fn minute(self) -> u16 {
        self.0 % 60
    }

    /// `"HH:MM"`, and nothing else: two digits each.
    pub fn parse(text: &str) -> Option<TimeOfDay> {
        let (hour, minute) = text.split_once(':')?;
        if hour.len() != 2 || minute.len() != 2 {
            return None;
        }
        TimeOfDay::new(hour.parse().ok()?, minute.parse().ok()?)
    }

    pub fn to_setting(self) -> String {
        format!("{:02}:{:02}", self.hour(), self.minute())
    }

    const fn ms(self) -> u32 {
        // At most 1,439 minutes: well inside a u32 of milliseconds.
        self.0 as u32 * MS_PER_MINUTE
    }
}

/// The adapters settings can turn off: the reports, each with its report path, and the metrics
/// measured from the committed source, which have none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AdapterKind {
    Coverage,
    /// Test results: a `JUnit` report and the `record-tests` status file (weather).
    Tests,
    /// The Knip report.
    DeadCode,
    Complexity,
    TodoDensity,
}

impl AdapterKind {
    pub const ALL: [AdapterKind; 5] = [
        AdapterKind::Coverage,
        AdapterKind::Tests,
        AdapterKind::DeadCode,
        AdapterKind::Complexity,
        AdapterKind::TodoDensity,
    ];

    const fn enabled_key(self) -> &'static str {
        match self {
            AdapterKind::Coverage => SETTING_COVERAGE_ENABLED,
            AdapterKind::Tests => SETTING_WEATHER_ENABLED,
            AdapterKind::DeadCode => SETTING_DEAD_CODE_ENABLED,
            AdapterKind::Complexity => SETTING_COMPLEXITY_ENABLED,
            AdapterKind::TodoDensity => SETTING_TODO_DENSITY_ENABLED,
        }
    }

    /// The setting holding its report path; `None` for a metric, which reads no report.
    const fn path_key(self) -> Option<&'static str> {
        match self {
            AdapterKind::Coverage => Some(SETTING_COVERAGE_REPORT_PATH),
            AdapterKind::Tests => Some(SETTING_WEATHER_REPORT_PATH),
            AdapterKind::DeadCode => Some(SETTING_DEAD_CODE_REPORT_PATH),
            AdapterKind::Complexity | AdapterKind::TodoDensity => None,
        }
    }

    pub fn enabled(self, enabled: Enabled) -> bool {
        match self {
            AdapterKind::Coverage => enabled.coverage,
            AdapterKind::Tests => enabled.weather,
            AdapterKind::DeadCode => enabled.dead_code,
            AdapterKind::Complexity => enabled.complexity,
            AdapterKind::TodoDensity => enabled.todo_density,
        }
    }

    fn set_enabled(self, enabled: &mut Enabled, on: bool) {
        match self {
            AdapterKind::Coverage => enabled.coverage = on,
            AdapterKind::Tests => enabled.weather = on,
            AdapterKind::DeadCode => enabled.dead_code = on,
            AdapterKind::Complexity => enabled.complexity = on,
            AdapterKind::TodoDensity => enabled.todo_density = on,
        }
    }

    pub fn path(self, overrides: &Overrides) -> Option<&str> {
        match self {
            AdapterKind::Coverage => overrides.coverage.as_deref(),
            AdapterKind::Tests => overrides.weather.as_deref(),
            AdapterKind::DeadCode => overrides.dead_code.as_deref(),
            AdapterKind::Complexity | AdapterKind::TodoDensity => None,
        }
    }

    fn set_path(self, overrides: &mut Overrides, path: Option<String>) {
        match self {
            AdapterKind::Coverage => overrides.coverage = path,
            AdapterKind::Tests => overrides.weather = path,
            AdapterKind::DeadCode => overrides.dead_code = path,
            AdapterKind::Complexity | AdapterKind::TodoDensity => {}
        }
    }
}

/// Every setting the strip and its windows read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefs {
    /// The monitor chosen; `None` leaves the choice to the strip (`monitor::choose`).
    pub monitor: Option<MonitorId>,
    pub edge: Edge,
    pub size_step: SizeStep,
    pub raise: Binding,
    pub timelapse: Binding,
    pub day_starts: TimeOfDay,
    pub night_starts: TimeOfDay,
    pub dim_after_minutes: u32,
    pub open_at_login: bool,
    pub crash_log: bool,
    pub adapters: AdapterSettings,
}

impl Default for Prefs {
    fn default() -> Prefs {
        let day_night = DayNight::default();
        Prefs {
            monitor: None,
            edge: Edge::Bottom,
            size_step: SizeStep::Full,
            raise: Which::Raise.default_binding(),
            timelapse: Which::TimeLapse.default_binding(),
            day_starts: of_ms(day_night.day_starts),
            night_starts: of_ms(day_night.night_starts),
            dim_after_minutes: DIM_DEFAULT,
            open_at_login: false,
            crash_log: false,
            adapters: AdapterSettings::default(),
        }
    }
}

/// The time of day `ms` after midnight falls in, to the minute.
fn of_ms(ms: u32) -> TimeOfDay {
    TimeOfDay(u16::try_from(ms / MS_PER_MINUTE).unwrap_or(0) % MINUTES_PER_DAY)
}

/// One change of one setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pref {
    Monitor(Option<MonitorId>),
    Edge(Edge),
    SizeStep(SizeStep),
    Hotkey(Which, Binding),
    DayStarts(TimeOfDay),
    NightStarts(TimeOfDay),
    DimAfter(u32),
    OpenAtLogin(bool),
    CrashLog(bool),
    AdapterEnabled(AdapterKind, bool),
    /// A report path, relative to the repository; `None` goes back to the fixed places.
    ReportPath(AdapterKind, Option<String>),
    /// The repository Knip may run in, the connected one; `None` withdraws it.
    KnipRun(Option<RepoBinding>),
    /// The GitHub repository typed for the connected repository's CI weather; `None` goes back
    /// to the one its `origin` names.
    CiRepository(Option<CiRepository>),
}

impl Pref {
    /// Whether the engine reads it: an adapter's setting, which a refresh then takes in.
    pub const fn is_adapter(&self) -> bool {
        matches!(
            self,
            Pref::AdapterEnabled(..)
                | Pref::ReportPath(..)
                | Pref::KnipRun(_)
                | Pref::CiRepository(_)
        )
    }
}

/// A change to the GitHub token in the keychain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenChange {
    Save(Token),
    Remove,
}

impl Prefs {
    /// What `store` holds, each setting it lacks or cannot read at its default.
    pub fn load(store: &Store) -> Prefs {
        let defaults = Prefs::default();
        let read = |key: &str| -> Option<serde_json::Value> { store.setting(key).ok().flatten() };
        let text = |key: &str| read(key).and_then(|v| v.as_str().map(str::to_owned));
        let flag = |key: &str| read(key).and_then(|v| v.as_bool());
        let edge = match text(SETTING_STRIP_EDGE).as_deref() {
            Some("top") => Edge::Top,
            _ => Edge::Bottom,
        };
        let size_step = match read(SETTING_STRIP_SIZE_STEP).and_then(|v| v.as_i64()) {
            Some(-1) => SizeStep::OneSmaller,
            _ => SizeStep::Full,
        };
        // A binding the settings window would refuse, a bare key above all, is never
        // registered: it would take that key from every app.
        let binding = |key: &str| {
            text(key)
                .and_then(|text| Binding::parse(&text))
                .filter(|binding| binding.allowed().is_ok())
        };
        let raise = binding(SETTING_HOTKEYS_RAISE).unwrap_or(defaults.raise);
        let timelapse = binding(SETTING_HOTKEYS_TIMELAPSE)
            .filter(|binding| *binding != raise)
            .unwrap_or(defaults.timelapse);
        let day = text(SETTING_DAYNIGHT_DAY_START).and_then(|t| TimeOfDay::parse(&t));
        let night = text(SETTING_DAYNIGHT_NIGHT_START).and_then(|t| TimeOfDay::parse(&t));
        let (day_starts, night_starts) = match (
            day.unwrap_or(defaults.day_starts),
            night.unwrap_or(defaults.night_starts),
        ) {
            // Equal times would leave no night at all: both go back to their defaults.
            (day, night) if day == night => (defaults.day_starts, defaults.night_starts),
            times => times,
        };
        let dim_after_minutes = read(SETTING_DIM_AFTER_MINUTES)
            .and_then(|v| v.as_u64())
            .and_then(|minutes| u32::try_from(minutes).ok())
            .filter(|minutes| (DIM_MIN..=DIM_MAX).contains(minutes))
            .unwrap_or(DIM_DEFAULT);
        Prefs {
            monitor: text(SETTING_STRIP_MONITOR).map(MonitorId),
            edge,
            size_step,
            raise,
            timelapse,
            day_starts,
            night_starts,
            dim_after_minutes,
            open_at_login: flag(SETTING_LOGIN_OPEN_AT_LOGIN).unwrap_or(defaults.open_at_login),
            crash_log: flag(SETTING_CRASHLOG_OPT_IN).unwrap_or(defaults.crash_log),
            adapters: stored_adapters(store).unwrap_or_default(),
        }
    }

    /// The day and night the composer is given.
    pub fn day_night(&self) -> DayNight {
        DayNight {
            day_starts: self.day_starts.ms(),
            night_starts: self.night_starts.ms(),
        }
    }

    /// How long without input before the strip dims.
    pub fn dim_after(&self) -> Duration {
        dim_after(self.dim_after_minutes)
    }

    /// The keys `which` hotkey is set to.
    pub fn binding(&self, which: Which) -> Binding {
        match which {
            Which::Raise => self.raise,
            Which::TimeLapse => self.timelapse,
        }
    }

    /// Takes `pref` into the model.
    pub fn apply(&mut self, pref: &Pref) {
        match pref {
            Pref::Monitor(monitor) => self.monitor.clone_from(monitor),
            Pref::Edge(edge) => self.edge = *edge,
            Pref::SizeStep(step) => self.size_step = *step,
            Pref::Hotkey(Which::Raise, binding) => self.raise = *binding,
            Pref::Hotkey(Which::TimeLapse, binding) => self.timelapse = *binding,
            Pref::DayStarts(time) => self.day_starts = *time,
            Pref::NightStarts(time) => self.night_starts = *time,
            Pref::DimAfter(minutes) => self.dim_after_minutes = *minutes,
            Pref::OpenAtLogin(on) => self.open_at_login = *on,
            Pref::CrashLog(on) => self.crash_log = *on,
            Pref::AdapterEnabled(kind, on) => kind.set_enabled(&mut self.adapters.enabled, *on),
            Pref::ReportPath(kind, path) => {
                kind.set_path(&mut self.adapters.overrides, path.clone());
            }
            Pref::KnipRun(binding) => self.adapters.knip_run.clone_from(binding),
            Pref::CiRepository(typed) => self.adapters.ci_repository.clone_from(typed),
        }
    }
}

/// Writes `pref` under its key.
pub fn save(store: &mut Store, pref: &Pref) -> Result<(), StoreError> {
    match pref {
        Pref::Monitor(Some(id)) => store.set_setting(SETTING_STRIP_MONITOR, &id.0),
        Pref::Monitor(None) => store.clear_setting(SETTING_STRIP_MONITOR),
        Pref::Edge(edge) => store.set_setting(
            SETTING_STRIP_EDGE,
            match edge {
                Edge::Bottom => "bottom",
                Edge::Top => "top",
            },
        ),
        Pref::SizeStep(step) => store.set_setting(
            SETTING_STRIP_SIZE_STEP,
            &match step {
                SizeStep::Full => 0_i64,
                SizeStep::OneSmaller => -1,
            },
        ),
        Pref::Hotkey(which, binding) => {
            let key = match which {
                Which::Raise => SETTING_HOTKEYS_RAISE,
                Which::TimeLapse => SETTING_HOTKEYS_TIMELAPSE,
            };
            store.set_setting(key, &binding.to_setting())
        }
        Pref::DayStarts(time) => store.set_setting(SETTING_DAYNIGHT_DAY_START, &time.to_setting()),
        Pref::NightStarts(time) => {
            store.set_setting(SETTING_DAYNIGHT_NIGHT_START, &time.to_setting())
        }
        Pref::DimAfter(minutes) => store.set_setting(SETTING_DIM_AFTER_MINUTES, minutes),
        Pref::OpenAtLogin(on) => store.set_setting(SETTING_LOGIN_OPEN_AT_LOGIN, on),
        Pref::CrashLog(on) => store.set_setting(SETTING_CRASHLOG_OPT_IN, on),
        Pref::AdapterEnabled(kind, on) => store.set_setting(kind.enabled_key(), on),
        // The adapters tab asks for a path only of a report.
        Pref::ReportPath(kind, path) => match (kind.path_key(), path) {
            (Some(key), Some(path)) => store.set_setting(key, path),
            (Some(key), None) => store.clear_setting(key),
            (None, _) => Ok(()),
        },
        Pref::KnipRun(Some(binding)) => store.set_setting(SETTING_KNIP_RUN_OPT_IN, binding),
        Pref::KnipRun(None) => store.clear_setting(SETTING_KNIP_RUN_OPT_IN),
        Pref::CiRepository(Some(typed)) => store.set_setting(SETTING_CI_GITHUB_REPO, typed),
        Pref::CiRepository(None) => store.clear_setting(SETTING_CI_GITHUB_REPO),
    }
}

/// What the writer did with one change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Saved {
    /// `pref` was written, or `problem` says why not. Opening at login is also asked of the OS:
    /// `login` is its refusal, if it gave one.
    Pref {
        pref: Pref,
        problem: Option<String>,
        login: Option<LoginError>,
    },
    /// What the OS says of opening at login.
    LoginStatus(LoginStatus),
    /// The token was saved (`saved`) or removed, or `problem` says why the keychain would not.
    Token {
        saved: bool,
        problem: Option<KeychainError>,
    },
}

enum Job {
    Save(Pref),
    LoginStatus,
    Token(TokenChange),
}

/// The thread that writes settings, in the order they were changed, and asks the OS about
/// opening at login, which can also take a moment. Its answers go to `answer`.
pub struct Writer {
    jobs: Sender<Job>,
}

impl Writer {
    /// Starts the writer on the store in `data`, keeping the token in `keychain`. Settings
    /// changed before the store opens wait for it; if it cannot open, each change answers with
    /// why.
    pub fn start(
        data: DataDir,
        mut login: Box<dyn LoginItem + Send>,
        status: fn() -> LoginStatus,
        keychain: Arc<dyn Keychain>,
        answer: Box<dyn Fn(Saved) + Send>,
    ) -> Writer {
        let (jobs, rx) = mpsc::channel::<Job>();
        let _ = thread::Builder::new()
            .name("terrarium-settings".into())
            .spawn(move || {
                let mut store = Store::open(&data);
                for job in rx {
                    match job {
                        Job::Save(pref) => {
                            let problem = match &mut store {
                                Ok(store) => save(store, &pref).err().map(|err| err.to_string()),
                                Err(err) => Some(err.to_string()),
                            };
                            let login = match pref {
                                Pref::OpenAtLogin(on) if problem.is_none() => login.set(on).err(),
                                _ => None,
                            };
                            answer(Saved::Pref {
                                pref,
                                problem,
                                login,
                            });
                        }
                        Job::LoginStatus => answer(Saved::LoginStatus(status())),
                        Job::Token(change) => {
                            let saved = matches!(change, TokenChange::Save(_));
                            let done = match &change {
                                TokenChange::Save(token) => keychain.save(token),
                                TokenChange::Remove => keychain.remove(),
                            };
                            answer(Saved::Token {
                                saved,
                                problem: done.err(),
                            });
                        }
                    }
                }
            });
        Writer { jobs }
    }

    pub fn save(&self, pref: Pref) {
        let _ = self.jobs.send(Job::Save(pref));
    }

    pub fn ask_login_status(&self) {
        let _ = self.jobs.send(Job::LoginStatus);
    }

    /// Saves or removes the GitHub token in the keychain.
    pub fn change_token(&self, change: TokenChange) {
        let _ = self.jobs.send(Job::Token(change));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use terrarium_adapters::ci::MemoryKeychain;
    use terrarium_platform::login::LoginError;

    use super::*;
    use crate::hotkey::Mods;

    /// A store in a temp dir of its own, closed before the dir goes.
    struct Fixture {
        store: Store,
        data: DataDir,
        _dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Fixture {
            let dir = tempfile::tempdir().unwrap();
            let data = DataDir::at(dir.path());
            Fixture {
                store: Store::open(&data).unwrap(),
                data,
                _dir: dir,
            }
        }
    }

    fn changed() -> Vec<Pref> {
        let binding = Binding::parse("ctrl+shift+K").unwrap();
        vec![
            Pref::Monitor(Some(MonitorId(
                "37D8832A-2D66-02CA-B9F7-8F30A301B230".into(),
            ))),
            Pref::Edge(Edge::Top),
            Pref::SizeStep(SizeStep::OneSmaller),
            Pref::Hotkey(Which::Raise, binding),
            Pref::Hotkey(
                Which::TimeLapse,
                Binding {
                    mods: Mods {
                        control: true,
                        command: true,
                        ..Mods::default()
                    },
                    ..binding
                },
            ),
            Pref::DayStarts(TimeOfDay::new(6, 15).unwrap()),
            Pref::NightStarts(TimeOfDay::new(21, 45).unwrap()),
            Pref::DimAfter(10),
            Pref::OpenAtLogin(true),
            Pref::CrashLog(true),
            Pref::AdapterEnabled(AdapterKind::Coverage, false),
            Pref::AdapterEnabled(AdapterKind::Tests, false),
            Pref::AdapterEnabled(AdapterKind::DeadCode, false),
            Pref::AdapterEnabled(AdapterKind::Complexity, false),
            Pref::AdapterEnabled(AdapterKind::TodoDensity, false),
            Pref::ReportPath(AdapterKind::Coverage, Some("out/lcov.info".into())),
            Pref::ReportPath(AdapterKind::Tests, Some("out/junit.xml".into())),
            Pref::ReportPath(AdapterKind::DeadCode, Some("out/knip.json".into())),
            Pref::KnipRun(Some(moss())),
            Pref::CiRepository(Some(CiRepository {
                binding: moss(),
                repo: terrarium_adapters::ci::Slug::parse("example-garden/moss-bed").unwrap(),
            })),
        ]
    }

    /// The connected repository, as a setting given for it names it.
    fn moss() -> RepoBinding {
        RepoBinding {
            seed: "5e".repeat(32),
            path: "/projects/moss".into(),
        }
    }

    #[test]
    fn an_empty_store_gives_every_default() {
        let fx = Fixture::new();
        let prefs = Prefs::load(&fx.store);
        assert_eq!(prefs, Prefs::default());
        assert_eq!(prefs.day_night(), DayNight::default());
        assert_eq!(prefs.raise.to_setting(), "ctrl+alt+T");
        assert_eq!(prefs.timelapse.to_setting(), "ctrl+alt+H");
        assert_eq!(prefs.dim_after_minutes, 30);
        assert!(prefs.adapters.enabled.coverage && prefs.adapters.enabled.weather);
    }

    #[test]
    fn every_setting_written_reads_back_as_it_was_changed() {
        let mut fx = Fixture::new();
        let mut expected = Prefs::default();
        for pref in changed() {
            save(&mut fx.store, &pref).unwrap();
            expected.apply(&pref);
            assert_eq!(Prefs::load(&fx.store), expected, "after {pref:?}");
        }
        assert_ne!(expected, Prefs::default());
        // Going back: the monitor to the strip's choice, the paths to the fixed places.
        for pref in [
            Pref::Monitor(None),
            Pref::ReportPath(AdapterKind::Coverage, None),
            Pref::Edge(Edge::Bottom),
            Pref::SizeStep(SizeStep::Full),
            Pref::KnipRun(None),
            Pref::CiRepository(None),
        ] {
            save(&mut fx.store, &pref).unwrap();
            expected.apply(&pref);
        }
        let back = Prefs::load(&fx.store);
        assert_eq!(back, expected);
        assert_eq!(back.monitor, None);
        assert_eq!(back.adapters.overrides.coverage, None);
        assert_eq!(back.adapters.knip_run, None);
        assert_eq!(back.adapters.ci_repository, None);
        assert_eq!(
            fx.store.setting::<String>(SETTING_STRIP_MONITOR).unwrap(),
            None
        );
    }

    #[test]
    fn values_stored_as_section_4_7_reads_them() {
        let mut fx = Fixture::new();
        for pref in changed() {
            save(&mut fx.store, &pref).unwrap();
        }
        let raw = |key: &str| fx.store.setting::<serde_json::Value>(key).unwrap().unwrap();
        assert_eq!(raw(SETTING_STRIP_EDGE), "top");
        assert_eq!(raw(SETTING_STRIP_SIZE_STEP), -1);
        assert_eq!(raw(SETTING_HOTKEYS_RAISE), "ctrl+shift+K");
        assert_eq!(raw(SETTING_DAYNIGHT_DAY_START), "06:15");
        assert_eq!(raw(SETTING_DAYNIGHT_NIGHT_START), "21:45");
        assert_eq!(raw(SETTING_DIM_AFTER_MINUTES), 10);
        assert_eq!(raw(SETTING_COVERAGE_ENABLED), false);
        assert_eq!(raw(SETTING_COMPLEXITY_ENABLED), false);
        assert_eq!(raw(SETTING_TODO_DENSITY_ENABLED), false);
        assert_eq!(raw(SETTING_CRASHLOG_OPT_IN), true);
    }

    #[test]
    fn a_value_this_version_cannot_read_falls_back_to_its_default() {
        let mut fx = Fixture::new();
        let store = &mut fx.store;
        store.set_setting(SETTING_STRIP_EDGE, "left").unwrap();
        store.set_setting(SETTING_STRIP_SIZE_STEP, &-2).unwrap();
        store
            .set_setting(SETTING_HOTKEYS_RAISE, "ctrl+alt+")
            .unwrap();
        store.set_setting(SETTING_HOTKEYS_TIMELAPSE, &7).unwrap();
        store
            .set_setting(SETTING_DAYNIGHT_DAY_START, "7:00")
            .unwrap();
        store
            .set_setting(SETTING_DAYNIGHT_NIGHT_START, "24:00")
            .unwrap();
        store.set_setting(SETTING_DIM_AFTER_MINUTES, &0).unwrap();
        store
            .set_setting(SETTING_LOGIN_OPEN_AT_LOGIN, "yes")
            .unwrap();
        store.set_setting(SETTING_STRIP_MONITOR, &3).unwrap();
        store.set_setting(SETTING_COVERAGE_ENABLED, "off").unwrap();
        assert_eq!(Prefs::load(store), Prefs::default());
        // Equal day and night would leave no night: both go back to their defaults.
        store
            .set_setting(SETTING_DAYNIGHT_DAY_START, "08:00")
            .unwrap();
        store
            .set_setting(SETTING_DAYNIGHT_NIGHT_START, "08:00")
            .unwrap();
        store
            .set_setting(SETTING_DIM_AFTER_MINUTES, &(DIM_MAX + 1))
            .unwrap();
        assert_eq!(Prefs::load(store), Prefs::default());
        // Bindings the rules refuse are never loaded: a bare key, Option and Shift alone.
        for refused in ["T", "shift+T", "alt+shift+T", "super+T"] {
            store.set_setting(SETTING_HOTKEYS_RAISE, refused).unwrap();
            store
                .set_setting(SETTING_HOTKEYS_TIMELAPSE, refused)
                .unwrap();
            let prefs = Prefs::load(store);
            assert_eq!(prefs.raise, Prefs::default().raise, "{refused}");
            assert_eq!(prefs.timelapse, Prefs::default().timelapse, "{refused}");
        }
        store.clear_setting(SETTING_HOTKEYS_TIMELAPSE).unwrap();
        // Two equal hotkeys: the raise keeps its own.
        store
            .set_setting(SETTING_HOTKEYS_RAISE, "ctrl+alt+H")
            .unwrap();
        let prefs = Prefs::load(store);
        assert_eq!(prefs.raise.to_setting(), "ctrl+alt+H");
        assert_eq!(prefs.timelapse, Prefs::default().timelapse);
        assert_eq!(prefs.raise, prefs.timelapse, "the default itself is taken");
    }

    #[test]
    fn a_time_of_day_is_two_digits_each_and_inside_the_day() {
        assert_eq!(TimeOfDay::parse("07:00"), TimeOfDay::new(7, 0));
        assert_eq!(TimeOfDay::parse("23:59"), TimeOfDay::new(23, 59));
        for bad in [
            "7:00", "07:0", "24:00", "07:60", "0700", "07:00 ", "-1:00", "",
        ] {
            assert_eq!(TimeOfDay::parse(bad), None, "{bad}");
        }
        assert_eq!(TimeOfDay::new(19, 30).unwrap().to_setting(), "19:30");
        assert_eq!(
            Prefs {
                day_starts: TimeOfDay::new(6, 0).unwrap(),
                night_starts: TimeOfDay::new(1, 15).unwrap(),
                ..Prefs::default()
            }
            .day_night(),
            DayNight {
                day_starts: 6 * 3_600_000,
                night_starts: 75 * 60_000,
            }
        );
    }

    /// A login item that records what it is asked, and refuses.
    struct Refusing(Arc<Mutex<Vec<bool>>>);

    impl LoginItem for Refusing {
        fn set(&mut self, on: bool) -> Result<(), LoginError> {
            self.0.lock().unwrap().push(on);
            Err(LoginError::NotBundled)
        }
    }

    #[test]
    fn the_writer_saves_in_order_and_asks_the_os_only_about_opening_at_login() {
        let fx = Fixture::new();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let writer = Writer::start(
            fx.data.clone(),
            Box::new(Refusing(Arc::clone(&asked))),
            || LoginStatus::Unavailable,
            Arc::new(MemoryKeychain::default()),
            Box::new(move |saved| tx.send(saved).unwrap()),
        );
        writer.save(Pref::DimAfter(15));
        writer.save(Pref::DimAfter(20));
        writer.save(Pref::OpenAtLogin(true));
        writer.ask_login_status();
        let answers: Vec<Saved> = (0..4).map(|_| rx.recv().unwrap()).collect();
        assert_eq!(
            answers,
            [
                Saved::Pref {
                    pref: Pref::DimAfter(15),
                    problem: None,
                    login: None
                },
                Saved::Pref {
                    pref: Pref::DimAfter(20),
                    problem: None,
                    login: None
                },
                Saved::Pref {
                    pref: Pref::OpenAtLogin(true),
                    problem: None,
                    login: Some(LoginError::NotBundled)
                },
                Saved::LoginStatus(LoginStatus::Unavailable),
            ]
        );
        assert_eq!(*asked.lock().unwrap(), [true]);
        let store = Store::open(&fx.data).unwrap();
        let prefs = Prefs::load(&store);
        assert_eq!(prefs.dim_after_minutes, 20);
        assert!(
            prefs.open_at_login,
            "the choice is kept when the OS refuses"
        );
    }

    #[test]
    fn the_token_goes_to_the_keychain_and_never_into_the_settings() {
        let fx = Fixture::new();
        let keychain = Arc::new(MemoryKeychain::default());
        let (tx, rx) = mpsc::channel();
        let writer = Writer::start(
            fx.data.clone(),
            Box::new(Refusing(Arc::default())),
            || LoginStatus::Unavailable,
            Arc::clone(&keychain) as Arc<dyn Keychain>,
            Box::new(move |saved| tx.send(saved).unwrap()),
        );
        let secret = "github_pat_fictional_sentinel";
        writer.change_token(TokenChange::Save(Token::new(secret).unwrap()));
        for pref in changed() {
            writer.save(pref);
        }
        let answers: Vec<Saved> = (0..=changed().len()).map(|_| rx.recv().unwrap()).collect();
        assert_eq!(
            answers[0],
            Saved::Token {
                saved: true,
                problem: None
            }
        );
        assert_eq!(keychain.held().as_deref(), Some(secret));
        assert!(!format!("{answers:?}").contains("sentinel"));
        drop(writer);
        // Every file the data directory holds, the database's journal included: none holds it.
        for entry in std::fs::read_dir(fx.data.database().parent().unwrap()).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let bytes = std::fs::read(&path).unwrap();
                assert!(
                    !String::from_utf8_lossy(&bytes).contains("sentinel"),
                    "{path:?}"
                );
            }
        }
        let (tx, rx) = mpsc::channel();
        let writer = Writer::start(
            fx.data.clone(),
            Box::new(Refusing(Arc::default())),
            || LoginStatus::Unavailable,
            Arc::clone(&keychain) as Arc<dyn Keychain>,
            Box::new(move |saved| tx.send(saved).unwrap()),
        );
        writer.change_token(TokenChange::Remove);
        assert_eq!(
            rx.recv().unwrap(),
            Saved::Token {
                saved: false,
                problem: None
            }
        );
        assert_eq!(keychain.held(), None);
    }
}
