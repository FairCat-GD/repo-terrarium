//! One app at a time. The running app holds a lock on a file in the data directory. A second
//! launch finds it held, leaves a request in the data directory's requests folder, and ends;
//! the running app watches that folder, and shows its settings when a request arrives. The OS
//! lets go of the lock when the process ends, however it ends.
//!
//! The installed app on macOS never reaches this: opening it again sends the running app a
//! reopen event instead (`terrarium_platform::macos::on_reopen`). A build run from the command
//! line, and every launch on Windows, does.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use terrarium_store::DataDir;

/// Whether this launch is the one app.
pub enum Claim {
    First(Instance),
    /// Another app holds the lock.
    Running,
}

/// The claim of the one app running. Dropping it lets another start.
pub struct Instance {
    _lock: File,
    claimed: SystemTime,
    requests: PathBuf,
    request: PathBuf,
}

/// The watch on the requests folder. Dropping it stops the watch.
pub struct RequestWatch {
    _watcher: RecommendedWatcher,
}

impl Instance {
    /// Takes the lock in `data`, creating the folder and the file as needed. The file is never
    /// truncated: on Windows the lock covers the file, and a truncating open would fail
    /// against it rather than find it held.
    pub fn claim(data: &DataDir) -> io::Result<Claim> {
        fs::create_dir_all(data.path())?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(data.instance_lock())?;
        match lock.try_lock() {
            Ok(()) => Ok(Claim::First(Instance {
                _lock: lock,
                claimed: SystemTime::now(),
                requests: data.requests_dir(),
                request: data.show_settings_request(),
            })),
            Err(TryLockError::WouldBlock) => Ok(Claim::Running),
            Err(TryLockError::Error(err)) => Err(err),
        }
    }

    /// Calls `on_request` for each request a later launch makes, from the watcher's thread.
    /// Also says whether one was already waiting once the watch began: made between the claim
    /// and the watch, it is answered here, by the caller. A request older than the claim was
    /// left by a launch that found an earlier app running, and is only cleared.
    pub fn watch(
        &self,
        mut on_request: impl FnMut() + Send + 'static,
    ) -> notify::Result<(RequestWatch, bool)> {
        fs::create_dir_all(&self.requests)?;
        let request = self.request.clone();
        let claimed = self.claimed;
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<_>| {
            if event.is_ok() && take(&request, claimed) {
                on_request();
            }
        })?;
        watcher.watch(&self.requests, RecursiveMode::NonRecursive)?;
        let waiting = take(&self.request, self.claimed);
        Ok((RequestWatch { _watcher: watcher }, waiting))
    }
}

/// Removes the request at `path`: true when there was one, made since `claimed`, and this call
/// removed it, so exactly one caller answers it.
fn take(path: &Path, claimed: SystemTime) -> bool {
    let Ok(made) = fs::metadata(path).and_then(|meta| meta.modified()) else {
        return false;
    };
    fs::remove_file(path).is_ok() && made >= claimed
}

/// Asks the app running on `data` to show its settings.
pub fn ask_to_show(data: &DataDir) -> io::Result<()> {
    fs::create_dir_all(data.requests_dir())?;
    fs::write(data.show_settings_request(), b"")
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    fn data() -> (tempfile::TempDir, DataDir) {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::at(dir.path().join("data"));
        (dir, data)
    }

    #[test]
    fn a_second_launch_finds_the_first_and_the_lock_goes_with_it() {
        let (_dir, data) = data();
        let Claim::First(first) = Instance::claim(&data).unwrap() else {
            panic!("the first claim");
        };
        assert!(matches!(Instance::claim(&data).unwrap(), Claim::Running));
        drop(first);
        assert!(matches!(Instance::claim(&data).unwrap(), Claim::First(_)));
    }

    #[test]
    fn a_request_reaches_the_running_app_and_one_left_from_before_it_does_not() {
        let (_dir, data) = data();
        // Left by a launch that found an earlier app running.
        ask_to_show(&data).unwrap();
        let old = SystemTime::now() - Duration::from_secs(60);
        File::options()
            .write(true)
            .open(data.show_settings_request())
            .unwrap()
            .set_modified(old)
            .unwrap();
        let Claim::First(first) = Instance::claim(&data).unwrap() else {
            panic!("the first claim");
        };
        let (heard, hear) = mpsc::channel();
        let (watch, waiting) = first.watch(move || heard.send(()).unwrap()).unwrap();
        assert!(!waiting, "a stale request is no request");
        assert!(!data.show_settings_request().exists(), "and is cleared");
        ask_to_show(&data).unwrap();
        hear.recv_timeout(Duration::from_secs(10))
            .expect("the request arrives");
        assert!(!data.show_settings_request().exists(), "answered once");
        drop(watch);
    }

    #[test]
    fn a_request_made_before_the_watch_began_is_answered_by_the_caller() {
        let (_dir, data) = data();
        let Claim::First(first) = Instance::claim(&data).unwrap() else {
            panic!("the first claim");
        };
        ask_to_show(&data).unwrap();
        let (_watch, waiting) = first.watch(|| {}).unwrap();
        assert!(waiting);
        assert!(!data.show_settings_request().exists());
    }
}
