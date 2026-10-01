//! `cargo xtask render`: a scene written as PNGs at 1× and 3×, through the same compositor and
//! upscaler as the strip.
//!
//! - `--demo <out.png>` writes the sample scene the strip shows.
//! - `--preset <name> <out.png>` writes one of the composer's presets, settled, by day.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use terrarium_compose::catalog::Art;
use terrarium_compose::compositor::composite;
use terrarium_compose::export;
use terrarium_compose::frame::{Frame, NATIVE_HEIGHT, NATIVE_WIDTH};
use terrarium_compose::scale::{StripScale, upscale_into};
use terrarium_compose::variants::ArtTable;
use terrarium_compose::{preset, sample};

/// The label every picture shows: the binary's name, as the app's copy module gives it to the
/// strip.
const SAMPLE_LABEL: &str = "repo-terrarium";
/// The larger picture, the exact scale INTEGRATION.md names (1920×222).
const LARGE_SCALE: u32 = 3;

pub fn run(out: &Path) -> ExitCode {
    report(render_demo(out))
}

pub fn run_preset(name: &str, out: &Path) -> ExitCode {
    report(render_preset(name, out))
}

fn report(result: Result<[PathBuf; 2], String>) -> ExitCode {
    match result {
        Ok(written) => {
            for path in written {
                println!("render: wrote {}", path.display());
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("render: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Writes the sample by day to `out` at 1× and beside it, `@3x` before the extension, at 3×.
pub fn render_demo(out: &Path) -> Result<[PathBuf; 2], String> {
    let art = Art::embedded();
    let table = table(art)?;
    write(art, &sample::frame(art, &table, SAMPLE_LABEL, 0, 0), out)
}

/// Writes the named preset, as [`render_demo`] writes the sample.
pub fn render_preset(name: &str, out: &Path) -> Result<[PathBuf; 2], String> {
    let art = Art::embedded();
    let table = table(art)?;
    let frame = preset::frame(art, &table, name, SAMPLE_LABEL).ok_or_else(|| {
        format!(
            "no preset is called `{name}`; the presets are {}",
            preset::NAMES.join(", ")
        )
    })?;
    write(art, &frame, out)
}

fn table(art: &Art) -> Result<ArtTable, String> {
    ArtTable::new(&art.catalog).map_err(|gaps| format!("{} art gaps", gaps.len()))
}

/// Composites `frame` and writes it at 1× to `out` and at 3× beside it.
fn write(art: &Art, frame: &Frame, out: &Path) -> Result<[PathBuf; 2], String> {
    let image = composite(art, frame);
    let padding = sample::padding(art, 0, 0);
    let large = StripScale::exact(LARGE_SCALE);
    let mut pixels = vec![[0; 4]; large.pixel_count()];
    upscale_into(&image, &large, &padding, |colour| colour, &mut pixels).map_err(|err| {
        format!(
            "the 3× image is {} pixels, not {}",
            err.actual, err.expected
        )
    })?;
    let outputs = [
        (
            out.to_path_buf(),
            export::png(NATIVE_WIDTH, NATIVE_HEIGHT, image.pixels()),
        ),
        (
            beside(out, LARGE_SCALE),
            export::png(large.width, large.height, &pixels),
        ),
    ];
    for (path, bytes) in &outputs {
        std::fs::write(path, bytes).map_err(|err| format!("{}: {err}", path.display()))?;
    }
    Ok(outputs.map(|(path, _)| path))
}

/// `dir/name.png` becomes `dir/name@3x.png`.
fn beside(out: &Path, scale: u32) -> PathBuf {
    let stem = out.file_stem().unwrap_or_default().to_string_lossy();
    let name = match out.extension() {
        Some(ext) => format!("{stem}@{scale}x.{}", ext.to_string_lossy()),
        None => format!("{stem}@{scale}x"),
    };
    out.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::TempDir;

    fn decode(path: &Path) -> (u32, u32, Vec<[u8; 4]>) {
        export::read_png(&std::fs::read(path).unwrap()).expect("an RGB PNG")
    }

    #[test]
    fn the_3x_picture_is_written_beside_the_1x_one() {
        assert_eq!(
            beside(Path::new("out/demo.png"), 3),
            Path::new("out/demo@3x.png")
        );
        assert_eq!(beside(Path::new("demo"), 3), Path::new("demo@3x"));
    }

    #[test]
    fn a_preset_is_written_with_the_composer_s_pixels() {
        let dir = TempDir::new("render-preset");
        let [small, large] = render_preset("dormant", &dir.0.join("dormant.png")).unwrap();
        assert_eq!(large, dir.0.join("dormant@3x.png"));
        let art = Art::embedded();
        let table = ArtTable::new(&art.catalog).unwrap();
        let frame = preset::frame(art, &table, "dormant", SAMPLE_LABEL).unwrap();
        let (w, h, pixels) = decode(&small);
        assert_eq!((w, h), (640, 74));
        assert_eq!(pixels, composite(art, &frame).pixels());
        assert_eq!(decode(&large).0, 1920);
    }

    #[test]
    fn an_unknown_preset_is_refused_with_the_names() {
        let dir = TempDir::new("render-unknown");
        let out = dir.0.join("lush.png");
        let err = render_preset("lush", &out).unwrap_err();
        assert!(
            err.contains("`lush`") && err.contains("struggling-poor-soil"),
            "{err}"
        );
        assert!(!out.exists());
    }

    #[test]
    fn the_demo_is_written_at_1x_and_3x_with_the_composited_pixels() {
        let dir = TempDir::new("render-demo");
        let [small, large] = render_demo(&dir.0.join("demo.png")).unwrap();
        assert_eq!(large, dir.0.join("demo@3x.png"));

        let art = Art::embedded();
        let table = ArtTable::new(&art.catalog).unwrap();
        let image = composite(art, &sample::frame(art, &table, SAMPLE_LABEL, 0, 0));
        let (w, h, pixels) = decode(&small);
        assert_eq!((w, h), (640, 74));
        assert_eq!(pixels, image.pixels());
        let (w, h, pixels) = decode(&large);
        assert_eq!((w, h), (1920, 222));
        // Native (x, y) fills the 3×3 block at (3x, 3y).
        for (x, y) in [(0, 0), (100, 40), (639, 73)] {
            let block = pixels[(3 * y * 1920 + 3 * x + 2 * 1920 + 2) as usize];
            assert_eq!(block, image.pixel(x, y));
        }
    }
}
