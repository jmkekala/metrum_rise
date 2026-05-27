//! Overlay and quantized geometry helpers for road-surface tests.

use super::*;
use crate::simulation::network::surface::backend::{RoadVec2, RoadVec3};

pub(in crate::simulation::network::surface::tests) trait TestPointXz {
    fn to_road_xz(self) -> RoadVec2;
}

impl TestPointXz for Vector2 {
    fn to_road_xz(self) -> RoadVec2 {
        RoadVec2::new(f64::from(self.x), f64::from(self.y))
    }
}

impl TestPointXz for RoadVec2 {
    fn to_road_xz(self) -> RoadVec2 {
        self
    }
}

pub(in crate::simulation::network::surface::tests) fn triangle_centroid_xz(
    triangle: [RoadVec3; 3],
) -> Vector2 {
    Vector2::new(
        ((triangle[0].x + triangle[1].x + triangle[2].x) / 3.0) as f32,
        ((triangle[0].z + triangle[1].z + triangle[2].z) / 3.0) as f32,
    )
}

pub(in crate::simulation::network::surface::tests) fn point_inside_visual_polygons(
    polygons: &[RoadSurfaceVisualPolygon],
    point: impl TestPointXz,
) -> bool {
    let point = point.to_road_xz();
    polygons.iter().any(|polygon| {
        if polygon.triangles_world.is_empty() {
            RoadSurfaceSystem::polygon_contains_point_xz(&polygon.points_world, point)
        } else {
            polygon.triangles_world.iter().any(|&triangle| {
                RoadSurfaceSystem::triangle_barycentric_weights_xz(triangle, point).is_some()
            })
        }
    })
}

pub(in crate::simulation::network::surface::tests) fn visual_polygon_boundary_contains_xz(
    polygons: &[RoadSurfaceVisualPolygon],
    point: impl TestPointXz,
) -> bool {
    let point = point.to_road_xz();
    polygons
        .iter()
        .flat_map(|polygon| polygon.points_world.iter())
        .any(|candidate| {
            RoadVec2::new(candidate.x - point.x, candidate.z - point.y).length()
                <= f64::from(SAMPLE_EPSILON_M) * 2.0
        })
}

pub(in crate::simulation::network::surface::tests) fn overlay_contours_from_polygons(
    polygons: &[RoadSurfaceVisualPolygon],
) -> Vec<super::NodeOverlayContour> {
    polygons
        .iter()
        .filter_map(|polygon| {
            let contour = overlay_contour_from_world_points(&polygon.points_world);
            (contour.len() >= 3).then_some(contour)
        })
        .collect()
}

pub(in crate::simulation::network::surface::tests) fn overlay_contour_from_world_points(
    points: &[RoadVec3],
) -> super::NodeOverlayContour {
    let mut contour = Vec::with_capacity(points.len());
    for point in points {
        let overlay_point =
            super::backend::road_vec2_to_overlay_point(super::backend::road_vec3_xz(*point));
        if contour.last().is_none_or(|last| *last != overlay_point) {
            contour.push(overlay_point);
        }
    }
    if contour.len() >= 2 && contour.first() == contour.last() {
        contour.pop();
    }
    contour
}

pub(in crate::simulation::network::surface::tests) fn overlay_contours_from_top_polygons<'a>(
    polygons: impl IntoIterator<Item = &'a RoadSurfaceVisualPolygon>,
) -> Vec<super::NodeOverlayContour> {
    let mut contours = Vec::new();
    for polygon in polygons {
        if polygon.triangles_world.is_empty() {
            let contour = overlay_contour_from_world_points(&polygon.points_world);
            if contour.len() >= 3 {
                contours.push(contour);
            }
            continue;
        }
        for triangle in &polygon.triangles_world {
            let contour = overlay_contour_from_world_points(triangle);
            if contour.len() >= 3 {
                contours.push(contour);
            }
        }
    }
    contours
}

pub(in crate::simulation::network::surface::tests) fn overlay_area_m2(
    shapes: &super::NodeOverlayShapes,
) -> f32 {
    shapes
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum()
}

