//! Final-owned footprint boundary support proofs.

use super::sources::node_footprint_boundary_vertex_source_for_edge_point;
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NodeFinalOwnedFootprintBoundarySupport {
    DirectVertex,
    ExactSourceEdge,
}

impl NodeFinalOwnedFootprintBoundarySupport {
    fn is_exact(self) -> bool {
        matches!(self, Self::DirectVertex | Self::ExactSourceEdge)
    }
}

impl NodeFootprintBoundaryExportSources {
    pub(in crate::simulation::network::surface) fn has_exact_final_owned_footprint_boundary_support_at_point(
        &self,
        point_key: ArrangementBoundaryPointKey,
    ) -> bool {
        self.final_owned_footprint_boundary_support_at_point(point_key)
            .is_some_and(|support| support.is_exact())
    }

    fn final_owned_footprint_boundary_support_at_point(
        &self,
        point_key: ArrangementBoundaryPointKey,
    ) -> Option<NodeFinalOwnedFootprintBoundarySupport> {
        if let Some(source) = self.direct_vertex_sources.get(&point_key).copied() {
            if matches!(source.source, NodeFootprintBoundaryVertexSource::Direct(_)) {
                return Some(NodeFinalOwnedFootprintBoundarySupport::DirectVertex);
            }
        }
        self.source_edges.iter().find_map(|source_edge| {
            node_footprint_boundary_vertex_source_for_edge_point(source_edge, point_key)
                .map(|_| NodeFinalOwnedFootprintBoundarySupport::ExactSourceEdge)
        })
    }
}
