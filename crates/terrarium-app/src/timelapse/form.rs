//! The time-lapse window: the strip at a reduced scale, a
//! scrubber with the date, Play and Stop, the range of days, by day or by night, and the
//! exports: a GIF, a postcard, and an MP4 when the person's ffmpeg was found. A thin view over a model the strip drives: what its presses ask for comes back as
//! [`Intent`]s.
//!
//! An export is two steps: the save dialog asks where ([`Intent::Pick`]), and only the path it
//! answers with writes ([`Intent::Write`]), so the writing can be driven without the dialog.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use egui::{ColorImage, TextureHandle, TextureOptions, Vec2};
use terrarium_adapters::exec::Tool;
use terrarium_compose::frame::{NATIVE_HEIGHT, NATIVE_WIDTH};
use terrarium_compose::salt::InstallSalt;
use terrarium_compose::sky::LocalDate;
use terrarium_compose::timelapse::{FRAMES, Shot};

use super::Film;
use super::player::Player;
use crate::copy::timelapse as words;
use crate::shell::{Export, Intent, Picking};

/// The scales an export is written at, at the least: 1× to 4×, or to the strip's own if larger.
const EXPORT_SCALES: u32 = 4;

pub struct TimeLapseForm {
    /// The repository's name, for the files' names.
    name: String,
    salt: InstallSalt,
    /// The largest scale the window shows the strip at: a step below the strip's own.
    shown_up_to: u32,
    /// The largest scale an export is written at.
    scales: u32,
    /// The person's ffmpeg, kept apart from the phase, since its answer can come while the
    /// history is read.
    ffmpeg: Ffmpeg,
    phase: Phase,
    intents: Vec<Intent>,
    texture: Option<Texture>,
}

enum Ffmpeg {
    /// Being looked for: neither the MP4 nor why not is shown yet.
    Looking,
    Found(Tool),
    Missing,
}

enum Phase {
    /// The history being read, and how far.
    Reading(Option<u8>),
    Failed(String),
    Ready(Box<Ready>),
}

/// A history read: its days, the range and look asked for, and the film playing.
struct Ready {
    /// The date of each day the history has finished.
    dates: Vec<LocalDate>,
    /// The range the sliders stand at, and the range last asked for.
    from: usize,
    to: usize,
    asked: (usize, usize),
    night: bool,
    scale: u32,
    player: Player,
    /// A film is asked for and not yet made: what plays is another range's, so nothing is
    /// written until it comes.
    pending: bool,
    /// What the last export or film said.
    status: Option<String>,
    /// The export being written.
    writing: Option<Export>,
}

/// The picture as uploaded to the window's egui context, and which picture it is. A window
/// opened again has a new context, which needs it uploaded anew.
struct Texture {
    ctx: egui::Context,
    handle: TextureHandle,
    serial: u64,
}

impl TimeLapseForm {
    /// A window for the repository `name`, laid out by its `salt`, beside a strip at
    /// `strip_scale`.
    pub fn new(name: String, salt: InstallSalt, strip_scale: u32) -> TimeLapseForm {
        TimeLapseForm {
            name,
            salt,
            shown_up_to: strip_scale.saturating_sub(1).max(1),
            scales: strip_scale.max(EXPORT_SCALES),
            ffmpeg: Ffmpeg::Looking,
            phase: Phase::Reading(None),
            intents: Vec::new(),
            texture: None,
        }
    }

    /// Whether the person's ffmpeg was found: the MP4 is offered only then.
    pub fn ffmpeg(&mut self, ffmpeg: Option<Tool>) {
        self.ffmpeg = ffmpeg.map_or(Ffmpeg::Missing, Ffmpeg::Found);
    }

    pub fn reading(&mut self, percent: u8) {
        if let Phase::Reading(heard) = &mut self.phase {
            *heard = Some(percent);
        }
    }

    pub fn failed(&mut self, why: String) {
        self.phase = Phase::Failed(why);
    }

