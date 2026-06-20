//! Triangulation of individual node-owned regions.

use super::coverage::{
    overlay_shape_from_arrangement_region, reject_triangle_coverage_mismatch,
    triangle_coverage_residual_shapes, triangle_double_area_m2, triangle_is_inside_owner,
    triangle_sort_key,
};
use super::vertices::push_arrangement_constraint_loop;
use super::*;
use crate::simulation::network::surface::segments;

pub(super) fn triangulate_arrangement_region(
    node_id: u32,
    region_index: usize,
    arrangement: &NodeArrangement,
    region: &NodeOwnedRegion,
) -> Result<NodeTriangulatedRegion, NodeTriangulationError> {
    if region.outer_loop().is_empty() {
        return Err(NodeTriangulationError::EmptyRegionShape {
            node_id,
            region_index,
        });
    }

    let mut vertices = Vec::new();
    let mut vertex_lookup = BTreeMap::new();
    let mut constraints = BTreeSet::new();
    let owner = region.owner();
    let height_field_id = region.height_field_id();
    push_arrangement_constraint_loop(
        node_id,
        region_index,
        0,
        region.outer_loop(),
        arrangement,
        &mut vertices,
        &mut vertex_lookup,
        &mut constraints,
    )?;
    for (hole_index, hole) in region.holes().iter().enumerate() {
        push_arrangement_constraint_loop(
            node_id,
            region_index,
            hole_index + 1,
            hole,
            arrangement,
            &mut vertices,
            &mut vertex_lookup,
            &mut constraints,
        )?;
    }
    let owner_shape = overlay_shape_from_arrangement_region(arrangement, region);
    insert_carriageway_interior_guides(
        node_id,
        region_index,
        arrangement,
        region,
        &owner_shape,
        owner,
        height_field_id,
        &mut vertices,
        &mut vertex_lookup,
    )?;
    let spade_vertices = vertices
        .iter()
        .map(|vertex| Point2::new(vertex.point_world.x, vertex.point_world.z))
        .collect::<Vec<_>>();
    let constraint_list = constraints.iter().copied().collect::<Vec<_>>();
    let mut invalid_constraints = 0usize;
    let mut first_invalid_constraint = None;
    let cdt =
        SurfaceCdt::try_bulk_load_cdt(spade_vertices, constraint_list.clone(), |constraint| {
            invalid_constraints += 1;
            first_invalid_constraint
                .get_or_insert(normalized_vertex_edge(constraint[0], constraint[1]));
        })
        .map_err(|_| NodeTriangulationError::CdtBuildFailed {
            node_id,
            region_index,
        })?;
    if invalid_constraints > 0 {
        return Err(NodeTriangulationError::InvalidConstraint {
            node_id,
            region_index,
            constraint_count: invalid_constraints,
            first_constraint_index: first_invalid_constraint
                .and_then(|constraint| constraint_list.iter().position(|edge| *edge == constraint)),
            first_constraint: first_invalid_constraint,
        });
    }

    let mut triangles = Vec::new();
    for face in cdt.inner_faces() {
        let [a, b, c] = face.vertices();
        let triangle = NodeTriangulatedTriangle {
            vertices: [a.fix().index(), b.fix().index(), c.fix().index()],
        };
        if triangle_area_is_numeric_dust(&triangle, &vertices) {
            continue;
        }
        if triangle_is_inside_owner(node_id, region_index, &triangle, &vertices, &owner_shape)? {
            triangles.push(triangle);
        } else {
            append_owner_clipped_triangles(
                node_id,
                region_index,
                &triangle,
                &owner_shape,
                owner,
                height_field_id,
                &mut vertices,
                &mut vertex_lookup,
                &mut constraints,
                &mut triangles,
            )?;
        }
    }
    append_missing_owner_coverage_triangles(
        node_id,
        region_index,
        arrangement,
        region,
        &owner_shape,
        owner,
        height_field_id,
        &mut vertices,
        &mut vertex_lookup,
        &mut constraints,
        &mut triangles,
    )?;
    triangles.sort_by(|a, b| triangle_sort_key(a, &vertices).cmp(&triangle_sort_key(b, &vertices)));
    triangles.dedup();
    if triangles.is_empty() {
        return Err(NodeTriangulationError::EmptyTriangulation {
            node_id,
            region_index,
        });
    }

    reject_triangle_coverage_mismatch(node_id, region_index, &owner_shape, &triangles, &vertices)?;
    append_effective_exposed_boundary_constraints(
        &mut constraints,
        &triangles,
        &vertices,
        &owner_shape,
    );
    canonicalize_boundary_constraint_numeric_dust_vertices(&mut constraints, &vertices);

    Ok(NodeTriangulatedRegion {
        kind: owner.kind(),
        owner,
        height_field_id,
        vertices,
        boundary_constraints: constraints.into_iter().collect(),
        triangles,
        area_m2: region.area_m2(),
    })
}

