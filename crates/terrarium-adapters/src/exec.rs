//! The one place a process is started (I-5). `cargo xtask verify`'s graph stage refuses
//! `std::process::Command` in any other crate source.
//!
//! - [`run_user_command`] runs the command a person typed after `repo-terrarium record-tests
//!   --`. It is theirs, named explicitly on their own command line, so it runs as their shell
//!   would run it: their environment, their working directory, their terminal.
//! - [`run_fed`] runs a [`Tool`] the person installed, found by [`find_tool`] where tools are
//!   installed and never inside the repository, feeding it on stdin: the time-lapse's MP4 goes
//!   through their ffmpeg this way. It inherits nothing: no environment, no terminal, no
//!   working directory but the one given.
//!
//! Each is executed directly, never through a shell, so its arguments reach it exactly as
//! given. Nothing found in a repository is ever run from here without the user's per-adapter
//! opt-in.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::Duration;

/// How a command ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finished {
    /// It exited with this code.
    Code(i32),
    /// A signal ended it (Unix), so it never reported a result.
    Signal(i32),
}

/// Why a command did not start.
#[derive(Debug)]
pub enum ExecError {
    /// No program by that name was found.
    NotFound(io::Error),
    /// The program exists but could not be started (permissions, format, …).
    CannotRun(io::Error),
}

/// Runs `program` with `args`, inheriting stdin, stdout, stderr, the environment and the
/// working directory, and waits for it to end.
pub fn run_user_command(program: &OsStr, args: &[OsString]) -> Result<Finished, ExecError> {
    let status = Command::new(resolve(program))
        .args(args)
        .status()
        .map_err(|err| match err.kind() {
            io::ErrorKind::NotFound => ExecError::NotFound(err),
            _ => ExecError::CannotRun(err),
        })?;
    Ok(finished(status))
}

/// The program as given. Unix searches `PATH` for it as a shell would.
#[cfg(not(windows))]
fn resolve(program: &OsStr) -> OsString {
    program.to_owned()
}

/// On Windows, `Command` looks only for `<name>.exe`, and `npm`, `npx`, `yarn` and `pnpm` are
/// `.cmd` scripts. A bare name is looked up on `PATH` with each `PATHEXT` extension, as
/// `cmd.exe` would; `Command` then runs a script found that way through `cmd.exe`, quoting its
/// arguments, or refuses arguments it cannot quote safely.
#[cfg(windows)]
fn resolve(program: &OsStr) -> OsString {
    let path = std::env::var_os("PATH");
    let pathext = std::env::var_os("PATHEXT");
    find_on_path(program, path.as_deref(), pathext.as_deref(), Path::is_file)
        .map_or_else(|| program.to_owned(), PathBuf::into_os_string)
}

/// The first `<dir>\<program><ext>` that exists, for each `PATH` directory in order and each
/// `PATHEXT` extension in order; nothing for a program named with a directory or an extension.
#[cfg(any(windows, test))]
fn find_on_path(
    program: &OsStr,
    path: Option<&OsStr>,
    pathext: Option<&OsStr>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let name = Path::new(program);
    if name.extension().is_some() || name.components().count() != 1 {
        return None;
    }
    let pathext = pathext
        .and_then(OsStr::to_str)
        .unwrap_or(".COM;.EXE;.BAT;.CMD");
    let extensions: Vec<&str> = pathext.split(';').filter(|ext| !ext.is_empty()).collect();
    std::env::split_paths(path?).find_map(|dir| {
        extensions.iter().find_map(|ext| {
            let mut file = program.to_owned();
            file.push(ext);
            let candidate = dir.join(file);
            exists(&candidate).then_some(candidate)
        })
    })
}

/// A program found where a person installs tools, outside the repository: the only kind
/// [`run_fed`] starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool(PathBuf);

