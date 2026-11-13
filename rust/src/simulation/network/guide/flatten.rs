// SPDX-License-Identifier: GPL-2.0-only

//! The one flattening rule for every guide segment kind (`ROAD-112` plan 2.2).
//!
//! A segment of length `L` and smallest radius `R` is cut into
//! `n = max(1, ceil(L / min(4.0 m, sqrt(8 R 0.05 m))))` equal arc-length steps. A chord over an
//! arc of length `h` and radius `R` sags `R (1 - cos(h / 2R)) <= h^2 / 8R`, so the step bound
//! keeps the sag at most 5 cm, and a chord is never longer than its arc, so no step exceeds 4 m
//! (today's `ROAD_SURFACE_CURVE_STEP_M`). The rule applies to lines too (4 m steps). Radii under
//! `FLATTEN_RADIUS_FLOOR_M` count as the floor: no road type allows them and the validator
//! refuses them, and the floor bounds the point count of a degenerate Bezier (a cusp would
//! otherwise ask for unbounded points). The ends of every segment are written bit for bit, and
//! a joint is written once.

use super::{DVec2, GuidePath, GuideSegment};

/// The largest chord sag a flattened guide may have, in metres.
pub const FLATTEN_MAX_SAG_M: f64 = 0.05;
/// The longest step of a flattened guide, in metres.
pub const FLATTEN_MAX_STEP_M: f64 = 4.0;
/// The smallest radius the rule resolves, in metres (module docs).
pub const FLATTEN_RADIUS_FLOOR_M: f64 = 1.0;

/// The rule's step count for a segment of `length_m` whose smallest radius is `min_radius_m`
/// (infinite for a line): at least 1.
pub fn flatten_count(length_m: f64, min_radius_m: f64) -> usize {
    let radius = min_radius_m.max(FLATTEN_RADIUS_FLOOR_M);
    let step = FLATTEN_MAX_STEP_M.min((8.0 * radius * FLATTEN_MAX_SAG_M).sqrt());
    ((length_m / step).ceil() as usize).max(1)
}

impl GuideSegment {
    /// The rule's step count for this segment.
    pub fn flatten_count(&self) -> usize {
        flatten_count(self.length(), self.min_radius())
    }

    // Writes this segment's flattened points after its start (which the caller has written).
    pub(super) fn append_flattened(&self, out: &mut Vec<DVec2>) {
        match self {
            Self::Line(line) => {
                let count = flatten_count(line.length(), f64::INFINITY);
                for index in 1..count {
                    out.push(line.a + (line.b - line.a) * (index as f64 / count as f64));
                }
                out.push(line.b);
            }
            Self::Arc(arc) => arc.append_flattened(flatten_count(arc.length(), arc.radius()), out),
            Self::Bezier(curve) => {
                let table = curve.arc_length_table();
                let count = flatten_count(table.length(), curve.min_radius());
                curve.append_flattened(&table, count, out);
            }
        }
    }
}

impl GuidePath {
    /// Writes the flattened polyline into `out` (cleared first): the path's start, then every
    /// segment's equal steps by the rule, joints once, ends bit for bit. This is the polyline a
    /// stroke request sends to the road edit back end. `O(points)`; allocates nothing beyond
    /// growing `out`, so a reused buffer allocates nothing after warm-up. Same path, same points
    /// bit for bit.
    pub fn flatten_into(&self, out: &mut Vec<DVec2>) {
        out.clear();
        let Some(start) = self.start() else {
            return;
        };
        out.push(start);
        for segment in self.segments() {
            segment.append_flattened(out);
        }
    }
}
