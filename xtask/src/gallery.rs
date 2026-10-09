//! `cargo xtask gallery`: the A-kit's pictures, written to `target/gallery/` (under
//! `CARGO_TARGET_DIR` when it is set, as cargo's own output is).
//!
//! - `states/NN-day.png` and `NN-night.png`: the 30 states of the synthetic climate's health ×
//!   activity grid, at 3×. `NN` is a fixed shuffle of the grid, so a viewer rating them cannot
//!   read the answer from the order; `key.csv` holds it.
//! - `named/<name>-day.png`, `-night.png` and `-dim.png`: the named states at 3×, Recovering
//!   among them, which the grid cannot hold.
//! - `contact-sheet.png`: the grid at 1×, health falling down the rows and activity rising
//!   along the columns, each cell's day above its night.
//! - `pieces/`: the background art on its own, for art review (`pieces.rs`).
//! - `SHA256SUMS`: every other file's hash. The pictures are composed with the presets' salt
//!   and date, so the same build always writes the same bytes.

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use terrarium_compose::catalog::Art;
use terrarium_compose::compositor::composite;
use terrarium_compose::export;
use terrarium_compose::frame::{NATIVE_HEIGHT, NATIVE_WIDTH, NativeImage};
use terrarium_compose::synthetic::{self, Cell};
use terrarium_compose::variants::ArtTable;

use crate::pieces::{self, BACKDROP};
use crate::render::{LABEL, large_png};
use crate::sweep;

/// The contact sheet's gutter between cells, and between a cell's day and night.
pub const GUTTER: u32 = 4;
const GAP: u32 = 2;

/// The gallery's files, by path under its directory, in the order written.
pub type Files = Vec<(String, Vec<u8>)>;

