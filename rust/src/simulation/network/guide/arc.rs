// SPDX-License-Identifier: GPL-2.0-only

//! Exact circular arcs, stored as their two ends plus the bulge, and the simple curve's
//! three-click fillet (`ROAD-112` decision U1).
//!
//! The bulge is `tan(sweep / 4)`, signed like the sweep (positive turns towards the left normal).
//! It covers every sweep under a full turn, needs no centre (so a nearly straight arc with a huge
//! radius stays well conditioned), reverses exactly by swapping the ends and negating it, and
//! gives the half-sweep's cosine and sine without trigonometry:
//! `cos(sweep / 2) = (1 - b^2) / (1 + b^2)`, `sin(sweep / 2) = 2b / (1 + b^2)`. The tangent at
//! each end is the chord direction turned by half the sweep, back at the start and on at the end.

use super::{GuideError, GuidePath, GuideSegment, Line, OffsetRefusal};
use glam::DVec2;
use std::f64::consts::TAU;

/// An exact circular arc from `start` to `end` with the given bulge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arc {
    start: DVec2,
    end: DVec2,
    bulge: f64,
}

// The arc's derived quantities, closed form from the chord and the bulge.
struct Frame {
    // Unit tangent at the start.
    start_tangent: DVec2,
    // Unit tangent at the end.
    end_tangent: DVec2,
    // Signed sweep in radians, `4 atan(bulge)`.
    sweep: f64,
    length: f64,
    // Positive for a left turn: the centre is `start + start_tangent.perp() * signed_radius`.
    signed_radius: f64,
}

// `v` turned by the angle whose cosine and sine are given.
fn rotate(v: DVec2, cos: f64, sin: f64) -> DVec2 {
    DVec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
}

// The bulge of the arc leaving `start` along `tangent` and ending at `end`: the half-angle
// `tan(alpha / 2) = cross / (|v| + dot)` of the angle `alpha` from the tangent to the chord `v`,
// which is half the sweep. Zero when `end` lies ahead on the tangent.
pub(super) fn tangent_bulge(start: DVec2, tangent: DVec2, end: DVec2) -> Result<f64, GuideError> {
    let tangent = tangent.normalize_or_zero();
    let chord = end - start;
    let chord_length = chord.length();
    if tangent == DVec2::ZERO || chord_length == 0.0 {
        return Err(GuideError::Degenerate);
    }
    let denominator = chord_length + tangent.dot(chord);
    if denominator <= 0.0 {
        return Err(GuideError::FullTurn);
    }
    Ok(tangent.perp_dot(chord) / denominator)
}

impl Arc {
    /// The arc from `start` to `end` with bulge `tan(sweep / 4)`. Refused when the ends
    /// coincide or the bulge is zero (a line) or not finite (a full turn).
    pub fn from_bulge(start: DVec2, end: DVec2, bulge: f64) -> Result<Self, GuideError> {
        if start == end || bulge == 0.0 || !bulge.is_finite() {
            return Err(GuideError::Degenerate);
        }
        Ok(Self { start, end, bulge })
    }

    /// The arc of `radius` about `centre` from `start_angle` through the signed `sweep`
    /// (radians from +X towards +Z). Refused unless `radius > 0` and `0 < |sweep| < 2 pi`.
    pub fn from_centre(
        centre: DVec2,
        radius: f64,
        start_angle: f64,
        sweep: f64,
    ) -> Result<Self, GuideError> {
        if !(radius > 0.0 && sweep != 0.0 && sweep.abs() < TAU) {
            return Err(GuideError::Degenerate);
        }
        let (sin_a, cos_a) = start_angle.sin_cos();
        let (sin_b, cos_b) = (start_angle + sweep).sin_cos();
        Self::from_bulge(
            centre + DVec2::new(cos_a, sin_a) * radius,
            centre + DVec2::new(cos_b, sin_b) * radius,
            (sweep / 4.0).tan(),
        )
    }

