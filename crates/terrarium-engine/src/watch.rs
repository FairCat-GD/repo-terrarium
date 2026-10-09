//! What the live pipeline watches, and how a file event becomes a refresh.
//!
//! An event only says that something may have changed; the refresh then reads what did. So the
//! watcher watches directories rather than files, which survives a file replaced by a rename
//! (`packed-refs`, a report a tool writes atomically) and a file that does not exist yet (the
//! first `coverage/lcov.info`, a new Surefire `TEST-*.xml`). `FSEvents` reports what happened
//! under a directory, coalesced and sometimes as the directory alone, so a directory event
//! counts, and so does a rescan.
//!
//! - **Git:** the git directory and the common directory, for `HEAD` and `packed-refs`; `refs/`
//!   and `logs/` under the common directory, every level. A linked worktree keeps its `HEAD`
//!   apart from the refs it shares.
//! - **Reports:** for every path a coverage or test report may be found at, the deepest
//!   directory before its first `*`, every level when a `*` names a directory; while that
//!   directory does not exist, its nearest existing parent inside the repository, one level,
//!   so its creation is seen.
//! - **Recorded runs:** the data directory's `local-tests/`, for this repository's file.
//!
//! Every directory is registered in one batch, so `FSEvents` starts one stream for all of them:
//! each start costs a fixed delay, up to more than half a second, and watching directory by
//! directory restarts the stream each time. On macOS every root is canonical: `FSEvents` names
//! `/private/var/…` for a `/var/…` path.
//!
//! Each event that touched something is passed on at once. The worker's schedule is the
//! debounce: it refreshes once events have stopped for 500 ms.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use terrarium_adapters::discovery;

/// The files of `HEAD` and `packed-refs`, and the locks git writes them through.
const GIT_FILES: [&str; 4] = ["HEAD", "HEAD.lock", "packed-refs", "packed-refs.lock"];

/// What an event touched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Touched {
    /// A ref, `HEAD`, or the git directory itself.
    pub git: bool,
    /// A report, a directory a report would be written into, or a recorded run.
    pub reports: bool,
}

impl Touched {
    const BOTH: Touched = Touched {
        git: true,
        reports: true,
    };

    fn any(self) -> bool {
        self.git || self.reports
    }

    #[must_use]
    fn or(self, other: Touched) -> Touched {
        Touched {
            git: self.git || other.git,
            reports: self.reports || other.reports,
        }
    }
}

/// Where a repository's watch roots are, before any is checked or canonicalized.
#[derive(Clone, Debug)]
pub struct Places<'a> {
    pub git_dir: &'a Path,
    pub common_dir: &'a Path,
    /// The worktree, or `None` for a bare repository (which has no reports).
    pub root: Option<&'a Path>,
    /// `discovery::report_patterns`.
    pub patterns: &'a [String],
    /// The data directory's `local-tests/`.
    pub status_dir: &'a Path,
    /// The seed's hex, which names this repository's status file.
    pub status_name: &'a str,
}

/// The directories to watch, and what an event under each means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchSet {
    /// Each directory, and whether every level under it is watched.
    pub dirs: BTreeMap<PathBuf, bool>,
    git_dirs: Vec<PathBuf>,
    git_trees: Vec<PathBuf>,
    root: Option<PathBuf>,
    patterns: Vec<String>,
    status_dir: PathBuf,
    status_name: String,
}

impl WatchSet {
    /// The set for `places` as the file system is now.
    pub fn new(places: &Places<'_>) -> WatchSet {
        let git_dir = canonical(places.git_dir);
        let common_dir = canonical(places.common_dir);
        let mut dirs = BTreeMap::new();
        let mut git_dirs = vec![git_dir.clone()];
        if common_dir != git_dir {
            git_dirs.push(common_dir.clone());
        }
        for dir in &git_dirs {
            watch(&mut dirs, dir.clone(), false);
        }
        let git_trees: Vec<PathBuf> = ["refs", "logs"]
            .iter()
            .map(|name| common_dir.join(name))
            .collect();
        for tree in &git_trees {
            if tree.is_dir() {
                watch(&mut dirs, tree.clone(), true);
            }
        }
        let root = places.root.map(canonical);
        if let Some(root) = &root {
            for pattern in places.patterns {
                let (fixed, recursive) = fixed_dir(pattern);
                let wanted = root.join(&fixed);
                if wanted.is_dir() {
                    watch(&mut dirs, wanted, recursive);
                } else if let Some(existing) = wanted
                    .ancestors()
                    .skip(1)
                    .find(|dir| dir.starts_with(root) && dir.is_dir())
                {
                    watch(&mut dirs, existing.to_path_buf(), false);
                }
            }
        }
        let status_dir = canonical(places.status_dir);
        if status_dir.is_dir() {
            watch(&mut dirs, status_dir.clone(), false);
        }
        WatchSet {
            dirs,
            git_dirs,
            git_trees,
            root,
            patterns: places.patterns.to_vec(),
            status_dir,
            status_name: places.status_name.to_owned(),
        }
    }

