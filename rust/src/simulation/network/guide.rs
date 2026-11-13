// SPDX-License-Identifier: GPL-2.0-only

//! Road tool guide geometry (`ROAD-112` round A): the exact curves the road tool draws with
//! (straight lines, circular arcs and cubic Bezier spans), the closed-form constructions that
//! place them (the three-click fillet and the biarc S-curve), and the one flattening rule that
//! turns a guide into the polyline the road edit back end receives.
//!
//! Engine-free and `f64` throughout, in the horizontal XZ plane: a [`DVec2`] is `(x, z)` and
//! heights are not part of a guide. Positive angles and sweeps turn from +X towards +Z, so the
//! left normal `perp()` (`(-z, x)`) is the side a positive sweep turns towards. Constructions are
//! closed form, with no iteration counts or tolerances that tune a result; the only fixed-count
//! iteration is the Bezier arc-length inversion (`bezier.rs`). Same inputs give bitwise equal
//! outputs on one machine. Arc points use the platform `sin_cos` and `atan`, as every other
//! trigonometric call in the crate, so two operating systems' maths libraries may differ in the
//! last bit.
//!
//! The graph will own stored guides in `ROAD-113` (`docs/roads.md` "Road tools"), so the kernel
//! lives under `simulation/network` rather than beside the tool in `nodes/`. The arc's stored
//! form is its two ends plus the bulge, so reversal and splitting are exact and neighbours share
//! their joint point bit for bit.

mod arc;
mod bezier;
mod biarc;
mod flatten;
mod line;
mod offset;
#[cfg(test)]
mod tests;

pub use arc::Arc;
pub use bezier::CubicBezier;
pub use flatten::{FLATTEN_MAX_SAG_M, FLATTEN_MAX_STEP_M, FLATTEN_RADIUS_FLOOR_M, flatten_count};
pub use glam::DVec2;
pub use line::Line;
pub use offset::OffsetRefusal;

/// Why a construction produced no guide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuideError {
    /// Two control points that must differ coincide, or a tangent is zero, so a leg, chord or
    /// handle has no direction.
    Degenerate,
    /// The fillet's legs fold straight back (a deflection of a half turn): no arc is tangent to
    /// both.
    FoldBack,
    /// The end lies straight behind the start tangent: the arc through it would be a full turn.
    FullTurn,
    /// The biarc's end tangents are parallel and point away from each other along the chord, so
    /// the equal-tangent-length rule has no positive solution.
    NoBiarc,
}

/// One span of a guide.
///
/// A spiral (clothoid) transition can be added as a further variant later; nothing outside this
/// module matches on the variants except through the methods below.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GuideSegment {
    /// A straight line.
    Line(Line),
    /// An exact circular arc.
    Arc(Arc),
    /// A cubic Bezier span (the freeform tool's).
    Bezier(CubicBezier),
}

impl GuideSegment {
    /// The straight line or arc that leaves `start` along `tangent` and ends at `end` (the
    /// continuous mode's arc: a start tangent and an end point fix it). A line when `end` lies
    /// exactly ahead on the tangent.
    pub fn tangent_to(start: DVec2, tangent: DVec2, end: DVec2) -> Result<Self, GuideError> {
        let bulge = arc::tangent_bulge(start, tangent, end)?;
        Ok(if bulge == 0.0 {
            Self::Line(Line::new(start, end))
        } else {
            Self::Arc(Arc::from_bulge(start, end, bulge)?)
        })
    }

    /// The first point.
    pub fn start(&self) -> DVec2 {
        match self {
            Self::Line(line) => line.a,
            Self::Arc(arc) => arc.start(),
            Self::Bezier(curve) => curve.points()[0],
        }
    }

    /// The last point.
    pub fn end(&self) -> DVec2 {
        match self {
            Self::Line(line) => line.b,
            Self::Arc(arc) => arc.end(),
            Self::Bezier(curve) => curve.points()[3],
        }
    }

    /// Arc length in metres: closed form for a line and an arc, the documented quadrature for a
    /// Bezier (`CubicBezier::length`).
    pub fn length(&self) -> f64 {
        match self {
            Self::Line(line) => line.length(),
            Self::Arc(arc) => arc.length(),
            Self::Bezier(curve) => curve.length(),
        }
    }

