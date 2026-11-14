// SPDX-License-Identifier: GPL-2.0-only

//! A draft as an ordered stroke list for the batch preview (`ROAD-112` round G1, plan 6.2): the
//! preview requests of a segmented draft's strokes ([`super::height::segment_draft`]) in drawing
//! order, and tier 1's ribbon ghost of a stroke's prepared points. Grid and parallel builders
//! (rounds J, K) add their own stroke lists here.

use godot::prelude::Vector3;

use super::height::{DraftSection, DraftStroke};
use crate::nodes::sim::core::RoadPreviewRequest;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::road_edit::PreparedRoadInput;

/// The preview requests of a draft's strokes, in drawing order (the batch's determinism rule),
/// on the view of `surface_generation`. `O(points)`.
pub(crate) fn draft_requests(
    strokes: &[DraftStroke],
    section: DraftSection,
    surface_generation: u64,
) -> Vec<RoadPreviewRequest> {
    strokes
        .iter()
        .map(|stroke| RoadPreviewRequest {
            surface_generation,
            points: stroke.points.clone(),
            fwd_lanes: section.fwd_lanes,
            bkw_lanes: section.bkw_lanes,
            snap_to_existing_roads: section.snap_to_existing_roads,
            profile: stroke.profile,
            median: section.median,
            parking: section.parking,
            attributes: section.attributes,
        })
        .collect()
}

/// Half the width a stroke's road covers: its lanes, median and parking, plus its sidewalks (or
/// shoulders), as the corridor checks measure it.
pub(crate) fn ghost_half_width_m(request: &RoadPreviewRequest) -> f32 {
    let lanes = |count: i32| count.clamp(0, i32::from(u8::MAX)) as u8;
    let (fwd, bkw) = (lanes(request.fwd_lanes), lanes(request.bkw_lanes));
    Edge::cross_section_width_m(fwd, bkw, request.median, request.parking) * 0.5
        + request.attributes.corridor_side_m(fwd == 0 && bkw == 0)
}

/// Appends tier 1's ghost of one prepared stroke to `out`: a ribbon of `half_width_m` either
/// side of its prepared points, as a triangle list (two triangles a segment), offset along the
/// mitred normal in the XZ plane at each joint (the plain normal at the ends). A segment
/// without horizontal length adds nothing. `O(points)`, no allocation beyond `out`.
pub(crate) fn push_ghost_ribbon(prepared: &PreparedRoadInput, half_width_m: f32, out: &mut Vec<Vector3>) {
    push_ribbon(&prepared.points, half_width_m, out);
}

// [`push_ghost_ribbon`] of `points`.
fn push_ribbon(points: &[Vector3], half_width_m: f32, out: &mut Vec<Vector3>) {
    for i in 1..points.len() {
        if plan_normal(points[i - 1], points[i]).is_none() {
            continue;
        }
        let (a, b) = (
            ribbon_offset(points, i - 1, half_width_m),
            ribbon_offset(points, i, half_width_m),
        );
        let (left_a, right_a) = (points[i - 1] + a, points[i - 1] - a);
        let (left_b, right_b) = (points[i] + b, points[i] - b);
        out.extend([left_a, right_a, left_b, left_b, right_a, right_b]);
    }
}

/// Appends the two edges of [`push_ghost_ribbon`]'s ribbon of `stroke` (a prepared stroke's
/// points) as two strips (round V2: the ghost
/// outlined in the road tool's line look): `starts` gets each strip's first index in `points`,
/// the left edge first. Points without horizontal length on both sides (a vertical stroke) are
/// left out, as the ribbon leaves out their segments. `O(points)`, no allocation beyond the
/// outputs.
pub(crate) fn push_ghost_edges(
    stroke: &[Vector3],
    half_width_m: f32,
    points: &mut Vec<Vector3>,
    starts: &mut Vec<u32>,
) {
    for side in [1.0, -1.0] {
        starts.push(points.len() as u32);
        for i in 0..stroke.len() {
            let offset = ribbon_offset(stroke, i, half_width_m);
            if offset != Vector3::ZERO {
                points.push(stroke[i] + offset * side);
            }
        }
    }
}

// The unit normal of segment `a`-`b` in the XZ plane (left of its direction), `None` without
// horizontal length.
pub(super) fn plan_normal(a: Vector3, b: Vector3) -> Option<Vector3> {
    let along = Vector3::new(b.x - a.x, 0.0, b.z - a.z);
    let length = along.length();
    (length > f32::EPSILON).then(|| Vector3::new(-along.z, 0.0, along.x) / length)
}

// The ribbon's offset at point `i`: the mitre of its two segments' normals, its one segment's at
// an end; capped at twice the half width so a sharp joint cannot throw the ribbon far.
pub(super) fn ribbon_offset(points: &[Vector3], i: usize, half_width_m: f32) -> Vector3 {
    let before = (i > 0).then(|| plan_normal(points[i - 1], points[i])).flatten();
    let after = (i + 1 < points.len())
        .then(|| plan_normal(points[i], points[i + 1]))
        .flatten();
    let mitre = match (before, after) {
        (Some(a), Some(b)) => {
            let sum = a + b;
            let cos = sum.length() * 0.5;
            if cos > 0.5 { sum.normalized() / cos } else { b * 2.0 }
        }
        (Some(n), None) | (None, Some(n)) => n,
        (None, None) => Vector3::ZERO,
    };
    mitre * half_width_m
}

#[cfg(test)]
mod tests {
    use super::*;

    // The ghost's edges are the ribbon's two sides, point for point (round V2).
    #[test]
    fn ghost_edges_are_the_ribbons_sides() {
        let stroke = [
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(10.0, 1.0, 0.0),
            Vector3::new(20.0, 2.0, 10.0),
        ];
        let (mut ribbon, mut edges, mut starts) = (Vec::new(), Vec::new(), Vec::new());
        push_ribbon(&stroke, 4.0, &mut ribbon);
        push_ghost_edges(&stroke, 4.0, &mut edges, &mut starts);
        assert_eq!(starts, vec![0, 3]);
        // Left side: each segment's left_a, left_b; right side: right_a, right_b.
        let (left, right) = edges.split_at(3);
        for segment in 0..2 {
            let triangles = &ribbon[segment * 6..segment * 6 + 6];
            assert_eq!(triangles[0], left[segment]);
            assert_eq!(triangles[2], left[segment + 1]);
            assert_eq!(triangles[1], right[segment]);
            assert_eq!(triangles[5], right[segment + 1]);
        }
        // The first side is the left of travel (+z for a stroke along +x) at the stroke's
        // half width.
        assert_eq!(left[0], Vector3::new(0.0, 1.0, 4.0));
    }
}
