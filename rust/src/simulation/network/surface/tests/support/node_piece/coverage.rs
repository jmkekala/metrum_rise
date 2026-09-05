// SPDX-License-Identifier: GPL-2.0-only

//! Node-piece top-surface coverage assertions.

use super::*;
use crate::simulation::network::surface::NODE_OVERLAY_NUMERIC_DUST_WIDTH_M;

pub(in crate::simulation::network::surface::tests) fn assert_node_top_covers_footprint(
    piece: &RoadSurfaceVisualNodePiece,
) {
    let (missing_area_m2, extra_area_m2, budget_m2, missing_shapes, extra_shapes) =
        node_top_coverage_details_m2(piece);
    assert!(
        missing_area_m2 <= budget_m2 && extra_area_m2 <= budget_m2,
        "node top surfaces must exactly cover the canonical footprint; kind={:?} missing_area={missing_area_m2:.6} extra_area={extra_area_m2:.6} budget={budget_m2:.6} missing_shapes={missing_shapes:?} extra_shapes={extra_shapes:?}",
        piece.kind
    );
}

pub(in crate::simulation::network::surface::tests) fn assert_no_missing_top_footprint_shapes_touch_boundaries(
    piece: &RoadSurfaceVisualNodePiece,
) {
    let (missing_area_m2, _extra_area_m2, budget_m2, missing_shapes, _extra_shapes) =
        node_top_coverage_details_m2(piece);
    let top_edges = test_owned_top_boundary_edges(piece);
    let touching_missing_shapes = missing_shapes
        .iter()
        .enumerate()
        .filter_map(|(index, shape)| {
            missing_footprint_shape_is_suspicious(shape, &top_edges).then_some((
                index,
                overlay_shape_thin_width_m(shape),
                shape,
            ))
        })
        .collect::<Vec<_>>();
    assert!(
        touching_missing_shapes.is_empty(),
        "node top-surface footprint holes must not leave visible road/curb/sidewalk boundary gaps; kind={:?} missing_area={missing_area_m2:.8} budget={budget_m2:.8} touching_missing_shapes={touching_missing_shapes:?}",
        piece.kind
    );
}

fn missing_footprint_shape_is_suspicious(
    shape: &NodeOverlayShape,
    top_edges: &[TestTopBoundaryEdge],
) -> bool {
    overlay_shape_touches_test_top_boundary(shape, top_edges)
        && RoadSurfaceSystem::overlay_shape_area_m2(shape) > f32::EPSILON
        && !missing_footprint_shape_is_canonical_numeric_dust(shape)
}

fn missing_footprint_shape_is_canonical_numeric_dust(shape: &NodeOverlayShape) -> bool {
    overlay_shape_thin_width_m(shape) <= NODE_OVERLAY_NUMERIC_DUST_WIDTH_M
        && RoadSurfaceSystem::overlay_shape_area_m2(shape)
            <= overlay_shape_numeric_area_budget_m2(shape)
}

fn overlay_shape_numeric_area_budget_m2(shape: &NodeOverlayShape) -> f32 {
    let perimeter_m = shape
        .iter()
        .map(|contour| RoadSurfaceSystem::overlay_contour_perimeter_m(contour))
        .sum::<f32>();
    let vertex_count = shape.iter().map(Vec::len).sum::<usize>();
    RoadSurfaceSystem::overlay_numeric_area_budget_m2(perimeter_m, vertex_count)
}

fn overlay_shape_thin_width_m(shape: &NodeOverlayShape) -> f32 {
    let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(shape);
    let perimeter_m = shape
        .iter()
        .map(|contour| RoadSurfaceSystem::overlay_contour_perimeter_m(contour))
        .sum::<f32>();
    if perimeter_m <= f32::EPSILON {
        return 0.0;
    }
    2.0 * area_m2 / perimeter_m
}

fn overlay_shape_touches_test_top_boundary(
    shape: &NodeOverlayShape,
    top_edges: &[TestTopBoundaryEdge],
) -> bool {
    shape
        .iter()
        .flat_map(|contour| contour.iter().copied())
        .any(|point| {
            top_edges.iter().copied().any(|edge| {
                let point = TestRenderVertexKey::from_point(RoadVec3::new(
                    point[0],
                    edge.avg_y_m,
                    point[1],
                ));
                let start = TestRenderVertexKey::from_point(edge.start);
                let end = TestRenderVertexKey::from_point(edge.end);
                test_boundary_segment_parameter_xz(point, start, end).is_some_and(
                    |(numerator, denominator)| {
                        let tolerance = (denominator / 1_000_000).max(1);
                        numerator >= -tolerance && numerator <= denominator + tolerance
                    },
                )
            })
        })
}

