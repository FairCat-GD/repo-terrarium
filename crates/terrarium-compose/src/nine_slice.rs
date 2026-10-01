//! The nine-slice panel: four corners, edges and a centre, from 3×3 tiles.
//!
//! The edges tile along their axis and the centre tiles both ways. A repeat that would run
//! past its band is cropped by a clip, never overlapped, so a panel fading in blends every
//! pixel once.

use crate::catalog::{Catalog, Layer, SpriteId};
use crate::frame::{Area, Draw};

/// The tiles, row by row: top-left to bottom-right.
const TILES: [&str; 9] = [
    "tmp.ui.panel.nw",
    "tmp.ui.panel.n",
    "tmp.ui.panel.ne",
    "tmp.ui.panel.w",
    "tmp.ui.panel.c",
    "tmp.ui.panel.e",
    "tmp.ui.panel.sw",
    "tmp.ui.panel.s",
    "tmp.ui.panel.se",
];

#[derive(Clone, Copy, Debug)]
pub struct NineSlice<'a> {
    catalog: &'a Catalog,
    tiles: [SpriteId; 9],
    tile: i32,
    minimum_w: i32,
    minimum_h: i32,
}

impl<'a> NineSlice<'a> {
    /// `None` if the catalog lacks a tile.
    pub fn new(catalog: &'a Catalog) -> Option<NineSlice<'a>> {
        let mut tiles = [SpriteId::from_index(0)?; 9];
        for (tile, name) in tiles.iter_mut().zip(TILES) {
            *tile = catalog.id(name)?;
        }
        let metrics = catalog.ui.nine_slice;
        Some(NineSlice {
            catalog,
            tiles,
            tile: i32::from(metrics.tile_size),
            minimum_w: i32::from(metrics.minimum.w),
            minimum_h: i32::from(metrics.minimum.h),
        })
    }

    /// A panel filling `area`, grown to the art's minimum size if smaller.
    pub fn draws(&self, area: Area, layer: Layer) -> Vec<Draw> {
        let (w, h) = (area.w.max(self.minimum_w), area.h.max(self.minimum_h));
        let t = self.tile;
        let columns = bands(area.x, w, t);
        let rows = bands(area.y, h, t);
        let mut draws = Vec::new();
        for (row, &(top, bottom)) in rows.iter().enumerate() {
            for (column, &(left, right)) in columns.iter().enumerate() {
                let tile = self.tiles[row * 3 + column];
                let band = Area {
                    x: left,
                    y: top,
                    w: right - left,
                    h: bottom - top,
                };
                for y in steps(top, bottom, t) {
                    for x in steps(left, right, t) {
                        let overhangs = x + t > right || y + t > bottom;
                        draws.push(Draw {
                            clip: overhangs.then_some(band),
                            ..Draw::in_layer(self.catalog, tile, layer, x, y)
                        });
                    }
                }
            }
        }
        draws
    }
}

/// A side's three bands: the first tile, the repeats between, and the last tile.
fn bands(start: i32, length: i32, tile: i32) -> [(i32, i32); 3] {
    let end = start + length;
    [
        (start, start + tile),
        (start + tile, end - tile),
        (end - tile, end),
    ]
}

/// Where each tile of a band starts.
fn steps(from: i32, to: i32, tile: i32) -> impl Iterator<Item = i32> {
    std::iter::successors(Some(from), move |at| Some(at + tile)).take_while(move |&at| at < to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::blend;
    use crate::catalog::{Art, Blend};
    use crate::compositor::composite;
    use crate::frame::Frame;
    use crate::palette::{Lut, Slot};

    fn panel(area: Area) -> Vec<Draw> {
        let catalog = &Art::embedded().catalog;
        NineSlice::new(catalog)
            .expect("the panel tiles")
            .draws(area, Layer::SoilPanel)
    }

    fn area(x: i32, y: i32, w: i32, h: i32) -> Area {
        Area { x, y, w, h }
    }

    #[test]
    fn a_panel_smaller_than_the_minimum_grows_to_9_by_9() {
        let draws = panel(area(20, 10, 4, 4));
        assert_eq!(draws.len(), 9);
        assert!(draws.iter().all(|d| d.clip.is_none()));
        let right = draws.iter().map(|d| d.x).max().unwrap();
        assert_eq!(right, 20 + 6);
    }

    #[test]
    fn edges_and_centre_repeat_and_only_the_final_repeat_is_cropped() {
        // 10 wide: the top band runs 3..7, a whole tile at 3 and a cropped one at 6.
        let draws = panel(area(0, 0, 10, 9));
        let top: Vec<&Draw> = draws.iter().filter(|d| d.y == 0).collect();
        let xs: Vec<i32> = top.iter().map(|d| d.x).collect();
        assert_eq!(xs, [0, 3, 6, 7]);
        assert_eq!(top[1].clip, None);
        assert_eq!(top[2].clip, Some(area(3, 0, 4, 3)));
        assert_eq!(top[3].clip, None, "the corner is never cropped");
        // Three rows of four: a corner or edge, two repeats, and a corner or edge.
        assert_eq!(draws.len(), 12);
    }

    #[test]
    fn a_panel_paints_exactly_its_area_with_a_solid_centre() {
        let art = Art::embedded();
        for (w, h) in [(9, 9), (10, 10), (11, 13), (40, 30)] {
            let at = area(100, 20, w, h);
            let out = composite(
                art,
                &Frame {
                    draws: panel(at),
                    ..Frame::default()
                },
            );
            let lut = Lut::new(&art.catalog.palette, 0);
            let colour = |slot| {
                let [r, g, b] = lut.colour(slot);
                [r, g, b, 255]
            };
            let sky = colour(Slot::SKY_BASE);
            let centre = colour(Slot::new(1).unwrap());
            for y in 0..74 {
                for x in 80..160 {
                    let (xi, yi) = (i32::try_from(x).unwrap(), i32::try_from(y).unwrap());
                    let inside = (100..100 + w).contains(&xi) && (20..20 + h).contains(&yi);
                    let interior =
                        (103..100 + w - 3).contains(&xi) && (23..20 + h - 3).contains(&yi);
                    let pixel = out.pixel(x, y);
                    if !inside {
                        assert_eq!(pixel, sky, "{w}×{h} outside at ({x}, {y})");
                    } else if interior {
                        assert_eq!(pixel, centre, "{w}×{h} centre at ({x}, {y})");
                    }
                }
            }
        }
    }

    #[test]
    fn a_fading_panel_blends_every_pixel_once() {
        // 10 wide, the last centre repeat overhangs the east edge by two columns. Uncropped, it
        // would sit under the edge, and the edge's translucent pixels would blend twice.
        let art = Art::embedded();
        let draws = panel(area(0, 0, 10, 9))
            .into_iter()
            .map(|d| Draw { opacity: 128, ..d })
            .collect();
        let out = composite(
            art,
            &Frame {
                draws,
                ..Frame::default()
            },
        );
        let lut = Lut::new(&art.catalog.palette, 0);
        let sky = lut.colour(Slot::SKY_BASE);
        let east = art.catalog.id("tmp.ui.panel.e").unwrap();
        let row = art.rows(east).nth(1).unwrap();
        for (x, &index) in (7..10).zip(row) {
            let ink = lut.colour(Slot::from_atlas(index).unwrap());
            let once: Vec<u8> = (0..3)
                .map(|c| blend(sky[c], ink[c], 128, Blend::Normal))
                .collect();
            assert_eq!(out.pixel(x, 4)[..3], once[..], "column {x}");
        }
    }
}
