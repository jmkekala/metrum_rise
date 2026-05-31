//! Triangle validation router and shared triangle index helpers.

mod coverage;
mod edge_heights;

use super::super::indices::normalized_vertex_edge;
use super::super::triangulation::{
    NodeTriangulatedRegion, NodeTriangulatedTriangle, NodeTriangulationSolution,
};
use super::super::{NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, RoadSurfaceBandKind};
use super::boundaries::{
    diagnostic_min_distance_to_boundary_mm,
    edge_lies_on_explicit_boundary_constraint_or_backend_epsilon,
};
use super::report::{
    NodeBoundaryRegionDiagnostic, NodeGeometryBackend, NodeGeometryDiagnostic,
    NodeGeometryDiagnosticKind, NodeGeometryStage, NodeInvalidConstraintReason,
    push_validation_diagnostic,
};
use super::{
    BoundarySegment, NodeValidationEdgeKey, NodeValidationPointKey,
    VALIDATION_MIN_SEGMENT_LENGTH_M, edge_key_for_indices, point_key_from_world, quantize_m,
};
use std::collections::{BTreeMap, BTreeSet};

const NODE_TOP_SURFACE_MAX_CARRIAGEWAY_ASPECT_RATIO: f64 = 10_000.0;
const NODE_TOP_SURFACE_MAX_CARRIAGEWAY_SLOPE_DEGREES: f64 = 80.0;
const NODE_TOP_SURFACE_MAX_ADJACENT_NORMAL_ANGLE_DEGREES: f64 = 70.0;
const NODE_TOP_SURFACE_MIN_BLOCKING_QUALITY_AREA_M2: f64 = 0.01;
const TRIANGLE_QUALITY_EPS: f64 = 1.0e-12;

pub(super) fn validate_cross_region_triangle_edge_heights(
    solution: &NodeTriangulationSolution,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    edge_heights::validate_cross_region_triangle_edge_heights(solution, diagnostics);
}

pub(super) fn validate_triangle_area_coverage(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    coverage::validate_triangle_area_coverage(solution, region_index, region, diagnostics);
}

pub(super) fn validate_top_surface_triangle_quality(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    if !matches!(
        solution.piece_kind,
        super::super::RoadSurfaceVisualNodePieceKind::Bend
            | super::super::RoadSurfaceVisualNodePieceKind::JunctionN
    ) || region.kind != RoadSurfaceBandKind::Carriageway
    {
        return;
    }

    let adjacent_angles = triangle_adjacent_normal_angles(region);
    let plane_residual_max_m = region_plane_residual_max_m(region);
    for (triangle_index, triangle) in region.triangles.iter().enumerate() {
        let Some(quality) = triangle_quality(region, triangle) else {
            continue;
        };
        let max_adjacent_normal_angle_degrees = adjacent_angles
            .get(&triangle_index)
            .copied()
            .unwrap_or_default();
        let blocks_visual_quality =
            quality.area_m2 >= NODE_TOP_SURFACE_MIN_BLOCKING_QUALITY_AREA_M2;
        let reason = if blocks_visual_quality
            && quality.min_edge_m < f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M)
        {
            Some("numeric_dust_edge")
        } else if blocks_visual_quality
            && quality.aspect_ratio > NODE_TOP_SURFACE_MAX_CARRIAGEWAY_ASPECT_RATIO
        {
            Some("carriageway_aspect_ratio")
        } else if blocks_visual_quality
            && quality.slope_degrees > NODE_TOP_SURFACE_MAX_CARRIAGEWAY_SLOPE_DEGREES
        {
            Some("carriageway_slope")
        } else if blocks_visual_quality
            && max_adjacent_normal_angle_degrees
                > NODE_TOP_SURFACE_MAX_ADJACENT_NORMAL_ANGLE_DEGREES
        {
            Some("carriageway_adjacent_normal")
        } else {
            None
        };
        let Some(reason) = reason else {
            continue;
        };

        let vertex_keys = quality.vertex_keys;
        diagnostics.push(NodeGeometryDiagnostic {
            node_id: solution.node_id,
            piece_kind: solution.piece_kind,
            stage: NodeGeometryStage::Validation,
            backend: NodeGeometryBackend::Spade,
            kind: NodeGeometryDiagnosticKind::PathologicalTopSurfaceTriangle {
                region_index,
                owner: region.owner.kind(),
                owner_index: region.owner.owner_index(),
                height_field_id: region.height_field_id,
                triangle_index,
                reason,
                area_m2: quality.area_m2,
                min_edge_m: quality.min_edge_m,
                max_edge_m: quality.max_edge_m,
                aspect_ratio: quality.aspect_ratio,
                slope_degrees: quality.slope_degrees,
                y_delta_m: quality.y_delta_m,
                max_adjacent_normal_angle_degrees,
                plane_residual_max_m,
                vertex_x_keys: vertex_keys.map(|key| key.x_key),
                vertex_z_keys: vertex_keys.map(|key| key.z_key),
                vertex_x_mm: vertex_keys.map(|key| key.x_mm()),
                vertex_z_mm: vertex_keys.map(|key| key.z_mm()),
                vertex_height_mm: quality.vertex_height_mm,
            },
        });
    }
}

