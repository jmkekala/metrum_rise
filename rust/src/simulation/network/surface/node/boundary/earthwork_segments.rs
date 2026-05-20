//! Source-backed earthwork boundary segment export from footprint loops.

use super::super::band_semantics::{raised_step_band_rank, raised_step_kinds_can_contact};
use super::sources::{
    node_footprint_boundary_vertex_source_at_point,
    node_footprint_boundary_vertex_source_for_edge_point,
};
use super::*;
use crate::simulation::network::surface::{
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceSource,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug)]
pub(super) struct NodeFootprintBoundarySplitPoint {
    pub(super) point_key: ArrangementBoundaryPointKey,
    pub(super) source: Option<NodeFootprintBoundaryDirectVertex>,
}

#[derive(Clone, Copy, Debug)]
struct NodeEarthworkBoundarySourceCandidate {
    face_source: RoadSurfaceEarthworkFaceSource,
    height_field_id: Option<arrangement::NodeBandHeightFieldId>,
}

impl NodeEarthworkBoundarySourceCandidate {
    fn from_face_source(face_source: RoadSurfaceEarthworkFaceSource) -> Self {
        Self {
            face_source,
            height_field_id: None,
        }
    }
}

pub(in crate::simulation::network::surface) fn node_earthwork_boundary_segments_from_footprint_loops(
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    footprint_loops: &[Vec<NodeFootprintBoundaryPoint>],
    sources: &NodeFootprintBoundaryExportSources,
) -> Result<Vec<Vec<RoadSurfaceEarthworkBoundarySegment>>, NodeBoundaryExportError> {
    if sources.source_edges.is_empty() {
        return Err(NodeBoundaryExportError::MissingEarthworkBoundarySource);
    }

    let mut loops = Vec::new();
    for footprint_loop in footprint_loops {
        for points in same_winding_boundary_point_loops_from_loop(footprint_loop) {
            let mut segments = Vec::new();
            for index in 0..points.len() {
                push_sourced_node_earthwork_boundary_segments(
                    node_id,
                    kind,
                    points[index],
                    points[(index + 1) % points.len()],
                    &sources.source_edges,
                    &sources.direct_vertex_sources,
                    &sources.direct_vertex_source_candidates,
                    &sources.direct_vertex_source_conflicts,
                    &sources.explicit_vertical_step_segments,
                    &mut segments,
                )?;
            }
            if segments.len() >= 3 {
                loops.push(segments);
            }
        }
    }

    (!loops.is_empty())
        .then_some(loops)
        .ok_or(NodeBoundaryExportError::MissingEarthworkBoundarySource)
}

