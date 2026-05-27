//! Boundary segment splitting before source provenance resolution.

use super::sources::node_earthwork_source_for_boundary_subsegment;
use super::*;
use std::collections::BTreeMap;

pub(in crate::simulation::network::surface::node::boundary) fn push_sourced_node_earthwork_boundary_segments(
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
        node_footprint_boundary_split_point_from_boundary_point(
            start,
            direct_vertex_sources,
            source_edges,
        )?,
    );
    split_points.insert(
        ArrangementSegmentParameter::one(),
        node_footprint_boundary_split_point_from_boundary_point(
            end,
            direct_vertex_sources,
            source_edges,
        )?,
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
            let split_point_key = ArrangementBoundaryPointKey {
                y_mm: expected_height_mm,
                ..split_point_key
            };
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
        if RoadVec2::new(sub_end.x - sub_start.x, sub_end.z - sub_start.z).length_squared()
            <= f64::from(
                super::super::super::SAMPLE_EPSILON_M * super::super::super::SAMPLE_EPSILON_M,
            )
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
            return Err(
                NodeBoundaryExportError::MissingEarthworkBoundarySegmentSource {
                    start_x_key: sub_start_split.point_key.x_key,
                    start_z_key: sub_start_split.point_key.z_key,
                    end_x_key: sub_end_split.point_key.x_key,
                    end_z_key: sub_end_split.point_key.z_key,
                    nearby_source_edges: nearby_source_edges_for_missing_segment(
                        sub_start_split.point_key.xz_key(),
                        sub_end_split.point_key.xz_key(),
                        source_edges,
                    ),
                },
            );
        };
        segments.push(RoadSurfaceEarthworkBoundarySegment {
            inner_start: sub_start,
            inner_end: sub_end,
            source,
        });
    }
    Ok(())
}

fn nearby_source_edges_for_missing_segment(
    start_key: arrangement::NodeArrangementKey,
    end_key: arrangement::NodeArrangementKey,
    source_edges: &[NodeEarthworkBoundarySourceEdge],
) -> Vec<((i64, i64), (i64, i64), RoadSurfaceBandKind, usize, bool)> {
    const DIAGNOSTIC_OVERLAY_KEY_MARGIN: i64 = 1024;
    let min_x = start_key.x_key().min(end_key.x_key()) - DIAGNOSTIC_OVERLAY_KEY_MARGIN;
    let max_x = start_key.x_key().max(end_key.x_key()) + DIAGNOSTIC_OVERLAY_KEY_MARGIN;
    let min_z = start_key.z_key().min(end_key.z_key()) - DIAGNOSTIC_OVERLAY_KEY_MARGIN;
    let max_z = start_key.z_key().max(end_key.z_key()) + DIAGNOSTIC_OVERLAY_KEY_MARGIN;
    source_edges
        .iter()
        .filter(|edge| {
            let edge_min_x = edge.start_key.x_key().min(edge.end_key.x_key());
            let edge_max_x = edge.start_key.x_key().max(edge.end_key.x_key());
            let edge_min_z = edge.start_key.z_key().min(edge.end_key.z_key());
            let edge_max_z = edge.start_key.z_key().max(edge.end_key.z_key());
            edge_min_x <= max_x && edge_max_x >= min_x && edge_min_z <= max_z && edge_max_z >= min_z
        })
        .take(12)
        .map(|edge| {
            (
                (edge.start_key.x_key(), edge.start_key.z_key()),
                (edge.end_key.x_key(), edge.end_key.z_key()),
                edge.owner_kind,
                edge.owner_index,
                edge.final_footprint_boundary,
            )
        })
        .collect()
}

impl NodeFootprintBoundarySplitPoint {
    fn point_world(self) -> RoadVec3 {
        arrangement_boundary_point_to_world(self.point_key)
    }
}

fn node_footprint_boundary_split_point_from_boundary_point(
    point: NodeFootprintBoundaryPoint,
    direct_vertex_sources: &BTreeMap<
        ArrangementBoundaryPointKey,
        NodeFootprintBoundaryDirectVertex,
    >,
    source_edges: &[NodeEarthworkBoundarySourceEdge],
) -> Result<NodeFootprintBoundarySplitPoint, NodeBoundaryExportError> {
    let point_key = point.point_key;
    let source =
        match node_footprint_boundary_vertex_source_at_point(point_key, direct_vertex_sources) {
            Some(source) => Some(source),
            None => node_footprint_boundary_split_source_from_edges(point_key, source_edges)?,
        };
    Ok(NodeFootprintBoundarySplitPoint { point_key, source })
}

fn node_footprint_boundary_split_source_from_edges(
    point_key: ArrangementBoundaryPointKey,
    source_edges: &[NodeEarthworkBoundarySourceEdge],
) -> Result<Option<NodeFootprintBoundaryDirectVertex>, NodeBoundaryExportError> {
    let final_source =
        node_footprint_boundary_split_source_from_matching_edges(point_key, source_edges, true)?;
    if final_source.is_some() {
        return Ok(final_source);
    }
    node_footprint_boundary_split_source_from_matching_edges(point_key, source_edges, false)
}

fn node_footprint_boundary_split_source_from_matching_edges(
    point_key: ArrangementBoundaryPointKey,
    source_edges: &[NodeEarthworkBoundarySourceEdge],
    final_footprint_boundary: bool,
) -> Result<Option<NodeFootprintBoundaryDirectVertex>, NodeBoundaryExportError> {
    let mut source = None;
    for source_edge in source_edges
        .iter()
        .filter(|edge| edge.final_footprint_boundary == final_footprint_boundary)
    {
        let Some(vertex_source) =
            node_footprint_boundary_vertex_source_for_edge_point(source_edge, point_key)
        else {
            continue;
        };
        if let Err(error) = merge_node_footprint_boundary_point_source(
            point_key,
            &mut source,
            NodeFootprintBoundaryDirectVertex {
                source: vertex_source,
                owner_kind: source_edge.owner_kind,
                owner_index: source_edge.owner_index,
            },
        ) {
            if matches!(
                error,
                NodeBoundaryExportError::AmbiguousFootprintBoundaryPointSource { .. }
            ) {
                return Ok(None);
            }
            return Err(error);
        }
    }
    Ok(source)
}

pub(in crate::simulation::network::surface::node::boundary) fn insert_node_footprint_boundary_split_point(
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