pub(super) fn validate_triangles(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    boundary_segments: &[BoundarySegment],
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) -> Vec<NodeValidationEdgeKey> {
    let boundary_edges = boundary_segments
        .iter()
        .map(|segment| segment.edge)
        .collect::<BTreeSet<_>>();
    let mut triangle_edge_counts = BTreeMap::<[usize; 2], usize>::new();
    for triangle in &region.triangles {
        if !triangle_indices_valid(triangle, region.vertices.len()) {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index,
                    constraint_index: None,
                    reason: NodeInvalidConstraintReason::OutOfRange,
                },
            );
            continue;
        }
        for edge in triangle_edges(triangle) {
            *triangle_edge_counts.entry(edge).or_default() += 1;
        }
    }

    let mut exposed_edges = Vec::new();
    for (edge, count) in triangle_edge_counts {
        if count > 2 {
            let edge_key = edge_key_for_indices(region, edge);
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::DuplicateExposedEdge {
                    region_index: Some(region_index),
                    regions: vec![NodeBoundaryRegionDiagnostic {
                        region_index,
                        owner: region.owner.kind(),
                        owner_index: region.owner.owner_index(),
                        height_field_id: region.height_field_id,
                    }],
                    start_x_key: edge_key.start.x_key,
                    start_z_key: edge_key.start.z_key,
                    end_x_key: edge_key.end.x_key,
                    end_z_key: edge_key.end.z_key,
                    start_x_mm: edge_key.start.x_mm(),
                    start_z_mm: edge_key.start.z_mm(),
                    end_x_mm: edge_key.end.x_mm(),
                    end_z_mm: edge_key.end.z_mm(),
                    count,
                },
            );
            continue;
        }
        if count != 1 {
            continue;
        }
        let edge_key = edge_key_for_indices(region, edge);
        exposed_edges.push(edge_key);
        if boundary_edges.contains(&edge)
            || edge_lies_on_explicit_boundary_constraint_or_backend_epsilon(
                region,
                edge,
                boundary_segments,
            )
        {
            continue;
        }
        let start_distance_mm = diagnostic_min_distance_to_boundary_mm(
            region.vertices[edge[0]].point_world,
            boundary_segments,
        );
        let end_distance_mm = diagnostic_min_distance_to_boundary_mm(
            region.vertices[edge[1]].point_world,
            boundary_segments,
        );
        for (vertex_index, distance_mm) in
            [(edge[0], start_distance_mm), (edge[1], end_distance_mm)]
        {
            if distance_mm > quantize_m(f64::from(VALIDATION_MIN_SEGMENT_LENGTH_M)) {
                let key = point_key_from_world(region.vertices[vertex_index].point_world);
                push_validation_diagnostic(
                    solution,
                    diagnostics,
                    NodeGeometryBackend::Parry2d,
                    NodeGeometryDiagnosticKind::NonExplicitBoundaryVertex {
                        region_index,
                        owner: region.owner.kind(),
                        owner_index: region.owner.owner_index(),
                        height_field_id: region.height_field_id,
                        x_key: key.x_key,
                        z_key: key.z_key,
                        x_mm: key.x_mm(),
                        z_mm: key.z_mm(),
                        min_boundary_distance_mm: distance_mm,
                    },
                );
            }
        }
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::Parry2d,
            NodeGeometryDiagnosticKind::OpenBoundary {
                region_index,
                owner: region.owner.kind(),
                owner_index: region.owner.owner_index(),
                height_field_id: region.height_field_id,
                vertex_index: None,
                x_key: None,
                z_key: None,
                x_mm: None,
                z_mm: None,
                start_x_key: Some(edge_key.start.x_key),
                start_z_key: Some(edge_key.start.z_key),
                end_x_key: Some(edge_key.end.x_key),
                end_z_key: Some(edge_key.end.z_key),
                start_x_mm: Some(edge_key.start.x_mm()),
                start_z_mm: Some(edge_key.start.z_mm()),
                end_x_mm: Some(edge_key.end.x_mm()),
                end_z_mm: Some(edge_key.end.z_mm()),
                degree: 1,
            },
        );
    }
    exposed_edges
}

