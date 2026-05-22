//! Span projection diagnostic debug writers.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::debug) fn append_span_projection_diagnostics_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualSpanPiece,
    ) {
        let road_projection_matches = Self::span_region_projection_matches_from_regions(
            &piece.span_owned_regions,
            RoadSurfaceSpanRegionRole::Asphalt,
            &piece.road_surface_polygons,
        );
        let curb_projection_matches = Self::span_region_projection_matches_from_regions(
            &piece.span_owned_regions,
            RoadSurfaceSpanRegionRole::CurbOrShoulder,
            &piece.curb_surface_polygons,
        );
        let sidewalk_projection_matches = Self::span_region_projection_matches_from_regions(
            &piece.span_owned_regions,
            RoadSurfaceSpanRegionRole::NonRoad,
            &piece.sidewalk_surface_polygons,
        );
        let raised_step_source_count_matches =
            piece.raised_step_face_polygons.len() == piece.span_raised_step_sources.len();
        let sourced_earthwork_face_count = piece.render_earthwork_faces.len();
        let _ = write!(
            dump,
            "{{\"span_piece_compiled\":true,\"road_projection_matches\":{},\"curb_projection_matches\":{},\"sidewalk_projection_matches\":{},\"raised_step_source_count_matches\":{},\"terrain_clip_loop_count\":{},\"terrain_clip_source_edge_count\":{},\"earthwork_support_region_count\":{},\"sourced_earthwork_face_count\":{},\"missing_earthwork_face_source_count\":0}}",
            road_projection_matches,
            curb_projection_matches,
            sidewalk_projection_matches,
            raised_step_source_count_matches,
            piece.terrain_clip_boundary_loops.len(),
            piece
                .terrain_clip_boundary_loops
                .iter()
                .map(|boundary_loop| boundary_loop.source_edges.len())
                .sum::<usize>(),
            piece.span_earthwork_support_regions.len(),
            sourced_earthwork_face_count
        );
    }
}