    /// The first point.
    pub fn start(&self) -> DVec2 {
        self.start
    }

    /// The last point.
    pub fn end(&self) -> DVec2 {
        self.end
    }

    /// `tan(sweep / 4)`, positive for a left turn.
    pub fn bulge(&self) -> f64 {
        self.bulge
    }

    fn frame(&self) -> Frame {
        let chord = self.end - self.start;
        let chord_length = chord.length();
        let chord_unit = chord / chord_length;
        let b = self.bulge;
        let one_plus = 1.0 + b * b;
        let half_cos = (1.0 - b * b) / one_plus;
        let half_sin = 2.0 * b / one_plus;
        let half_turn = b.atan();
        Frame {
            start_tangent: rotate(chord_unit, half_cos, -half_sin),
            end_tangent: rotate(chord_unit, half_cos, half_sin),
            sweep: 4.0 * half_turn,
            // R |sweep| with R = c (1 + b^2) / (4 |b|); atan(b) / b stays exact as b -> 0.
            length: chord_length * one_plus * (half_turn / b),
            signed_radius: chord_length * one_plus / (4.0 * b),
        }
    }

    /// The signed sweep in radians (positive turns from +X towards +Z), under a full turn.
    pub fn sweep(&self) -> f64 {
        4.0 * self.bulge.atan()
    }

    /// The radius in metres.
    pub fn radius(&self) -> f64 {
        self.frame().signed_radius.abs()
    }

    /// The centre of the circle.
    pub fn centre(&self) -> DVec2 {
        let frame = self.frame();
        self.start + frame.start_tangent.perp() * frame.signed_radius
    }

    /// The angle of the start as seen from the centre (radians from +X towards +Z).
    pub fn start_angle(&self) -> f64 {
        let offset = self.start - self.centre();
        offset.y.atan2(offset.x)
    }

    /// Arc length in metres, closed form.
    pub fn length(&self) -> f64 {
        self.frame().length
    }

    /// The unit tangent at the start.
    pub fn start_tangent(&self) -> DVec2 {
        self.frame().start_tangent
    }

    /// The unit tangent at the end, in the direction of travel.
    pub fn end_tangent(&self) -> DVec2 {
        self.frame().end_tangent
    }

    /// The point at arc length `s`, clamped; the ends are returned exactly.
    pub fn point_at(&self, s: f64) -> DVec2 {
        let frame = self.frame();
        if s <= 0.0 {
            self.start
        } else if s >= frame.length {
            self.end
        } else {
            point_at_turn(self.start, &frame, frame.sweep * (s / frame.length))
        }
    }

    /// The unit tangent at arc length `s`, clamped.
    pub fn tangent_at(&self, s: f64) -> DVec2 {
        let frame = self.frame();
        let turn = frame.sweep * (s / frame.length).clamp(0.0, 1.0);
        let (sin, cos) = turn.sin_cos();
        rotate(frame.start_tangent, cos, sin)
    }

    // Split at `0 < s < length`: both pieces keep the circle (to rounding) and share the split
    // point; their sweeps add up to this arc's.
    pub(super) fn split_at(&self, s: f64) -> (Self, Self) {
        let frame = self.frame();
        let head_sweep = frame.sweep * (s / frame.length);
        let split = point_at_turn(self.start, &frame, head_sweep);
        (
            Self {
                start: self.start,
                end: split,
                bulge: (head_sweep / 4.0).tan(),
            },
            Self {
                start: split,
                end: self.end,
                bulge: ((frame.sweep - head_sweep) / 4.0).tan(),
            },
        )
    }

    /// The same arc travelled from `end` to `start` (ends swapped, bulge negated: exact).
    pub fn reversed(&self) -> Self {
        Self {
            start: self.end,
            end: self.start,
            bulge: -self.bulge,
        }
    }

