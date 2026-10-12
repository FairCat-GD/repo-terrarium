//! `replay <path>`: a headless replay, printed as truth's canonical dump (`--dump`) or written as
//! the whole history's time-lapse (`--gif <file>`, or `--mp4 <file>` through the person's
//! ffmpeg).
//!
//! It opens no window. Its store lives in a temporary directory that is removed when it ends,
//! so it never reads or writes the data directory.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::ExitCode;

use terrarium_adapters::exec::find_ffmpeg;

use terrarium_compose::salt::InstallSalt;
use terrarium_engine::headless::{Request, lapse, replay_dump};
use terrarium_store::DataDir;
use terrarium_truth::canonical_dump;

use crate::cli::{ReplayArgs, ReplayOutput};
use crate::timelapse::mp4::write_mp4;
use crate::timelapse::{Film, write_gif};
use crate::{copy, now_secs, output};

/// The salt of every headless replay: the same file on every machine.
const SALT: [u8; 16] = [0; 16];

pub fn run(args: &ReplayArgs) -> ExitCode {
    let done = match &args.output {
        ReplayOutput::Dump => {
            dump(args).map(|text| output::print(&text, copy::replay_write_failed))
        }
        ReplayOutput::Gif { path, night, scale } => {
            gif(args, path, *night, *scale).map(|()| ExitCode::SUCCESS)
        }
        ReplayOutput::Mp4 { path, night, scale } => {
            mp4(args, path, *night, *scale).map(|()| ExitCode::SUCCESS)
        }
    };
    done.unwrap_or_else(|message| {
        eprintln!("{message}");
        ExitCode::FAILURE
    })
}

/// The request for `args`, with the observations read from their file.
fn request<'a>(
    args: &'a ReplayArgs,
    data: &'a DataDir,
    observations: Option<&'a mut BufReader<File>>,
) -> Request<'a> {
    Request {
        repo: &args.repo,
        data,
        observations: observations.map(|file| file as &mut dyn BufRead),
        at_secs: args.at.unwrap_or_else(now_secs),
        install_salt: SALT,
    }
}

fn observations(args: &ReplayArgs) -> Result<Option<BufReader<File>>, String> {
    args.observations
        .as_ref()
        .map(|path| {
            File::open(path)
                .map(BufReader::new)
                .map_err(|err| copy::replay_observations_unreadable(path, &err))
        })
        .transpose()
}

fn dump(args: &ReplayArgs) -> Result<String, String> {
    let temp = tempfile::tempdir().map_err(|err| copy::replay_store_unavailable(&err))?;
    let data = DataDir::at(temp.path());
    let mut observations = observations(args)?;
    let dump = replay_dump(request(args, &data, observations.as_mut()))
        .map_err(|err| copy::replay_failed(&args.repo, &err))?;
    Ok(canonical_dump(&dump))
}

/// The whole history's time-lapse, by day or by `night`.
fn film(args: &ReplayArgs, night: bool) -> Result<Film, String> {
    let temp = tempfile::tempdir().map_err(|err| copy::replay_store_unavailable(&err))?;
    let data = DataDir::at(temp.path());
    let mut observations = observations(args)?;
    let lapse = lapse(request(args, &data, observations.as_mut()))
        .map_err(|err| copy::replay_failed(&args.repo, &err))?;
    let closes = lapse.closes();
    let Some(last) = closes.len().checked_sub(1) else {
        return Err(copy::replay_no_day(&args.repo));
    };
    Film::new(&lapse, &closes, 0, last, night)
        .map_err(|err| copy::replay_failed(&args.repo, &err.into()))
}

fn gif(args: &ReplayArgs, path: &Path, night: bool, scale: u32) -> Result<(), String> {
    let film = film(args, night)?;
    write_gif(&film, InstallSalt::new(SALT), scale, path, &mut |_| {})
        .map_err(|err| copy::replay_gif_failed(path, &err))
}

/// The MP4, through the person's ffmpeg, looked for before the history is read, and never in
/// the repository.
fn mp4(args: &ReplayArgs, path: &Path, night: bool, scale: u32) -> Result<(), String> {
    let ffmpeg = find_ffmpeg(Some(&args.repo)).ok_or_else(copy::replay_no_ffmpeg)?;
    let film = film(args, night)?;
    write_mp4(
        &film,
        InstallSalt::new(SALT),
        scale,
        &ffmpeg,
        path,
        &mut |_| {},
    )
    .map_err(|err| copy::replay_mp4_failed(path, &err))
}