#[derive(Clone, Copy)]
struct TopSurfaceTriangleQuality {
    area_m2: f64,
    min_edge_m: f64,
    max_edge_m: f64,
    aspect_ratio: f64,
    slope_degrees: f64,
    y_delta_m: f64,
    vertex_keys: [NodeValidationPointKey; 3],
    vertex_height_mm: [i64; 3],
}

fn triangle_quality(
    region: &NodeTriangulatedRegion,
    triangle: &NodeTriangulatedTriangle,
) -> Option<TopSurfaceTriangleQuality> {
    if !triangle_indices_valid(triangle, region.vertices.len()) {
        return None;
    }
    let points = triangle
        .vertices
        .map(|index| region.vertices[index].point_world);
    let edge_lengths = [
        xz_distance(points[0], points[1]),
        xz_distance(points[1], points[2]),
        xz_distance(points[2], points[0]),
    ];
    let min_edge_m = edge_lengths.into_iter().fold(f64::INFINITY, f64::min);
    let max_edge_m = edge_lengths.into_iter().fold(0.0_f64, f64::max);
    let area_m2 = triangle_xz_area_m2(points);
    let aspect_ratio = if area_m2 <= TRIANGLE_QUALITY_EPS {
        f64::INFINITY
    } else {
        max_edge_m * max_edge_m / (2.0 * area_m2)
    };
    let normal = normalized_triangle_normal(points)?;
    let y_min = points
        .iter()
        .map(|point| point.y)
        .fold(f64::INFINITY, f64::min);
    let y_max = points
        .iter()
        .map(|point| point.y)
        .fold(f64::NEG_INFINITY, f64::max);
    Some(TopSurfaceTriangleQuality {
        area_m2,
        min_edge_m,
        max_edge_m,
        aspect_ratio,
        slope_degrees: triangle_slope_degrees_from_normal(normal),
        y_delta_m: y_max - y_min,
        vertex_keys: points.map(point_key_from_world),
        vertex_height_mm: points.map(|point| quantize_m(point.y)),
    })
}

fn triangle_adjacent_normal_angles(region: &NodeTriangulatedRegion) -> BTreeMap<usize, f64> {
    let mut edges = BTreeMap::<[usize; 2], Vec<(usize, super::super::backend::RoadVec3)>>::new();
    for (triangle_index, triangle) in region.triangles.iter().enumerate() {
        if !triangle_indices_valid(triangle, region.vertices.len()) {
            continue;
        }
        let points = triangle
            .vertices
            .map(|index| region.vertices[index].point_world);
        let Some(normal) = normalized_triangle_normal(points) else {
            continue;
        };
        for edge in triangle_edges(triangle) {
            edges
                .entry(edge)
                .or_default()
                .push((triangle_index, normal));
        }
    }

    let mut max_angle_by_triangle = BTreeMap::<usize, f64>::new();
    for samples in edges.values() {
        for left_index in 0..samples.len() {
            for right in samples.iter().skip(left_index + 1) {
                let left = samples[left_index];
                let angle = normal_angle_degrees(left.1, right.1);
                max_angle_by_triangle
                    .entry(left.0)
                    .and_modify(|value| *value = (*value).max(angle))
                    .or_insert(angle);
                max_angle_by_triangle
                    .entry(right.0)
                    .and_modify(|value| *value = (*value).max(angle))
                    .or_insert(angle);
            }
        }
    }
    max_angle_by_triangle
}

fn region_plane_residual_max_m(region: &NodeTriangulatedRegion) -> Option<f64> {
    let mut points_by_key = BTreeMap::new();
    for triangle in &region.triangles {
        if !triangle_indices_valid(triangle, region.vertices.len()) {
            continue;
        }
        for vertex_index in triangle.vertices {
            let point = region.vertices[vertex_index].point_world;
            points_by_key.insert(point_key_from_world(point), point);
        }
    }
    plane_fit_residual_max_m(&points_by_key.into_values().collect::<Vec<_>>())
}

