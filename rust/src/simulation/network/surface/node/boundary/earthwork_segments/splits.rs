//! Boundary segment splitting before source provenance resolution.

use super::sources::node_earthwork_source_for_boundary_subsegment;
use super::*;
use godot::prelude::{Vector2, Vector3};
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
            <= super::super::super::SAMPLE_EPSILON_M * super::super::super::SAMPLE_EPSILON_M
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
