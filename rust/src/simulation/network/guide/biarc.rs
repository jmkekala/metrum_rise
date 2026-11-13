// SPDX-License-Identifier: GPL-2.0-only

//! The complex curve's S-curve: a biarc between two points with given tangents.

use super::{GuideError, GuidePath, GuideSegment};
use glam::DVec2;

impl GuidePath {
    /// Replaces the path with the biarc from `a` leaving along `start_tangent` to `b` arriving
    /// along `end_tangent`, by the equal-tangent-length rule: the control points
    /// `q1 = a + d t_a` and `q2 = b - d t_b` lie `2d` apart, and the joint is their midpoint.
    /// `d` is the positive root of `2 (t_a.t_b - 1) d^2 - 2 (v.t) d + v.v = 0` with
    /// `v = b - a`, `t = t_a + t_b`, taken in the cancellation-free form
    /// `d = v.v / (v.t + sqrt((v.t)^2 + 2 (1 - t_a.t_b) v.v))`. Each half is the arc (or line)
    /// tangent to its end and to `q2 - q1` at the joint, so the joint is tangent-continuous.
    /// Refused (`NoBiarc`) for parallel tangents pointing away along the chord, and
    /// (`Degenerate`) when the joint falls on an end, as for `b` straight behind `a` with the
    /// tangent reversed. Closed form, `O(1)`, no allocation once the path has capacity for two
    /// segments.
    pub fn set_biarc(
        &mut self,
        a: DVec2,
        start_tangent: DVec2,
        b: DVec2,
        end_tangent: DVec2,
    ) -> Result<(), GuideError> {
        let (t_a, t_b) = (
            start_tangent.normalize_or_zero(),
            end_tangent.normalize_or_zero(),
        );
        let chord = b - a;
        if t_a == DVec2::ZERO || t_b == DVec2::ZERO || chord == DVec2::ZERO {
            return Err(GuideError::Degenerate);
        }
        let chord_along = chord.dot(t_a + t_b);
        let chord_sq = chord.length_squared();
        let root = (chord_along * chord_along + 2.0 * (1.0 - t_a.dot(t_b)) * chord_sq).sqrt();
        let denominator = chord_along + root;
        if denominator <= 0.0 {
            return Err(GuideError::NoBiarc);
        }
        let d = chord_sq / denominator;
        let joint = ((a + t_a * d) + (b - t_b * d)) * 0.5;
        let head = GuideSegment::tangent_to(a, t_a, joint)?;
        // Built backwards from `b` so the end tangent comes from `t_b` itself.
        let tail = GuideSegment::tangent_to(b, -t_b, joint)?.reversed();
        self.clear();
        self.push(head);
        self.push(tail);
        Ok(())
    }
}