pub(in crate::simulation::network::surface::tests) fn assert_material_triangles_do_not_overlap(
    piece: &RoadSurfaceVisualNodePiece,
) {
    for non_road_region in piece
        .owned_regions
        .iter()
        .filter(|region| region.kind != RoadSurfaceBandKind::Carriageway)
    {
        for &non_road_triangle in &non_road_region.polygon.triangles_world {
            for road_region in piece
                .owned_regions
                .iter()
                .filter(|region| region.kind == RoadSurfaceBandKind::Carriageway)
            {
                for &road_triangle in &road_region.polygon.triangles_world {
                    let overlap_area_m2 =
                        triangle_overlap_area_m2(non_road_triangle, road_triangle);
                    let area_budget_m2 =
                        triangle_overlap_numeric_budget_m2(non_road_triangle, road_triangle);
                    assert!(
                        overlap_area_m2 <= area_budget_m2,
                        "node material triangles must not overlap beyond numeric dust; kind={:?} overlap_area={overlap_area_m2:.8} budget={area_budget_m2:.8} non_road_triangle={non_road_triangle:?} road_triangle={road_triangle:?}",
                        non_road_region.kind
                    );
                }
            }
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_top_surface_triangles_face_up(
    piece: &RoadSurfaceVisualNodePiece,
) {
    for triangle in piece
        .road_surface_polygons
        .iter()
        .chain(piece.curb_surface_polygons.iter())
        .chain(piece.sidewalk_surface_polygons.iter())
        .flat_map(|polygon| polygon.triangles_world.iter().copied())
    {
        let double_area_xz = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
            - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
        assert!(
            double_area_xz >= -0.001,
            "node top-surface triangles must remain front-facing from above; kind={:?} triangle={triangle:?} double_area_xz={double_area_xz:.6}",
            piece.kind
        );
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_node_piece_uses_band_owned_regions(
    piece: &RoadSurfaceVisualNodePiece,
) {
    assert!(
        !piece.owned_regions.is_empty(),
        "node piece must keep explicit band-owned regions as its source of rendered top surfaces"
    );
    let carriageway_count = piece
        .owned_regions
        .iter()
        .filter(|region| region.kind == RoadSurfaceBandKind::Carriageway)
        .count();
    let non_road_count = piece
        .owned_regions
        .iter()
        .filter(|region| {
            region.kind != RoadSurfaceBandKind::Carriageway
                && region.kind != RoadSurfaceBandKind::CurbOrShoulder
        })
        .count();
    let curb_count = piece
        .owned_regions
        .iter()
        .filter(|region| region.kind == RoadSurfaceBandKind::CurbOrShoulder)
        .count();
    assert_eq!(
        carriageway_count,
        piece.road_surface_polygons.len(),
        "asphalt polygons must be derived from carriageway-owned node regions"
    );
    assert_eq!(
        curb_count,
        piece.curb_surface_polygons.len(),
        "curb polygons must be derived from curb/shoulder-owned node regions"
    );
    assert_eq!(
        non_road_count,
        piece.sidewalk_surface_polygons.len(),
        "sidewalk polygons must be derived from sidewalk-owned node regions"
    );
    let degenerate_owned_regions = piece
        .owned_regions
        .iter()
        .enumerate()
        .filter(|(_, region)| !RoadSurfaceSystem::polygon_has_area_xz(&region.polygon.points_world))
        .map(|(index, region)| {
            (
                index,
                region.kind,
                region.owner_index,
                RoadSurfaceSystem::signed_polygon_area_xz(&region.polygon.points_world),
                region.polygon.points_world.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert!(
        degenerate_owned_regions.is_empty(),
        "owned node regions must be non-degenerate before triangulation; degenerate_owned_regions={degenerate_owned_regions:?}"
    );
    assert_node_top_surface_sources_have_grade_authority(piece);
    assert_node_terrain_clip_sources_have_footprint_provenance(piece);
}

pub(in crate::simulation::network::surface::tests) fn assert_node_piece_has_curb_and_sidewalk_owners(
    piece: &RoadSurfaceVisualNodePiece,
) {
    assert!(
        piece
            .owned_regions
            .iter()
            .any(|region| region.kind == RoadSurfaceBandKind::CurbOrShoulder),
        "node non-road hardcut must expose explicit curb/shoulder owners"
    );
    assert!(
        piece
            .owned_regions
            .iter()
            .any(|region| region.kind == RoadSurfaceBandKind::Sidewalk),
        "node non-road hardcut must expose explicit sidewalk owners"
    );
}
