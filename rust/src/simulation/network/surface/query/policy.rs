//! Visibility and integrated-earthwork policy for surface queries.

use super::super::earthwork::EARTHWORK_PAVEMENT_DEPTH_M;
use super::super::node::NodeEarthworkOwnerSource;
use super::super::{
    RoadSurfaceEarthworkFaceSource, RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem,
    RoadSurfaceVisualNodePiece, RoadSurfaceVisualSpanPiece,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::terrain::TerrainSystem;

impl RoadSurfaceSystem {
    pub(crate) fn node_uses_visible_surface(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
    ) -> bool {
        if node_id as usize >= graph.node_adjacency_count() {
            return false;
        }

        let mut has_supported_surface = false;
        let mut has_visible_surface_attachment = false;
        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted || !Self::is_surface_edge(edge) {
                continue;
            }
            let Some(_) = self.compiled_sections.get(&edge_idx) else {
                continue;
            };

            has_supported_surface = true;
            if edge.primary_type == TransitType::Foot || edge.class != EdgeClass::Tunnel {
                has_visible_surface_attachment = true;
                continue;
            }

            let at_start = graph.get_valid_node(edge.start_node) == node_id;
            if self.tunnel_throat_is_visible(edge_idx, at_start, terrain) {
                has_visible_surface_attachment = true;
            }
        }

        has_supported_surface && has_visible_surface_attachment
    }

    pub(crate) fn span_piece_uses_visible_earthwork(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
    ) -> bool {
        piece.edge_class != EdgeClass::Standard
    }

    pub(crate) fn node_piece_uses_visible_earthwork(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        terrain: &TerrainSystem,
    ) -> bool {
        let Some(piece) = self.compiled_visual_node_pieces().get(&node_id) else {
            return false;
        };
        piece.earthwork_owner_sources.iter().any(|source| {
            self.node_earthwork_owner_source_uses_visible_earthwork(graph, terrain, node_id, source)
        })
    }

    pub(crate) fn node_earthwork_face_uses_visible_earthwork(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        face: &RoadSurfaceEarthworkRenderFace,
    ) -> bool {
        let RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            owner_kind,
            owner_index,
            ..
        } = face.source
        else {
            return false;
        };
        self.node_earthwork_owner_uses_visible_earthwork(
            graph,
            terrain,
            node_id,
            piece,
            owner_kind,
            owner_index,
        )
    }

    pub(in crate::simulation::network::surface) fn node_earthwork_owner_uses_visible_earthwork(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        owner_kind: super::super::RoadSurfaceBandKind,
        owner_index: usize,
    ) -> bool {
        piece
            .earthwork_owner_sources
            .iter()
            .filter(|source| source.owner_kind == owner_kind && source.owner_index == owner_index)
            .any(|source| {
                self.node_earthwork_owner_source_uses_visible_earthwork(
                    graph, terrain, node_id, source,
                )
            })
    }

    fn node_earthwork_owner_source_uses_visible_earthwork(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        source: &NodeEarthworkOwnerSource,
    ) -> bool {
        if source.edge_idx >= graph.edge_count() {
            return false;
        }
        let edge = graph.edge(source.edge_idx);
        if edge.deleted || !Self::is_surface_edge(edge) {
            return false;
        }
        match edge.class {
            EdgeClass::Standard => false,
            EdgeClass::Bridge => true,
            EdgeClass::Tunnel => {
                let at_start = graph.get_valid_node(edge.start_node) == node_id;
                self.tunnel_throat_is_visible(source.edge_idx, at_start, terrain)
            }
        }
    }

    pub(in crate::simulation::network::surface) fn span_piece_integrated_surface_offset_m(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
    ) -> f32 {
        if self.span_piece_uses_visible_earthwork(piece) {
            EARTHWORK_PAVEMENT_DEPTH_M
        } else {
            0.0
        }
    }

    pub(in crate::simulation::network::surface) fn node_piece_integrated_surface_offset_m(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        terrain: &TerrainSystem,
    ) -> f32 {
        if self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
            EARTHWORK_PAVEMENT_DEPTH_M
        } else {
            0.0
        }
    }
}