pub(in crate::simulation::network::surface::tests) fn node_top_coverage_details_m2(
    piece: &RoadSurfaceVisualNodePiece,
) -> (
    f32,
    f32,
    f32,
    super::NodeOverlayShapes,
    super::NodeOverlayShapes,
) {
    let footprint_contours = overlay_contours_from_polygons(&piece.outer_boundary_loops);
    let footprint_shapes = RoadSurfaceSystem::overlay_union_contours(&footprint_contours)
        .expect("node footprint overlay union should succeed");
    let top_contours = overlay_contours_from_top_polygons(
        piece
            .road_surface_polygons
            .iter()
            .chain(piece.curb_surface_polygons.iter())
            .chain(piece.sidewalk_surface_polygons.iter()),
    );
    let top_shapes = RoadSurfaceSystem::overlay_union_contours(&top_contours)
        .expect("node top overlay union should succeed");
    let missing_shapes = RoadSurfaceSystem::overlay_binary_shapes(
        &footprint_shapes,
        &top_shapes,
        OverlayRule::Difference,
    )
    .expect("node footprint/top difference should succeed");
    let extra_shapes = RoadSurfaceSystem::overlay_binary_shapes(
        &top_shapes,
        &footprint_shapes,
        OverlayRule::Difference,
    )
    .expect("node top/footprint difference should succeed");
    let budget_m2 = RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&footprint_shapes)
        .max(RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(
            &top_shapes,
        ));
    (
        overlay_area_m2(&missing_shapes),
        overlay_area_m2(&extra_shapes),
        budget_m2,
        missing_shapes,
        extra_shapes,
    )
}

pub(in crate::simulation::network::surface::tests) fn test_xz_key_lies_on_segment(
    point: (i64, i64),
    start: (i64, i64),
    end: (i64, i64),
) -> bool {
    if point == start || point == end {
        return true;
    }
    if start == end {
        return false;
    }
    let dx = i128::from(end.0 - start.0);
    let dz = i128::from(end.1 - start.1);
    let px = i128::from(point.0 - start.0);
    let pz = i128::from(point.1 - start.1);
    if px * dz - pz * dx != 0 {
        return false;
    }
    let dot = px * dx + pz * dz;
    let len_squared = dx * dx + dz * dz;
    dot >= 0 && dot <= len_squared
}

pub(in crate::simulation::network::surface::tests) fn test_xz_key(point: RoadVec3) -> (i64, i64) {
    let point = super::backend::road_vec2_to_overlay_point(super::backend::road_vec3_xz(point));
    (
        (point[0] * super::backend::ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point[1] * super::backend::ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
}

pub(in crate::simulation::network::surface::tests) fn triangle_overlap_area_m2(
    a: [RoadVec3; 3],
    b: [RoadVec3; 3],
) -> f32 {
    RoadSurfaceSystem::overlay_binary_shapes(
        &triangle_overlay_shapes(a),
        &triangle_overlay_shapes(b),
        OverlayRule::Intersect,
    )
    .unwrap_or_default()
    .iter()
    .map(RoadSurfaceSystem::overlay_shape_area_m2)
    .sum()
}

pub(in crate::simulation::network::surface::tests) fn triangle_overlap_numeric_budget_m2(
    a: [RoadVec3; 3],
    b: [RoadVec3; 3],
) -> f32 {
    RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&triangle_overlay_shapes(a)).max(
        RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&triangle_overlay_shapes(b)),
    )
}

pub(in crate::simulation::network::surface::tests) fn triangle_overlay_shapes(
    triangle: [RoadVec3; 3],
) -> super::NodeOverlayShapes {
    let mut contour = triangle
        .iter()
        .map(|point| [point.x, point.z])
        .collect::<Vec<_>>();
    let area = (contour[1][0] - contour[0][0]) * (contour[2][1] - contour[0][1])
        - (contour[1][1] - contour[0][1]) * (contour[2][0] - contour[0][0]);
    if area < 0.0 {
        contour.swap(1, 2);
    }
    vec![vec![contour]]
}
