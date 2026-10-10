//! Opening at login: `SMAppService` on macOS, the per-user `Run` key on Windows.
//!
//! On macOS only an app bundle can be a login item. A binary run from a build folder, as
//! `cargo run` runs it, is not one, so it is never registered; it says so, and nothing is
//! touched. Only a packaged app exercises the registration itself.

use std::fmt;

/// Whether the app opens at login, as the OS has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginStatus {
    Enabled,
    Disabled,
    /// Registered, but waiting for the person to allow it in the OS's settings (macOS).
    RequiresApproval,
    /// This build cannot be a login item: it is not an app bundle (macOS).
    Unavailable,
}

/// Why the app could not be made to open at login, or stop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginError {
    /// Not an app bundle with `app_id`, so nothing was registered (macOS).
    NotBundled,
    /// The OS refused, in its own words.
    Os(String),
}

impl fmt::Display for LoginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoginError::NotBundled => f.write_str("not an app bundle"),
            LoginError::Os(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for LoginError {}

/// Whether the app with `app_id` (the bundle id) opens at login.
pub fn status(app_id: &str) -> LoginStatus {
    imp::status(app_id)
}

/// Makes the app with `app_id` open at login, or stop.
pub fn set(app_id: &str, on: bool) -> Result<(), LoginError> {
    imp::set(app_id, on)
}

/// The `Run` value's data: the executable's path, quoted, so a path with spaces is one
/// command.
#[cfg_attr(not(windows), allow(dead_code))]
fn run_command(exe: &std::path::Path) -> String {
    format!("\"{}\"", exe.display())
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2_foundation::NSBundle;
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    use super::{LoginError, LoginStatus};

    /// Whether this process is the app bundle `app_id` names.
    fn bundled(app_id: &str) -> bool {
        NSBundle::mainBundle()
            .bundleIdentifier()
            .is_some_and(|id| id.to_string() == app_id)
    }

    pub fn status(app_id: &str) -> LoginStatus {
        if !bundled(app_id) {
            return LoginStatus::Unavailable;
        }
        // SAFETY: `mainAppService` takes nothing and returns the main app's service, which
        // exists for every app bundle on macOS 13 and later, the app's floor.
        let service = unsafe { SMAppService::mainAppService() };
        // SAFETY: `status` only reads the service's registration.
        match unsafe { service.status() } {
            SMAppServiceStatus::Enabled => LoginStatus::Enabled,
            SMAppServiceStatus::RequiresApproval => LoginStatus::RequiresApproval,
            _ => LoginStatus::Disabled,
        }
    }

    pub fn set(app_id: &str, on: bool) -> Result<(), LoginError> {
        if !bundled(app_id) {
            return Err(LoginError::NotBundled);
        }
        // SAFETY: as in `status`.
        let service = unsafe { SMAppService::mainAppService() };
        let done = if on {
            // SAFETY: registering the main app takes no arguments; a failure comes back as
            // the error.
            unsafe { service.registerAndReturnError() }
        } else {
            // SAFETY: as registering.
            unsafe { service.unregisterAndReturnError() }
        };
        done.map_err(|err| LoginError::Os(err.localizedDescription().to_string()))
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };
    use windows::core::{HSTRING, PCWSTR};

    use super::{LoginError, LoginStatus, run_command};

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    fn command() -> Result<String, LoginError> {
        std::env::current_exe()
            .map(|exe| run_command(&exe))
            .map_err(|err| LoginError::Os(err.to_string()))
    }

    pub fn status(app_id: &str) -> LoginStatus {
        let key = HSTRING::from(RUN_KEY);
        let name = HSTRING::from(app_id);
        // SAFETY: the key and value names are live, NUL-terminated strings; with no buffer
        // the call only says whether the value exists.
        let found = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                None,
                None,
            )
        };
        if found == ERROR_SUCCESS {
            LoginStatus::Enabled
        } else {
            LoginStatus::Disabled
        }
    }

    pub fn set(app_id: &str, on: bool) -> Result<(), LoginError> {
        let key = HSTRING::from(RUN_KEY);
        let name = HSTRING::from(app_id);
        let done = if on {
            let command = HSTRING::from(command()?);
            // The value's bytes: its UTF-16 text with the terminating NUL, as `REG_SZ` holds it.
            let bytes = u32::try_from((command.len() + 1) * 2)
                .map_err(|err| LoginError::Os(err.to_string()))?;
            // SAFETY: the key and value names are live, NUL-terminated strings, and `bytes`
            // covers `command`'s text and its terminating NUL, which `HSTRING` keeps.
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    PCWSTR(key.as_ptr()),
                    PCWSTR(name.as_ptr()),
                    REG_SZ.0,
                    Some(command.as_ptr().cast()),
                    bytes,
                )
            }
        } else {
            // SAFETY: the key and value names are live, NUL-terminated strings.
            let deleted = unsafe {
                RegDeleteKeyValueW(
                    HKEY_CURRENT_USER,
                    PCWSTR(key.as_ptr()),
                    PCWSTR(name.as_ptr()),
                )
            };
            // Nothing to delete is as good as deleted.
            if deleted == ERROR_FILE_NOT_FOUND {
                ERROR_SUCCESS
            } else {
                deleted
            }
        };
        if done == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(LoginError::Os(
                windows::core::Error::from(done.to_hresult()).message(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_value_quotes_the_executable() {
        assert_eq!(
            run_command(std::path::Path::new(
                r"C:\Program Files\Repo Terrarium\repo-terrarium.exe"
            )),
            r#""C:\Program Files\Repo Terrarium\repo-terrarium.exe""#
        );
    }

    /// A test binary is no app bundle, so it never registers anything: only the status is
    /// asked, which reads and changes nothing.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_build_that_is_no_app_bundle_is_never_a_login_item() {
        assert_eq!(
            status("io.github.faircat-gd.repo-terrarium"),
            LoginStatus::Unavailable
        );
    }
}
