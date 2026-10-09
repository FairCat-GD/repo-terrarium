//! `cargo xtask arc`: the fixture arc replayed a day at a time and drawn in key frames, for a
//! review of how it looks, written to `target/arc/`.
//!
//! The arc is built in a temporary directory as `golden` builds it, and each day's scene, with
//! the event the live view would play, goes to one composer in turn, which settles between days,
//! so what stands where carries over as it does on the strip. A frame is kept, settled, at each
//! change of the presentation timeline, on each day an event plays on the ground, and on the
//! last day:
//!
//! - `frames/NN-day.png` and `NN-night.png`: each kept frame at 3×;
//! - `contact-sheet.png`: every kept frame at 1×, its day beside its night, in order;
//! - `key.csv`: each frame's day of the arc, its tick, presentation state, health and activity
//!   class, and the events played since the frame before;
//! - `SHA256SUMS`: every other file's hash. The same build always writes the same bytes.
//!
//! It fails unless the frames show all six presentation states.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::ExitCode;

use terrarium_adapters::fixture;
use terrarium_compose::catalog::Art;
use terrarium_compose::clock::LocalClock;
use terrarium_compose::composer::{Composer, OverlayInput};
use terrarium_compose::compositor::composite;
use terrarium_compose::export;
use terrarium_compose::frame::{NATIVE_HEIGHT, NATIVE_WIDTH, NativeImage};
use terrarium_compose::preset;
use terrarium_compose::tween::Millis;
use terrarium_compose::variants::ArtTable;
use terrarium_engine::headless::{Days, Request, daily_scenes};
use terrarium_store::DataDir;
use terrarium_truth::scene::{EventKind, SceneInput};
use terrarium_truth::timeline::TimelineEntry;
use terrarium_truth::{PresentationState, Tick};

use crate::gallery::{self, Files, GUTTER};
use crate::pieces::BACKDROP;
use crate::render::{LABEL, large_png};
use crate::sweep;