pub(super) fn push_sourced_node_earthwork_boundary_segments(
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    start: NodeFootprintBoundaryPoint,
    end: NodeFootprintBoundaryPoint,
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
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
    segments: &mut Vec<RoadSurfaceEarthworkBoundarySegment>,
) -> Result<(), NodeBoundaryExportError> {
    let start_key = start.xz_key();
    let end_key = end.xz_key();
    if start_key == end_key {
        return Ok(());
    }
    let mut split_points =
        BTreeMap::<ArrangementSegmentParameter, NodeFootprintBoundarySplitPoint>::new();
    split_points.insert(
        ArrangementSegmentParameter::zero(),
        node_footprint_boundary_split_point_from_boundary_point(start, direct_vertex_sources),
    );
    split_points.insert(
        ArrangementSegmentParameter::one(),
        node_footprint_boundary_split_point_from_boundary_point(end, direct_vertex_sources),
    );
    for source_edge in source_edges {
        for (split_key, split_point_key, split_source) in [
            (
                source_edge.start_key,
                source_edge.start_point_key,
                source_edge.start_source,
            ),
            (
                source_edge.end_key,
                source_edge.end_point_key,
                source_edge.end_source,
            ),
        ] {
            if !arrangement_key_lies_exactly_on_segment(split_key, start_key, end_key) {
                continue;
            }
            let Some(parameter) =
                arrangement_key_segment_parameter_xz(split_key, start_key, end_key)
            else {
                continue;
            };
            if parameter <= ArrangementSegmentParameter::zero()
                || parameter >= ArrangementSegmentParameter::one()
            {
                continue;
            }
            let expected_height_mm =
                interpolated_segment_height_mm(start.point_key, end.point_key, parameter);
            if (expected_height_mm - split_point_key.y_mm).abs() > 1 {
                continue;
            }
            insert_node_footprint_boundary_split_point(
                &mut split_points,
                parameter,
                NodeFootprintBoundarySplitPoint {
                    point_key: split_point_key,
                    source: Some(NodeFootprintBoundaryDirectVertex {
                        source: NodeFootprintBoundaryVertexSource::Direct(split_source),
                        owner_kind: source_edge.owner_kind,
                        owner_index: source_edge.owner_index,
                    }),
                },
            )?;
        }
    }

    let ordered_points = split_points.into_iter().collect::<Vec<_>>();
    for pair in ordered_points.windows(2) {
        let sub_start_split = pair[0].1;
        let sub_end_split = pair[1].1;
        let sub_start = sub_start_split.point_world();
        let sub_end = sub_end_split.point_world();
        if Vector2::new(sub_end.x - sub_start.x, sub_end.z - sub_start.z).length_squared()
            <= super::super::SAMPLE_EPSILON_M * super::super::SAMPLE_EPSILON_M
        {
            continue;
        }
        let source = node_earthwork_source_for_boundary_subsegment(
            node_id,
            kind,
            sub_start_split.point_key,
            sub_end_split.point_key,
            source_edges,
            direct_vertex_sources,
            direct_vertex_source_candidates,
            direct_vertex_source_conflicts,
            sub_start_split.source,
            sub_end_split.source,
            explicit_vertical_step_segments,
        )?;
        let Some(source) = source else {
            return Err(NodeBoundaryExportError::MissingEarthworkBoundarySource);
        };
        segments.push(RoadSurfaceEarthworkBoundarySegment {
            inner_start: sub_start,
            inner_end: sub_end,
            source,
        });
    }
    Ok(())
}

impl NodeFootprintBoundarySplitPoint {
    fn point_world(self) -> Vector3 {
        arrangement_boundary_point_to_world(self.point_key)
    }
}

fn node_footprint_boundary_split_point_from_boundary_point(
    point: NodeFootprintBoundaryPoint,
    direct_vertex_sources: &BTreeMap<
        ArrangementBoundaryPointKey,
        NodeFootprintBoundaryDirectVertex,
    >,
) -> NodeFootprintBoundarySplitPoint {
    let point_key = point.point_key;
    NodeFootprintBoundarySplitPoint {
        point_key,
        source: node_footprint_boundary_vertex_source_at_point(point_key, direct_vertex_sources),
    }
}

pub(super) fn insert_node_footprint_boundary_split_point(
    split_points: &mut BTreeMap<ArrangementSegmentParameter, NodeFootprintBoundarySplitPoint>,
    parameter: ArrangementSegmentParameter,
    incoming: NodeFootprintBoundarySplitPoint,
) -> Result<(), NodeBoundaryExportError> {
    let Some(existing) = split_points.get_mut(&parameter) else {
        split_points.insert(parameter, incoming);
        return Ok(());
    };
    if existing.point_key.x_key != incoming.point_key.x_key
        || existing.point_key.z_key != incoming.point_key.z_key
    {
        return Err(
            NodeBoundaryExportError::ConflictingFootprintBoundarySplitHeight {
                x_key: incoming.point_key.x_key,
                z_key: incoming.point_key.z_key,
                existing_y_mm: existing.point_key.y_mm,
                incoming_y_mm: incoming.point_key.y_mm,
            },
        );
    }
    if existing.point_key.y_mm != incoming.point_key.y_mm {
        return Err(
            NodeBoundaryExportError::ConflictingFootprintBoundarySplitHeight {
                x_key: incoming.point_key.x_key,
                z_key: incoming.point_key.z_key,
                existing_y_mm: existing.point_key.y_mm,
                incoming_y_mm: incoming.point_key.y_mm,
            },
        );
    }
    match (existing.source, incoming.source) {
        (Some(existing_source), Some(incoming_source)) => {
            if !node_footprint_direct_vertices_share_source_identity(
                existing_source,
                incoming_source,
            ) {
                existing.source = None;
            }
        }
        (None, Some(incoming_source)) => {
            existing.source = Some(incoming_source);
        }
        _ => {}
    }
    Ok(())
}

