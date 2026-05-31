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

    let indices = contour
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
