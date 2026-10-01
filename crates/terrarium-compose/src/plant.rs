//! A plant's parts: a seedling is one combined image; a young or mature plant is its stem, then
//! the state's foliage at the same ground point, then the bloom on top. The bloom shows only on
//! a mature, thriving plant whose `bloom` truth set.

use terrarium_truth::scene::{GrowthStage, PlantSpecies, PlantState};

use crate::catalog::{Catalog, SpriteId};
use crate::frame::{Draw, PLANT_GROUND_Y};
use crate::variants::{ArtTable, PlantArt};

/// The sprites a plant is drawn with, bottom to top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parts {
    /// `None` for a seedling, whose single image carries its own stem.
    pub stem: Option<SpriteId>,
    /// The state's foliage, or the combined seedling. Dead foliage carries its bleached
    /// override of the stem.
    pub foliage: SpriteId,
    pub bloom: Option<SpriteId>,
}

impl Parts {
    pub fn new(
        table: &ArtTable,
        species: PlantSpecies,
        growth: GrowthStage,
        health: PlantState,
        bloom: bool,
    ) -> Parts {
        let (stem, foliage) = match table.plant(species, growth, health) {
            PlantArt::Seedling(combined) => (None, combined),
            PlantArt::Parts { stem, foliage } => (Some(stem), foliage),
        };
        Parts {
            stem,
            foliage,
            bloom: shows_bloom(growth, health, bloom).then(|| table.bloom(species)),
        }
    }

    /// Bottom to top.
    pub fn sprites(self) -> impl Iterator<Item = SpriteId> {
        self.stem
            .into_iter()
            .chain([self.foliage])
            .chain(self.bloom)
    }

    /// Every part anchored on the ground point at `x`, bottom to top.
    pub fn draws(self, catalog: &Catalog, x: i32) -> Vec<Draw> {
        self.sprites()
            .map(|sprite| Draw::anchored(catalog, sprite, x, PLANT_GROUND_Y))
            .collect()
    }
}

/// Whether a plant shows its bloom: only when mature, thriving and authorized by truth.
pub fn shows_bloom(growth: GrowthStage, health: PlantState, bloom: bool) -> bool {
    bloom && growth == GrowthStage::Mature && health == PlantState::Thriving
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Art;

    fn table() -> ArtTable {
        ArtTable::new(&Art::embedded().catalog).unwrap()
    }

    #[test]
    fn bloom_shows_only_on_a_mature_thriving_plant_that_truth_lets_bloom() {
        let table = table();
        let parts = |growth, health, bloom| {
            Parts::new(&table, PlantSpecies::Foxglove, growth, health, bloom).bloom
        };
        let bloom = Some(table.bloom(PlantSpecies::Foxglove));
        assert_eq!(
            parts(GrowthStage::Mature, PlantState::Thriving, true),
            bloom
        );
        assert_eq!(
            parts(GrowthStage::Mature, PlantState::Thriving, false),
            None
        );
        assert_eq!(parts(GrowthStage::Mature, PlantState::Pale, true), None);
        assert_eq!(parts(GrowthStage::Young, PlantState::Thriving, true), None);
    }

    #[test]
    fn a_seedling_is_one_image_and_older_plants_are_stem_then_foliage() {
        let table = table();
        let seedling = Parts::new(
            &table,
            PlantSpecies::Oak,
            GrowthStage::Seedling,
            PlantState::Thriving,
            true,
        );
        assert_eq!(seedling.sprites().count(), 1);
        let mature = Parts::new(
            &table,
            PlantSpecies::Oak,
            GrowthStage::Mature,
            PlantState::Dead,
            false,
        );
        assert_eq!(
            mature.sprites().collect::<Vec<_>>(),
            [mature.stem.unwrap(), mature.foliage]
        );
    }

    #[test]
    fn every_part_stands_on_the_ground_point() {
        let art = Art::embedded();
        let table = table();
        let parts = Parts::new(
            &table,
            PlantSpecies::Rowan,
            GrowthStage::Mature,
            PlantState::Thriving,
            true,
        );
        for (draw, sprite) in parts.draws(&art.catalog, 300).iter().zip(parts.sprites()) {
            let anchor = art.catalog.sprite(sprite).anchor;
            assert_eq!(draw.x + i32::from(anchor.x), 300);
            assert_eq!(draw.y + i32::from(anchor.y), PLANT_GROUND_Y);
        }
    }
}
