// SPDX-License-Identifier: GPL-2.0-only

//! The grading rules (`TERRAIN-07`): tie-in cones, the overlap rule, the inside rule under a
//! road, and the exact pruning reach that bounds where a seam can change a value.
//!
//! The overlap rule (`TD7`, the user's `T2` decision) is `g = max(L, min(b, U))`: the base
//! clamped into `[L, U]`, and `L` where the cones conflict, so the upper road's embankment wins
//! and the lower road gets a retaining wall. At a seam its own cone gives `L >= h >= U`, so the
//! graded ground never lies below a seam.

/// Tie-in slope ratio `k` of every seam kind: 1:2.
pub const TIE_IN_SLOPE: f64 = 0.5;
/// The cap `C`: a seam farther than this from a point never affects it, metres.
pub const CAP_M: f64 = 64.0;
/// Largest depth `σ_max` the inside rule's fill arm rises over, one `GL0` texel diagonal, metres.
pub const FILL_ARM_MAX_M: f64 = 0.5 * std::f64::consts::SQRT_2;
/// Depth `D_in` within which the inside rule follows the nearest road seam's tie-in arm, the
/// `GL0` footprint distance band, metres. Every level's band is at least this wide, so the
/// nearest road seam within it is found exactly at every level.
pub const INSIDE_REACH_M: f64 = 2.0;

// Relative and absolute slack added to a computed reach, so that rounding in the bound can only
// make it larger; a reach is a bound, never a parameter of a value.
const REACH_SLACK: f64 = 1.0e-9;
const REACH_SLACK_M: f64 = 1.0e-6;

/// The cone bounds `L = max (h - k d)` and `U = min (h + k d)` over the seams added, `-∞` and
/// `+∞` with none. `max` and `min` are exact, so the order seams are added in never matters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Cones {
    pub(super) lower: f64,
    pub(super) upper: f64,
}

impl Cones {
    pub(super) const NONE: Self = Self {
        lower: f64::NEG_INFINITY,
        upper: f64::INFINITY,
    };

    /// Adds the cone of a seam at height `height`, `distance` from the point.
    pub(super) fn add(&mut self, height: f64, distance: f64) {
        self.lower = self.lower.max(height - TIE_IN_SLOPE * distance);
        self.upper = self.upper.min(height + TIE_IN_SLOPE * distance);
    }

    /// The overlap rule on these bounds.
    pub(super) fn ground(&self, base: f64) -> f64 {
        overlap(base, self.lower, self.upper)
    }
}

/// The overlap rule: the base clamped into `[lower, upper]`, or the highest lower bound where
/// `lower > upper` (two seams closer than 1:2 allows: the upper one's embankment wins, a wall
/// stands at the lower one). Both branches agree at `lower = upper`, so the result is
/// continuous, and it is the base itself, bitwise, where nothing binds.
pub fn overlap(base: f64, lower: f64, upper: f64) -> f64 {
    if lower <= upper {
        base.clamp(lower, upper)
    } else {
        lower
    }
}

/// The inside rule under a road, `depth` inside the nearest road seam at height `height`: on a
/// cut (`base >= height`) the seam's tie-in arm continued down, on a fill continued up for at
/// most [`FILL_ARM_MAX_M`]. Triangles across a straight edge are then linear through the seam.
pub(super) fn inside(base: f64, height: f64, depth: f64) -> f64 {
    if base >= height {
        height - TIE_IN_SLOPE * depth
    } else {
        height + TIE_IN_SLOPE * depth.min(FILL_ARM_MAX_M)
    }
}

/// The inside rule deeper than [`INSIDE_REACH_M`] from every road seam, from the road surface
/// `road` there: the arm's value at that depth on a cut, the capped fill arm on a fill. Only
/// coarse levels' triangles reach such samples, and the mask discards them.
pub(super) fn inside_deep(base: f64, road: f64) -> f64 {
    if base >= road {
        road - TIE_IN_SLOPE * INSIDE_REACH_M
    } else {
        road + TIE_IN_SLOPE * FILL_ARM_MAX_M
    }
}

/// What bounds one seam's reach over a window: the seam's own height range, the base range
/// over the cells under it, the lowest height of every seam in the window and a Lipschitz bound
/// of the base over the window (its largest cell gradient).
#[derive(Clone, Copy, Debug)]
pub(super) struct ReachBounds {
    pub(super) seam: (f64, f64),
    pub(super) base_under: (f64, f64),
    pub(super) window_low: f64,
    pub(super) slope: f64,
}

