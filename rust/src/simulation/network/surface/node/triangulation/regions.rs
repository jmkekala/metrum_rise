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
    let spade_vertices = vertices
        .iter()
        .map(|vertex| Point2::new(vertex.point_world.x, vertex.point_world.z))
        .collect::<Vec<_>>();
    let mut invalid_constraints = 0usize;
    let cdt = SurfaceCdt::try_bulk_load_cdt(
        spade_vertices,
        constraints.iter().copied().collect(),
        |_| invalid_constraints += 1,
    )
    .map_err(|_| NodeTriangulationError::CdtBuildFailed {
        node_id,
        region_index,
    })?;
    if invalid_constraints > 0 {
        return Err(NodeTriangulationError::InvalidConstraint {
            node_id,
            region_index,
            constraint_count: invalid_constraints,
        });
    }

    let owner_shape = overlay_shape_from_arrangement_region(arrangement, region);
    let owner = region.owner();
    let mut triangles = Vec::new();
    for face in cdt.inner_faces() {
        let [a, b, c] = face.vertices();
        let triangle = NodeTriangulatedTriangle {
            vertices: [a.fix().index(), b.fix().index(), c.fix().index()],
        };
        if triangle_double_area_m2(&triangle, &vertices) <= f64::from(NODE_OVERLAY_MIN_AREA_M2) {
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
                region.height_field_id(),
                &mut vertices,
                &mut vertex_lookup,
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
        region.height_field_id(),
        &mut vertices,
        &mut vertex_lookup,
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

    Ok(NodeTriangulatedRegion {
        kind: owner.kind(),
        owner,
        height_field_id: region.height_field_id(),
        vertices,
        boundary_constraints: constraints.into_iter().collect(),
        triangles,
        area_m2: region.area_m2(),
    })
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
    triangles: &mut Vec<NodeTriangulatedTriangle>,
) -> Result<(), NodeTriangulationError> {
    let mut local_to_global = Vec::<usize>::new();
    let mut local_by_global = BTreeMap::<usize, usize>::new();
    let mut constraints = BTreeSet::<[usize; 2]>::new();

    for (contour_index, contour) in clipped_shape.iter().enumerate() {
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
            if RoadSurfaceSystem::overlay_contour_area(contour).abs() > NODE_OVERLAY_MIN_AREA_M2 {
                return Err(NodeTriangulationError::DegenerateRegionContour {
                    node_id,
                    region_index,
                    contour_index,
                    vertex_count: contour_indices.len(),
                });
            }
            continue;
        }
        for index in 0..contour_indices.len() {
            let edge = normalized_vertex_edge(
                contour_indices[index],
                contour_indices[(index + 1) % contour_indices.len()],
            );
            if edge[0] != edge[1] {
                constraints.insert(edge);
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
    let mut invalid_constraints = 0usize;
    let cdt =
        SurfaceCdt::try_bulk_load_cdt(spade_vertices, constraints.into_iter().collect(), |_| {
            invalid_constraints += 1
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
        if triangle_double_area_m2(&triangle, vertices) <= f64::from(NODE_OVERLAY_MIN_AREA_M2) {
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
    triangles: &mut Vec<NodeTriangulatedTriangle>,
) -> Result<(), NodeTriangulationError> {
    let mut local_to_global = Vec::<usize>::new();
    let mut local_by_global = BTreeMap::<usize, usize>::new();
    let mut constraints = BTreeSet::<[usize; 2]>::new();

    for (contour_index, contour) in missing_shape.iter().enumerate() {
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
            if RoadSurfaceSystem::overlay_contour_area(contour).abs() > NODE_OVERLAY_MIN_AREA_M2 {
                return Err(NodeTriangulationError::DegenerateRegionContour {
                    node_id,
                    region_index,
                    contour_index,
                    vertex_count: contour_indices.len(),
                });
            }
            continue;
        }
        for index in 0..contour_indices.len() {
            let edge = normalized_vertex_edge(
                contour_indices[index],
                contour_indices[(index + 1) % contour_indices.len()],
            );
            if edge[0] != edge[1] {
                constraints.insert(edge);
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
    let mut invalid_constraints = 0usize;
    let cdt =
        SurfaceCdt::try_bulk_load_cdt(spade_vertices, constraints.into_iter().collect(), |_| {
            invalid_constraints += 1
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
        if triangle_double_area_m2(&triangle, vertices) <= f64::from(NODE_OVERLAY_MIN_AREA_M2) {
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

    let index = vertices.len();
    let point_xz = point_key.road_xz();
    let point_world = RoadVec3::new(point_xz.x, height_m, point_xz.y);
    vertices.push(NodeTriangulatedVertex {
        point_world,
        height_field_id,
        grade_authority: NodeGradeVertexAuthority::new(
            point_xz,
            height_m,
            owner,
            height_field_id,
            NodeGradeCarrierDecision::SameOwnerCanonicalVertex,
        ),
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
