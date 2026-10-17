//! Fixed-point numbers, the only fractional arithmetic truth has.
//!
//! `Q16` is Q16.16 in an `i32` and `Q32` is Q32.32 in an `i64`. Two rules hold for every
//! operation, and both are part of truth's output, so changing either bumps `SIM_VERSION`:
//!
//! - **Saturation.** A result beyond the type's range becomes `MIN` or `MAX`; nothing wraps and
//!   nothing panics. Division by zero saturates by the sign of the dividend, and `0 / 0` is zero.
//! - **Rounding half away from zero.** A result between two representable values goes to the
//!   nearer one, and an exact tie goes away from zero: +0.5 ULP becomes +1 ULP, −0.5 ULP
//!   becomes −1 ULP.
//!
//! There are no operator impls: every step names its rule (`saturating_mul`, not `*`).

use serde::{Deserialize, Serialize};

/// Q16.16: 16 integer bits (sign included) and 16 fraction bits, so ±32768 in steps of 2⁻¹⁶.
/// Readings and health live here.
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Q16(i32);

/// Q32.32: 32 integer bits (sign included) and 32 fraction bits, for values that outgrow `Q16`,
/// such as line counts on their way into `log2`.
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Q32(i64);

const Q16_SCALE: i128 = 1 << 16;
const Q32_SCALE: i128 = 1 << 32;

impl Q16 {
    pub const FRAC_BITS: u32 = 16;
    pub const ZERO: Q16 = Q16(0);
    pub const ONE: Q16 = Q16(1 << 16);
    pub const HALF: Q16 = Q16(1 << 15);
    /// The smallest step, 2⁻¹⁶.
    pub const EPSILON: Q16 = Q16(1);
    pub const MIN: Q16 = Q16(i32::MIN);
    pub const MAX: Q16 = Q16(i32::MAX);

    /// The value whose bit pattern is `raw`, that is `raw / 2¹⁶`.
    pub const fn from_raw(raw: i32) -> Q16 {
        Q16(raw)
    }

    pub const fn raw(self) -> i32 {
        self.0
    }

    /// `n`, saturated to the range of `Q16`.
    pub const fn from_int(n: i32) -> Q16 {
        Q16(n.saturating_mul(1 << 16))
    }

    /// A count, saturated to the range of `Q16`.
    pub fn from_count(n: u32) -> Q16 {
        Q16::from_int(i32::try_from(n).unwrap_or(i32::MAX))
    }

    /// `num / den`, rounded and saturated; a zero `den` saturates by the sign of `num`. It is
    /// `const`, so tuning reads as the ratio it is: `const HEALTHY: Q16 = Q16::from_ratio(55,
    /// 100);`.
    pub const fn from_ratio(num: i32, den: i32) -> Q16 {
        if den == 0 {
            return if num > 0 {
                Q16::MAX
            } else if num < 0 {
                Q16::MIN
            } else {
                Q16::ZERO
            };
        }
        let magnitude = ratio_magnitude_u32(num.unsigned_abs(), den.unsigned_abs());
        let raw = if (num < 0) != (den < 0) {
            if magnitude >= 1 << 31 {
                i32::MIN
            } else {
                -magnitude.cast_signed()
            }
        } else if magnitude > i32::MAX.cast_unsigned() {
            i32::MAX
        } else {
            magnitude.cast_signed()
        };
        Q16(raw)
    }

    pub const fn saturating_add(self, rhs: Q16) -> Q16 {
        Q16(self.0.saturating_add(rhs.0))
    }

    pub const fn saturating_sub(self, rhs: Q16) -> Q16 {
        Q16(self.0.saturating_sub(rhs.0))
    }

    pub fn saturating_mul(self, rhs: Q16) -> Q16 {
        let product = i128::from(self.0) * i128::from(rhs.0);
        Q16(saturate_i32(div_round(product, Q16_SCALE)))
    }

    pub fn saturating_div(self, rhs: Q16) -> Q16 {
        if rhs.0 == 0 {
            return Q16::from_ratio(self.0, 0);
        }
        Q16(saturate_i32(div_round(
            i128::from(self.0) * Q16_SCALE,
            i128::from(rhs.0),
        )))
    }

    /// Clamped to `[0, 1]`.
    pub const fn clamp01(self) -> Q16 {
        if self.0 < 0 {
            Q16::ZERO
        } else if self.0 > Q16::ONE.0 {
            Q16::ONE
        } else {
            self
        }
    }

