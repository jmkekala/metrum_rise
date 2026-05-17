//! Source-backed earthwork boundary segment export from footprint loops.

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

pub(in crate::simulation::network::surface) fn node_earthwork_boundary_segments_from_footprint_loops(
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    footprint_loops: &[Vec<Vector3>],
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
    start: Vector3,
    end: Vector3,
    source_edges: &[NodeEarthworkBoundarySourceEdge],
    direct_vertex_sources: &BTreeMap<
        ArrangementBoundaryPointKey,
        NodeFootprintBoundaryDirectVertex,
    >,
    segments: &mut Vec<RoadSurfaceEarthworkBoundarySegment>,
) -> Result<(), NodeBoundaryExportError> {
    let start_key = ArrangementBoundaryPointKey::from_world(start).xz_key();
    let end_key = ArrangementBoundaryPointKey::from_world(end).xz_key();
    if start_key == end_key {
        return Ok(());
    }
    let mut split_points =
        BTreeMap::<ArrangementSegmentParameter, NodeFootprintBoundarySplitPoint>::new();
    split_points.insert(
        ArrangementSegmentParameter::zero(),
        node_footprint_boundary_split_point_from_world(start, direct_vertex_sources),
    );
    split_points.insert(
        ArrangementSegmentParameter::one(),
        node_footprint_boundary_split_point_from_world(end, direct_vertex_sources),
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
            if !arrangement_key_lies_on_segment(split_key, start_key, end_key) {
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
        let sub_start_point_key = ArrangementBoundaryPointKey::from_world(sub_start);
        let sub_end_point_key = ArrangementBoundaryPointKey::from_world(sub_end);
        let source = node_earthwork_source_for_boundary_subsegment(
            node_id,
            kind,
            sub_start_point_key,
            sub_end_point_key,
            source_edges,
            direct_vertex_sources,
            sub_start_split.source,
            sub_end_split.source,
        );
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

fn node_footprint_boundary_split_point_from_world(
    point: Vector3,
    direct_vertex_sources: &BTreeMap<
        ArrangementBoundaryPointKey,
        NodeFootprintBoundaryDirectVertex,
    >,
) -> NodeFootprintBoundarySplitPoint {
    let point_key = ArrangementBoundaryPointKey::from_world(point);
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
        let source_ordering =
            node_footprint_split_source_ordering(incoming.source, existing.source);
        if source_ordering.is_eq() {
            return Err(
                NodeBoundaryExportError::ConflictingFootprintBoundarySplitHeight {
                    x_key: incoming.point_key.x_key,
                    z_key: incoming.point_key.z_key,
                    existing_y_mm: existing.point_key.y_mm,
                    incoming_y_mm: incoming.point_key.y_mm,
                },
            );
        }
        if source_ordering.is_gt() {
            *existing = incoming;
        }
        return Ok(());
    }
    if node_footprint_split_source_ordering(incoming.source, existing.source).is_gt() {
        existing.source = incoming.source;
    }
    Ok(())
}

fn node_footprint_split_source_ordering(
    a: Option<NodeFootprintBoundaryDirectVertex>,
    b: Option<NodeFootprintBoundaryDirectVertex>,
) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => node_footprint_direct_vertex_ordering(a, b),
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (None, None) => std::cmp::Ordering::Equal,
    }
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
    start_split_source: Option<NodeFootprintBoundaryDirectVertex>,
    end_split_source: Option<NodeFootprintBoundaryDirectVertex>,
) -> Option<RoadSurfaceEarthworkFaceSource> {
    source_edges
        .iter()
        .filter_map(|source_edge| {
            node_earthwork_source_edge_for_subsegment(source_edge, start_point_key, end_point_key)
        })
        .min_by(|a, b| a.source_ordering(*b))
        .or_else(|| {
            node_earthwork_source_for_direct_boundary_segment(
                node_id,
                kind,
                start_point_key,
                end_point_key,
                direct_vertex_sources,
            )
        })
        .or_else(|| {
            node_earthwork_source_for_split_boundary_segment(
                node_id,
                kind,
                start_split_source?,
                end_split_source?,
            )
        })
}

