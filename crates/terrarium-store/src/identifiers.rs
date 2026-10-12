//! The names data and credentials live under (I-12). Renaming the product changes none of them;
//! the test below pins each one.

/// The bundle identifier, which also names the data directory.
pub const BUNDLE_ID: &str = "io.github.faircat-gd.repo-terrarium";
/// The executable's name.
pub const BINARY: &str = "repo-terrarium";
/// Every workspace crate's name starts with it.
pub const CRATE_PREFIX: &str = "terrarium-";
/// The database's file name inside the data directory.
pub const DATABASE_FILE: &str = "terrarium.sqlite3";
/// The folder inside the data directory where `record-tests` writes one status file per
/// repository, `<seed-hex>.json`.
pub const LOCAL_TESTS_DIR: &str = "local-tests";
/// The keychain service a CI token is stored under.
pub const KEYCHAIN_SERVICE: &str = BUNDLE_ID;
/// The keychain account the GitHub token is stored under: one for every repository.
pub const KEYCHAIN_ACCOUNT_GITHUB: &str = "github-token";
/// The environment variable that points tests and harnesses at a data directory they own.
pub const DATA_DIR_ENV: &str = "TERRARIUM_DATA_DIR";
/// The file inside the data directory the running app holds a lock on, so a second launch
/// finds it: another version that named it otherwise would run beside it.
pub const INSTANCE_LOCK_FILE: &str = "instance.lock";
/// The folder inside the data directory where a second launch leaves its request for the
/// running app, and the request's file name.
pub const REQUESTS_DIR: &str = "requests";
pub const SHOW_SETTINGS_REQUEST: &str = "show-settings";

/// Migration 1's tables, in its order.
pub const TABLES: [&str; 5] = [
    "settings",
    "install",
    "terrariums",
    "observations",
    "sim_cache",
];
/// The triggers that keep observations append-only.
pub const TRIGGERS: [&str; 2] = ["observations_no_update", "observations_no_delete"];

