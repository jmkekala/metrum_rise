//! Final-owned footprint boundary support proofs.

use super::sources::node_footprint_boundary_vertex_source_for_edge_point;
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NodeFinalOwnedFootprintBoundarySupport {
    DirectVertex,
    ExactSourceEdge,
    CanonicalEndpointDust,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeFinalBoundaryEndpointDustSupport {
    owner_kind: RoadSurfaceBandKind,
    owner_index: usize,
    y_mm: i64,
}

impl NodeFinalOwnedFootprintBoundarySupport {
    fn is_exact(self) -> bool {
        matches!(
            self,
            Self::DirectVertex | Self::ExactSourceEdge | Self::CanonicalEndpointDust
        )
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
        if self
            .final_endpoint_dust_height_candidates_at_key(point_key.xz_key())
            .iter()
            .any(|candidate| candidate.height_mm == point_key.y_mm)
        {
            return Some(NodeFinalOwnedFootprintBoundarySupport::CanonicalEndpointDust);
        }
        self.source_edges
            .iter()
            .filter(|edge| edge.final_footprint_boundary)
            .find_map(|source_edge| {
                node_footprint_boundary_vertex_source_for_edge_point(source_edge, point_key)
                    .map(|_| NodeFinalOwnedFootprintBoundarySupport::ExactSourceEdge)
            })
    }

    pub(in crate::simulation::network::surface::node::boundary) fn final_endpoint_dust_height_candidates_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> Vec<NodeFootprintBoundaryHeightCandidate> {
        let supports = self.final_endpoint_dust_supports_at_key(key);
        let mut candidates = Vec::new();
        for support in &supports {
            if supports
                .iter()
                .filter(|candidate| {
                    candidate.owner_kind == support.owner_kind
                        && candidate.owner_index == support.owner_index
                        && candidate.y_mm == support.y_mm
                })
                .count()
                < 2
            {
                continue;
            }
            let candidate = NodeFootprintBoundaryHeightCandidate {
                height_mm: support.y_mm,
                source: NodeFootprintBoundaryDirectVertex {
                    source: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                        x_key: key.x_key(),
                        z_key: key.z_key(),
                        y_mm: support.y_mm,
                    },
                    owner_kind: support.owner_kind,
                    owner_index: support.owner_index,
                },
            };
            if !candidates.iter().any(|existing| *existing == candidate) {
                candidates.push(candidate);
            }
        }
        candidates
    }

    fn final_endpoint_dust_supports_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> Vec<NodeFinalBoundaryEndpointDustSupport> {
        let point = arrangement_key(key);
        self.final_height_edges
            .iter()
            .filter_map(|edge| {
                let start = arrangement_key(edge.start_point_key.xz_key());
                let end = arrangement_key(edge.end_point_key.xz_key());
                let near_start = key_distance_squared(point, start)
                    <= i128::from(BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS)
                        * i128::from(BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS);
                let near_end = key_distance_squared(point, end)
                    <= i128::from(BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS)
                        * i128::from(BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS);
                let y_mm = match (near_start, near_end) {
                    (true, false) => edge.start_point_key.y_mm,
                    (false, true) => edge.end_point_key.y_mm,
                    (true, true) if edge.start_point_key.y_mm == edge.end_point_key.y_mm => {
                        edge.start_point_key.y_mm
                    }
                    _ => return None,
                };
                Some(NodeFinalBoundaryEndpointDustSupport {
                    owner_kind: edge.owner_kind,
                    owner_index: edge.owner_index,
                    y_mm,
                })
            })
            .collect()
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
