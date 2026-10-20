//! One app at a time. The running app holds a lock on a file in the data directory. A later
//! launch finds it held, leaves a request in the data directory's requests folder, and ends;
//! the running app watches that folder, and answers each request as it arrives: a second
//! launch's by showing its settings, `--quit`'s by quitting. The OS lets go of the lock when
//! the process ends, however it ends.
//!
//! The installed app on macOS never gets the second launch's request: opening it again sends
//! the running app a reopen event instead (`terrarium_platform::macos::on_reopen`). A build run
//! from the command line, and every launch on Windows, sends it.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use terrarium_store::DataDir;

/// How many times a launch looks at a lock it finds held, and how far apart: `--quit`'s look
/// holds it for far less.
const CLAIM_TRIES: u32 = 5;
const CLAIM_RETRY: Duration = Duration::from_millis(20);

/// Whether this launch is the one app.
pub enum Claim {
    First(Instance),
    /// Another app holds the lock.
    Running,
}

/// What a later launch asks the running app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// `--quit`: quit, as the menu's Quit does.
    Quit,
    /// A second launch: show the settings.
    ShowSettings,
}

/// The claim of the one app running. Dropping it lets another start.
pub struct Instance {
    _lock: File,
    requests: PathBuf,
    quit: PathBuf,
    show_settings: PathBuf,
}

/// The watch on the requests folder. Dropping it stops the watch.
pub struct RequestWatch {
    _watcher: RecommendedWatcher,
}

impl Instance {
    /// Takes the lock in `data`, creating the folder and the file as needed. The file is never
    /// truncated: on Windows the lock covers the file, and a truncating open would fail
    /// against it rather than find it held. A lock found held is looked at again a few times,
    /// since `--quit` holds it for a moment to see whether an app does. A request already
    /// waiting was left by a launch that found an earlier app running, and is cleared: every
    /// request from now on is this one's.
    pub fn claim(data: &DataDir) -> io::Result<Claim> {
        fs::create_dir_all(data.path())?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(data.instance_lock())?;
        let mut tries = 1;
        let taken = loop {
            match lock.try_lock() {
                Err(TryLockError::WouldBlock) if tries < CLAIM_TRIES => {
                    tries += 1;
                    std::thread::sleep(CLAIM_RETRY);
                }
                taken => break taken,
            }
        };
        match taken {
            Ok(()) => {
                let quit = data.quit_request();
                let show_settings = data.show_settings_request();
                let _ = fs::remove_file(&quit);
                let _ = fs::remove_file(&show_settings);
                Ok(Claim::First(Instance {
                    _lock: lock,
                    requests: data.requests_dir(),
                    quit,
                    show_settings,
                }))
            }
            Err(TryLockError::WouldBlock) => Ok(Claim::Running),
            Err(TryLockError::Error(err)) => Err(err),
        }
    }

    /// Calls `on_request` for each request a later launch makes, from the watcher's thread; a
    /// quit is answered before a settings request found with it. Also says which request was
    /// already waiting once the watch began: made between the claim and the watch, it is
    /// answered here, by the caller.
    pub fn watch(
        &self,
        mut on_request: impl FnMut(Request) + Send + 'static,
    ) -> notify::Result<(RequestWatch, Option<Request>)> {
        fs::create_dir_all(&self.requests)?;
        let waiting = Waiting {
            quit: self.quit.clone(),
            show_settings: self.show_settings.clone(),
        };
        let watching = waiting.clone();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<_>| {
            if event.is_ok()
                && let Some(request) = watching.take()
            {
                on_request(request);
            }
        })?;
        watcher.watch(&self.requests, RecursiveMode::NonRecursive)?;
        Ok((RequestWatch { _watcher: watcher }, waiting.take()))
    }
}

/// The requests' files, taken in the order they are answered.
#[derive(Clone)]
struct Waiting {
    quit: PathBuf,
    show_settings: PathBuf,
}

impl Waiting {
    /// The request waiting, removed so exactly one caller answers it: a quit first.
    fn take(&self) -> Option<Request> {
        if take(&self.quit) {
            Some(Request::Quit)
        } else if take(&self.show_settings) {
            Some(Request::ShowSettings)
        } else {
            None
        }
    }
}

/// Removes the request at `path`: true when there was one and this call removed it, so exactly
/// one caller answers it.
fn take(path: &Path) -> bool {
    fs::remove_file(path).is_ok()
}

/// Asks the app running on `data` to show its settings.
pub fn ask_to_show(data: &DataDir) -> io::Result<()> {
    fs::create_dir_all(data.requests_dir())?;
    fs::write(data.show_settings_request(), b"")
}

/// How `--quit` ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Quit {
    /// No app held the lock, so none was running.
    NotRunning,
    /// The app running quit.
    Ended,
    /// The app running had not quit by the time given; its request is withdrawn.
    StillRunning,
}

/// How often `--quit` looks again for the lock let go.
const QUIT_POLL: Duration = Duration::from_millis(100);

