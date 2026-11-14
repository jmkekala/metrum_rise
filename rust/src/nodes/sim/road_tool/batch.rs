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
    let points = &prepared.points;
    let normal = |a: Vector3, b: Vector3| {
        let along = Vector3::new(b.x - a.x, 0.0, b.z - a.z);
        let length = along.length();
        (length > f32::EPSILON).then(|| Vector3::new(-along.z, 0.0, along.x) / length)
    };
    // The offset at point `i`: the mitre of its two segments' normals, its one segment's at an
    // end; capped at twice the half width so a sharp joint cannot throw the ribbon far.
    let offset = |i: usize| {
        let before = (i > 0).then(|| normal(points[i - 1], points[i])).flatten();
        let after = (i + 1 < points.len())
            .then(|| normal(points[i], points[i + 1]))
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
    };
    for i in 1..points.len() {
        if normal(points[i - 1], points[i]).is_none() {
            continue;
        }
        let (a, b) = (offset(i - 1), offset(i));
        let (left_a, right_a) = (points[i - 1] + a, points[i - 1] - a);
        let (left_b, right_b) = (points[i] + b, points[i] - b);
        out.extend([left_a, right_a, left_b, left_b, right_a, right_b]);
    }
}
