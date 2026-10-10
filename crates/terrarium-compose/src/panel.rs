//! The soil panel and the hover labels: pages the app words, laid out on the nine-slice panel,
//! and the small plate a monument's year sits on.
//!
//! A page is a title over up to seven rows. A list shows one reading per row: a pointer column,
//! the name, the value, then the trend and freshness icons, each in its own column so they line
//! up. A page of files lists up to 21 paths with their counts, seven to a column, each path cut
//! at its head so the file's name stays. The panel stands on the frame's bottom edge, rising out
//! of the soil band, centred on its anchor and kept inside the glass. [`layout`] is shared by
//! the drawing and by the app's hit-testing, so what is pointed at is what is drawn.

use crate::catalog::{Catalog, Layer, SpriteId};
use crate::frame::{Area, Draw, NATIVE_HEIGHT};
use crate::nine_slice::NineSlice;
use crate::palette::Slot;
use crate::text::{Face, Text, fit, fit_tail};
use crate::world::LABEL_X;

/// Rows on a page, and in each column of files.
pub const ROWS: usize = 7;
/// Columns of files at most: 21 places for the top 20 and a branch's reading.
pub const FILE_COLUMNS: usize = 3;
/// The longest a row's name, value or a fossil's label may be, in characters.
pub const MAX_CHARS: usize = 40;

/// Which way a reading moved over the week.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Trend {
    Up,
    Flat,
    Down,
}

/// A reading that is not fresh; a fresh one shows no icon (there is no fresh icon).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Freshness {
    Stale,
    Unknown,
}

/// How a page lays out its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Shape {
    /// One reading per row, with its icons.
    List,
    /// Paths and their counts, in columns.
    Files,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Row {
    pub name: String,
    pub value: String,
    pub trend: Option<Trend>,
    pub freshness: Option<Freshness>,
    /// Whether choosing it opens a page of its own.
    pub opens: bool,
}

impl Row {
    /// A row that only states a fact.
    pub fn line(text: impl Into<String>) -> Row {
        Row {
            name: text.into(),
            value: String::new(),
            trend: None,
            freshness: None,
            opens: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Page {
    pub title: String,
    /// Whether choosing the title goes back up a level.
    pub back: bool,
    pub shape: Shape,
    pub rows: Vec<Row>,
}

/// What the pointer is over on a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    Title,
    Row(usize),
}

/// The soil panel as the app has it open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    pub page: Page,
    /// The viewport column it centres on.
    pub anchor: i32,
    /// The title or row the pointer marks: only one that opens something.
    pub pointer: Option<Target>,
}

/// A monument or a fossil, by truth's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Landmark {
    Monument(u32),
    Fossil(u32),
}

/// A hover label: a monument's year on the small plate, a fossil's on a one-line panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub landmark: Landmark,
    pub text: String,
}

/// The hover layer: the soil panel and a hover label, each when shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hover {
    pub panel: Option<Panel>,
    pub tag: Option<Tag>,
}

/// Where a page's parts lie in the viewport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The whole panel, at its place when risen.
    pub area: Area,
    /// The title's row, the width of the content.
    pub title: Area,
    /// Each row, in the page's order: the content's width for a list, a column for files.
    pub rows: Vec<Area>,
    columns: Columns,
}

/// The columns a page's text is set in, as offsets from the content's left edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Columns {
    content_x: i32,
    content_w: i32,
    /// A list's value column, and its icons'.
    value_x: i32,
    icons_x: i32,
    /// A page of files: each column's width, and the characters its paths and counts get.
    file_w: i32,
    path_chars: usize,
    count_chars: usize,
}

/// The border the nine-slice draws, and the space inside it.
const INSET: i32 = 3;
const PAD: i32 = 1;
/// The pointer's column, and the gap after it.
const POINTER_W: i32 = 6;
/// The title's cell is 7 rows, then a gap, the divider and a gap.
const TITLE_Y: i32 = INSET + PAD;
const DIVIDER_Y: i32 = TITLE_Y + 8;
const ROWS_Y: i32 = DIVIDER_Y + 2;
/// One row of body text and the gap below it.
const PITCH: i32 = 7;
/// Between a list's name and its value, and its value and icons.
const NAME_GAP: i32 = 6;
const VALUE_GAP: i32 = 4;
/// The trend icon, a gap, the freshness icon.
const ICONS_W: i32 = 12;
const ICON: i32 = 5;
const FILE_GAP: i32 = 8;