fn node_earthwork_source_for_boundary_subsegment(
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

fn merge_node_earthwork_source_candidate(
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

fn node_earthwork_source_edge_for_subsegment(
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

fn node_earthwork_source_for_split_boundary_segment(
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

fn node_earthwork_source_for_boundary_vertices(
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

fn node_earthwork_source_for_direct_boundary_segment(
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

fn node_earthwork_boundary_owner_for_direct_vertices(
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
                && !node_footprint_direct_vertices_share_source_identity(left, right)
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

fn node_earthwork_source_for_direct_vertex_pair(
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

fn ambiguous_earthwork_boundary_segment_source_error(
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    existing_source: RoadSurfaceEarthworkFaceSource,
    incoming_source: RoadSurfaceEarthworkFaceSource,
) -> NodeBoundaryExportError {
    NodeBoundaryExportError::AmbiguousEarthworkBoundarySegmentSource {
        start_x_key: start_point_key.x_key,
        start_z_key: start_point_key.z_key,
        end_x_key: end_point_key.x_key,
        end_z_key: end_point_key.z_key,
        existing_source,
        incoming_source,
    }
}

fn merged_node_earthwork_source_candidate(
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    a: NodeEarthworkBoundarySourceCandidate,
    b: NodeEarthworkBoundarySourceCandidate,
) -> Option<NodeEarthworkBoundarySourceCandidate> {
    match (a.face_source, b.face_source) {
        (
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id: a_node_id,
                kind: a_kind,
                owner_kind: a_owner_kind,
                owner_index: a_owner_index,
                boundary_source: a_boundary_source,
            },
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id: b_node_id,
                kind: b_kind,
                owner_kind: b_owner_kind,
                owner_index: b_owner_index,
                boundary_source: b_boundary_source,
            },
        ) => {
            if a_node_id != b_node_id || a_kind != b_kind {
                return None;
            }
            let (owner_kind, owner_index, boundary_source, height_field_id) =
                merged_node_earthwork_boundary_source(
                    start_point_key,
                    end_point_key,
                    a_owner_kind,
                    a_owner_index,
                    a_boundary_source,
                    a.height_field_id,
                    b_owner_kind,
                    b_owner_index,
                    b_boundary_source,
                    b.height_field_id,
                )?;
            Some(NodeEarthworkBoundarySourceCandidate {
                face_source: RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                    node_id: a_node_id,
                    kind: a_kind,
                    owner_kind,
                    owner_index,
                    boundary_source,
                },
                height_field_id,
            })
        }
        _ => {
            (a.face_source == b.face_source && a.height_field_id == b.height_field_id).then_some(a)
        }
    }
}

fn merged_node_earthwork_boundary_source(
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    a_owner_kind: RoadSurfaceBandKind,
    a_owner_index: usize,
    a: Option<NodeFootprintBoundarySegmentSource>,
    a_height_field_id: Option<arrangement::NodeBandHeightFieldId>,
    b_owner_kind: RoadSurfaceBandKind,
    b_owner_index: usize,
    b: Option<NodeFootprintBoundarySegmentSource>,
    b_height_field_id: Option<arrangement::NodeBandHeightFieldId>,
) -> Option<(
    RoadSurfaceBandKind,
    usize,
    Option<NodeFootprintBoundarySegmentSource>,
    Option<arrangement::NodeBandHeightFieldId>,
)> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let start_matches = node_earthwork_boundary_vertex_sources_share_identity_at_point(
                start_point_key,
                a.start,
                b.start,
            );
            let end_matches = node_earthwork_boundary_vertex_sources_share_identity_at_point(
                end_point_key,
                a.end,
                b.end,
            );
            if start_matches && end_matches {
                let (owner_kind, owner_index, height_field_id) =
                    canonical_earthwork_boundary_owner(
                        a_owner_kind,
                        a_owner_index,
                        a_height_field_id,
                        b_owner_kind,
                        b_owner_index,
                        b_height_field_id,
                    )?;
                return Some((owner_kind, owner_index, Some(a), height_field_id));
            }
            if a_owner_kind == b_owner_kind && a_owner_index == b_owner_index {
                return None;
            }
            if a_owner_kind == b_owner_kind {
                let (owner_kind, owner_index, height_field_id) =
                    canonical_earthwork_boundary_owner(
                        a_owner_kind,
                        a_owner_index,
                        a_height_field_id,
                        b_owner_kind,
                        b_owner_index,
                        b_height_field_id,
                    )?;
                return Some((
                    owner_kind,
                    owner_index,
                    Some(NodeFootprintBoundarySegmentSource {
                        start: canonical_boundary_point_source(start_point_key),
                        end: canonical_boundary_point_source(end_point_key),
                    }),
                    height_field_id,
                ));
            }
            if !raised_step_kinds_can_contact(a_owner_kind, b_owner_kind)
                || (start_matches == end_matches)
            {
                return None;
            }
            let (owner_kind, owner_index, height_field_id) =
                canonical_adjacent_material_earthwork_boundary_owner(
                    a_owner_kind,
                    a_owner_index,
                    a_height_field_id,
                    b_owner_kind,
                    b_owner_index,
                    b_height_field_id,
                )?;
            Some((
                owner_kind,
                owner_index,
                Some(NodeFootprintBoundarySegmentSource {
                    start: if start_matches {
                        a.start
                    } else {
                        canonical_boundary_point_source(start_point_key)
                    },
                    end: if end_matches {
                        a.end
                    } else {
                        canonical_boundary_point_source(end_point_key)
                    },
                }),
                height_field_id,
            ))
        }
        (None, None) => {
            let (owner_kind, owner_index, height_field_id) = canonical_earthwork_boundary_owner(
                a_owner_kind,
                a_owner_index,
                a_height_field_id,
                b_owner_kind,
                b_owner_index,
                b_height_field_id,
            )?;
            Some((owner_kind, owner_index, None, height_field_id))
        }
        _ => None,
    }
}

fn canonical_earthwork_boundary_owner(
    a_owner_kind: RoadSurfaceBandKind,
    a_owner_index: usize,
    a_height_field_id: Option<arrangement::NodeBandHeightFieldId>,
    b_owner_kind: RoadSurfaceBandKind,
    b_owner_index: usize,
    b_height_field_id: Option<arrangement::NodeBandHeightFieldId>,
) -> Option<(
    RoadSurfaceBandKind,
    usize,
    Option<arrangement::NodeBandHeightFieldId>,
)> {
    if a_owner_kind == b_owner_kind && a_owner_index == b_owner_index {
        if a_height_field_id != b_height_field_id {
            return None;
        }
        return Some((a_owner_kind, a_owner_index, a_height_field_id));
    }
    if a_owner_kind == b_owner_kind {
        return Some(if a_owner_index <= b_owner_index {
            (a_owner_kind, a_owner_index, a_height_field_id)
        } else {
            (b_owner_kind, b_owner_index, b_height_field_id)
        });
    }
    canonical_adjacent_material_earthwork_boundary_owner(
        a_owner_kind,
        a_owner_index,
        a_height_field_id,
        b_owner_kind,
        b_owner_index,
        b_height_field_id,
    )
}

fn canonical_adjacent_material_earthwork_boundary_owner(
    a_owner_kind: RoadSurfaceBandKind,
    a_owner_index: usize,
    a_height_field_id: Option<arrangement::NodeBandHeightFieldId>,
    b_owner_kind: RoadSurfaceBandKind,
    b_owner_index: usize,
    b_height_field_id: Option<arrangement::NodeBandHeightFieldId>,
) -> Option<(
    RoadSurfaceBandKind,
    usize,
    Option<arrangement::NodeBandHeightFieldId>,
)> {
    let a_rank = raised_step_band_rank(a_owner_kind)?;
    let b_rank = raised_step_band_rank(b_owner_kind)?;
    if a_rank == b_rank || a_rank.abs_diff(b_rank) != 1 {
        return None;
    }
    Some(if a_rank > b_rank {
        (a_owner_kind, a_owner_index, a_height_field_id)
    } else {
        (b_owner_kind, b_owner_index, b_height_field_id)
    })
}

fn canonical_boundary_point_source(
    point_key: ArrangementBoundaryPointKey,
) -> NodeFootprintBoundaryVertexSource {
    NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
        x_key: point_key.x_key,
        z_key: point_key.z_key,
        y_mm: point_key.y_mm,
    }
}

fn node_footprint_boundary_direct_vertex_is_canonical_point(
    vertex: NodeFootprintBoundaryDirectVertex,
) -> bool {
    matches!(
        vertex.source,
        NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. }
    )
}

