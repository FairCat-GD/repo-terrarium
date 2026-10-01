//! Bitmap text in the delivered fonts, and the label plate.
//!
//! Glyphs are masks: each draw is tinted with one palette slot. Metrics come from the catalog,
//! which read them from the fonts manifest. This module draws strings it is given and authors
//! none; the words shown live in the app's copy module (I-8).

use crate::catalog::{Catalog, Font, Layer, Rect, SpriteId};
use crate::frame::Draw;
use crate::palette::Slot;

/// The two delivered fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// 4×6 cells, the 95 printable ASCII characters.
    Body,
    /// 5×7 cells, 65 characters and no lowercase: lowercase is drawn in uppercase, as its
    /// manifest's case policy says.
    Headings,
}

impl Face {
    /// The font's name in the catalog.
    pub const fn font_name(self) -> &'static str {
        match self {
            Face::Body => "body-4x6",
            Face::Headings => "headings-5x7",
        }
    }
}

/// A face, ready to lay out lines of text.
#[derive(Clone, Copy, Debug)]
pub struct Text<'a> {
    catalog: &'a Catalog,
    font: &'a Font,
    face: Face,
    fallback: SpriteId,
}

impl<'a> Text<'a> {
    /// `None` if the catalog lacks the font or its fallback glyph.
    pub fn new(catalog: &'a Catalog, face: Face) -> Option<Text<'a>> {
        let font = catalog.font(face.font_name())?;
        let fallback = font.glyph(font.fallback)?;
        Some(Text {
            catalog,
            font,
            face,
            fallback,
        })
    }

    /// The horizontal step from one character to the next.
    pub fn advance(&self) -> i32 {
        i32::from(self.font.advance)
    }

    /// The width of a line, a whole cell per character.
    pub fn width(&self, text: &str) -> i32 {
        i32::try_from(text.chars().count()).map_or(i32::MAX, |n| n.saturating_mul(self.advance()))
    }

    /// The glyph a character is drawn with: its own, its uppercase in the headings face, or
    /// the fallback (`?`).
    pub fn glyph(&self, c: char) -> SpriteId {
        let c = match self.face {
            Face::Body => c,
            Face::Headings => c.to_ascii_uppercase(),
        };
        self.font.glyph(u32::from(c)).unwrap_or(self.fallback)
    }

    /// One line of text whose cells start at `(x, y)`, the top-left of the first cell, every
    /// glyph tinted `ink`. Each glyph sits on the baseline by its own anchor.
    pub fn draws(&self, text: &str, x: i32, y: i32, ink: Slot, layer: Layer) -> Vec<Draw> {
        let baseline = y + i32::from(self.font.ascent);
        (0..)
            .zip(text.chars())
            .map(|(i, c)| {
                let glyph = self.glyph(c);
                let anchor = self.catalog.sprite(glyph).anchor;
                let pen = x + i * self.advance();
                Draw {
                    tint: Some(ink),
                    ..Draw::in_layer(
                        self.catalog,
                        glyph,
                        layer,
                        pen - i32::from(anchor.x),
                        baseline - i32::from(anchor.y),
                    )
                }
            })
            .collect()
    }
}

/// Fits a line into `limit` characters: a character outside ASCII becomes `?`, and a longer
/// line keeps its first `limit − 1` characters and ends in `~`.
pub fn fit(text: &str, limit: usize) -> String {
    let ascii = text.chars().map(|c| if c.is_ascii() { c } else { '?' });
    if text.chars().count() <= limit {
        return ascii.collect();
    }
    ascii
        .take(limit.saturating_sub(1))
        .chain((limit > 0).then_some('~'))
        .collect()
}

/// The main label: the stone plate with one line of body text in its text inset, in
/// `soil.deep`.
#[derive(Clone, Copy, Debug)]
pub struct Label<'a> {
    text: Text<'a>,
    plate: SpriteId,
    inset: Rect,
}

impl<'a> Label<'a> {
    pub const PLATE: &'static str = "tmp.ui.label.plate";