    /// What an event at `path` touched.
    pub fn classify(&self, path: &Path) -> Touched {
        let git = self.git_trees.iter().any(|tree| path.starts_with(tree))
            || self.git_dirs.iter().any(|dir| {
                path == dir
                    || (path.parent() == Some(dir.as_path())
                        && path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| GIT_FILES.contains(&name)))
            });
        let status = path.parent() == Some(self.status_dir.as_path())
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(self.status_name.as_str()));
        let report = self.root.as_ref().is_some_and(|root| {
            relative(root, path).is_some_and(|rel| {
                rel.is_empty()
                    || self.patterns.iter().any(|pattern| {
                        discovery::path_matches(pattern, &rel) || discovery::leads_to(pattern, &rel)
                    })
            })
        });
        Touched {
            git,
            reports: status || report,
        }
    }
}

/// Watches `dir`, every level under it when any of its reasons asks for that.
fn watch(dirs: &mut BTreeMap<PathBuf, bool>, dir: PathBuf, recursive: bool) {
    let entry = dirs.entry(dir).or_insert(recursive);
    *entry |= recursive;
}

/// The deepest directory of `pattern` before its first `*`, and whether a `*` names a directory
/// (so every level under it is watched).
fn fixed_dir(pattern: &str) -> (PathBuf, bool) {
    let parts: Vec<&str> = pattern.split('/').collect();
    let directories = &parts[..parts.len().saturating_sub(1)];
    let fixed: PathBuf = directories
        .iter()
        .take_while(|part| !part.contains('*'))
        .collect();
    let recursive = directories.iter().any(|part| part.contains('*'));
    (fixed, recursive)
}