    /// The nearest integer, ties away from zero.
    pub fn round_to_int(self) -> i32 {
        saturate_i32(div_round(i128::from(self.0), Q16_SCALE))
    }

    /// The largest integer not above `self`.
    pub const fn floor_to_int(self) -> i32 {
        self.0 >> Q16::FRAC_BITS
    }

    /// The smallest integer not below `self`.
    pub fn ceil_to_int(self) -> i32 {
        let raw = i64::from(self.0);
        saturate_i32(i128::from(-((-raw) >> Q16::FRAC_BITS)))
    }

    /// One step of an exponential moving average: `self + alpha × (sample − self)`, rounded.
    ///
    /// `alpha` is clamped to `[0, 1]`, so the result always lies between `self` and `sample`.
    /// Rounding means a long run of one sample can settle up to about `1 / (2 × alpha)`
    /// epsilons short of it, so nothing should expect an average to reach its input exactly.
    pub fn ema(self, sample: Q16, alpha: Q16) -> Q16 {
        let alpha = i128::from(alpha.clamp01().0);
        let difference = i128::from(sample.0) - i128::from(self.0);
        let step = div_round(alpha * difference, Q16_SCALE);
        Q16(saturate_i32(i128::from(self.0) + step))
    }

    /// Exact: every `Q16` is a `Q32`.
    pub fn to_q32(self) -> Q32 {
        Q32(i64::from(self.0) << 16)
    }

    /// `log2(self)`, rounded to the nearest `Q16`. `None` for zero and negative values.
    pub fn log2(self) -> Option<Q16> {
        (self.0 > 0).then(|| log2_scaled(u64::from(self.0.unsigned_abs()), Q16::FRAC_BITS))
    }
}

impl Q32 {
    pub const FRAC_BITS: u32 = 32;
    pub const ZERO: Q32 = Q32(0);
    pub const ONE: Q32 = Q32(1 << 32);
    pub const HALF: Q32 = Q32(1 << 31);
    /// The smallest step, 2⁻³².
    pub const EPSILON: Q32 = Q32(1);
    pub const MIN: Q32 = Q32(i64::MIN);
    pub const MAX: Q32 = Q32(i64::MAX);

    /// The value whose bit pattern is `raw`, that is `raw / 2³²`.
    pub const fn from_raw(raw: i64) -> Q32 {
        Q32(raw)
    }

    pub const fn raw(self) -> i64 {
        self.0
    }

    /// `n`, saturated to the range of `Q32`.
    pub const fn from_int(n: i64) -> Q32 {
        Q32(n.saturating_mul(1 << 32))
    }

    /// `num / den`, rounded and saturated; a zero `den` saturates by the sign of `num`.
    pub const fn from_ratio(num: i64, den: i64) -> Q32 {
        if den == 0 {
            return if num > 0 {
                Q32::MAX
            } else if num < 0 {
                Q32::MIN
            } else {
                Q32::ZERO
            };
        }
        let magnitude = ratio_magnitude_u64(num.unsigned_abs(), den.unsigned_abs());
        let raw = if (num < 0) != (den < 0) {
            if magnitude >= 1 << 63 {
                i64::MIN
            } else {
                -magnitude.cast_signed()
            }
        } else if magnitude > i64::MAX.cast_unsigned() {
            i64::MAX
        } else {
            magnitude.cast_signed()
        };
        Q32(raw)
    }

    pub const fn saturating_add(self, rhs: Q32) -> Q32 {
        Q32(self.0.saturating_add(rhs.0))
    }

    pub const fn saturating_sub(self, rhs: Q32) -> Q32 {
        Q32(self.0.saturating_sub(rhs.0))
    }

    pub fn saturating_mul(self, rhs: Q32) -> Q32 {
        let product = i128::from(self.0) * i128::from(rhs.0);
        Q32(saturate_i64(div_round(product, Q32_SCALE)))
    }

    pub fn saturating_div(self, rhs: Q32) -> Q32 {
        if rhs.0 == 0 {
            return Q32::from_ratio(self.0, 0);
        }
        Q32(saturate_i64(div_round(
            i128::from(self.0) * Q32_SCALE,
            i128::from(rhs.0),
        )))
    }

    /// Clamped to `[0, 1]`.
    pub const fn clamp01(self) -> Q32 {
        if self.0 < 0 {
            Q32::ZERO
        } else if self.0 > Q32::ONE.0 {
            Q32::ONE
        } else {
            self
        }
    }

