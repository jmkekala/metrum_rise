//! Source-edge matching and merge helpers for earthwork boundary export.

use super::*;

pub(super) fn merge_node_earthwork_source_candidate(
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    source: &mut Option<NodeEarthworkBoundarySourceCandidate>,
    candidate: NodeEarthworkBoundarySourceCandidate,
) -> Result<(), NodeBoundaryExportError> {
    let Some(existing) = *source else {
        *source = Some(candidate);
        return Ok(());
    };
    let Some(merged) =
        merged_node_earthwork_source_candidate(start_point_key, end_point_key, existing, candidate)
    else {
        return Err(ambiguous_earthwork_boundary_segment_source_error(
            start_point_key,
            end_point_key,
            existing.face_source,
            candidate.face_source,
        ));
    };
    *source = Some(merged);
    Ok(())
}

pub(super) fn node_earthwork_source_edge_for_subsegment(
    source_edge: &NodeEarthworkBoundarySourceEdge,
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
) -> Option<NodeEarthworkBoundarySourceCandidate> {
    if !arrangement_key_lies_exactly_on_segment(
        start_point_key.xz_key(),
        source_edge.start_key,
        source_edge.end_key,
    ) || !arrangement_key_lies_exactly_on_segment(
        end_point_key.xz_key(),
        source_edge.start_key,
        source_edge.end_key,
    ) {
        return None;
    }
    Some(NodeEarthworkBoundarySourceCandidate {
        face_source: RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            node_id: source_edge.node_id,
            kind: source_edge.kind,
            owner_kind: source_edge.owner_kind,
            owner_index: source_edge.owner_index,
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                start: node_footprint_boundary_vertex_source_for_edge_point(
                    source_edge,
                    start_point_key,
                )?,
                end: node_footprint_boundary_vertex_source_for_edge_point(
                    source_edge,
                    end_point_key,
                )?,
            }),
        },
        height_field_id: Some(source_edge.height_field_id),
    })
}
