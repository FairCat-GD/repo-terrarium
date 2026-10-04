//! The one place a process is started (I-5). `cargo xtask verify`'s graph stage refuses
//! `std::process::Command` in any other crate source.
//!
//! [`run_user_command`] runs the command a person typed after `repo-terrarium record-tests --`.
//! It is theirs, named explicitly on their own command line, so it runs as their shell would run
//! it: their environment, their working directory, their terminal. It is executed directly,
//! never through a shell, so its arguments reach it exactly as given. Nothing found in a
//! repository is ever run from here without the user's per-adapter opt-in.

use std::ffi::{OsStr, OsString};
use std::io;
#[cfg(any(windows, test))]
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

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
}
