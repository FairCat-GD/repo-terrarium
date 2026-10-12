//! The time-lapse as an MP4, through the person's own ffmpeg: the app ships no video encoder.
//! Each frame is composed as for the GIF, upscaled here with nearest sampling (I-11), and fed
//! to ffmpeg as raw RGB on its stdin, never through a shell. ffmpeg only turns it into H.264:
//! at 4× or more, with even sides, each art pixel spans whole 2×2 blocks of `yuv420p`'s halved
//! colour, so no colour bleeds across a pixel's edge.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use terrarium_adapters::exec::{FedError, Tool, run_fed};
use terrarium_compose::catalog::Art;
use terrarium_compose::salt::InstallSalt;
use terrarium_compose::scale::{StripScale, upscale_into};
use terrarium_compose::timelapse::Reel;
use terrarium_compose::variants::ArtTable;

use super::Film;
use crate::files::fill_beside;

/// sRGB frames converted with BT.709's matrix into video range, then tagged BT.709 throughout
/// on the frames themselves, which the encoder writes into the stream: players read the
/// colours as they were drawn. ffmpeg takes a frame's own tags over its output options.
const VIDEO_FILTER: &str = "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,\
     setparams=colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv";

/// The smallest scale an MP4 is written at.
pub const MIN_SCALE: u32 = 4;

/// The time-lapse's frames a second: the GIF's 7/7/6 cs cadence, 15 exactly.
pub const FPS: u32 = 15;

/// How long ffmpeg may take before it is stopped: far past what 30 seconds at 8× takes.
pub const LIMIT: Duration = Duration::from_secs(600);

/// The size an MP4 asked for at `scale` is written at.
pub fn size(scale: u32) -> StripScale {
    StripScale::exact(scale.max(MIN_SCALE))
}

/// ffmpeg's arguments for frames of `size` into the file `part`, which must be absolute: named
/// as a `file:` URL, it can be neither a network address nor an option.
///
/// # Panics
/// If `size` has an odd side, which `yuv420p` cannot hold, or `part` is relative.
pub fn ffmpeg_args(size: &StripScale, part: &Path) -> Vec<OsString> {
    assert!(
        size.width.is_multiple_of(2) && size.height.is_multiple_of(2),
        "even sides: {}×{}",
        size.width,
        size.height
    );
    assert!(part.is_absolute(), "an absolute output: {}", part.display());
    let mut output = OsString::from("file:");
    output.push(part);
    let video_size = format!("{}x{}", size.width, size.height);
    let fps = FPS.to_string();
    [
        "-hide_banner",
        "-nostats",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "-video_size",
        &video_size,
        "-framerate",
        &fps,
        "-i",
        "pipe:0",
        "-an",
        "-vf",
        VIDEO_FILTER,
        "-c:v",
        "libx264",
        "-preset",
        "slow",
        "-crf",
        "18",
        "-tune",
        "animation",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
        "-y",
    ]
    .into_iter()
    .map(OsString::from)
    .chain([output])
    .collect()
}

/// Writes `film`'s frames into `out` as raw RGB at `size`, one frame at a time through one
/// buffer. `progress` hears each frame written, in percent.
pub fn feed_frames(
    film: &Film,
    salt: InstallSalt,
    size: &StripScale,
    out: &mut dyn Write,
    progress: &mut dyn FnMut(u8),
) -> io::Result<()> {
    let reel = Reel::new(
        Art::embedded(),
        ArtTable::embedded(),
        salt,
        film.prime.as_ref(),
        &film.shots,
        &film.sampling,
    );
    let frames = reel.len().max(1);
    let mut rgb = vec![[0_u8; 3]; size.pixel_count()];
    for (i, image) in reel.enumerate() {
        upscale_into(&image, size, |[r, g, b, _]| [r, g, b], &mut rgb)
            .expect("a buffer of the frame's size");
        out.write_all(rgb.as_flattened())?;
        progress(u8::try_from((i + 1) * 100 / frames).unwrap_or(100));
    }
    out.flush()
}

