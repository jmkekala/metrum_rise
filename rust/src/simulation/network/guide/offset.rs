// SPDX-License-Identifier: GPL-2.0-only

//! Offsets of a guide by a signed distance: lines to parallel lines and arcs to concentric arcs
//! (exact), Bezier spans by a sampled offset. The parallel mode's copy (`ROAD-112` round K, plan
//! 6.4) and the shift in Replace (`ROAD-113`) build on these; a one-way copy is the reversed
//! offset path.
//!
//! A positive distance offsets to the left of the direction of travel. An offset towards a
//! centre of curvature is refused where the radius is not larger than the distance (the offset
//! curve would reach the centre and fold over).
//!
//! A Bezier span is offset at its own equal arc-length steps, so a step of the offset is the
//! span's step times the stretch `1 - k d`; on the outside of a near cusp (a folded or flipped
//! handle: radius near zero, the curvature finite but huge) the stretch, and with it the step
//! count, was unbounded and a pose ran the game out of memory (`ROAD-112` round FX). A span
//! whose radius falls under `FLATTEN_RADIUS_FLOOR_M` is refused ([`OffsetRefusal::TooTight`]),
//! its speed minima read with the samples (`bezier.rs`), so the stretch is at most
//! `1 + |d| / 1 m`; and an offset keeps the flattening's bound of `FLATTEN_MAX_STEPS` steps
//! ([`OffsetRefusal::TooManySteps`]).

use super::bezier::CubicBezier;
use super::flatten::{FLATTEN_RADIUS_FLOOR_M, TooManySteps, check_steps, flatten_count};
use super::{DVec2, GuidePath, GuideSegment};

// Joints closer to tangent-continuous than this sine are offset as smooth joints; the
// constructions leave their joints within rounding (about 1e-16) of it.
const SMOOTH_JOINT_SIN_MAX: f64 = 1e-9;

/// Why an offset was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OffsetRefusal {
    /// The offset reaches the centre of curvature: somewhere on this segment the radius is not
    /// larger than the offset towards it.
    RadiusUnderOffset {
        /// The segment's index in the path.
        segment: usize,
    },
    /// The path turns a corner at the start of this segment (its tangent differs from the
    /// previous end tangent), so offset pieces would leave a gap or overlap.
    Corner {
        /// The index of the segment that starts at the corner.
        joint: usize,
    },
    /// This segment is a Bezier span, which has no closed-form offset; use
    /// `GuidePath::flatten_offset_into`.
    NotClosedForm {
        /// The segment's index in the path.
        segment: usize,
    },
    /// This Bezier span's radius falls under `FLATTEN_RADIUS_FLOOR_M` somewhere (a cusp or a
    /// tiny loop: a folded or flipped handle), which the flattening rule does not resolve.
    TooTight {
        /// The segment's index in the path.
        segment: usize,
    },
    /// The offset would write more than `FLATTEN_MAX_STEPS` steps.
    TooManySteps,
}

impl OffsetRefusal {
    fn at(self, index: usize) -> Self {
        match self {
            Self::RadiusUnderOffset { .. } => Self::RadiusUnderOffset { segment: index },
            Self::Corner { .. } => Self::Corner { joint: index },
            Self::NotClosedForm { .. } => Self::NotClosedForm { segment: index },
            Self::TooTight { .. } => Self::TooTight { segment: index },
            Self::TooManySteps => Self::TooManySteps,
        }
    }
}

impl From<TooManySteps> for OffsetRefusal {
    fn from(_: TooManySteps) -> Self {
        Self::TooManySteps
    }
}

// The exact offset of a line or an arc.
fn exact_offset(
    segment: &GuideSegment,
    d: f64,
    index: usize,
) -> Result<GuideSegment, OffsetRefusal> {
    match segment {
        GuideSegment::Line(line) => Ok(GuideSegment::Line(line.offset(d))),
        GuideSegment::Arc(arc) => arc
            .offset(d)
            .map(GuideSegment::Arc)
            .map_err(|refusal| refusal.at(index)),
        GuideSegment::Bezier(_) => Err(OffsetRefusal::NotClosedForm { segment: index }),
    }
}

