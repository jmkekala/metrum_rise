// SPDX-License-Identifier: GPL-2.0-only

//! Canonical ordering and key normalization for CDT provenance and edges.

use super::super::*;

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_road_constraint_source_cmp(
    a: TerrainCdtRoadConstraintSource,
    b: TerrainCdtRoadConstraintSource,
) -> std::cmp::Ordering {
    a.stable_piece_id
        .cmp(&b.stable_piece_id)
        .then(a.local_loop_index.cmp(&b.local_loop_index))
        .then(a.local_edge_index.cmp(&b.local_edge_index))
        .then_with(|| terrain_cdt_boundary_source_cmp(a.boundary_source, b.boundary_source))
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_optional_boundary_source_cmp(
    a: Option<TerrainCdtRoadBoundarySource>,
    b: Option<TerrainCdtRoadBoundarySource>,
) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => terrain_cdt_boundary_source_cmp(a, b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_boundary_sources_cmp(
    a: &[TerrainCdtRoadBoundarySource],
    b: &[TerrainCdtRoadBoundarySource],
) -> std::cmp::Ordering {
    for (source_a, source_b) in a.iter().copied().zip(b.iter().copied()) {
        let ordering = terrain_cdt_boundary_source_cmp(source_a, source_b);
        if !ordering.is_eq() {
            return ordering;
        }
    }
    a.len().cmp(&b.len())
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_boundary_source_cmp(
    a: TerrainCdtRoadBoundarySource,
    b: TerrainCdtRoadBoundarySource,
) -> std::cmp::Ordering {
    a.cmp(&b)
}

pub(in crate::simulation::terrain::cdt) fn normalize_edge(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

pub(in crate::simulation::terrain::cdt) fn normalize_edge_array(a: usize, b: usize) -> [usize; 2] {
    if a < b { [a, b] } else { [b, a] }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_vertex_xz_key(
    vertex: TerrainCdtVertex,
) -> (i64, i64) {
    (quantized_coord(vertex.x), quantized_coord(vertex.z))
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_vertex_key(
    vertex: TerrainCdtVertex,
) -> (i64, i64, i64) {
    let (x_key, z_key) = terrain_cdt_vertex_xz_key(vertex);
    (x_key, z_key, quantized_coord(f64::from(vertex.height_m)))
}
