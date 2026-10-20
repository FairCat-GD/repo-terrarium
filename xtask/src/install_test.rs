//! `cargo xtask install-test`, on Windows (the release workflow's Windows job): installs the
//! installer `package` made, as a scripted install runs it (`/S`), and checks what it put in
//! place and that the installed app runs; then uninstalls it and checks that nothing it wrote
//! is left: the files, the Start menu's shortcut, the installed apps' entry and the login item.
//! Every launch runs in a data folder of its own (`TERRARIUM_DATA_DIR`), so a machine's own is
//! never touched. When the machine can show the app, an app started before the uninstall must
//! be quit by it.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use terrarium_store::identifiers::{
    BINARY, DATA_DIR_ENV, INSTANCE_LOCK_FILE, WINDOWS_INSTALL_DIR, WINDOWS_RUN_VALUE,
    WINDOWS_UNINSTALL_KEY,
};

use crate::package::{self, words};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// How long the uninstaller may take: `--quit` waits up to a minute for the app.
const UNINSTALL_WAIT: Duration = Duration::from_secs(120);
/// How long a started app is given to show it runs.
const START_WAIT: Duration = Duration::from_secs(10);

pub fn run(root: &Path) -> ExitCode {
    if !cfg!(windows) {
        println!("install-test: runs on Windows, where the installer installs");
        return ExitCode::FAILURE;
    }
    match test(root) {
        Ok(()) => {
            println!("install-test: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("install-test: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Where the installer puts what it installs, under this user's folders.
struct Places {
    dir: PathBuf,
    exe: PathBuf,
    uninstaller: PathBuf,
    licences: PathBuf,
    shortcut: PathBuf,
}

impl Places {
    fn new(local_app_data: &Path, app_data: &Path, product: &str) -> Places {
        let dir = local_app_data.join("Programs").join(WINDOWS_INSTALL_DIR);
        Places {
            exe: dir.join(format!("{BINARY}.exe")),
            uninstaller: dir.join("uninstall.exe"),
            licences: dir.join("LICENSES.txt"),
            shortcut: app_data
                .join(r"Microsoft\Windows\Start Menu\Programs")
                .join(format!("{product}.lnk")),
            dir,
        }
    }
}

const UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

fn uninstall_entry() -> String {
    format!(r"{UNINSTALL_KEY}\{WINDOWS_UNINSTALL_KEY}")
}

fn test(root: &Path) -> Result<(), String> {
    let installer = package::out_dir(root).join(package::installer_name(VERSION));
    if !installer.is_file() {
        return Err(format!(
            "no installer at {}: run cargo xtask package first",
            installer.display()
        ));
    }
    let folder = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .ok_or_else(|| format!("{name} is not set"))
    };
    let words = words::read(root)?;
    let data = tempfile::tempdir().map_err(|err| err.to_string())?;
    let machine = Machine {
        places: Places::new(
            &folder("LOCALAPPDATA")?,
            &folder("APPDATA")?,
            &words.product,
        ),
        data: data.path(),
    };
    machine.install(&installer)?;
    machine.installed_runs()?;
    let app = machine.start_app()?;
    machine.uninstall(app)
}

/// This machine's install, and the data folder every launch here is given.
struct Machine<'a> {
    places: Places,
    data: &'a Path,
}

impl Machine<'_> {
    /// `program`, run with the test's data folder.
    fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command.env(DATA_DIR_ENV, self.data);
        command
    }

    /// Installs as a script does, and checks what the install put in place.
    fn install(&self, installer: &Path) -> Result<(), String> {
        succeeds(self.command(installer).arg("/S"), "the installer")?;
        let places = &self.places;
        for path in [
            &places.exe,
            &places.uninstaller,
            &places.licences,
            &places.shortcut,
        ] {
            if !path.is_file() {
                return Err(format!("the install left no {}", path.display()));
            }
        }
        let listed = reg(&["query", &uninstall_entry(), "/v", "DisplayVersion"])?;
        if !listed.contains(VERSION) {
            return Err(format!(
                "the installed apps' entry is not {VERSION}: {listed}"
            ));
        }
        if app_running()? {
            return Err("a scripted install (/S) opened the app".to_owned());
        }
        println!("install-test: installed, listed as {VERSION}, nothing opened");
        Ok(())
    }

    /// The installed app runs: its version, and `--quit` with nothing running, which writes
    /// nothing.
    fn installed_runs(&self) -> Result<(), String> {
        let said = output(self.command(&self.places.exe).arg("--version"), "--version")?;
        if said.trim() != format!("{BINARY} {VERSION}") {
            return Err(format!("--version said {said:?}"));
        }
        succeeds(
            self.command(&self.places.exe).arg("--quit"),
            "--quit with nothing running",
        )?;
        if self.data.join(INSTANCE_LOCK_FILE).exists() {
            return Err("--quit with nothing running wrote into the data folder".to_owned());
        }
        println!("install-test: the installed app runs");
        Ok(())
    }

    /// Writes the login item as the app writes it, and starts the app for the uninstall to
    /// quit, when this machine can show it: the app, if it stayed open.
    fn start_app(&self) -> Result<Option<Child>, String> {
        let quoted = format!("\"{}\"", self.places.exe.display());
        reg(&[
            "add",
            RUN_KEY,
            "/v",
            WINDOWS_RUN_VALUE,
            "/t",
            "REG_SZ",
            "/d",
            &quoted,
            "/f",
        ])?;
        let mut app = self
            .command(&self.places.exe)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("starting the app: {err}"))?;
        std::thread::sleep(START_WAIT);
        if app.try_wait().map_err(|err| err.to_string())?.is_none() {
            println!("install-test: the app is running; the uninstall must quit it");
            Ok(Some(app))
        } else {
            println!("install-test: the app could not stay open here; its quit is not tested");
            Ok(None)
        }
    }

    /// Uninstalls as a script does, and checks that nothing is left. The uninstaller copies
    /// itself away and returns at once, so the folder's going says it is done.
    fn uninstall(&self, app: Option<Child>) -> Result<(), String> {
        succeeds(
            self.command(&self.places.uninstaller).arg("/S"),
            "the uninstaller",
        )?;
        let until = Instant::now() + UNINSTALL_WAIT;
        while self.places.dir.exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(500));
        }
        let mut left = Vec::new();
        for path in [&self.places.dir, &self.places.shortcut] {
            if path.exists() {
                left.push(path.display().to_string());
            }
        }
        if reg(&["query", &uninstall_entry()]).is_ok() {
            left.push(uninstall_entry());
        }
        if reg(&["query", RUN_KEY, "/v", WINDOWS_RUN_VALUE]).is_ok() {
            left.push(format!(r"{RUN_KEY}\{WINDOWS_RUN_VALUE}"));
        }
        if let Some(mut app) = app
            && app.try_wait().map_err(|err| err.to_string())?.is_none()
        {
            let _ = app.kill();
            left.push("the running app".to_owned());
        }
        if !left.is_empty() {
            return Err(format!("the uninstall left {}", left.join(", ")));
        }
        println!("install-test: uninstalled, nothing left; the data folder was the test's own");
        Ok(())
    }
}

