// SPDX-License-Identifier: GPL-2.0-only

//! Arrangement vertex insertion and CDT constraint collection.

use super::*;

pub(super) fn push_arrangement_constraint_loop(
    node_id: u32,
    region_index: usize,
    contour_index: usize,
    contour: &[NodeArrangementVertexId],
    arrangement: &NodeArrangement,
    vertices: &mut Vec<NodeTriangulatedVertex>,
    vertex_lookup: &mut BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
    constraints: &mut BTreeSet<[usize; 2]>,
) -> Result<(), NodeTriangulationError> {
    if contour.len() < 3 {
        return Err(NodeTriangulationError::DegenerateRegionContour {
            node_id,
            region_index,
            contour_index,
            vertex_count: contour.len(),
        });
    }

    let mut indices = contour
        .iter()
        .map(|vertex_id| {
            let vertex = arrangement.vertices().get(vertex_id.index()).ok_or(
                NodeTriangulationError::DegenerateRegionContour {
                    node_id,
                    region_index,
                    contour_index,
                    vertex_count: contour.len(),
                },
            )?;
            insert_arrangement_vertex(node_id, region_index, vertex, vertices, vertex_lookup)
        })
        .collect::<Result<Vec<_>, _>>()?;
    clean_triangulation_constraint_loop(&mut indices, vertices);
    if indices.len() < 3 {
        return Err(NodeTriangulationError::DegenerateRegionContour {
            node_id,
            region_index,
            contour_index,
            vertex_count: indices.len(),
        });
    }
    for index in 0..indices.len() {
        push_constraint(
            indices[index],
            indices[(index + 1) % indices.len()],
            constraints,
        );
    }
    Ok(())
}

fn push_constraint(start: usize, end: usize, constraints: &mut BTreeSet<[usize; 2]>) {
    let constraint = normalized_vertex_edge(start, end);
    if constraint[0] != constraint[1] {
        constraints.insert(constraint);
    }
}

fn clean_triangulation_constraint_loop(
    indices: &mut Vec<usize>,
    vertices: &[NodeTriangulatedVertex],
) {
    loop {
        let starting_len = indices.len();
        remove_immediate_backtracking_index(indices, vertices);
        remove_consecutive_equivalent_indices(indices, vertices);
        if indices.len() == starting_len || indices.len() < 3 {
            break;
        }
    }
}

fn remove_consecutive_equivalent_indices(
    indices: &mut Vec<usize>,
    vertices: &[NodeTriangulatedVertex],
) {
    let dust_key_units = node_triangulation_dust_key_units();
    indices.dedup_by(|left, right| {
        constraint_loop_vertices_are_equivalent(*left, *right, vertices, dust_key_units)
    });
    if indices.len() >= 2
        && constraint_loop_vertices_are_equivalent(
            indices[0],
            *indices.last().expect("constraint loop has last vertex"),
            vertices,
            dust_key_units,
        )
    {
        indices.pop();
    }
}

fn remove_immediate_backtracking_index(
    indices: &mut Vec<usize>,
    vertices: &[NodeTriangulatedVertex],
) {
    if indices.len() < 3 {
        return;
    }
    let Some(index) = (0..indices.len()).find(|index| {
        let previous = indices[(*index + indices.len() - 1) % indices.len()];
        let current = indices[*index];
        let next = indices[(*index + 1) % indices.len()];
        constraint_loop_vertices_are_equivalent(
            previous,
            next,
            vertices,
            NODE_TRIANGULATION_CONSTRAINT_LOOP_DUST_KEY_UNITS,
        ) && constraint_loop_spur_area_is_numeric_dust(previous, current, next, vertices)
    }) else {
        return;
    };
    let next_index = (index + 1) % indices.len();
    indices.remove(index);
    if indices.len() > 3 {
        let duplicate_return_index = if next_index > index { index } else { 0 };
        if duplicate_return_index < indices.len() {
            indices.remove(duplicate_return_index);
        }
    }
}

fn constraint_loop_vertices_are_equivalent(
    left: usize,
    right: usize,
    vertices: &[NodeTriangulatedVertex],
    dust_key_units: i64,
) -> bool {
    if left == right {
        return true;
    }
    let (Some(left), Some(right)) = (vertices.get(left), vertices.get(right)) else {
        return false;
    };
    if left.height_field_id != right.height_field_id
        || quantize_m(left.point_world.y) != quantize_m(right.point_world.y)
        || left.grade_authority.owner != right.grade_authority.owner
        || left.grade_authority.height_field_id != right.grade_authority.height_field_id
        || left.grade_authority.height_key != right.grade_authority.height_key
    {
        return false;
    }
    let left_key = NodeTriangulationPointKey::from_world(left.point_world);
    let right_key = NodeTriangulationPointKey::from_world(right.point_world);
    let dust_sq = i128::from(dust_key_units) * i128::from(dust_key_units);
    left_key.distance_key_units_sq(right_key) <= dust_sq
}

fn constraint_loop_spur_area_is_numeric_dust(
    previous: usize,
    current: usize,
    next: usize,
    vertices: &[NodeTriangulatedVertex],
) -> bool {
    let (Some(previous), Some(current), Some(next)) = (
        vertices.get(previous),
        vertices.get(current),
        vertices.get(next),
    ) else {
        return false;
    };
    let area_m2 = RoadSurfaceSystem::road_triangle_double_area_xz_m2([
        previous.point_world,
        current.point_world,
        next.point_world,
    ])
    .abs()
        * 0.5;
    if area_m2 <= f64::from(NODE_OVERLAY_MIN_AREA_M2) {
        return true;
    }

    area_m2 <= constraint_loop_backtrack_area_budget_m2(previous, current, next)
}

fn constraint_loop_backtrack_area_budget_m2(
    previous: &NodeTriangulatedVertex,
    current: &NodeTriangulatedVertex,
    next: &NodeTriangulatedVertex,
) -> f64 {
    let dust_width_m =
        NODE_TRIANGULATION_CONSTRAINT_LOOP_DUST_KEY_UNITS as f64 / SURFACE_XZ_KEY_SCALE as f64;
    let previous_length_m = xz_distance_m(previous.point_world, current.point_world);
    let next_length_m = xz_distance_m(current.point_world, next.point_world);
    previous_length_m.max(next_length_m) * dust_width_m
}

fn xz_distance_m(a: RoadVec3, b: RoadVec3) -> f64 {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    dx.hypot(dz)
}

fn insert_arrangement_vertex(
    node_id: u32,
    region_index: usize,
    vertex: &NodeArrangementVertex,
    vertices: &mut Vec<NodeTriangulatedVertex>,
    vertex_lookup: &mut BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
) -> Result<usize, NodeTriangulationError> {
    let point_key = NodeTriangulationPointKey::from_arrangement_vertex(vertex);
    let height_key = NodeTriangulationHeightKey::from_arrangement_vertex(vertex);
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
    let grade_authority = vertex.grade_authority();
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
    let point_xz = point_key.road_xz();
    vertices.push(NodeTriangulatedVertex {
        point_world: RoadVec3::new(point_xz.x, vertex.height_m(), point_xz.y),
        height_field_id: vertex.height_field_id(),
        grade_authority,
    });
    vertex_lookup.insert(point_key, (index, height_key));
    Ok(index)
}
