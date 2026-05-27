//! Earthwork skirt geometry, transition vectors, and top-surface intrusion checks.

use super::super::{
    NodeOverlayContour, NodeOverlayShapes, RoadSurfaceSystem, RoadSurfaceVisualPolygon,
    SAMPLE_EPSILON_M,
    backend::{self, RoadVec2, RoadVec3},
};
use super::{
    EARTHWORK_CUT_SLOPE_RATE, EARTHWORK_FILL_SLOPE_RATE, EARTHWORK_MARGIN_SAMPLE_STEP_M,
    EARTHWORK_MAX_MARGIN_M, EARTHWORK_MIN_MARGIN_M, EARTHWORK_RETAINING_WALL_SLOPE_THRESHOLD,
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceKind,
    RoadSurfaceEarthworkGeometryError, RoadSurfaceEarthworkRenderFace,
};
use crate::config;
use crate::simulation::terrain::TerrainSystem;
use i_overlay::core::overlay_rule::OverlayRule;

mod build;
mod faces;
mod intrusion;
mod orientation;
#[cfg(test)]
mod tests;
mod transitions;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::earthwork) fn earthwork_visual_polygon_from_road_points(
        points: Vec<RoadVec3>,
    ) -> Option<RoadSurfaceVisualPolygon> {
        Self::make_visual_polygon(points)
    }

    pub(in crate::simulation::network::surface::earthwork) fn earthwork_signed_polygon_area_xz(
        points: &[RoadVec3],
    ) -> f64 {
        if points.len() < 3 {
            return 0.0;
        }
        let origin = points[0];
        let mut double_area = 0.0;
        for index in 0..points.len() {
            let current = points[index] - origin;
            let next = points[(index + 1) % points.len()] - origin;
            double_area += current.x * next.z - next.x * current.z;
        }
        double_area * 0.5
    }

    pub(in crate::simulation::network::surface::earthwork) fn earthwork_point_segment_distance_squared_xz(
        point: RoadVec2,
        start: RoadVec3,
        end: RoadVec3,
    ) -> f64 {
        let start_xz = RoadVec2::new(start.x, start.z);
        let end_xz = RoadVec2::new(end.x, end.z);
        let segment = end_xz - start_xz;
        let length_squared = segment.length_squared();
        if length_squared <= f64::from(SAMPLE_EPSILON_M) {
            return point.distance_squared(start_xz);
        }
        let t = ((point - start_xz).dot(segment) / length_squared).clamp(0.0, 1.0);
        point.distance_squared(start_xz + segment * t)
    }

    pub(in crate::simulation::network::surface::earthwork) fn earthwork_polygon_contains_point_xz(
        points_world: &[RoadVec3],
        point: RoadVec2,
    ) -> bool {
        if points_world.len() < 3 {
            return false;
        }
        let mut inside = false;
        for index in 0..points_world.len() {
            let start = points_world[index];
            let end = points_world[(index + 1) % points_world.len()];
            if Self::earthwork_point_segment_distance_squared_xz(point, start, end) <= 0.0001 {
                return true;
            }
            if (start.z > point.y) != (end.z > point.y) {
                let edge_x_at_point_z =
                    (end.x - start.x) * (point.y - start.z) / (end.z - start.z) + start.x;
                if point.x < edge_x_at_point_z {
                    inside = !inside;
                }
            }
        }
        inside
    }
}
