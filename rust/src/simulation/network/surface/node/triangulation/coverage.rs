//! Triangle inclusion and coverage validation for node CDT output.

use super::*;

pub(super) fn triangle_is_inside_owner(
    node_id: u32,
    region_index: usize,
    triangle: &NodeTriangulatedTriangle,
    vertices: &[NodeTriangulatedVertex],
    owner_shape: &NodeOverlayShape,
) -> Result<bool, NodeTriangulationError> {
    let centroid = triangle.vertices.iter().fold([0.0, 0.0], |mut sum, index| {
        let point = vertices[*index].point_world;
        sum[0] += point.x / 3.0;
        sum[1] += point.z / 3.0;
        sum
    });
    // Every owner contour is a CDT constraint, so a face whose interior point belongs to the
    // owner cannot cross an outer or hole boundary. Keep the boolean fallback for exterior and
    // boundary-adjacent numeric cases instead of relying on that invariant to reject triangles.
    if overlay_shape_contains_interior_point(owner_shape, centroid) {
        return Ok(true);
    }
    let triangle_shape = vec![positive_triangle_contour(triangle, vertices)];
    let triangle_shapes = vec![triangle_shape];
    let owner_shapes = vec![owner_shape.clone()];
    let residual = overlay_difference(
        node_id,
        region_index,
        &triangle_shapes,
        &owner_shapes,
        "triangle_minus_owner",
    )?;
    Ok(residual.is_empty())
}

pub(super) fn overlay_shape_contains_interior_point(
    shape: &NodeOverlayShape,
    point: NodeOverlayPoint,
) -> bool {
    let mut inside = false;
    for contour in shape {
        if point_lies_on_contour(point, contour) {
            return false;
        }
        if contour_contains_point(contour, point) {
            inside = !inside;
        }
    }
    inside
}

fn contour_contains_point(contour: &NodeOverlayContour, point: NodeOverlayPoint) -> bool {
    if contour.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = contour[contour.len() - 1];
    for current in contour {
        if (current[1] > point[1]) != (previous[1] > point[1]) {
            let crossing_x = previous[0]
                + (point[1] - previous[1]) * (current[0] - previous[0])
                    / (current[1] - previous[1]);
            if point[0] < crossing_x {
                inside = !inside;
            }
        }
        previous = *current;
    }
    inside
}

fn point_lies_on_contour(point: NodeOverlayPoint, contour: &NodeOverlayContour) -> bool {
    if contour.len() < 2 {
        return false;
    }
    (0..contour.len()).any(|index| {
        point_lies_on_segment(point, contour[index], contour[(index + 1) % contour.len()])
    })
}

fn point_lies_on_segment(
    point: NodeOverlayPoint,
    start: NodeOverlayPoint,
    end: NodeOverlayPoint,
) -> bool {
    let dx = end[0] - start[0];
    let dz = end[1] - start[1];
    let px = point[0] - start[0];
    let pz = point[1] - start[1];
    let length = dx.hypot(dz);
    if length <= f64::EPSILON {
        return false;
    }
    let cross = (dx * pz - dz * px).abs();
    if cross > f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) * length {
        return false;
    }
    let dot = px * dx + pz * dz;
    dot >= 0.0 && dot <= length * length
}

pub(super) fn reject_triangle_coverage_mismatch(
    node_id: u32,
    region_index: usize,
    owner_shape: &NodeOverlayShape,
    triangles: &[NodeTriangulatedTriangle],
    vertices: &[NodeTriangulatedVertex],
) -> Result<(), NodeTriangulationError> {
    let (missing, extra) =
        triangle_coverage_residual_shapes(node_id, region_index, owner_shape, triangles, vertices)?;
    let missing_area_m2 = overlay_area_m2(&missing);
    let extra_area_m2 = overlay_area_m2(&extra);
    let area_budget_m2 = RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(owner_shape);
    if missing_area_m2 > area_budget_m2 || extra_area_m2 > area_budget_m2 {
        return Err(NodeTriangulationError::TriangleCoverageMismatch {
            node_id,
            region_index,
            missing_area_m2,
            extra_area_m2,
        });
    }
    Ok(())
}