pub fn run(root: &Path) -> ExitCode {
    let dir = sweep::target_dir(root).join("arc");
    match render(root).and_then(|files| gallery::write(&dir, &files).map(|()| files.len())) {
        Ok(count) => {
            println!("arc: wrote {count} files to {}", dir.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("arc: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The arc's days, replayed.
fn replay(root: &Path) -> Result<Days, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let repo = dir.path().join("repo");
    fixture::arc(&repo).map_err(|e| e.to_string())?;
    let observations = root.join(crate::fixture::COMMITTED);
    let file = File::open(&observations)
        .map_err(|e| format!("cannot read {}: {e}", observations.display()))?;
    let mut reader = BufReader::new(file);
    daily_scenes(Request {
        repo: &repo,
        data: &DataDir::at(dir.path().join("data")),
        observations: Some(&mut reader as &mut dyn BufRead),
        at_secs: fixture::ARC_REPLAY_END_SECS,
        install_salt: [0; 16],
    })
    .map_err(|e| format!("the replay failed: {e:?}"))
}

/// The events that play on the ground, which a frame is kept for.
const fn on_the_ground(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::ClearingBurn
            | EventKind::Flowering
            | EventKind::Lightning
            | EventKind::Earthquake
            | EventKind::Sinkhole
    )
}

/// The days a frame is kept for, in order: the first day at or after each change of the
/// timeline, the first day of each run of days an event plays on the ground (lightning strikes
/// day after day), and the last day.
fn key_days(days: &[(Tick, SceneInput)], timeline: &[TimelineEntry]) -> Vec<usize> {
    let mut kept = BTreeSet::new();
    for entry in timeline {
        if let Some(i) = days.iter().position(|&(tick, _)| tick >= entry.tick) {
            kept.insert(i);
        }
    }
    let mut before = None;
    for (i, (_, scene)) in days.iter().enumerate() {
        let kind = scene.event.map(|e| e.kind);
        if let Some(kind) = kind
            && Some(kind) != before
            && on_the_ground(kind)
        {
            kept.insert(i);
        }
        before = kind;
    }
    kept.extend(days.len().checked_sub(1));
    kept.into_iter().collect()
}

/// Whether the kept days show every presentation state.
fn missing_states(days: &[(Tick, SceneInput)], kept: &[usize]) -> Vec<PresentationState> {
    let shown: BTreeSet<PresentationState> =
        kept.iter().map(|&i| days[i].1.presentation.state).collect();
    PresentationState::ALL
        .iter()
        .copied()
        .filter(|state| !shown.contains(state))
        .collect()
}

/// Every file of the arc's review, rendered in memory.
pub fn render(root: &Path) -> Result<Files, String> {
    let replayed = replay(root)?;
    let days = &replayed.days;
    let kept = key_days(days, &replayed.timeline);
    let missing = missing_states(days, &kept);
    if !missing.is_empty() {
        return Err(format!("the arc's frames never show {missing:?}"));
    }
    let art = Art::embedded();
    let table = ArtTable::new(&art.catalog).map_err(|gaps| format!("{} art gaps", gaps.len()))?;
    let overlay = OverlayInput::new(vec![LABEL.to_owned()], LocalClock::noon(preset::DATE));
    let mut composer = Composer::new(&table, preset::SALT);
    let scenery = composer.scenery(art, &table);
    let mut now = Millis(0);
    let mut files = Files::new();
    let mut key = String::from("frame,day,tick,state,health,activity,played\n");
    let mut sheet = Vec::new();
    let mut played = BTreeSet::new();
    let mut next = kept.iter().copied().peekable();
    for (i, (tick, scene)) in days.iter().enumerate() {
        composer.update(scene, &overlay, now);
        now = composer.settled_at();
        if let Some(event) = scene.event {
            played.insert(event.kind.name());
        }
        if next.peek() != Some(&i) {
            continue;
        }
        next.next();
        let frame = files.len() / 2 + 1;
        let [day, night] = [0, u8::MAX]
            .map(|mix| composite(art, &composer.frame(art, &table, &scenery, mix, now)));
        files.push((format!("frames/{frame:02}-day.png"), large_png(&day)?));
        files.push((format!("frames/{frame:02}-night.png"), large_png(&night)?));
        let entry = replayed
            .timeline
            .iter()
            .rev()
            .find(|entry| entry.tick <= *tick);
        let _ = writeln!(
            key,
            "{frame:02},{},{},{},{},{},{}",
            tick.hours() / 24,
            tick.hours(),
            scene.presentation.state.name(),
            entry.map_or("", |e| e.health.name()),
            entry.map_or("", |e| e.activity.name()),
            std::mem::take(&mut played)
                .into_iter()
                .collect::<Vec<_>>()
                .join("+"),
        );
        sheet.push((day, night));
    }
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

/// Every kept frame at 1×, one to a row, its day beside its night.
fn contact_sheet(frames: &[(NativeImage, NativeImage)]) -> Vec<u8> {
    let rows = u32::try_from(frames.len()).unwrap_or(0);
    let width = 2 * NATIVE_WIDTH + 3 * GUTTER;
    let height = rows * NATIVE_HEIGHT + (rows + 1) * GUTTER;
    let mut pixels = vec![BACKDROP; usize::try_from(width * height).unwrap_or(0)];
    for (row, (day, night)) in (0_u32..).zip(frames) {
        let y0 = GUTTER + row * (NATIVE_HEIGHT + GUTTER);
        for (column, image) in (0_u32..).zip([day, night]) {
            let x0 = GUTTER + column * (NATIVE_WIDTH + GUTTER);
            for y in 0..NATIVE_HEIGHT {
                for x in 0..NATIVE_WIDTH {
                    let at = usize::try_from((y0 + y) * width + x0 + x).unwrap_or(0);
                    pixels[at] = image.pixel(x, y);
                }
            }
        }
    }
    export::png(width, height, &pixels)
}

#[cfg(test)]
mod tests {
    use terrarium_truth::domain::{ActivityClass, HealthClass};
    use terrarium_truth::scene::{ActiveEvent, Side, Strength};

    use super::*;

    fn day(tick: u32, state: PresentationState, event: Option<EventKind>) -> (Tick, SceneInput) {
        let mut scene = preset::bare(state, Vec::new());
        scene.event = event.map(|kind| ActiveEvent {
            kind,
            started_tick: Tick::new(tick / 48),
            strength: Strength::Mid,
            side: Side::Left,
            fossil: (kind == EventKind::Sinkhole).then_some(1),
        });
        (Tick::new(tick), scene)
    }

    fn entry(tick: u32, state: PresentationState) -> TimelineEntry {
        TimelineEntry {
            tick: Tick::new(tick),
            state,
            health: HealthClass::Healthy,
            activity: ActivityClass::Active,
        }
    }

    #[test]
    fn a_frame_is_kept_at_each_change_each_ground_event_and_the_end() {
        use EventKind::{Flowering, Storm};
        use PresentationState::{Mature, Thriving};
        let days = [
            day(24, Thriving, None),
            day(48, Thriving, Some(Flowering)),
            // A run of days an event plays on: one frame for it.
            day(72, Thriving, Some(Flowering)),
            day(96, Mature, Some(Storm)),
            day(120, Mature, None),
            day(144, Mature, None),
        ];
        let timeline = [entry(0, Thriving), entry(90, Mature)];
        assert_eq!(key_days(&days, &timeline), [0, 1, 3, 5]);
        assert_eq!(
            missing_states(&days, &key_days(&days, &timeline)).len(),
            PresentationState::ALL.len() - 2
        );
    }
}