/// The rows a page shows: up to seven, or up to 21 files.
pub fn shown_rows(page: &Page) -> usize {
    let most = match page.shape {
        Shape::List => ROWS,
        Shape::Files => ROWS * FILE_COLUMNS,
    };
    page.rows.len().min(most)
}

fn chars(text: &str) -> i32 {
    i32::try_from(text.chars().count().min(MAX_CHARS)).unwrap_or(0)
}

/// Where `page` lies centred on `anchor` in a viewport `columns` wide, standing on its bottom
/// edge, inside the glass.
pub fn layout(catalog: &Catalog, page: &Page, anchor: i32, columns: i32) -> Option<Layout> {
    let body = Text::new(catalog, Face::Body)?.advance();
    let heading = Text::new(catalog, Face::Headings)?.advance();
    let shown = shown_rows(page);
    let rows = &page.rows[..shown];
    let title_w = POINTER_W + chars(&page.title) * heading;
    let lines = i32::try_from(shown.clamp(1, ROWS)).unwrap_or(1);
    let room = columns - 2 * LABEL_X - 2 * (INSET + PAD + 1);
    let (content_w, mut cols) = match page.shape {
        Shape::List => {
            let name = rows.iter().map(|r| chars(&r.name)).max().unwrap_or(0) * body;
            let value = rows.iter().map(|r| chars(&r.value)).max().unwrap_or(0) * body;
            let value_x = POINTER_W + name + NAME_GAP;
            let icons_x = value_x + value + VALUE_GAP;
            let cols = Columns {
                content_x: 0,
                content_w: 0,
                value_x,
                icons_x,
                file_w: 0,
                path_chars: 0,
                count_chars: 0,
            };
            ((icons_x + ICONS_W).max(title_w), cols)
        }
        Shape::Files => {
            let count_chars = rows.iter().map(|r| chars(&r.value)).max().unwrap_or(0);
            let path = rows.iter().map(|r| chars(&r.name)).max().unwrap_or(0);
            let used = i32::try_from(shown.div_ceil(ROWS).clamp(1, FILE_COLUMNS)).unwrap_or(1);
            let widest = (room - (used - 1) * FILE_GAP) / used / body;
            let cell = (path + 1 + count_chars).min(widest).max(count_chars + 2);
            let file_w = cell * body;
            let cols = Columns {
                content_x: 0,
                content_w: 0,
                value_x: 0,
                icons_x: 0,
                file_w,
                path_chars: usize::try_from(cell - 1 - count_chars).unwrap_or(1),
                count_chars: usize::try_from(count_chars).unwrap_or(0),
            };
            ((used * file_w + (used - 1) * FILE_GAP).max(title_w), cols)
        }
    };
    let content_w = content_w.min(room.max(1));
    let width = content_w + 2 * (INSET + PAD + 1);
    let height = ROWS_Y + lines * PITCH + PAD + INSET - 1;
    let left = (anchor - width / 2).clamp(LABEL_X, (columns - LABEL_X - width).max(LABEL_X));
    let top = i32::try_from(NATIVE_HEIGHT).unwrap_or(74) - height;
    let content_x = left + INSET + PAD + 1;
    cols.content_x = content_x;
    cols.content_w = content_w;
    let row_area = |i: usize| -> Area {
        let i = i32::try_from(i).unwrap_or(0);
        match page.shape {
            Shape::List => Area {
                x: content_x,
                y: top + ROWS_Y + i * PITCH - 1,
                w: content_w,
                h: PITCH,
            },
            Shape::Files => {
                let per = i32::try_from(ROWS).unwrap_or(7);
                let (column, line) = (i / per, i % per);
                Area {
                    x: content_x + column * (cols.file_w + FILE_GAP),
                    y: top + ROWS_Y + line * PITCH - 1,
                    w: cols.file_w,
                    h: PITCH,
                }
            }
        }
    };
    Some(Layout {
        area: Area {
            x: left,
            y: top,
            w: width,
            h: height,
        },
        title: Area {
            x: content_x,
            y: top + TITLE_Y - 1,
            w: content_w,
            h: DIVIDER_Y - TITLE_Y + 1,
        },
        rows: (0..shown).map(row_area).collect(),
        columns: cols,
    })
}

