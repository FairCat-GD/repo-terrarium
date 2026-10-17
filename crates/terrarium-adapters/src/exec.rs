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
//! - [`KnipRun::run`] runs the Knip a repository installed, with the person's own node, and
//!   keeps what it prints. It is the one run of code from a repository, so a [`KnipRun`] is
//!   made only from the opt-in the settings hold for that repository
//!   ([`KnipRun::permitted`]). Knip writes its own cache inside the repository
//!   (`node_modules/.cache`): the one exemption from N-10, and Knip's write, never the app's.
//!
//! Each is executed directly, never through a shell, so its arguments reach it exactly as
//! given. Nothing found in a repository is ever run from here without the user's per-adapter
//! opt-in.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

use crate::binding::RepoBinding;
use crate::report::{Format, KNIP_MIN_MAJOR, installed_knip, major, read_bounded, text};

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

/// node's file name: on Windows only `node.exe`, for the same reason as ffmpeg's.
const NODE_NAMES: &[&str] = if cfg!(windows) {
    &["node.exe"]
} else {
    &["node"]
};

/// The person's ffmpeg: on `PATH`, then, on macOS, where its package managers put it; never
/// one inside `outside`, the repository.
pub fn find_ffmpeg(outside: Option<&Path>) -> Option<Tool> {
    find_installed(FFMPEG_NAMES, outside)
}

/// The person's node, looked for as ffmpeg is; never one inside `outside`, the repository, so
/// a repository cannot hand Knip a node of its own.
pub fn find_node(outside: Option<&Path>) -> Option<Tool> {
    find_installed(NODE_NAMES, outside)
}

/// The first of `names` on `PATH`, then, on macOS, where its package managers put tools.
fn find_installed(names: &[&str], outside: Option<&Path>) -> Option<Tool> {
    let also: &[&str] = if cfg!(target_os = "macos") {
        MACOS_TOOL_DIRS
    } else {
        &[]
    };
    let also: Vec<&Path> = also.iter().map(Path::new).collect();
    let path = std::env::var_os("PATH");
    find_tool(names, path.as_deref(), &also, outside)
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

/// What a started tool keeps of this process's environment on Windows: `SystemRoot`, which its
/// system libraries need, and the temporary folder, which Windows otherwise puts in its own
/// folder, where a person cannot write.
const WINDOWS_KEPT_ENV: &[&str] = &["SystemRoot", "TEMP", "TMP"];

/// What it keeps elsewhere: nothing.
const KEPT_ENV: &[&str] = if cfg!(windows) { WINDOWS_KEPT_ENV } else { &[] };

/// Each of `names` that `get` finds set, with its value.
fn kept_env(
    names: &[&'static str],
    get: impl Fn(&str) -> Option<OsString>,
) -> Vec<(&'static str, OsString)> {
    names
        .iter()
        .filter_map(|&name| get(name).map(|value| (name, value)))
        .collect()
}

/// Runs `tool` with `args` in `cwd`, writing its stdin through `feed`, and waits for it to end.
/// Its stdout is discarded and the end of its stderr kept for the error; its environment is
/// empty but for what Windows needs ([`WINDOWS_KEPT_ENV`]). Past `limit` it is
/// stopped, even while `feed` is writing. It is always waited for, so none outlives the run;
/// a panic in `feed` stops it, then carries on.
pub fn run_fed(
    tool: &Tool,
    args: &[OsString],
    cwd: &Path,
    limit: Duration,
    feed: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> Result<(), FedError> {
    let mut command = bare(tool, args, cwd);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(FedError::CannotRun)?;
    let (Some(mut stdin), Some(stderr)) = (child.stdin.take(), child.stderr.take()) else {
        end(&mut child);
        return Err(FedError::NoPipes);
    };
    let watched = Watched::start(child, stderr, limit).map_err(FedError::CannotRun)?;
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| feed(&mut stdin)));
    // Closed, so the tool sees its input end.
    drop(stdin);
    if !matches!(outcome, Ok(Ok(()))) {
        let _ = lock(&watched.child).kill();
    }
    let ended = watched.wait(None);
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(payload) => panic::resume_unwind(payload),
    };
    if ended.timed_out {
        return Err(FedError::TimedOut);
    }
    let status = ended.status.map_err(FedError::CannotRun)?;
    match (outcome, status.success()) {
        (Ok(()), true) => Ok(()),
        (Err(err), true) => Err(FedError::Feed(err)),
        (_, false) => Err(FedError::Failed {
            ended: finished(status),
            stderr: ended.stderr,
        }),
    }
}

