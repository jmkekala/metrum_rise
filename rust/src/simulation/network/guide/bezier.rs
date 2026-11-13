// SPDX-License-Identifier: GPL-2.0-only

//! Cubic Bezier spans (the freeform tool's), with arc length by a fixed quadrature table.
//!
//! **Length quadrature.** The parameter range is cut into `TABLE_INTERVALS` (64) equal
//! intervals and each is integrated with the 5-point Gauss-Legendre rule, which is exact for a
//! polynomial speed of degree 9; the speed of a cubic is the square root of a quartic, smooth
//! wherever the curve has no cusp, and the rule's error on a 1/64 interval of a road-sized span
//! is far below `1e-9` relative (`tests.rs` checks it against 4,096 intervals). The cumulative
//! table maps arc length to the parameter: a binary search finds the interval, linear
//! interpolation gives the first guess, and exactly `NEWTON_STEPS` Newton steps on the
//! interval's own quadrature refine it. The step count is fixed for every input, so the result
//! is a pure function of the control points.
//!
//! **Minimum radius** is sampled: the radius `|B'|^3 / |B' x B''|` at `RADIUS_SAMPLES + 1`
//! equal parameter steps. A curvature peak narrower than a sample step (near a cusp) can be
//! missed; such a span has a radius below every road type's limit wherever it is sampled
//! closely, and the stroke validator refuses it on its flattened polyline.

use super::GuideError;
use glam::DVec2;

const TABLE_INTERVALS: usize = 64;
const RADIUS_SAMPLES: usize = 256;
const NEWTON_STEPS: usize = 2;
// 5-point Gauss-Legendre nodes and weights on [-1, 1].
const GAUSS_NODES: [f64; 5] = [
    0.0,
    -0.538_469_310_105_683_1,
    0.538_469_310_105_683_1,
    -0.906_179_845_938_664,
    0.906_179_845_938_664,
];
const GAUSS_WEIGHTS: [f64; 5] = [
    0.568_888_888_888_888_9,
    0.478_628_670_499_366_5,
    0.478_628_670_499_366_5,
    0.236_926_885_056_189_1,
    0.236_926_885_056_189_1,
];

/// A cubic Bezier span `p0 p1 p2 p3`, with non-zero end handles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicBezier {
    points: [DVec2; 4],
}

// Cumulative arc length at the `TABLE_INTERVALS + 1` equal parameter steps (on the stack).
pub(super) struct ArcLengthTable {
    cumulative: [f64; TABLE_INTERVALS + 1],
}

impl ArcLengthTable {
    pub(super) fn length(&self) -> f64 {
        self.cumulative[TABLE_INTERVALS]
    }
}

impl CubicBezier {
    /// The span through `p0` and `p3` with handles `p1` and `p2`. Refused when an end handle
    /// has zero length (no end tangent) or the ends coincide.
    pub fn new(p0: DVec2, p1: DVec2, p2: DVec2, p3: DVec2) -> Result<Self, GuideError> {
        if p1 == p0 || p2 == p3 || p0 == p3 {
            return Err(GuideError::Degenerate);
        }
        Ok(Self {
            points: [p0, p1, p2, p3],
        })
    }

    /// The control points `[p0, p1, p2, p3]`.
    pub fn points(&self) -> [DVec2; 4] {
        self.points
    }