    /// The point at arc length `s` from the start, `s` clamped to the segment.
    pub fn point_at(&self, s: f64) -> DVec2 {
        match self {
            Self::Line(line) => line.point_at(s),
            Self::Arc(arc) => arc.point_at(s),
            Self::Bezier(curve) => curve.point_at(s),
        }
    }

    /// The unit tangent at arc length `s`, in the direction of travel.
    pub fn tangent_at(&self, s: f64) -> DVec2 {
        match self {
            Self::Line(line) => line.direction(),
            Self::Arc(arc) => arc.tangent_at(s),
            Self::Bezier(curve) => curve.tangent_at(s),
        }
    }

    /// The unit tangent at the start.
    pub fn start_tangent(&self) -> DVec2 {
        match self {
            Self::Line(line) => line.direction(),
            Self::Arc(arc) => arc.start_tangent(),
            Self::Bezier(curve) => curve.start_tangent(),
        }
    }

    /// The unit tangent at the end, in the direction of travel.
    pub fn end_tangent(&self) -> DVec2 {
        match self {
            Self::Line(line) => line.direction(),
            Self::Arc(arc) => arc.end_tangent(),
            Self::Bezier(curve) => curve.end_tangent(),
        }
    }

    /// The smallest radius of curvature: infinite for a line, the radius for an arc, the
    /// sampled minimum for a Bezier (`CubicBezier::min_radius`).
    pub fn min_radius(&self) -> f64 {
        match self {
            Self::Line(_) => f64::INFINITY,
            Self::Arc(arc) => arc.radius(),
            Self::Bezier(curve) => curve.min_radius(),
        }
    }

    /// The two pieces either side of arc length `s`, or `None` unless `0 < s < length`. The
    /// first piece starts at this segment's start, the second ends at its end, and both share
    /// the split point bit for bit; an arc's pieces keep its circle, a Bezier's are its
    /// de Casteljau halves.
    pub fn split_at(&self, s: f64) -> Option<(Self, Self)> {
        if !(s > 0.0 && s < self.length()) {
            return None;
        }
        Some(match self {
            Self::Line(line) => {
                let (head, tail) = line.split_at(s);
                (Self::Line(head), Self::Line(tail))
            }
            Self::Arc(arc) => {
                let (head, tail) = arc.split_at(s);
                (Self::Arc(head), Self::Arc(tail))
            }
            Self::Bezier(curve) => {
                let (head, tail) = curve.split_at(s);
                (Self::Bezier(head), Self::Bezier(tail))
            }
        })
    }

    /// The same curve travelled the other way. Exact: reversing twice gives back the segment bit
    /// for bit.
    pub fn reversed(&self) -> Self {
        match self {
            Self::Line(line) => Self::Line(line.reversed()),
            Self::Arc(arc) => Self::Arc(arc.reversed()),
            Self::Bezier(curve) => Self::Bezier(curve.reversed()),
        }
    }

    // The same segment with its first point replaced by `start` (a joint shared bit for bit with
    // the previous segment's end after an offset; the two differ by rounding only).
    fn with_start(&self, start: DVec2) -> Self {
        match self {
            Self::Line(line) => Self::Line(Line::new(start, line.b)),
            Self::Arc(arc) => Self::Arc(arc.with_start(start)),
            Self::Bezier(curve) => Self::Bezier(curve.with_start(start)),
        }
    }
}

/// An ordered chain of guide segments, each starting where the previous one ends (bit for bit).
///
/// The segment list is a reusable buffer: [`GuidePath::clear`] and the `set_*` constructions keep
/// its capacity, so a session that rebuilds its draft every pointer move allocates nothing after
/// the first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GuidePath {
    segments: Vec<GuideSegment>,
}

impl GuidePath {
    /// An empty path.
    pub fn new() -> Self {
        Self::default()
    }

    /// A path of one segment.
    pub fn from_segment(segment: GuideSegment) -> Self {
        Self {
            segments: vec![segment],
        }
    }

    /// Removes every segment, keeping the capacity.
    pub fn clear(&mut self) {
        self.segments.clear();
    }

    /// Appends a segment. It must start exactly where the path ends (checked in debug builds).
    pub fn push(&mut self, segment: GuideSegment) {
        debug_assert!(
            self.segments
                .last()
                .is_none_or(|last| last.end() == segment.start()),
            "guide segment does not start at the path's end"
        );
        self.segments.push(segment);
    }

