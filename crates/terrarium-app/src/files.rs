//! Every file the app writes for the person (the time-lapse's exports, the data exports) is
//! written beside its path and takes the name only once whole, so a path is never half a file
//! and a failure leaves what was there.

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};

/// Writes `path` through `write`, into a file beside it that takes its name only once whole;
/// a failure leaves `path` as it was and removes what was begun. `write`'s own error comes
/// back as it is.
pub fn write_beside<E: From<io::Error>>(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> Result<(), E>,
) -> Result<(), E> {
    fill_beside(path, |part| {
        let mut file = BufWriter::new(File::create(part)?);
        write(&mut file)?;
        Ok(file
            .into_inner()
            .map_err(io::IntoInnerError::into_error)?
            .sync_all()?)
    })
}

/// Has `fill` write the file beside `path`, as a tool writing its own output does, then gives
/// it `path`'s name; a failure leaves `path` as it was and removes what was begun.
pub fn fill_beside<E: From<io::Error>>(
    path: &Path,
    fill: impl FnOnce(&Path) -> Result<(), E>,
) -> Result<(), E> {
    let part = beside(path);
    match fill(&part).and_then(|()| Ok(std::fs::rename(&part, path)?)) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&part);
            Err(err)
        }
    }
}

/// The file an export is written into before it takes its name: `<name>.part`, beside it.
pub fn beside(path: &Path) -> PathBuf {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    PathBuf::from(part)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[test]
    fn a_write_that_fails_leaves_the_path_as_it_was_and_nothing_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fern.png");
        std::fs::write(&path, b"before").unwrap();
        let failed = write_beside(&path, |_| Err(io::Error::other("full")));
        assert_eq!(
            failed.unwrap_err().to_string(),
            "full",
            "the writer's own error"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"before");
        assert!(!beside(&path).exists());
        let missing = dir.path().join("gone").join("fern.png");
        assert!(write_beside::<io::Error>(&missing, |file| file.write_all(b"after")).is_err());
        assert!(!beside(&missing).exists());
        write_beside::<io::Error>(&path, |file| file.write_all(b"after")).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"after");
    }

    #[test]
    fn a_tool_s_failure_after_writing_part_of_its_file_leaves_nothing_beside_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fern.mp4");
        let failed: io::Result<()> = fill_beside(&path, |part| {
            std::fs::write(part, b"half")?;
            Err(io::Error::other("the tool stopped"))
        });
        assert!(failed.is_err());
        assert!(!path.exists() && !beside(&path).exists());
        fill_beside::<io::Error>(&path, |part| std::fs::write(part, b"whole")).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"whole");
        assert!(!beside(&path).exists());
    }
}
