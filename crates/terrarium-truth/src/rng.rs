//! Randomness for truth: `PCG32` seeded through `SplitMix64`, with Lemire's bounded ranges, and
//! one stream per purpose.
//!
//! Both generators and the stream derivation are hand-rolled with known-answer tests, because
//! truth's output depends on every bit they produce: a library update must never change a
//! terrarium. Changing anything here bumps `SIM_VERSION`.

use std::num::NonZeroU32;

use sha2::{Digest, Sha256};

use crate::canon::canonical_enum;
use crate::identity::RepoSeed;

/// Vigna's `SplitMix64`, used to spread one 64-bit seed over a generator's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub const fn new(seed: u64) -> SplitMix64 {
        SplitMix64 { state: seed }
    }

    pub const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// O'Neill's `PCG32` (`pcg32_random_r`: 64-bit LCG state, XSH-RR output).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcg32 {
    state: u64,
    increment: u64,
}

const PCG_MULTIPLIER: u64 = 6_364_136_223_846_793_005;

/// A stream's purpose. Its `name()`, the variant's own name, is hashed into the stream's seed,
/// so each purpose draws from its own sequence, no two purposes can share one, and a new
/// purpose never shifts another's draws. A variant may be added anywhere, but renaming one
/// changes every draw from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StreamTag {
    /// Which species a terrarium's seed picks.
    SpeciesPool,
    /// Which side an event enters from.
    EventSide,
    /// Which species each day's seedlings are.
    Germination,
    /// Which five resident species a terrarium's seed picks.
    ResidentPool,
    /// Which species each day's arriving residents are.
    ResidentArrival,
    /// Which form a deletion's fossil takes, keyed by its commit.
    Fossil,
}

canonical_enum!(StreamTag {
    SpeciesPool,
    EventSide,
    Germination,
    ResidentPool,
    ResidentArrival,
    Fossil,
});

/// Separates stream seeds from every other SHA-256 truth computes.
const STREAM_DOMAIN: &[u8] = b"repo-terrarium rng stream v1\0";

impl Pcg32 {
    /// The reference `pcg32_srandom_r(initstate, initseq)`.
    pub const fn new(initstate: u64, initseq: u64) -> Pcg32 {
        let mut rng = Pcg32 {
            state: 0,
            increment: (initseq << 1) | 1,
        };
        rng.step();
        rng.state = rng.state.wrapping_add(initstate);
        rng.step();
        rng
    }

    /// A generator whose state and sequence both come from `SplitMix64(seed)`.
    pub const fn from_seed(seed: u64) -> Pcg32 {
        let mut mix = SplitMix64::new(seed);
        let initstate = mix.next_u64();
        let initseq = mix.next_u64();
        Pcg32::new(initstate, initseq)
    }

    /// The stream for one purpose within one terrarium. `key` separates draws of the same
    /// purpose, for example one stream per tick, so a draw never depends on how many draws
    /// came before it.
    ///
    /// The seed is the first 8 bytes (little-endian) of SHA-256 over the domain, the repo seed,
    /// `key` (little-endian) and the tag's name. Everything before the name has a fixed width,
    /// so no two inputs share a hash input.
    pub fn stream(seed: &RepoSeed, tag: StreamTag, key: u64) -> Pcg32 {
        let digest: [u8; 32] = Sha256::new()
            .chain_update(STREAM_DOMAIN)
            .chain_update(seed.as_bytes())
            .chain_update(key.to_le_bytes())
            .chain_update(tag.name().as_bytes())
            .finalize()
            .into();
        Pcg32::from_seed(digest_u64(&digest))
    }

    const fn step(&mut self) {
        self.state = self
            .state
            .wrapping_mul(PCG_MULTIPLIER)
            .wrapping_add(self.increment);
    }

    pub const fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.step();
        let xorshifted = low_u32(((old >> 18) ^ old) >> 27);
        let rotation = low_u32(old >> 59);
        xorshifted.rotate_right(rotation)
    }

    /// A uniform draw from `0..bound`, by Lemire's nearly divisionless method: the high half
    /// of `draw × bound`, redrawing the rare low halves that would bias it.
    pub fn below(&mut self, bound: NonZeroU32) -> u32 {
        let bound = bound.get();
        let mut product = u64::from(self.next_u32()) * u64::from(bound);
        if low_u32(product) < bound {
            // 2³² mod bound: the number of low halves that would overweight some results.
            let threshold = bound.wrapping_neg() % bound;
            while low_u32(product) < threshold {
                product = u64::from(self.next_u32()) * u64::from(bound);
            }
        }
        low_u32(product >> 32)
    }
}

