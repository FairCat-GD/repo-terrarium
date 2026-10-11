//! The soil panel and the hover labels as the pointer has them.
//!
//! - **Hovering the soil band** raises the panel on the climate page, centred where the pointer
//!   came in. While it is up, the panel itself counts as hover too; moving off both lowers it.
//! - **A click on the soil band or the panel pins it**, so it stays when the pointer leaves. A
//!   click on a row that opens something shows that row's page; a click on a lower page's title
//!   goes back up a level. A click anywhere off the panel closes it, pinned or not.
//! - **Leaving the strip** lowers a panel that is not pinned. **Dim mode** closes even a pinned
//!   one, so nothing static is left on the screen.
//! - **A monument or a fossil** under the pointer shows its label while the panel is down. A
//!   fossil lies in the soil band, so over a fossil its label shows instead of the panel rising.
//!
//! The pages are worded here from the engine's facts and the copy module's words; which row
//! opens which page has one home, [`opens`], for the pages and the clicks alike.

use terrarium_compose::catalog::Catalog;
use terrarium_compose::frame::GROUND_Y;
use terrarium_compose::panel::{
    self, Freshness, Hover, Landmark, Layout, Page, Panel, Row, Shape, Tag, Target, Trend,
};
use terrarium_engine::Details;
use terrarium_store::Contributor;
use terrarium_truth::climate::Trend as TruthTrend;
use terrarium_truth::{Q16, Reading};

use crate::copy::panel as words;
use crate::hover::ViewportPoint;
use crate::live::PanelFacts;

/// A page of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// The five climates and the two habitat readings.
    Climate,
    /// Soil's three causes.
    Causes,
    /// The files behind a reading.
    Files(Source),
}

/// A reading whose files a page lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Light,
    Weather,
    DeadCode,
    Complexity,
    TodoDensity,
}

/// The climate page's rows, in order.
const CLIMATE_ROWS: usize = 7;

/// The page row `row` of `level` opens, if any.
pub fn opens(level: Level, row: usize) -> Option<Level> {
    match (level, row) {
        (Level::Climate, 0) => Some(Level::Files(Source::Light)),
        (Level::Climate, 1) => Some(Level::Causes),
        (Level::Climate, 4) => Some(Level::Files(Source::Weather)),
        (Level::Causes, 0) => Some(Level::Files(Source::DeadCode)),
        (Level::Causes, 1) => Some(Level::Files(Source::Complexity)),
        (Level::Causes, 2) => Some(Level::Files(Source::TodoDensity)),
        _ => None,
    }
}

/// The page a page's title goes back to.
fn parent(level: Level) -> Option<Level> {
    match level {
        Level::Climate => None,
        Level::Causes | Level::Files(Source::Light | Source::Weather) => Some(Level::Climate),
        Level::Files(_) => Some(Level::Causes),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Open {
    Closed,
    Hover,
    Pinned,
}

/// What the pointer has the panel do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoilPanel {
    open: Open,
    level: Level,
    /// The viewport column the panel centres on: where the pointer raised it.
    anchor: i32,
    /// Where the pointer is over the strip, in viewport pixels.
    point: Option<ViewportPoint>,
    /// The landmark under the pointer, while the panel is down.
    landmark: Option<Landmark>,
}

impl Default for SoilPanel {
    fn default() -> SoilPanel {
        SoilPanel {
            open: Open::Closed,
            level: Level::Climate,
            anchor: 0,
            point: None,
            landmark: None,
        }
    }
}

/// What lies under the pointer: the panel as it is drawn, and a landmark.
#[derive(Clone, Copy, Debug, Default)]
pub struct Hits<'a> {
    pub panel: Option<&'a Layout>,
    pub landmark: Option<Landmark>,
}

fn on_soil(point: ViewportPoint) -> bool {
    point.y >= GROUND_Y.cast_signed()
}

impl SoilPanel {
    fn on_panel(&self, hits: &Hits<'_>, point: ViewportPoint) -> bool {
        self.open != Open::Closed
            && hits
                .panel
                .is_some_and(|panel| panel.covers(point.x, point.y))
    }

    fn close(&mut self) {
        self.open = Open::Closed;
        self.level = Level::Climate;
        self.landmark = None;
    }

    fn raise(&mut self, open: Open, point: ViewportPoint) {
        self.open = open;
        self.level = Level::Climate;
        self.anchor = point.x;
        self.landmark = None;
    }

