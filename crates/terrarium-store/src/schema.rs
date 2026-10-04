//! The schema: the migrations, the connection's pragmas, and opening the store.

use rusqlite::{Connection, OpenFlags};
use rusqlite_migration::{M, Migrations};

use crate::{DataDir, Store, StoreError};

/// Every migration, in order; the schema version is their count, kept in `user_version`.
/// Migrations are forward-only and an applied one is never edited: a change is a new file
/// appended here, and the test below pins each one's hash.
const SQL: [&str; 1] = [include_str!("../migrations/0001.sql")];

fn migrations() -> Migrations<'static> {
    Migrations::new(SQL.iter().map(|sql| M::up(sql)).collect())
}

impl Store {
    /// Opens the database in `dir`, creating both if needed, and brings its schema up to date.
    /// A database written by a newer version, with migrations this one does not know, is
    /// refused rather than opened.
    pub fn open(dir: &DataDir) -> Result<Store, StoreError> {
        std::fs::create_dir_all(dir.path())?;
        // No URI flag: the path is a path, never a `file:` URI with options of its own.
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut conn = Connection::open_with_flags(dir.database(), flags)?;
        configure(&conn)?;
        migrations().to_latest(&mut conn)?;
        Ok(Store { conn })
    }
}

/// The pragmas, set on every connection. `recursive_triggers` is what makes `INSERT OR REPLACE`
/// fire the delete trigger when it would replace an observation.
fn configure(conn: &Connection) -> Result<(), StoreError> {
    conn.pragma_update(None, "busy_timeout", 5000)?;
    let mode: String =
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::NotWal(mode));
    }
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "trusted_schema", "OFF")?;
    conn.pragma_update(None, "recursive_triggers", "ON")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::identifiers::{TABLES, TRIGGERS};
    use crate::test_support::Fixture;

    /// The SHA-256 of each applied migration, in order. Append a line for a new migration;
    /// never change one.
    const APPLIED: [&str; 1] = ["eaa9dfbd7cec335f70f085049f9e8e55cd200506ee9f880938eb8993692c20c8"];

    #[test]
    fn the_migrations_validate() {
        migrations()
            .validate()
            .expect("the migrations apply to an empty database");
    }

    #[test]
    fn applied_migrations_are_pinned() {
        assert_eq!(SQL.len(), APPLIED.len(), "a new migration pins its hash");
        for (sql, pinned) in SQL.iter().zip(APPLIED) {
            let hash = terrarium_truth::canon::to_hex(&Sha256::digest(sql.as_bytes()));
            assert_eq!(hash, pinned, "an applied migration was edited");
        }
    }

    fn names(conn: &Connection, kind: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_schema WHERE type = ?1 AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name",
            )
            .unwrap();
        stmt.query_map([kind], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn sorted<const N: usize>(names: [&str; N]) -> Vec<String> {
        let mut names: Vec<String> = names.iter().map(|&n| n.to_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn the_schema_has_the_pinned_tables_and_triggers_all_strict() {
        let fx = Fixture::new();
        let conn = &fx.store.conn;
        assert_eq!(names(conn, "table"), sorted(TABLES));
        assert_eq!(names(conn, "trigger"), sorted(TRIGGERS));
        for table in TABLES {
            let strict: i64 = conn
                .query_row(
                    "SELECT strict FROM pragma_table_list WHERE schema = 'main' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(strict, 1, "{table} is STRICT");
        }
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn every_pragma_is_set() {
        let fx = Fixture::new();
        let conn = &fx.store.conn;
        let text = |name: &str| -> String {
            conn.pragma_query_value(None, name, |row| row.get(0))
                .unwrap()
        };
        let number = |name: &str| -> i64 {
            conn.pragma_query_value(None, name, |row| row.get(0))
                .unwrap()
        };
        assert_eq!(text("journal_mode"), "wal");
        assert_eq!(number("synchronous"), 1, "NORMAL");
        assert_eq!(number("foreign_keys"), 1);
        assert_eq!(number("trusted_schema"), 0);
        assert_eq!(number("recursive_triggers"), 1);
        assert_eq!(number("busy_timeout"), 5000);
    }

    #[test]
    fn opening_creates_the_directory_and_reopening_keeps_the_data() {
        let fx = Fixture::new();
        let dir = DataDir::at(fx.dir.path().join("nested").join("data"));
        let mut store = Store::open(&dir).expect("opens");
        assert!(dir.database().is_file());
        store.set_setting("strip.edge", "top").unwrap();
        drop(store);
        let store = Store::open(&dir).expect("reopens");
        assert_eq!(
            store.setting::<String>("strip.edge").unwrap().as_deref(),
            Some("top")
        );
        let version: i64 = store
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn a_database_from_a_newer_version_is_refused() {
        let fx = Fixture::new();
        fx.store
            .conn
            .pragma_update(None, "user_version", 2)
            .unwrap();
        let dir = fx.data_dir();
        let reopened = Store::open(&dir);
        assert!(
            matches!(
                reopened,
                Err(StoreError::Migration(
                    rusqlite_migration::Error::MigrationDefinition(
                        rusqlite_migration::MigrationDefinitionError::DatabaseTooFarAhead
                    )
                ))
            ),
            "{:?}",
            reopened.err()
        );
    }
}