/// The first 8 bytes of a SHA-256 digest, little-endian: how truth turns a hash into a number.
pub fn digest_u64(digest: &[u8; 32]) -> u64 {
    let mut first = [0; 8];
    first.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(first)
}

/// The low 32 bits.
const fn low_u32(value: u64) -> u32 {
    let [a, b, c, d, ..] = value.to_le_bytes();
    u32::from_le_bytes([a, b, c, d])
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::test_support::proptest_config;

    // Expected values below come from the reference implementations (`pcg32-demo` for
    // `srandom(42, 54)`, Vigna's `splitmix64.c`) and an independent Python model for the rest.

    #[test]
    fn splitmix64_is_known() {
        let mut mix = SplitMix64::new(0);
        let outputs: Vec<u64> = (0..4).map(|_| mix.next_u64()).collect();
        assert_eq!(
            outputs,
            [
                0xe220_a839_7b1d_cdaf,
                0x6e78_9e6a_a1b9_65f4,
                0x06c4_5d18_8009_454f,
                0xf88b_b8a8_724c_81ec
            ]
        );
    }

    #[test]
    fn pcg32_is_known() {
        let mut rng = Pcg32::new(42, 54);
        let outputs: Vec<u32> = (0..6).map(|_| rng.next_u32()).collect();
        assert_eq!(
            outputs,
            [
                0xa15c_02b7,
                0x7b47_f409,
                0xba1d_3330,
                0x83d2_f293,
                0xbfa4_784b,
                0xcbed_606e
            ]
        );
        let mut seeded = Pcg32::from_seed(0);
        let outputs: Vec<u32> = (0..4).map(|_| seeded.next_u32()).collect();
        assert_eq!(
            outputs,
            [0x9064_4221, 0x4618_e85f, 0x8f5b_d9cd, 0xaf2c_0306]
        );
    }

    #[test]
    fn below_is_known() {
        let mut rng = Pcg32::from_seed(0x7465_7272_6172_6975);
        let bounds = [7, 1000, 3, (1 << 31) + 1, 1, 6, 6, 6];
        let draws: Vec<u32> = bounds
            .into_iter()
            .map(|bound| rng.below(NonZeroU32::new(bound).expect("nonzero")))
            .collect();
        assert_eq!(draws, [5, 164, 2, 2_074_520_455, 0, 2, 5, 4]);
    }

    fn first_draws(mut rng: Pcg32) -> Vec<u32> {
        (0..4).map(|_| rng.next_u32()).collect()
    }

    #[test]
    fn streams_are_known() {
        let zero = RepoSeed::from_bytes([0; 32]);
        assert_eq!(
            first_draws(Pcg32::stream(&zero, StreamTag::SpeciesPool, 0)),
            [0x824a_0383, 0x8881_349b, 0xb983_4675, 0xb210_8deb]
        );
        let mut counting = [0; 32];
        for (byte, value) in counting.iter_mut().zip(0..) {
            *byte = value;
        }
        let counting = RepoSeed::from_bytes(counting);
        assert_eq!(
            first_draws(Pcg32::stream(&counting, StreamTag::EventSide, 1234)),
            [0xf1cd_00e3, 0x879b_d870, 0x5bc7_935f, 0xfb1d_c5ef]
        );
    }

    #[test]
    fn streams_are_independent() {
        let seed = RepoSeed::from_bytes([7; 32]);
        let alone = first_draws(Pcg32::stream(&seed, StreamTag::SpeciesPool, 3));
        // Another purpose drawing first changes nothing.
        let mut other = Pcg32::stream(&seed, StreamTag::EventSide, 3);
        for _ in 0..100 {
            other.next_u32();
        }
        assert_eq!(
            first_draws(Pcg32::stream(&seed, StreamTag::SpeciesPool, 3)),
            alone
        );
        assert_ne!(
            first_draws(Pcg32::stream(&seed, StreamTag::EventSide, 3)),
            alone
        );
        assert_ne!(
            first_draws(Pcg32::stream(&seed, StreamTag::SpeciesPool, 4)),
            alone
        );
        let other_seed = RepoSeed::from_bytes([8; 32]);
        assert_ne!(
            first_draws(Pcg32::stream(&other_seed, StreamTag::SpeciesPool, 3)),
            alone
        );
    }

    proptest! {
        #![proptest_config(proptest_config())]

        #[test]
        fn below_stays_below(seed: u64, bound in 1..=u32::MAX) {
            let bound = NonZeroU32::new(bound).expect("nonzero");
            let mut rng = Pcg32::from_seed(seed);
            for _ in 0..8 {
                prop_assert!(rng.below(bound) < bound.get());
            }
        }
    }
}
