//! A scene assembled by hand from the delivered art: a few plants on the soil band, the glass
//! edges and the label. The strip shows it, and `cargo xtask render --demo` writes it, until the
//! composer draws the terrarium from `SceneInput`.

use terrarium_truth::scene::{GrowthStage, PlantSpecies, PlantState};

use crate::catalog::{Art, Catalog, SpriteId};
use crate::compositor::composite;
use crate::frame::{Draw, Frame, GROUND_Y, NATIVE_HEIGHT, NATIVE_WIDTH};
use crate::scale::Padding;
use crate::text::Label;
use crate::variants::{ArtTable, PlantArt};

const WIDTH: i32 = NATIVE_WIDTH.cast_signed();
const HEIGHT: i32 = NATIVE_HEIGHT.cast_signed();
const GROUND: i32 = GROUND_Y.cast_signed();
/// Where a plant's ground point stands: the row above the ground line. A plant's anchor row
/// is its last, so it sits on the soil's top edge rather than under it.
const PLANT_GROUND: i32 = GROUND - 1;
/// The gap between the label plate and the right glass edge.
const LABEL_MARGIN: i32 = 4;

/// The plants, left to right: species, stage, state, bloom, and the x of the ground point.
const PLANTS: &[(PlantSpecies, GrowthStage, PlantState, bool, i32)] = &[
    (
        PlantSpecies::Rowan,
        GrowthStage::Mature,
        PlantState::Thriving,
        false,
        70,
    ),
    (
        PlantSpecies::Fern,
        GrowthStage::Young,
        PlantState::Thriving,
        false,
        122,
    ),
    (
        PlantSpecies::Oak,
        GrowthStage::Mature,
        PlantState::Thriving,
        true,
        190,
    ),
    (
        PlantSpecies::Clover,
        GrowthStage::Seedling,
        PlantState::Thriving,
        false,
        246,
    ),
    (
        PlantSpecies::Foxglove,
        GrowthStage::Mature,
        PlantState::Thriving,
        true,
        282,
    ),
    (
        PlantSpecies::Birch,
        GrowthStage::Young,
        PlantState::Thriving,
        false,
        340,
    ),
    (
        PlantSpecies::TallGrass,
        GrowthStage::Mature,
        PlantState::Thriving,
        false,
        392,
    ),
    (
        PlantSpecies::MossMound,
        GrowthStage::Mature,
        PlantState::Thriving,
        false,
        440,
    ),
];

/// The scene at a palette mix and dim, its label reading `label`.
///
/// # Panics
/// If the catalog lacks the soil, glass or label art, which the embedded art has.
pub fn frame(art: &Art, table: &ArtTable, label: &str, palette_mix: u8, dim: u8) -> Frame {
    let catalog = &art.catalog;
    let mut draws: Vec<Draw> = PLANTS
        .iter()
        .flat_map(|&(species, growth, health, bloom, x)| {
            plant(catalog, table, (species, growth, health), bloom, x)
        })
        .collect();
    draws.extend(soil_and_glass(catalog, WIDTH));
    let plate = sprite(catalog, Label::PLATE);
    let size = catalog.sprite(plate).rect;
    let glass = catalog
        .sprite(sprite(catalog, "tmp.glass.edge.right"))
        .rect
        .w;
    let plate_x = WIDTH - i32::from(glass) - LABEL_MARGIN - i32::from(size.w);
    let plate_y = HEIGHT - i32::from(size.h);
    let label_art = Label::new(catalog).expect("the delivered label plate");
    draws.extend(label_art.draws(label, plate_x, plate_y));
    // Depth order. The sort is stable, so each plant keeps its stem, foliage, bloom order.
    draws.sort_by_key(|draw| draw.layer);
    Frame {
        draws,
        palette_mix,
        dim,
    }
}

/// Each row's colour beyond the image: the outer column of the glass and the soil alone,
/// whatever the scene draws beside them, so nothing in the scene is ever stretched across the
/// monitor's spare width.
pub fn padding(art: &Art, palette_mix: u8, dim: u8) -> Padding {
    let frame = Frame {
        draws: soil_and_glass(&art.catalog, 1),
        palette_mix,
        dim,
    };
    let image = composite(art, &frame);
    std::array::from_fn(|y| image.pixel(0, u32::try_from(y).unwrap_or(0)))
}