fn append_effective_exposed_boundary_constraints(
    boundary_constraints: &mut BTreeSet<[usize; 2]>,
    triangles: &[NodeTriangulatedTriangle],
    vertices: &[NodeTriangulatedVertex],
    owner_shape: &NodeOverlayShape,
) {
    let mut triangle_edge_counts = BTreeMap::<[usize; 2], usize>::new();
    for triangle in triangles {
        for edge_index in 0..3 {
            let edge = normalized_vertex_edge(
                triangle.vertices[edge_index],
                triangle.vertices[(edge_index + 1) % 3],
            );
            *triangle_edge_counts.entry(edge).or_default() += 1;
        }
    }
    for (edge, count) in triangle_edge_counts {
        if count != 1 || boundary_constraints.contains(&edge) {
            continue;
        }
        let Some(start) = vertices.get(edge[0]) else {
            continue;
        };
        let Some(end) = vertices.get(edge[1]) else {
            continue;
        };
        if edge_lies_on_owner_shape_boundary(start.point_world, end.point_world, owner_shape) {
            boundary_constraints.insert(edge);
        }
    }
}

fn canonicalize_boundary_constraint_numeric_dust_vertices(
    boundary_constraints: &mut BTreeSet<[usize; 2]>,
    vertices: &[NodeTriangulatedVertex],
) {
    if boundary_constraints.is_empty() {
        return;
    }
    let canonical_indices = canonical_boundary_constraint_vertex_indices(vertices);
    let canonical_constraints = boundary_constraints
        .iter()
        .filter_map(|edge| {
            let start = *canonical_indices.get(edge[0])?;
            let end = *canonical_indices.get(edge[1])?;
            (start != end).then_some(normalized_vertex_edge(start, end))
        })
        .collect::<BTreeSet<_>>();
    *boundary_constraints = canonical_constraints;
}

fn canonical_boundary_constraint_vertex_indices(vertices: &[NodeTriangulatedVertex]) -> Vec<usize> {
    let mut canonical_by_index = Vec::with_capacity(vertices.len());
    let mut canonical_by_key = BTreeMap::<NodeTriangulationPointKey, Vec<usize>>::new();
    let dust_key_units = NODE_TRIANGULATION_CONSTRAINT_LOOP_DUST_KEY_UNITS;
    let dust_key_units_sq = i128::from(dust_key_units) * i128::from(dust_key_units);

    for (index, vertex) in vertices.iter().enumerate() {
        let key = NodeTriangulationPointKey::from_world(vertex.point_world);
        let range_start = NodeTriangulationPointKey {
            x_mm: key.x_mm - dust_key_units,
            z_mm: i64::MIN,
        };
        let range_end = NodeTriangulationPointKey {
            x_mm: key.x_mm + dust_key_units,
            z_mm: i64::MAX,
        };
        let canonical = canonical_by_key
            .range(range_start..=range_end)
            .flat_map(|(candidate_key, candidate_indices)| {
                candidate_indices
                    .iter()
                    .copied()
                    .map(|candidate_index| (*candidate_key, candidate_index))
            })
            .filter_map(|(candidate_key, candidate_index)| {
                if key.distance_key_units_sq(candidate_key) > dust_key_units_sq {
                    return None;
                }
                let candidate = vertices.get(candidate_index)?;
                boundary_constraint_vertices_are_equivalent(vertex, candidate).then_some((
                    key.distance_key_units_sq(candidate_key),
                    candidate_key,
                    candidate_index,
                ))
            })
            .min()
            .map(|(_, _, candidate_index)| candidate_index)
            .unwrap_or(index);
        canonical_by_index.push(canonical);
        canonical_by_key.entry(key).or_default().push(canonical);
    }

    canonical_by_index
}

fn boundary_constraint_vertices_are_equivalent(
    left: &NodeTriangulatedVertex,
    right: &NodeTriangulatedVertex,
) -> bool {
    left.height_field_id == right.height_field_id
        && quantize_m(left.point_world.y) == quantize_m(right.point_world.y)
        && left.grade_authority.owner == right.grade_authority.owner
        && left.grade_authority.height_field_id == right.grade_authority.height_field_id
        && left.grade_authority.height_key == right.grade_authority.height_key
}

fn edge_lies_on_owner_shape_boundary(
    start: RoadVec3,
    end: RoadVec3,
    owner_shape: &NodeOverlayShape,
) -> bool {
    let start = [start.x, start.z];
    let end = [end.x, end.z];
    owner_shape
        .iter()
        .any(|contour| edge_lies_on_contour_boundary(start, end, contour))
}

fn edge_lies_on_contour_boundary(
    start: NodeOverlayPoint,
    end: NodeOverlayPoint,
    contour: &NodeOverlayContour,
) -> bool {
    if contour.len() < 2 {
        return false;
    }
    let start_edges = contour_edges_touching_point(start, contour);
    let end_edges = contour_edges_touching_point(end, contour);
    start_edges.iter().copied().any(|start_edge| {
        end_edges.iter().copied().any(|end_edge| {
            contour_path_between_edges_lies_on_segment(start, end, start_edge, end_edge, contour)
                || contour_path_between_edges_lies_on_segment(
                    end, start, end_edge, start_edge, contour,
                )
        })
    })
}

fn contour_edges_touching_point(
    point: NodeOverlayPoint,
    contour: &NodeOverlayContour,
) -> Vec<usize> {
    (0..contour.len())
        .filter(|index| {
            point_lies_on_segment(
                point,
                contour[*index],
                contour[(*index + 1) % contour.len()],
            )
        })
        .collect()
}

