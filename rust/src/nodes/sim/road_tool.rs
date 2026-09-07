// SPDX-License-Identifier: GPL-2.0-only

//! Road-tool query helpers shared by Godot-facing bridge methods.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::{RoadPreviewValidation, RoadSurfaceSystem};
use crate::simulation::network::types::EdgeClass;
use crate::simulation::water::WaterSystem;
use godot::prelude::{Vector2, Vector3};

#[cfg(test)]
mod ghost_tests;

pub(crate) const GHOST_GRID_SPACING_M: f32 = 80.0;
pub(crate) const GHOST_MAX_OFFSETS: usize = 3;
pub(crate) const GHOST_OUTWARD_EXTEND_M: f32 = 200.0;
pub(crate) const GHOST_TICK_INTERVAL_M: f32 = 20.0;
pub(crate) const GHOST_TICK_HALF_M: f32 = 1.5;
pub(crate) const GHOST_LINE_LIFT_M: f32 = 0.06;
pub(crate) const GHOST_TICK_LIFT_M: f32 = 0.07;
pub(crate) const GHOST_OFFSET_ALPHAS: [f32; GHOST_MAX_OFFSETS] = [0.30, 0.12, 0.04];
/// Stable preview rejection reason for a grounded roadbed that overlaps authored water.
pub(crate) const ROAD_WATER_REQUIRES_BRIDGE_REASON: &str = "water_requires_bridge";

/// Applies the authored-water placement rule to an otherwise validated road candidate.
pub(crate) fn validate_road_candidate_against_water(
    edge_class: EdgeClass,
    prepared_points: &[Vector3],
    fwd_lanes: u8,
    bkw_lanes: u8,
    water: &WaterSystem,
    validation: RoadPreviewValidation,
) -> RoadPreviewValidation {
    if !validation.is_valid || edge_class != EdgeClass::Standard || prepared_points.len() < 2 {
        return validation;
    }
    let half_width_m = RoadSurfaceSystem::preview_candidate_roadbed_half_width_m(
        prepared_points,
        edge_class,
        fwd_lanes,
        bkw_lanes,
    );
    if water.road_corridor_overlaps_visible_water(prepared_points, half_width_m) {
        return validation.with_invalid_reason(ROAD_WATER_REQUIRES_BRIDGE_REASON);
    }
    validation
}

/// Finds the nearest guide using the existing road-edge index, without snapshot-time rebuilding.
/// O(log E + S) for E indexed edges and S source segments in the bounded candidate neighborhood;
/// no candidate/offset buffers are allocated. Equal-distance ties use world-XZ order.
pub(crate) fn nearest_road_ghost_point(
    graph: &RegionGraph,
    pos: Vector2,
    max_dist_m: f32,
) -> Option<Vector2> {
    if !pos.is_finite() || !max_dist_m.is_finite() {
        return None;
    }
    // Offset guides never extend their source endpoints, and outward guides have fixed length.
    // Expand by that exact maximum reach; round outwards so f32 boundary rounding cannot cull a hit.
    let reach = ((GHOST_MAX_OFFSETS as f32 * GHOST_GRID_SPACING_M).max(GHOST_OUTWARD_EXTEND_M)
        + max_dist_m.abs())
    .next_up();
    let min = Vector3::new(
        (pos.x - reach).next_down(),
        0.0,
        (pos.y - reach).next_down(),
    );
    let max = Vector3::new((pos.x + reach).next_up(), 0.0, (pos.y + reach).next_up());
    let mut best_dist_sq = max_dist_m * max_dist_m;
    let mut best_point: Option<Vector2> = None;
    graph.visit_edges_near_aabb(min, max, |edge_idx| {
        let edge = graph.edge(edge_idx);
        if edge.deleted {
            return;
        }
        visit_ghost_snap_segments(&edge.physical_geometry, &mut |segment| {
            let point = segment.closest_point(pos);
            let dist_sq = (point - pos).length_squared();
            if dist_sq < best_dist_sq
                || (dist_sq == best_dist_sq
                    && best_point.is_some_and(|best| {
                        point
                            .x
                            .total_cmp(&best.x)
                            .then(point.y.total_cmp(&best.y))
                            .is_lt()
                    }))
            {
                best_dist_sq = dist_sq;
                best_point = Some(point);
            }
        });
    });
    best_point
}

