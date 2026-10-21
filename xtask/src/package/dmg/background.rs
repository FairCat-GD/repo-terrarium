//! The DMG window's picture: the two PNGs in `packaging/dmg/`, one at the window's size in
//! points and one at twice that for Retina displays, each drawn on its own. They are checked
//! and combined as delivered into the one TIFF Finder shows. `packaging/dmg/README.md` says
//! what to deliver.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::WINDOW;
use crate::package::icon;

/// Where the pictures are, from the workspace root.
pub const DIR: &str = "packaging/dmg";

/// The two pictures, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub one: PathBuf,
    pub two: PathBuf,
}

/// Finds and checks the pictures in `root`.
pub fn load(root: &Path) -> Result<Picture, String> {
    let dir = root.join(DIR);
    let (width, height) = WINDOW;
    let read = |name: &str, scale: u32| -> Result<PathBuf, String> {
        let path = dir.join(name);
        let bytes = std::fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        let (w, h, rgba) =
            icon::decode_rgba(&bytes).map_err(|err| format!("{}: {err}", path.display()))?;
        check(w, h, &rgba, (width * scale, height * scale))
            .map_err(|err| format!("{}: {err}", path.display()))?;
        Ok(path)
    };
    Ok(Picture {
        one: read("background.png", 1)?,
        two: read("background@2x.png", 2)?,
    })
}

/// Whether a picture is `size` and opaque: Finder would stretch one of another size, and show
/// its own white through a pixel that is not opaque.
pub fn check(width: u32, height: u32, rgba: &[u8], size: (u32, u32)) -> Result<(), String> {
    if (width, height) != size {
        return Err(format!("is {width}×{height}, not {}×{}", size.0, size.1));
    }
    if let Some(at) = rgba.as_chunks::<4>().0.iter().position(|px| px[3] != 255) {
        let at = u32::try_from(at).unwrap_or(u32::MAX);
        return Err(format!(
            "the pixel at {},{} is not opaque",
            at % width,
            at / width
        ));
    }
    Ok(())
}

/// The window's TIFF, written to `out`: the two pictures combined by macOS's `tiffutil`, which
/// checks the second is twice the first, so Finder shows the sharp one on a Retina display.
pub fn tiff(picture: &Picture, out: &Path) -> Result<(), String> {
    super::run(
        Command::new("tiffutil")
            .arg("-cathidpicheck")
            .arg(&picture.one)
            .arg(&picture.two)
            .arg("-out")
            .arg(out),
        "tiffutil -cathidpicheck",
    )
    .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(width: u32, height: u32) -> Vec<u8> {
        [243, 239, 223, 255].repeat(usize::try_from(width * height).unwrap())
    }

    #[test]
    fn each_picture_is_its_size_and_opaque() {
        assert_eq!(check(660, 400, &plain(660, 400), (660, 400)), Ok(()));
        assert_eq!(check(1320, 800, &plain(1320, 800), (1320, 800)), Ok(()));
        assert_eq!(
            check(330, 200, &plain(330, 200), (660, 400)),
            Err("is 330×200, not 660×400".to_owned())
        );
        let mut holed = plain(660, 400);
        holed[(5 * 660 + 7) * 4 + 3] = 254;
        assert_eq!(
            check(660, 400, &holed, (660, 400)),
            Err("the pixel at 7,5 is not opaque".to_owned())
        );
    }
}