    /// The pointer moved to `point`, or off the strip's picture.
    pub fn moved(&mut self, point: Option<ViewportPoint>, hits: &Hits<'_>) {
        self.point = point;
        match self.open {
            Open::Closed => {
                self.landmark = point.and(hits.landmark);
                if let Some(point) = point.filter(|&p| self.landmark.is_none() && on_soil(p)) {
                    self.raise(Open::Hover, point);
                }
            }
            Open::Hover => {
                if !point.is_some_and(|p| on_soil(p) || self.on_panel(hits, p)) {
                    self.close();
                }
            }
            Open::Pinned => {}
        }
    }

    /// The left button went down where the pointer is.
    pub fn pressed(&mut self, hits: &Hits<'_>) {
        let Some(point) = self.point else {
            return;
        };
        if self.open == Open::Closed {
            if on_soil(point) {
                self.raise(Open::Pinned, point);
            }
            return;
        }
        if !self.on_panel(hits, point) {
            if self.open == Open::Hover && on_soil(point) {
                self.open = Open::Pinned;
            } else {
                self.close();
            }
            return;
        }
        self.open = Open::Pinned;
        let target = hits
            .panel
            .and_then(|panel| panel.target_at(point.x, point.y));
        let next = match target {
            Some(Target::Row(row)) => opens(self.level, row),
            Some(Target::Title) => parent(self.level),
            None => None,
        };
        if let Some(level) = next {
            self.level = level;
        }
    }

    /// The pointer left the strip.
    pub fn left(&mut self) {
        self.point = None;
        self.landmark = None;
        if self.open == Open::Hover {
            self.close();
        }
    }

    /// Dim mode began.
    pub fn dimmed(&mut self) {
        self.point = None;
        self.close();
    }

    /// The hover layer to draw: the panel's page worded from `facts` with the wall clock at
    /// `now` (Unix seconds), in a viewport `columns` wide, its pointer on the row under the
    /// pointer if that row opens something; or the label of the landmark under it.
    pub fn hover(
        &self,
        facts: Option<&PanelFacts>,
        now: i64,
        catalog: &Catalog,
        columns: i32,
    ) -> Hover {
        let Some(facts) = facts else {
            return Hover::default();
        };
        if self.open == Open::Closed {
            return Hover {
                panel: None,
                tag: self
                    .landmark
                    .and_then(|landmark| tag(landmark, &facts.details)),
            };
        }
        let page = page(self.level, facts, now);
        let pointer = self.point.and_then(|point| {
            let layout = panel::layout(catalog, &page, self.anchor, columns)?;
            match layout.target_at(point.x, point.y)? {
                Target::Title if page.back => Some(Target::Title),
                Target::Row(row) if page.rows.get(row).is_some_and(|r| r.opens) => {
                    Some(Target::Row(row))
                }
                _ => None,
            }
        });
        Hover {
            panel: Some(Panel {
                page,
                anchor: self.anchor,
                pointer,
            }),
            tag: None,
        }
    }
}

/// A landmark's label: a monument's year, a fossil's directory and year.
fn tag(landmark: Landmark, details: &Details) -> Option<Tag> {
    let text = match landmark {
        Landmark::Monument(id) => {
            let stone = details.monuments.iter().find(|m| m.id == id)?;
            words::year(stone.at)
        }
        Landmark::Fossil(id) => {
            let fossil = details.fossils.iter().find(|f| f.id == id)?;
            words::fossil(fossil.path.as_deref(), fossil.at)
        }
    };
    Some(Tag { landmark, text })
}

fn trend(trend: Option<TruthTrend>) -> Option<Trend> {
    trend.map(|trend| match trend {
        TruthTrend::Up => Trend::Up,
        TruthTrend::Flat => Trend::Flat,
        TruthTrend::Down => Trend::Down,
    })
}

/// A reading's value and freshness: its value and the stale mark, or `unknown` and its mark.
fn reading(reading: &Reading) -> (String, Option<Freshness>) {
    match reading {
        Reading::Known { value, stale, .. } => {
            (words::value(*value), stale.then_some(Freshness::Stale))
        }
        Reading::Unknown => (words::UNKNOWN.to_owned(), Some(Freshness::Unknown)),
    }
}

/// The page `level` shows of `facts`, with the wall clock at `now`.
pub fn page(level: Level, facts: &PanelFacts, now: i64) -> Page {
    match level {
        Level::Climate => climate(facts, now),
        Level::Causes => causes(facts),
        Level::Files(source) => files(source, facts),
    }
}

fn row(level: Level, index: usize, name: &str, value: String) -> Row {
    Row {
        name: name.to_owned(),
        value,
        trend: None,
        freshness: None,
        opens: opens(level, index).is_some(),
    }
}

