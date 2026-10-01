//! `cargo xtask render --demo <out.png>`: the sample scene the strip shows, written as PNGs at
//! 1× and 3×, through the same compositor and upscaler as the strip.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use terrarium_compose::catalog::Art;
use terrarium_compose::compositor::composite;
use terrarium_compose::export;
use terrarium_compose::frame::{NATIVE_HEIGHT, NATIVE_WIDTH};
use terrarium_compose::sample;
use terrarium_compose::scale::{StripScale, upscale_into};
use terrarium_compose::variants::ArtTable;

/// The sample's label: the binary's name, as the app's copy module gives it to the strip.
const SAMPLE_LABEL: &str = "repo-terrarium";
/// The larger picture, the exact scale INTEGRATION.md names (1920×222).
const LARGE_SCALE: u32 = 3;

pub fn run(out: &Path) -> ExitCode {
    match render_demo(out) {
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
    let table = ArtTable::new(&art.catalog).map_err(|gaps| format!("{} art gaps", gaps.len()))?;
    let image = composite(art, &sample::frame(art, &table, SAMPLE_LABEL, 0, 0));
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

    /// A directory the test owns under the system temp dir, removed when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let path = std::env::temp_dir().join(format!(
                "terrarium-render-test-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("the temp dir is writable");
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn decode(path: &Path) -> (u32, u32, Vec<[u8; 3]>) {
        let file = std::fs::File::open(path).unwrap();
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .unwrap();
        let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buffer).unwrap();
        let (rgb, _) = buffer[..info.buffer_size()].as_chunks::<3>();
        (info.width, info.height, rgb.to_vec())
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
    fn the_demo_is_written_at_1x_and_3x_with_the_composited_pixels() {
        let dir = TempDir::new("demo");
        let [small, large] = render_demo(&dir.0.join("demo.png")).unwrap();
        assert_eq!(large, dir.0.join("demo@3x.png"));

        let art = Art::embedded();
        let table = ArtTable::new(&art.catalog).unwrap();
        let image = composite(art, &sample::frame(art, &table, SAMPLE_LABEL, 0, 0));
        let (w, h, pixels) = decode(&small);
        assert_eq!((w, h), (640, 74));
        for (got, want) in pixels.iter().zip(image.pixels()) {
            assert_eq!(got[..], want[..3]);
        }
        let (w, h, pixels) = decode(&large);
        assert_eq!((w, h), (1920, 222));
        // Native (x, y) fills the 3×3 block at (3x, 3y).
        for (x, y) in [(0, 0), (100, 40), (639, 73)] {
            let block = pixels[(3 * y * 1920 + 3 * x + 2 * 1920 + 2) as usize];
            assert_eq!(block[..], image.pixel(x, y)[..3]);
        }
    }
}