    /// The point at parameter `t` in `[0, 1]`.
    pub fn position(&self, t: f64) -> DVec2 {
        let [p0, p1, p2, p3] = self.points;
        let u = 1.0 - t;
        p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t)
    }

    /// The first derivative with respect to the parameter.
    pub fn derivative(&self, t: f64) -> DVec2 {
        let [p0, p1, p2, p3] = self.points;
        let u = 1.0 - t;
        ((p1 - p0) * (u * u) + (p2 - p1) * (2.0 * u * t) + (p3 - p2) * (t * t)) * 3.0
    }

    /// The second derivative with respect to the parameter.
    pub fn second_derivative(&self, t: f64) -> DVec2 {
        let [p0, p1, p2, p3] = self.points;
        ((p2 - p1 * 2.0 + p0) * (1.0 - t) + (p3 - p2 * 2.0 + p1) * t) * 6.0
    }

    fn speed(&self, t: f64) -> f64 {
        self.derivative(t).length()
    }

    // Arc length from `t0` to `t1` by the 5-point Gauss-Legendre rule.
    fn quadrature(&self, t0: f64, t1: f64) -> f64 {
        let half = 0.5 * (t1 - t0);
        let mid = 0.5 * (t0 + t1);
        let mut sum = 0.0;
        for (node, weight) in GAUSS_NODES.iter().zip(GAUSS_WEIGHTS) {
            sum += weight * self.speed(mid + half * node);
        }
        sum * half
    }

    pub(super) fn arc_length_table(&self) -> ArcLengthTable {
        let mut cumulative = [0.0; TABLE_INTERVALS + 1];
        for index in 0..TABLE_INTERVALS {
            let t0 = index as f64 / TABLE_INTERVALS as f64;
            let t1 = (index + 1) as f64 / TABLE_INTERVALS as f64;
            cumulative[index + 1] = cumulative[index] + self.quadrature(t0, t1);
        }
        ArcLengthTable { cumulative }
    }

    // The parameter at arc length `s` (clamped), from the table plus the fixed Newton steps.
    pub(super) fn parameter_at(&self, table: &ArcLengthTable, s: f64) -> f64 {
        if s <= 0.0 {
            return 0.0;
        }
        if s >= table.length() {
            return 1.0;
        }
        // The interval `[c[i], c[i + 1])` holding `s`.
        let index = table.cumulative.partition_point(|&c| c <= s) - 1;
        let t0 = index as f64 / TABLE_INTERVALS as f64;
        let t1 = (index + 1) as f64 / TABLE_INTERVALS as f64;
        let (c0, c1) = (table.cumulative[index], table.cumulative[index + 1]);
        let mut t = if c1 > c0 {
            t0 + (t1 - t0) * ((s - c0) / (c1 - c0))
        } else {
            t0
        };
        for _ in 0..NEWTON_STEPS {
            let speed = self.speed(t);
            if speed == 0.0 {
                break;
            }
            t = (t - (c0 + self.quadrature(t0, t) - s) / speed).clamp(t0, t1);
        }
        t
    }

    /// Arc length in metres by the documented quadrature (module docs).
    pub fn length(&self) -> f64 {
        self.arc_length_table().length()
    }

    /// The point at arc length `s`, clamped; the ends are returned exactly.
    pub fn point_at(&self, s: f64) -> DVec2 {
        let table = self.arc_length_table();
        if s >= table.length() {
            return self.points[3];
        }
        if s <= 0.0 {
            return self.points[0];
        }
        self.position(self.parameter_at(&table, s))
    }

    /// The unit tangent at arc length `s` (zero at an interior cusp).
    pub fn tangent_at(&self, s: f64) -> DVec2 {
        let table = self.arc_length_table();
        self.derivative(self.parameter_at(&table, s))
            .normalize_or_zero()
    }

    /// The unit tangent at the start, along the first handle.
    pub fn start_tangent(&self) -> DVec2 {
        (self.points[1] - self.points[0]).normalize()
    }

    /// The unit tangent at the end, along the last handle.
    pub fn end_tangent(&self) -> DVec2 {
        (self.points[3] - self.points[2]).normalize()
    }

    /// Signed curvature at parameter `t` (positive turning left); infinite where the speed is
    /// zero (a cusp).
    pub fn signed_curvature(&self, t: f64) -> f64 {
        let first = self.derivative(t);
        let speed = first.length();
        if speed == 0.0 {
            return f64::INFINITY;
        }
        first.perp_dot(self.second_derivative(t)) / (speed * speed * speed)
    }

    /// The sampled minimum radius of curvature (module docs): infinite for a straight span,
    /// zero where a sample lands on a cusp.
    pub fn min_radius(&self) -> f64 {
        Self::radius_sample_parameters()
            .map(|t| 1.0 / self.signed_curvature(t).abs())
            .fold(f64::INFINITY, f64::min)
    }

    // The curvature samples of `min_radius`, as parameters.
    pub(super) fn radius_sample_parameters() -> impl Iterator<Item = f64> {
        (0..=RADIUS_SAMPLES).map(|index| index as f64 / RADIUS_SAMPLES as f64)
    }

    // Split at `0 < s < length` by de Casteljau; both halves share the split point and keep this
    // span's ends.
    pub(super) fn split_at(&self, s: f64) -> (Self, Self) {
        let table = self.arc_length_table();
        let t = self.parameter_at(&table, s);
        let [p0, p1, p2, p3] = self.points;
        let (q0, q1, q2) = (p0.lerp(p1, t), p1.lerp(p2, t), p2.lerp(p3, t));
        let (r0, r1) = (q0.lerp(q1, t), q1.lerp(q2, t));
        let split = r0.lerp(r1, t);
        (
            Self {
                points: [p0, q0, r0, split],
            },
            Self {
                points: [split, r1, q2, p3],
            },
        )
    }

    /// The same span travelled from `p3` to `p0` (control points reversed: exact).
    pub fn reversed(&self) -> Self {
        let [p0, p1, p2, p3] = self.points;
        Self {
            points: [p3, p2, p1, p0],
        }
    }

    // The span with `p0` replaced and `p1` moved with it (a joint shared after an offset).
    pub(super) fn with_start(&self, start: DVec2) -> Self {
        let [p0, p1, p2, p3] = self.points;
        Self {
            points: [start, p1 + (start - p0), p2, p3],
        }
    }

    // Writes the interior points of `count` equal arc-length steps (not the start, then the
    // exact end).
    pub(super) fn append_flattened(
        &self,
        table: &ArcLengthTable,
        count: usize,
        out: &mut Vec<DVec2>,
    ) {
        let length = table.length();
        for index in 1..count {
            let s = length * (index as f64 / count as f64);
            out.push(self.position(self.parameter_at(table, s)));
        }
        out.push(self.points[3]);
    }
}