    /// The history is read, with a day at each of `days` (days since 1970-01-01): the whole
    /// range by day is asked for. None has ended yet is said, and asks for nothing.
    pub fn read(&mut self, days: &[i64]) -> Option<Intent> {
        let Some(last) = days.len().checked_sub(1) else {
            self.failed(words::NO_DAY_YET.to_owned());
            return None;
        };
        self.phase = Phase::Ready(Box::new(Ready {
            dates: days
                .iter()
                .map(|&day| LocalDate::from_days_since_epoch(day))
                .collect(),
            from: 0,
            to: last,
            asked: (0, last),
            night: false,
            scale: 1,
            player: Player::new(self.salt),
            pending: true,
            status: None,
            writing: None,
        }));
        Some(Intent::Film {
            from: 0,
            to: last,
            night: false,
        })
    }

    /// The film asked for last is ready; one asked before a change of Day or Night takes the
    /// look chosen since.
    pub fn filmed(&mut self, film: Arc<Film>, now: Instant) {
        if let Phase::Ready(ready) = &mut self.phase {
            ready.pending = false;
            let film = if film.night == ready.night {
                film
            } else {
                Arc::new(film.reclocked(ready.night))
            };
            ready.player.set_film(film, now);
        }
    }

    /// A film could not be made: said, and with none playing yet, all the window says.
    pub fn film_failed(&mut self, why: String) {
        match &mut self.phase {
            Phase::Ready(ready) if ready.player.film().is_some() => {
                ready.pending = false;
                ready.status = Some(why);
            }
            _ => self.failed(why),
        }
    }

    /// How far a GIF's or an MP4's writing has come.
    pub fn writing(&mut self, percent: u8) {
        if let Phase::Ready(ready) = &mut self.phase
            && let Some(export) = ready.writing
        {
            ready.status = Some(words::writing(export, percent));
        }
    }

    /// An export is written, or why not.
    pub fn written(&mut self, written: Result<PathBuf, String>) {
        if let Phase::Ready(ready) = &mut self.phase {
            ready.writing = None;
            ready.status = Some(match written {
                Ok(path) => words::saved(&path),
                Err(why) => why,
            });
        }
    }

    /// The save dialog answered: a path chosen writes the export there, if there is still
    /// something to write.
    pub fn picked(&mut self, export: Export, path: Option<PathBuf>) {
        let writable = match export {
            Export::Gif => self.gif().is_some(),
            Export::Postcard => self.postcard().is_some(),
            Export::Mp4 => self.mp4().is_some(),
        };
        if let (Phase::Ready(ready), Some(path), true) = (&mut self.phase, path, writable) {
            ready.writing = Some(export);
            ready.status = None;
            self.intents.push(Intent::Write { export, path });
        }
    }

    /// The name the save dialog offers for `export`.
    pub fn file_name(&self, export: Export) -> String {
        match export {
            Export::Gif => words::gif_name(&self.name),
            Export::Mp4 => words::mp4_name(&self.name),
            Export::Postcard => {
                words::postcard_name(&self.name, self.postcard().map(|(shot, _)| shot.clock.date))
            }
        }
    }

    /// The film to write as a GIF, and its scale: none while another is on its way.
    pub fn gif(&self) -> Option<(Arc<Film>, u32)> {
        let ready = self.settled()?;
        Some((Arc::clone(ready.player.film()?), ready.scale))
    }

    /// The film to write as an MP4, its scale, and the ffmpeg that writes it: none without
    /// one, or while another film is on its way.
    pub fn mp4(&self) -> Option<(Arc<Film>, u32, Tool)> {
        let (film, scale) = self.gif()?;
        match &self.ffmpeg {
            Ffmpeg::Found(ffmpeg) => Some((film, scale, ffmpeg.clone())),
            Ffmpeg::Looking | Ffmpeg::Missing => None,
        }
    }

    /// The shot shown, to write as a postcard, and its scale: none while another film is on
    /// its way.
    pub fn postcard(&self) -> Option<(Shot, u32)> {
        let ready = self.settled()?;
        let film = ready.player.film()?;
        Some((film.shot(ready.player.frame()).clone(), ready.scale))
    }