fn visit_ghost_snap_segments(points: &[Vector3], visit: &mut impl FnMut(RoadGhostSnapSegment)) {
    if points.len() < 2 {
        return;
    }
    let end = points.len() - 1;
    for (anchor, neighbor) in [(points[0], points[1]), (points[end], points[end - 1])] {
        if let Some(tangent) = endpoint_tangent_xz(anchor, neighbor) {
            let a = Vector2::new(anchor.x, anchor.z);
            if let Some(segment) =
                RoadGhostSnapSegment::new(a, a + tangent * GHOST_OUTWARD_EXTEND_M)
            {
                visit(segment);
            }
        }
    }
    for index in 1..=GHOST_MAX_OFFSETS {
        let offset = index as f32 * GHOST_GRID_SPACING_M;
        visit_offset_snap_segments(points, offset, visit);
        visit_offset_snap_segments(points, -offset, visit);
    }
}

#[derive(Clone, Copy, Debug)]
struct RoadGhostSnapSegment {
    start: Vector2,
    end: Vector2,
}

impl RoadGhostSnapSegment {
    fn new(start: Vector2, end: Vector2) -> Option<Self> {
        if (end - start).length_squared() < 0.01 {
            return None;
        }
        Some(Self { start, end })
    }

    fn closest_point(&self, pos: Vector2) -> Vector2 {
        let segment = self.end - self.start;
        let along = ((pos - self.start).dot(segment) / segment.length_squared()).clamp(0.0, 1.0);
        self.start + segment * along
    }
}

/// Returns the outward endpoint tangent normalized in the horizontal road plane.
pub(crate) fn endpoint_tangent_xz(anchor: Vector3, neighbor: Vector3) -> Option<Vector2> {
    let tangent = Vector2::new(anchor.x - neighbor.x, anchor.z - neighbor.z);
    (tangent.length_squared() > 1e-6).then(|| tangent.normalized())
}

fn visit_offset_snap_segments(
    points: &[Vector3],
    offset_m: f32,
    visit: &mut impl FnMut(RoadGhostSnapSegment),
) {
    let mut offset_segments = points
        .windows(2)
        .filter_map(|segment| {
            let a = Vector2::new(segment[0].x, segment[0].z);
            let b = Vector2::new(segment[1].x, segment[1].z);
            let seg = b - a;
            if seg.length_squared() < 0.01 {
                return None;
            }
            let seg_norm = seg.normalized();
            let perp = Vector2::new(-seg_norm.y, seg_norm.x);
            let offset_a = a + perp * offset_m;
            let offset_b = b + perp * offset_m;
            if (offset_b - offset_a).dot(seg_norm) < 0.0 {
                return None;
            }
            Some((offset_a, offset_b))
        })
        .peekable();

    while let Some((a, b)) = offset_segments.next() {
        if offset_segments
            .peek()
            .is_some_and(|&(next_a, next_b)| segments_cross_2d(a, b, next_a, next_b))
        {
            offset_segments.next();
            continue;
        }
        if let Some(segment) = RoadGhostSnapSegment::new(a, b) {
            visit(segment);
        }
    }
}

fn segments_cross_2d(a1: Vector2, b1: Vector2, a2: Vector2, b2: Vector2) -> bool {
    let d1 = b1 - a1;
    let d2 = b2 - a2;
    let denom = d1.x * d2.y - d1.y * d2.x;
    if denom.abs() < 1e-6 {
        return false;
    }
    let t = ((a2.x - a1.x) * d2.y - (a2.y - a1.y) * d2.x) / denom;
    let u = ((a2.x - a1.x) * d1.y - (a2.y - a1.y) * d1.x) / denom;
    t > 0.0 && t < 1.0 && u > 0.0 && u < 1.0
}

