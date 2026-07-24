//! CDT orchestration from canonical input to classified output mesh.

use std::collections::HashSet;

use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

use super::*;

type SpadeCdt = ConstrainedDelaunayTriangulation<Point2<f64>>;

pub(crate) fn build_road_touched_terrain_patch(
    input: TerrainCdtInput,
) -> Result<TerrainCdtMesh, TerrainCdtError> {
    if !input.patch.is_valid() {
        return Err(TerrainCdtError::InvalidPatch);
    }

    let canonical = canonicalize_input(input)?;
    let spade_vertices = canonical
        .vertices
        .iter()
        .map(|vertex| vertex.point2())
        .collect::<Vec<_>>();
    let mut invalid_constraint_edges = canonical.invalid_constraint_edges;
    let mut invalid_constraint_samples = Vec::new();
    let cdt = SpadeCdt::try_bulk_load_cdt(spade_vertices, canonical.constraints.clone(), |edge| {
        invalid_constraint_edges += 1;
        insert_invalid_constraint_sample(
            &mut invalid_constraint_samples,
            normalize_edge_array(edge[0], edge[1]),
            &canonical.vertices,
            &canonical.road_constraint_sources,
        );
    })
    .map_err(|_| TerrainCdtError::TriangulationFailed)?;

    let mut triangles = Vec::new();
    let mut rejected_road_faces = 0usize;
    let mut all_inner_edges = HashSet::new();
    let mut rejected_face_edges = HashSet::new();
    for face in cdt.inner_faces() {
        let [a, b, c] = face.vertices();
        let triangle = [a.fix().index(), b.fix().index(), c.fix().index()];
        let face_edges = triangle_edges(&triangle);
        all_inner_edges.extend(face_edges);
        let points = [
            canonical.vertices[triangle[0]],
            canonical.vertices[triangle[1]],
            canonical.vertices[triangle[2]],
        ];
        if terrain_triangle_is_road_owned(
            triangle,
            points,
            &canonical.road_constraint_sources,
            &canonical.road_loops,
        ) {
            rejected_road_faces += 1;
            rejected_face_edges.extend(face_edges);
            continue;
        }
        triangles.push(triangle);
    }

    let mut accepted_edges = emitted_triangle_edges(&triangles);
    let mut preserved_road_constraint_edges = canonical
        .road_constraint_edges
        .iter()
        .filter(|edge| accepted_edges.contains(&normalize_edge(edge[0], edge[1])))
        .count();
    let building_site_constraint_edges = canonical
        .road_constraint_edges
        .iter()
        .filter(|edge| terrain_cdt_constraint_edge_is_building_site(**edge, &canonical))
        .count();
    let mut preserved_building_site_constraint_edges = canonical
        .road_constraint_edges
        .iter()
        .filter(|edge| {
            terrain_cdt_constraint_edge_is_building_site(**edge, &canonical)
                && accepted_edges.contains(&normalize_edge(edge[0], edge[1]))
        })
        .count();
    let mut unpreserved_road_constraint_edges = canonical
        .road_constraint_edges
        .iter()
        .copied()
        .filter(|edge| !accepted_edges.contains(&normalize_edge(edge[0], edge[1])))
        .collect::<Vec<_>>();
    if !unpreserved_road_constraint_edges.is_empty() {
        let mut noncrossing_triangles = Vec::with_capacity(triangles.len());
        for triangle in triangles {
            let points = [
                canonical.vertices[triangle[0]],
                canonical.vertices[triangle[1]],
                canonical.vertices[triangle[2]],
            ];
            if triangle_crosses_any_road_constraint(
                points,
                &unpreserved_road_constraint_edges,
                &canonical.vertices,
            ) {
                rejected_road_faces += 1;
                rejected_face_edges.extend(triangle_edges(&triangle));
            } else {
                noncrossing_triangles.push(triangle);
            }
        }
        triangles = noncrossing_triangles;
        accepted_edges = emitted_triangle_edges(&triangles);
        preserved_road_constraint_edges = canonical
            .road_constraint_edges
            .iter()
            .filter(|edge| accepted_edges.contains(&normalize_edge(edge[0], edge[1])))
            .count();
        preserved_building_site_constraint_edges = canonical
            .road_constraint_edges
            .iter()
            .filter(|edge| {
                terrain_cdt_constraint_edge_is_building_site(**edge, &canonical)
                    && accepted_edges.contains(&normalize_edge(edge[0], edge[1]))
            })
            .count();
        unpreserved_road_constraint_edges = canonical
            .road_constraint_edges
            .iter()
            .copied()
            .filter(|edge| !accepted_edges.contains(&normalize_edge(edge[0], edge[1])))
            .collect();
    }
    let spade_missing_road_constraint_edges = canonical
        .road_constraint_edges
        .iter()
        .filter(|edge| !all_inner_edges.contains(&normalize_edge(edge[0], edge[1])))
        .count();
    let rejected_road_constraint_edges = canonical
        .road_constraint_edges
        .iter()
        .filter(|edge| {
            let edge = normalize_edge(edge[0], edge[1]);
            rejected_face_edges.contains(&edge) && !accepted_edges.contains(&edge)
        })
        .count();
    let unpreserved_road_constraint_samples = unpreserved_road_constraint_samples(
        &unpreserved_road_constraint_edges,
        &accepted_edges,
        &canonical.vertices,
        &canonical.road_constraint_sources,
    );
    let diagnostics = terrain_face_diagnostics(
        &canonical.vertices,
        &triangles,
        &canonical.road_constraint_sources,
        &canonical.retaining_wall_required_sources,
    );
    Ok(TerrainCdtMesh {
        stats: TerrainCdtStats {
            input_vertices: canonical.vertices.len(),
            constraint_edges: canonical.constraints.len(),
            road_constraint_edges: canonical.road_constraint_edges.len(),
            building_site_constraint_edges,
            accepted_faces: triangles.len(),
            rejected_road_faces,
            preserved_road_constraint_edges,
            preserved_building_site_constraint_edges,
            spade_missing_road_constraint_edges,
            rejected_road_constraint_edges,
            internal_road_constraint_edges: canonical.internal_road_constraint_edges,
            invalid_constraint_edges,
            max_face_y_delta_m: diagnostics.max_face_y_delta_m,
            max_face_slope_ratio: diagnostics.max_face_slope_ratio,
            longest_triangle_edge_m: diagnostics.longest_triangle_edge_m,
            road_seam_faces: diagnostics.road_seam_faces,
            road_seam_max_y_delta_m: diagnostics.road_seam_max_y_delta_m,
            road_seam_max_slope_ratio: diagnostics.road_seam_max_slope_ratio,
            retaining_wall_faces: diagnostics.retaining_wall_faces,
            retaining_wall_max_y_delta_m: diagnostics.retaining_wall_max_y_delta_m,
            retaining_wall_max_slope_ratio: diagnostics.retaining_wall_max_slope_ratio,
            accepted_seam_edges: canonical.accepted_seam_edges,
            merged_subbudget_seam_edges: canonical.merged_subbudget_seam_edges,
            retaining_wall_required_seam_edges: canonical.retaining_wall_required_seam_edges,
            retaining_wall_required_seam_faces: diagnostics.retaining_wall_faces,
            blocking_degenerate_seam_edges: canonical.blocking_degenerate_seam_edges,
            tie_in_widened_source_samples: canonical.tie_in_widened_source_samples,
            tie_in_widened_max_y_delta_m: canonical.tie_in_widened_max_y_delta_m,
            tie_in_widened_max_slope_ratio: canonical.tie_in_widened_max_slope_ratio,
        },
        vertices: canonical.vertices,
        #[cfg(test)]
        emitted_faces: diagnostics.emitted_faces,
        triangles: diagnostics.terrain_triangles,
        terrain_triangle_sources: diagnostics.terrain_triangle_sources,
        retaining_wall_triangles: diagnostics.retaining_wall_triangles,
        retaining_wall_triangle_sources: diagnostics.retaining_wall_triangle_sources,
        invalid_constraint_samples,
        road_seam_face_samples: diagnostics.road_seam_face_samples,
        retaining_wall_face_samples: diagnostics.retaining_wall_face_samples,
        tie_in_widened_samples: canonical.tie_in_widened_samples,
        seam_quality_samples: canonical.seam_quality_samples,
        unpreserved_road_constraint_samples,
    })
}

fn terrain_cdt_constraint_edge_is_building_site(
    edge: [usize; 2],
    canonical: &CanonicalTerrainCdtInput,
) -> bool {
    canonical
        .road_constraint_sources
        .get(&edge)
        .is_some_and(|source| {
            matches!(
                source.boundary_source,
                TerrainCdtRoadBoundarySource::BuildingSiteBoundary { .. }
            )
        })
}