    /// `None` if the catalog lacks the plate, its text inset or the body font.
    pub fn new(catalog: &'a Catalog) -> Option<Label<'a>> {
        let plate = catalog.id(Label::PLATE)?;
        Some(Label {
            text: Text::new(catalog, Face::Body)?,
            plate,
            inset: catalog.sprite(plate).text_inset?,
        })
    }

    /// How many body characters fit the text inset: 18 for the delivered plate.
    pub fn capacity(&self) -> usize {
        usize::try_from(i32::from(self.inset.w) / self.text.advance()).unwrap_or(0)
    }

    /// The plate with its top-left at `(x, y)`, and `line` fitted to its capacity and centred
    /// in its inset. Descenders land one row below the inset, still on the plate.
    pub fn draws(&self, line: &str, x: i32, y: i32) -> Vec<Draw> {
        let mut draws = vec![self.plate(x, y)];
        draws.extend(self.text_draws(line, x, y));
        draws
    }

    /// The plate alone, its top-left at `(x, y)`.
    pub fn plate(&self, x: i32, y: i32) -> Draw {
        Draw::in_layer(self.text.catalog, self.plate, Layer::Label, x, y)
    }

    /// The text alone, as [`Label::draws`] lays it on a plate whose top-left is `(x, y)`.
    pub fn text_draws(&self, line: &str, x: i32, y: i32) -> Vec<Draw> {
        self.fitted_draws(&fit(line, self.capacity()), x, y)
    }

    /// As [`Label::text_draws`], for a line already fitted to [`Label::capacity`].
    pub fn fitted_draws(&self, line: &str, x: i32, y: i32) -> Vec<Draw> {
        let spare = i32::from(self.inset.w) - self.text.width(line);
        let text_x = x + i32::from(self.inset.x) + spare / 2;
        let text_y = y + i32::from(self.inset.y);
        self.text
            .draws(line, text_x, text_y, Slot::SOIL_DEEP, Layer::Label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Art;

    fn catalog() -> &'static Catalog {
        &Art::embedded().catalog
    }

    fn glyph_char(catalog: &Catalog, id: SpriteId) -> u32 {
        catalog
            .fonts
            .iter()
            .flat_map(|font| &font.glyphs)
            .find(|(_, glyph)| *glyph == id)
            .map(|(c, _)| *c)
            .expect("a glyph")
    }

    #[test]
    fn both_faces_and_their_fallbacks_exist() {
        for face in [Face::Body, Face::Headings] {
            let text = Text::new(catalog(), face).expect("the face");
            assert_eq!(
                glyph_char(catalog(), text.glyph('\u{2603}')),
                u32::from('?')
            );
        }
    }

    #[test]
    fn headings_draw_lowercase_in_uppercase_and_body_keeps_it() {
        let headings = Text::new(catalog(), Face::Headings).unwrap();
        assert_eq!(headings.glyph('a'), headings.glyph('A'));
        assert_eq!(glyph_char(catalog(), headings.glyph('q')), u32::from('Q'));
        let body = Text::new(catalog(), Face::Body).unwrap();
        assert_eq!(glyph_char(catalog(), body.glyph('q')), u32::from('q'));
        assert_ne!(body.glyph('a'), body.glyph('A'));
    }

    #[test]
    fn a_line_advances_a_cell_per_character_from_its_top_left() {
        let body = Text::new(catalog(), Face::Body).unwrap();
        let draws = body.draws("ab?", 10, 20, Slot::STONE_LIGHT, Layer::SoilPanel);
        let xs: Vec<i32> = draws.iter().map(|d| d.x).collect();
        assert_eq!(xs, [10, 14, 18]);
        // Body glyphs are 6 tall, anchored on the baseline at y = 5: cells start at y.
        assert!(draws.iter().all(|d| d.y == 20));
        assert!(draws.iter().all(|d| d.tint == Some(Slot::STONE_LIGHT)));
        assert!(draws.iter().all(|d| d.layer == Layer::SoilPanel));
        let headings = Text::new(catalog(), Face::Headings).unwrap();
        assert_eq!(headings.width("abc"), 15);
        // Characters, not bytes.
        assert_eq!(body.width("é"), 4);
    }

    #[test]
    fn fit_replaces_non_ascii_and_truncates_with_a_tilde() {
        assert_eq!(fit("repo-terrarium", 18), "repo-terrarium");
        assert_eq!(fit("exactly-eighteen..", 18), "exactly-eighteen..");
        assert_eq!(fit("feature/a-longer-branch", 18), "feature/a-longer-~");
        assert_eq!(fit("feature/a-longer-branch", 18).len(), 18);
        assert_eq!(fit("café · clear", 18), "caf? ? clear");
        // Counted in characters: eighteen non-ASCII characters still fit.
        assert_eq!(fit(&"é".repeat(18), 18), "?".repeat(18));
        assert_eq!(fit(&"é".repeat(19), 18), format!("{}~", "?".repeat(17)));
        assert_eq!(fit("abc", 0), "");
    }

    #[test]
    fn the_label_holds_eighteen_body_characters() {
        assert_eq!(Label::new(catalog()).unwrap().capacity(), 18);
    }

    #[test]
    fn the_label_centres_its_line_in_the_inset_in_soil_deep() {
        let label = Label::new(catalog()).unwrap();
        let draws = label.draws("abcd", 100, 50);
        assert_eq!((draws[0].x, draws[0].y), (100, 50));
        assert_eq!(draws[0].layer, Layer::Label);
        assert_eq!(draws.len(), 5);
        // The inset is 72 wide at (9, 2): four cells leave 56, so the line starts 28 in.
        assert_eq!((draws[1].x, draws[1].y), (100 + 9 + 28, 52));
        assert!(
            draws[1..]
                .iter()
                .all(|d| d.tint == Some(Slot::SOIL_DEEP) && d.layer == Layer::Label)
        );
        let long = label.draws("feature/a-longer-branch-name", 0, 0);
        assert_eq!(long.len(), 1 + 18);
        assert_eq!(long[1].x, 9);
        let tilde = catalog().font("body-4x6").unwrap().glyph(u32::from('~'));
        assert_eq!(Some(long[18].sprite), tilde);
    }
}
