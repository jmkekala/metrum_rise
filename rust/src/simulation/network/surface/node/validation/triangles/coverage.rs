//! Triangle area and coverage validation.

use super::super::super::triangulation::{
    NodeTriangulatedRegion, NodeTriangulatedTriangle, NodeTriangulationSolution,
};
use super::super::super::{NodeOverlayContour, RoadSurfaceSystem};
use super::super::report::{
    NodeGeometryBackend, NodeGeometryDiagnostic, NodeGeometryDiagnosticKind,
    push_validation_diagnostic,
};
use super::triangle_indices_valid;

pub(super) fn validate_triangle_area_coverage(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    if region.triangles.is_empty() {
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::Spade,
            NodeGeometryDiagnosticKind::BackendFailure {
                reason: "empty_triangle_set",
            },
        );
        return;
    }
    let triangle_contours = region
        .triangles
        .iter()
        .filter(|triangle| triangle_indices_valid(triangle, region.vertices.len()))
        .map(|triangle| triangle_contour(region, triangle))
        .collect::<Vec<_>>();
    let Some(triangle_shapes) = RoadSurfaceSystem::overlay_union_contours(&triangle_contours)
    else {
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::IOverlay,
            NodeGeometryDiagnosticKind::BackendFailure {
                reason: "triangle_union_failed",
            },
        );
        return;
    };
    let union_area = triangle_shapes
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum::<f32>();
    let triangle_area_sum = region
        .triangles
        .iter()
        .filter(|triangle| triangle_indices_valid(triangle, region.vertices.len()))
        .map(|triangle| triangle_area_m2(region, triangle))
        .sum::<f32>();
    let overlap_area_m2 = (triangle_area_sum - union_area).max(0.0);
    let area_budget_m2 =
        RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&triangle_shapes);
    if overlap_area_m2 > area_budget_m2 {
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::IOverlay,
            NodeGeometryDiagnosticKind::TriangleOverlap {
                region_index,
                overlap_area_m2,
            },
        );
    }

    let area_delta = union_area - region.area_m2;
    if area_delta.abs() > area_budget_m2 {
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::IOverlay,
            NodeGeometryDiagnosticKind::TriangleCoverageMismatch {
                region_index,
                missing_area_m2: (-area_delta).max(0.0),
                extra_area_m2: area_delta.max(0.0),
            },
        );
    }
}

fn triangle_contour(
    region: &NodeTriangulatedRegion,
    triangle: &NodeTriangulatedTriangle,
) -> NodeOverlayContour {
    let mut contour = triangle
        .vertices
        .iter()
        .map(|index| {
            let point = region.vertices[*index].point_world;
            [point.x, point.z]
        })
        .collect::<Vec<_>>();
    if RoadSurfaceSystem::overlay_contour_area(&contour) < 0.0 {
        contour.swap(1, 2);
    }
    contour
}

fn triangle_area_m2(region: &NodeTriangulatedRegion, triangle: &NodeTriangulatedTriangle) -> f32 {
    let points = triangle
        .vertices
        .map(|index| region.vertices[index].point_world);
    (RoadSurfaceSystem::road_triangle_double_area_xz_m2(points) * 0.5) as f32
}