fn plane_fit_residual_max_m(points: &[super::super::backend::RoadVec3]) -> Option<f64> {
    if points.len() < 3 {
        return None;
    }
    let inv_count = 1.0 / points.len() as f64;
    let x_origin = points.iter().map(|point| point.x).sum::<f64>() * inv_count;
    let z_origin = points.iter().map(|point| point.z).sum::<f64>() * inv_count;

    let mut s_x = 0.0;
    let mut s_z = 0.0;
    let mut s_y = 0.0;
    let mut s_xx = 0.0;
    let mut s_xz = 0.0;
    let mut s_zz = 0.0;
    let mut s_xy = 0.0;
    let mut s_zy = 0.0;
    for point in points {
        let x = point.x - x_origin;
        let z = point.z - z_origin;
        s_x += x;
        s_z += z;
        s_y += point.y;
        s_xx += x * x;
        s_xz += x * z;
        s_zz += z * z;
        s_xy += x * point.y;
        s_zy += z * point.y;
    }

    let solution = solve_3x3(
        [
            [s_xx, s_xz, s_x],
            [s_xz, s_zz, s_z],
            [s_x, s_z, points.len() as f64],
        ],
        [s_xy, s_zy, s_y],
    )?;
    let grade_x = solution[0];
    let grade_z = solution[1];
    let plane_y_at_origin = solution[2];

    points.iter().fold(Some(0.0_f64), |max_residual, point| {
        let max_residual = max_residual?;
        let fitted_y =
            grade_x * (point.x - x_origin) + grade_z * (point.z - z_origin) + plane_y_at_origin;
        Some(max_residual.max((point.y - fitted_y).abs()))
    })
}

fn solve_3x3(mut matrix: [[f64; 3]; 3], mut rhs: [f64; 3]) -> Option<[f64; 3]> {
    for column in 0..3 {
        let mut pivot_row = column;
        let mut pivot_abs = matrix[column][column].abs();
        for (row, values) in matrix.iter().enumerate().skip(column + 1) {
            let candidate_abs = values[column].abs();
            if candidate_abs > pivot_abs {
                pivot_abs = candidate_abs;
                pivot_row = row;
            }
        }
        if pivot_abs <= TRIANGLE_QUALITY_EPS {
            return None;
        }
        if pivot_row != column {
            matrix.swap(column, pivot_row);
            rhs.swap(column, pivot_row);
        }
        let pivot = matrix[column][column];
        for value in matrix[column].iter_mut().skip(column) {
            *value /= pivot;
        }
        rhs[column] /= pivot;

        let pivot_values = matrix[column];
        let pivot_rhs = rhs[column];
        for row in 0..3 {
            if row == column {
                continue;
            }
            let factor = matrix[row][column];
            if factor.abs() <= TRIANGLE_QUALITY_EPS {
                continue;
            }
            for value_column in column..3 {
                matrix[row][value_column] -= factor * pivot_values[value_column];
            }
            rhs[row] -= factor * pivot_rhs;
        }
    }
    Some(rhs)
}

fn normalized_triangle_normal(
    points: [super::super::backend::RoadVec3; 3],
) -> Option<super::super::backend::RoadVec3> {
    let normal = (points[1] - points[0]).cross(points[2] - points[0]);
    let length_squared = normal.length_squared();
    if length_squared <= TRIANGLE_QUALITY_EPS {
        return None;
    }
    Some(normal / length_squared.sqrt())
}

fn normal_angle_degrees(
    left: super::super::backend::RoadVec3,
    right: super::super::backend::RoadVec3,
) -> f64 {
    left.dot(right).abs().clamp(0.0, 1.0).acos().to_degrees()
}

fn triangle_slope_degrees_from_normal(normal: super::super::backend::RoadVec3) -> f64 {
    let vertical = normal.y.abs();
    if vertical <= TRIANGLE_QUALITY_EPS {
        90.0
    } else {
        (normal.x.hypot(normal.z) / vertical).atan().to_degrees()
    }
}

fn xz_distance(a: super::super::backend::RoadVec3, b: super::super::backend::RoadVec3) -> f64 {
    (a.x - b.x).hypot(a.z - b.z)
}

fn triangle_xz_area_m2(points: [super::super::backend::RoadVec3; 3]) -> f64 {
    ((points[1].x - points[0].x) * (points[2].z - points[0].z)
        - (points[1].z - points[0].z) * (points[2].x - points[0].x))
        .abs()
        * 0.5
}

fn triangle_edges(triangle: &NodeTriangulatedTriangle) -> [[usize; 2]; 3] {
    [
        normalized_vertex_edge(triangle.vertices[0], triangle.vertices[1]),
        normalized_vertex_edge(triangle.vertices[1], triangle.vertices[2]),
        normalized_vertex_edge(triangle.vertices[2], triangle.vertices[0]),
    ]
}

fn triangle_indices_valid(triangle: &NodeTriangulatedTriangle, vertex_count: usize) -> bool {
    triangle.vertices.iter().all(|index| *index < vertex_count)
        && triangle.vertices[0] != triangle.vertices[1]
        && triangle.vertices[1] != triangle.vertices[2]
        && triangle.vertices[2] != triangle.vertices[0]
}

fn edge_indices_valid(edge: [usize; 2], vertex_count: usize) -> bool {
    edge[0] < vertex_count && edge[1] < vertex_count && edge[0] != edge[1]
}