impl Layout {
    /// What `(x, y)` in the viewport is over: the title or a row, if any.
    pub fn target_at(&self, x: i32, y: i32) -> Option<Target> {
        if contains(self.title, x, y) {
            return Some(Target::Title);
        }
        self.rows
            .iter()
            .position(|&area| contains(area, x, y))
            .map(Target::Row)
    }

    /// Whether `(x, y)` lies on the panel.
    pub fn covers(&self, x: i32, y: i32) -> bool {
        contains(self.area, x, y)
    }
}

pub fn contains(area: Area, x: i32, y: i32) -> bool {
    x >= area.x && x < area.x + area.w && y >= area.y && y < area.y + area.h
}

/// The parts a page is drawn in, bottom to top.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    Frame,
    Divider,
    Text,
    Icons,
    Pointer,
}

impl Part {
    pub const ALL: [Part; 5] = [
        Part::Frame,
        Part::Divider,
        Part::Text,
        Part::Icons,
        Part::Pointer,
    ];

    /// Its place bottom to top, as a draw's part.
    pub const fn index(self) -> u8 {
        match self {
            Part::Frame => 0,
            Part::Divider => 1,
            Part::Text => 2,
            Part::Icons => 3,
            Part::Pointer => 4,
        }
    }
}

/// The art the hover layer draws with.
#[derive(Clone, Copy, Debug)]
pub struct PanelArt<'a> {
    catalog: &'a Catalog,
    frame: NineSlice<'a>,
    body: Text<'a>,
    heading: Text<'a>,
    divider: SpriteId,
    pointer: SpriteId,
    trends: [SpriteId; 3],
    freshness: [SpriteId; 2],
    plate: SpriteId,
}

impl<'a> PanelArt<'a> {
    /// `None` if the catalog lacks a piece, which the embedded art has.
    pub fn new(catalog: &'a Catalog) -> Option<PanelArt<'a>> {
        let id = |name: &str| catalog.id(name);
        Some(PanelArt {
            catalog,
            frame: NineSlice::new(catalog)?,
            body: Text::new(catalog, Face::Body)?,
            heading: Text::new(catalog, Face::Headings)?,
            divider: id("tmp.ui.divider")?,
            pointer: id("tmp.ui.row.pointer")?,
            trends: [
                id("tmp.ui.trend.up")?,
                id("tmp.ui.trend.flat")?,
                id("tmp.ui.trend.down")?,
            ],
            freshness: [
                id("tmp.ui.freshness.stale")?,
                id("tmp.ui.freshness.unknown")?,
            ],
            plate: id("tmp.ui.hover.plate.small")?,
        })
    }

    fn at(&self, sprite: SpriteId, x: i32, y: i32) -> Draw {
        Draw::in_layer(self.catalog, sprite, Layer::SoilPanel, x, y)
    }

    /// `page` drawn as `layout` places it, raised by `drop` rows less than its place, each
    /// draw with its part. Without the pointer, which [`PanelArt::pointer`] draws.
    pub fn page(&self, page: &Page, layout: &Layout, drop: i32) -> Vec<(Part, Draw)> {
        let mut out = Vec::new();
        let area = Area {
            y: layout.area.y + drop,
            ..layout.area
        };
        out.extend(
            self.frame
                .draws(area, Layer::SoilPanel)
                .into_iter()
                .map(|draw| (Part::Frame, draw)),
        );
        let cols = layout.columns;
        let top = area.y;
        for x in cols.content_x..cols.content_x + cols.content_w {
            out.push((Part::Divider, self.at(self.divider, x, top + DIVIDER_Y)));
        }
        let ink = Slot::STONE_LIGHT;
        let text = |face: &Text<'_>, line: &str, x: i32, y: i32| {
            face.draws(line, x, y, ink, Layer::SoilPanel)
                .into_iter()
                .map(|draw| (Part::Text, draw))
                .collect::<Vec<_>>()
        };
        let title = fit(&page.title, MAX_CHARS);
        out.extend(text(
            &self.heading,
            &title,
            cols.content_x + POINTER_W,
            top + TITLE_Y,
        ));
        for (row, area) in page.rows.iter().zip(&layout.rows) {
            let y = area.y + drop + 1;
            match page.shape {
                Shape::List => {
                    let name = fit(&row.name, MAX_CHARS);
                    out.extend(text(&self.body, &name, area.x + POINTER_W, y));
                    let value = fit(&row.value, MAX_CHARS);
                    out.extend(text(&self.body, &value, cols.content_x + cols.value_x, y));
                    let icons = cols.content_x + cols.icons_x;
                    let icon_y = y;
                    if let Some(trend) = row.trend {
                        let sprite = self.trends[match trend {
                            Trend::Up => 0,
                            Trend::Flat => 1,
                            Trend::Down => 2,
                        }];
                        out.push((Part::Icons, self.at(sprite, icons, icon_y)));
                    }
                    if let Some(freshness) = row.freshness {
                        let sprite = self.freshness[match freshness {
                            Freshness::Stale => 0,
                            Freshness::Unknown => 1,
                        }];
                        out.push((Part::Icons, self.at(sprite, icons + ICON + 2, icon_y)));
                    }
                }
                Shape::Files => {
                    let path = fit_tail(&row.name, cols.path_chars);
                    out.extend(text(&self.body, &path, area.x, y));
                    let count = fit(&row.value, cols.count_chars);
                    let right = area.x + area.w - self.body.width(&count);
                    out.extend(text(&self.body, &count, right, y));
                }
            }
        }
        out
    }