pub fn run(root: &Path) -> ExitCode {
    let dir = sweep::target_dir(root).join("gallery");
    match render().and_then(|files| write(&dir, &files).map(|()| files.len())) {
        Ok(count) => {
            println!("gallery: wrote {count} files to {}", dir.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("gallery: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Writes `files` into `dir`, after removing whatever an earlier run left there.
pub fn write(dir: &Path, files: &Files) -> Result<(), String> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    }
    for (name, bytes) in files {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        std::fs::write(&path, bytes).map_err(|err| format!("{}: {err}", path.display()))?;
    }
    Ok(())
}

/// The number a grid cell is shown under: a fixed shuffle of the 30 positions.
pub fn number(index: usize, cells: usize) -> usize {
    (index * 7 + 3) % cells + 1
}

/// Every file of the gallery, rendered in memory.
pub fn render() -> Result<Files, String> {
    let art = Art::embedded();
    let table = ArtTable::new(&art.catalog).map_err(|gaps| format!("{} art gaps", gaps.len()))?;
    let cells = synthetic::gallery();
    let mut files = Files::new();
    let mut key = String::from("number,health,activity,state\n");
    let mut sheet = Vec::with_capacity(cells.len());
    for (index, cell) in cells.iter().enumerate() {
        let shown = number(index, cells.len());
        let day = compose(art, &table, cell, 0, false);
        let night = compose(art, &table, cell, 255, false);
        files.push((format!("states/{shown:02}-day.png"), large_png(&day)?));
        files.push((format!("states/{shown:02}-night.png"), large_png(&night)?));
        let activity = cell
            .activity
            .map_or_else(|| "inactive".to_owned(), |a| a.to_string());
        let _ = writeln!(
            key,
            "{shown:02},{},{activity},{}",
            cell.health,
            cell.knobs.state().name().to_ascii_lowercase()
        );
        sheet.push((cell.row, cell.column, day, night));
    }
    for (name, knobs) in synthetic::NAMED {
        for (suffix, mix, dim) in [("day", 0, false), ("night", 255, false), ("dim", 0, true)] {
            let frame = synthetic::frame(art, &table, &knobs, LABEL, mix, dim);
            let image = composite(art, &frame);
            files.push((format!("named/{name}-{suffix}.png"), large_png(&image)?));
        }
    }
    files.extend(pieces::render(art, &table, LABEL)?);
    files.push(("key.csv".to_owned(), key.into_bytes()));
    files.push(("contact-sheet.png".to_owned(), contact_sheet(&sheet)));
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut sums = String::new();
    for (name, bytes) in &files {
        let _ = writeln!(sums, "{}  {name}", crate::sha256_hex(bytes));
    }
    files.push(("SHA256SUMS".to_owned(), sums.into_bytes()));
    Ok(files)
}

/// A grid cell's terrarium, settled, by day or by night.
fn compose(art: &Art, table: &ArtTable, cell: &Cell, mix: u8, dim: bool) -> NativeImage {
    composite(
        art,
        &synthetic::frame(art, table, &cell.knobs, LABEL, mix, dim),
    )
}

/// The grid at 1×: each cell's day above its night, in grid order.
fn contact_sheet(cells: &[(usize, usize, NativeImage, NativeImage)]) -> Vec<u8> {
    let columns = u32::try_from(synthetic::GALLERY_ACTIVITY.len()).unwrap_or(1);
    let rows = u32::try_from(synthetic::GALLERY_HEALTH.len()).unwrap_or(1);
    let cell_height = 2 * NATIVE_HEIGHT + GAP;
    let width = columns * NATIVE_WIDTH + (columns + 1) * GUTTER;
    let height = rows * cell_height + (rows + 1) * GUTTER;
    let mut pixels = vec![BACKDROP; usize::try_from(width * height).unwrap_or(0)];
    for (row, column, day, night) in cells {
        let x0 = GUTTER + u32::try_from(*column).unwrap_or(0) * (NATIVE_WIDTH + GUTTER);
        let y0 = GUTTER + u32::try_from(*row).unwrap_or(0) * (cell_height + GUTTER);
        for (image, top) in [(day, y0), (night, y0 + NATIVE_HEIGHT + GAP)] {
            for y in 0..NATIVE_HEIGHT {
                for x in 0..NATIVE_WIDTH {
                    let at = usize::try_from((top + y) * width + x0 + x).unwrap_or(0);
                    pixels[at] = image.pixel(x, y);
                }
            }
        }
    }
    export::png(width, height, &pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shuffle_numbers_every_cell_once() {
        let mut numbers: Vec<usize> = (0..30).map(|i| number(i, 30)).collect();
        numbers.sort_unstable();
        assert_eq!(numbers, (1..=30).collect::<Vec<_>>());
        assert_ne!(number(0, 30), 1, "the shuffle keeps grid order");
    }

    #[test]
    fn the_gallery_is_the_same_bytes_every_time_and_holds_every_picture() {
        let first = render().unwrap();
        let second = render().unwrap();
        assert!(first == second, "two renders differ");
        let named = |prefix: &str| first.iter().filter(|(n, _)| n.starts_with(prefix)).count();
        assert_eq!(named("states/"), 60);
        assert_eq!(named("named/"), 21);
        assert_eq!(named("pieces/"), 21);
        let key = &first.iter().find(|(n, _)| n == "key.csv").unwrap().1;
        assert_eq!(String::from_utf8_lossy(key).lines().count(), 31);
        let sums = &first.iter().find(|(n, _)| n == "SHA256SUMS").unwrap().1;
        assert_eq!(
            String::from_utf8_lossy(sums).lines().count(),
            first.len() - 1
        );

        let sheet = &first
            .iter()
            .find(|(n, _)| n == "contact-sheet.png")
            .unwrap()
            .1;
        let (w, h, _) = export::read_png(sheet).unwrap();
        assert_eq!((w, h), (5 * 640 + 6 * GUTTER, 6 * 150 + 7 * GUTTER));
        let (w, h, _) = export::read_png(&first[first.len() - 2].1).unwrap();
        assert_eq!((w, h), (1920, 222));
    }

    #[test]
    fn a_rerun_replaces_what_an_earlier_one_left() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("stale.png"), b"old").unwrap();
        let files = vec![("states/01-day.png".to_owned(), b"new".to_vec())];
        write(dir.path(), &files).unwrap();
        assert!(!dir.path().join("stale.png").exists());
        assert_eq!(
            std::fs::read(dir.path().join("states/01-day.png")).unwrap(),
            b"new"
        );
    }
}
