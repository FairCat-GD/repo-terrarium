//! The connect window, open while no repository is connected.
//!
//! Picking a folder and connecting it are two steps: the picker only fills in the path, and
//! connecting is a separate press, so a harness drives the connect without any dialog. On the
//! first run, "Open at login" is offered, checked; never after. Once connected, the window
//! shows the discovery report, once.
//!
//! The form is a model the window only draws; what a press asks for is an [`Intent`] the strip
//! carries out.
//!
//! Across the top of the window stands a still of a thriving terrarium, the sample's, by day or
//! by night as the clock is: the first window looks like what it connects. It is composed as
//! wide as the window, at the largest whole scale its scene fits (I-11: never stretched), and is
//! a picture of the product, never of the repository; the report takes its place once one is
//! connected.

use std::path::PathBuf;

use terrarium_compose::catalog::Art;
use terrarium_compose::compositor::composite;
use terrarium_compose::frame::{NATIVE_HEIGHT, NATIVE_WIDTH, NativeImage, Viewport, WIDEST};
use terrarium_compose::preset;
use terrarium_compose::variants::ArtTable;

use crate::copy::connect as words;
use crate::picture;
use crate::shell::{Intent, Picking};

/// The most the still is scaled up, in screen pixels per art pixel.
const STILL_UP_TO: u32 = 8;

/// The sample terrarium, thriving and settled, `columns` wide, at `palette_mix` (0 by day, 255
/// by night).
pub fn still(palette_mix: u8, columns: u32) -> Option<NativeImage> {
    let art = Art::embedded();
    let frame = preset::frame_in(
        art,
        ArtTable::embedded(),
        "thriving",
        crate::copy::sample_label(),
        palette_mix,
        Viewport::new(columns),
    )?;
    Some(composite(art, &frame))
}

/// For a window `room` screen pixels wide: the whole scale of the still and the columns it is
/// composed with, so that it spans the window.
pub fn still_fit(room: f32) -> (u32, u32) {
    let k = picture::whole_scale(NATIVE_WIDTH, room, STILL_UP_TO);
    // The most whole columns that fit, within what the composer draws.
    let fits = |columns: u32| u16::try_from(columns * k).is_ok_and(|wide| f32::from(wide) <= room);
    let columns = (NATIVE_WIDTH..=WIDEST)
        .rev()
        .find(|&columns| fits(columns))
        .unwrap_or(NATIVE_WIDTH);
    (k, columns)
}

/// How many screen pixels from the window's left edge a still `width` pixels wide starts, so
/// that whatever whole columns do not fill is shared by both sides; a whole number of pixels,
/// so each art pixel still lands on whole screen pixels.
fn centred(window: f32, width: f32) -> f32 {
    ((window - width) / 2.0).max(0.0).floor()
}

/// The still as shown: its palette mix, and the picture with its serial once composed.
struct Still {
    mix: u8,
    image: Option<NativeImage>,
    serial: u64,
}

pub struct ConnectForm {
    still: Option<Still>,
    texture: Option<picture::Texture>,
    first_run: bool,
    path: Option<PathBuf>,
    open_at_login: bool,
    page: Page,
    /// Why the last connect failed, said until the next.
    problem: Option<String>,
    /// A connect is under way.
    busy: bool,
    intent: Option<Intent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Page {
    Choose,
    /// Connected: the repository's name and the discovery report.
    Report {
        name: String,
        report: String,
    },
}

impl ConnectForm {
    pub fn new(first_run: bool) -> ConnectForm {
        ConnectForm {
            still: None,
            texture: None,
            first_run,
            path: None,
            open_at_login: true,
            page: Page::Choose,
            problem: None,
            busy: false,
            intent: None,
        }
    }

    /// Shows the still across the window's top, at `palette_mix`.
    #[must_use]
    pub fn with_still(mut self, palette_mix: u8) -> ConnectForm {
        self.still = Some(Still {
            mix: palette_mix,
            image: None,
            serial: 0,
        });
        self
    }

    /// "Choose folder…": asks for the picker.
    pub fn choose(&mut self) {
        if !self.busy {
            self.intent = Some(Intent::Pick(Picking::Repository));
        }
    }

