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

    /// Every string literal in a source file, tests included, so a string anywhere in this module
    /// is checked; a list of rendered strings would miss one nobody added to it.
    fn string_literals(source: &str) -> Vec<String> {
        let mut literals = Vec::new();
        let mut chars = source.chars();
        while let Some(c) = chars.next() {
            match c {
                // A comment runs to the end of its line.
                '/' if chars.clone().next() == Some('/') => {
                    chars.by_ref().find(|&c| c == '\n');
                }
                // A char literal (`'"'`, `'\\''`) is skipped; a lifetime (`'static`) is not one.
                '\'' => {
                    let mut ahead = chars.clone();
                    match (ahead.next(), ahead.next()) {
                        (Some('\\'), _) => {
                            chars.nth(1);
                            chars.by_ref().find(|&c| c == '\'');
                        }
                        (Some(_), Some('\'')) => {
                            chars.nth(1);
                        }
                        _ => {}
                    }
                }
                '"' => {
                    let mut literal = String::new();
                    while let Some(c) = chars.next() {
                        match c {
                            '\\' => literal.extend(chars.next()),
                            '"' => break,
                            _ => literal.push(c),
                        }
                    }
                    literals.push(literal);
                }
                _ => {}
            }
        }
        literals
    }

    #[test]
    fn copy_has_no_exclamation_marks() {
        let literals = string_literals(include_str!("copy.rs"));
        assert!(
            literals.iter().any(|l| l == "Repo Terrarium"),
            "the scan found {literals:?}"
        );
        for literal in literals {
            assert!(!literal.contains('!'), "copy must not exclaim: {literal:?}");
        }
    }

    #[test]
    fn the_scan_finds_literals_and_skips_comments() {
        let source = "// a comment: \"skipped\"\nconst A: &'static str = \"one\";\nfn f() { g(\"x \\\"y\\\" {}\", 1) }\nconst Q: [char; 2] = ['\"', '\\''];";
        assert_eq!(string_literals(source), ["one", "x \"y\" {}"]);
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