    /// One step of an exponential moving average, as [`Q16::ema`].
    pub fn ema(self, sample: Q32, alpha: Q32) -> Q32 {
        let alpha = i128::from(alpha.clamp01().0);
        let difference = i128::from(sample.0) - i128::from(self.0);
        let step = div_round(alpha * difference, Q32_SCALE);
        Q32(saturate_i64(i128::from(self.0) + step))
    }

    /// Rounded and saturated.
    pub fn to_q16(self) -> Q16 {
        Q16(saturate_i32(div_round(i128::from(self.0), Q16_SCALE)))
    }

    /// `log2(self)`, rounded to the nearest `Q16`. `None` for zero and negative values.
    pub fn log2(self) -> Option<Q16> {
        (self.0 > 0).then(|| log2_scaled(self.0.unsigned_abs(), Q32::FRAC_BITS))
    }
}

/// `log2(n)`, rounded to the nearest `Q16`. `None` for zero.
pub(crate) fn log2_int(n: u64) -> Option<Q16> {
    (n > 0).then(|| log2_scaled(n, 0))
}

/// `log2(mantissa / 2^frac_bits)` for a nonzero `mantissa`, rounded to the nearest `Q16`.
///
/// The integer part is the position of the top bit. The fraction comes from repeated squaring:
/// with `m` normalised into `[1, 2)`, squaring it doubles the logarithm, so `m² ≥ 2` means the
/// next fraction bit is 1 (and `m²` is halved back into range). `m` carries 63 fraction bits in
/// a `u128`, so `m²` never overflows, and truncating each square keeps the result monotonic.
/// Seventeen bits are taken and the last rounds the sixteenth. Rounding cannot meet a tie:
/// the logarithm of a rational is either an integer or irrational.
fn log2_scaled(mantissa: u64, frac_bits: u32) -> Q16 {
    const ONE: u128 = 1 << 63;
    let top_bit = mantissa.ilog2();
    let mut m = u128::from(mantissa) << (63 - top_bit);
    // Integer part, then one fraction bit per step: 17 fraction bits in all.
    let mut bits = i64::from(top_bit) - i64::from(frac_bits);
    for _ in 0..17 {
        m = (m * m) >> 63;
        bits <<= 1;
        if m >= 2 * ONE {
            m >>= 1;
            bits += 1;
        }
    }
    Q16(saturate_i32(i128::from((bits + 1) >> 1)))
}

/// `n / d` in units of 2⁻¹⁶, rounded half away from zero, for `d > 0`. The result is a
/// magnitude; anything at or above 2³¹ comes back as 2³¹ for the caller to saturate.
///
/// Long division in one width keeps it `const` without widening casts: the remainder stays
/// below `d ≤ 2³¹`, so doubling it fits a `u32`.
const fn ratio_magnitude_u32(n: u32, d: u32) -> u32 {
    let whole = n / d;
    if whole >= 1 << 15 {
        return 1 << 31;
    }
    let mut remainder = n % d;
    let mut bits = whole;
    let mut step = 0;
    while step < 17 {
        remainder <<= 1;
        bits <<= 1;
        if remainder >= d {
            remainder -= d;
            bits |= 1;
        }
        step += 1;
    }
    (bits >> 1) + (bits & 1)
}

/// As [`ratio_magnitude_u32`], in units of 2⁻³² and saturating at 2⁶³.
const fn ratio_magnitude_u64(n: u64, d: u64) -> u64 {
    let whole = n / d;
    if whole >= 1 << 31 {
        return 1 << 63;
    }
    let mut remainder = n % d;
    let mut bits = whole;
    let mut step = 0;
    while step < 33 {
        remainder <<= 1;
        bits <<= 1;
        if remainder >= d {
            remainder -= d;
            bits |= 1;
        }
        step += 1;
    }
    (bits >> 1) + (bits & 1)
}

/// `n / d` rounded half away from zero, for `d ≠ 0` and `|n| < 2¹²⁶`.
fn div_round(n: i128, d: i128) -> i128 {
    let quotient = n / d;
    let remainder = (n % d).unsigned_abs();
    let divisor = d.unsigned_abs();
    if remainder >= divisor - remainder {
        if (n < 0) == (d < 0) {
            quotient + 1
        } else {
            quotient - 1
        }
    } else {
        quotient
    }
}

fn saturate_i32(value: i128) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