/// A plant's draws at a ground point: the seedling, or the stem then the foliage, then the
/// bloom when it has one.
fn plant(
    catalog: &Catalog,
    table: &ArtTable,
    (species, growth, health): (PlantSpecies, GrowthStage, PlantState),
    bloom: bool,
    x: i32,
) -> Vec<Draw> {
    let at = |sprite| Draw::anchored(catalog, sprite, x, PLANT_GROUND);
    let mut draws = match table.plant(species, growth, health) {
        PlantArt::Seedling(sprite) => vec![at(sprite)],
        PlantArt::Parts { stem, foliage } => vec![at(stem), at(foliage)],
    };
    if bloom {
        draws.push(at(table.bloom(species)));
    }
    draws
}

/// The soil band and its top edge from the left edge to `width`, then both glass edges.
fn soil_and_glass(catalog: &Catalog, width: i32) -> Vec<Draw> {
    let tiles = [
        sprite(catalog, "tmp.soil.tile.01"),
        sprite(catalog, "tmp.soil.tile.02"),
    ];
    let edges = [
        sprite(catalog, "tmp.soil.edge.01"),
        sprite(catalog, "tmp.soil.edge.02"),
    ];
    let step = usize::from(catalog.sprite(tiles[0]).rect.w).max(1);
    // Every third tile is the second variant, offset between tile and edge, so the band does
    // not repeat every tile.
    let variant = |i: usize| usize::from(i % 3 == 2);
    let columns: Vec<(usize, i32)> = (0..width).step_by(step).enumerate().collect();
    let mut draws: Vec<Draw> = columns
        .iter()
        .map(|&(i, x)| Draw::new(catalog, tiles[variant(i)], x, GROUND))
        .collect();
    draws.extend(
        columns
            .iter()
            .map(|&(i, x)| Draw::new(catalog, edges[variant(i + 1)], x, GROUND)),
    );
    let right = sprite(catalog, "tmp.glass.edge.right");
    let right_x = WIDTH - i32::from(catalog.sprite(right).rect.w);
    draws.push(Draw::new(
        catalog,
        sprite(catalog, "tmp.glass.edge.left"),
        0,
        0,
    ));
    draws.push(Draw::new(catalog, right, right_x, 0));
    draws
}

fn sprite(catalog: &Catalog, name: &str) -> SpriteId {
    catalog
        .id(name)
        .unwrap_or_else(|| panic!("the delivered {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Layer;
    use crate::compositor::dim;

    fn sample(mix: u8, dim: u8) -> Frame {
        let art = Art::embedded();
        let table = ArtTable::new(&art.catalog).expect("the embedded art is complete");
        frame(art, &table, "repo-terrarium", mix, dim)
    }

    #[test]
    fn the_sample_is_in_depth_order_from_plants_to_label() {
        let frame = sample(0, 0);
        assert!(
            frame
                .draws
                .windows(2)
                .all(|pair| pair[0].layer <= pair[1].layer)
        );
        let layers: Vec<Layer> = frame.draws.iter().map(|d| d.layer).collect();
        for layer in [
            Layer::FarPlants,
            Layer::Soil,
            Layer::Ground,
            Layer::Glass,
            Layer::Label,
        ] {
            assert!(layers.contains(&layer), "{layer:?}");
        }
    }

    #[test]
    fn a_plant_stands_with_its_last_row_on_the_soil() {
        let art = Art::embedded();
        let frame = sample(0, 0);
        let plants = frame
            .draws
            .iter()
            .filter(|d| d.layer < Layer::ResidentsGround);
        for draw in plants {
            let rect = art.catalog.sprite(draw.sprite).rect;
            let bottom = draw.y + i32::from(rect.h) - 1;
            assert!(
                bottom <= PLANT_GROUND,
                "{} reaches row {bottom}",
                art.catalog.sprite(draw.sprite).id
            );
        }
    }

    #[test]
    fn the_padding_is_the_sample_s_glass_and_soil_column_dimmed_with_it() {
        let art = Art::embedded();
        for (mix, by) in [(0, 0), (255, 0), (128, 90)] {
            let image = composite(art, &sample(mix, by));
            let padding = padding(art, mix, by);
            for y in 0..NATIVE_HEIGHT {
                assert_eq!(
                    padding[y as usize],
                    image.pixel(0, y),
                    "row {y} at {mix}, {by}"
                );
            }
        }
        let dimmed = padding(art, 0, 200);
        assert_eq!(dimmed[0], dim(padding(art, 0, 0)[0], 200));
    }
}
