//! Terminal source-edge footprint authorization.

use super::*;

pub(super) fn terminal_source_edge_endpoints_authorize_footprint_height_pair(
    key: arrangement::NodeArrangementKey,
    lower: NodeFootprintBoundaryHeightCandidate,
    raised: NodeFootprintBoundaryHeightCandidate,
    source_edges: &[NodeEarthworkBoundarySourceEdge],
) -> bool {
    if !raised_step_kinds_can_contact(lower.source.owner_kind, raised.source.owner_kind) {
        return false;
    }
    let Some(lower_rank) = raised_step_band_rank(lower.source.owner_kind) else {
        return false;
    };
    let Some(raised_rank) = raised_step_band_rank(raised.source.owner_kind) else {
        return false;
    };
    if lower_rank >= raised_rank {
        return false;
    }
    source_edges.iter().any(|lower_edge| {
        terminal_source_edge_endpoint_proves_candidate_at_key(lower_edge, key, lower)
            && source_edges.iter().any(|raised_edge| {
                terminal_source_edge_endpoint_proves_candidate_at_key(raised_edge, key, raised)
            })
    })
}

fn terminal_source_edge_endpoint_proves_candidate_at_key(
    source_edge: &NodeEarthworkBoundarySourceEdge,
    key: arrangement::NodeArrangementKey,
    candidate: NodeFootprintBoundaryHeightCandidate,
) -> bool {
    if source_edge.kind != RoadSurfaceVisualNodePieceKind::Terminal
        || source_edge.owner_kind != candidate.source.owner_kind
        || source_edge.owner_index != candidate.source.owner_index
    {
        return false;
    }
    terminal_source_edge_endpoint_matches_candidate(
        source_edge.start_key,
        source_edge.start_point_key.y_mm,
        source_edge.start_source,
        key,
        candidate,
    ) || terminal_source_edge_endpoint_matches_candidate(
        source_edge.end_key,
        source_edge.end_point_key.y_mm,
        source_edge.end_source,
        key,
        candidate,
    )
}

fn terminal_source_edge_endpoint_matches_candidate(
    endpoint_key: arrangement::NodeArrangementKey,
    endpoint_height_mm: i64,
    endpoint_source: NodeFootprintBoundaryDirectSource,
    key: arrangement::NodeArrangementKey,
    candidate: NodeFootprintBoundaryHeightCandidate,
) -> bool {
    if endpoint_height_mm != candidate.height_mm {
        return false;
    }
    endpoint_key == key
        && candidate.source.source == NodeFootprintBoundaryVertexSource::Direct(endpoint_source)
}