/// Writes `film` as an MP4 at `scale` (at least [`MIN_SCALE`]) to `path` through `ffmpeg`,
/// which writes the file beside it in its folder, so `path` is only ever the whole video.
pub fn write_mp4(
    film: &Film,
    salt: InstallSalt,
    scale: u32,
    ffmpeg: &Tool,
    path: &Path,
    progress: &mut dyn FnMut(u8),
) -> Result<(), FedError> {
    let size = size(scale);
    let path = std::path::absolute(path).map_err(FedError::Feed)?;
    let folder = path.parent().unwrap_or(&path).to_path_buf();
    fill_beside(&path, |part| {
        run_fed(ffmpeg, &ffmpeg_args(&size, part), &folder, LIMIT, |stdin| {
            feed_frames(film, salt, &size, stdin, progress)
        })
    })
}

#[cfg(test)]
mod tests {
    use terrarium_compose::frame::NativeImage;
    use terrarium_compose::timelapse::{FRAMES, SECONDS, frame_ms};

    use super::*;

    #[test]
    fn every_scale_is_written_at_4x_or_more_with_even_sides() {
        for k in 1..=crate::cli::MAX_SCALE {
            let size = size(k);
            assert_eq!(size.scale, k.max(4));
            assert!(size.width >= 2560 && size.width.is_multiple_of(2), "{k}");
            assert!(size.height >= 296 && size.height.is_multiple_of(2), "{k}");
        }
        assert_eq!(
            u64::from(FPS) * u64::from(SECONDS),
            u64::try_from(FRAMES).unwrap()
        );
        assert_eq!(frame_ms(FRAMES), u64::from(SECONDS) * 1000);
    }

    #[test]
    fn ffmpeg_reads_raw_rgb_and_writes_yuv420p_h264_to_a_file_url() {
        let part = std::path::absolute("out/-fern.mp4.part").unwrap();
        let args: Vec<String> = ffmpeg_args(&size(1), &part)
            .into_iter()
            .map(|arg| arg.into_string().unwrap())
            .collect();
        let at = |flag: &str| {
            let i = args.iter().position(|arg| arg == flag).unwrap();
            args[i + 1].as_str()
        };
        assert_eq!(at("-f"), "rawvideo");
        assert_eq!(at("-pix_fmt"), "rgb24");
        assert_eq!(at("-video_size"), "2560x296");
        assert_eq!(at("-framerate"), "15");
        assert_eq!(at("-i"), "pipe:0");
        assert_eq!(
            at("-vf"),
            "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,\
             setparams=colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv"
        );
        assert_eq!(at("-c:v"), "libx264");
        assert_eq!(
            args[args.len() - 3..],
            ["mp4", "-y", &format!("file:{}", part.display())]
        );
        assert!(
            args[..args.len() - 1].iter().all(|arg| !arg.contains(' ')),
            "one word each; only the path may hold spaces"
        );
    }

    #[test]
    #[should_panic(expected = "an absolute output")]
    fn a_relative_output_is_never_handed_to_ffmpeg() {
        ffmpeg_args(&size(4), Path::new("fern.mp4.part"));
    }

    /// Counts what is written and keeps the first frame, never the rest.
    struct Counted {
        bytes: usize,
        first: Vec<u8>,
        frame: usize,
    }

    impl Write for Counted {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let room = self.frame.saturating_sub(self.first.len());
            self.first.extend_from_slice(&buf[..room.min(buf.len())]);
            self.bytes += buf.len();
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_frames_go_out_one_at_a_time_upscaled_as_the_reel_draws_them() {
        let (_dir, lapse) = super::super::tests::arc();
        let closes = lapse.closes();
        let film = Film::new(&lapse, &closes, 255, 275, false).unwrap();
        let size = size(1);
        let frame = size.pixel_count() * 3;
        let mut out = Counted {
            bytes: 0,
            first: Vec::new(),
            frame,
        };
        let mut heard = Vec::new();
        let salt = InstallSalt::new([0; 16]);
        feed_frames(&film, salt, &size, &mut out, &mut |p| heard.push(p)).unwrap();
        assert_eq!(out.bytes, FRAMES * frame);
        assert_eq!((heard.len(), heard.last()), (FRAMES, Some(&100)));
        let first: NativeImage = Reel::new(
            Art::embedded(),
            ArtTable::embedded(),
            salt,
            film.prime.as_ref(),
            &film.shots,
            &film.sampling,
        )
        .next()
        .unwrap();
        let mut expected = vec![[0; 4]; size.pixel_count()];
        upscale_into(&first, &size, |rgba| rgba, &mut expected).unwrap();
        let expected: Vec<u8> = expected.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        assert!(out.first == expected, "the first frame, as RGB at 4×");
    }
}
