//! Helpers the task tests share.

use std::path::PathBuf;

/// A directory the test owns under the system temp dir, emptied when made and removed when
/// dropped, so a run that was killed leaves nothing a later run trips on.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "terrarium-xtask-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the temp dir is writable");
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
