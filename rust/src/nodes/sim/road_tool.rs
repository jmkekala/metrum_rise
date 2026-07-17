//! Road-tool query helpers shared by Godot-facing bridge methods.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::{RoadPreviewValidation, RoadSurfaceSystem};
use crate::simulation::network::types::EdgeClass;
use crate::simulation::water::WaterSystem;
use godot::prelude::{Vector2, Vector3};
use rstar::{AABB, PointDistance, RTree, RTreeObject};

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

#[derive(Clone)]
pub(crate) struct RoadGhostSnapIndex {
    segments: RTree<RoadGhostSnapSegment>,
}

impl RoadGhostSnapIndex {
    pub(crate) fn from_graph(graph: &RegionGraph) -> Self {
        let mut segments = Vec::new();
        for edge in graph
            .edges()
            .iter()
            .filter(|edge| !edge.deleted && edge.physical_geometry.len() >= 2)
        {
            let geom = &edge.physical_geometry;
            let end_index = geom.len() - 1;
            if let Some(start_tangent) = endpoint_tangent_xz(geom[0], geom[1]) {
                append_outward_snap_segment(geom[0], start_tangent, &mut segments);
            }

            if let Some(end_tangent) = endpoint_tangent_xz(geom[end_index], geom[end_index - 1]) {
                append_outward_snap_segment(geom[end_index], end_tangent, &mut segments);
            }

            for offset_index in 1..=GHOST_MAX_OFFSETS {
                let offset = offset_index as f32 * GHOST_GRID_SPACING_M;
                append_offset_snap_segments(geom, offset, &mut segments);
                append_offset_snap_segments(geom, -offset, &mut segments);
            }
        }

        Self {
            segments: RTree::bulk_load(segments),
        }
    }

    pub(crate) fn nearest_point(&self, pos: Vector2, max_dist_m: f32) -> Option<Vector2> {
        let query = [pos.x, pos.y];
        let mut best_dist_sq = max_dist_m * max_dist_m;
        let mut best_point = None;
        for segment in self.segments.locate_within_distance(query, best_dist_sq) {
            let point = segment.closest_point(pos);
            let dist_sq = (point - pos).length_squared();
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best_point = Some(point);
            }
        }
        best_point
    }
}

impl Default for RoadGhostSnapIndex {
    fn default() -> Self {
        Self {
            segments: RTree::new(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct RoadGhostSnapSegment {
    start: Vector2,
    end: Vector2,
    envelope: AABB<[f32; 2]>,
}

impl RoadGhostSnapSegment {
    fn new(start: Vector2, end: Vector2) -> Option<Self> {
        if (end - start).length_squared() < 0.01 {
            return None;
        }
        let min_x = start.x.min(end.x);
        let max_x = start.x.max(end.x);
        let min_z = start.y.min(end.y);
        let max_z = start.y.max(end.y);
        Some(Self {
            start,
            end,
            envelope: AABB::from_corners([min_x, min_z], [max_x, max_z]),
        })
    }

    fn closest_point(&self, pos: Vector2) -> Vector2 {
        let segment = self.end - self.start;
        let along = ((pos - self.start).dot(segment) / segment.length_squared()).clamp(0.0, 1.0);
        self.start + segment * along
    }
}

impl RTreeObject for RoadGhostSnapSegment {
    type Envelope = AABB<[f32; 2]>;

    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

impl PointDistance for RoadGhostSnapSegment {
    fn distance_2(&self, point: &[f32; 2]) -> f32 {
        let pos = Vector2::new(point[0], point[1]);
        (self.closest_point(pos) - pos).length_squared()
    }
}

/// Returns the outward endpoint tangent normalized in the horizontal road plane.
pub(crate) fn endpoint_tangent_xz(anchor: Vector3, neighbor: Vector3) -> Option<Vector2> {
    let tangent = Vector2::new(anchor.x - neighbor.x, anchor.z - neighbor.z);
    (tangent.length_squared() > 1e-6).then(|| tangent.normalized())
}

fn append_outward_snap_segment(
    anchor: Vector3,
    tangent: Vector2,
    segments: &mut Vec<RoadGhostSnapSegment>,
) {
    let anchor_xz = Vector2::new(anchor.x, anchor.z);
    let end_xz = anchor_xz + tangent * GHOST_OUTWARD_EXTEND_M;
    if let Some(segment) = RoadGhostSnapSegment::new(anchor_xz, end_xz) {
        segments.push(segment);
    }
}

fn append_offset_snap_segments(
    points: &[Vector3],
    offset_m: f32,
    segments: &mut Vec<RoadGhostSnapSegment>,
) {
    if points.len() < 2 {
        return;
    }

    let mut offset_segments = Vec::with_capacity(points.len().saturating_sub(1));
    for segment in points.windows(2) {
        let a = Vector2::new(segment[0].x, segment[0].z);
        let b = Vector2::new(segment[1].x, segment[1].z);
        let seg = b - a;
        if seg.length_squared() < 0.01 {
            continue;
        }
        let seg_norm = seg.normalized();
        let perp = Vector2::new(-seg_norm.y, seg_norm.x);
        let offset_a = a + perp * offset_m;
        let offset_b = b + perp * offset_m;
        if (offset_b - offset_a).dot(seg_norm) < 0.0 {
            continue;
        }
        offset_segments.push((offset_a, offset_b));
    }

    let mut skip_next = false;
    for index in 0..offset_segments.len() {
        if skip_next {
            skip_next = false;
            continue;
        }
        let (a, b) = offset_segments[index];
        if let Some((next_a, next_b)) = offset_segments.get(index + 1).copied() {
            if segments_cross_2d(a, b, next_a, next_b) {
                skip_next = true;
                continue;
            }
        }
        if let Some(segment) = RoadGhostSnapSegment::new(a, b) {
            segments.push(segment);
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