impl Tool {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// ffmpeg's file name. On Windows only `ffmpeg.exe`: std runs a `.cmd` or `.bat` through
/// `cmd.exe`, a shell.
const FFMPEG_NAMES: &[&str] = if cfg!(windows) {
    &["ffmpeg.exe"]
} else {
    &["ffmpeg"]
};

/// Where macOS's package managers put tools (Homebrew on Apple silicon and on Intel, then
/// `MacPorts`). An app opened from Finder, the Dock or at login gets only the system's `PATH`,
/// which holds none of them.
const MACOS_TOOL_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"];

/// The person's ffmpeg: on `PATH`, then, on macOS, where its package managers put it; never
/// one inside `outside`, the repository.
pub fn find_ffmpeg(outside: Option<&Path>) -> Option<Tool> {
    let also: &[&str] = if cfg!(target_os = "macos") {
        MACOS_TOOL_DIRS
    } else {
        &[]
    };
    let also: Vec<&Path> = also.iter().map(Path::new).collect();
    let path = std::env::var_os("PATH");
    find_tool(FFMPEG_NAMES, path.as_deref(), &also, outside)
}

/// The first file named one of `names` that can run, in each absolute directory of `path` in
/// order, then in each of `also`. An empty or relative `PATH` entry names the working
/// directory, which could be anywhere, so it is skipped; a candidate that is, or resolves
/// through a link to, a file inside `outside` is refused (I-5).
pub fn find_tool(
    names: &[&str],
    path: Option<&OsStr>,
    also: &[&Path],
    outside: Option<&Path>,
) -> Option<Tool> {
    let outside = outside.and_then(|dir| dir.canonicalize().ok());
    let refused = |candidate: &Path| match candidate.canonicalize() {
        Ok(real) => outside.as_ref().is_some_and(|dir| real.starts_with(dir)),
        Err(_) => true,
    };
    path.into_iter()
        .flat_map(std::env::split_paths)
        .filter(|dir| dir.is_absolute())
        .chain(also.iter().map(|dir| dir.to_path_buf()))
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| runnable(candidate) && !refused(candidate))
        .map(Tool)
}

#[cfg(unix)]
fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(path: &Path) -> bool {
    path.is_file()
}

/// Why a fed run did not succeed.
#[derive(Debug)]
pub enum FedError {
    /// The tool could not be started.
    CannotRun(io::Error),
    /// It ended without success, with the end of what it wrote to stderr.
    Failed { ended: Finished, stderr: String },
    /// Its stdin or stderr could not be opened, so it was stopped before being fed.
    NoPipes,
    /// It ran past its time and was stopped.
    TimedOut,
    /// Feeding it, or writing beside it, failed on this side, though it ended well.
    Feed(io::Error),
}

impl From<io::Error> for FedError {
    fn from(err: io::Error) -> FedError {
        FedError::Feed(err)
    }
}

/// How much of a tool's stderr is kept: its end, where the reason it stopped is.
const STDERR_TAIL: usize = 4096;

/// How often a fed tool is looked at once its input is closed.
const WAIT_STEP: Duration = Duration::from_millis(10);

/// How long the end of a tool's stderr is waited for once it has ended. A process it started
/// can hold the pipe open past its end; the run does not wait on that.
const TAIL_GRACE: Duration = Duration::from_secs(2);

/// `CREATE_NO_WINDOW`: a console tool started from the app opens no console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Runs `tool` with `args` in `cwd`, writing its stdin through `feed`, and waits for it to end.
/// Its stdout is discarded and the end of its stderr kept for the error; its environment is
/// empty (Windows keeps `SystemRoot`, which its system libraries need). Past `limit` it is
/// stopped, even while `feed` is writing. It is always waited for, so none outlives the run;
/// a panic in `feed` stops it, then carries on.
pub fn run_fed(
    tool: &Tool,
    args: &[OsString],
    cwd: &Path,
    limit: Duration,
    feed: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> Result<(), FedError> {
    let mut command = Command::new(&tool.0);
    command
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(FedError::CannotRun)?;
    let (Some(mut stdin), Some(stderr)) = (child.stdin.take(), child.stderr.take()) else {
        end(&mut child);
        return Err(FedError::NoPipes);
    };
    let (said, tail) = mpsc::channel();
    let reader = thread::Builder::new()
        .name("terrarium-tool-stderr".into())
        .spawn(move || {
            let _ = said.send(keep_tail(stderr));
        });
    match reader {
        Ok(_) => {}
        Err(err) => {
            end(&mut child);
            return Err(FedError::CannotRun(err));
        }
    }
    let child = Arc::new(Mutex::new(child));
    let timed_out = Arc::new(AtomicBool::new(false));
    let (done, ran) = mpsc::channel::<()>();
    let watchdog = {
        let (child, timed_out) = (Arc::clone(&child), Arc::clone(&timed_out));
        thread::Builder::new()
            .name("terrarium-tool-watchdog".into())
            .spawn(move || {
                if ran.recv_timeout(limit) == Err(mpsc::RecvTimeoutError::Timeout) {
                    // Only a tool still running is stopped: one that ended as the time ran
                    // out ended on its own.
                    let mut child = lock(&child);
                    if matches!(child.try_wait(), Ok(None)) {
                        timed_out.store(true, Ordering::SeqCst);
                        let _ = child.kill();
                    }
                }
            })
    };
    let watchdog = match watchdog {
        Ok(watchdog) => watchdog,
        Err(err) => {
            end(&mut lock(&child));
            return Err(FedError::CannotRun(err));
        }
    };
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| feed(&mut stdin)));
    // Closed, so the tool sees its input end.
    drop(stdin);
    if !matches!(outcome, Ok(Ok(()))) {
        let _ = lock(&child).kill();
    }
    let status = loop {
        match lock(&child).try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(WAIT_STEP),
            Err(err) => break Err(err),
        }
    };
    drop(done);
    let _ = watchdog.join();
    let stderr = tail.recv_timeout(TAIL_GRACE).unwrap_or_default();
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(payload) => panic::resume_unwind(payload),
    };
    if timed_out.load(Ordering::SeqCst) {
        return Err(FedError::TimedOut);
    }
    let status = status.map_err(FedError::CannotRun)?;
    match (outcome, status.success()) {
        (Ok(()), true) => Ok(()),
        (Err(err), true) => Err(FedError::Feed(err)),
        (_, false) => Err(FedError::Failed {
            ended: finished(status),
            stderr,
        }),
    }
}