fn contour_path_between_edges_lies_on_segment(
    start: NodeOverlayPoint,
    end: NodeOverlayPoint,
    start_edge: usize,
    end_edge: usize,
    contour: &NodeOverlayContour,
) -> bool {
    let mut cursor = (start_edge + 1) % contour.len();
    loop {
        if !point_lies_on_segment(contour[cursor], start, end) {
            return false;
        }
        if cursor == end_edge {
            return true;
        }
        cursor = (cursor + 1) % contour.len();
        if cursor == (start_edge + 1) % contour.len() {
            return false;
        }
    }
}

fn triangle_area_is_numeric_dust(
    triangle: &NodeTriangulatedTriangle,
    vertices: &[NodeTriangulatedVertex],
) -> bool {
    triangle_double_area_m2(triangle, vertices).abs() * 0.5 <= f64::from(NODE_OVERLAY_MIN_AREA_M2)
}

fn insert_carriageway_interior_guides(
    node_id: u32,
    region_index: usize,
    arrangement: &NodeArrangement,
    region: &NodeOwnedRegion,
    owner_shape: &NodeOverlayShape,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    vertices: &mut Vec<NodeTriangulatedVertex>,
    vertex_lookup: &mut BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
) -> Result<(), NodeTriangulationError> {
    if !matches!(
        arrangement.piece_kind(),
        RoadSurfaceVisualNodePieceKind::Bend | RoadSurfaceVisualNodePieceKind::JunctionN
    ) || region.owner().kind() != RoadSurfaceBandKind::Carriageway
    {
        return Ok(());
    }
    let boundary_points = region_vertices_world(arrangement, region);
    if !carriageway_region_needs_interior_guides(&boundary_points) {
        return Ok(());
    }
    let Some(plane) = carriageway_region_grade_plane(&boundary_points) else {
        return Ok(());
    };
    let Some((min_x, min_z, max_x, max_z)) = overlay_shape_bounds(owner_shape) else {
        return Ok(());
    };
    let span_x = max_x - min_x;
    let span_z = max_z - min_z;
    let longest_span = span_x.max(span_z);
    if longest_span <= NODE_TRIANGULATION_CARRIAGEWAY_GUIDE_SPACING_M * 2.0 {
        return Ok(());
    }
    let segment_count = ((longest_span / NODE_TRIANGULATION_CARRIAGEWAY_GUIDE_SPACING_M).ceil()
        as usize)
        .clamp(1, NODE_TRIANGULATION_MAX_GUIDE_SEGMENTS_PER_EDGE);
    for guide_index in 1..segment_count {
        let parameter = guide_index as f64 / segment_count as f64;
        let point = if span_x >= span_z {
            RoadVec2::new(min_x + span_x * parameter, (min_z + max_z) * 0.5)
        } else {
            RoadVec2::new((min_x + max_x) * 0.5, min_z + span_z * parameter)
        };
        let point_key = SurfaceXzKey::from_road_xz(point);
        let canonical_xz = point_key.to_road_xz();
        let overlay_point = [canonical_xz.x, canonical_xz.y];
        if !overlay_shape_contains_interior_point(owner_shape, overlay_point) {
            continue;
        }
        let height_m = plane_height_m(plane, canonical_xz);
        insert_carriageway_interior_guide_vertex(
            node_id,
            region_index,
            canonical_xz,
            height_m,
            owner,
            height_field_id,
            vertices,
            vertex_lookup,
        )?;
    }
    Ok(())
}

fn carriageway_region_needs_interior_guides(points: &[RoadVec3]) -> bool {
    if points.len() < 3 {
        return false;
    }
    let min_y = points
        .iter()
        .map(|point| point.y)
        .fold(f64::INFINITY, f64::min);
    let max_y = points
        .iter()
        .map(|point| point.y)
        .fold(f64::NEG_INFINITY, f64::max);
    max_y - min_y >= NODE_TRIANGULATION_GUIDE_MIN_HEIGHT_DELTA_M
}

fn carriageway_region_grade_plane(points: &[RoadVec3]) -> Option<[f64; 3]> {
    if points.len() < 3 {
        return None;
    }
    let first = points[0];
    let second = *points.iter().skip(1).max_by(|left, right| {
        xz_distance_squared(first, **left).total_cmp(&xz_distance_squared(first, **right))
    })?;
    if xz_distance_squared(first, second) <= 1.0e-12 {
        return None;
    }
    let third = *points.iter().max_by(|left, right| {
        xz_line_distance_numerator(first, second, **left)
            .total_cmp(&xz_line_distance_numerator(first, second, **right))
    })?;
    if xz_line_distance_numerator(first, second, third) <= 1.0e-9 {
        return None;
    }
    let plane = grade_plane_from_points([first, second, third])?;
    points
        .iter()
        .all(|point| {
            (plane_height_m(plane, RoadVec2::new(point.x, point.z)) - point.y).abs()
                <= NODE_TRIANGULATION_GUIDE_PLANE_MAX_RESIDUAL_M
        })
        .then_some(plane)
}

fn xz_distance_squared(a: RoadVec3, b: RoadVec3) -> f64 {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    dx * dx + dz * dz
}