    /// What the picker returned; `None` when it was cancelled, which changes nothing.
    pub fn picked(&mut self, path: Option<PathBuf>) {
        if let Some(path) = path {
            self.path = Some(path);
            self.problem = None;
        }
    }

    pub fn set_open_at_login(&mut self, on: bool) {
        self.open_at_login = on;
    }

    /// "Connect": asks to connect the folder picked, with the first run's login choice.
    pub fn connect(&mut self) {
        let Some(path) = self.path.clone().filter(|_| !self.busy) else {
            return;
        };
        self.busy = true;
        self.problem = None;
        self.intent = Some(Intent::Connect {
            path,
            open_at_login: self.first_run.then_some(self.open_at_login),
        });
    }

    /// The connect failed, and why, in the window's words.
    pub fn failed(&mut self, problem: String) {
        self.busy = false;
        self.problem = Some(problem);
    }

    /// Connected `name`: the report shows, once.
    pub fn connected(&mut self, name: String, report: String) {
        self.busy = false;
        self.page = Page::Report { name, report };
    }

    /// "Done": the report is read.
    pub fn done(&mut self) {
        if matches!(self.page, Page::Report { .. }) {
            self.intent = Some(Intent::Done);
        }
    }

    pub fn take_intent(&mut self) -> Option<Intent> {
        self.intent.take()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        match self.page.clone() {
            Page::Choose => self.choose_ui(ui),
            Page::Report { name, report } => {
                ui.heading(words::connected(&name));
                ui.add_space(8.0);
                // Room is kept for "Done" below it, but never less than a line of the report.
                let room = ui.available_height() - 40.0;
                egui::ScrollArea::vertical()
                    .max_height(room.max(ui.spacing().interact_size.y))
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(report).monospace()).wrap());
                    });
                ui.add_space(8.0);
                if ui.button(words::DONE).clicked() {
                    self.done();
                }
            }
        }
    }

    /// The still across the top of the window, edge to edge, composed again when the window's
    /// width asks for other columns.
    fn still_ui(&mut self, ui: &mut egui::Ui) {
        let Some(still) = &mut self.still else {
            return;
        };
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let window = ctx.content_rect();
        let (k, columns) = still_fit(window.width() * ppp);
        if still.image.as_ref().map(NativeImage::width) != Some(columns) {
            still.image = self::still(still.mix, columns);
            still.serial += 1;
        }
        let Some(image) = &still.image else {
            return;
        };
        let texture = picture::upload(&mut self.texture, &ctx, image, still.serial);
        let points = |native: u32| f32::from(u16::try_from(native * k).unwrap_or(u16::MAX)) / ppp;
        let size = egui::vec2(points(columns), points(NATIVE_HEIGHT));
        let rect = egui::Rect::from_min_size(
            egui::pos2(
                window.min.x + centred(window.width() * ppp, size.x * ppp) / ppp,
                window.min.y,
            ),
            size,
        );
        let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter()
            .with_clip_rect(window)
            .image(texture.id(), rect, whole, egui::Color32::WHITE);
        // The form starts a little below it.
        ui.add_space(rect.bottom() - ui.cursor().top() + 12.0);
    }

    fn choose_ui(&mut self, ui: &mut egui::Ui) {
        self.still_ui(ui);
        ui.heading(words::HEADING);
        ui.add_space(4.0);
        ui.label(words::INTRO);
        ui.add_space(12.0);
        if ui
            .add_enabled(!self.busy, egui::Button::new(words::CHOOSE))
            .clicked()
        {
            self.choose();
        }
        // A path can be longer than the window is wide: it wraps on a line of its own.
        match &self.path {
            Some(path) => ui.add(
                egui::Label::new(egui::RichText::new(path.display().to_string()).monospace())
                    .wrap(),
            ),
            None => ui.weak(words::NONE_CHOSEN),
        };
        if self.first_run {
            let mut on = self.open_at_login;
            if ui
                .add_enabled(
                    !self.busy,
                    egui::Checkbox::new(&mut on, words::OPEN_AT_LOGIN),
                )
                .changed()
            {
                self.set_open_at_login(on);
            }
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let ready = self.path.is_some() && !self.busy;
            if ui
                .add_enabled(ready, egui::Button::new(words::CONNECT))
                .clicked()
            {
                self.connect();
            }
            if self.busy {
                ui.spinner();
                ui.label(words::READING);
            }
        });
        if let Some(problem) = &self.problem {
            ui.add_space(8.0);
            ui.label(problem);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_still_is_the_sample_terrarium_thriving_by_day_or_by_night() {
        let art = Art::embedded();
        for (mix, columns) in [(0, 640), (255, 680)] {
            let frame = preset::frame_in(
                art,
                ArtTable::embedded(),
                "thriving",
                "repo-terrarium",
                mix,
                Viewport::new(columns),
            )
            .unwrap();
            let image = still(mix, columns).unwrap();
            assert_eq!(image.width(), columns);
            assert_eq!(image, composite(art, &frame));
        }
        assert_ne!(still(0, 640), still(255, 640));
    }

    #[test]
    fn the_still_spans_the_window_at_a_whole_scale() {
        // 680 points on a 2x display: 2 screen pixels an art pixel, 680 columns.
        assert_eq!(still_fit(1_360.0), (2, 680));
        // At 1x: one to one, 680 columns.
        assert_eq!(still_fit(680.0), (1, 680));
        // Wider than the composer draws: the most columns it draws.
        assert_eq!(still_fit(1_800.0), (2, 840));
        // Narrower than the scene: the scene's 640.
        assert_eq!(still_fit(500.0), (1, 640));
        // What whole columns leave is shared by both sides.
        for (window, width, left) in [
            (900.0, 840.0, 30.0),
            (680.0, 680.0, 0.0),
            (300.0, 320.0, 0.0),
        ] {
            assert!(
                (centred(window, width) - left).abs() < f32::EPSILON,
                "{window} {width}"
            );
        }
    }

    #[test]
    fn a_folder_is_picked_then_connected_in_two_steps() {
        let mut form = ConnectForm::new(true);
        // Nothing to connect before a folder is picked.
        form.connect();
        assert_eq!(form.take_intent(), None);
        form.choose();
        assert_eq!(form.take_intent(), Some(Intent::Pick(Picking::Repository)));
        // A cancelled pick changes nothing; a pick fills in the path only.
        form.picked(None);
        assert_eq!(form.path, None);
        form.picked(Some(PathBuf::from("/projects/moss")));
        assert_eq!(form.take_intent(), None);
        form.connect();
        assert_eq!(
            form.take_intent(),
            Some(Intent::Connect {
                path: PathBuf::from("/projects/moss"),
                open_at_login: Some(true),
            })
        );
        // While it connects, nothing else is asked.
        form.connect();
        form.choose();
        assert_eq!(form.take_intent(), None);
    }

    #[test]
    fn open_at_login_is_offered_checked_on_the_first_run_only() {
        let mut first = ConnectForm::new(true);
        first.picked(Some(PathBuf::from("/moss")));
        first.set_open_at_login(false);
        first.connect();
        assert!(matches!(
            first.take_intent(),
            Some(Intent::Connect {
                open_at_login: Some(false),
                ..
            })
        ));
        let mut later = ConnectForm::new(false);
        later.picked(Some(PathBuf::from("/moss")));
        later.connect();
        assert!(matches!(
            later.take_intent(),
            Some(Intent::Connect {
                open_at_login: None,
                ..
            })
        ));
    }

    #[test]
    fn a_failure_is_said_and_another_try_can_follow_and_success_shows_the_report() {
        let mut form = ConnectForm::new(false);
        form.picked(Some(PathBuf::from("/plain")));
        form.connect();
        form.take_intent();
        form.failed("This folder is not a git repository.".into());
        assert_eq!(
            form.problem.as_deref(),
            Some("This folder is not a git repository.")
        );
        form.picked(Some(PathBuf::from("/moss")));
        assert_eq!(form.problem, None);
        form.connect();
        assert!(form.take_intent().is_some());
        // "Done" means nothing before the report.
        form.done();
        assert_eq!(form.take_intent(), None);
        form.connected("moss".into(), "Found\n".into());
        assert_eq!(
            form.page,
            Page::Report {
                name: "moss".into(),
                report: "Found\n".into()
            }
        );
        form.done();
        assert_eq!(form.take_intent(), Some(Intent::Done));
    }
}
