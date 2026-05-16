//! Geometry-backend adapter boundary for road-surface compilation.

use super::NodeOverlayPoint;
use cavalier_contours::polyline::{PlineCreation, PlineSource, PlineVertex, Polyline};
use glam::{DVec2, DVec3};
use godot::prelude::{Vector2, Vector3};

pub(crate) type RoadVec2 = DVec2;
pub(crate) type RoadVec3 = DVec3;
pub(crate) type RoadPolyline = Polyline<f64>;
pub(crate) type RoadPolylineVertex = PlineVertex<f64>;

const LINE_SEGMENT_BULGE: f64 = 0.0;
pub(crate) const ROAD_OVERLAY_COORDINATE_SCALE: f64 = 1_000_000.0;

pub(crate) fn godot_vec2_to_road(point: Vector2) -> RoadVec2 {
    RoadVec2::new(f64::from(point.x), f64::from(point.y))
}

pub(crate) fn godot_vec3_to_road(point: Vector3) -> RoadVec3 {
    RoadVec3::new(f64::from(point.x), f64::from(point.y), f64::from(point.z))
}

pub(crate) fn godot_vec3_xz_to_road(point: Vector3) -> RoadVec2 {
    RoadVec2::new(f64::from(point.x), f64::from(point.z))
}

pub(crate) fn road_xz_with_height_to_godot(point_xz: RoadVec2, height_m: f64) -> Vector3 {
    Vector3::new(point_xz.x as f32, height_m as f32, point_xz.y as f32)
}

pub(crate) fn overlay_point_to_road(point: NodeOverlayPoint) -> RoadVec2 {
    quantize_road_vec2_to_overlay_grid(RoadVec2::new(point[0], point[1]))
}

pub(crate) fn quantize_road_vec2_to_overlay_grid(point: RoadVec2) -> RoadVec2 {
    RoadVec2::new(
        (point.x * ROAD_OVERLAY_COORDINATE_SCALE).round() / ROAD_OVERLAY_COORDINATE_SCALE,
        (point.y * ROAD_OVERLAY_COORDINATE_SCALE).round() / ROAD_OVERLAY_COORDINATE_SCALE,
    )
}

pub(crate) fn quantize_road_vec3_xz_to_overlay_grid(point: RoadVec3) -> RoadVec3 {
    let point_xz = quantize_road_vec2_to_overlay_grid(road_vec3_xz(point));
    RoadVec3::new(point_xz.x, point.y, point_xz.y)
}

pub(crate) fn quantize_road_vec3_path_xz_to_overlay_grid(points: &mut [RoadVec3]) {
    for point in points {
        *point = quantize_road_vec3_xz_to_overlay_grid(*point);
    }
}

pub(crate) fn road_vec3_xz(point: RoadVec3) -> RoadVec2 {
    RoadVec2::new(point.x, point.z)
}

pub(crate) fn road_vec2_to_overlay_point(point: RoadVec2) -> NodeOverlayPoint {
    let point = quantize_road_vec2_to_overlay_grid(point);
    [point.x, point.y]
}

pub(crate) fn road_vec2_to_polyline_vertex(point: RoadVec2) -> RoadPolylineVertex {
    RoadPolylineVertex::new(point.x, point.y, LINE_SEGMENT_BULGE)
}

pub(crate) fn polyline_vertex_to_road(vertex: RoadPolylineVertex) -> RoadVec2 {
    RoadVec2::new(vertex.x, vertex.y)
}

pub(crate) fn road_points_to_polyline(
    points: impl IntoIterator<Item = RoadVec2>,
    is_closed: bool,
) -> RoadPolyline {
    RoadPolyline::from_iter(
        points.into_iter().map(road_vec2_to_polyline_vertex),
        is_closed,
    )
}

pub(crate) fn polyline_to_road_points(polyline: &RoadPolyline) -> Vec<RoadVec2> {
    polyline
        .iter_vertexes()
        .map(polyline_vertex_to_road)
        .collect()
}