fn climate(facts: &PanelFacts, now: i64) -> Page {
    let PanelFacts { report, details } = facts;
    let climate = &report.climate;
    let trends = &report.trends;
    let at = |index: usize, name: &str, read: &Reading, moved: Option<TruthTrend>| {
        let (value, freshness) = reading(read);
        Row {
            trend: trend(moved),
            freshness,
            ..row(Level::Climate, index, name, value)
        }
    };
    let weather = match climate.weather.state {
        Reading::Known { value, .. } if value >= Q16::HALF => words::PASS,
        Reading::Known { .. } => words::FAIL,
        Reading::Unknown => words::UNKNOWN,
    };
    let age = details.root.map_or_else(
        || words::UNKNOWN.to_owned(),
        |root| words::age((now - root).div_euclid(86_400)),
    );
    let rows = vec![
        at(0, words::LIGHT, &climate.light, trends.light),
        at(1, words::SOIL, &climate.soil, trends.soil),
        Row {
            trend: trend(trends.rain),
            ..row(Level::Climate, 2, words::RAIN, reading(&climate.rain).0)
        },
        Row {
            trend: trend(trends.wind),
            ..row(Level::Climate, 3, words::WIND, reading(&climate.wind).0)
        },
        Row {
            freshness: (weather == words::UNKNOWN).then_some(Freshness::Unknown),
            ..row(Level::Climate, 4, words::WEATHER, weather.to_owned())
        },
        Row {
            trend: trend(trends.terrain),
            ..row(
                Level::Climate,
                5,
                words::TERRAIN,
                words::lines(details.terrain),
            )
        },
        row(Level::Climate, 6, words::AGE, age),
    ];
    debug_assert_eq!(rows.len(), CLIMATE_ROWS);
    Page {
        title: words::CLIMATE.to_owned(),
        back: false,
        shape: Shape::List,
        rows,
    }
}

fn causes(facts: &PanelFacts) -> Page {
    let report = &facts.report;
    let causes = &report.causes;
    let trends = &report.trends;
    let rows = [
        (words::DEAD_CODE, &causes.dead_code, trends.dead_code),
        (words::COMPLEXITY, &causes.complexity, trends.complexity),
        (
            words::TODO_DENSITY,
            &causes.todo_density,
            trends.todo_density,
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (name, read, moved))| {
        let (value, freshness) = reading(read);
        Row {
            trend: trend(moved),
            freshness,
            ..row(Level::Causes, index, name, value)
        }
    })
    .collect();
    Page {
        title: words::SOIL.to_owned(),
        back: true,
        shape: Shape::List,
        rows,
    }
}

fn files(source: Source, facts: &PanelFacts) -> Page {
    let PanelFacts { report, details } = facts;
    let files = &details.files;
    let found = details.found;
    let known = |read: &Reading| matches!(read, Reading::Known { .. });
    let (title, listed, empty): (&str, &[Contributor], &str) = match source {
        Source::Light => (
            words::UNCOVERED_LINES,
            &files.light,
            if known(&report.climate.light) {
                words::NO_UNCOVERED_LINES
            } else if !found.enabled.coverage {
                words::COVERAGE_OFF
            } else if found.coverage {
                words::COVERAGE_NOT_READ
            } else {
                words::NO_COVERAGE_REPORT
            },
        ),
        Source::Weather => (
            words::FAILING_TESTS,
            &files.weather,
            match report.climate.weather.state {
                Reading::Known { value, .. } if value >= Q16::HALF => words::NO_FAILING_TESTS,
                Reading::Known { .. } => words::FAILING_UNNAMED,
                Reading::Unknown if !found.enabled.weather => words::TESTS_OFF,
                Reading::Unknown if found.test_report => words::TESTS_NOT_READ,
                Reading::Unknown => words::NO_TEST_REPORT,
            },
        ),
        Source::DeadCode => cause(words::DEAD_CODE, &files.dead_code, &report.causes.dead_code),
        Source::Complexity => cause(
            words::COMPLEXITY,
            &files.complexity,
            &report.causes.complexity,
        ),
        Source::TodoDensity => cause(
            words::TODO_DENSITY,
            &files.todo_density,
            &report.causes.todo_density,
        ),
    };
    let mut rows: Vec<Row> = Vec::new();
    if source == Source::Light
        && let Some(reading) = &details.branch_reading
    {
        let value = reading
            .value
            .map_or_else(|| words::UNKNOWN.to_owned(), words::value);
        rows.push(Row {
            value,
            ..Row::line(words::on_branch(&reading.branch))
        });
    }
    rows.extend(listed.iter().map(|file| Row {
        value: words::count(file.value),
        ..Row::line(file.path.clone())
    }));
    if listed.is_empty() {
        rows.push(Row::line(empty));
    }
    Page {
        title: title.to_owned(),
        back: true,
        shape: Shape::Files,
        rows,
    }
}