/// `tool` with `args` in `cwd`, executed directly, with an empty environment but for what
/// Windows needs ([`WINDOWS_KEPT_ENV`]), and on Windows no console window.
fn bare(tool: &Tool, args: &[OsString], cwd: &Path) -> Command {
    let mut command = Command::new(&tool.0);
    command
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(kept_env(KEPT_ENV, |name| std::env::var_os(name)));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// `path` as a program run from here should be given it. On Windows a canonical path is
/// verbatim (`\\?\C:\…`), which node's module loader and the working directory a program sees
/// do not take as a plain path; it is given plain when the plain form names the same file.
fn plain_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(plain) = path.to_str().and_then(plain_windows_path) {
        return PathBuf::from(plain);
    }
    path.to_path_buf()
}

/// The plain form of a verbatim Windows path, when it names the same file: `\\?\C:\a` is
/// `C:\a`, and `\\?\UNC\server\share\a` is `\\server\share\a`. `None` for any other form, and
/// for a verbatim path whose plain form Windows would read otherwise: one holding `/`, a
/// component ending in a dot (`.` and `..` among them) or a space, a device name, or longer than
/// `MAX_PATH`.
#[cfg(any(windows, test))]
fn plain_windows_path(text: &str) -> Option<String> {
    const MAX_PATH: usize = 260;
    const DEVICES: &[&str] = &["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
    let rest = text.strip_prefix(r"\\?\")?;
    let (plain, components) = if let Some(unc) = rest.strip_prefix(r"UNC\") {
        (format!(r"\\{unc}"), unc)
    } else {
        let mut chars = rest.chars();
        let drive = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.next() == Some(':')
            && chars.next() == Some('\\');
        if !drive {
            return None;
        }
        (rest.to_owned(), rest.get(3..)?)
    };
    let device = |name: &str| {
        let stem = name.split('.').next().unwrap_or(name).trim_end();
        let numbered = |prefix: &str| {
            stem.len() == 4
                && stem
                    .get(..3)
                    .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
                && stem[3..].chars().all(|c| matches!(c, '1'..='9'))
        };
        DEVICES.iter().any(|d| stem.eq_ignore_ascii_case(d)) || numbered("COM") || numbered("LPT")
    };
    let reads_otherwise = plain.contains('/')
        || plain.encode_utf16().count() >= MAX_PATH
        || components
            .split('\\')
            .any(|part| part.ends_with(['.', ' ']) || device(part));
    (!reads_otherwise).then_some(plain)
}

/// A started tool, watched until it ends: the end of its stderr is read on a thread of its
/// own, so a tool that writes much to it never blocks on a full pipe, and a watchdog stops it
/// once `limit` has passed.
struct Watched {
    child: Arc<Mutex<Child>>,
    timed_out: Arc<AtomicBool>,
    done: mpsc::Sender<()>,
    watchdog: JoinHandle<()>,
    tail: mpsc::Receiver<String>,
}

/// How a watched tool ended.
struct Ended {
    status: io::Result<ExitStatus>,
    /// The end of what it wrote to stderr.
    stderr: String,
    /// The watchdog stopped it at its limit.
    timed_out: bool,
    /// It was stopped because `stop` was set.
    stopped: bool,
}

impl Watched {
    /// Watches `child`. When a thread cannot start, the child is stopped and waited for.
    fn start(mut child: Child, stderr: ChildStderr, limit: Duration) -> io::Result<Watched> {
        let (said, tail) = mpsc::channel();
        let reader = thread::Builder::new()
            .name("terrarium-tool-stderr".into())
            .spawn(move || {
                let _ = said.send(keep_tail(stderr));
            });
        if let Err(err) = reader {
            end(&mut child);
            return Err(err);
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
        match watchdog {
            Ok(watchdog) => Ok(Watched {
                child,
                timed_out,
                done,
                watchdog,
                tail,
            }),
            Err(err) => {
                end(&mut lock(&child));
                Err(err)
            }
        }
    }

    /// Waits for the tool to end, stopping it as soon as `stop` is set, then for the end of its
    /// stderr, never longer than [`TAIL_GRACE`] once it has ended.
    fn wait(self, stop: Option<&AtomicBool>) -> Ended {
        let mut stopped = false;
        let status = loop {
            let mut child = lock(&self.child);
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {
                    if !stopped && stop.is_some_and(|stop| stop.load(Ordering::SeqCst)) {
                        stopped = true;
                        let _ = child.kill();
                    }
                }
                Err(err) => break Err(err),
            }
            drop(child);
            thread::sleep(WAIT_STEP);
        };
        drop(self.done);
        let _ = self.watchdog.join();
        Ended {
            status,
            stderr: self.tail.recv_timeout(TAIL_GRACE).unwrap_or_default(),
            timed_out: self.timed_out.load(Ordering::SeqCst),
            stopped,
        }
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

/// What a captured run printed, and how it ended.
#[derive(Debug)]
struct Captured {
    stdout: Vec<u8>,
    ended: Finished,
    stderr: String,
}

/// Why a captured run printed nothing to keep.
#[derive(Debug)]
enum CaptureError {
    CannotRun(io::Error),
    NoPipes,
    TimedOut,
    Stopped,
    /// It printed more than the cap, and was stopped.
    TooLarge,
}

/// Runs `tool` with `args` in `cwd`, with nothing on stdin and an environment holding only
/// `PATH` (and Windows' `SystemRoot`, `TEMP` and `TMP`), and keeps what it prints to stdout, up to `cap` bytes:
/// past that it is stopped. Past `limit`, or once `stop` is set, it is stopped too. It is
/// always waited for; a process it started that holds stdout open is not waited for past
/// [`TAIL_GRACE`].
fn run_captured(
    tool: &Tool,
    args: &[OsString],
    cwd: &Path,
    path: &OsStr,
    limit: Duration,
    cap: u64,
    stop: &AtomicBool,
) -> Result<Captured, CaptureError> {
    let mut command = bare(tool, args, cwd);
    command
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(CaptureError::CannotRun)?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        end(&mut child);
        return Err(CaptureError::NoPipes);
    };
    let watched = Watched::start(child, stderr, limit).map_err(CaptureError::CannotRun)?;
    let kept = Arc::new(Mutex::new(Vec::new()));
    let too_large = Arc::new(AtomicBool::new(false));
    let (read, finished_reading) = mpsc::channel::<()>();
    let reader = {
        let (kept, too_large, child) = (
            Arc::clone(&kept),
            Arc::clone(&too_large),
            Arc::clone(&watched.child),
        );
        thread::Builder::new()
            .name("terrarium-tool-stdout".into())
            .spawn(move || {
                keep_head(stdout, cap, &kept, &too_large, &child);
                let _ = read.send(());
            })
    };
    if let Err(err) = reader {
        let _ = lock(&watched.child).kill();
        let _ = watched.wait(None);
        return Err(CaptureError::CannotRun(err));
    }
    let ended = watched.wait(Some(stop));
    let _ = finished_reading.recv_timeout(TAIL_GRACE);
    if ended.stopped {
        return Err(CaptureError::Stopped);
    }
    if ended.timed_out {
        return Err(CaptureError::TimedOut);
    }
    if too_large.load(Ordering::SeqCst) {
        return Err(CaptureError::TooLarge);
    }
    let status = ended.status.map_err(CaptureError::CannotRun)?;
    let stdout = std::mem::take(&mut *kept.lock().unwrap_or_else(PoisonError::into_inner));
    Ok(Captured {
        stdout,
        ended: finished(status),
        stderr: ended.stderr,
    })
}

/// Reads `stdout` into `kept` until it ends, or until it passes `cap`: then the tool is
/// stopped, since what it prints can no longer be kept whole.
fn keep_head(
    mut stdout: impl Read,
    cap: u64,
    kept: &Mutex<Vec<u8>>,
    too_large: &AtomicBool,
    child: &Mutex<Child>,
) {
    let mut buf = [0; 8192];
    loop {
        match stdout.read(&mut buf) {
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Ok(0) | Err(_) => return,
            Ok(n) => {
                let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
                kept.extend_from_slice(&buf[..n]);
                if u64::try_from(kept.len()).unwrap_or(u64::MAX) > cap {
                    kept.clear();
                    too_large.store(true, Ordering::SeqCst);
                    let _ = lock(child).kill();
                    return;
                }
            }
        }
    }
}

/// How long Knip may run.
pub const KNIP_LIMIT: Duration = Duration::from_secs(180);
/// Knip's report on stdout, and nothing else there.
const KNIP_ARGS: [&str; 3] = ["--reporter", "json", "--no-progress"];
/// Where an installed Knip's package lives, and its manifest, relative to the repository root.
const KNIP_PACKAGE: &str = "node_modules/knip";
const KNIP_MANIFEST_MAX_BYTES: u64 = 1024 * 1024;

/// Knip, permitted to run in one repository. Made only by [`KnipRun::permitted`], from the
/// opt-in the settings hold, so nothing runs from a repository the person has not opted in
/// for (I-5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnipRun {
    root: PathBuf,
}

/// Why Knip did not give a report.
#[derive(Debug)]
pub enum KnipError {
    /// No Knip in the repository's `node_modules`, or one whose manifest names no program.
    NotInstalled,
    /// An installed Knip before 6, whose report this does not read: never run.
    Before6(String),
    /// The program Knip's manifest names is outside `node_modules`: never run.
    Outside,
    CannotRun(io::Error),
    TimedOut,
    /// The run was stopped: the opt-in was withdrawn, or the app is closing.
    Stopped,
    /// It printed more than a Knip report is read up to.
    TooLarge,
    /// It ended without a report: Knip's own error (code 2), any other code or a signal, or
    /// nothing printed. The end of its stderr says why.
    Failed {
        ended: Finished,
        stderr: String,
    },
}

impl KnipRun {
    /// Knip for the repository `here`, when `opt_in` names it: the same seed at the same
    /// folder.
    pub fn permitted(opt_in: Option<&RepoBinding>, here: &RepoBinding) -> Option<KnipRun> {
        (opt_in? == here).then(|| KnipRun {
            root: PathBuf::from(&here.path),
        })
    }

    /// The repository Knip runs in.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Runs the repository's Knip with `node`, the person's (found outside the repository by
    /// [`find_node`]), and returns its JSON report. The program is the one Knip's manifest
    /// names, which `node_modules/.bin/knip` runs too; it is run by node directly, since
    /// `.bin/knip` would need node on `PATH`, and is a `.cmd`, run by a shell, on Windows. A
    /// Knip before 6, or a program outside `node_modules`, is never run. Exit codes 0 and 1
    /// carry a report (1 says it found something); anything else does not.
    pub fn run(&self, node: &Tool, stop: &AtomicBool) -> Result<Vec<u8>, KnipError> {
        let version = installed_knip(&self.root).ok_or(KnipError::NotInstalled)?;
        if major(&version).is_some_and(|major| major < KNIP_MIN_MAJOR) {
            return Err(KnipError::Before6(version));
        }
        let script = knip_program(&self.root)?;
        let mut args = vec![plain_path(&script).into_os_string()];
        args.extend(KNIP_ARGS.iter().map(OsString::from));
        let cap = Format::Knip.max_file_bytes();
        let cwd = plain_path(&self.root);
        match run_captured(node, &args, &cwd, &node_path(node), KNIP_LIMIT, cap, stop) {
            Ok(captured) => {
                let printed = captured.stdout.iter().any(|b| !b.is_ascii_whitespace());
                match captured.ended {
                    Finished::Code(0 | 1) if printed => Ok(captured.stdout),
                    ended => Err(KnipError::Failed {
                        ended,
                        stderr: captured.stderr,
                    }),
                }
            }
            Err(CaptureError::CannotRun(err)) => Err(KnipError::CannotRun(err)),
            Err(CaptureError::NoPipes) => Err(KnipError::CannotRun(io::Error::other(
                "the run's output could not be read",
            ))),
            Err(CaptureError::TimedOut) => Err(KnipError::TimedOut),
            Err(CaptureError::Stopped) => Err(KnipError::Stopped),
            Err(CaptureError::TooLarge) => Err(KnipError::TooLarge),
        }
    }
}

/// The `PATH` node runs with: its own folder alone, so a program it starts by name is one
/// beside it.
fn node_path(node: &Tool) -> OsString {
    node.path()
        .parent()
        .map_or_else(OsString::new, |dir| dir.as_os_str().to_owned())
}

/// How one run of the probe's node check went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunCheck {
    /// It ran and printed what was asked: node's version, or the script's answer.
    Answered(String),
    /// It ended well but printed something else.
    Unexpected,
    /// It ended with this, without the answer.
    Failed(Finished),
    /// It could not be started, or its output could not be read.
    CannotRun,
    TimedOut,
}

/// What the probe learns about running node the way Knip is run: with its own folder as `PATH`,
/// the kept environment, no console window, and a script whose canonical path and folder are
/// given plain. It carries no path and nothing node wrote to stderr, which name the person's
/// folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeCheck {
    /// `node --version`.
    pub version: RunCheck,
    /// A module script in a folder of its own, in the temporary folder.
    pub script: RunCheck,
    /// Whether node's temporary folder was under Windows' own folder, where a person cannot
    /// write; `None` when the script did not say.
    pub temp_in_system_root: Option<bool>,
}

