// SPDX-License-Identifier: GPL-2.0-only

//! The grading rules (`TERRAIN-07`): tie-in cones, the overlap rule, the inside rule under a
//! road, and the reaches and tests of the two-pass exact pruning that bound where a seam can
//! change a value.
//!
//! The overlap rule (`TD7`, the user's `T2` decision) is `g = max(L, min(b, U))`: the base
//! clamped into `[L, U]`, and `L` where the cones conflict, so the upper road's embankment wins
//! and the lower road gets a retaining wall. At a seam its own cone gives `L >= h >= U`, so the
//! graded ground never lies below a seam.

use super::seams::Seam;

/// Tie-in slope ratio `k` of every seam kind: 1:2.
pub const TIE_IN_SLOPE: f64 = 0.5;
/// The cap `C`: a seam farther than this from a point never affects it, metres.
pub const CAP_M: f64 = 64.0;
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
        let (lower, upper) = cone(height, distance);
        self.lower = self.lower.max(lower);
        self.upper = self.upper.min(upper);
    }

    /// The overlap rule on these bounds.
    pub(super) fn ground(&self, base: f64) -> f64 {
        overlap(base, self.lower, self.upper)
    }
}

/// The cone interval `[h - k d, h + k d]` of a seam at height `height`, `distance` from a
/// point.
pub(super) fn cone(height: f64, distance: f64) -> (f64, f64) {
    (
        height - TIE_IN_SLOPE * distance,
        height + TIE_IN_SLOPE * distance,
    )
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

/// The inside rule under a road, `depth` inside the nearest road seam at height `height`: the
/// seam's tie-in arm continued down, on cuts and fills alike (`TERRAIN-07-D` round 8, T34 (b)).
/// Outside, the seam's own cone bounds the ground by `h + k d`, so every lattice value is at
/// most `h + k s` for its signed distance `s` from a straight seam and any triangle's surface on
/// the seam is at most `h`: the cut edge never stands above a road edge but in a conflict, whose
/// walls close it, and the skirt covers its sag (at most `k s / 2` for a lattice spacing `s`).
/// The fill arm that rose under fills put the cut edge above the edge where the ground beyond
/// rose too.
pub(super) fn inside(height: f64, depth: f64) -> f64 {
    height - TIE_IN_SLOPE * depth
}

/// The inside rule deeper than [`INSIDE_REACH_M`] from every road seam, from the road surface
/// `road` there: the arm's value at that depth. Only coarse levels' triangles reach such
/// samples, and the mask discards them.
pub(super) fn inside_deep(road: f64) -> f64 {
    road - TIE_IN_SLOPE * INSIDE_REACH_M
}

/// Whether a point under a road, `road_distance` from its nearest road seam (`+∞` with none
/// within the band), takes the ground's value instead of the inside rule: where a wall seam of a
/// structure mouth (`TERRAIN-07-C`, `docs/terrain.md` T22 4 (a)) lies strictly nearer, within
/// the band `band`. Along a road seam the inside rule's arm continues the seam's cone, so the
/// rendered cut edge is the seam; a mouth row has no cone to continue, and a hidden inside value
/// there (the fill arm rose up to 0.35 m above the road before T34) would lift the cut edge and the terrain
/// beyond the row above the deck (`TERRAIN-07-D` round 5). With the ground's value on both sides
/// the cut edge along the row is the graded ground the structure walls close. `O(walls)`.
pub(super) fn continues_ground(
    walls: &[Seam],
    x: f64,
    z: f64,
    road_distance: f64,
    band: f64,
) -> bool {
    walls.iter().any(|wall| {
        let distance = wall.at(x, z).distance;
        distance < band && distance < road_distance
    })
}

/// What bounds one seam's base reach over a window: the seam's own height range, the base range
/// over the cells under it and a Lipschitz bound of the base over the window (its largest cell
/// gradient).
#[derive(Clone, Copy, Debug)]
pub(super) struct ReachBounds {
    pub(super) seam: (f64, f64),
    pub(super) base_under: (f64, f64),
    pub(super) slope: f64,
}

// A computed reach grown so that rounding in the bound can only make it larger.
fn padded(reach: f64) -> f64 {
    reach * (1.0 + REACH_SLACK) + REACH_SLACK_M
}

/// The Lipschitz base reach: beyond it the seam's cone holds the base, `h - k d <= b` and
/// `h + k d >= b`, at every point of the window. The cap where the base is as steep as the cone.
/// `O(1)`.
pub(super) fn base_reach(bounds: &ReachBounds) -> f64 {
    let k = TIE_IN_SLOPE;
    if bounds.slope >= k {
        return padded(CAP_M);
    }
    let (h_lo, h_hi) = bounds.seam;
    let (b_lo, b_hi) = bounds.base_under;
    let reach = ((h_hi - b_lo) / (k - bounds.slope))
        .max((b_hi - h_lo) / (k - bounds.slope))
        .max(0.0);
    padded(reach).min(padded(CAP_M))
}

/// The bind reach over a base range `base`: beyond it the cone of a seam with heights `seam`
/// holds every base in that range, `h_hi - k d <= lo` and `h_lo + k d >= hi`. `O(1)`.
pub(super) fn bind_reach(seam: (f64, f64), base: (f64, f64)) -> f64 {
    let reach = (seam.1 - base.0).max(base.1 - seam.0).max(0.0) / TIE_IN_SLOPE;
    padded(reach).min(padded(CAP_M))
}

/// The lift reach: beyond it the lower bound `h - k d` of a seam whose highest point is
/// `seam_hi` stays at most `floor`, so it cannot raise a value of at least `floor`. Negative
/// infinity where the seam never rises above `floor`. `O(1)`.
pub(super) fn lift_reach(seam_hi: f64, floor: f64) -> f64 {
    let reach = (seam_hi - floor) / TIE_IN_SLOPE;
    if reach > 0.0 {
        padded(reach).min(padded(CAP_M))
    } else {
        f64::NEG_INFINITY
    }
}

/// The band reach: the band `band` grown like a reach, so a seam a sample names as its nearest
/// within the band is always listed.
pub(super) fn band_reach(band: f64) -> f64 {
    padded(band)
}

/// The early exit of a sample's evaluation: whether no seam at least `distance` from the point
/// with heights within `heights` (`(lowest, highest)`) can change the sample's value or its
/// nearest road seam (at `nearest`, `+inf` without one, within `band`), or every such seam lies
/// beyond the cap. A covered sample (`base` `None`) reads only the nearest road seam (the inside
/// rule). A ground sample's value is `g = max(L, min(b, U))` over `cones`: a seam whose lower
/// bound stays below `g` and whose upper bound stays above `min(b, U)` leaves `min(b, U)` and
/// `g` as they are, and the overlap rule then returns the same operand, bitwise; the bounds it
/// would tighten are not part of the value. The margin is a padded reach's, so rounding never
/// stops early. `O(1)`.
pub(super) fn settled(
    cones: &Cones,
    base: Option<f64>,
    heights: (f64, f64),
    distance: f64,
    nearest: f64,
    band: f64,
) -> bool {
    let k = TIE_IN_SLOPE;
    let farther = distance - REACH_SLACK_M;
    if farther > padded(CAP_M) {
        return true;
    }
    if farther <= nearest.min(band) * (1.0 + REACH_SLACK) {
        return false;
    }
    base.is_none_or(|base| {
        let floor = base.min(cones.upper);
        heights.1 - k * farther < cones.lower.max(floor) && heights.0 + k * farther > floor
    })
}

/// Pass 1 of the exact pruning: whether a seam with heights `seam` may change a value at points
/// at least `distance` from it whose base lies in `base`, because its cone can bind that base or
/// it lies within the band `band` (a road seam's; a pad seam takes none). Where it is false the
/// cone holds the base there and the seam is not the nearest. `O(1)`.
pub(super) fn may_bind(seam: (f64, f64), base: (f64, f64), distance: f64, band: f64) -> bool {
    distance <= bind_reach(seam, base).max(band_reach(band))
}

/// Pass 2 of the exact pruning: whether a seam whose highest point is `seam_hi` may lift a
/// first-pass ground of at least `floor` at points at least `distance` from it. `O(1)`.
pub(super) fn may_lift(seam_hi: f64, floor: f64, distance: f64) -> bool {
    distance <= lift_reach(seam_hi, floor)
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
    fn the_inside_rule_continues_the_arm_down() {
        // The arm runs down at 1:2 at any depth, on cuts and fills alike.
        assert_eq!(inside(1.0, 1.5), 1.0 - 0.75);
        assert_eq!(inside(1.0, 0.25), 1.0 - 0.125);
        assert_eq!(inside_deep(1.0), 1.0 - TIE_IN_SLOPE * INSIDE_REACH_M);
    }

    #[test]
    fn reach_is_capped_and_grows_with_slope_and_spread() {
        let flat = ReachBounds {
            seam: (1.0, 1.0),
            base_under: (1.0, 1.0),
            slope: 0.0,
        };
        assert!(base_reach(&flat) < 1.0e-3);
        // A seam 5 m above the base binds it within 10 m, at a 0.25 slope within 20 m.
        let above = ReachBounds {
            seam: (6.0, 6.0),
            ..flat
        };
        assert!((base_reach(&above) - 10.0).abs() < 1.0e-3);
        let sloped = ReachBounds {
            slope: 0.25,
            ..above
        };
        assert!((base_reach(&sloped) - 20.0).abs() < 1.0e-3);
        let steep = ReachBounds { slope: 0.5, ..flat };
        assert!(base_reach(&steep) >= CAP_M);
        // Over a base range: the farther side of the range decides.
        assert!((bind_reach((1.0, 1.0), (-2.0, 3.0)) - 6.0).abs() < 1.0e-3);
        assert!(may_bind((1.0, 1.0), (-2.0, 3.0), 5.9, 2.0));
        assert!(!may_bind((1.0, 1.0), (-2.0, 3.0), 6.1, 2.0));
        assert!(may_bind((1.0, 1.0), (1.0, 1.0), 1.9, 2.0));
        // A seam 5 m above a floor can lift it within 10 m; one below it never.
        assert!((lift_reach(6.0, 1.0) - 10.0).abs() < 1.0e-3);
        assert!(may_lift(6.0, 1.0, 9.9) && !may_lift(6.0, 1.0, 10.1));
        assert!(!may_lift(1.0, 1.0, 0.0) && !may_lift(1.0, f64::INFINITY, 0.0));
        assert!(lift_reach(200.0, 0.0) <= CAP_M + 1.0e-3);
    }
}