/// A soil cause's page: its files, or why there are none.
fn cause<'a>(
    title: &'a str,
    files: &'a [Contributor],
    read: &Reading,
) -> (&'a str, &'a [Contributor], &'a str) {
    let empty = match read {
        Reading::Known { .. } => words::NO_FILES,
        Reading::Unknown => words::NOT_MEASURED,
    };
    (title, files, empty)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use sha2::{Digest, Sha256};
    use terrarium_adapters::discovery::Enabled;
    use terrarium_compose::catalog::Art;
    use terrarium_compose::clock::LocalClock;
    use terrarium_compose::composer::{Composer, OverlayInput};
    use terrarium_compose::compositor::composite;
    use terrarium_compose::export;
    use terrarium_compose::frame::{Area, NATIVE_HEIGHT, Viewport};
    use terrarium_compose::preset;
    use terrarium_compose::scale::{SizeStep, StripScale};
    use terrarium_compose::tween::Millis;
    use terrarium_compose::variants::ArtTable;
    use terrarium_engine::details::BranchReading;
    use terrarium_engine::{Buried, Found, Raised};
    use terrarium_truth::climate::{ClimateReport, SoilCauses, Trends};
    use terrarium_truth::domain::{Climate, WeatherReading};
    use terrarium_truth::scene::{FossilForm, FossilView, MonumentAge, MonumentView, SceneInput};
    use terrarium_truth::{PresentationState, Tick};

    use super::*;
    use crate::hover::to_viewport;

    /// 2025-03-03, 12:00 UTC.
    const NOW: i64 = 1_741_003_200;
    const DAY: i64 = 86_400;

    fn catalog() -> &'static Catalog {
        &Art::embedded().catalog
    }

    /// 4× on a 3024 px display: 756 columns, the scene 58 of them in.
    fn scale() -> StripScale {
        StripScale::for_physical_width(3024, SizeStep::Full)
    }

    fn known(value: Q16, stale: bool) -> Reading {
        Reading::Known {
            value,
            stale,
            since_tick: Tick::ZERO,
        }
    }

    fn files(prefix: &str, n: i64) -> Vec<Contributor> {
        (0..n)
            .map(|i| Contributor {
                path: format!("{prefix}/module_{i:02}/fern_and_moss.rs"),
                value: 400 - 17 * i,
            })
            .collect()
    }

    /// A fictional terrarium with something of everything to show.
    fn facts() -> PanelFacts {
        let report = ClimateReport {
            climate: Climate {
                light: known(Q16::from_ratio(78, 100), false),
                soil: known(Q16::from_ratio(61, 100), true),
                rain: known(Q16::from_ratio(120, 100), false),
                wind: known(Q16::from_ratio(85, 100), false),
                weather: WeatherReading {
                    state: known(Q16::ZERO, false),
                    source: None,
                },
            },
            health: Q16::from_ratio(61, 100),
            activity: Q16::HALF,
            settling_days_left: None,
            trends: Trends {
                light: Some(TruthTrend::Up),
                soil: Some(TruthTrend::Flat),
                rain: Some(TruthTrend::Down),
                wind: None,
                health: None,
                dead_code: Some(TruthTrend::Up),
                complexity: None,
                todo_density: Some(TruthTrend::Down),
                terrain: Some(TruthTrend::Up),
            },
            causes: SoilCauses {
                dead_code: known(Q16::from_ratio(90, 100), false),
                complexity: Reading::Unknown,
                todo_density: known(Q16::from_ratio(72, 100), true),
            },
        };
        let details = Details {
            terrain: 48_210,
            root: Some(NOW - 400 * DAY),
            dormant_since: None,
            monuments: vec![Raised {
                id: 1,
                at: NOW - 35 * DAY,
            }],
            fossils: vec![Buried {
                id: 9,
                at: NOW - 700 * DAY,
                path: Some("src/old-roots".to_owned()),
            }],
            files: terrarium_engine::Files {
                light: files("src/garden", 20),
                weather: files("tests/beds", 3),
                dead_code: files("src/shed", 20),
                complexity: Vec::new(),
                todo_density: files("src/potting", 5),
            },
            branch_reading: Some(BranchReading {
                branch: "feature/moss".to_owned(),
                value: Some(Q16::from_ratio(81, 100)),
            }),
            found: Found {
                coverage: true,
                test_report: true,
                enabled: Enabled::default(),
            },
        };
        PanelFacts { report, details }
    }

    /// The same terrarium before any report: nothing but git.
    fn bare() -> PanelFacts {
        let mut facts = facts();
        let climate = &mut facts.report.climate;
        climate.light = Reading::Unknown;
        climate.soil = Reading::Unknown;
        climate.weather.state = Reading::Unknown;
        facts.report.causes.dead_code = Reading::Unknown;
        facts.report.causes.todo_density = Reading::Unknown;
        // An unknown reading has no trend (truth's `Trends::of`).
        let trends = &mut facts.report.trends;
        trends.light = None;
        trends.soil = None;
        trends.dead_code = None;
        trends.todo_density = None;
        facts.details.files = terrarium_engine::Files::default();
        facts.details.branch_reading = None;
        facts.details.found = Found::default();
        facts
    }

    /// The strip at 4×, as the pointer drives it: the panel, the hover layer drawn, and the
    /// landmarks under the pointer.
    struct Strip {
        soil: SoilPanel,
        shown: Hover,
        facts: PanelFacts,
        landmarks: Vec<(Area, Landmark)>,
    }

    impl Strip {
        fn new() -> Strip {
            Strip {
                soil: SoilPanel::default(),
                shown: Hover::default(),
                facts: facts(),
                landmarks: vec![
                    (
                        Area {
                            x: 400,
                            y: 30,
                            w: 20,
                            h: 35,
                        },
                        Landmark::Monument(1),
                    ),
                    (
                        Area {
                            x: 600,
                            y: 68,
                            w: 12,
                            h: 6,
                        },
                        Landmark::Fossil(9),
                    ),
                ],
            }
        }

        fn columns() -> i32 {
            i32::try_from(scale().columns).unwrap()
        }

        fn drawn(&self) -> Option<Layout> {
            let shown = self.shown.panel.as_ref()?;
            panel::layout(catalog(), &shown.page, shown.anchor, Strip::columns())
        }

        fn redraw(&mut self) {
            self.shown = self
                .soil
                .hover(Some(&self.facts), NOW, catalog(), Strip::columns());
        }

        /// The pointer at a physical position on the window.
        fn to(&mut self, x: f64, y: f64) {
            let point = to_viewport(x, y, &scale());
            let landmark = point.and_then(|p| {
                self.landmarks
                    .iter()
                    .find(|(area, _)| panel::contains(*area, p.x, p.y))
                    .map(|&(_, landmark)| landmark)
            });
            let drawn = self.drawn();
            self.soil.moved(
                point,
                &Hits {
                    panel: drawn.as_ref(),
                    landmark,
                },
            );
            self.redraw();
        }

        /// The pointer on a viewport pixel, at the middle of its physical pixels.
        fn at(&mut self, x: i32, y: i32) {
            self.to(f64::from(x) * 4.0 + 1.5, f64::from(y) * 4.0 + 1.5);
        }

        fn click(&mut self) {
            let drawn = self.drawn();
            self.soil.pressed(&Hits {
                panel: drawn.as_ref(),
                landmark: None,
            });
            self.redraw();
        }

        fn leave(&mut self) {
            self.soil.left();
            self.redraw();
        }

        fn title(&self) -> Option<&str> {
            Some(self.shown.panel.as_ref()?.page.title.as_str())
        }

        fn row(&self, row: usize) -> Area {
            self.drawn().unwrap().rows[row]
        }

        fn at_row(&mut self, row: usize) {
            let area = self.row(row);
            self.at(area.x + 1, area.y + 2);
        }

        fn at_title(&mut self) {
            let title = self.drawn().unwrap().title;
            self.at(title.x + 1, title.y + 2);
        }
    }

    #[test]
    fn hovering_the_soil_raises_the_climate_page_where_the_pointer_came_in() {
        let mut strip = Strip::new();
        strip.at(300, 40);
        assert_eq!(strip.shown, Hover::default(), "the sky raises nothing");
        strip.at(300, 66);
        assert_eq!(strip.title(), Some(words::CLIMATE));
        let panel = strip.shown.panel.as_ref().unwrap();
        assert_eq!(panel.anchor, 300);
        assert_eq!(panel.page.rows.len(), 7);
        // The soil band beside the scene, in a wider viewport, counts too.
        let mut wide = Strip::new();
        wide.at(5, 70);
        assert!(wide.shown.panel.is_some());
    }

    #[test]
    fn the_panel_stays_while_the_pointer_is_on_it_and_lowers_when_it_leaves_both() {
        let mut strip = Strip::new();
        strip.at(300, 70);
        strip.at_row(0);
        assert_eq!(strip.title(), Some(words::CLIMATE));
        let pointer = |strip: &Strip| strip.shown.panel.as_ref().unwrap().pointer;
        assert_eq!(
            pointer(&strip),
            Some(Target::Row(0)),
            "light opens its files"
        );
        strip.at_row(2);
        assert_eq!(pointer(&strip), None, "rain opens nothing");
        strip.at_title();
        assert_eq!(
            pointer(&strip),
            None,
            "the climate page has nowhere back to"
        );
        let area = strip.drawn().unwrap().area;
        strip.at(area.x - 1, area.y + 1);
        assert_eq!(strip.shown.panel, None);
    }

    #[test]
    fn hit_testing_is_in_native_pixels_after_the_scale() {
        let mut strip = Strip::new();
        strip.at(300, 70);
        let area = strip.drawn().unwrap().area;
        // The last physical pixel of the row above the panel is off it; the first of its top
        // row is on it.
        let top = f64::from(area.y) * 4.0;
        strip.to(f64::from(area.x + 2) * 4.0, top);
        assert!(strip.shown.panel.is_some());
        strip.to(f64::from(area.x + 2) * 4.0, top - 0.5);
        assert_eq!(strip.shown.panel, None);
    }

    #[test]
    fn a_click_pins_the_panel_through_leaving_and_rows_and_titles_move_between_pages() {
        let mut strip = Strip::new();
        strip.at(300, 70);
        strip.click();
        strip.at(300, 20);
        assert_eq!(strip.title(), Some(words::CLIMATE), "pinned, it stays");
        strip.leave();
        assert_eq!(strip.title(), Some(words::CLIMATE));
        strip.at_row(1);
        strip.click();
        assert_eq!(strip.title(), Some(words::SOIL));
        strip.at_row(1);
        strip.click();
        assert_eq!(strip.title(), Some(words::COMPLEXITY));
        strip.at_title();
        assert_eq!(
            strip.shown.panel.as_ref().unwrap().pointer,
            Some(Target::Title)
        );
        strip.click();
        assert_eq!(strip.title(), Some(words::SOIL));
        strip.at_title();
        strip.click();
        assert_eq!(strip.title(), Some(words::CLIMATE));
        strip.at_row(4);
        strip.click();
        assert_eq!(strip.title(), Some(words::FAILING_TESTS));
        strip.at_title();
        strip.click();
        assert_eq!(strip.title(), Some(words::CLIMATE));
        // A row that opens nothing leaves the page as it is.
        strip.at_row(3);
        strip.click();
        assert_eq!(strip.title(), Some(words::CLIMATE));
    }

    #[test]
    fn a_click_off_the_panel_closes_it_pinned_or_not_and_dim_mode_closes_it() {
        let mut strip = Strip::new();
        strip.at(300, 70);
        strip.click();
        strip.at(200, 10);
        strip.click();
        assert_eq!(strip.shown.panel, None);
        // Raised by hover and clicked on the soil beside it, it pins.
        strip.at(300, 70);
        let area = strip.drawn().unwrap().area;
        strip.at(area.x + area.w + 30, 70);
        strip.click();
        strip.at(100, 10);
        assert!(strip.shown.panel.is_some());
        strip.soil.dimmed();
        strip.redraw();
        assert_eq!(strip.shown, Hover::default());
        // A click on the soil with the panel down raises it pinned.
        strip.at(100, 10);
        strip.soil.point = Some(ViewportPoint { x: 100, y: 70 });
        strip.click();
        strip.leave();
        assert!(strip.shown.panel.is_some());
    }

    #[test]
    fn leaving_the_strip_lowers_a_panel_that_is_not_pinned() {
        let mut strip = Strip::new();
        strip.at(300, 70);
        strip.leave();
        assert_eq!(strip.shown, Hover::default());
        strip.to(-10.0, 280.0);
        assert_eq!(strip.shown, Hover::default());
    }

    #[test]
    fn a_landmark_shows_its_label_while_the_panel_is_down() {
        let mut strip = Strip::new();
        strip.at(405, 40);
        let tag = strip.shown.tag.clone().expect("the stone's label");
        assert_eq!(tag.landmark, Landmark::Monument(1));
        assert_eq!(tag.text, "2025");
        assert!(tag.text.chars().count() <= 5, "the plate holds five");
        // A fossil lies in the soil band: its label shows, and the panel stays down.
        strip.at(605, 70);
        assert_eq!(strip.shown.panel, None);
        let tag = strip.shown.tag.clone().expect("the fossil's label");
        assert_eq!(tag.text, "src/old-roots - 2023");
        // Off it, on the soil, the panel rises and no label shows with it, pinned or not.
        strip.at(300, 70);
        assert!(strip.shown.panel.is_some());
        assert_eq!(strip.shown.tag, None);
        strip.click();
        strip.at(405, 40);
        assert!(strip.shown.panel.is_some());
        assert_eq!(strip.shown.tag, None);
        strip.at(200, 10);
        strip.click();
        // A landmark the engine has not dated has no label.
        strip.leave();
        strip.facts.details.monuments.clear();
        strip.at(405, 40);
        assert_eq!(strip.shown, Hover::default());
    }

    #[test]
    fn nothing_shows_before_the_engine_has_said() {
        let mut soil = SoilPanel::default();
        soil.moved(Some(ViewportPoint { x: 300, y: 70 }), &Hits::default());
        assert_eq!(soil.open, Open::Hover);
        assert_eq!(soil.hover(None, NOW, catalog(), 756), Hover::default());
    }

    #[test]
    fn each_row_opens_what_its_page_says() {
        let facts = facts();
        for level in [
            Level::Climate,
            Level::Causes,
            Level::Files(Source::Light),
            Level::Files(Source::Weather),
            Level::Files(Source::DeadCode),
        ] {
            let page = page(level, &facts, NOW);
            assert_eq!(page.back, parent(level).is_some(), "{level:?}");
            for (i, row) in page.rows.iter().enumerate() {
                assert_eq!(row.opens, opens(level, i).is_some(), "{level:?} {i}");
            }
        }
    }

    #[test]
    fn the_climate_page_states_each_reading() {
        let page = page(Level::Climate, &facts(), NOW);
        let rows: Vec<(&str, &str, Option<Trend>, Option<Freshness>)> = page
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.value.as_str(), r.trend, r.freshness))
            .collect();
        assert_eq!(
            rows,
            [
                ("light", "0.78", Some(Trend::Up), None),
                ("soil", "0.61", Some(Trend::Flat), Some(Freshness::Stale)),
                ("rain", "1.20", Some(Trend::Down), None),
                ("wind", "0.85", None, None),
                ("weather", "fail", None, None),
                ("terrain", "48k lines", Some(Trend::Up), None),
                ("age", "13 months", None, None),
            ]
        );
        let bare = page_of(Level::Climate, &bare());
        assert_eq!(bare.rows[0].value, "unknown");
        assert_eq!(bare.rows[0].freshness, Some(Freshness::Unknown));
        assert_eq!(bare.rows[4].value, "unknown");
        assert_eq!(bare.rows[4].freshness, Some(Freshness::Unknown));
    }

    fn page_of(level: Level, facts: &PanelFacts) -> Page {
        page(level, facts, NOW)
    }

    #[test]
    fn a_page_of_files_lists_them_or_says_why_it_cannot() {
        let light = page_of(Level::Files(Source::Light), &facts());
        assert_eq!(light.shape, Shape::Files);
        assert_eq!(light.rows.len(), 21);
        assert_eq!(
            (light.rows[0].name.as_str(), light.rows[0].value.as_str()),
            ("on feature/moss", "0.81")
        );
        assert_eq!(light.rows[1].value, "400");
        let lines = |level: Level, facts: &PanelFacts| -> Vec<String> {
            page_of(level, facts)
                .rows
                .into_iter()
                .map(|r| r.name)
                .collect()
        };
        let bare = bare();
        assert_eq!(
            lines(Level::Files(Source::Light), &bare),
            [words::NO_COVERAGE_REPORT]
        );
        assert_eq!(
            lines(Level::Files(Source::Weather), &bare),
            [words::NO_TEST_REPORT]
        );
        assert_eq!(
            lines(Level::Files(Source::Complexity), &bare),
            [words::NOT_MEASURED]
        );
        let mut found = bare.clone();
        found.details.found = Found {
            coverage: true,
            test_report: true,
            enabled: Enabled::default(),
        };
        assert_eq!(
            lines(Level::Files(Source::Light), &found),
            [words::COVERAGE_NOT_READ]
        );
        assert_eq!(
            lines(Level::Files(Source::Weather), &found),
            [words::TESTS_NOT_READ]
        );
        // Found, but off in the settings: the page says so, whether or not a report is there.
        let mut off = found.clone();
        off.details.found.enabled = Enabled {
            coverage: false,
            dead_code: true,
            weather: false,
        };
        assert_eq!(
            lines(Level::Files(Source::Light), &off),
            [words::COVERAGE_OFF]
        );
        assert_eq!(
            lines(Level::Files(Source::Weather), &off),
            [words::TESTS_OFF]
        );
        let mut read = facts();
        read.details.files = terrarium_engine::Files::default();
        read.details.branch_reading = None;
        assert_eq!(
            lines(Level::Files(Source::Light), &read),
            [words::NO_UNCOVERED_LINES]
        );
        assert_eq!(
            lines(Level::Files(Source::Weather), &read),
            [words::FAILING_UNNAMED]
        );
        assert_eq!(
            lines(Level::Files(Source::DeadCode), &read),
            [words::NO_FILES]
        );
        read.report.climate.weather.state = known(Q16::ONE, false);
        assert_eq!(
            lines(Level::Files(Source::Weather), &read),
            [words::NO_FAILING_TESTS]
        );
    }

    // ---- Pictures of every level -----------------------------------------------------------

    fn snapshot(name: &str, scene: &SceneInput, hover: &Hover, night: bool, rise_ms: Option<u64>) {
        let art = Art::embedded();
        let table = ArtTable::embedded();
        let mut composer = Composer::new(table, preset::SALT);
        composer.set_viewport(Viewport::SCENE);
        let clock = if night {
            LocalClock::midnight(preset::DATE)
        } else {
            LocalClock::noon(preset::DATE)
        };
        let overlay = OverlayInput::new(vec!["moss".to_owned()], clock);
        composer.update(scene, &overlay, Millis(0));
        let settled = composer.settled_at();
        composer.hover(hover, settled);
        let at = rise_ms.map_or_else(|| composer.settled_at(), |ms| Millis(settled.0 + ms));
        let scenery = composer.scenery(art, table);
        let frame = composer.frame(art, table, &scenery, composer.palette_mix(at), at);
        let image = composite(art, &frame);
        let hex = Sha256::digest(image.pixels().as_flattened()).iter().fold(
            String::new(),
            |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            },
        );
        let width = image.width();
        insta::assert_snapshot!(
            format!("{name}-pixels"),
            format!("{width}x{NATIVE_HEIGHT} RGBA8, SHA-256 {hex}")
        );
        insta::assert_binary_snapshot!(
            format!("{name}.png").as_str(),
            export::png(width, NATIVE_HEIGHT, image.pixels())
        );
    }

    fn scene() -> SceneInput {
        let mut scene = preset::scene("thriving").unwrap();
        scene.monuments = vec![MonumentView {
            id: 1,
            age: MonumentAge::Lichened,
            marks: 1,
        }];
        scene.fossils = vec![FossilView {
            id: 9,
            form: FossilForm::Shell,
        }];
        scene
    }

    fn opened(level: Level, facts: &PanelFacts, anchor: i32, pointer: Option<Target>) -> Hover {
        Hover {
            panel: Some(Panel {
                page: page(level, facts, NOW),
                anchor,
                pointer,
            }),
            tag: None,
        }
    }

    #[test]
    fn every_level_of_the_panel_as_drawn() {
        let facts = facts();
        let scene = scene();
        let pages = [
            ("panel-climate", Level::Climate, Some(Target::Row(0))),
            ("panel-causes", Level::Causes, Some(Target::Title)),
            ("panel-files-light", Level::Files(Source::Light), None),
            ("panel-files-weather", Level::Files(Source::Weather), None),
            (
                "panel-files-dead-code",
                Level::Files(Source::DeadCode),
                None,
            ),
        ];
        for (name, level, pointer) in pages {
            snapshot(
                name,
                &scene,
                &opened(level, &facts, 200, pointer),
                false,
                None,
            );
        }
        let bare = bare();
        for (name, level) in [
            ("panel-climate-bare", Level::Climate),
            ("panel-files-light-bare", Level::Files(Source::Light)),
            (
                "panel-files-complexity-bare",
                Level::Files(Source::Complexity),
            ),
        ] {
            snapshot(name, &scene, &opened(level, &bare, 560, None), false, None);
        }
        let climate = opened(Level::Climate, &facts, 200, None);
        snapshot("panel-climate-night", &scene, &climate, true, None);
        snapshot("panel-rising", &scene, &climate, false, Some(150));
    }

    #[test]
    fn the_hover_labels_as_drawn() {
        let facts = facts();
        let scene = scene();
        for (name, landmark) in [
            ("label-monument", Landmark::Monument(1)),
            ("label-fossil", Landmark::Fossil(9)),
        ] {
            let hover = Hover {
                panel: None,
                tag: tag(landmark, &facts.details),
            };
            assert!(hover.tag.is_some());
            snapshot(name, &scene, &hover, false, None);
        }
    }

    #[test]
    fn the_preset_scene_is_thriving() {
        assert_eq!(
            scene().presentation.state,
            PresentationState::Thriving,
            "the pictures stand on a thriving scene"
        );
    }
}
