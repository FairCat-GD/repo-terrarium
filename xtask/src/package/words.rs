//! The words the installers carry. They live in the app's copy module (I-8): the app prints
//! them (`--package-copy`) and this reads them, as `probe-zip` reads the probe's README.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

#[derive(Debug, PartialEq, Eq)]
pub struct Words {
    /// The product's name: the Mac app's, the Start menu's and the installed apps' list's.
    pub product: String,
    /// Who publishes it, in the Windows list of installed apps and the exe's details.
    pub publisher: String,
    /// The licences, as the file details' copyright line.
    pub licence: String,
    /// The reason macOS shows when the app first reads a folder it guards.
    pub folder_reason: String,
    /// What the Windows installer says when the strip will not quit.
    pub still_running: String,
}

/// Runs the app's `--package-copy` and reads what it prints.
pub fn read(root: &Path) -> Result<Words, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let out = Command::new(&cargo)
        .args(["run", "--quiet", "--locked", "-p", "terrarium-app", "--"])
        .arg("--package-copy")
        .current_dir(root)
        .output()
        .map_err(|err| format!("repo-terrarium --package-copy: {err}"))?;
    if !out.status.success() {
        return Err("repo-terrarium --package-copy failed".to_owned());
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// `key<tab>value` lines, each key once. A value is refused if the installer script would read
/// any of it as its own (`$`, `"`), or if it is empty.
fn parse(text: &str) -> Result<Words, String> {
    let value = |key: &str| -> Result<String, String> {
        let mut found = text
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .filter(|(name, _)| *name == key)
            .map(|(_, value)| value);
        let value = found
            .next()
            .ok_or_else(|| format!("--package-copy printed no {key}"))?;
        if found.next().is_some() {
            return Err(format!("--package-copy printed {key} twice"));
        }
        if value.is_empty() || value.contains(['$', '"']) {
            return Err(format!("--package-copy's {key} cannot be used: {value:?}"));
        }
        Ok(value.to_owned())
    };
    Ok(Words {
        product: value("product")?,
        publisher: value("publisher")?,
        licence: value("licence")?,
        folder_reason: value("folder-reason")?,
        still_running: value("still-running")?,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn words() -> Words {
        Words {
            product: "Moss Box".to_owned(),
            publisher: "Example Garden".to_owned(),
            licence: "Code MIT; art CC0".to_owned(),
            folder_reason: "Moss Box reads what you connected.".to_owned(),
            still_running: "Moss Box is still running.".to_owned(),
        }
    }

    #[test]
    fn each_word_is_read_from_its_line() {
        let text = "product\tMoss Box\npublisher\tExample Garden\nlicence\tCode MIT; art CC0\n\
                    folder-reason\tMoss Box reads what you connected.\n\
                    still-running\tMoss Box is still running.\n";
        assert_eq!(parse(text), Ok(words()));
        assert!(
            parse("product\tMoss Box\n")
                .unwrap_err()
                .contains("publisher")
        );
        let twice = format!("{text}product\tOther\n");
        assert!(parse(&twice).unwrap_err().contains("twice"));
        let dollar = text.replace("Example Garden", "$INSTDIR");
        assert!(parse(&dollar).unwrap_err().contains("publisher"));
    }
}