fn node_earthwork_boundary_vertex_sources_share_identity_at_point(
    point_key: ArrangementBoundaryPointKey,
    a: NodeFootprintBoundaryVertexSource,
    b: NodeFootprintBoundaryVertexSource,
) -> bool {
    if node_footprint_boundary_vertex_sources_share_identity(a, b) {
        return true;
    }
    match (a, b) {
        (NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { x_key, z_key, y_mm }, _)
        | (_, NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { x_key, z_key, y_mm }) => {
            x_key == point_key.x_key && z_key == point_key.z_key && y_mm == point_key.y_mm
        }
        _ => false,
    }
}

pub(in crate::simulation::network::surface::node) fn same_winding_boundary_point_loops_from_loop(
    points: &[NodeFootprintBoundaryPoint],
) -> Vec<Vec<NodeFootprintBoundaryPoint>> {
    if !boundary_point_loop_has_repeated_xz(points) {
        return vec![points.to_vec()];
    }

    let source_area_m2 = signed_boundary_point_loop_area_xz(points);
    split_boundary_point_loop_at_repeated_xz(points.to_vec())
        .into_iter()
        .filter_map(|points| {
            let points = canonicalize_boundary_point_loop(points);
            if points.len() < 3 {
                return None;
            }
            let split_area_m2 = signed_boundary_point_loop_area_xz(&points);
            if split_area_m2.abs() <= boundary_point_loop_numeric_area_budget_m2(&points) {
                return None;
            }
            (source_area_m2.signum() == split_area_m2.signum()).then_some(points)
        })
        .collect()
}