fn lock(child: &Mutex<Child>) -> std::sync::MutexGuard<'_, Child> {
    child.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Stops a child that will not be fed, and waits for it, so it leaves no zombie.
fn end(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Reads `stderr` to its end, keeping its last [`STDERR_TAIL`] bytes, as text.
fn keep_tail(mut stderr: impl Read) -> String {
    let mut tail = Vec::new();
    let mut buf = [0; 8192];
    loop {
        match stderr.read(&mut buf) {
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Ok(0) | Err(_) => break,
            Ok(n) => {
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > STDERR_TAIL {
                    tail.drain(..tail.len() - STDERR_TAIL);
                }
            }
        }
    }
    String::from_utf8_lossy(&tail).trim().to_owned()
}

#[cfg(unix)]
fn finished(status: ExitStatus) -> Finished {
    use std::os::unix::process::ExitStatusExt as _;
    match (status.code(), status.signal()) {
        (Some(code), _) => Finished::Code(code),
        (None, Some(signal)) => Finished::Signal(signal),
        // A stopped or continued child is not an ended one; `status()` never returns either.
        (None, None) => Finished::Signal(0),
    }
}

#[cfg(not(unix))]
fn finished(status: ExitStatus) -> Finished {
    // Windows always has an exit code.
    Finished::Code(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_program_is_not_found() {
        let missing = OsStr::new("repo-terrarium-test-no-such-program");
        assert!(matches!(
            run_user_command(missing, &[]),
            Err(ExecError::NotFound(_))
        ));
    }

    #[test]
    fn a_bare_name_is_found_with_the_path_extensions_in_order() {
        let dirs = std::env::join_paths([Path::new("/tools/a"), Path::new("/tools/b")]).unwrap();
        let present = |path: &Path| {
            [
                Path::new("/tools/b/npm.CMD"),
                Path::new("/tools/b/npm.EXE"),
                Path::new("/tools/a/node.EXE"),
            ]
            .contains(&path)
        };
        let find = |program: &str| {
            find_on_path(
                OsStr::new(program),
                Some(&dirs),
                Some(OsStr::new(".COM;.EXE;.BAT;.CMD")),
                present,
            )
        };
        assert_eq!(find("npm"), Some(PathBuf::from("/tools/b/npm.EXE")));
        assert_eq!(find("node"), Some(PathBuf::from("/tools/a/node.EXE")));
        assert_eq!(find("cargo"), None);
        assert_eq!(find("npm.cmd"), None, "an extension is used as given");
        assert_eq!(find("bin/npm"), None, "a directory is used as given");
        let only_cmd = |path: &Path| path == Path::new("/tools/b/npm.CMD");
        assert_eq!(
            find_on_path(OsStr::new("npm"), Some(&dirs), None, only_cmd),
            Some(PathBuf::from("/tools/b/npm.CMD")),
            "without PATHEXT, Windows' default list"
        );
        assert_eq!(find_on_path(OsStr::new("npm"), None, None, only_cmd), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_exit_code_comes_back() {
        // `sh` is the command here, typed by the test, as a person types theirs.
        let code = run_user_command(OsStr::new("sh"), &["-c".into(), "exit 3".into()]).unwrap();
        assert_eq!(code, Finished::Code(3));
        let killed =
            run_user_command(OsStr::new("sh"), &["-c".into(), "kill -TERM $$".into()]).unwrap();
        assert_eq!(killed, Finished::Signal(15));
    }

    /// A file at `path` that runs, or does not.
    #[cfg(unix)]
    fn tool_file(path: &Path, runs: bool) {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        let mode = if runs { 0o755 } else { 0o644 };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_is_found_on_path_then_in_the_folders_and_never_inside_the_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let [empty, first, second, folder, closed, repo] =
            ["empty", "first", "second", "folder", "closed", "repo/bin"].map(|d| root.join(d));
        std::fs::create_dir_all(&empty).unwrap();
        tool_file(&first.join("ffmpeg"), true);
        tool_file(&second.join("ffmpeg"), true);
        tool_file(&folder.join("ffmpeg"), true);
        tool_file(&closed.join("ffmpeg"), false);
        tool_file(&repo.join("ffmpeg"), true);
        let path = |dirs: &[&Path]| std::env::join_paths(dirs).unwrap();
        let find = |path: Option<&OsStr>, also: &[&Path], outside: Option<&Path>| {
            find_tool(&["ffmpeg"], path, also, outside).map(|tool| tool.path().to_path_buf())
        };
        // Absent: nothing on PATH and no folders, whatever this machine holds.
        assert_eq!(find(None, &[], None), None);
        assert_eq!(find(Some(&path(&[&empty])), &[], None), None);
        // In PATH's order, past a file that cannot run.
        assert_eq!(
            find(Some(&path(&[&empty, &closed, &second, &first])), &[], None),
            Some(second.join("ffmpeg"))
        );
        // PATH before the folders.
        assert_eq!(
            find(Some(&path(&[&first])), &[&folder], None),
            Some(first.join("ffmpeg"))
        );
        assert_eq!(
            find(Some(&path(&[&empty])), &[&folder], None),
            Some(folder.join("ffmpeg"))
        );
        // A relative entry names the working directory's descendants, so it is skipped, even
        // when it reaches a tool.
        let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
        let up: PathBuf = cwd.components().skip(1).map(|_| "..").collect();
        let relative = up.join(first.strip_prefix("/").unwrap());
        assert!(relative.join("ffmpeg").is_file(), "{relative:?}");
        assert_eq!(find(Some(&path(&[&relative])), &[], None), None);
        // Inside the repository, directly or through a link, it is refused.
        let repository = root.join("repo");
        let link = root.join("linked");
        std::os::unix::fs::symlink(&repo, &link).unwrap();
        for dir in [&repo, &link] {
            assert_eq!(
                find(Some(&path(&[dir])), &[], Some(&repository)),
                None,
                "{dir:?}"
            );
            assert_eq!(
                find(Some(&path(&[dir])), &[&folder], Some(&repository)),
                Some(folder.join("ffmpeg"))
            );
        }
        assert_eq!(
            find(Some(&path(&[&repo])), &[], None),
            Some(repo.join("ffmpeg")),
            "only the repository is refused"
        );
    }

    #[cfg(unix)]
    #[test]
    fn on_windows_only_an_exe_is_taken_never_a_script() {
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools");
        tool_file(&tools.join("ffmpeg.cmd"), true);
        tool_file(&tools.join("ffmpeg.bat"), true);
        let path = std::env::join_paths([&tools]).unwrap();
        let find = || find_tool(&["ffmpeg.exe"], Some(&path), &[], None);
        assert_eq!(find(), None);
        tool_file(&tools.join("ffmpeg.exe"), true);
        assert_eq!(find().map(|tool| tool.0), Some(tools.join("ffmpeg.exe")));
    }

    /// `/bin/sh -c <script>`, as a fed tool: the test's own command, run by its full path, since
    /// a fed tool has no `PATH`.
    #[cfg(unix)]
    fn sh(script: &str, cwd: &Path, limit: Duration, bytes: usize) -> Result<(), FedError> {
        let pattern: Vec<u8> = (0..=255).cycle().take(bytes).collect();
        run_fed(
            &Tool(PathBuf::from("/bin/sh")),
            &["-c".into(), script.into()],
            cwd,
            limit,
            |stdin| stdin.write_all(&pattern),
        )
    }

    #[cfg(unix)]
    #[test]
    fn what_is_fed_arrives_whole_in_its_folder_with_no_environment() {
        let dir = tempfile::tempdir().unwrap();
        sh(
            "/bin/cat > fed; /usr/bin/env > env",
            dir.path(),
            Duration::from_secs(30),
            1 << 20,
        )
        .unwrap();
        let fed = std::fs::read(dir.path().join("fed")).unwrap();
        assert_eq!(fed.len(), 1 << 20);
        assert!(fed.iter().zip((0..=255).cycle()).all(|(a, b)| *a == b));
        let env = std::fs::read_to_string(dir.path().join("env")).unwrap();
        for name in ["PATH=", "HOME=", "USER="] {
            assert!(!env.lines().any(|line| line.starts_with(name)), "{env}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_that_stops_reading_and_fails_says_how_it_ended() {
        let dir = tempfile::tempdir().unwrap();
        // 64 MB is far more than a pipe holds, so the feed meets the closed pipe.
        let ended = sh(
            "echo starting >&2; echo the encoder is missing >&2; exit 3",
            dir.path(),
            Duration::from_secs(30),
            64 << 20,
        );
        match ended {
            Err(FedError::Failed { ended, stderr }) => {
                assert_eq!(ended, Finished::Code(3));
                assert_eq!(stderr, "starting\nthe encoder is missing");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            run_fed(
                &Tool(dir.path().join("missing")),
                &[],
                dir.path(),
                Duration::from_secs(30),
                |_| Ok(()),
            ),
            Err(FedError::CannotRun(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_that_writes_much_to_stderr_before_reading_is_not_stuck() {
        let dir = tempfile::tempdir().unwrap();
        // A megabyte of stderr fills its pipe many times over before stdin is read.
        sh(
            "/bin/dd if=/dev/zero bs=65536 count=16 >&2 2>/dev/null; /bin/cat > /dev/null",
            dir.path(),
            Duration::from_secs(20),
            1 << 20,
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_that_ended_before_its_limit_is_never_called_timed_out() {
        let dir = tempfile::tempdir().unwrap();
        // The tool ends at once; its feed is still busy when the limit passes.
        let ran = run_fed(
            &Tool(PathBuf::from("/bin/sh")),
            &["-c".into(), "exit 0".into()],
            dir.path(),
            Duration::from_millis(200),
            |_| {
                thread::sleep(Duration::from_secs(1));
                Ok(())
            },
        );
        assert!(ran.is_ok(), "{ran:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_process_left_holding_stderr_does_not_hold_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let start = std::time::Instant::now();
        // The shell ends at once; the sleep it leaves keeps stderr open for 20 s.
        sh(
            "/bin/sleep 20 & exit 0",
            dir.path(),
            Duration::from_secs(60),
            0,
        )
        .unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            start.elapsed()
        );
    }

    /// A reader that is interrupted once, then gives what it holds.
    struct Interrupted<'a>(bool, &'a [u8]);

    impl Read for Interrupted<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if !self.0 {
                self.0 = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            self.1.read(buf)
        }
    }

    #[test]
    fn an_interrupted_read_of_stderr_reads_on() {
        assert_eq!(keep_tail(Interrupted(false, b"why\n")), "why");
        let long: Vec<u8> = b"x"
            .repeat(STDERR_TAIL)
            .into_iter()
            .chain(*b"end")
            .collect();
        let tail = keep_tail(&long[..]);
        assert_eq!(tail.len(), STDERR_TAIL);
        assert!(tail.ends_with("xend"));
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_that_never_reads_is_stopped_at_its_limit() {
        let dir = tempfile::tempdir().unwrap();
        let start = std::time::Instant::now();
        let ended = sh(
            "/bin/sleep 30",
            dir.path(),
            Duration::from_millis(500),
            16 << 20,
        );
        assert!(matches!(ended, Err(FedError::TimedOut)), "{ended:?}");
        assert!(start.elapsed() < Duration::from_secs(10));
    }
}