    /// Appends every segment of `other` (a join). `other` must start where this path ends.
    pub fn append(&mut self, other: &GuidePath) {
        for segment in &other.segments {
            self.push(*segment);
        }
    }

    /// The segments in order.
    pub fn segments(&self) -> &[GuideSegment] {
        &self.segments
    }

    /// Whether the path has no segment.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// The first point, `None` for an empty path.
    pub fn start(&self) -> Option<DVec2> {
        self.segments.first().map(GuideSegment::start)
    }

    /// The last point, `None` for an empty path.
    pub fn end(&self) -> Option<DVec2> {
        self.segments.last().map(GuideSegment::end)
    }

    /// The unit tangent at the start, `None` for an empty path.
    pub fn start_tangent(&self) -> Option<DVec2> {
        self.segments.first().map(GuideSegment::start_tangent)
    }

    /// The unit tangent at the end, `None` for an empty path.
    pub fn end_tangent(&self) -> Option<DVec2> {
        self.segments.last().map(GuideSegment::end_tangent)
    }

    /// Total arc length in metres.
    pub fn length(&self) -> f64 {
        self.segments.iter().map(GuideSegment::length).sum()
    }

    /// The smallest radius of curvature over every segment (infinite for straight paths and the
    /// empty path).
    pub fn min_radius(&self) -> f64 {
        self.segments
            .iter()
            .map(GuideSegment::min_radius)
            .fold(f64::INFINITY, f64::min)
    }

    /// The point at arc length `s` from the start, clamped to the path. `None` for an empty
    /// path. `O(segments)`.
    pub fn point_at(&self, s: f64) -> Option<DVec2> {
        let (segment, local) = self.locate(s)?;
        Some(segment.point_at(local))
    }

    /// The unit tangent at arc length `s`, clamped to the path. `None` for an empty path.
    pub fn tangent_at(&self, s: f64) -> Option<DVec2> {
        let (segment, local) = self.locate(s)?;
        Some(segment.tangent_at(local))
    }

    // The segment holding arc length `s` and the arc length within it. A station exactly on a
    // joint belongs to the earlier segment.
    fn locate(&self, s: f64) -> Option<(&GuideSegment, f64)> {
        let (last, head) = self.segments.split_last()?;
        let mut remaining = s.max(0.0);
        for segment in head {
            let length = segment.length();
            if remaining <= length {
                return Some((segment, remaining));
            }
            remaining -= length;
        }
        Some((last, remaining))
    }

    /// The two paths either side of arc length `s`, or `None` unless `0 < s < length`. A
    /// station on a joint splits between the segments; any other splits its segment
    /// (`GuideSegment::split_at`). Appending the second to the first gives a path through the
    /// same points, with the original's ends bit for bit.
    pub fn split_at(&self, s: f64) -> Option<(GuidePath, GuidePath)> {
        if !(s > 0.0 && s < self.length()) {
            return None;
        }
        let mut remaining = s;
        for (index, segment) in self.segments.iter().enumerate() {
            let length = segment.length();
            if remaining < length {
                let mut head = Self {
                    segments: self.segments[..index].to_vec(),
                };
                let mut tail = Self::new();
                match segment.split_at(remaining) {
                    Some((first, second)) => {
                        head.segments.push(first);
                        tail.segments.push(second);
                    }
                    // `remaining` is 0 (or rounds to it): the split falls on the joint before.
                    None => tail.segments.push(*segment),
                }
                tail.segments.extend_from_slice(&self.segments[index + 1..]);
                if head.is_empty() {
                    return None;
                }
                return Some((head, tail));
            }
            if remaining == length {
                return Some((
                    Self {
                        segments: self.segments[..=index].to_vec(),
                    },
                    Self {
                        segments: self.segments[index + 1..].to_vec(),
                    },
                ));
            }
            remaining -= length;
        }
        None
    }

    /// The path travelled the other way (segments in reverse order, each reversed). Exact:
    /// reversing twice gives back the path bit for bit. A one-way road's parallel copy is the
    /// reversed offset path.
    pub fn reversed(&self) -> GuidePath {
        Self {
            segments: self
                .segments
                .iter()
                .rev()
                .map(GuideSegment::reversed)
                .collect(),
        }
    }
}
