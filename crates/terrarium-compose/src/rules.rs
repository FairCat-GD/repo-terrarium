//! One message at a time: each presentation state has one dominant visual, and the composer
//! quiets its own dressing so nothing competes with it.
//!
//! | State | Dominant | The composer quiets |
//! |---|---|---|
//! | Thriving | plants in leaf and bloom | nothing |
//! | Mature | established plants, monuments | clouds to one |
//! | Struggling | plant forms, ground cover | clouds to one; decorative tufts |
//! | Recovering | new growth | clouds to one |
//! | Dormant | condensation, still plants | clouds; sun; stars; decorative tufts |
//! | Dead | bleached plants, litter, dust | clouds; sun; decorative tufts |
//!
//! Only the composer's own dressing is ever quieted. What truth reports always shows: plants,
//! weeds, litter, fungus, monuments, fossils, glass films and light. The moon follows the date
//! in every state, and the weather decides before this table does ([`crate::sky`]).

use terrarium_truth::PresentationState;

/// What of its own dressing the composer may show in a state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dressing {
    /// The most clouds, of those the weather brings.
    pub clouds: usize,
    pub sun: bool,
    pub stars: bool,
    /// The decorative tufts between the slots; never the filler.
    pub tufts: bool,
}

const ALL_CLOUDS: usize = crate::sky::MAX_CLOUDS;

/// The table above, row by row.
pub const fn dressing(state: PresentationState) -> Dressing {
    let (clouds, sun, stars, tufts) = match state {
        PresentationState::Thriving => (ALL_CLOUDS, true, true, true),
        PresentationState::Mature | PresentationState::Recovering => (1, true, true, true),
        PresentationState::Struggling => (1, true, true, false),
        PresentationState::Dormant => (0, false, false, false),
        PresentationState::Dead => (0, false, true, false),
    };
    Dressing {
        clouds,
        sun,
        stars,
        tufts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_thriving_keeps_every_cloud_and_the_still_states_keep_none() {
        assert_eq!(dressing(PresentationState::Thriving).clouds, ALL_CLOUDS);
        for state in [PresentationState::Dormant, PresentationState::Dead] {
            let quiet = dressing(state);
            assert_eq!((quiet.clouds, quiet.sun, quiet.tufts), (0, false, false));
        }
        assert!(dressing(PresentationState::Dead).stars);
        assert!(!dressing(PresentationState::Dormant).stars);
    }
}