/// Asks the app running on `data` to quit, and waits up to `wait` for it to let go of its
/// lock. The lock's file is opened, never created: with no app ever run on `data`, nothing is
/// written there, so an uninstall that asks leaves no data folder behind.
pub fn ask_to_quit(data: &DataDir, wait: Duration) -> io::Result<Quit> {
    let lock = match OpenOptions::new()
        .read(true)
        .write(true)
        .open(data.instance_lock())
    {
        Ok(lock) => lock,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Quit::NotRunning),
        Err(err) => return Err(err),
    };
    let held = |lock: &File| match lock.try_lock() {
        Ok(()) => {
            let _ = lock.unlock();
            Ok(false)
        }
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(TryLockError::Error(err)) => Err(err),
    };
    if !held(&lock)? {
        return Ok(Quit::NotRunning);
    }
    fs::create_dir_all(data.requests_dir())?;
    fs::write(data.quit_request(), b"")?;
    let until = Instant::now() + wait;
    while Instant::now() < until {
        std::thread::sleep(QUIT_POLL);
        if !held(&lock)? {
            return Ok(Quit::Ended);
        }
    }
    // Left in place, it would end the app whenever its loop next looked.
    let _ = fs::remove_file(data.quit_request());
    Ok(Quit::StillRunning)
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
        // Left by a launch that found an earlier app running, dated whenever the clock said.
        ask_to_show(&data).unwrap();
        File::options()
            .write(true)
            .open(data.show_settings_request())
            .unwrap()
            .set_modified(std::time::SystemTime::now() + Duration::from_secs(3_600))
            .unwrap();
        let Claim::First(first) = Instance::claim(&data).unwrap() else {
            panic!("the first claim");
        };
        assert!(
            !data.show_settings_request().exists(),
            "cleared at the claim"
        );
        let (heard, hear) = mpsc::channel();
        let (watch, waiting) = first
            .watch(move |request| heard.send(request).unwrap())
            .unwrap();
        assert_eq!(waiting, None, "a stale request is no request");
        ask_to_show(&data).unwrap();
        assert_eq!(
            hear.recv_timeout(Duration::from_secs(10)),
            Ok(Request::ShowSettings),
            "the request arrives"
        );
        assert!(!data.show_settings_request().exists(), "answered once");
        drop(watch);
    }

    #[test]
    fn a_request_made_before_the_watch_began_is_answered_by_the_caller_whatever_its_date() {
        let (_dir, data) = data();
        let Claim::First(first) = Instance::claim(&data).unwrap() else {
            panic!("the first claim");
        };
        ask_to_show(&data).unwrap();
        // Made after the claim, though the clock was set back meanwhile.
        File::options()
            .write(true)
            .open(data.show_settings_request())
            .unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(3_600))
            .unwrap();
        let (_watch, waiting) = first.watch(|_| {}).unwrap();
        assert_eq!(waiting, Some(Request::ShowSettings));
        assert!(!data.show_settings_request().exists());
    }

    #[test]
    fn a_lock_held_for_a_moment_is_no_running_app() {
        let (_dir, data) = data();
        let Claim::First(first) = Instance::claim(&data).unwrap() else {
            panic!("the first claim");
        };
        // As `--quit`'s look does: held, then let go a moment later.
        let looker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            drop(first);
        });
        assert!(matches!(Instance::claim(&data).unwrap(), Claim::First(_)));
        looker.join().unwrap();
    }

    #[test]
    fn a_quit_with_nothing_running_writes_nothing() {
        let (_dir, data) = data();
        assert_eq!(
            ask_to_quit(&data, Duration::from_secs(5)).unwrap(),
            Quit::NotRunning
        );
        assert!(!data.path().exists(), "no data folder made");
        // An app ran once and ended: its lock's file is there, held by no one.
        drop(Instance::claim(&data).unwrap());
        assert_eq!(
            ask_to_quit(&data, Duration::from_secs(5)).unwrap(),
            Quit::NotRunning
        );
        assert!(!data.quit_request().exists());
    }

    #[test]
    fn a_quit_reaches_the_running_app_and_waits_for_it_to_let_go() {
        let (_dir, data) = data();
        let Claim::First(first) = Instance::claim(&data).unwrap() else {
            panic!("the first claim");
        };
        let (heard, hear) = mpsc::channel();
        let (watch, waiting) = first
            .watch(move |request| heard.send(request).unwrap())
            .unwrap();
        assert_eq!(waiting, None);
        // The app: it quits when asked, letting go of its lock.
        let app = std::thread::spawn(move || {
            let request = hear.recv_timeout(Duration::from_secs(10));
            drop(watch);
            drop(first);
            request
        });
        assert_eq!(
            ask_to_quit(&data, Duration::from_secs(20)).unwrap(),
            Quit::Ended
        );
        assert_eq!(app.join().unwrap(), Ok(Request::Quit));
        assert!(!data.quit_request().exists(), "answered once");
    }

    #[test]
    fn an_app_that_does_not_quit_keeps_running_and_the_request_is_withdrawn() {
        let (_dir, data) = data();
        let Claim::First(_first) = Instance::claim(&data).unwrap() else {
            panic!("the first claim");
        };
        assert_eq!(
            ask_to_quit(&data, Duration::from_millis(300)).unwrap(),
            Quit::StillRunning
        );
        assert!(!data.quit_request().exists());
        assert!(matches!(Instance::claim(&data).unwrap(), Claim::Running));
    }

    #[test]
    fn a_quit_left_from_before_is_cleared_and_one_waiting_wins_over_the_settings() {
        let (_dir, data) = data();
        fs::create_dir_all(data.requests_dir()).unwrap();
        fs::write(data.quit_request(), b"").unwrap();
        let Claim::First(first) = Instance::claim(&data).unwrap() else {
            panic!("the first claim");
        };
        assert!(!data.quit_request().exists(), "cleared at the claim");
        ask_to_show(&data).unwrap();
        fs::write(data.quit_request(), b"").unwrap();
        let (_watch, waiting) = first.watch(|_| {}).unwrap();
        assert_eq!(waiting, Some(Request::Quit));
    }
}