// Writes the sampled offset of a Bezier span: `position + d * left normal` at the span's own
// equal arc-length steps, counted by the flattening rule for the offset curve. The offset
// stretches a step by `1 - k d` (signed curvature `k`) and has radius `(1 - k d) / |k|`, so the
// rule takes the length times the largest stretch and the smallest offset radius over the
// curvature samples and the speed minima; a sample with `1 - k d <= 0` refuses, and so does a
// radius under `FLATTEN_RADIUS_FLOOR_M` (module docs), which bounds the stretch by
// `1 + |d| / 1 m`. `skip_start` leaves out the first point (a joint already written).
fn append_bezier_offset(
    curve: &CubicBezier,
    d: f64,
    index: usize,
    skip_start: bool,
    out: &mut Vec<DVec2>,
) -> Result<(), OffsetRefusal> {
    let refusal = OffsetRefusal::RadiusUnderOffset { segment: index };
    let too_tight = OffsetRefusal::TooTight { segment: index };
    let stretch_at = |t: f64| {
        let curvature = curve.signed_curvature(t);
        if !curvature.is_finite() || curvature.abs() * FLATTEN_RADIUS_FLOOR_M > 1.0 {
            return Err(too_tight);
        }
        let stretch = 1.0 - curvature * d;
        if stretch > 0.0 {
            Ok((stretch, stretch / curvature.abs()))
        } else {
            Err(refusal)
        }
    };
    let minima = curve.speed_minima();
    for t in minima.into_iter().flatten() {
        // The robust radius at a speed minimum: a fold's control points on one line read 0.
        if curve.radius_at_speed_minimum(t) < FLATTEN_RADIUS_FLOOR_M {
            return Err(too_tight);
        }
    }
    let (mut max_stretch, mut min_radius) = (1.0_f64, f64::INFINITY);
    for t in CubicBezier::radius_sample_parameters().chain(minima.into_iter().flatten()) {
        let (stretch, radius) = stretch_at(t)?;
        max_stretch = max_stretch.max(stretch);
        min_radius = min_radius.min(radius);
    }
    let table = curve.arc_length_table();
    let length = table.length();
    let count = flatten_count(length * max_stretch, min_radius);
    check_steps(out.len() + usize::from(!skip_start), count)?;
    for step in usize::from(skip_start)..=count {
        let t = match step {
            0 => 0.0,
            _ if step == count => 1.0,
            _ => curve.parameter_at(&table, length * (step as f64 / count as f64)),
        };
        stretch_at(t)?;
        let normal = curve.derivative(t).normalize_or_zero().perp();
        out.push(curve.position(t) + normal * d);
    }
    Ok(())
}

impl GuidePath {
    // Refuses a corner: an offset is defined here for tangent-continuous paths only.
    fn check_smooth_joints(&self) -> Result<(), OffsetRefusal> {
        for (index, pair) in self.segments().windows(2).enumerate() {
            let (before, after) = (pair[0].end_tangent(), pair[1].start_tangent());
            if before.dot(after) <= 0.0 || before.perp_dot(after).abs() > SMOOTH_JOINT_SIN_MAX {
                return Err(OffsetRefusal::Corner { joint: index + 1 });
            }
        }
        Ok(())
    }

    /// The exact offset by `d` metres (left of travel when positive): each line to its parallel,
    /// each arc to its concentric arc with the same sweep, joints shared bit for bit. Refused for
    /// a corner, for an arc whose radius is not larger than an offset towards its centre, and
    /// for a Bezier span (`flatten_offset_into` samples those). `O(segments)`.
    pub fn offset(&self, d: f64) -> Result<GuidePath, OffsetRefusal> {
        self.check_smooth_joints()?;
        let mut shifted = GuidePath::new();
        for (index, segment) in self.segments().iter().enumerate() {
            let piece = exact_offset(segment, d, index)?;
            let piece = match shifted.end() {
                Some(joint) => piece.with_start(joint),
                None => piece,
            };
            shifted.push(piece);
        }
        Ok(shifted)
    }

    /// Writes the flattened offset by `d` into `out` (cleared first; unspecified after a
    /// refusal): lines and arcs offset exactly and flattened by the rule, Bezier spans offset
    /// per sample (`position + d * normal` at their own arc-length steps, counted by the rule
    /// for the offset curve) and refused where the radius is not larger than an offset towards
    /// the centre, and for a Bezier span whose radius falls under `FLATTEN_RADIUS_FLOOR_M` (a
    /// cusp or tiny loop). Refused beyond `FLATTEN_MAX_STEPS` steps, before writing them. The
    /// polyline a parallel copy sends to the back end. `O(points)`, at most
    /// `FLATTEN_MAX_STEPS + 1`, no allocation beyond growing `out`.
    pub fn flatten_offset_into(&self, d: f64, out: &mut Vec<DVec2>) -> Result<(), OffsetRefusal> {
        out.clear();
        self.check_smooth_joints()?;
        for (index, segment) in self.segments().iter().enumerate() {
            match segment {
                GuideSegment::Bezier(curve) => {
                    append_bezier_offset(curve, d, index, !out.is_empty(), out)?;
                }
                _ => {
                    let piece = exact_offset(segment, d, index)?;
                    let piece = match out.last() {
                        Some(&joint) => piece.with_start(joint),
                        None => {
                            out.push(piece.start());
                            piece
                        }
                    };
                    piece.append_flattened(out)?;
                }
            }
        }
        Ok(())
    }
}
