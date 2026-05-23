//! Split boundary segment source selection.

use super::*;

pub(super) fn node_earthwork_source_for_split_boundary_segment(
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    start: Option<NodeFootprintBoundaryDirectVertex>,
    end: Option<NodeFootprintBoundaryDirectVertex>,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> Result<Option<RoadSurfaceEarthworkFaceSource>, NodeBoundaryExportError> {
    let (Some(start), Some(end)) = (start, end) else {
        return Ok(None);
    };
    node_earthwork_source_for_boundary_vertices(
        node_id,
        kind,
        start_point_key,
        end_point_key,
        start,
        end,
        explicit_vertical_step_segments,
    )
}

pub(super) fn node_earthwork_source_for_boundary_vertices(
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    start: NodeFootprintBoundaryDirectVertex,
    end: NodeFootprintBoundaryDirectVertex,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> Result<Option<RoadSurfaceEarthworkFaceSource>, NodeBoundaryExportError> {
    let Some(owner) = node_earthwork_boundary_owner_for_direct_vertices(
        start_point_key,
        end_point_key,
        start,
        end,
        explicit_vertical_step_segments,
    ) else {
        let start_candidate =
            node_earthwork_source_for_direct_vertex_pair(node_id, kind, start, start, end);
        let end_candidate =
            node_earthwork_source_for_direct_vertex_pair(node_id, kind, end, start, end);
        if (start.owner_kind != end.owner_kind
            || start.owner_index != end.owner_index
            || node_footprint_boundary_direct_vertex_is_canonical_point(start)
            || node_footprint_boundary_direct_vertex_is_canonical_point(end))
            && let Some(merged) = merged_node_earthwork_source_candidate(
                start_point_key,
                end_point_key,
                NodeEarthworkBoundarySourceCandidate::from_face_source(start_candidate),
                NodeEarthworkBoundarySourceCandidate::from_face_source(end_candidate),
            )
        {
            return Ok(Some(merged.face_source));
        }
        return Err(ambiguous_earthwork_boundary_segment_source_error(
            start_point_key,
            end_point_key,
            start_candidate,
            end_candidate,
        ));
    };

    Ok(Some(
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            node_id,
            kind,
            owner_kind: owner.owner_kind,
            owner_index: owner.owner_index,
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                start: start.source,
                end: end.source,
            }),
        },
    ))
}
