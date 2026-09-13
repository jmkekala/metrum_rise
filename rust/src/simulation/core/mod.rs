// SPDX-License-Identifier: GPL-2.0-only

//! Core simulation primitives: time and tick control.

pub mod config;
pub(crate) mod sparse_chunk_grid;
pub mod time;

/// Rounds a finite or non-finite `f64` with `f64::round` semantics and saturates to `i64`.
#[inline]
pub(crate) fn round_f64_to_i64(value: f64) -> i64 {
    value.round() as i64
}

#[cfg(test)]
mod tests {
    use super::round_f64_to_i64;

    #[test]
    fn integer_quantization_matches_round_then_cast() {
        for value in [
            f64::NEG_INFINITY,
            -9_007_199_254_740_992.0,
            -4_503_599_627_370_497.0,
            i64::MIN as f64,
            -2.5,
            -1.5,
            -0.500_000_000_1,
            -0.5,
            -0.499_999_999_9,
            -0.0,
            0.0,
            0.499_999_999_9,
            0.5,
            0.500_000_000_1,
            1.5,
            2.5,
            4_503_599_627_370_497.0,
            9_007_199_254_740_992.0,
            i64::MAX as f64,
            f64::INFINITY,
            f64::NAN,
        ] {
            assert_eq!(round_f64_to_i64(value), value.round() as i64);
        }
        // Offset-then-cast can cross a rounding boundary before the integer conversion.
        for midpoint in [0.5_f64, 1.5, 2.5, 1023.5, 1_125_899_906_842_624.5] {
            for bits in midpoint.to_bits() - 1..=midpoint.to_bits() + 1 {
                for value in [f64::from_bits(bits), -f64::from_bits(bits)] {
                    assert_eq!(round_f64_to_i64(value), value.round() as i64, "{value:?}");
                }
            }
        }
    }
}