    /// The window, ready and playing the film asked for last.
    fn settled(&self) -> Option<&Ready> {
        match &self.phase {
            Phase::Ready(ready) if !ready.pending => Some(ready),
            _ => None,
        }
    }

    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    /// Moves the range's sliders: `from` moved past `to` carries it along, and the other way
    /// round. A range let go of is asked for, unless it is the one asked for last.
    fn range(&mut self, from: usize, to: usize, moved_from: bool, released: bool) {
        let Phase::Ready(ready) = &mut self.phase else {
            return;
        };
        let (from, to) = match (from > to, moved_from) {
            (true, true) => (from, from),
            (true, false) => (to, to),
            (false, _) => (from, to),
        };
        ready.from = from;
        ready.to = to;
        if released && (from, to) != ready.asked {
            ready.asked = (from, to);
            ready.pending = true;
            self.intents.push(Intent::Film {
                from,
                to,
                night: ready.night,
            });
        }
    }

    /// Takes Day or Night: the film playing is drawn at the other hour, with no replay, and
    /// one still on its way takes it when it comes.
    fn set_night(&mut self, night: bool, now: Instant) {
        let Phase::Ready(ready) = &mut self.phase else {
            return;
        };
        if night != ready.night {
            ready.night = night;
            if let Some(film) = ready.player.film() {
                let film = Arc::new(film.reclocked(night));
                ready.player.set_film(film, now);
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let now = Instant::now();
        let ready = match &mut self.phase {
            Phase::Reading(percent) => {
                ui.label(words::reading(*percent));
                return;
            }
            Phase::Failed(why) => {
                ui.label(why.as_str());
                return;
            }
            Phase::Ready(ready) => ready,
        };
        ready.player.advance(now);
        if let Some(delay) = ready.player.next_in() {
            ui.ctx().request_repaint_after(delay);
        }
        let Some((image, serial)) = ready.player.image() else {
            ui.label(words::PREPARING);
            return;
        };
        let texture = upload(&mut self.texture, ui.ctx(), image, serial);
        ui.image((texture.id(), picture_size(ui, self.shown_up_to)));
        let asked = ready.controls(ui, self.scales, &self.ffmpeg, now);
        if let Some((moved_from, released)) = asked.moved {
            self.range(asked.from, asked.to, moved_from, released);
        }
        self.set_night(asked.night, now);
        if let Some(export) = asked.picked {
            self.intents.push(Intent::Pick(Picking::Save(export)));
        }
    }
}

/// What a pass over the controls asked for beyond the player.
struct Asked {
    /// The range's sliders as they stand, and, if one moved, whether it was `from` and
    /// whether it is let go.
    from: usize,
    to: usize,
    moved: Option<(bool, bool)>,
    night: bool,
    picked: Option<Export>,
}

impl Ready {
    /// Draws the controls under the picture; the player and the scale take their changes at
    /// once.
    fn controls(&mut self, ui: &mut egui::Ui, scales: u32, ffmpeg: &Ffmpeg, now: Instant) -> Asked {
        let film = self.player.film().cloned();
        let mut asked = Asked {
            from: self.from,
            to: self.to,
            moved: None,
            night: self.night,
            picked: None,
        };
        ui.horizontal(|ui| {
            let playing = self.player.playing();
            if ui
                .button(if playing { words::STOP } else { words::PLAY })
                .clicked()
            {
                if playing {
                    self.player.stop(now);
                } else {
                    self.player.play(now);
                    ui.ctx().request_repaint();
                }
            }
            let mut frame = self.player.frame();
            let scrubbed = ui.add(egui::Slider::new(&mut frame, 0..=FRAMES - 1).show_value(false));
            if scrubbed.changed() {
                self.player.seek(frame, now);
                ui.ctx().request_repaint();
            }
            if let Some(film) = &film {
                ui.label(words::day(film.shot(self.player.frame()).clock.date));
            }
        });
        let last_day = self.dates.len() - 1;
        ui.horizontal(|ui| {
            for (name, value, moved_from) in [
                (words::FROM, &mut asked.from, true),
                (words::TO, &mut asked.to, false),
            ] {
                ui.label(name);
                let slider = ui.add(egui::Slider::new(value, 0..=last_day).show_value(false));
                ui.label(words::day(self.dates[*value]));
                if slider.changed() || slider.drag_stopped() {
                    asked.moved = Some((moved_from, slider.drag_stopped() || !slider.dragged()));
                }
            }
        });
        ui.horizontal(|ui| {
            ui.radio_value(&mut asked.night, false, words::BY_DAY);
            ui.radio_value(&mut asked.night, true, words::BY_NIGHT);
            ui.add_space(16.0);
            ui.label(words::SCALE);
            egui::ComboBox::from_id_salt("time-lapse-scale")
                .selected_text(words::scale(self.scale))
                .show_ui(ui, |ui| {
                    for k in 1..=scales {
                        ui.selectable_value(&mut self.scale, k, words::scale(k));
                    }
                });
        });
        let writable = film.is_some() && self.writing.is_none() && !self.pending;
        let mp4 = matches!(ffmpeg, Ffmpeg::Found(_));
        ui.horizontal(|ui| {
            for (export, word) in [
                (Export::Gif, words::SAVE_GIF),
                (Export::Postcard, words::SAVE_POSTCARD),
                (Export::Mp4, words::SAVE_MP4),
            ] {
                if (export != Export::Mp4 || mp4)
                    && ui.add_enabled(writable, egui::Button::new(word)).clicked()
                {
                    asked.picked = Some(export);
                }
            }
            if let Some(status) = &self.status {
                ui.label(status.as_str());
            }
        });
        if let Ffmpeg::Missing = ffmpeg {
            ui.add(egui::Label::new(words::NO_FFMPEG).wrap());
        }
        asked
    }
}

/// The picture's size in points: the largest whole scale of it whose physical width fits the
/// window, at most `shown_up_to`, at least 1×.
fn picture_size(ui: &egui::Ui, shown_up_to: u32) -> Vec2 {
    let ppp = ui.ctx().pixels_per_point();
    let room = ui.available_width() * ppp;
    let fits = |k: u32| u16::try_from(NATIVE_WIDTH * k).is_ok_and(|width| f32::from(width) <= room);
    let k = (1..=shown_up_to).rev().find(|&k| fits(k)).unwrap_or(1);
    let size = |native: u32| u16::try_from(native * k).map_or(f32::MAX, f32::from) / ppp;
    Vec2::new(size(NATIVE_WIDTH), size(NATIVE_HEIGHT))
}

/// The picture in `ctx`, uploaded once per picture and anew for a new context.
fn upload<'a>(
    texture: &'a mut Option<Texture>,
    ctx: &egui::Context,
    image: &terrarium_compose::frame::NativeImage,
    serial: u64,
) -> &'a TextureHandle {
    let colours = || {
        let bytes: Vec<u8> = image.pixels().iter().flatten().copied().collect();
        ColorImage::from_rgba_unmultiplied([NATIVE_WIDTH as usize, NATIVE_HEIGHT as usize], &bytes)
    };
    match texture {
        Some(shown) if shown.ctx == *ctx => {
            if shown.serial != serial {
                shown.handle.set(colours(), TextureOptions::NEAREST);
                shown.serial = serial;
            }
        }
        _ => {
            *texture = Some(Texture {
                ctx: ctx.clone(),
                handle: ctx.load_texture("time-lapse", colours(), TextureOptions::NEAREST),
                serial,
            });
        }
    }
    &texture.as_ref().expect("uploaded").handle
}

