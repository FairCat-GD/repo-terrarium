//! The install salt: 16 random bytes per install, stored beside the terrarium. It moves the
//! slots within their cells and picks between equivalent sprites, and nothing else (I-2). Truth
//! never sees it, so two installs may look different but never disagree.

/// What a salted value is for. Each purpose draws an independent value from the same salt, so
/// adding a purpose never moves an existing one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// How far a slot sits from the centre of its cell.
    SlotJitter,
    /// Which of a filler's equivalent sprites a cell shows.
    FillVariant,
}

impl Purpose {
    const fn tag(self) -> u64 {
        match self {
            Purpose::SlotJitter => 1,
            Purpose::FillVariant => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstallSalt([u8; 16]);

impl InstallSalt {
    pub const fn new(bytes: [u8; 16]) -> InstallSalt {
        InstallSalt(bytes)
    }

    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }

    /// A value for one purpose and index, spread over all 64 bits.
    pub fn mix(self, purpose: Purpose, index: u64) -> u64 {
        let [lo, hi] = [&self.0[..8], &self.0[8..]].map(|half| {
            let mut bytes = [0; 8];
            bytes.copy_from_slice(half);
            u64::from_le_bytes(bytes)
        });
        let z = finalize(lo ^ purpose.tag().wrapping_mul(GOLDEN));
        let z = finalize(z ^ hi);
        finalize(z ^ index.wrapping_add(1).wrapping_mul(GOLDEN))
    }

    /// A salted value in `0..n`, or 0 when `n` is 0.
    pub fn below(self, purpose: Purpose, index: u64, n: u64) -> u64 {
        // The high 32 bits scaled down: no modulo, so no value is favoured by more than one
        // part in 2^32.
        ((self.mix(purpose, index) >> 32) * n) >> 32
    }
}

const GOLDEN: u64 = 0x9e37_79b9_7f4a_7c15;

/// `SplitMix64`'s output function: every input bit reaches every output bit.
const fn finalize(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: InstallSalt = InstallSalt::new(*b"a fictional salt");
    const B: InstallSalt = InstallSalt::new(*b"another one, too");

    #[test]
    fn the_mix_is_pinned() {
        // Snapshots depend on every bit; if this moves, every salted picture moves with it.
        assert_eq!(A.mix(Purpose::SlotJitter, 0), 0xdc7e_9c09_c3fb_9540);
        assert_eq!(A.mix(Purpose::FillVariant, 7), 0xaf62_f4e6_bad2_2e62);
    }

    #[test]
    fn salts_purposes_and_indices_each_give_their_own_value() {
        let values = [
            A.mix(Purpose::SlotJitter, 0),
            A.mix(Purpose::SlotJitter, 1),
            A.mix(Purpose::FillVariant, 0),
            B.mix(Purpose::SlotJitter, 0),
        ];
        for (i, a) in values.iter().enumerate() {
            for b in &values[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert_eq!(A.mix(Purpose::SlotJitter, 3), A.mix(Purpose::SlotJitter, 3));
    }

    #[test]
    fn below_stays_in_range_and_reaches_every_value() {
        let mut seen = [false; 15];
        for index in 0..400 {
            let value = A.below(Purpose::SlotJitter, index, 15);
            seen[usize::try_from(value).unwrap()] = true;
        }
        assert!(seen.iter().all(|&s| s));
        assert_eq!(A.below(Purpose::SlotJitter, 0, 0), 0);
    }
}
