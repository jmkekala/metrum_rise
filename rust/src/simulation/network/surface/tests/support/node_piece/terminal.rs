// SPDX-License-Identifier: GPL-2.0-only

//! Terminal ownership assertion helpers.

use super::*;

pub(in crate::simulation::network::surface::tests) fn assert_terminal_mouth_handoff_surface_is_owned(
    piece: &RoadSurfaceVisualNodePiece,
    mouth: &super::IncidentMouthProfile,
    material: RoadSurfaceBandKind,
    start_boundary_index: usize,
    end_boundary_index: usize,
    label: &str,
) {
    let start = mouth.boundary_points_world[start_boundary_index];
    let end = mouth.boundary_points_world[end_boundary_index];
    let inward = mouth.inward_direction_xz.normalize();
    let sample = RoadVec2::new(
        (start.x + end.x) * 0.5 - inward.x * 0.1,
        (start.z + end.z) * 0.5 - inward.y * 0.1,
    );
    let polygons = match material {
        RoadSurfaceBandKind::CurbOrShoulder => &piece.curb_surface_polygons,
        RoadSurfaceBandKind::Sidewalk => &piece.sidewalk_surface_polygons,
        RoadSurfaceBandKind::Carriageway => &piece.road_surface_polygons,
        _ => &piece.sidewalk_surface_polygons,
    };
    assert!(
        point_inside_visual_polygons(polygons, sample),
        "terminal handoff surface must be owned by {material:?}; label={label} sample={sample:?}"
    );
}

pub(in crate::simulation::network::surface::tests) fn assert_terminal_band_interval_grid_is_owned(
    piece: &RoadSurfaceVisualNodePiece,
    endpoint: &super::IncidentMouthProfile,
    mouth: &super::IncidentMouthProfile,
    material: RoadSurfaceBandKind,
    start_boundary_index: usize,
    end_boundary_index: usize,
    label: &str,
) {
    let polygons = match material {
        RoadSurfaceBandKind::CurbOrShoulder => &piece.curb_surface_polygons,
        RoadSurfaceBandKind::Sidewalk => &piece.sidewalk_surface_polygons,
        RoadSurfaceBandKind::Carriageway => &piece.road_surface_polygons,
        _ => &piece.sidewalk_surface_polygons,
    };
    for longitudinal_t in [0.1_f64, 0.5, 0.9, 0.98] {
        for lateral_t in [0.05_f64, 0.5, 0.95] {
            let endpoint_start = endpoint.boundary_points_world[start_boundary_index];
            let endpoint_end = endpoint.boundary_points_world[end_boundary_index];
            let mouth_start = mouth.boundary_points_world[start_boundary_index];
            let mouth_end = mouth.boundary_points_world[end_boundary_index];
            let endpoint_sample = endpoint_start.lerp(endpoint_end, lateral_t);
            let mouth_sample = mouth_start.lerp(mouth_end, lateral_t);
            let sample_world = endpoint_sample.lerp(mouth_sample, longitudinal_t);
            let sample = RoadVec2::new(sample_world.x, sample_world.z);
            assert!(
                point_inside_visual_polygons(polygons, sample),
                "terminal band interval must be owned by {material:?}; label={label} longitudinal_t={longitudinal_t} lateral_t={lateral_t} sample={sample:?}"
            );
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_terminal_band_interval_grid_is_not_duplicated_by_span(
    span_piece: &super::RoadSurfaceVisualSpanPiece,
    endpoint: &super::IncidentMouthProfile,
    mouth: &super::IncidentMouthProfile,
    start_boundary_index: usize,
    end_boundary_index: usize,
    label: &str,
) {
    for longitudinal_t in [0.1_f64, 0.5, 0.9, 0.98] {
        for lateral_t in [0.05_f64, 0.5, 0.95] {
            let endpoint_start = endpoint.boundary_points_world[start_boundary_index];
            let endpoint_end = endpoint.boundary_points_world[end_boundary_index];
            let mouth_start = mouth.boundary_points_world[start_boundary_index];
            let mouth_end = mouth.boundary_points_world[end_boundary_index];
            let endpoint_sample = endpoint_start.lerp(endpoint_end, lateral_t);
            let mouth_sample = mouth_start.lerp(mouth_end, lateral_t);
            let sample_world = endpoint_sample.lerp(mouth_sample, longitudinal_t);
            let sample = RoadVec2::new(sample_world.x, sample_world.z);
            let duplicated =
                point_inside_visual_polygons(&span_piece.road_surface_polygons, sample)
                    || point_inside_visual_polygons(&span_piece.curb_surface_polygons, sample)
                    || point_inside_visual_polygons(&span_piece.sidewalk_surface_polygons, sample);
            assert!(
                !duplicated,
                "terminal band interval must not be duplicated by span top surfaces; label={label} longitudinal_t={longitudinal_t} lateral_t={lateral_t} sample={sample:?}"
            );
        }
    }
}