    /// The pointer at `target` on a page `layout` places, lowered by `drop`.
    pub fn pointer(&self, layout: &Layout, target: Target, drop: i32) -> Option<Draw> {
        let area = match target {
            Target::Title => layout.title,
            Target::Row(i) => *layout.rows.get(i)?,
        };
        let y = match target {
            Target::Title => area.y + 2,
            Target::Row(_) => area.y + 1,
        };
        Some(self.at(self.pointer, area.x, y + drop))
    }

    /// A monument's label: the small plate centred over `(x, top)`, the stone's top, with its
    /// text centred in the plate's safe area, kept inside `columns`.
    pub fn plate(&self, text: &str, x: i32, top: i32, columns: i32) -> Vec<Draw> {
        let hover = self.catalog.ui.hover_plate;
        let (w, h) = (i32::from(hover.size.w), i32::from(hover.size.h));
        let left = (x - w / 2).clamp(LABEL_X, (columns - LABEL_X - w).max(LABEL_X));
        let y = (top - h - 1).max(0);
        let text = fit(text, usize::from(hover.max_body_characters));
        let safe = hover.safe_text;
        let spare = i32::from(safe.w) - self.body.width(&text);
        let mut out = vec![self.at(self.plate, left, y)];
        out.extend(self.body.draws(
            &text,
            left + i32::from(safe.x) + spare / 2,
            y + i32::from(safe.y),
            Slot::STONE_LIGHT,
            Layer::SoilPanel,
        ));
        out
    }

