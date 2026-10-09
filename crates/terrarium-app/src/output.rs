//! A command's result on stdout (`--version`, `--probe-readme`, `replay --dump`, `discover`).

use std::io::{ErrorKind, Write as _};
use std::process::ExitCode;

/// Writes `text` to stdout. A reader that stops early, as `head` does, is not a failure; any
/// other error is printed as `failed` words it.
pub fn print(text: &str, failed: impl FnOnce(&std::io::Error) -> String) -> ExitCode {
    let mut out = std::io::stdout().lock();
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if err.kind() == ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{}", failed(&err));
            ExitCode::FAILURE
        }
    }
}