/// The `install` table's keys.
pub const INSTALL_LAST_SIM_VERSION: &str = "last_sim_version";
pub const INSTALL_FIRST_RUN_DONE: &str = "first_run_done";
pub const INSTALL_SHOWN_EVENTS: &str = "shown_events";
/// The data directory's folder for the opt-in crash log.
pub const LOGS_DIR: &str = "logs";
/// The `settings` keys: the connected terrarium's row id; the strip's monitor, edge and size
/// step; the two hotkeys; when day and night begin; the dim timeout; opening at login; each
/// adapter's switch and report path, and the Knip run's opt-in; the CI repository; and the
/// crash log's opt-in.
pub const SETTING_REPO_ACTIVE: &str = "repo.active";
pub const SETTING_STRIP_MONITOR: &str = "strip.monitor";
pub const SETTING_STRIP_EDGE: &str = "strip.edge";
pub const SETTING_STRIP_SIZE_STEP: &str = "strip.size_step";
pub const SETTING_HOTKEYS_RAISE: &str = "hotkeys.raise";
pub const SETTING_HOTKEYS_TIMELAPSE: &str = "hotkeys.timelapse";
pub const SETTING_DAYNIGHT_DAY_START: &str = "daynight.day_start";
pub const SETTING_DAYNIGHT_NIGHT_START: &str = "daynight.night_start";
pub const SETTING_DIM_AFTER_MINUTES: &str = "dim.after_minutes";
pub const SETTING_LOGIN_OPEN_AT_LOGIN: &str = "login.open_at_login";
pub const SETTING_COVERAGE_ENABLED: &str = "adapters.coverage.enabled";
pub const SETTING_DEAD_CODE_ENABLED: &str = "adapters.dead_code.enabled";
pub const SETTING_WEATHER_ENABLED: &str = "adapters.weather.enabled";
pub const SETTING_COMPLEXITY_ENABLED: &str = "adapters.complexity.enabled";
pub const SETTING_TODO_DENSITY_ENABLED: &str = "adapters.todo_density.enabled";
pub const SETTING_KNIP_RUN_OPT_IN: &str = "adapters.knip.run_opt_in";
pub const SETTING_COVERAGE_REPORT_PATH: &str = "adapters.coverage.report_path";
pub const SETTING_DEAD_CODE_REPORT_PATH: &str = "adapters.dead_code.report_path";
pub const SETTING_WEATHER_REPORT_PATH: &str = "adapters.weather.report_path";
pub const SETTING_CI_GITHUB_REPO: &str = "ci.github.repo";
pub const SETTING_CRASHLOG_OPT_IN: &str = "crashlog.opt_in";
/// Present only inside the guarded deletion's transaction; while it is, observations may be
/// deleted (the `observations_no_delete` trigger reads it).
pub(crate) const INSTALL_DELETING_TERRARIUM: &str = "deleting_terrarium";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_pinned() {
        // Data, credentials and installs live under these names. Changing one strands them.
        assert_eq!(BUNDLE_ID, "io.github.faircat-gd.repo-terrarium");
        assert_eq!(BINARY, "repo-terrarium");
        assert_eq!(CRATE_PREFIX, "terrarium-");
        assert_eq!(DATABASE_FILE, "terrarium.sqlite3");
        assert_eq!(LOCAL_TESTS_DIR, "local-tests");
        assert_eq!(KEYCHAIN_SERVICE, "io.github.faircat-gd.repo-terrarium");
        assert_eq!(KEYCHAIN_ACCOUNT_GITHUB, "github-token");
        assert_eq!(DATA_DIR_ENV, "TERRARIUM_DATA_DIR");
        assert_eq!(INSTANCE_LOCK_FILE, "instance.lock");
        assert_eq!(REQUESTS_DIR, "requests");
        assert_eq!(SHOW_SETTINGS_REQUEST, "show-settings");
        assert_eq!(
            TABLES,
            [
                "settings",
                "install",
                "terrariums",
                "observations",
                "sim_cache"
            ]
        );
        assert_eq!(
            TRIGGERS,
            ["observations_no_update", "observations_no_delete"]
        );
        assert_eq!(INSTALL_LAST_SIM_VERSION, "last_sim_version");
        assert_eq!(INSTALL_FIRST_RUN_DONE, "first_run_done");
        assert_eq!(INSTALL_SHOWN_EVENTS, "shown_events");
        assert_eq!(INSTALL_DELETING_TERRARIUM, "deleting_terrarium");
        assert_eq!(LOGS_DIR, "logs");
        assert_eq!(SETTING_REPO_ACTIVE, "repo.active");
        assert_eq!(SETTING_STRIP_MONITOR, "strip.monitor");
        assert_eq!(SETTING_STRIP_EDGE, "strip.edge");
        assert_eq!(SETTING_STRIP_SIZE_STEP, "strip.size_step");
        assert_eq!(SETTING_HOTKEYS_RAISE, "hotkeys.raise");
        assert_eq!(SETTING_HOTKEYS_TIMELAPSE, "hotkeys.timelapse");
        assert_eq!(SETTING_DAYNIGHT_DAY_START, "daynight.day_start");
        assert_eq!(SETTING_DAYNIGHT_NIGHT_START, "daynight.night_start");
        assert_eq!(SETTING_DIM_AFTER_MINUTES, "dim.after_minutes");
        assert_eq!(SETTING_LOGIN_OPEN_AT_LOGIN, "login.open_at_login");
        assert_eq!(SETTING_COVERAGE_ENABLED, "adapters.coverage.enabled");
        assert_eq!(SETTING_DEAD_CODE_ENABLED, "adapters.dead_code.enabled");
        assert_eq!(SETTING_WEATHER_ENABLED, "adapters.weather.enabled");
        assert_eq!(SETTING_COMPLEXITY_ENABLED, "adapters.complexity.enabled");
        assert_eq!(
            SETTING_TODO_DENSITY_ENABLED,
            "adapters.todo_density.enabled"
        );
        assert_eq!(SETTING_KNIP_RUN_OPT_IN, "adapters.knip.run_opt_in");
        assert_eq!(SETTING_CI_GITHUB_REPO, "ci.github.repo");
        assert_eq!(SETTING_CRASHLOG_OPT_IN, "crashlog.opt_in");
        assert_eq!(
            SETTING_COVERAGE_REPORT_PATH,
            "adapters.coverage.report_path"
        );
        assert_eq!(
            SETTING_DEAD_CODE_REPORT_PATH,
            "adapters.dead_code.report_path"
        );
        assert_eq!(SETTING_WEATHER_REPORT_PATH, "adapters.weather.report_path");
        assert!(env!("CARGO_PKG_NAME").starts_with(CRATE_PREFIX));
    }
}
