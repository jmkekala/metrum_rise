//! Visual node-piece assembly entry points.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn compile_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        let valid = graph.get_valid_node(node_id);
        let incidents = self.sorted_incident_surface_edges(graph, valid);
        match self.classify_visual_node_kind(&incidents) {
            CompiledNodeKind::Terminal => incidents.first().and_then(|incident| {
                self.build_terminal_visual_node_piece(graph, terrain, valid, *incident)
            }),
            CompiledNodeKind::PassThrough => None,
            CompiledNodeKind::Bend => {
                self.build_bend_visual_node_piece(graph, terrain, valid, &incidents)
            }
            CompiledNodeKind::JunctionN => {
                self.build_junction_visual_node_piece(graph, terrain, valid, &incidents)
            }
        }
    }

    fn build_terminal_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        incident: IncidentSurfaceEdge,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        let mouths = self.build_ordered_piece_mouths(&[incident])?;
        self.build_canonical_visual_node_piece(
            graph,
            terrain,
            node_id,
            RoadSurfaceVisualNodePieceKind::Terminal,
            &mouths,
        )
    }

    fn build_bend_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if incidents.len() != 2 {
            return None;
        }
        let mouths = self.build_ordered_piece_mouths(incidents)?;
        self.build_canonical_visual_node_piece(
            graph,
            terrain,
            node_id,
            RoadSurfaceVisualNodePieceKind::Bend,
            &mouths,
        )
    }

    fn build_junction_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if incidents.len() < 3 {
            return None;
        }
        if incidents
            .iter()
            .any(|incident| self.incident_edge_visual_handoff_is_overconstrained(graph, *incident))
        {
            return None;
        }
        let Some(mouths) = self.build_ordered_piece_mouths(incidents) else {
            return None;
        };
        self.build_canonical_visual_node_piece(
            graph,
            terrain,
            node_id,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &mouths,
        )
    }

    fn build_canonical_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        let node_regions = self.compile_canonical_node_surface_regions(node_id, kind, mouths)?;
        let top_surface_shapes = Self::top_surface_overlay_shapes(
            node_regions
                .road_surface_polygons
                .iter()
                .chain(node_regions.curb_surface_polygons.iter())
                .chain(node_regions.sidewalk_surface_polygons.iter()),
        );
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_segments(
                &node_regions.earthwork_boundary_segments,
                terrain,
                top_surface_shapes.as_ref(),
            )
            .ok()?;
        let earthwork_owner_sources = Self::node_earthwork_owner_sources_from_regions(
            graph,
            mouths,
            &node_regions.owned_regions,
            &node_regions.node_top_surface_sources,
        );

        self.assemble_explicit_node_piece(
            node_id,
            kind,
            node_regions.outer_boundary_loops,
            node_regions.terrain_clip_boundary_loops,
            node_regions.road_surface_polygons,
            node_regions.curb_surface_polygons,
            node_regions.raised_step_faces,
            node_regions.sidewalk_surface_polygons,
            node_regions.explicit_vertical_step_segments,
            node_regions.node_grade_authorities,
            node_regions.node_top_surface_sources,
            node_regions.owned_regions,
            earthwork_owner_sources,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        )
    }

    fn node_earthwork_owner_sources_from_regions(
        graph: &RegionGraph,
        mouths: &[OrderedIncidentPieceMouth],
        owned_regions: &[NodeOwnedRegion],
        node_top_surface_sources: &[NodeTopSurfacePolygonSource],
    ) -> Vec<NodeEarthworkOwnerSource> {
        let mut sources = Vec::new();
        for (region, top_source) in owned_regions.iter().zip(node_top_surface_sources) {
            let mouth_order_index = top_source.height_field_id.mouth_order_index();
            let Some(mouth) = mouths.get(mouth_order_index) else {
                continue;
            };
            if mouth.edge_idx >= graph.edge_count() {
                continue;
            }
            sources.push(NodeEarthworkOwnerSource {
                owner_kind: region.kind,
                owner_index: region.owner_index,
                mouth_order_index,
                edge_idx: mouth.edge_idx,
                edge_class: graph.edge(mouth.edge_idx).class,
            });
        }
        sources.sort_by(|a, b| {
            a.owner_kind
                .cmp(&b.owner_kind)
                .then(a.owner_index.cmp(&b.owner_index))
                .then(a.mouth_order_index.cmp(&b.mouth_order_index))
                .then(a.edge_idx.cmp(&b.edge_idx))
                .then(edge_class_sort_key(a.edge_class).cmp(&edge_class_sort_key(b.edge_class)))
        });
        sources.dedup_by(|a, b| {
            a.owner_kind == b.owner_kind
                && a.owner_index == b.owner_index
                && a.mouth_order_index == b.mouth_order_index
                && a.edge_idx == b.edge_idx
                && a.edge_class == b.edge_class
        });
        sources
    }
}

fn edge_class_sort_key(edge_class: EdgeClass) -> u8 {
    match edge_class {
        EdgeClass::Standard => 0,
        EdgeClass::Bridge => 1,
        EdgeClass::Tunnel => 2,
    }
}
