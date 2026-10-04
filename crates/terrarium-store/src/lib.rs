#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod cache;
mod data_dir;
mod error;
pub mod identifiers;
mod jsonl;
mod observations;
mod schema;
mod settings;
mod terrariums;
#[cfg(test)]
mod test_support;

pub use cache::SimCache;
pub use data_dir::{DataDir, DataDirError};
pub use error::{Invalid, StoreError};
pub use jsonl::{
    Conflict, EXPORT_FORMAT, EXPORT_FORMAT_VERSION, ImportError, ImportReport, LineProblem,
    MAX_IMPORT_LINE_BYTES,
};
pub use observations::{
    Contributor, MAX_CONTRIBUTORS, MAX_RAW_BYTES, ObservationId, ObservationRecord,
    StoredObservation,
};
pub use settings::InstallKey;
pub use terrariums::{NewTerrarium, RepoLocation, Terrarium, TerrariumId};

/// The open database: one connection, owned by whoever does the writing.
pub struct Store {
    conn: rusqlite::Connection,
}
