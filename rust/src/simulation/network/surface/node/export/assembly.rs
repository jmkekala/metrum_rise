//! Final explicit node-piece assembly from exported surface regions.

use super::super::*;
use crate::simulation::network::surface::RoadSurfaceTriangleQueryIndex;
use std::sync::Arc;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::node) fn assemble_explicit_node_piece(
        &self,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
        mut terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
        mut road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut raised_step_faces: Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
        mut sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        explicit_vertical_step_segments: Vec<NodeExplicitVerticalStepSegment>,
        node_grade_authorities: Vec<height::NodeGradeVertexAuthority>,
        mut node_top_surface_sources: Vec<NodeTopSurfacePolygonSource>,
        mut owned_regions: Vec<NodeOwnedRegion>,
        boolean_debug: Option<NodeBooleanDebugSnapshot>,
        mut earthwork_owner_sources: Vec<NodeEarthworkOwnerSource>,
        mut earthwork_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut earthwork_outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
        mut render_earthwork_faces: Vec<RoadSurfaceEarthworkRenderFace>,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if road_surface_polygons.is_empty()
            && curb_surface_polygons.is_empty()
            && sidewalk_surface_polygons.is_empty()
        {
            return None;
        }
        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut curb_surface_polygons);
        Self::sort_raised_step_faces(&mut raised_step_faces);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        if node_top_surface_sources.len() != owned_regions.len() {
            return None;
        }
        Self::sort_node_owned_regions_with_sources(
            &mut owned_regions,
            &mut node_top_surface_sources,
        )
        .ok()?;
        for source_edge in terrain_clip_boundary_loops
            .iter_mut()
            .flat_map(|boundary_loop| &mut boundary_loop.source_edges)
        {
            source_edge.source = source_edge.source.with_node_identity(node_id, kind);
        }
        for face in &mut render_earthwork_faces {
            face.source = face.source.with_node_identity(node_id, kind);
        }
        Self::sort_terrain_clip_loops(&mut terrain_clip_boundary_loops);
        Self::sort_visual_polygons(&mut earthwork_surface_polygons);
        Self::sort_visual_polygons(&mut earthwork_outer_boundary_loops);
        Self::sort_earthwork_render_faces(&mut render_earthwork_faces);
        earthwork_owner_sources.sort_by(|a, b| {
            a.owner_kind
                .cmp(&b.owner_kind)
                .then(a.owner_index.cmp(&b.owner_index))
                .then(a.mouth_order_index.cmp(&b.mouth_order_index))
                .then(a.edge_idx.cmp(&b.edge_idx))
        });
        earthwork_owner_sources.dedup_by(|a, b| {
            a.owner_kind == b.owner_kind
                && a.owner_index == b.owner_index
                && a.mouth_order_index == b.mouth_order_index
                && a.edge_idx == b.edge_idx
                && a.edge_class == b.edge_class
        });
        if outer_boundary_loops.is_empty() {
            return None;
        }
        let surface_query = Arc::new(RoadSurfaceTriangleQueryIndex::from_surface_polygons(
            &road_surface_polygons,
            &curb_surface_polygons,
            &sidewalk_surface_polygons,
        ));
        let (raised_step_face_polygons, raised_step_face_sources) =
            raised_step_faces.into_iter().unzip();
        Some(RoadSurfaceVisualNodePiece {
            node_id,
            kind,
            outer_boundary_loops,
            terrain_clip_boundary_loops,
            road_surface_polygons,
            curb_surface_polygons,
            raised_step_face_polygons,
            raised_step_face_sources,
            sidewalk_surface_polygons,
            surface_query,
            explicit_vertical_step_segments,
            node_grade_authorities,
            node_top_surface_sources,
            owned_regions,
            boolean_debug,
            earthwork_owner_sources,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        })
    }
}
