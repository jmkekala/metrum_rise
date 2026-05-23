//! Direct boundary vertex source selection.

use super::*;

pub(super) fn node_earthwork_source_for_direct_boundary_segment(
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
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
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> Result<Option<RoadSurfaceEarthworkFaceSource>, NodeBoundaryExportError> {
    let start_candidates = node_footprint_boundary_vertex_sources_at_point(
        start_point_key,
        direct_vertex_sources,
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts,
    )?;
    let end_candidates = node_footprint_boundary_vertex_sources_at_point(
        end_point_key,
        direct_vertex_sources,
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts,
    )?;
    if start_candidates.is_empty() || end_candidates.is_empty() {
        return Ok(None);
    };

    let mut source = None;
    let mut saw_direct_endpoint_candidates = false;
    for start in &start_candidates {
        for end in &end_candidates {
            saw_direct_endpoint_candidates = true;
            let Some(owner) = node_earthwork_boundary_owner_for_direct_vertices(
                start_point_key,
                end_point_key,
                *start,
                *end,
                explicit_vertical_step_segments,
            ) else {
                continue;
            };
            let candidate =
                node_earthwork_source_for_direct_vertex_pair(node_id, kind, owner, *start, *end);
            merge_node_earthwork_source_candidate(
                start_point_key,
                end_point_key,
                &mut source,
                NodeEarthworkBoundarySourceCandidate::from_face_source(candidate),
            )?;
        }
    }
    if let Some(source) = source {
        return Ok(Some(source.face_source));
    }
    if saw_direct_endpoint_candidates {
        return node_earthwork_source_for_boundary_vertices(
            node_id,
            kind,
            start_point_key,
            end_point_key,
            start_candidates[0],
            end_candidates[0],
            explicit_vertical_step_segments,
        );
    }

    let start =
        node_footprint_boundary_vertex_source_at_point(start_point_key, direct_vertex_sources);
    let end = node_footprint_boundary_vertex_source_at_point(end_point_key, direct_vertex_sources);
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

pub(super) fn node_earthwork_boundary_owner_for_direct_vertices(
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    start: NodeFootprintBoundaryDirectVertex,
    end: NodeFootprintBoundaryDirectVertex,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> Option<NodeFootprintBoundaryDirectVertex> {
    if node_footprint_direct_vertices_share_owner_identity(start, end) {
        return Some(start);
    }
    explicit_vertical_step_boundary_owner(
        start_point_key,
        end_point_key,
        start,
        end,
        explicit_vertical_step_segments,
    )
    .or_else(|| raised_step_boundary_connector_owner(start_point_key, end_point_key, start, end))
}

fn explicit_vertical_step_boundary_owner(
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    start: NodeFootprintBoundaryDirectVertex,
    end: NodeFootprintBoundaryDirectVertex,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> Option<NodeFootprintBoundaryDirectVertex> {
    if !raised_step_kinds_can_contact(start.owner_kind, end.owner_kind) {
        return None;
    }
    let start_rank = raised_step_band_rank(start.owner_kind)?;
    let end_rank = raised_step_band_rank(end.owner_kind)?;
    if start_rank == end_rank {
        return None;
    }

    let start_owner = arrangement::NodeBandOwner::new(start.owner_kind, start.owner_index);
    let end_owner = arrangement::NodeBandOwner::new(end.owner_kind, end.owner_index);
    let expected_step = arrangement::NodeExplicitVerticalStepSegment::new(
        start_point_key.xz_key(),
        end_point_key.xz_key(),
        start_owner,
        end_owner,
    )?;
    if !explicit_vertical_step_segments
        .iter()
        .any(|segment| *segment == expected_step)
    {
        return None;
    }

    Some(if start_rank > end_rank { start } else { end })
}

fn raised_step_boundary_connector_owner(
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    start: NodeFootprintBoundaryDirectVertex,
    end: NodeFootprintBoundaryDirectVertex,
) -> Option<NodeFootprintBoundaryDirectVertex> {
    let start_rank = raised_step_band_rank(start.owner_kind)?;
    let end_rank = raised_step_band_rank(end.owner_kind)?;
    match start_rank.cmp(&end_rank) {
        std::cmp::Ordering::Greater if start_point_key.y_mm > end_point_key.y_mm => Some(start),
        std::cmp::Ordering::Less if end_point_key.y_mm > start_point_key.y_mm => Some(end),
        _ => None,
    }
}

fn node_footprint_boundary_vertex_sources_at_point(
    point_key: ArrangementBoundaryPointKey,
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
) -> Result<Vec<NodeFootprintBoundaryDirectVertex>, NodeBoundaryExportError> {
    if let Some(candidates) = direct_vertex_source_candidates.get(&point_key) {
        return canonicalized_same_material_boundary_point_sources(point_key, candidates);
    }
    if let Some(conflict) = direct_vertex_source_conflicts.get(&point_key).copied() {
        return canonicalized_same_material_boundary_point_sources(
            point_key,
            &[conflict.existing, conflict.incoming],
        );
    }
    Ok(
        node_footprint_boundary_vertex_source_at_point(point_key, direct_vertex_sources)
            .into_iter()
            .collect(),
    )
}

fn canonicalized_same_material_boundary_point_sources(
    point_key: ArrangementBoundaryPointKey,
    candidates: &[NodeFootprintBoundaryDirectVertex],
) -> Result<Vec<NodeFootprintBoundaryDirectVertex>, NodeBoundaryExportError> {
    for (left_index, left) in candidates.iter().copied().enumerate() {
        for right in candidates.iter().copied().skip(left_index + 1) {
            if left.owner_kind == right.owner_kind
                && left.owner_index == right.owner_index
                && !node_footprint_direct_vertices_share_boundary_point_authority(
                    point_key, left, right,
                )
            {
                return Err(ambiguous_footprint_boundary_point_source_error(
                    point_key, left, right,
                ));
            }
        }
    }

    let mut owner_counts = BTreeMap::<RoadSurfaceBandKind, usize>::new();
    for candidate in candidates {
        *owner_counts.entry(candidate.owner_kind).or_default() += 1;
    }
    let mut canonicalized = candidates.to_vec();
    for candidate in &mut canonicalized {
        if owner_counts
            .get(&candidate.owner_kind)
            .copied()
            .unwrap_or_default()
            > 1
        {
            candidate.source = NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                x_key: point_key.x_key,
                z_key: point_key.z_key,
                y_mm: point_key.y_mm,
            };
        }
    }
    Ok(canonicalized)
}

pub(super) fn node_earthwork_source_for_direct_vertex_pair(
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    owner: NodeFootprintBoundaryDirectVertex,
    start: NodeFootprintBoundaryDirectVertex,
    end: NodeFootprintBoundaryDirectVertex,
) -> RoadSurfaceEarthworkFaceSource {
    RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
        node_id,
        kind,
        owner_kind: owner.owner_kind,
        owner_index: owner.owner_index,
        boundary_source: Some(NodeFootprintBoundarySegmentSource {
            start: start.source,
            end: end.source,
        }),
    }
}
