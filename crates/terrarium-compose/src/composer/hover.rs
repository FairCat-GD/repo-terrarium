//! The hover layer as drawn (I-15): the soil panel, its row pointer and a hover label, above
//! everything else.
//!
//! - **The panel rises** out of the bottom of the frame over [`RISE_MS`] and lowers back into it
//!   the same way; what lies wholly below the frame is not drawn. Lowered, it is gone.
//! - **A new page** rises over the one shown, which holds until the new one stands and then
//!   falls ([`super::Restyle::Cover`]), each over [`PAGE_MS`].
//! - **The pointer** fades onto its row and off it over [`POINTER_MS`].
//! - **A hover label** fades in over its landmark and out again over [`TAG_MS`].
//!
//! Each page and label keeps a serial number for its life, so a draw is the same draw from one
//! moment to the next however the others come and go. Draws are stacked in the order given: text
//! lies over its panel, a newer page over an older, and the panel over a label.

use crate::blend::scale;
use crate::catalog::Art;
use crate::frame::{Area, Draw, NATIVE_HEIGHT};
use crate::order::{KeyedDraw, Place, Subject};
use crate::panel::{Hover, Landmark, Page, PanelArt, Part, Tag, Target, layout};
use crate::tween::durations::{PAGE_MS, POINTER_MS, RISE_MS, TAG_MS};
use crate::tween::{Millis, Tween};

use super::{Earliest, SHOWN};

const FRAME_BOTTOM: i32 = NATIVE_HEIGHT.cast_signed();

#[derive(Clone, Debug)]
pub(super) struct HoverLayer {
    /// How far the panel has risen: 0 lowered, [`SHOWN`] standing.
    rise: Tween,
    /// The pages, oldest first.
    pages: Vec<PageLook>,
    /// The hover labels, oldest first.
    tags: Vec<TagLook>,
    serial: u16,
}

#[derive(Clone, Debug)]
struct PageLook {
    serial: u16,
    page: Page,
    anchor: i32,
    opacity: Tween,
    /// The pointer's places on this page, oldest first.
    pointers: Vec<(Target, Tween)>,
}

#[derive(Clone, Debug)]
struct TagLook {
    serial: u16,
    tag: Tag,
    opacity: Tween,
}

/// One look of a landmark as drawn: its spot and its look's place among the landmark's looks,
/// where its sprite lies in the viewport, the column its label centres on and the row the label
/// stands over, how visible it is, and whether it is the newest look, standing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Placed {
    pub landmark: Landmark,
    pub spot: u8,
    pub look: u16,
    pub area: Area,
    pub centre: i32,
    pub top: i32,
    pub opacity: u8,
    pub standing: bool,
}

impl Default for HoverLayer {
    fn default() -> HoverLayer {
        HoverLayer {
            rise: Tween::at(0),
            pages: Vec::new(),
            tags: Vec::new(),
            serial: 0,
        }
    }
}

impl HoverLayer {
    fn next_serial(&mut self) -> u16 {
        self.serial = self.serial.wrapping_add(1);
        self.serial
    }

    /// Moves the layer toward `hover` from `now`; the frame at `now` does not change.
    pub(super) fn set(&mut self, hover: &Hover, now: Millis) {
        self.prune(now);
        match &hover.panel {
            None => {
                if self.rise.target() != 0 {
                    self.rise = Tween::new(self.rise.value(now), 0, now, RISE_MS);
                }
            }
            Some(panel) => {
                let lowered = self.rise.value(now) == 0;
                if lowered {
                    let serial = self.next_serial();
                    self.pages = vec![PageLook {
                        serial,
                        page: panel.page.clone(),
                        anchor: panel.anchor,
                        opacity: Tween::at(SHOWN),
                        pointers: panel
                            .pointer
                            .map(|target| (target, Tween::at(SHOWN)))
                            .into_iter()
                            .collect(),
                    }];
                }
                if self.rise.target() != SHOWN {
                    self.rise = Tween::new(self.rise.value(now), SHOWN, now, RISE_MS);
                }
                let current = self
                    .pages
                    .iter()
                    .rposition(|look| look.opacity.target() == SHOWN);
                let same = current.is_some_and(|i| {
                    self.pages[i].page == panel.page && self.pages[i].anchor == panel.anchor
                });
                if !same {
                    for look in &mut self.pages {
                        if look.opacity.target() == SHOWN {
                            let value = look.opacity.value(now);
                            look.opacity = Tween::new(value, 0, now.plus(PAGE_MS), PAGE_MS);
                        }
                    }
                    let serial = self.next_serial();
                    self.pages.push(PageLook {
                        serial,
                        page: panel.page.clone(),
                        anchor: panel.anchor,
                        opacity: Tween::new(0, SHOWN, now, PAGE_MS),
                        pointers: Vec::new(),
                    });
                }
                if let Some(look) = self.pages.last_mut() {
                    point(look, panel.pointer, now);
                }
            }
        }
        let current = self
            .tags
            .iter()
            .rposition(|look| look.opacity.target() == SHOWN);
        if current.map(|i| &self.tags[i].tag) != hover.tag.as_ref() {
            for look in &mut self.tags {
                if look.opacity.target() == SHOWN {
                    look.opacity = Tween::new(look.opacity.value(now), 0, now, TAG_MS);
                }
            }
            if let Some(tag) = &hover.tag {
                let serial = self.next_serial();
                self.tags.push(TagLook {
                    serial,
                    tag: tag.clone(),
                    opacity: Tween::new(0, SHOWN, now, TAG_MS),
                });
            }
        }
    }

