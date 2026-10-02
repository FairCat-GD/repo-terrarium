//! A scene assembled by hand from the delivered art: the clear weather's backdrop, a few plants
//! on the soil band, the glass and the label at the bottom right. The pinning probe shows it,
//! and `cargo xtask render --demo` writes it. The composer draws its own world; this one only
//! follows the art.

use terrarium_truth::scene::{GrowthStage, PlantSpecies, PlantState, Weather};

use crate::catalog::Art;
use crate::frame::{Draw, Frame, NATIVE_HEIGHT, NATIVE_WIDTH, Viewport};
use crate::plant::Parts;
use crate::text::Label;
use crate::variants::ArtTable;
use crate::world::{fixed, soil_and_glass, sprite};

const WIDTH: i32 = NATIVE_WIDTH.cast_signed();
const HEIGHT: i32 = NATIVE_HEIGHT.cast_signed();
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
    frame_in(art, table, label, palette_mix, dim, Viewport::SCENE)
}

/// As [`frame`], in `viewport`: the plants in its middle, the rest to its edges.
///
/// # Panics
/// If the catalog lacks the soil, glass or label art, which the embedded art has.
pub fn frame_in(
    art: &Art,
    table: &ArtTable,
    label: &str,
    palette_mix: u8,
    dim: u8,
    viewport: Viewport,
) -> Frame {
    let catalog = &art.catalog;
    let mut draws: Vec<Draw> = PLANTS
        .iter()
        .flat_map(|&(species, growth, health, bloom, x)| {
            Parts::new(table, species, growth, health, bloom).draws(catalog, x + viewport.inset())
        })
        .collect();
    if let Some(&backdrop) = table.weather(Weather::Clear).first() {
        draws.push(fixed(art, backdrop, viewport));
    }
    draws.extend(scenery(art, label, viewport));
    // Depth order. The sort is stable, so each plant keeps its stem, foliage, bloom order.
    draws.sort_by_key(|draw| draw.layer);
    Frame {
        draws,
        palette_mix,
        viewport,
        dim,
    }
}

/// The fixed scenery in `viewport`: the soil band and its edge, the glass, and the label
/// plate at the bottom right reading `label`.
///
/// # Panics
/// If the catalog lacks the soil, glass or label art, which the embedded art has.
pub fn scenery(art: &Art, label: &str, viewport: Viewport) -> Vec<Draw> {
    let catalog = &art.catalog;
    let mut draws = soil_and_glass(art, viewport);
    let plate = sprite(catalog, Label::PLATE);
    let size = catalog.sprite(plate).rect;
    let glass = catalog
        .sprite(sprite(catalog, "tmp.glass.edge.right"))
        .rect
        .w;
    let right = WIDTH + viewport.extra();
    let plate_x = right - i32::from(glass) - LABEL_MARGIN - i32::from(size.w);
    let plate_y = HEIGHT - i32::from(size.h);
    let label_art = Label::new(catalog).expect("the delivered label plate");
    draws.extend(label_art.draws(label, plate_x, plate_y));
    draws
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Layer;
    use crate::compositor::composite;
    use crate::frame::PLANT_GROUND_Y;

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
            .filter(|d| (Layer::FarPlants..Layer::ResidentsGround).contains(&d.layer));
        for draw in plants {
            let sprite = art.catalog.sprite(draw.sprite);
            let bottom = draw.y + i32::from(sprite.rect.h) - 1;
            // Only a rooted oak tucks its last rows into the soil's edge.
            let deepest = if sprite.keys.species.as_deref() == Some("oak") {
                PLANT_GROUND_Y + 2
            } else {
                PLANT_GROUND_Y
            };
            assert!(bottom <= deepest, "{} reaches row {bottom}", sprite.id);
        }
    }

    #[test]
    fn a_wider_viewport_keeps_the_plants_in_its_middle_and_the_glass_at_its_edges() {
        let art = Art::embedded();
        let table = ArtTable::new(&art.catalog).expect("the embedded art is complete");
        let scene = frame(art, &table, "repo-terrarium", 0, 0);
        let wide = Viewport::new(735);
        let shown = frame_in(art, &table, "repo-terrarium", 0, 0, wide);
        let plants = |frame: &Frame| -> Vec<(i32, i32)> {
            frame
                .draws
                .iter()
                .filter(|d| (Layer::FarPlants..Layer::ResidentsGround).contains(&d.layer))
                .map(|d| (d.x, d.y))
                .collect()
        };
        let moved: Vec<(i32, i32)> = plants(&scene).iter().map(|&(x, y)| (x + 48, y)).collect();
        assert_eq!(plants(&shown), moved, "735 columns put the scene 48 in");
        let glass: Vec<i32> = shown
            .draws
            .iter()
            .filter(|d| d.layer == Layer::Glass)
            .map(|d| d.x)
            .collect();
        // Each edge's empty outer column lies just beyond the viewport.
        assert!(
            glass.contains(&-1) && glass.contains(&(735 - 5)),
            "{glass:?}"
        );
        let image = composite(art, &shown);
        assert_eq!(image.width(), 735);
    }
}