fn xz_line_distance_numerator(start: RoadVec3, end: RoadVec3, point: RoadVec3) -> f64 {
    ((end.x - start.x) * (point.z - start.z) - (end.z - start.z) * (point.x - start.x)).abs()
}

fn region_vertices_world(arrangement: &NodeArrangement, region: &NodeOwnedRegion) -> Vec<RoadVec3> {
    let mut points_by_key = BTreeMap::new();
    for vertex_id in std::iter::once(region.outer_loop())
        .chain(region.holes().iter().map(Vec::as_slice))
        .flatten()
    {
        if let Some(vertex) = arrangement.vertices().get(vertex_id.index()) {
            let point = vertex.point_xz();
            points_by_key.insert(
                vertex.key(),
                RoadVec3::new(point.x, vertex.height_m(), point.y),
            );
        }
    }
    points_by_key.into_values().collect()
}

fn grade_plane_from_points(points: [RoadVec3; 3]) -> Option<[f64; 3]> {
    let [a, b, c] = points;
    let determinant = a.x * (b.z - c.z) + b.x * (c.z - a.z) + c.x * (a.z - b.z);
    if determinant.abs() <= 1.0e-9 {
        return None;
    }
    Some([
        (a.y * (b.z - c.z) + b.y * (c.z - a.z) + c.y * (a.z - b.z)) / determinant,
        (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y)) / determinant,
        (a.x * (b.z * c.y - c.z * b.y)
            + b.x * (c.z * a.y - a.z * c.y)
            + c.x * (a.z * b.y - b.z * a.y))
            / determinant,
    ])
}

fn plane_height_m(plane: [f64; 3], point: RoadVec2) -> f64 {
    plane[0] * point.x + plane[1] * point.y + plane[2]
}

fn insert_carriageway_interior_guide_vertex(
    node_id: u32,
    region_index: usize,
    point_xz: RoadVec2,
    height_m: f64,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    vertices: &mut Vec<NodeTriangulatedVertex>,
    vertex_lookup: &mut BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
) -> Result<usize, NodeTriangulationError> {
    let point_key =
        NodeTriangulationPointKey::from_world(RoadVec3::new(point_xz.x, height_m, point_xz.y));
    let height_key = NodeTriangulationHeightKey(quantize_m(height_m));
    if let Some((index, existing_height_key)) = vertex_lookup.get(&point_key).copied() {
        if existing_height_key != height_key {
            return Err(NodeTriangulationError::DuplicateVertexHeightConflict {
                node_id,
                region_index,
                x_mm: point_key.x_mm,
                z_mm: point_key.z_mm,
                existing_height_mm: existing_height_key.0,
                incoming_height_mm: height_key.0,
            });
        }
        return Ok(index);
    }
    let grade_authority = NodeGradeVertexAuthority::new(
        point_xz,
        height_m,
        owner,
        height_field_id,
        NodeGradeCarrierDecision::SameOwnerCanonicalVertex,
    );
    if let Some(index) = same_authority_numeric_dust_vertex(
        point_key,
        height_key,
        grade_authority,
        vertices,
        vertex_lookup,
    ) {
        return Ok(index);
    }
    let index = vertices.len();
    vertices.push(NodeTriangulatedVertex {
        point_world: RoadVec3::new(point_xz.x, height_m, point_xz.y),
        height_field_id,
        grade_authority,
    });
    vertex_lookup.insert(point_key, (index, height_key));
    Ok(index)
}

fn overlay_shape_bounds(shape: &NodeOverlayShape) -> Option<(f64, f64, f64, f64)> {
    let mut bounds: Option<(f64, f64, f64, f64)> = None;
    for point in shape.iter().flatten() {
        bounds = Some(match bounds {
            Some((min_x, min_z, max_x, max_z)) => (
                min_x.min(f64::from(point[0])),
                min_z.min(f64::from(point[1])),
                max_x.max(f64::from(point[0])),
                max_z.max(f64::from(point[1])),
            ),
            None => (
                f64::from(point[0]),
                f64::from(point[1]),
                f64::from(point[0]),
                f64::from(point[1]),
            ),
        });
    }
    bounds
}