    /// The concentric arc `d` metres to the left of the direction of travel (right for negative
    /// `d`), with the same sweep. Refused when the offset reaches the centre (the radius is not
    /// larger than an offset towards it).
    pub fn offset(&self, d: f64) -> Result<Self, OffsetRefusal> {
        let frame = self.frame();
        // The left offset of a left turn shrinks the radius: 1 - d / R must stay positive.
        if 1.0 - d / frame.signed_radius <= 0.0 {
            return Err(OffsetRefusal::RadiusUnderOffset { segment: 0 });
        }
        Ok(Self {
            start: self.start + frame.start_tangent.perp() * d,
            end: self.end + frame.end_tangent.perp() * d,
            bulge: self.bulge,
        })
    }

    // The arc with its start moved to `start` (the same bulge), for joints shared after an
    // offset.
    pub(super) fn with_start(&self, start: DVec2) -> Self {
        Self { start, ..*self }
    }

    // Writes the interior points of `count` equal steps (not the start, then the exact end).
    pub(super) fn append_flattened(&self, count: usize, out: &mut Vec<DVec2>) {
        let frame = self.frame();
        for index in 1..count {
            let turn = frame.sweep * (index as f64 / count as f64);
            out.push(point_at_turn(self.start, &frame, turn));
        }
        out.push(self.end);
    }
}

// The point after turning through `turn` from the start: start + R (sin(turn) t0 +
// (1 - cos(turn)) n0) with the signed radius R and left normal n0, written with the half angle
// (sin = 2 s c, 1 - cos = 2 s^2) so it stays exact for small turns and huge radii.
fn point_at_turn(start: DVec2, frame: &Frame, turn: f64) -> DVec2 {
    let (half_sin, half_cos) = (0.5 * turn).sin_cos();
    let along = 2.0 * half_sin * half_cos;
    let across = 2.0 * half_sin * half_sin;
    start
        + (frame.start_tangent * along + frame.start_tangent.perp() * across) * frame.signed_radius
}

impl GuidePath {
    /// Replaces the path with the simple curve of three clicks `a`, `c`, `b` (decision U1): the
    /// arc tangent to both legs `a -> c` and `c -> b` with tangent length
    /// `T = min(|ac|, |cb|)`, so radius `T / tan(deflection / 2)`, plus a straight lead on the
    /// longer leg. The path starts at `a` along `a -> c` and ends at `b` along `c -> b`, both
    /// ends bit for bit. Collinear legs give the line `a -> b`; legs that fold straight back are
    /// refused. Closed form, `O(1)`, no allocation once the path has capacity for three
    /// segments.
    pub fn set_fillet(&mut self, a: DVec2, c: DVec2, b: DVec2) -> Result<(), GuideError> {
        let (leg_in, leg_out) = (c - a, b - c);
        let (in_length, out_length) = (leg_in.length(), leg_out.length());
        if in_length == 0.0 || out_length == 0.0 {
            return Err(GuideError::Degenerate);
        }
        let (in_unit, out_unit) = (leg_in / in_length, leg_out / out_length);
        if in_unit.perp_dot(out_unit) == 0.0 {
            if in_unit.dot(out_unit) < 0.0 {
                return Err(GuideError::FoldBack);
            }
            self.clear();
            self.push(GuideSegment::Line(Line::new(a, b)));
            return Ok(());
        }
        // The tangent points at distance T from the corner; the shorter leg's is its click.
        let tangent_length = in_length.min(out_length);
        let arc_start = if in_length <= out_length {
            a
        } else {
            c - in_unit * tangent_length
        };
        let arc_end = if out_length <= in_length {
            b
        } else {
            c + out_unit * tangent_length
        };
        let arc = Arc::from_bulge(
            arc_start,
            arc_end,
            tangent_bulge(arc_start, in_unit, arc_end)?,
        )?;
        self.clear();
        if arc_start != a {
            self.push(GuideSegment::Line(Line::new(a, arc_start)));
        }
        self.push(GuideSegment::Arc(arc));
        if arc_end != b {
            self.push(GuideSegment::Line(Line::new(arc_end, b)));
        }
        Ok(())
    }
}