fn split_boundary_point_loop_at_repeated_xz(
    points: Vec<NodeFootprintBoundaryPoint>,
) -> Vec<Vec<NodeFootprintBoundaryPoint>> {
    let points = canonicalize_boundary_point_loop(points);
    if points.len() < 3 {
        return Vec::new();
    }

    let mut loops = Vec::new();
    let mut stack = vec![points[0]];
    let mut seen = BTreeMap::<arrangement::NodeArrangementKey, usize>::new();
    seen.insert(points[0].xz_key(), 0);
    for index in 1..=points.len() {
        let current = points[index % points.len()];
        let current_key = current.xz_key();
        if let Some(start_index) = seen.get(&current_key).copied() {
            let mut cycle = stack[start_index..].to_vec();
            cycle.push(current);
            let cycle = canonicalize_boundary_point_loop(cycle);
            if cycle.len() >= 3 {
                loops.push(cycle);
            }
            stack.truncate(start_index + 1);
            if let Some(last) = stack.last_mut() {
                *last = current;
            }
            seen.clear();
            for (stack_index, point) in stack.iter().enumerate() {
                seen.insert(point.xz_key(), stack_index);
            }
        } else {
            stack.push(current);
            seen.insert(current_key, stack.len() - 1);
        }
    }

    if loops.is_empty() {
        vec![points]
    } else {
        loops
    }
}

fn boundary_point_loop_has_repeated_xz(points: &[NodeFootprintBoundaryPoint]) -> bool {
    let mut seen = BTreeSet::new();
    for point in canonicalize_boundary_point_loop(points.to_vec()) {
        if !seen.insert(point.xz_key()) {
            return true;
        }
    }
    false
}

fn canonicalize_boundary_point_loop(
    mut points: Vec<NodeFootprintBoundaryPoint>,
) -> Vec<NodeFootprintBoundaryPoint> {
    points.dedup_by(|a, b| a.point_key == b.point_key);
    if points.len() >= 2
        && points.first().map(|point| point.point_key) == points.last().map(|point| point.point_key)
    {
        points.pop();
    }
    points
}

fn boundary_point_loop_world_points(points: &[NodeFootprintBoundaryPoint]) -> Vec<Vector3> {
    points.iter().map(|point| point.point_world()).collect()
}

fn signed_boundary_point_loop_area_xz(points: &[NodeFootprintBoundaryPoint]) -> f32 {
    RoadSurfaceSystem::signed_polygon_area_xz(&boundary_point_loop_world_points(points))
}

fn boundary_point_loop_numeric_area_budget_m2(points: &[NodeFootprintBoundaryPoint]) -> f32 {
    boundary_points_numeric_area_budget_m2(&boundary_point_loop_world_points(points))
}
