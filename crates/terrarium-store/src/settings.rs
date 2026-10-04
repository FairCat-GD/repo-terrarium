//! Settings and install facts: two key-value tables. Settings hold JSON the person chose;
//! install facts are this install's own bookkeeping.

use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::identifiers::{INSTALL_FIRST_RUN_DONE, INSTALL_LAST_SIM_VERSION};
use crate::{Store, StoreError};

/// The `install` keys a caller may write. The deletion guard's key is not one of them, so only
/// [`Store::delete_terrarium`] can open it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallKey {
    LastSimVersion,
    FirstRunDone,
}

impl InstallKey {
    pub const fn as_key(self) -> &'static str {
        match self {
            InstallKey::LastSimVersion => INSTALL_LAST_SIM_VERSION,
            InstallKey::FirstRunDone => INSTALL_FIRST_RUN_DONE,
        }
    }
}

impl Store {
    /// The setting's value, decoded from its JSON.
    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, StoreError> {
        let text: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(text.map(|text| serde_json::from_str(&text)).transpose()?)
    }

    /// Stores the setting's value as JSON, replacing any earlier one.
    pub fn set_setting<T: Serialize + ?Sized>(
        &mut self,
        key: &str,
        value: &T,
    ) -> Result<(), StoreError> {
        let text = serde_json::to_string(value)?;
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, text],
        )?;
        Ok(())
    }

    pub fn install_value(&self, key: InstallKey) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM install WHERE key = ?1",
                [key.as_key()],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn set_install_value(&mut self, key: InstallKey, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO install (key, value) VALUES (?1, ?2) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key.as_key(), value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Fixture;

    #[test]
    fn settings_hold_typed_json_values() {
        let mut fx = Fixture::new();
        assert_eq!(fx.store.setting::<u32>("dim.after_minutes").unwrap(), None);
        fx.store.set_setting("dim.after_minutes", &30_u32).unwrap();
        fx.store.set_setting("dim.after_minutes", &45_u32).unwrap();
        fx.store.set_setting("login.open_at_login", &true).unwrap();
        fx.store.set_setting("strip.edge", "bottom").unwrap();
        assert_eq!(
            fx.store.setting::<u32>("dim.after_minutes").unwrap(),
            Some(45)
        );
        assert_eq!(
            fx.store.setting::<bool>("login.open_at_login").unwrap(),
            Some(true)
        );
        assert_eq!(
            fx.store.setting::<String>("strip.edge").unwrap().as_deref(),
            Some("bottom")
        );
        let stored: String = fx
            .store
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'strip.edge'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "\"bottom\"", "values are JSON");
        assert!(matches!(
            fx.store.setting::<u32>("strip.edge"),
            Err(StoreError::Json(_))
        ));
    }

    #[test]
    fn install_values_are_kept_under_their_pinned_keys() {
        let mut fx = Fixture::new();
        assert_eq!(
            fx.store.install_value(InstallKey::FirstRunDone).unwrap(),
            None
        );
        fx.store
            .set_install_value(InstallKey::LastSimVersion, "1")
            .unwrap();
        fx.store
            .set_install_value(InstallKey::LastSimVersion, "2")
            .unwrap();
        fx.store
            .set_install_value(InstallKey::FirstRunDone, "true")
            .unwrap();
        assert_eq!(
            fx.store
                .install_value(InstallKey::LastSimVersion)
                .unwrap()
                .as_deref(),
            Some("2")
        );
        let keys: Vec<String> = fx
            .store
            .conn
            .prepare("SELECT key FROM install ORDER BY key")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(keys, ["first_run_done", "last_sim_version"]);
    }
}
