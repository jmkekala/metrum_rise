// SPDX-License-Identifier: GPL-2.0-only

//! Compiled node-piece lookup helpers.

use super::*;

pub(in crate::simulation::network::surface::tests) fn assert_compiled_bend_piece<'a>(
    surface: &'a RoadSurfaceSystem,
    graph: &RegionGraph,
    bend: u32,
) -> &'a RoadSurfaceVisualNodePiece {
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&bend)
        .unwrap_or_else(|| {
            panic!(
                "bend should compile through canonical owned regions: {}",
                canonical_node_pipeline_report(
                    surface,
                    graph,
                    bend,
                    RoadSurfaceVisualNodePieceKind::Bend
                )
            )
        });
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    assert_material_triangles_do_not_overlap(piece);
    assert!(!piece.outer_boundary_loops.is_empty());
    assert!(
        !piece.render_earthwork_faces.is_empty(),
        "bend piece must emit terrain skirt faces from its canonical outer boundary"
    );
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.curb_surface_polygons.is_empty());
    assert!(!piece.raised_step_face_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_top_mesh_centroids_inside_outer_boundary(piece);
    assert_top_surface_triangles_face_up(piece);
    assert_raised_step_faces_have_top_support(piece);
    assert_no_duplicate_raised_step_render_faces(piece);
    assert_raised_step_faces_visible_from_lower_owner(piece);
    assert_top_raised_step_owner_boundaries_have_vertical_faces(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);
    assert_node_top_covers_footprint(piece);
    assert_node_earthwork_faces_have_footprint_provenance(piece);
    assert_earthwork_faces_stay_outside_top_footprint(piece);
    piece
}

pub(in crate::simulation::network::surface::tests) fn assert_compiled_junction_piece<'a>(
    surface: &'a RoadSurfaceSystem,
    graph: &RegionGraph,
    junction: u32,
) -> &'a RoadSurfaceVisualNodePiece {
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&junction)
        .unwrap_or_else(|| {
            panic!(
                "junction should compile through canonical owned regions: {}",
                canonical_junction_pipeline_report(surface, graph, junction)
            )
        });
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    assert_material_triangles_do_not_overlap(piece);
    assert!(!piece.outer_boundary_loops.is_empty());
    assert!(
        !piece.render_earthwork_faces.is_empty(),
        "junction piece must emit terrain skirt faces from its canonical outer boundary"
    );
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.curb_surface_polygons.is_empty());
    assert!(!piece.raised_step_face_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_top_mesh_centroids_inside_outer_boundary(piece);
    assert_top_surface_triangles_face_up(piece);
    assert_raised_step_faces_have_top_support(piece);
    assert_no_duplicate_raised_step_render_faces(piece);
    assert_raised_step_faces_visible_from_lower_owner(piece);
    assert_top_raised_step_owner_boundaries_have_vertical_faces(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);
    assert_node_top_covers_footprint(piece);
    assert_node_earthwork_faces_have_footprint_provenance(piece);
    assert_earthwork_faces_stay_outside_top_footprint(piece);
    piece
}