fn overlay_shape_contains_interior_point(
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
        if (f64::from(current[1]) > f64::from(point[1]))
            != (f64::from(previous[1]) > f64::from(point[1]))
        {
            let crossing_x = f64::from(previous[0])
                + (f64::from(point[1]) - f64::from(previous[1]))
                    * (f64::from(current[0]) - f64::from(previous[0]))
                    / (f64::from(current[1]) - f64::from(previous[1]));
            if f64::from(point[0]) < crossing_x {
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
    let dx = f64::from(end[0] - start[0]);
    let dz = f64::from(end[1] - start[1]);
    let px = f64::from(point[0] - start[0]);
    let pz = f64::from(point[1] - start[1]);
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

fn append_owner_clipped_triangles(
    node_id: u32,
    region_index: usize,
    triangle: &NodeTriangulatedTriangle,
    owner_shape: &NodeOverlayShape,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    vertices: &mut Vec<NodeTriangulatedVertex>,
    vertex_lookup: &mut BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
    boundary_constraints: &mut BTreeSet<[usize; 2]>,
    triangles: &mut Vec<NodeTriangulatedTriangle>,
) -> Result<(), NodeTriangulationError> {
    let triangle_shape = vec![vec![positive_triangle_contour(triangle, vertices)]];
    let owner_shapes = vec![owner_shape.clone()];
    let mut clipped_shapes = RoadSurfaceSystem::overlay_binary_shapes(
        &triangle_shape,
        &owner_shapes,
        OverlayRule::Intersect,
    )
    .ok_or(NodeTriangulationError::BooleanOperationFailed {
        node_id,
        region_index,
        stage: "triangle_owner_intersection",
    })?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut clipped_shapes);
    for clipped_shape in clipped_shapes {
        if RoadSurfaceSystem::overlay_shape_area_m2(&clipped_shape) <= NODE_OVERLAY_MIN_AREA_M2 {
            continue;
        }
        append_triangulated_clipped_shape(
            node_id,
            region_index,
            triangle,
            &clipped_shape,
            owner,
            height_field_id,
            vertices,
            vertex_lookup,
            boundary_constraints,
            triangles,
        )?;
    }
    Ok(())
}

fn append_missing_owner_coverage_triangles(
    node_id: u32,
    region_index: usize,
    arrangement: &NodeArrangement,
    region: &NodeOwnedRegion,
    owner_shape: &NodeOverlayShape,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    vertices: &mut Vec<NodeTriangulatedVertex>,
    vertex_lookup: &mut BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
    boundary_constraints: &mut BTreeSet<[usize; 2]>,
    triangles: &mut Vec<NodeTriangulatedTriangle>,
) -> Result<(), NodeTriangulationError> {
    let source_triangles = triangles.clone();
    let (missing_shapes, _) =
        triangle_coverage_residual_shapes(node_id, region_index, owner_shape, triangles, vertices)?;
    for missing_shape in missing_shapes {
        if RoadSurfaceSystem::overlay_shape_area_m2(&missing_shape) <= NODE_OVERLAY_MIN_AREA_M2 {
            continue;
        }
        append_triangulated_missing_owner_shape(
            node_id,
            region_index,
            arrangement,
            region,
            &source_triangles,
            &missing_shape,
            owner,
            height_field_id,
            vertices,
            vertex_lookup,
            boundary_constraints,
            triangles,
        )?;
    }
    Ok(())
}

fn append_triangulated_clipped_shape(
    node_id: u32,
    region_index: usize,
    source_triangle: &NodeTriangulatedTriangle,
    clipped_shape: &NodeOverlayShape,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    vertices: &mut Vec<NodeTriangulatedVertex>,
    vertex_lookup: &mut BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
    boundary_constraints: &mut BTreeSet<[usize; 2]>,
    triangles: &mut Vec<NodeTriangulatedTriangle>,
) -> Result<(), NodeTriangulationError> {
    let mut local_to_global = Vec::<usize>::new();
    let mut local_by_global = BTreeMap::<usize, usize>::new();
    let mut constraints = BTreeSet::<[usize; 2]>::new();

    for contour in clipped_shape {
        let mut contour_indices = Vec::new();
        for point in contour {
            let global_index = insert_clipped_triangle_vertex(
                node_id,
                region_index,
                source_triangle,
                *point,
                owner,
                height_field_id,
                vertices,
                vertex_lookup,
            )?;
            let local_index = *local_by_global.entry(global_index).or_insert_with(|| {
                let index = local_to_global.len();
                local_to_global.push(global_index);
                index
            });
            if contour_indices.last().copied() != Some(local_index) {
                contour_indices.push(local_index);
            }
        }
        if contour_indices.len() >= 2
            && contour_indices.first().copied() == contour_indices.last().copied()
        {
            contour_indices.pop();
        }
        if contour_indices.len() < 3 {
            // Repair contours can collapse after snapping to already-heighted support vertices.
            // The final coverage check below decides whether the skipped repair leaves a
            // meaningful residual instead of failing here with a misleading CDT contour error.
            continue;
        }
        for index in 0..contour_indices.len() {
            let global_edge = normalized_vertex_edge(
                local_to_global[contour_indices[index]],
                local_to_global[contour_indices[(index + 1) % contour_indices.len()]],
            );
            let edge = normalized_vertex_edge(
                contour_indices[index],
                contour_indices[(index + 1) % contour_indices.len()],
            );
            if edge[0] != edge[1] {
                constraints.insert(edge);
            }
            if global_edge[0] != global_edge[1] {
                boundary_constraints.insert(global_edge);
            }
        }
    }

    if local_to_global.len() < 3 || constraints.is_empty() {
        return Ok(());
    }

    let spade_vertices = local_to_global
        .iter()
        .map(|global_index| {
            let point = vertices[*global_index].point_world;
            Point2::new(point.x, point.z)
        })
        .collect::<Vec<_>>();
    let constraint_list = constraints.into_iter().collect::<Vec<_>>();
    let mut invalid_constraints = 0usize;
    let mut first_invalid_constraint = None;
    let cdt =
        SurfaceCdt::try_bulk_load_cdt(spade_vertices, constraint_list.clone(), |constraint| {
            invalid_constraints += 1;
            first_invalid_constraint
                .get_or_insert(normalized_vertex_edge(constraint[0], constraint[1]));
        })
        .map_err(|_| NodeTriangulationError::CdtBuildFailed {
            node_id,
            region_index,
        })?;
    if invalid_constraints > 0 {
        return Err(NodeTriangulationError::InvalidConstraint {
            node_id,
            region_index,
            constraint_count: invalid_constraints,
            first_constraint_index: first_invalid_constraint
                .and_then(|constraint| constraint_list.iter().position(|edge| *edge == constraint)),
            first_constraint: first_invalid_constraint,
        });
    }

    for face in cdt.inner_faces() {
        let [a, b, c] = face.vertices();
        let triangle = NodeTriangulatedTriangle {
            vertices: [
                local_to_global[a.fix().index()],
                local_to_global[b.fix().index()],
                local_to_global[c.fix().index()],
            ],
        };
        if triangle_area_is_numeric_dust(&triangle, vertices) {
            continue;
        }
        if triangle_is_inside_owner(node_id, region_index, &triangle, vertices, clipped_shape)? {
            triangles.push(triangle);
        }
    }

    Ok(())
}

fn append_triangulated_missing_owner_shape(
    node_id: u32,
    region_index: usize,
    arrangement: &NodeArrangement,
    region: &NodeOwnedRegion,
    source_triangles: &[NodeTriangulatedTriangle],
    missing_shape: &NodeOverlayShape,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    vertices: &mut Vec<NodeTriangulatedVertex>,
    vertex_lookup: &mut BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
    boundary_constraints: &mut BTreeSet<[usize; 2]>,
    triangles: &mut Vec<NodeTriangulatedTriangle>,
) -> Result<(), NodeTriangulationError> {
    let mut local_to_global = Vec::<usize>::new();
    let mut local_by_global = BTreeMap::<usize, usize>::new();
    let mut constraints = BTreeSet::<[usize; 2]>::new();

    for contour in missing_shape {
        let mut contour_indices = Vec::new();
        for point in contour {
            let Some(global_index) = insert_missing_owner_vertex(
                node_id,
                region_index,
                arrangement,
                region,
                source_triangles,
                *point,
                owner,
                height_field_id,
                vertices,
                vertex_lookup,
            )?
            else {
                continue;
            };
            let local_index = *local_by_global.entry(global_index).or_insert_with(|| {
                let index = local_to_global.len();
                local_to_global.push(global_index);
                index
            });
            if contour_indices.last().copied() != Some(local_index) {
                contour_indices.push(local_index);
            }
        }
        if contour_indices.len() >= 2
            && contour_indices.first().copied() == contour_indices.last().copied()
        {
            contour_indices.pop();
        }
        if contour_indices.len() < 3 {
            // Missing-coverage repair may not have enough height-supported vertices to form a
            // local CDT. Skip it and let final owner-vs-triangle coverage validation report any
            // remaining meaningful gap.
            continue;
        }
        for index in 0..contour_indices.len() {
            let global_edge = normalized_vertex_edge(
                local_to_global[contour_indices[index]],
                local_to_global[contour_indices[(index + 1) % contour_indices.len()]],
            );
            let edge = normalized_vertex_edge(
                contour_indices[index],
                contour_indices[(index + 1) % contour_indices.len()],
            );
            if edge[0] != edge[1] {
                constraints.insert(edge);
            }
            if global_edge[0] != global_edge[1] {
                boundary_constraints.insert(global_edge);
            }
        }
    }

    if local_to_global.len() < 3 || constraints.is_empty() {
        return Ok(());
    }

    let spade_vertices = local_to_global
        .iter()
        .map(|global_index| {
            let point = vertices[*global_index].point_world;
            Point2::new(point.x, point.z)
        })
        .collect::<Vec<_>>();
    let constraint_list = constraints.into_iter().collect::<Vec<_>>();
    let mut invalid_constraints = 0usize;
    let mut first_invalid_constraint = None;
    let cdt =
        SurfaceCdt::try_bulk_load_cdt(spade_vertices, constraint_list.clone(), |constraint| {
            invalid_constraints += 1;
            first_invalid_constraint
                .get_or_insert(normalized_vertex_edge(constraint[0], constraint[1]));
        })
        .map_err(|_| NodeTriangulationError::CdtBuildFailed {
            node_id,
            region_index,
        })?;
    if invalid_constraints > 0 {
        return Err(NodeTriangulationError::InvalidConstraint {
            node_id,
            region_index,
            constraint_count: invalid_constraints,
            first_constraint_index: first_invalid_constraint
                .and_then(|constraint| constraint_list.iter().position(|edge| *edge == constraint)),
            first_constraint: first_invalid_constraint,
        });
    }

    for face in cdt.inner_faces() {
        let [a, b, c] = face.vertices();
        let triangle = NodeTriangulatedTriangle {
            vertices: [
                local_to_global[a.fix().index()],
                local_to_global[b.fix().index()],
                local_to_global[c.fix().index()],
            ],
        };
        if triangle_area_is_numeric_dust(&triangle, vertices) {
            continue;
        }
        if triangle_is_inside_owner(node_id, region_index, &triangle, vertices, missing_shape)? {
            triangles.push(triangle);
        }
    }

    Ok(())
}

fn insert_clipped_triangle_vertex(
    node_id: u32,
    region_index: usize,
    source_triangle: &NodeTriangulatedTriangle,
    point: NodeOverlayPoint,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    vertices: &mut Vec<NodeTriangulatedVertex>,
    vertex_lookup: &mut BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
) -> Result<usize, NodeTriangulationError> {
    let height_m = interpolate_source_triangle_height(source_triangle, vertices, point);
    let source_point_world = RoadVec3::new(f64::from(point[0]), height_m, f64::from(point[1]));
    let point_key = NodeTriangulationPointKey::from_world(source_point_world);
    let height_key = NodeTriangulationHeightKey(quantize_m(height_m));
    if let Some((index, existing_height_key)) = vertex_lookup.get(&point_key).copied() {
        if existing_height_key != height_key {
            return Err(NodeTriangulationError::DuplicateVertexHeightConflict {
                node_id,
                region_index,
                x_mm: point_key.x_mm,
                z_mm: point_key.z_mm,
                existing_height_mm: existing_height_key.0,
                incoming_height_mm: height_key.0,
            });
        }
        return Ok(index);
    }

    let point_xz = point_key.road_xz();
    let grade_authority = NodeGradeVertexAuthority::new(
        point_xz,
        height_m,
        owner,
        height_field_id,
        NodeGradeCarrierDecision::SameOwnerCanonicalVertex,
    );
    if let Some(index) = same_authority_numeric_dust_vertex(
        point_key,
        height_key,
        grade_authority,
        vertices,
        vertex_lookup,
    ) {
        return Ok(index);
    }
    let index = vertices.len();
    let point_world = RoadVec3::new(point_xz.x, height_m, point_xz.y);
    vertices.push(NodeTriangulatedVertex {
        point_world,
        height_field_id,
        grade_authority,
    });
    vertex_lookup.insert(point_key, (index, height_key));
    Ok(index)
}

fn insert_missing_owner_vertex(
    node_id: u32,
    region_index: usize,
    arrangement: &NodeArrangement,
    region: &NodeOwnedRegion,
    source_triangles: &[NodeTriangulatedTriangle],
    point: NodeOverlayPoint,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    vertices: &mut Vec<NodeTriangulatedVertex>,
    vertex_lookup: &mut BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
) -> Result<Option<usize>, NodeTriangulationError> {
    let point_key = SurfaceXzKey::from_overlay_point(point);
    let point_xz = point_key.to_road_xz();
    let point_world = RoadVec3::new(point_xz.x, 0.0, point_xz.y);
    let triangulation_key = NodeTriangulationPointKey::from_world(point_world);
    if let Some((index, _)) = vertex_lookup.get(&triangulation_key).copied() {
        return Ok(Some(index));
    }
    let Some((height_m, grade_authority)) = missing_owner_vertex_height_authority(
        arrangement,
        region,
        source_triangles,
        vertices,
        point,
        point_key,
        owner,
        height_field_id,
    ) else {
        return Ok(None);
    };
    let height_key = NodeTriangulationHeightKey(quantize_m(height_m));
    if let Some((index, existing_height_key)) = vertex_lookup.get(&triangulation_key).copied() {
        if existing_height_key != height_key {
            return Err(NodeTriangulationError::DuplicateVertexHeightConflict {
                node_id,
                region_index,
                x_mm: triangulation_key.x_mm,
                z_mm: triangulation_key.z_mm,
                existing_height_mm: existing_height_key.0,
                incoming_height_mm: height_key.0,
            });
        }
        return Ok(Some(index));
    }
    if let Some(index) = same_authority_numeric_dust_vertex(
        triangulation_key,
        height_key,
        grade_authority,
        vertices,
        vertex_lookup,
    ) {
        return Ok(Some(index));
    }
    let index = vertices.len();
    vertices.push(NodeTriangulatedVertex {
        point_world: RoadVec3::new(point_xz.x, height_m, point_xz.y),
        height_field_id,
        grade_authority,
    });
    vertex_lookup.insert(triangulation_key, (index, height_key));
    Ok(Some(index))
}

fn missing_owner_vertex_height_authority(
    arrangement: &NodeArrangement,
    region: &NodeOwnedRegion,
    source_triangles: &[NodeTriangulatedTriangle],
    vertices: &[NodeTriangulatedVertex],
    point: NodeOverlayPoint,
    point_key: SurfaceXzKey,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
) -> Option<(f64, NodeGradeVertexAuthority)> {
    height_authority_from_region_boundary(arrangement, region, point_key, owner, height_field_id)
        .or_else(|| {
            height_authority_from_source_triangles(
                source_triangles,
                vertices,
                point,
                point_key,
                owner,
                height_field_id,
            )
        })
        .or_else(|| {
            constant_region_height_authority(arrangement, region, point_key, owner, height_field_id)
        })
}

fn height_authority_from_region_boundary(
    arrangement: &NodeArrangement,
    region: &NodeOwnedRegion,
    point_key: SurfaceXzKey,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
) -> Option<(f64, NodeGradeVertexAuthority)> {
    std::iter::once(region.outer_loop())
        .chain(region.holes().iter().map(Vec::as_slice))
        .find_map(|contour| {
            for index in 0..contour.len() {
                let start = arrangement.vertices().get(contour[index].index())?;
                let end = arrangement
                    .vertices()
                    .get(contour[(index + 1) % contour.len()].index())?;
                let start_key =
                    SurfaceXzKey::from_raw_keys(start.key().x_key(), start.key().z_key());
                let end_key = SurfaceXzKey::from_raw_keys(end.key().x_key(), end.key().z_key());
                let parameter = segments::overlay_segment_parameter(point_key, start_key, end_key)?;
                let height_mm =
                    segments::interpolate_height_i64(start.height_mm(), end.height_mm(), parameter);
                let height_m = height_mm as f64 / 1000.0;
                let point_xz = point_key.to_road_xz();
                return Some((
                    height_m,
                    NodeGradeVertexAuthority::new(
                        point_xz,
                        height_m,
                        owner,
                        height_field_id,
                        NodeGradeCarrierDecision::SameOwnerCanonicalVertex,
                    ),
                ));
            }
            None
        })
}

fn height_authority_from_source_triangles(
    source_triangles: &[NodeTriangulatedTriangle],
    vertices: &[NodeTriangulatedVertex],
    point: NodeOverlayPoint,
    point_key: SurfaceXzKey,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
) -> Option<(f64, NodeGradeVertexAuthority)> {
    source_triangles.iter().find_map(|triangle| {
        point_lies_in_triangle_xz(*triangle, vertices, point).then(|| {
            let height_m = interpolate_source_triangle_height(triangle, vertices, point);
            let point_xz = point_key.to_road_xz();
            (
                height_m,
                NodeGradeVertexAuthority::new(
                    point_xz,
                    height_m,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SameOwnerCanonicalVertex,
                ),
            )
        })
    })
}

fn constant_region_height_authority(
    arrangement: &NodeArrangement,
    region: &NodeOwnedRegion,
    point_key: SurfaceXzKey,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
) -> Option<(f64, NodeGradeVertexAuthority)> {
    let mut heights = BTreeSet::new();
    for vertex_id in region
        .outer_loop()
        .iter()
        .chain(region.holes().iter().flatten())
    {
        heights.insert(arrangement.vertices().get(vertex_id.index())?.height_mm());
        if heights.len() > 1 {
            return None;
        }
    }
    let height_mm = heights.into_iter().next()?;
    let height_m = height_mm as f64 / 1000.0;
    let point_xz = point_key.to_road_xz();
    Some((
        height_m,
        NodeGradeVertexAuthority::new(
            point_xz,
            height_m,
            owner,
            height_field_id,
            NodeGradeCarrierDecision::SameOwnerCanonicalVertex,
        ),
    ))
}

fn point_lies_in_triangle_xz(
    triangle: NodeTriangulatedTriangle,
    vertices: &[NodeTriangulatedVertex],
    point: NodeOverlayPoint,
) -> bool {
    let a = vertices[triangle.vertices[0]].point_world;
    let b = vertices[triangle.vertices[1]].point_world;
    let c = vertices[triangle.vertices[2]].point_world;
    let px = point[0];
    let pz = point[1];
    let ab = (b.x - a.x) * (pz - a.z) - (b.z - a.z) * (px - a.x);
    let bc = (c.x - b.x) * (pz - b.z) - (c.z - b.z) * (px - b.x);
    let ca = (a.x - c.x) * (pz - c.z) - (a.z - c.z) * (px - c.x);
    let epsilon = 1.0e-7;
    let has_neg = ab < -epsilon || bc < -epsilon || ca < -epsilon;
    let has_pos = ab > epsilon || bc > epsilon || ca > epsilon;
    !(has_neg && has_pos)
}

fn interpolate_source_triangle_height(
    triangle: &NodeTriangulatedTriangle,
    vertices: &[NodeTriangulatedVertex],
    point: NodeOverlayPoint,
) -> f64 {
    let a = vertices[triangle.vertices[0]].point_world;
    let b = vertices[triangle.vertices[1]].point_world;
    let c = vertices[triangle.vertices[2]].point_world;
    let px = f64::from(point[0]);
    let pz = f64::from(point[1]);
    let denominator = (b.z - c.z) * (a.x - c.x) + (c.x - b.x) * (a.z - c.z);
    if denominator.abs() <= f64::EPSILON {
        return (a.y + b.y + c.y) / 3.0;
    }
    let weight_a = ((b.z - c.z) * (px - c.x) + (c.x - b.x) * (pz - c.z)) / denominator;
    let weight_b = ((c.z - a.z) * (px - c.x) + (a.x - c.x) * (pz - c.z)) / denominator;
    let weight_c = 1.0 - weight_a - weight_b;
    a.y * weight_a + b.y * weight_b + c.y * weight_c
}

fn positive_triangle_contour(
    triangle: &NodeTriangulatedTriangle,
    vertices: &[NodeTriangulatedVertex],
) -> NodeOverlayContour {
    let mut contour = triangle
        .vertices
        .iter()
        .map(|index| {
            let point = vertices[*index].point_world;
            [point.x, point.z]
        })
        .collect::<Vec<_>>();
    if RoadSurfaceSystem::overlay_contour_area(&contour) < 0.0 {
        contour.swap(1, 2);
    }
    contour
}
