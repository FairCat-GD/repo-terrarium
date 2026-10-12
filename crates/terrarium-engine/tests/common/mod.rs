//! What the engine's integration tests share.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Writes `text` at `path`, dated two seconds ago so a refresh reads it as still.
pub fn write_still(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(2))
        .unwrap();
}