fn node_earthwork_source_edge_for_subsegment(
    source_edge: &NodeEarthworkBoundarySourceEdge,
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
) -> Option<RoadSurfaceEarthworkFaceSource> {
    if !arrangement_key_lies_on_segment(
        start_point_key.xz_key(),
        source_edge.start_key,
        source_edge.end_key,
    ) || !arrangement_key_lies_on_segment(
        end_point_key.xz_key(),
        source_edge.start_key,
        source_edge.end_key,
    ) {
        return None;
    }
    Some(RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
        node_id: source_edge.node_id,
        kind: source_edge.kind,
        owner_kind: source_edge.owner_kind,
        owner_index: source_edge.owner_index,
        boundary_source: Some(NodeFootprintBoundarySegmentSource {
            start: node_footprint_boundary_vertex_source_for_edge_point(
                source_edge,
                start_point_key,
            )?,
            end: node_footprint_boundary_vertex_source_for_edge_point(source_edge, end_point_key)?,
        }),
    })
}

fn node_earthwork_source_for_split_boundary_segment(
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    start: NodeFootprintBoundaryDirectVertex,
    end: NodeFootprintBoundaryDirectVertex,
) -> Option<RoadSurfaceEarthworkFaceSource> {
    let owner = if node_footprint_direct_vertex_ordering(start, end).is_ge() {
        start
    } else {
        end
    };
    Some(RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
        node_id,
        kind,
        owner_kind: owner.owner_kind,
        owner_index: owner.owner_index,
        boundary_source: Some(NodeFootprintBoundarySegmentSource {
            start: start.source,
            end: end.source,
        }),
    })
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
) -> Option<RoadSurfaceEarthworkFaceSource> {
    let start =
        node_footprint_boundary_vertex_source_at_point(start_point_key, direct_vertex_sources)?;
    let end = node_footprint_boundary_vertex_source_at_point(end_point_key, direct_vertex_sources)?;
    let owner = if node_footprint_direct_vertex_ordering(start, end).is_ge() {
        start
    } else {
        end
    };
    Some(RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
        node_id,
        kind,
        owner_kind: owner.owner_kind,
        owner_index: owner.owner_index,
        boundary_source: Some(NodeFootprintBoundarySegmentSource {
            start: start.source,
            end: end.source,
        }),
    })
}

fn same_winding_boundary_point_loops_from_loop(points: &[Vector3]) -> Vec<Vec<Vector3>> {
    if !boundary_point_loop_has_repeated_xz(points) {
        return vec![points.to_vec()];
    }

    let source_area_m2 = RoadSurfaceSystem::signed_polygon_area_xz(points);
    split_boundary_point_loop_at_repeated_xz(points.to_vec())
        .into_iter()
        .filter_map(|points| {
            let points = RoadSurfaceSystem::canonicalize_loop_points(points);
            if points.len() < 3 {
                return None;
            }
            let split_area_m2 = RoadSurfaceSystem::signed_polygon_area_xz(&points);
            if split_area_m2.abs() <= boundary_points_numeric_area_budget_m2(&points) {
                return None;
            }
            (source_area_m2.signum() == split_area_m2.signum()).then_some(points)
        })
        .collect()
}

fn split_boundary_point_loop_at_repeated_xz(points: Vec<Vector3>) -> Vec<Vec<Vector3>> {
    let points = RoadSurfaceSystem::canonicalize_loop_points(points);
    if points.len() < 3 {
        return Vec::new();
    }

    let mut loops = Vec::new();
    let mut stack = vec![points[0]];
    let mut seen = BTreeMap::<arrangement::NodeArrangementKey, usize>::new();
    seen.insert(
        ArrangementBoundaryPointKey::from_world(points[0]).xz_key(),
        0,
    );
    for index in 1..=points.len() {
        let current = points[index % points.len()];
        let current_key = ArrangementBoundaryPointKey::from_world(current).xz_key();
        if let Some(start_index) = seen.get(&current_key).copied() {
            let mut cycle = stack[start_index..].to_vec();
            cycle.push(current);
            let cycle = RoadSurfaceSystem::canonicalize_loop_points(cycle);
            if cycle.len() >= 3 {
                loops.push(cycle);
            }
            stack.truncate(start_index + 1);
            if let Some(last) = stack.last_mut() {
                *last = current;
            }
            seen.clear();
            for (stack_index, point) in stack.iter().enumerate() {
                seen.insert(
                    ArrangementBoundaryPointKey::from_world(*point).xz_key(),
                    stack_index,
                );
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

fn boundary_point_loop_has_repeated_xz(points: &[Vector3]) -> bool {
    let mut seen = BTreeSet::new();
    for point in RoadSurfaceSystem::canonicalize_loop_points(points.to_vec()) {
        if !seen.insert(ArrangementBoundaryPointKey::from_world(point).xz_key()) {
            return true;
        }
    }
    false
}
