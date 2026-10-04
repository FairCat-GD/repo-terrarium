//! What can go wrong in the store. None of these texts is user-facing copy (I-8): the app words
//! what a person sees.

use std::fmt;

use crate::terrariums::TerrariumId;

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    Migration(rusqlite_migration::Error),
    /// A setting's value, or an export line, did not encode or decode as JSON.
    Json(serde_json::Error),
    /// `journal_mode=WAL` did not take; SQLite reported this mode instead.
    NotWal(String),
    /// An observation the store refuses to write.
    Invalid(Invalid),
    NoSuchTerrarium(TerrariumId),
    /// Another terrarium already has this seed; relink it instead.
    SeedTaken,
    /// An observation with the same (`kind`, `source`, `head_commit`, `recorded_at`) exists.
    AlreadyRecorded,
    /// A stored value that does not decode, naming its column.
    Corrupt(&'static str),
}

/// Why an observation is refused before it reaches the database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invalid {
    /// A weather observation names its source.
    WeatherWithoutSource,
    /// Only weather has a source; every other kind stores `''`.
    SourceWithoutWeather,
    /// A source whose column text does not read back as itself, such as `ci:` with no branch.
    BadSource,
    /// A Q16 column outside [0, 1].
    OutOfRange(&'static str),
    /// More contributors than the cap.
    TooManyContributors(usize),
    RawNotJson,
    RawTooLarge(usize),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(err) => write!(f, "data directory: {err}"),
            StoreError::Sqlite(err) => write!(f, "database: {err}"),
            StoreError::Migration(err) => write!(f, "migration: {err}"),
            StoreError::Json(err) => write!(f, "JSON: {err}"),
            StoreError::NotWal(mode) => write!(f, "journal mode is '{mode}', not WAL"),
            StoreError::Invalid(why) => write!(f, "observation refused: {why}"),
            StoreError::NoSuchTerrarium(id) => write!(f, "no terrarium {}", id.get()),
            StoreError::SeedTaken => f.write_str("a terrarium with this seed exists"),
            StoreError::AlreadyRecorded => f.write_str("this observation is already recorded"),
            StoreError::Corrupt(column) => write!(f, "a stored {column} does not decode"),
        }
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Invalid::WeatherWithoutSource => f.write_str("weather without a source"),
            Invalid::SourceWithoutWeather => f.write_str("a source on a kind other than weather"),
            Invalid::BadSource => f.write_str("a source that does not round-trip"),
            Invalid::OutOfRange(column) => write!(f, "{column} outside [0, 1]"),
            Invalid::TooManyContributors(n) => write!(f, "{n} contributors"),
            Invalid::RawNotJson => f.write_str("raw is not JSON"),
            Invalid::RawTooLarge(bytes) => write!(f, "raw is {bytes} bytes"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Io(err) => Some(err),
            StoreError::Sqlite(err) => Some(err),
            StoreError::Migration(err) => Some(err),
            StoreError::Json(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(err: std::io::Error) -> StoreError {
        StoreError::Io(err)
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> StoreError {
        StoreError::Sqlite(err)
    }
}

impl From<rusqlite_migration::Error> for StoreError {
    fn from(err: rusqlite_migration::Error) -> StoreError {
        StoreError::Migration(err)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(err: serde_json::Error) -> StoreError {
        StoreError::Json(err)
    }
}

impl From<Invalid> for StoreError {
    fn from(why: Invalid) -> StoreError {
        StoreError::Invalid(why)
    }
}

/// Whether `err` is a UNIQUE constraint failing, as opposed to a foreign key or a trigger.
pub(crate) fn is_unique_violation(err: &rusqlite::Error) -> bool {
    err.sqlite_error()
        .is_some_and(|e| e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE)
}