/// How long each run of the node check may take.
const CHECK_LIMIT: Duration = Duration::from_secs(30);
/// The most either run of the node check may print.
const CHECK_CAP: u64 = 4096;

/// The probe's script: it answers `ok`, and whether `os.tmpdir()` lies under `SystemRoot`.
const NODE_CHECK_SCRIPT: &str = "import os from 'node:os';
const root = (process.env.SystemRoot || '').toLowerCase();
const under = root !== '' && os.tmpdir().toLowerCase().startsWith(root);
console.log('ok ' + under);
";

/// Runs `node` as Knip's run does, twice: for its version, then on a module script written to a
/// folder of its own in the temporary folder, which is removed after.
pub fn check_node(node: &Tool) -> NodeCheck {
    check_runs(
        node,
        &["--version".into()],
        "repo-terrarium-check.mjs",
        NODE_CHECK_SCRIPT,
    )
}

fn check_runs(node: &Tool, version: &[OsString], file: &str, script: &str) -> NodeCheck {
    // Each check has its own folder, even two at once.
    static CHECKS: AtomicU32 = AtomicU32::new(0);
    let stop = AtomicBool::new(false);
    let path = node_path(node);
    let temp = std::env::temp_dir();
    let run = |args: &[OsString], cwd: &Path| match run_captured(
        node,
        args,
        cwd,
        &path,
        CHECK_LIMIT,
        CHECK_CAP,
        &stop,
    ) {
        Ok(captured) => match captured.ended {
            Finished::Code(0) => Ok(String::from_utf8_lossy(&captured.stdout).trim().to_owned()),
            ended => Err(RunCheck::Failed(ended)),
        },
        Err(CaptureError::TimedOut) => Err(RunCheck::TimedOut),
        Err(_) => Err(RunCheck::CannotRun),
    };
    let version = match run(version, &temp) {
        Ok(printed) if looks_like_a_version(&printed) => RunCheck::Answered(printed),
        Ok(_) => RunCheck::Unexpected,
        Err(check) => check,
    };
    let dir = temp.join(format!(
        "repo-terrarium-check-{}-{}",
        std::process::id(),
        CHECKS.fetch_add(1, Ordering::Relaxed)
    ));
    let ran = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join(file), script))
        .and_then(|()| dir.canonicalize());
    let (script, temp_in_system_root) = match ran {
        Err(_) => (RunCheck::CannotRun, None),
        Ok(canonical) => {
            let args = [plain_path(&canonical.join(file)).into_os_string()];
            match run(&args, &plain_path(&canonical)) {
                Ok(printed) => match printed.as_str() {
                    "ok true" => (RunCheck::Answered("ok".to_owned()), Some(true)),
                    "ok false" => (RunCheck::Answered("ok".to_owned()), Some(false)),
                    _ => (RunCheck::Unexpected, None),
                },
                Err(check) => (check, None),
            }
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    NodeCheck {
        version,
        script,
        temp_in_system_root,
    }
}

/// `v` and a dotted number, as `node --version` prints it: anything else is not kept.
fn looks_like_a_version(text: &str) -> bool {
    text.len() <= 32
        && text.strip_prefix('v').is_some_and(|rest| {
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.')
        })
}

/// The program Knip's manifest names (`bin`, a path or a map holding `knip`), which must be a
/// file inside the repository's `node_modules` once links are followed.
fn knip_program(root: &Path) -> Result<PathBuf, KnipError> {
    let package = root.join(KNIP_PACKAGE);
    let manifest = read_bounded(
        &package.join("package.json"),
        "node_modules/knip/package.json",
        KNIP_MANIFEST_MAX_BYTES,
    )
    .map_err(|_| KnipError::NotInstalled)?;
    let manifest: Value = text(&manifest)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or(KnipError::NotInstalled)?;
    let bin = match manifest.get("bin") {
        Some(Value::String(bin)) => bin,
        Some(Value::Object(bins)) => match bins.get("knip") {
            Some(Value::String(bin)) => bin,
            _ => return Err(KnipError::NotInstalled),
        },
        _ => return Err(KnipError::NotInstalled),
    };
    let modules = root
        .join("node_modules")
        .canonicalize()
        .map_err(|_| KnipError::NotInstalled)?;
    let program = package
        .join(bin)
        .canonicalize()
        .map_err(|_| KnipError::NotInstalled)?;
    if !program.starts_with(&modules) {
        return Err(KnipError::Outside);
    }
    if !program.is_file() {
        return Err(KnipError::NotInstalled);
    }
    Ok(program)
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

    /// `/bin/sh -c <script>`, captured, with `PATH` set to `/bin`.
    #[cfg(unix)]
    fn captured(
        script: &str,
        cwd: &Path,
        limit: Duration,
        cap: u64,
        stop: &AtomicBool,
    ) -> Result<Captured, CaptureError> {
        run_captured(
            &Tool(PathBuf::from("/bin/sh")),
            &["-c".into(), script.into()],
            cwd,
            OsStr::new("/bin"),
            limit,
            cap,
            stop,
        )
    }

    #[cfg(unix)]
    #[test]
    fn what_a_captured_tool_prints_is_kept_whole_with_only_path_in_its_environment() {
        let dir = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        let ran = captured(
            "/bin/dd if=/dev/zero bs=65536 count=16 2>/dev/null; echo why >&2; exit 1",
            dir.path(),
            Duration::from_secs(30),
            2 << 20,
            &stop,
        )
        .unwrap();
        assert_eq!(ran.stdout.len(), 1 << 20);
        assert_eq!(ran.ended, Finished::Code(1));
        assert_eq!(ran.stderr, "why");
        let env = captured(
            "/usr/bin/env",
            dir.path(),
            Duration::from_secs(30),
            1 << 20,
            &stop,
        )
        .unwrap();
        let env = String::from_utf8(env.stdout).unwrap();
        let names: Vec<&str> = env
            .lines()
            .filter_map(|line| line.split('=').next())
            .filter(|name| !name.is_empty())
            .collect();
        // `sh` sets a few of its own (`PWD`, `SHLVL`, `_`); nothing comes from this process.
        assert!(env.lines().any(|line| line == "PATH=/bin"), "{env}");
        for name in [
            "HOME",
            "USER",
            "TMPDIR",
            "CARGO",
            "RUST_LOG",
            "NODE_OPTIONS",
        ] {
            assert!(!names.contains(&name), "{env}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_captured_tool_is_stopped_past_its_cap_its_limit_or_its_stop() {
        let dir = tempfile::tempdir().unwrap();
        let go = AtomicBool::new(false);
        let start = std::time::Instant::now();
        let large = captured(
            "/bin/dd if=/dev/zero bs=65536 count=64 2>/dev/null; /bin/sleep 30",
            dir.path(),
            Duration::from_secs(60),
            1 << 20,
            &go,
        );
        assert!(matches!(large, Err(CaptureError::TooLarge)), "{large:?}");
        let slow = captured(
            "/bin/sleep 30",
            dir.path(),
            Duration::from_millis(300),
            1 << 20,
            &go,
        );
        assert!(matches!(slow, Err(CaptureError::TimedOut)), "{slow:?}");
        let stop = Arc::new(AtomicBool::new(false));
        let setter = {
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(200));
                stop.store(true, Ordering::SeqCst);
            })
        };
        let stopped = captured(
            "/bin/sleep 30",
            dir.path(),
            Duration::from_secs(60),
            1 << 20,
            &stop,
        );
        setter.join().unwrap();
        assert!(matches!(stopped, Err(CaptureError::Stopped)), "{stopped:?}");
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            start.elapsed()
        );
    }

    /// A repository whose `node_modules` holds a Knip of `version`, whose program (run here by
    /// `sh`, standing in for node) prints `prints`, says how it was called in `called`, and ends
    /// with `code`.
    #[cfg(unix)]
    fn fake_knip(root: &Path, version: &str, bin: &str, prints: &str, code: i32) {
        let package = root.join("node_modules/knip");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(
            package.join("package.json"),
            format!(r#"{{"name":"knip","version":"{version}","bin":{bin}}}"#),
        )
        .unwrap();
        std::fs::write(
            package.join("bin/knip.js"),
            format!("echo \"$@\" > called\nprintf '%s' '{prints}'\nexit {code}\n"),
        )
        .unwrap();
    }

    #[cfg(unix)]
    fn sh_as_node() -> Tool {
        find_tool(&["sh"], None, &[Path::new("/bin")], None).unwrap()
    }

    fn binding(seed: &str, path: &Path) -> RepoBinding {
        RepoBinding {
            seed: seed.to_owned(),
            path: path.to_string_lossy().into_owned(),
        }
    }

    #[test]
    fn knip_is_permitted_only_by_the_opt_in_given_for_this_repository() {
        let here = binding("aa", Path::new("/work/garden"));
        assert_eq!(KnipRun::permitted(None, &here), None);
        let other_seed = binding("bb", Path::new("/work/garden"));
        assert_eq!(KnipRun::permitted(Some(&other_seed), &here), None);
        let other_folder = binding("aa", Path::new("/work/garden-fork"));
        assert_eq!(KnipRun::permitted(Some(&other_folder), &here), None);
        let run = KnipRun::permitted(Some(&here), &here).unwrap();
        assert_eq!(run.root(), Path::new("/work/garden"));
    }

    #[cfg(unix)]
    #[test]
    fn knip_runs_its_own_program_with_its_report_arguments_and_gives_its_report() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let report = r#"{"issues":[]}"#;
        fake_knip(&root, "6.2.0", r#"{"knip":"bin/knip.js"}"#, report, 1);
        let run = KnipRun::permitted(Some(&binding("aa", &root)), &binding("aa", &root)).unwrap();
        let stop = AtomicBool::new(false);
        assert_eq!(run.run(&sh_as_node(), &stop).unwrap(), report.as_bytes());
        let called = std::fs::read_to_string(root.join("called")).unwrap();
        assert_eq!(called.trim(), "--reporter json --no-progress");
        // A plain path in `bin` names the program as well.
        fake_knip(&root, "6.2.0", r#""bin/knip.js""#, report, 0);
        assert_eq!(run.run(&sh_as_node(), &stop).unwrap(), report.as_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn knip_that_ends_without_a_report_says_how() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let run = KnipRun::permitted(Some(&binding("aa", &root)), &binding("aa", &root)).unwrap();
        let stop = AtomicBool::new(false);
        for (prints, code) in [("{}", 2), ("", 1), ("  ", 0)] {
            fake_knip(&root, "6.2.0", r#""bin/knip.js""#, prints, code);
            match run.run(&sh_as_node(), &stop) {
                Err(KnipError::Failed { ended, .. }) => assert_eq!(ended, Finished::Code(code)),
                other => panic!("{prints:?} {code}: {other:?}"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn knip_before_6_missing_or_outside_node_modules_never_runs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let run = KnipRun::permitted(Some(&binding("aa", &root)), &binding("aa", &root)).unwrap();
        let stop = AtomicBool::new(false);
        let node = sh_as_node();
        assert!(matches!(
            run.run(&node, &stop),
            Err(KnipError::NotInstalled)
        ));
        fake_knip(&root, "5.66.0", r#""bin/knip.js""#, "{}", 0);
        assert!(matches!(run.run(&node, &stop), Err(KnipError::Before6(v)) if v == "5.66.0"));
        std::fs::write(root.join("outside.js"), "touch called\n").unwrap();
        fake_knip(&root, "6.2.0", r#""../../outside.js""#, "{}", 0);
        assert!(matches!(run.run(&node, &stop), Err(KnipError::Outside)));
        fake_knip(&root, "6.2.0", r#"{"knip-bun":"bin/knip.js"}"#, "{}", 0);
        assert!(matches!(
            run.run(&node, &stop),
            Err(KnipError::NotInstalled)
        ));
        assert!(!root.join("called").exists(), "nothing ran");
    }

    #[test]
    fn a_verbatim_drive_path_loses_its_prefix() {
        assert_eq!(
            plain_windows_path(r"\\?\C:\work\garden\node_modules\knip\bin\knip.js").as_deref(),
            Some(r"C:\work\garden\node_modules\knip\bin\knip.js")
        );
        assert_eq!(plain_windows_path(r"\\?\d:\").as_deref(), Some(r"d:\"));
    }

    #[test]
    fn a_verbatim_unc_path_becomes_plain_unc() {
        assert_eq!(
            plain_windows_path(r"\\?\UNC\fileserver\share\garden").as_deref(),
            Some(r"\\fileserver\share\garden")
        );
    }

    #[test]
    fn a_path_verbatim_would_read_differently_keeps_its_prefix() {
        let long = format!(r"\\?\C:\{}", "a".repeat(260));
        for path in [
            r"\\?\C:\work/garden",
            r"\\?\C:\work\..\garden",
            r"\\?\C:\work\.\garden",
            r"\\?\C:\work\garden.",
            r"\\?\C:\work\garden ",
            r"\\?\C:\work\nul",
            r"\\?\C:\work\Con.txt",
            r"\\?\C:\work\com7",
            r"\\?\C:\work\LPT1.log",
            r"\\?\UNC\server\share\aux",
            &long,
        ] {
            assert_eq!(plain_windows_path(path), None, "{path}");
        }
        // Near-misses of a device name are ordinary names.
        for name in ["console", "com0", "com10", "nulls"] {
            let path = format!(r"\\?\C:\work\{name}");
            assert!(plain_windows_path(&path).is_some(), "{name}");
        }
    }

    #[test]
    fn other_forms_are_left_alone() {
        for path in [
            r"C:\work\garden",
            r"\\server\share\garden",
            r"\\?\Volume{0b3c5a0e-0000-0000-0000-100000000000}\garden",
            r"\\.\C:\work",
            r"\\?\GLOBALROOT\Device\HarddiskVolume3\work",
            "/work/garden",
            "",
        ] {
            assert_eq!(plain_windows_path(path), None, "{path}");
        }
    }

    #[test]
    fn windows_keeps_systemroot_temp_and_tmp_and_nothing_else() {
        let set = |name: &str| {
            [
                ("SystemRoot", r"C:\Windows"),
                ("TEMP", r"C:\Temp"),
                ("TMP", r"C:\Temp"),
                ("PATH", r"C:\tools"),
                ("USERPROFILE", r"C:\Users\robin"),
                ("NODE_OPTIONS", "--require evil.js"),
            ]
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| OsString::from(v))
        };
        let kept: Vec<&str> = kept_env(WINDOWS_KEPT_ENV, set)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(kept, ["SystemRoot", "TEMP", "TMP"]);
        assert_eq!(kept_env(WINDOWS_KEPT_ENV, |_| None), []);
        if !cfg!(windows) {
            assert!(KEPT_ENV.is_empty());
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_node_check_runs_it_for_its_version_and_on_a_script_of_its_own() {
        let sh = sh_as_node();
        // `sh` stands in for node: `-c` prints a version; the script is a shell script.
        let check = check_runs(
            &sh,
            &["-c".into(), "echo v22.11.0".into()],
            "check.sh",
            "echo ok false\n",
        );
        assert_eq!(check.version, RunCheck::Answered("v22.11.0".to_owned()));
        assert_eq!(check.script, RunCheck::Answered("ok".to_owned()));
        assert_eq!(check.temp_in_system_root, Some(false));
        let ours = format!("repo-terrarium-check-{}-", std::process::id());
        let leftover = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().starts_with(&ours));
        assert!(!leftover, "the script's folder is removed");
        // A version line that is not one, a script that says something else, and failures.
        let odd = check_runs(
            &sh,
            &["-c".into(), "echo /home/robin/node".into()],
            "check.sh",
            "echo hello\n",
        );
        assert_eq!(odd.version, RunCheck::Unexpected);
        assert_eq!(odd.script, RunCheck::Unexpected);
        assert_eq!(odd.temp_in_system_root, None);
        let failed = check_runs(&sh, &["-c".into(), "exit 4".into()], "check.sh", "exit 9\n");
        assert_eq!(failed.version, RunCheck::Failed(Finished::Code(4)));
        assert_eq!(failed.script, RunCheck::Failed(Finished::Code(9)));
        let missing = check_node(&Tool(PathBuf::from("/no/such/node")));
        assert_eq!(missing.version, RunCheck::CannotRun);
        assert_eq!(missing.script, RunCheck::CannotRun);
    }

    #[test]
    fn only_a_version_is_kept_from_node_s_version_line() {
        assert!(looks_like_a_version("v22.11.0"));
        assert!(!looks_like_a_version("v"));
        assert!(!looks_like_a_version("22.11.0"));
        assert!(!looks_like_a_version("v22.11.0 C:\\Users\\robin"));
        assert!(!looks_like_a_version(&format!("v{}", "1".repeat(40))));
    }

    #[cfg(unix)]
    #[test]
    fn node_is_never_one_inside_the_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        tool_file(&root.join("repo/bin/node"), true);
        let path = std::env::join_paths([root.join("repo/bin")]).unwrap();
        let repo = root.join("repo");
        assert_eq!(find_tool(NODE_NAMES, Some(&path), &[], Some(&repo)), None);
        assert!(find_tool(NODE_NAMES, Some(&path), &[], None).is_some());
    }
}