    /// Drops what has gone for good: pages and labels faded out, everything once lowered.
    fn prune(&mut self, now: Millis) {
        if self.rise.target() == 0 && now >= self.rise.end() {
            self.pages.clear();
        }
        let gone = |opacity: Tween| opacity.target() == 0 && now >= opacity.end();
        self.pages.retain(|look| !gone(look.opacity));
        for look in &mut self.pages {
            look.pointers.retain(|(_, opacity)| !gone(*opacity));
        }
        self.tags.retain(|look| !gone(look.opacity));
    }

    /// Whether a hover label is shown or fading.
    pub(super) fn has_tags(&self) -> bool {
        !self.tags.is_empty()
    }

    /// Every draw of the layer at `now`, in a viewport `columns` wide, with `landmarks` where
    /// the scene draws them.
    pub(super) fn push(
        &self,
        art: &Art,
        landmarks: &[Placed],
        columns: i32,
        now: Millis,
        out: &mut Vec<KeyedDraw>,
    ) {
        if self.pages.is_empty() && self.tags.is_empty() {
            return;
        }
        let Some(panel_art) = PanelArt::new(&art.catalog) else {
            return;
        };
        let mut push = |place: Place, stack: i32, draw: Draw, opacity: u8| {
            let opacity = scale(draw.opacity, opacity);
            if opacity == 0 || draw.y >= FRAME_BOTTOM {
                return;
            }
            out.push(KeyedDraw::at(place, (0, stack), Draw { opacity, ..draw }));
        };
        // A label over each look of its landmark, fading with it.
        for look in &self.tags {
            let value = look.opacity.value(now);
            for placed in landmarks.iter().filter(|p| p.landmark == look.tag.landmark) {
                let subject = Subject::Tag {
                    serial: look.serial,
                    spot: placed.spot,
                };
                let draws = match look.tag.landmark {
                    Landmark::Monument(_) => {
                        panel_art.plate(&look.tag.text, placed.centre, placed.top, columns)
                    }
                    Landmark::Fossil(_) => {
                        panel_art.note(&look.tag.text, placed.centre, placed.top, columns)
                    }
                };
                for (item, draw) in (0_u16..).zip(draws) {
                    let part = u8::from(draw.tint.is_some());
                    let place = Place::new(subject, part, placed.look).item(item);
                    push(place, i32::from(part), draw, scale(value, placed.opacity));
                }
            }
        }
        for (rank, look) in (0_i32..).zip(&self.pages) {
            let value = look.opacity.value(now);
            let Some(layout) = layout(&art.catalog, &look.page, look.anchor, columns) else {
                continue;
            };
            let drop = drop(self.rise.value(now), layout.area.h);
            let subject = Subject::Panel;
            let stack = |part: Part| 16 + rank * 8 + i32::from(part.index());
            let mut items = [0_u16; Part::ALL.len()];
            for (part, draw) in panel_art.page(&look.page, &layout, drop) {
                let index = usize::from(part.index());
                let place = Place::new(subject, part.index(), look.serial).item(items[index]);
                items[index] += 1;
                push(place, stack(part), draw, value);
            }
            for (item, (target, opacity)) in (0_u16..).zip(&look.pointers) {
                if let Some(draw) = panel_art.pointer(&layout, *target, drop) {
                    let part = Part::Pointer.index();
                    let place = Place::new(subject, part, look.serial).item(item);
                    push(
                        place,
                        stack(Part::Pointer),
                        draw,
                        scale(value, opacity.value(now)),
                    );
                }
            }
        }
    }

    /// The first moment after `now` at which the layer may draw otherwise.
    pub(super) fn next_change(&self, art: &Art, columns: i32, now: Millis) -> Option<Millis> {
        let mut next = Earliest::default();
        for look in &self.pages {
            if let Some(layout) = layout(&art.catalog, &look.page, look.anchor, columns) {
                let h = layout.area.h;
                next.take(
                    self.rise
                        .next_step_by(now, |rise| u8::try_from(drop(rise, h)).unwrap_or(u8::MAX)),
                );
            }
            next.take(look.opacity.next_step(now));
            for (_, opacity) in &look.pointers {
                next.take(opacity.next_step(now));
            }
        }
        for look in &self.tags {
            next.take(look.opacity.next_step(now));
        }
        next.0
    }

    /// When the last of the layer's tweens ends.
    pub(super) fn end(&self) -> Millis {
        self.pages
            .iter()
            .flat_map(|look| {
                std::iter::once(look.opacity).chain(look.pointers.iter().map(|(_, o)| *o))
            })
            .chain(self.tags.iter().map(|look| look.opacity))
            .chain((!self.pages.is_empty()).then_some(self.rise))
            .map(Tween::end)
            .max()
            .unwrap_or_default()
    }
}

/// Moves `look`'s pointer to `target`: the old place fades out as the new fades in.
fn point(look: &mut PageLook, target: Option<Target>, now: Millis) {
    let current = look
        .pointers
        .iter()
        .rposition(|(_, opacity)| opacity.target() == SHOWN);
    if current.map(|i| look.pointers[i].0) == target {
        return;
    }
    for (_, opacity) in &mut look.pointers {
        if opacity.target() == SHOWN {
            *opacity = Tween::new(opacity.value(now), 0, now, POINTER_MS);
        }
    }
    if let Some(target) = target {
        look.pointers
            .push((target, Tween::new(0, SHOWN, now, POINTER_MS)));
    }
}

/// How far below its place a panel `height` rows tall stands at `rise`: all of its height when
/// lowered, so it lies wholly below the frame.
fn drop(rise: u8, height: i32) -> i32 {
    let below = i32::from(SHOWN - rise);
    (below * height + 254) / 255
}
