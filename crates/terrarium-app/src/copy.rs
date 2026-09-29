//! Every user-facing string lives here (I-8). The world never talks: no praise, shame, nagging,
//! instructions, notifications or exclamation marks.

use std::fmt::Display;

use crate::cli::UsageError;

const PRODUCT_NAME: &str = "Repo Terrarium";
/// The binary's name, a machine identifier that never changes.
const BINARY_NAME: &str = "repo-terrarium";
/// The single source of the product version: the workspace manifest.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The `--version` output.
pub fn version_line() -> String {
    format!("{BINARY_NAME} {VERSION}")
}

pub fn window_title() -> String {
    format!("{PRODUCT_NAME} {VERSION}")
}

pub fn usage_error(err: &UsageError) -> String {
    format!(
        "{BINARY_NAME}: unexpected argument '{}'\nusage: {BINARY_NAME} [--version]",
        err.unexpected.to_string_lossy()
    )
}

pub fn window_failed(err: &impl Display) -> String {
    format!("{BINARY_NAME}: the window could not open: {err}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One rendering of every string above. A new string joins this list.
    fn all_copy() -> Vec<String> {
        vec![
            version_line(),
            window_title(),
            usage_error(&UsageError {
                unexpected: "--bogus".into(),
            }),
            window_failed(&"reason"),
        ]
    }

    #[test]
    fn copy_has_no_exclamation_marks() {
        for line in all_copy() {
            assert!(!line.contains('!'), "copy must not exclaim: {line:?}");
        }
    }

    #[test]
    fn the_version_line_and_title_carry_the_workspace_version() {
        assert_eq!(
            version_line(),
            format!("repo-terrarium {}", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            window_title(),
            format!("Repo Terrarium {}", env!("CARGO_PKG_VERSION"))
        );
    }
}