/// The exact pruning reach: beyond it the seam's cone has `h - k d <= min(b, H_lo)` and
/// `h + k d >= b` at every point of the window, so skipping it leaves `g = max(L, min(b, U))`
/// bitwise equal (`U >= H_lo` over the other seams). The first two terms bound where the cone
/// can bind the base, the third where its lower bound can set a conflict's `L`; in a conflict
/// only lower bounds decide. The cap where the base is as steep as the cone. `O(1)`.
pub(super) fn reach(bounds: &ReachBounds) -> f64 {
    let k = TIE_IN_SLOPE;
    if bounds.slope >= k {
        return CAP_M;
    }
    let (h_lo, h_hi) = bounds.seam;
    let (b_lo, b_hi) = bounds.base_under;
    let reach = [
        (h_hi - b_lo) / (k - bounds.slope),
        (b_hi - h_lo) / (k - bounds.slope),
        (h_hi - bounds.window_low) / k,
    ]
    .into_iter()
    .fold(0.0, f64::max);
    (reach * (1.0 + REACH_SLACK) + REACH_SLACK_M).min(CAP_M)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(cones: &[(f64, f64)]) -> Cones {
        let mut all = Cones::NONE;
        for &(height, distance) in cones {
            all.add(height, distance);
        }
        all
    }

    #[test]
    fn overlap_rule_is_order_free_and_continuous() {
        // Two seams 10 m apart at heights 0 and 10, a point midway on base 3: the cones
        // conflict and the upper seam's lower bound, 7.5, wins, in either order and in every
        // permutation of more.
        let cones = [
            (0.0, 5.0),
            (10.0, 5.0),
            (4.0, 9.0),
            (6.5, 7.25),
            (-2.0, 12.0),
        ];
        let expected = bounds(&cones).ground(3.0);
        assert_eq!(bounds(&cones[..2]).ground(3.0), 7.5);
        let mut order: Vec<usize> = (0..cones.len()).collect();
        // Every rotation and its reversal.
        for _ in 0..cones.len() {
            order.rotate_left(1);
            for reversed in [false, true] {
                let mut permuted: Vec<_> = order.iter().map(|&i| cones[i]).collect();
                if reversed {
                    permuted.reverse();
                }
                assert_eq!(bounds(&permuted).ground(3.0).to_bits(), expected.to_bits());
            }
        }
        // Consistent cones clamp the base, and the base is returned bitwise where none binds.
        assert_eq!(overlap(3.0, 1.0, 2.0), 2.0);
        assert_eq!(overlap(1.5, 1.0, 2.0).to_bits(), 1.5f64.to_bits());
        assert_eq!(Cones::NONE.ground(-7.25).to_bits(), (-7.25f64).to_bits());
        // Continuous where the cones start to conflict: sweep a seam's height through the
        // point where L = U and check that no step exceeds the change of its bound.
        for base in [-3.0, 0.0, 1.0, 2.5, 9.0] {
            let mut previous: Option<(f64, f64)> = None;
            for step in 0..=400 {
                let height = 1.0 + step as f64 * 0.01;
                let value = bounds(&[(0.0, 2.0), (height, 2.0)]).ground(base);
                if let Some((last_height, last)) = previous {
                    assert!(
                        (value - last).abs() <= (height - last_height) + 1e-12,
                        "jump at height {height}, base {base}: {last} -> {value}"
                    );
                }
                previous = Some((height, value));
            }
        }
    }

    #[test]
    fn the_inside_rule_continues_the_arm_and_caps_the_fill() {
        // Cut: the arm runs down at 1:2 at any depth.
        assert_eq!(inside(5.0, 1.0, 1.5), 1.0 - 0.75);
        // Fill: up at 1:2, no more than one texel diagonal.
        assert_eq!(inside(0.0, 1.0, 0.25), 1.0 + 0.125);
        assert_eq!(inside(0.0, 1.0, 3.0), 1.0 + 0.5 * FILL_ARM_MAX_M);
        assert!(inside_deep(5.0, 1.0) < 1.0 && inside_deep(0.0, 1.0) <= 1.0 + 0.36);
    }

    #[test]
    fn reach_is_capped_and_grows_with_slope_and_spread() {
        let flat = ReachBounds {
            seam: (1.0, 1.0),
            base_under: (1.0, 1.0),
            window_low: 1.0,
            slope: 0.0,
        };
        assert!(reach(&flat) < 1.0e-3);
        // A seam 5 m above the window's lowest can set a conflict's `L` within 10 m; a lower
        // seam's upper bound never decides one.
        let above = ReachBounds {
            seam: (6.0, 6.0),
            base_under: (6.0, 6.0),
            ..flat
        };
        assert!((reach(&above) - 10.0).abs() < 1.0e-3);
        let below = ReachBounds {
            window_low: -5.0,
            seam: (1.0, 1.0),
            ..flat
        };
        assert!((reach(&below) - 12.0).abs() < 1.0e-3);
        let steep = ReachBounds { slope: 0.5, ..flat };
        assert_eq!(reach(&steep), CAP_M);
    }
}
