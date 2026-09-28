// SPDX-License-Identifier: GPL-2.0-only

//! Road cursor constraints derived from the same persistent curb frames as zoning.

use super::{CELL_DEPTH, GridFrame, RoadCellAlignment};
use crate::simulation::network::graph::RegionGraph;
use glam::DVec2;
use godot::prelude::Vector3;

/// Immutable cursor view of road grid authority, shared with the road-tool graph snapshot.
#[derive(Clone)]
pub(crate) struct RoadGridSnap {
    alignments: imbl::HashMap<usize, RoadCellAlignment>,
    cell_m: f64,
}

impl RoadGridSnap {
    /// Captures an already shared alignment root and the configured cell size.
    pub(super) fn new(alignments: imbl::HashMap<usize, RoadCellAlignment>, cell_m: f64) -> Self {
        Self { alignments, cell_m }
    }

    /// Snaps a straight road's cursor to curb-compatible centreline coordinates.
    /// Uses the edge R-tree and prepared alignment lookup: O(log E + K log A), no allocations,
    /// for K nearby edges and A alignment entries (persistent hash-trie lookups). An active stroke keeps its start fixed and snaps only near a grid axis.
    /// Without a nearby straight road, the first stroke uses cardinal steps with room for two same-width corners.
    pub(crate) fn snap_road_cursor(
        &self,
        graph: &RegionGraph,
        cursor: DVec2,
        start: Option<DVec2>,
        road_width_m: f64,
    ) -> DVec2 {
        if let Some(start) = start
            && cursor.distance_squared(start) < 2.5 * 2.5
        {
            return start;
        }
        let cell_m = self.cell_m;
        let anchor = start.unwrap_or(cursor);
        let half_width = road_width_m * 0.5 + f64::from(crate::config::SIDEWALK_WIDTH);
        // Two facing six-row strips define the local block-building neighborhood.
        let reach = 2.0 * CELL_DEPTH as f64 * cell_m + 30.0;
        let mut best: Option<(f64, usize, GridFrame, DVec2, usize)> = None;
        graph.visit_edges_near_aabb(
            Vector3::new((anchor.x - reach) as f32, 0.0, (anchor.y - reach) as f32),
            Vector3::new((anchor.x + reach) as f32, 0.0, (anchor.y + reach) as f32),
            |id| {
                let Some(frames) = self
                    .alignments
                    .get(&id)
                    .and_then(|alignment| alignment.frames_for(graph, id))
                else {
                    return;
                };
                let edge = graph.edge(id);
                if edge.deleted {
                    return;
                }
                let a = edge.physical_geometry[0];
                let b = edge.physical_geometry[edge.physical_geometry.len() - 1];
                let a = DVec2::new(f64::from(a.x), f64::from(a.z));
                let b = DVec2::new(f64::from(b.x), f64::from(b.z));
                let delta = b - a;
                let closest =
                    a + delta * ((anchor - a).dot(delta) / delta.length_squared()).clamp(0.0, 1.0);
                let distance = anchor.distance_squared(closest);
                if distance > reach * reach
                    || best.is_some_and(|old| (distance, id) >= (old.0, old.1))
                {
                    return;
                }
                // Directed left is frame zero; use the curb facing the stroke/hover.
                let toward = (anchor + cursor) * 0.5 - a;
                let side = usize::from(delta.perp_dot(toward) < 0.0);
                let frame = frames[side];
                let (u, v) = frame.axes();
                let along = usize::from(delta.dot(v).abs() > delta.dot(u).abs());
                best = Some((distance, id, frame, closest, along));
            },
        );
        let Some((distance, _, frame, closest, along)) = best else {
            return start.map_or(cursor, |start| {
                let delta = cursor - start;
                let Some(axis) = nearby_axis(delta, cell_m) else {
                    return cursor;
                };
                let mut result = start;
                // Reserve both end corners so the first edge can also close a full block.
                let cells = ((delta[axis].abs() - 2.0 * half_width) / cell_m)
                    .round()
                    .max(0.0);
                result[axis] += delta[axis].signum() * (cells * cell_m + 2.0 * half_width);
                result
            });
        };
        let local = frame.local(cursor);
        let offset = half_width / frame.cell_m();
        if let Some(start) = start {
            let origin = frame.local(start);
            let delta = local - origin;
            let Some(axis) = nearby_axis(delta * frame.cell_m(), frame.cell_m()) else {
                return cursor;
            };
            let mut result = origin;
            // The terminating corner lies outside the last full cell, by half a road.
            let signed_offset = offset * delta[axis].signum();
            result[axis] = (local[axis] - signed_offset).round() + signed_offset;
            frame.world(result.x, result.y)
        } else {
            let mut result = local;
            for axis in 0..2 {
                // Either curb may bound a new corner. Choose the closest legal centreline.
                let positive = (local[axis] - offset).round() + offset;
                let negative = (local[axis] + offset).round() - offset;
                result[axis] = if (positive - local[axis]).abs() <= (negative - local[axis]).abs() {
                    positive
                } else {
                    negative
                };
            }
            // Starting a branch on an existing road must remain on its centreline even
            // when the selected road has a different width from the supplying road.
            if distance <= 25.0 {
                result[1 - along] = frame.local(closest)[1 - along];
            } else {
                let origin = frame.local(closest);
                let sign = (local[1 - along] - origin[1 - along]).signum();
                result[1 - along] = (local[1 - along] - sign * offset).round() + sign * offset;
            }
            frame.world(result.x, result.y)
        }
    }
}

// Capture within five degrees, bounded by half a cell of lateral movement. The distance
// cap prevents long strokes jumping far sideways; outside either bound drawing stays free.
fn nearby_axis(delta: DVec2, cell_m: f64) -> Option<usize> {
    const TAN_CAPTURE_ANGLE: f64 = 0.087488663525924; // tan(5 degrees)
    let axis = usize::from(delta.y.abs() > delta.x.abs());
    (delta[1 - axis].abs() <= (delta[axis].abs() * TAN_CAPTURE_ANGLE).min(cell_m * 0.5))
        .then_some(axis)
}
