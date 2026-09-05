//! Core simulation primitives: time and tick control.

pub mod config;
pub(crate) mod sparse_chunk_grid;
pub mod time;

/// Rounds a finite or non-finite `f64` with `f64::round` semantics and saturates to `i64`.
///
/// Quantized geometry calls this in very hot key-building loops. Expressing the operation as an
/// offset plus Rust's saturating float-to-integer cast avoids the platform `round` library call.
#[inline]
pub(crate) fn round_f64_to_i64(value: f64) -> i64 {
    if value >= 0.0 {
        (value + 0.5) as i64
    } else {
        (value - 0.5) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::round_f64_to_i64;

    #[test]
    fn integer_quantization_matches_round_then_cast() {
        for value in [
            f64::NEG_INFINITY,
            -9_007_199_254_740_992.0,
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
            9_007_199_254_740_992.0,
            f64::INFINITY,
            f64::NAN,
        ] {
            assert_eq!(round_f64_to_i64(value), value.round() as i64);
        }
    }
}