fn saturate_i64(value: i128) -> i64 {
    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::test_support::proptest_config;

    /// `n / d` rounded half away from zero, from the magnitudes: an independent reference for
    /// `div_round` and the const long division.
    fn reference(n: i128, d: i128) -> i128 {
        let divisor = d.unsigned_abs();
        let quotient = n.unsigned_abs() / divisor;
        let remainder = n.unsigned_abs() % divisor;
        let magnitude = if 2 * remainder >= divisor {
            quotient + 1
        } else {
            quotient
        };
        let magnitude = i128::try_from(magnitude).expect("test values stay small");
        if (n < 0) == (d < 0) {
            magnitude
        } else {
            -magnitude
        }
    }

    fn q16_clamped(value: i128) -> Q16 {
        Q16::from_raw(saturate_i32(value))
    }

    fn q32_clamped(value: i128) -> Q32 {
        Q32::from_raw(saturate_i64(value))
    }

    #[test]
    fn ties_round_away_from_zero() {
        let raw = Q16::from_raw;
        // 1 ULP × 0.5 is a tie at 0.5 ULP; 3 ULP × 0.5 is a tie at 1.5 ULP.
        assert_eq!(raw(1).saturating_mul(Q16::HALF), raw(1));
        assert_eq!(raw(-1).saturating_mul(Q16::HALF), raw(-1));
        assert_eq!(raw(3).saturating_mul(Q16::HALF), raw(2));
        assert_eq!(raw(-3).saturating_mul(Q16::HALF), raw(-2));
        let two = Q16::from_int(2);
        assert_eq!(raw(1).saturating_div(two), raw(1));
        assert_eq!(raw(-1).saturating_div(two), raw(-1));
        assert_eq!(raw(3).saturating_div(two), raw(2));
        assert_eq!(raw(-3).saturating_div(two), raw(-2));
        assert_eq!(Q16::from_ratio(1, 131_072), raw(1));
        assert_eq!(Q16::from_ratio(-1, 131_072), raw(-1));
        assert_eq!(Q16::from_ratio(3, 131_072), raw(2));
        assert_eq!(Q16::from_ratio(3, -131_072), raw(-2));
        assert_eq!(raw(98_304).round_to_int(), 2);
        assert_eq!(raw(-98_304).round_to_int(), -2);
        // Floor and ceiling ignore ties: they round toward their side.
        assert_eq!(raw(98_304).floor_to_int(), 1);
        assert_eq!(raw(98_304).ceil_to_int(), 2);
        assert_eq!(raw(-98_304).floor_to_int(), -2);
        assert_eq!(raw(-98_304).ceil_to_int(), -1);
        assert_eq!(raw(1).floor_to_int(), 0);
        assert_eq!(raw(1).ceil_to_int(), 1);
        assert_eq!(Q16::from_int(3).floor_to_int(), 3);
        assert_eq!(Q16::from_int(3).ceil_to_int(), 3);
        assert_eq!(Q16::MAX.floor_to_int(), 32_767);
        assert_eq!(Q16::MAX.ceil_to_int(), 32_768);
        assert_eq!(Q16::MIN.floor_to_int(), -32_768);
        assert_eq!(Q16::MIN.ceil_to_int(), -32_768);
        assert_eq!(Q32::from_ratio(1, 1 << 33), Q32::EPSILON);
        assert_eq!(Q32::from_ratio(-3, 1 << 33), Q32::from_raw(-2));
        assert_eq!(Q32::from_raw(3).saturating_mul(Q32::HALF), Q32::from_raw(2));
        assert_eq!(
            Q32::from_raw(-3).saturating_mul(Q32::HALF),
            Q32::from_raw(-2)
        );
    }

    #[test]
    fn ratios_are_known() {
        const TUNED: Q16 = Q16::from_ratio(45, 100);
        assert_eq!(TUNED.raw(), 29_491);
        assert_eq!(Q16::from_ratio(55, 100).raw(), 36_045);
        assert_eq!(Q16::from_ratio(-55, -100).raw(), 36_045);
        assert_eq!(Q16::from_ratio(1, 7).raw(), 9_362);
        assert_eq!(Q16::from_ratio(-1, 7).raw(), -9_362);
        assert_eq!(Q16::from_ratio(2, 15).raw(), 8_738);
        assert_eq!(Q32::from_ratio(8, 7).raw(), 4_908_534_053);
    }

    #[test]
    fn everything_saturates() {
        let two = Q16::from_int(2);
        assert_eq!(Q16::MAX.saturating_add(Q16::EPSILON), Q16::MAX);
        assert_eq!(Q16::MIN.saturating_sub(Q16::EPSILON), Q16::MIN);
        assert_eq!(Q16::MAX.saturating_mul(two), Q16::MAX);
        assert_eq!(Q16::MIN.saturating_mul(two), Q16::MIN);
        assert_eq!(Q16::MIN.saturating_div(Q16::from_raw(-1)), Q16::MAX);
        assert_eq!(Q16::from_int(40_000), Q16::MAX);
        assert_eq!(Q16::from_int(-40_000), Q16::MIN);
        assert_eq!(Q16::from_ratio(40_000, 1), Q16::MAX);
        assert_eq!(Q16::from_ratio(-40_000, 1), Q16::MIN);
        assert_eq!(Q16::from_ratio(-32_768, 1), Q16::MIN);
        assert_eq!(Q16::from_ratio(i32::MIN, 1), Q16::MIN);
        assert_eq!(Q16::from_ratio(i32::MIN, -1), Q16::MAX);
        assert_eq!(Q16::ONE.saturating_div(Q16::ZERO), Q16::MAX);
        assert_eq!(Q16::from_int(-1).saturating_div(Q16::ZERO), Q16::MIN);
        assert_eq!(Q16::ZERO.saturating_div(Q16::ZERO), Q16::ZERO);
        assert_eq!(Q16::from_ratio(0, 0), Q16::ZERO);
        assert_eq!(Q32::MAX.saturating_mul(Q32::from_int(2)), Q32::MAX);
        assert_eq!(Q32::from_int(i64::MIN), Q32::MIN);
        assert_eq!(Q32::from_ratio(i64::MIN, 1), Q32::MIN);
        assert_eq!(Q32::from_ratio(i64::MIN, -1), Q32::MAX);
        assert_eq!(Q32::ONE.saturating_div(Q32::ZERO), Q32::MAX);
        assert_eq!(Q32::MAX.to_q16(), Q16::MAX);
        assert_eq!(Q32::MIN.to_q16(), Q16::MIN);
    }

    #[test]
    fn log2_is_known() {
        // Expected values: log2(x) × 2¹⁶ rounded to nearest, computed with 80-digit decimal
        // arithmetic, independently of this code.
        let int = |n| log2_int(n).map(Q16::raw);
        assert_eq!(int(0), None);
        assert_eq!(int(1), Some(0));
        assert_eq!(int(2), Some(65_536));
        assert_eq!(int(3), Some(103_872));
        assert_eq!(int(10), Some(217_706));
        assert_eq!(int(1024), Some(655_360));
        assert_eq!(int(u64::MAX), Some(4_194_304));
        let q32 = |raw| Q32::from_raw(raw).log2().map(Q16::raw);
        // The rain floor, log2(1 + 1/7).
        assert_eq!(Q32::from_ratio(8, 7).log2().map(Q16::raw), Some(12_625));
        assert_eq!(q32(3 << 31), Some(38_336));
        assert_eq!(q32(3 << 30), Some(-27_200));
        assert_eq!(q32(3), Some(-1_993_280));
        assert_eq!(q32(0), None);
        assert_eq!(q32(-1), None);
        assert_eq!(Q16::from_ratio(8, 7).log2().map(Q16::raw), Some(12_625));
        assert_eq!(Q16::HALF.log2(), Some(Q16::from_int(-1)));
        assert_eq!(Q16::from_int(-3).log2(), None);
    }

    #[test]
    fn ema_is_known() {
        // Expected values from the same independent reference as `log2_is_known`.
        let alpha = Q16::from_ratio(2, 15);
        let mut average = Q16::ZERO;
        let mut steps = Vec::new();
        for sample in [Q16::ONE, Q16::ONE, Q16::ONE, Q16::ZERO] {
            average = average.ema(sample, alpha);
            steps.push(average.raw());
        }
        assert_eq!(steps, [8_738, 16_311, 22_874, 19_824]);
    }

    proptest! {
        #![proptest_config(proptest_config())]

        #[test]
        fn q16_ops_equal_the_exact_result_rounded_and_clamped(a: i32, b: i32) {
            let (x, y) = (Q16::from_raw(a), Q16::from_raw(b));
            let (wide_a, wide_b) = (i128::from(a), i128::from(b));
            prop_assert_eq!(x.saturating_add(y), q16_clamped(wide_a + wide_b));
            prop_assert_eq!(x.saturating_sub(y), q16_clamped(wide_a - wide_b));
            prop_assert_eq!(x.saturating_mul(y), q16_clamped(reference(wide_a * wide_b, 1 << 16)));
            if b != 0 {
                prop_assert_eq!(x.saturating_div(y), q16_clamped(reference(wide_a << 16, wide_b)));
                prop_assert_eq!(Q16::from_ratio(a, b), q16_clamped(reference(wide_a << 16, wide_b)));
            }
            prop_assert_eq!(x.round_to_int(), saturate_i32(reference(wide_a, 1 << 16)));
            prop_assert_eq!(i128::from(x.floor_to_int()), wide_a.div_euclid(1 << 16));
            prop_assert_eq!(i128::from(x.ceil_to_int()), -(-wide_a).div_euclid(1 << 16));
        }

        #[test]
        fn q32_ops_equal_the_exact_result_rounded_and_clamped(a: i64, b: i64) {
            let (x, y) = (Q32::from_raw(a), Q32::from_raw(b));
            let (wide_a, wide_b) = (i128::from(a), i128::from(b));
            prop_assert_eq!(x.saturating_add(y), q32_clamped(wide_a + wide_b));
            prop_assert_eq!(x.saturating_sub(y), q32_clamped(wide_a - wide_b));
            prop_assert_eq!(x.saturating_mul(y), q32_clamped(reference(wide_a * wide_b, 1 << 32)));
            if b != 0 {
                prop_assert_eq!(x.saturating_div(y), q32_clamped(reference(wide_a << 32, wide_b)));
                prop_assert_eq!(Q32::from_ratio(a, b), q32_clamped(reference(wide_a << 32, wide_b)));
            }
            prop_assert_eq!(x.to_q16(), q16_clamped(reference(wide_a, 1 << 16)));
        }

        #[test]
        fn identities_hold(a: i32, b: i32) {
            let (x, y) = (Q16::from_raw(a), Q16::from_raw(b));
            prop_assert_eq!(x.saturating_add(Q16::ZERO), x);
            prop_assert_eq!(x.saturating_mul(Q16::ONE), x);
            prop_assert_eq!(x.saturating_div(Q16::ONE), x);
            prop_assert_eq!(x.saturating_add(y), y.saturating_add(x));
            prop_assert_eq!(x.saturating_mul(y), y.saturating_mul(x));
            prop_assert_eq!(x.to_q32().to_q16(), x);
        }

        #[test]
        fn clamp01_lands_in_the_unit_interval(a: i32) {
            let clamped = Q16::from_raw(a).clamp01();
            prop_assert!(Q16::ZERO <= clamped && clamped <= Q16::ONE);
            if (0..=Q16::ONE.raw()).contains(&a) {
                prop_assert_eq!(clamped.raw(), a);
            }
        }

        #[test]
        fn ema_stays_between_average_and_sample(previous: i32, sample: i32, alpha: i32) {
            let (previous, sample) = (Q16::from_raw(previous), Q16::from_raw(sample));
            let next = previous.ema(sample, Q16::from_raw(alpha));
            prop_assert!(previous.min(sample) <= next && next <= previous.max(sample));
            prop_assert_eq!(previous.ema(sample, Q16::ZERO), previous);
            prop_assert_eq!(previous.ema(sample, Q16::ONE), sample);
            let (wide_previous, wide_sample) = (previous.to_q32(), sample.to_q32());
            let wide_next = wide_previous.ema(wide_sample, Q32::from_raw(i64::from(alpha)));
            prop_assert!(wide_previous.min(wide_sample) <= wide_next);
            prop_assert!(wide_next <= wide_previous.max(wide_sample));
        }

        #[test]
        fn log2_is_monotonic(a: u64, b: u64) {
            let (low, high) = (a.min(b).max(1), a.max(b).max(1));
            prop_assert!(log2_int(low) <= log2_int(high));
            let (low, high) = (Q32::from_raw(low.cast_signed().max(1)), Q32::from_raw(high.cast_signed().max(1)));
            prop_assert!(low.min(high).log2() <= low.max(high).log2());
        }

        #[test]
        fn log2_of_double_is_one_more(raw in 1..(1_i64 << 62)) {
            let x = Q32::from_raw(raw);
            let doubled = Q32::from_raw(raw * 2);
            prop_assert_eq!(doubled.log2(), x.log2().map(|l| l.saturating_add(Q16::ONE)));
        }
    }
}