#[cfg(test)]
mod tests {
    use terrarium_compose::clock::LocalClock;
    use terrarium_compose::preset;
    use terrarium_compose::timelapse::Sampling;

    use super::*;

    const DAY_ONE: i64 = 19_796; // 2024-03-14

    fn form() -> TimeLapseForm {
        TimeLapseForm::new("fern".to_owned(), preset::SALT, 4)
    }

    fn film(days: usize) -> Arc<Film> {
        let shots: Vec<Shot> = (0..days)
            .map(|day| Shot {
                scene: preset::scene("thriving").unwrap(),
                label: vec![format!("day {day}")],
                clock: LocalClock::noon(LocalDate::from_days_since_epoch(
                    DAY_ONE + i64::try_from(day).unwrap(),
                )),
            })
            .collect();
        Arc::new(Film {
            sampling: Sampling::new(0, days - 1, FRAMES),
            prime: None,
            shots,
            night: false,
        })
    }

    fn pass(form: &mut TimeLapseForm) {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| form.ui(ui));
        output.drop_without_applying_deltas();
    }

    /// The text one pass draws, a line per piece.
    fn drawn(form: &mut TimeLapseForm) -> String {
        fn texts(shape: &egui::Shape, out: &mut String) {
            match shape {
                egui::Shape::Text(text) => {
                    out.push_str(text.galley.text());
                    out.push('\n');
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| form.ui(ui));
        let mut out = String::new();
        for clipped in &output.shapes {
            texts(&clipped.shape, &mut out);
        }
        output.drop_without_applying_deltas();
        out
    }

    #[test]
    fn without_ffmpeg_the_mp4_is_not_offered_and_a_fact_says_why() {
        let mut form = form();
        // The answer comes while the history is read, and outlasts the read.
        form.ffmpeg(None);
        pass(&mut form);
        form.read(&[DAY_ONE, DAY_ONE + 1]);
        form.filmed(film(2), Instant::now());
        let text = drawn(&mut form);
        assert!(text.contains(words::SAVE_GIF), "{text}");
        assert!(!text.contains(words::SAVE_MP4), "{text}");
        assert!(text.contains(words::NO_FFMPEG), "{text}");
        assert!(form.mp4().is_none());
        form.picked(Export::Mp4, Some(PathBuf::from("fern.mp4")));
        assert!(form.take_intents().is_empty(), "nothing writes an MP4");
        // While it is looked for, neither is shown.
        let mut looking = self::form();
        looking.read(&[DAY_ONE, DAY_ONE + 1]);
        looking.filmed(film(2), Instant::now());
        let text = drawn(&mut looking);
        assert!(!text.contains(words::SAVE_MP4) && !text.contains(words::NO_FFMPEG));
    }

    #[cfg(unix)]
    #[test]
    fn with_ffmpeg_the_mp4_asks_where_then_writes_with_it() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ffmpeg");
        std::fs::write(&file, "").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths([dir.path()]).unwrap();
        let ffmpeg = terrarium_adapters::exec::find_tool(&["ffmpeg"], Some(&path), &[], None);
        assert!(ffmpeg.is_some());
        let mut form = form();
        form.read(&[DAY_ONE, DAY_ONE + 1]);
        form.ffmpeg(ffmpeg.clone());
        form.filmed(film(2), Instant::now());
        let text = drawn(&mut form);
        assert!(text.contains(words::SAVE_MP4) && !text.contains(words::NO_FFMPEG));
        assert_eq!(form.file_name(Export::Mp4), "fern-time-lapse.mp4");
        let (_, scale, tool) = form.mp4().unwrap();
        assert_eq!((scale, Some(tool)), (1, ffmpeg));
        form.picked(Export::Mp4, Some(PathBuf::from("fern.mp4")));
        assert_eq!(
            form.take_intents(),
            [Intent::Write {
                export: Export::Mp4,
                path: PathBuf::from("fern.mp4")
            }]
        );
        form.writing(60);
        let Phase::Ready(ready) = &form.phase else {
            unreachable!()
        };
        assert_eq!(ready.status.as_deref(), Some("Writing the MP4: 60%"));
    }

    #[test]
    fn a_history_read_asks_for_its_whole_range_by_day_and_one_with_no_day_says_so() {
        let mut form = form();
        form.reading(40);
        pass(&mut form);
        assert_eq!(
            form.read(&[DAY_ONE, DAY_ONE + 1, DAY_ONE + 2]),
            Some(Intent::Film {
                from: 0,
                to: 2,
                night: false
            })
        );
        pass(&mut form);
        let mut empty = TimeLapseForm::new("fern".to_owned(), preset::SALT, 1);
        assert_eq!(empty.read(&[]), None);
        assert!(matches!(&empty.phase, Phase::Failed(why) if why == words::NO_DAY_YET));
        assert_eq!(empty.shown_up_to, 1);
        assert_eq!(empty.scales, EXPORT_SCALES);
    }

    #[test]
    fn the_range_keeps_its_ends_in_order_and_asks_again_by_night() {
        let mut form = form();
        form.read(&[DAY_ONE, DAY_ONE + 1, DAY_ONE + 2, DAY_ONE + 3]);
        form.range(2, 3, true, true);
        // Dragged: the sliders move, and nothing is asked until let go.
        form.range(3, 3, true, false);
        form.range(3, 3, true, true);
        form.range(3, 1, false, true);
        form.range(1, 1, false, true);
        form.range(2, 1, true, true);
        // Night asks for nothing: the film playing is drawn again at midnight.
        form.set_night(true, Instant::now());
        let asked: Vec<(usize, usize, bool)> = form
            .take_intents()
            .into_iter()
            .map(|intent| match intent {
                Intent::Film { from, to, night } => (from, to, night),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            asked,
            [(2, 3, false), (3, 3, false), (1, 1, false), (2, 2, false),]
        );
    }

    #[test]
    fn an_export_asks_where_then_writes_there_and_says_how_it_went() {
        let mut form = form();
        form.read(&[DAY_ONE, DAY_ONE + 1]);
        assert!(form.gif().is_none(), "nothing to write before a film");
        form.picked(Export::Gif, Some(PathBuf::from("early.gif")));
        assert!(form.take_intents().is_empty(), "nor anything asked to");
        let now = Instant::now();
        form.filmed(film(2), now);
        pass(&mut form);
        assert_eq!(form.file_name(Export::Gif), "fern-time-lapse.gif");
        assert_eq!(form.file_name(Export::Postcard), "fern-2024-03-14.png");
        let (gif, scale) = form.gif().unwrap();
        assert_eq!((gif.shots.len(), scale), (2, 1));
        form.picked(Export::Gif, None);
        assert!(
            form.take_intents().is_empty(),
            "a dialog closed writes nothing"
        );
        form.picked(Export::Gif, Some(PathBuf::from("fern.gif")));
        assert_eq!(
            form.take_intents(),
            [Intent::Write {
                export: Export::Gif,
                path: PathBuf::from("fern.gif")
            }]
        );
        form.writing(45);
        let Phase::Ready(ready) = &form.phase else {
            unreachable!()
        };
        assert_eq!(ready.writing, Some(Export::Gif));
        assert_eq!(ready.status.as_deref(), Some("Writing the GIF: 45%"));
        form.written(Ok(PathBuf::from("/tmp/fern.gif")));
        let Phase::Ready(ready) = &form.phase else {
            unreachable!()
        };
        assert_eq!(ready.writing, None);
        assert_eq!(ready.status.as_deref(), Some("Saved fern.gif"));
        // A film that fails once one plays is said beside it.
        form.film_failed("no".to_owned());
        assert!(matches!(&form.phase, Phase::Ready(_)));
    }

    #[test]
    fn nothing_is_written_while_another_film_is_on_its_way_and_night_redraws_the_one_playing() {
        let mut form = form();
        form.read(&[DAY_ONE, DAY_ONE + 1, DAY_ONE + 2]);
        let now = Instant::now();
        form.filmed(film(3), now);
        assert!(form.gif().is_some());
        form.range(1, 2, true, true);
        assert!(form.gif().is_none() && form.postcard().is_none());
        form.picked(Export::Postcard, Some(PathBuf::from("fern.png")));
        assert!(
            form.take_intents()
                .iter()
                .all(|i| !matches!(i, Intent::Write { .. }))
        );
        // Night chosen while the new film is made: the one playing and the one to come are
        // both drawn at midnight.
        form.set_night(true, now);
        let night = |form: &TimeLapseForm| {
            let Phase::Ready(ready) = &form.phase else {
                unreachable!()
            };
            ready.player.film().unwrap().night
        };
        assert!(night(&form));
        form.filmed(film(2), now);
        assert!(night(&form));
        let (gif, _) = form.gif().unwrap();
        assert!(gif.shots.iter().all(|shot| shot.clock.ms_of_day == 0));
    }
}