/// `path` under `root`, as `/`-separated text; empty for the root itself.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = rest
        .components()
        .map(|part| match part {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    Some(parts?.join("/"))
}

/// `path` as `FSEvents` names it on macOS; as given elsewhere, where canonical paths are verbatim
/// (`\\?\C:\…`) and events are named as watched.
fn canonical(path: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

/// A running watch over a [`WatchSet`]: each event that touched something is sent on.
/// Dropping it stops watching.
pub struct Watcher {
    _watcher: RecommendedWatcher,
}

impl Watcher {
    /// Watches `set`, sending what each event touched through `send`. A directory that cannot
    /// be watched is left out: the hourly catch-up still reads what it would have shown.
    pub fn start(
        set: &WatchSet,
        send: impl Fn(Touched) + Send + 'static,
    ) -> Result<Watcher, notify::Error> {
        let classify = set.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                let touched = match result {
                    Ok(event) if event.need_rescan() => Touched::BOTH,
                    Ok(event) => event
                        .paths
                        .iter()
                        .fold(Touched::default(), |touched, path| {
                            touched.or(classify.classify(path))
                        }),
                    // A watch that failed may have missed anything.
                    Err(_) => Touched::BOTH,
                };
                if touched.any() {
                    send(touched);
                }
            })?;
        let mut paths = watcher.paths_mut();
        for (dir, &recursive) in &set.dirs {
            let mode = if recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            // A directory removed since the set was made is simply not watched.
            let _ = paths.add(dir, mode);
        }
        paths.commit()?;
        Ok(Watcher { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    struct Layout {
        _dir: tempfile::TempDir,
        root: PathBuf,
        git: PathBuf,
        status: PathBuf,
    }

    fn layout(dirs: &[&str]) -> Layout {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let git = root.join(".git");
        fs::create_dir_all(git.join("refs/heads")).unwrap();
        let status = dir.path().join("data/local-tests");
        fs::create_dir_all(&status).unwrap();
        for d in dirs {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        Layout {
            root: canonical(&root),
            git: canonical(&git),
            status: canonical(&status),
            _dir: dir,
        }
    }

    fn set(layout: &Layout) -> WatchSet {
        let patterns = discovery::report_patterns(&discovery::Overrides::default());
        WatchSet::new(&Places {
            git_dir: &layout.git,
            common_dir: &layout.git,
            root: Some(&layout.root),
            patterns: &patterns,
            status_dir: &layout.status,
            status_name: "abc123",
        })
    }

    #[test]
    fn fixed_directories_stop_at_the_first_wildcard() {
        assert_eq!(fixed_dir("lcov.info"), (PathBuf::new(), false));
        assert_eq!(
            fixed_dir("coverage/lcov.info"),
            (PathBuf::from("coverage"), false)
        );
        assert_eq!(
            fixed_dir("target/surefire-reports/TEST-*.xml"),
            (PathBuf::from("target/surefire-reports"), false)
        );
        assert_eq!(
            fixed_dir("build/test-results/*/TEST-*.xml"),
            (PathBuf::from("build/test-results"), true)
        );
    }

    #[test]
    fn existing_report_directories_are_watched_and_missing_ones_through_a_parent() {
        let l = layout(&["coverage", "target/nextest", "build"]);
        let set = set(&l);
        assert_eq!(set.dirs.get(&l.git), Some(&false));
        assert_eq!(set.dirs.get(&l.git.join("refs")), Some(&true));
        assert!(
            !set.dirs.contains_key(&l.git.join("logs")),
            "no reflogs here"
        );
        assert_eq!(set.dirs.get(&l.root), Some(&false));
        assert_eq!(set.dirs.get(&l.root.join("coverage")), Some(&false));
        assert_eq!(set.dirs.get(&l.root.join("target/nextest")), Some(&true));
        // `target/surefire-reports` is missing: its parent is watched for it, one level.
        assert_eq!(set.dirs.get(&l.root.join("target")), Some(&false));
        assert_eq!(set.dirs.get(&l.root.join("build")), Some(&false));
        assert_eq!(set.dirs.get(&l.status), Some(&false));
        assert!(
            set.dirs
                .keys()
                .all(|dir| !dir.starts_with(l.root.join("src")))
        );
    }

    #[test]
    fn events_are_classified_by_what_they_touched() {
        let l = layout(&["coverage"]);
        let set = set(&l);
        let git = Touched {
            git: true,
            reports: false,
        };
        let reports = Touched {
            git: false,
            reports: true,
        };
        assert_eq!(set.classify(&l.git.join("HEAD")), git);
        assert_eq!(set.classify(&l.git.join("packed-refs.lock")), git);
        assert_eq!(set.classify(&l.git.join("refs/heads/main")), git);
        assert_eq!(set.classify(&l.git), git);
        assert_eq!(set.classify(&l.git.join("index")), Touched::default());
        assert_eq!(
            set.classify(&l.git.join("objects/ab/cdef")),
            Touched::default()
        );
        assert_eq!(set.classify(&l.root.join("coverage/lcov.info")), reports);
        assert_eq!(set.classify(&l.root.join("coverage")), reports);
        assert_eq!(set.classify(&l.root.join("junit.xml")), reports);
        assert_eq!(
            set.classify(&l.root.join("target/surefire-reports/TEST-moss.xml")),
            reports
        );
        assert_eq!(set.classify(&l.root.join("README.md")), Touched::default());
        assert_eq!(set.classify(&l.root.join("src/lib.rs")), Touched::default());
        assert_eq!(set.classify(&l.status.join("abc123.json")), reports);
        assert_eq!(set.classify(&l.status.join(".abc123.json.7.tmp")), reports);
        assert_eq!(
            set.classify(&l.status.join("ffff.json")),
            Touched::default()
        );
        assert_eq!(
            set.classify(&l.status.parent().unwrap().join("terrarium.sqlite3-wal")),
            Touched::default()
        );
    }

    #[test]
    fn a_linked_worktree_watches_both_git_directories() {
        let l = layout(&[]);
        let common = l.git.clone();
        let own = l.git.join("worktrees/moss");
        fs::create_dir_all(&own).unwrap();
        let patterns = Vec::new();
        let set = WatchSet::new(&Places {
            git_dir: &own,
            common_dir: &common,
            root: None,
            patterns: &patterns,
            status_dir: &l.status,
            status_name: "abc123",
        });
        assert_eq!(set.dirs.get(&canonical(&own)), Some(&false));
        assert_eq!(set.dirs.get(&common), Some(&false));
        assert!(set.classify(&canonical(&own).join("HEAD")).git);
        assert!(set.classify(&common.join("packed-refs")).git);
        assert!(!set.classify(&common.join("HEAD.orig")).git);
    }
}
