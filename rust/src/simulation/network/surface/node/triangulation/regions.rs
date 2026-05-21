//! Triangulation of individual node-owned regions.

use super::coverage::{
    overlay_shape_from_arrangement_region, reject_triangle_coverage_mismatch,
    triangle_double_area_m2, triangle_is_inside_owner, triangle_sort_key,
};
use super::vertices::push_arrangement_constraint_loop;
use super::*;

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
        }
    }
    triangles.sort_by(|a, b| triangle_sort_key(a, &vertices).cmp(&triangle_sort_key(b, &vertices)));
    triangles.dedup();
    if triangles.is_empty() {
        return Err(NodeTriangulationError::EmptyTriangulation {
            node_id,
            region_index,
        });
    }

    reject_triangle_coverage_mismatch(node_id, region_index, &owner_shape, &triangles, &vertices)?;

    let owner = region.owner();
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