#[cfg(test)]
mod tests {
    use super::{
        ROAD_WATER_REQUIRES_BRIDGE_REASON, endpoint_tangent_xz,
        validate_road_candidate_against_water,
    };
    use crate::simulation::network::surface::RoadSurfaceSystem;
    use crate::simulation::network::types::EdgeClass;
    use crate::simulation::terrain::TerrainSystem;
    use crate::simulation::water::WaterSystem;
    use godot::prelude::Vector3;

    #[test]
    fn endpoint_tangent_xz_ignores_height_delta() {
        let tangent =
            endpoint_tangent_xz(Vector3::new(10.0, 100.0, 0.0), Vector3::new(0.0, 0.0, 0.0))
                .expect("non-degenerate horizontal endpoint direction");

        assert!((tangent.x - 1.0).abs() < 0.001);
        assert!(tangent.y.abs() < 0.001);
        assert!((tangent.length() - 1.0).abs() < 0.001);
    }

    #[test]
    fn standard_road_candidate_crossing_water_requires_bridge() {
        let terrain = TerrainSystem::with_chunking(9, 9, 10.0, 4, 0.0);
        let water = test_center_lake();
        let surface = RoadSurfaceSystem::new(30.0);
        let preview = surface.compile_preview_surface(
            &[Vector3::new(-30.0, 0.0, 0.0), Vector3::new(30.0, 0.0, 0.0)],
            1,
            1,
            &terrain,
        );

        assert_eq!(preview.edge_class, EdgeClass::Standard);
        let validation = validate_road_candidate_against_water(
            preview.edge_class,
            &preview.prepared_points,
            1,
            1,
            &water,
            preview.validation,
        );

        assert!(!validation.is_valid);
        assert_eq!(validation.invalid_reason, ROAD_WATER_REQUIRES_BRIDGE_REASON);
    }

    #[test]
    fn bridge_candidate_crossing_water_remains_valid() {
        let terrain = TerrainSystem::with_chunking(9, 9, 10.0, 4, 0.0);
        let water = test_center_lake();
        let surface = RoadSurfaceSystem::new(30.0);
        let preview = surface.compile_preview_surface(
            &[
                Vector3::new(-30.0, 3.0, 0.0),
                Vector3::new(0.0, 3.0, 0.0),
                Vector3::new(30.0, 3.0, 0.0),
            ],
            1,
            1,
            &terrain,
        );

        assert_eq!(preview.edge_class, EdgeClass::Bridge);
        let validation = validate_road_candidate_against_water(
            preview.edge_class,
            &preview.prepared_points,
            1,
            1,
            &water,
            preview.validation,
        );

        assert!(validation.is_valid);
    }

    #[test]
    fn standard_road_candidate_beside_water_remains_valid() {
        let terrain = TerrainSystem::with_chunking(9, 9, 10.0, 4, 0.0);
        let water = test_center_lake();
        let surface = RoadSurfaceSystem::new(30.0);
        let preview = surface.compile_preview_surface(
            &[
                Vector3::new(-30.0, 0.0, 30.0),
                Vector3::new(30.0, 0.0, 30.0),
            ],
            1,
            1,
            &terrain,
        );

        assert_eq!(preview.edge_class, EdgeClass::Standard);
        let validation = validate_road_candidate_against_water(
            preview.edge_class,
            &preview.prepared_points,
            1,
            1,
            &water,
            preview.validation,
        );

        assert!(validation.is_valid);
    }

    fn test_center_lake() -> WaterSystem {
        let mut water = WaterSystem::with_chunking(9, 9, 10.0, 4);
        let mut baseline = vec![0.0; 9 * 9];
        for z in 3..=5 {
            for x in 3..=5 {
                baseline[x + z * 9] = 2.0;
            }
        }
        water
            .replace_baseline_depth_from_dense(&baseline)
            .expect("test lake dimensions should match");
        water
    }
}