    /// A fossil's label: one line on a panel centred over `x`, its bottom just above `bottom`,
    /// kept inside `columns`.
    pub fn note(&self, text: &str, x: i32, bottom: i32, columns: i32) -> Vec<Draw> {
        let text = fit_tail(text, MAX_CHARS);
        let w = self.body.width(&text) + 2 * (INSET + PAD);
        let h = 6 + 2 * (INSET + PAD) - 1;
        let left = (x - w / 2).clamp(LABEL_X, (columns - LABEL_X - w).max(LABEL_X));
        let y = bottom - h - 1;
        let mut out = self
            .frame
            .draws(Area { x: left, y, w, h }, Layer::SoilPanel);
        out.extend(self.body.draws(
            &text,
            left + INSET + PAD,
            y + INSET + PAD,
            Slot::STONE_LIGHT,
            Layer::SoilPanel,
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Art;

    fn catalog() -> &'static Catalog {
        &Art::embedded().catalog
    }

    fn row(name: &str, value: &str) -> Row {
        Row {
            name: name.to_owned(),
            value: value.to_owned(),
            trend: Some(Trend::Up),
            freshness: Some(Freshness::Stale),
            opens: true,
        }
    }

    fn page(shape: Shape, rows: usize) -> Page {
        Page {
            title: "climate".to_owned(),
            back: false,
            shape,
            rows: (0..rows)
                .map(|i| row(&format!("src/moss/fern_{i}.rs"), "48k lines"))
                .collect(),
        }
    }

    #[test]
    fn every_page_stands_on_the_frame_s_bottom_inside_the_glass() {
        for columns in [640, 651, 735, 840] {
            for shape in [Shape::List, Shape::Files] {
                for rows in [0, 1, 3, 7, 8, 14, 20, 21, 30] {
                    for anchor in [-50, 0, 10, 320, columns / 2, columns - 1, columns + 50] {
                        let page = page(shape, rows);
                        let layout = layout(catalog(), &page, anchor, columns).unwrap();
                        let area = layout.area;
                        assert_eq!(area.y + area.h, 74, "{shape:?} {rows}");
                        assert!(area.y >= 0, "{shape:?} {rows} fits the scene");
                        assert!(area.x >= LABEL_X, "{shape:?} {rows} at {anchor}");
                        assert!(area.x + area.w <= columns - LABEL_X, "{shape:?} {rows}");
                        assert_eq!(layout.rows.len(), shown_rows(&page));
                        for row in &layout.rows {
                            assert!(row.y > layout.title.y + layout.title.h);
                            assert!(row.y + row.h <= area.y + area.h);
                            assert!(row.x >= area.x && row.x + row.w <= area.x + area.w);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_list_shows_seven_rows_and_files_fill_three_columns_of_seven() {
        let list = page(Shape::List, 9);
        assert_eq!(shown_rows(&list), 7);
        let full = layout(catalog(), &list, 320, 640).unwrap();
        assert_eq!(full.area.h, 66);
        let files = layout(catalog(), &page(Shape::Files, 20), 320, 640).unwrap();
        let columns: Vec<i32> = files.rows.iter().map(|r| r.x).collect();
        assert_eq!(
            columns
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        assert_eq!(files.rows[0].x, files.rows[6].x);
        assert!(files.rows[7].x > files.rows[6].x);
        assert_eq!(files.rows[7].y, files.rows[0].y);
        let one = layout(catalog(), &page(Shape::Files, 7), 320, 640).unwrap();
        assert!(one.rows.iter().all(|r| r.x == one.rows[0].x));
        let short = layout(catalog(), &page(Shape::List, 3), 320, 640).unwrap();
        assert!(
            short.area.h < full.area.h,
            "a shorter page is a lower panel"
        );
    }

    #[test]
    fn the_pointer_hits_the_title_a_row_or_nothing() {
        let page = page(Shape::List, 7);
        let layout = layout(catalog(), &page, 320, 640).unwrap();
        let title = layout.title;
        assert_eq!(layout.target_at(title.x, title.y), Some(Target::Title));
        for (i, row) in layout.rows.iter().enumerate() {
            assert_eq!(layout.target_at(row.x, row.y), Some(Target::Row(i)));
            assert_eq!(
                layout.target_at(row.x + row.w - 1, row.y + row.h - 1),
                Some(Target::Row(i))
            );
        }
        let area = layout.area;
        assert_eq!(layout.target_at(area.x, area.y), None, "the border");
        assert!(layout.covers(area.x, area.y));
        assert!(!layout.covers(area.x - 1, area.y));
        assert!(!layout.covers(area.x, area.y - 1));
    }

    #[test]
    fn a_page_draws_its_frame_text_icons_and_pointer() {
        let art = PanelArt::new(catalog()).unwrap();
        let page = page(Shape::List, 2);
        let layout = layout(catalog(), &page, 320, 640).unwrap();
        let draws = art.page(&page, &layout, 0);
        let parts: std::collections::BTreeSet<Part> = draws.iter().map(|(p, _)| *p).collect();
        assert_eq!(
            parts,
            [Part::Frame, Part::Divider, Part::Text, Part::Icons].into()
        );
        assert_eq!(draws.iter().filter(|(p, _)| *p == Part::Icons).count(), 4);
        assert!(draws.iter().all(|(_, d)| d.layer == Layer::SoilPanel));
        assert!(
            draws
                .iter()
                .filter(|(p, _)| *p == Part::Text)
                .all(|(_, d)| d.tint == Some(Slot::STONE_LIGHT))
        );
        let lowered = art.page(&page, &layout, 5);
        assert!(
            draws
                .iter()
                .zip(&lowered)
                .all(|((_, a), (_, b))| b.y == a.y + 5)
        );
        let pointer = art.pointer(&layout, Target::Row(1), 0).unwrap();
        assert_eq!(
            (pointer.x, pointer.y),
            (layout.rows[1].x, layout.rows[1].y + 1)
        );
        assert!(art.pointer(&layout, Target::Row(2), 0).is_none());
    }
}