pub(super) fn triangle_coverage_residual_shapes(
    node_id: u32,
    region_index: usize,
    owner_shape: &NodeOverlayShape,
    triangles: &[NodeTriangulatedTriangle],
    vertices: &[NodeTriangulatedVertex],
) -> Result<(NodeOverlayShapes, NodeOverlayShapes), NodeTriangulationError> {
    let owner_shapes = vec![owner_shape.clone()];
    let triangle_contours = triangles
        .iter()
        .map(|triangle| positive_triangle_contour(triangle, vertices))
        .collect::<Vec<_>>();
    let triangle_shapes =
        overlay_union(node_id, region_index, &triangle_contours, "triangle_union")?;
    let missing = overlay_difference(
        node_id,
        region_index,
        &owner_shapes,
        &triangle_shapes,
        "owner_minus_triangles",
    )?;
    let extra = overlay_difference(
        node_id,
        region_index,
        &triangle_shapes,
        &owner_shapes,
        "triangles_minus_owner",
    )?;
    Ok((missing, extra))
}

fn overlay_union(
    node_id: u32,
    region_index: usize,
    contours: &[NodeOverlayContour],
    stage: &'static str,
) -> Result<NodeOverlayShapes, NodeTriangulationError> {
    let mut shapes = RoadSurfaceSystem::overlay_union_contours(contours).ok_or(
        NodeTriangulationError::BooleanOperationFailed {
            node_id,
            region_index,
            stage,
        },
    )?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    Ok(shapes)
}

fn overlay_difference(
    node_id: u32,
    region_index: usize,
    subject: &NodeOverlayShapes,
    clip: &NodeOverlayShapes,
    stage: &'static str,
) -> Result<NodeOverlayShapes, NodeTriangulationError> {
    let mut shapes =
        RoadSurfaceSystem::overlay_binary_shapes(subject, clip, OverlayRule::Difference).ok_or(
            NodeTriangulationError::BooleanOperationFailed {
                node_id,
                region_index,
                stage,
            },
        )?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    Ok(shapes)
}

pub(super) fn overlay_shape_from_arrangement_region(
    arrangement: &NodeArrangement,
    region: &NodeOwnedRegion,
) -> NodeOverlayShape {
    std::iter::once(region.outer_loop())
        .chain(region.holes().iter().map(Vec::as_slice))
        .map(|contour| {
            contour
                .iter()
                .filter_map(|vertex_id| arrangement.vertices().get(vertex_id.index()))
                .map(|vertex| {
                    let point = vertex.point_xz();
                    [point.x, point.y]
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn positive_triangle_contour(
    triangle: &NodeTriangulatedTriangle,
    vertices: &[NodeTriangulatedVertex],
) -> NodeOverlayContour {
    let mut contour = triangle
        .vertices
        .iter()
        .map(|index| overlay_point_from_vertex(&vertices[*index]))
        .collect::<Vec<_>>();
    if RoadSurfaceSystem::overlay_contour_area(&contour) < 0.0 {
        contour.swap(1, 2);
    }
    contour
}

fn overlay_point_from_vertex(vertex: &NodeTriangulatedVertex) -> NodeOverlayPoint {
    [vertex.point_world.x, vertex.point_world.z]
}

fn overlay_area_m2(shapes: &NodeOverlayShapes) -> f32 {
    shapes
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum()
}

pub(super) fn triangle_double_area_m2(
    triangle: &NodeTriangulatedTriangle,
    vertices: &[NodeTriangulatedVertex],
) -> f64 {
    let a = vertices[triangle.vertices[0]].point_world;
    let b = vertices[triangle.vertices[1]].point_world;
    let c = vertices[triangle.vertices[2]].point_world;
    RoadSurfaceSystem::road_triangle_double_area_xz_m2([a, b, c])
}

pub(super) fn triangle_sort_key(
    triangle: &NodeTriangulatedTriangle,
    vertices: &[NodeTriangulatedVertex],
) -> [NodeTriangulationPointKey; 3] {
    let mut keys = triangle
        .vertices
        .map(|index| NodeTriangulationPointKey::from_world(vertices[index].point_world));
    keys.sort();
    keys
}
