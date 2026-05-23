//! Earthwork boundary source selection from final edges and direct vertices.

use super::merge::*;
use super::*;
use std::collections::BTreeMap;

mod direct;
mod edge_sources;
mod split;

use direct::*;
use edge_sources::*;
use split::*;

pub(super) fn node_earthwork_source_for_boundary_subsegment(
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    source_edges: &[NodeEarthworkBoundarySourceEdge],
    direct_vertex_sources: &BTreeMap<
        ArrangementBoundaryPointKey,
        NodeFootprintBoundaryDirectVertex,
    >,
    direct_vertex_source_candidates: &BTreeMap<
        ArrangementBoundaryPointKey,
        Vec<NodeFootprintBoundaryDirectVertex>,
    >,
    direct_vertex_source_conflicts: &BTreeMap<
        ArrangementBoundaryPointKey,
        NodeFootprintBoundaryDirectVertexConflict,
    >,
    start_split_source: Option<NodeFootprintBoundaryDirectVertex>,
    end_split_source: Option<NodeFootprintBoundaryDirectVertex>,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> Result<Option<RoadSurfaceEarthworkFaceSource>, NodeBoundaryExportError> {
    let mut source = None;
    for candidate in source_edges.iter().filter_map(|source_edge| {
        source_edge
            .final_footprint_boundary
            .then(|| {
                node_earthwork_source_edge_for_subsegment(
                    source_edge,
                    start_point_key,
                    end_point_key,
                )
            })
            .flatten()
    }) {
        merge_node_earthwork_source_candidate(
            start_point_key,
            end_point_key,
            &mut source,
            candidate,
        )?;
    }
    if let Some(source) = source {
        return Ok(Some(source.face_source));
    }

    for candidate in source_edges.iter().filter_map(|source_edge| {
        (!source_edge.final_footprint_boundary)
            .then(|| {
                node_earthwork_source_edge_for_subsegment(
                    source_edge,
                    start_point_key,
                    end_point_key,
                )
            })
            .flatten()
    }) {
        merge_node_earthwork_source_candidate(
            start_point_key,
            end_point_key,
            &mut source,
            candidate,
        )?;
    }
    if let Some(source) = source {
        return Ok(Some(source.face_source));
    }

    if let Some(candidate) = node_earthwork_source_for_direct_boundary_segment(
        node_id,
        kind,
        start_point_key,
        end_point_key,
        direct_vertex_sources,
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts,
        explicit_vertical_step_segments,
    )? {
        return Ok(Some(candidate));
    }

    node_earthwork_source_for_split_boundary_segment(
        node_id,
        kind,
        start_point_key,
        end_point_key,
        start_split_source,
        end_split_source,
        explicit_vertical_step_segments,
    )
}
