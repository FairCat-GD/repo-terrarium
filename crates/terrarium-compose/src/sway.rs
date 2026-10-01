//! Plants swaying in the wind. Foliage leans about its `shearPivot` in stepped poses, as pixel
//! art moves, and the stem never shears. Wind raises how far a crown leans, never how many
//! plants sway: at most six of the thirty cells gust in any turn, a different six each turn,
//! each at its own moment.

use terrarium_truth::scene::Wind;

use crate::frame::Shear;

/// The most plants swaying at once.
pub const MOST_SWAYING: usize = 6;

/// A gust in a breeze: the crown leans a pixel downwind, back, and again, each pose held so
/// many milliseconds, then rests.
const BREEZE: [(i32, u64); 3] = [(1, 700), (0, 500), (1, 700)];
/// A gust in a gale: twice as far, and quicker between.
const GALE: [(i32, u64); 5] = [(1, 300), (2, 700), (1, 300), (2, 700), (1, 300)];
/// A cell's second gust comes this long after its first, both within the turn.
const SECOND_GUST_MS: u64 = 9000;

/// Whether a plant in `cell` gusts in the turn whose motion count is `j`: six cells in thirty,
/// spread along the strip and different every turn.
pub const fn gusts(cell: u8, j: u64) -> bool {
    (cell as u64 * 7 + j).is_multiple_of(5)
}

/// How far into its turn a cell's first gust starts: staggered by cell, so neighbours never
/// sway in step.
const fn first_gust(cell: u8) -> u64 {
    1000 + (cell as u64 * 2311) % 8000
}

fn poses(wind: Wind) -> &'static [(i32, u64)] {
    match wind {
        Wind::Calm => &[],
        Wind::Breezy => &BREEZE,
        Wind::Gale => &GALE,
    }
}

/// Every moment, as milliseconds into the turn, at which a gusting cell's crown changes pose,
/// each with the pose it takes. Both gusts end at rest well before the turn does.
fn moves(wind: Wind, cell: u8) -> impl Iterator<Item = (u64, i32)> {
    let poses = poses(wind);
    let first = first_gust(cell);
    [first, first + SECOND_GUST_MS]
        .into_iter()
        .flat_map(move |start| {
            let mut at = start;
            let leans = poses.iter().map(move |&(lean, hold)| {
                let moment = at;
                at += hold;
                (moment, lean)
            });
            let rest = start + poses.iter().map(|&(_, hold)| hold).sum::<u64>();
            leans.chain((!poses.is_empty()).then_some((rest, 0)))
        })
}

/// How many pixels downwind the crown of a gusting cell leans, `since` milliseconds into the
/// turn.
pub fn crown(wind: Wind, cell: u8, since: u64) -> i32 {
    moves(wind, cell)
        .take_while(|&(moment, _)| moment <= since)
        .last()
        .map_or(0, |(_, lean)| lean)
}

/// The next moment after `since`, in milliseconds into the turn, at which the crown of a
/// gusting cell changes pose; `None` if not again this turn.
pub fn next_move(wind: Wind, cell: u8, since: u64) -> Option<u64> {
    moves(wind, cell)
        .map(|(moment, _)| moment)
        .find(|&moment| moment > since)
}

/// The shear that leans a sprite's top row `crown` pixels right about its row `pivot_y`, the
/// rows between in proportion; none for a sprite with nothing above its pivot.
pub fn shear(pivot_y: i16, crown: i32) -> Shear {
    if crown == 0 || pivot_y <= 0 {
        return Shear::NONE;
    }
    // Rounded to nearest: the top row's slide is then exactly the crown's lean.
    let pivot = i32::from(pivot_y);
    let slope = (2 * crown * 256 + pivot) / (2 * pivot);
    Shear {
        pivot_y,
        slope: i16::try_from(slope).unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Art;

    #[test]
    fn six_cells_in_thirty_gust_each_turn_and_each_cell_in_turn() {
        for j in 0..20 {
            assert_eq!((0..30).filter(|&cell| gusts(cell, j)).count(), MOST_SWAYING);
        }
        for cell in 0..30 {
            assert!((0..5).any(|j| gusts(cell, j)), "cell {cell} never gusts");
        }
    }

    #[test]
    fn a_gust_leans_step_by_step_and_rests_before_the_turn_ends() {
        for cell in 0..30 {
            let first = first_gust(cell);
            assert_eq!(crown(Wind::Gale, cell, first - 1), 0);
            assert_eq!(crown(Wind::Gale, cell, first), 1);
            assert_eq!(crown(Wind::Gale, cell, first + 300), 2);
            assert_eq!(crown(Wind::Breezy, cell, first + 700), 0);
            assert_eq!(crown(Wind::Calm, cell, first + 300), 0);
            let mut last = 0;
            let mut since = 0;
            while let Some(next) = next_move(Wind::Gale, cell, since) {
                let lean = crown(Wind::Gale, cell, next);
                assert_eq!(crown(Wind::Gale, cell, next - 1), last);
                assert_eq!((lean - last).abs(), 1, "a pixel at a time");
                (last, since) = (lean, next);
            }
            assert_eq!(last, 0, "it ends at rest");
            assert!(since < crate::motion::TURN_MS - 1000, "{since}");
        }
        assert_eq!(next_move(Wind::Calm, 3, 0), None);
    }

    #[test]
    fn every_swaying_foliage_leans_its_top_row_by_exactly_the_crown() {
        let catalog = &Art::embedded().catalog;
        let foliage = catalog.sprites.iter().filter(|sprite| {
            sprite.keys.part.as_deref() == Some("foliage") && sprite.keys.event.is_none()
        });
        let mut seen = 0;
        for sprite in foliage {
            let pivot = sprite
                .shear_pivot
                .expect("young and mature foliage has a pivot")
                .y;
            for lean in [1, 2] {
                assert_eq!(shear(pivot, lean).offset(0), lean, "{}", sprite.id);
                assert_eq!(shear(pivot, lean).offset(i32::from(pivot)), 0);
            }
            seen += 1;
        }
        assert_eq!(seen, 120, "twelve species, two stages, five states");
        assert_eq!(shear(0, 2), Shear::NONE);
    }
}
