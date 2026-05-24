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
    pub(in crate::simulation::network::surface) fn has_exact_final_owned_footprint_boundary_support_at_xz_key(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> bool {
        self.direct_vertex_sources
            .iter()
            .any(|(point_key, source)| {
                point_key.xz_key() == key
                    && matches!(source.source, NodeFootprintBoundaryVertexSource::Direct(_))
            })
            || self
                .final_vertex_sources
                .keys()
                .any(|point_key| point_key.xz_key() == key)
            || self.final_height_edges.iter().any(|source_edge| {
                boundary_segment_parameter_xz_on_segment(
                    ArrangementBoundaryPointKey {
                        x_key: key.x_key(),
                        z_key: key.z_key(),
                        y_mm: 0,
                    },
                    source_edge.start_point_key,
                    source_edge.end_point_key,
                )
                .is_some()
            })
            || self
                .source_edges
                .iter()
                .filter(|edge| edge.final_footprint_boundary)
                .any(|source_edge| {
                    boundary_segment_parameter_xz_on_segment(
                        ArrangementBoundaryPointKey {
                            x_key: key.x_key(),
                            z_key: key.z_key(),
                            y_mm: 0,
                        },
                        source_edge.start_point_key,
                        source_edge.end_point_key,
                    )
                    .is_some()
                })
    }

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
            if node_footprint_boundary_vertex_source_is_exact(source.source) {
                return Some(NodeFinalOwnedFootprintBoundarySupport::DirectVertex);
            }
        }
        if self
            .final_vertex_sources
            .get(&point_key)
            .is_some_and(|sources| {
                sources
                    .iter()
                    .any(|source| node_footprint_boundary_vertex_source_is_exact(source.source))
            })
        {
            return Some(NodeFinalOwnedFootprintBoundarySupport::DirectVertex);
        }
        if self.final_height_edges.iter().any(|source_edge| {
            let Some(parameter) = boundary_segment_parameter_xz_on_segment(
                point_key,
                source_edge.start_point_key,
                source_edge.end_point_key,
            ) else {
                return false;
            };
            let expected_height_mm = interpolated_segment_height_mm(
                source_edge.start_point_key,
                source_edge.end_point_key,
                parameter,
            );
            expected_height_mm == point_key.y_mm
        }) {
            return Some(NodeFinalOwnedFootprintBoundarySupport::ExactSourceEdge);
        }
        self.source_edges
            .iter()
            .filter(|edge| edge.final_footprint_boundary)
            .find_map(|source_edge| {
                node_footprint_boundary_vertex_source_for_edge_point(source_edge, point_key)
                    .map(|_| NodeFinalOwnedFootprintBoundarySupport::ExactSourceEdge)
            })
    }
}

fn node_footprint_boundary_vertex_source_is_exact(
    source: NodeFootprintBoundaryVertexSource,
) -> bool {
    matches!(
        source,
        NodeFootprintBoundaryVertexSource::Direct(_)
            | NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. }
    )
}