fn succeeds(command: &mut Command, what: &str) -> Result<(), String> {
    let status = command.status().map_err(|err| format!("{what}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} ended {status}"))
    }
}

fn output(command: &mut Command, what: &str) -> Result<String, String> {
    let out = command.output().map_err(|err| format!("{what}: {err}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!("{what} ended {}", out.status))
    }
}

/// Runs `reg.exe` with `args`: what it printed, or why it failed.
fn reg(args: &[&str]) -> Result<String, String> {
    output(
        Command::new("reg").args(args),
        &format!("reg {}", args.join(" ")),
    )
}

/// Whether any `repo-terrarium.exe` is running.
fn app_running() -> Result<bool, String> {
    let filter = format!("IMAGENAME eq {BINARY}.exe");
    let listed = output(
        Command::new("tasklist").args(["/FI", &filter, "/NH", "/FO", "CSV"]),
        "tasklist",
    )?;
    Ok(listed.contains(&format!("\"{BINARY}.exe\"")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_places_follow_the_pinned_names_and_the_products() {
        let places = Places::new(
            Path::new(r"C:\Users\someone\AppData\Local"),
            Path::new(r"C:\Users\someone\AppData\Roaming"),
            "Moss Box",
        );
        let shown = |path: &Path| path.display().to_string().replace('\\', "/");
        assert_eq!(
            shown(&places.exe),
            "C:/Users/someone/AppData/Local/Programs/repo-terrarium/repo-terrarium.exe"
        );
        assert!(shown(&places.uninstaller).ends_with("repo-terrarium/uninstall.exe"));
        assert!(shown(&places.shortcut).ends_with("Start Menu/Programs/Moss Box.lnk"));
        assert_eq!(
            uninstall_entry(),
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\io.github.faircat-gd.repo-terrarium"
        );
    }
}
