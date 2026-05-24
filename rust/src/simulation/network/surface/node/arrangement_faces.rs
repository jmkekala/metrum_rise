//! Arrangement face boundary intervals used by node vertical face export.

use super::arrangement::NodeArrangementFace;
use super::boundary_edges::normalized_arrangement_boundary_segment_key;
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct ArrangementFaceBoundaryInterval {
    pub(super) owner: NodeBandOwner,
    pub(super) start: ArrangementSegmentParameter,
    pub(super) end: ArrangementSegmentParameter,
    pub(super) edge_start: ArrangementBoundaryPointKey,
    pub(super) edge_end: ArrangementBoundaryPointKey,
}

pub(super) fn arrangement_owner_face_boundary_intervals_for_segment(
    arrangement: &NodeArrangement,
    owner: NodeBandOwner,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    include_internal_edges: bool,
) -> Vec<ArrangementFaceBoundaryInterval> {
    let mut edge_counts = BTreeMap::<
        (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
        (
            usize,
            ArrangementBoundaryPointKey,
            ArrangementBoundaryPointKey,
        ),
    >::new();
    for face in arrangement
        .faces()
        .iter()
        .filter(|face| face.owner() == owner)
    {
        let Some(face_area_m2) = arrangement_face_area_abs_m2(arrangement, face) else {
            continue;
        };
        if face_area_m2 <= f64::from(NODE_OVERLAY_MIN_AREA_M2) {
            continue;
        }
        let Some(vertices) =
            RoadSurfaceSystem::arrangement_face_canonical_vertex_ids(arrangement, face)
        else {
            continue;
        };
        for index in 0..vertices.len() {
            let Some(edge_start) =
                arrangement_vertex_boundary_point_key(arrangement, vertices[index])
            else {
                continue;
            };
            let Some(edge_end) = arrangement_vertex_boundary_point_key(
                arrangement,
                vertices[(index + 1) % vertices.len()],
            ) else {
                continue;
            };
            let key = normalized_arrangement_boundary_segment_key(edge_start, edge_end);
            edge_counts
                .entry(key)
                .and_modify(|(count, _, _)| *count += 1)
                .or_insert((1, edge_start, edge_end));
        }
    }

    let mut intervals = Vec::new();
    for (_, (count, edge_start, edge_end)) in edge_counts {
        if !include_internal_edges && count != 1 {
            continue;
        }
        if let Some((start, end)) =
            arrangement_face_boundary_overlap_interval(segment_key, edge_start, edge_end)
        {
            intervals.push(ArrangementFaceBoundaryInterval {
                owner,
                start,
                end,
                edge_start,
                edge_end,
            });
        }
    }
    for region in arrangement
        .regions()
        .iter()
        .filter(|region| region.owner() == owner)
    {
        for edge_id in region.boundary_edges() {
            let Some(edge) = arrangement.edges().get(edge_id.index()) else {
                continue;
            };
            if edge.owner() != owner {
                continue;
            }
            let Some(edge_start) = arrangement_vertex_boundary_point_key(arrangement, edge.start())
            else {
                continue;
            };
            let Some(edge_end) = arrangement_vertex_boundary_point_key(arrangement, edge.end())
            else {
                continue;
            };
            if let Some((start, end)) =
                arrangement_face_boundary_overlap_interval(segment_key, edge_start, edge_end)
            {
                intervals.push(ArrangementFaceBoundaryInterval {
                    owner,
                    start,
                    end,
                    edge_start,
                    edge_end,
                });
            }
        }
    }
    intervals.sort();
    intervals.dedup();
    intervals
}

fn arrangement_face_area_abs_m2(
    arrangement: &NodeArrangement,
    face: &NodeArrangementFace,
) -> Option<f64> {
    let vertices = RoadSurfaceSystem::arrangement_face_canonical_vertex_ids(arrangement, face)?;
    let a = arrangement.vertices().get(vertices[0].index())?.point_xz();
    let b = arrangement.vertices().get(vertices[1].index())?.point_xz();
    let c = arrangement.vertices().get(vertices[2].index())?.point_xz();
    let double_area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    Some(double_area.abs() * 0.5)
}

fn arrangement_vertex_boundary_point_key(
    arrangement: &NodeArrangement,
    vertex_id: super::arrangement::NodeArrangementVertexId,
) -> Option<ArrangementBoundaryPointKey> {
    let vertex = arrangement.vertices().get(vertex_id.index())?;
    Some(arrangement_key_boundary_point(
        vertex.key(),
        vertex.height_mm(),
    ))
}

pub(super) fn arrangement_shared_face_boundary_intervals(
    lower_intervals: &[ArrangementFaceBoundaryInterval],
    raised_intervals: &[ArrangementFaceBoundaryInterval],
) -> Vec<(
    ArrangementFaceBoundaryInterval,
    ArrangementFaceBoundaryInterval,
    ArrangementSegmentParameter,
    ArrangementSegmentParameter,
)> {
    let mut shared = Vec::new();
    for lower in lower_intervals {
        for raised in raised_intervals {
            let start = lower.start.max(raised.start);
            let end = lower.end.min(raised.end);
            if end > start {
                shared.push((*lower, *raised, start, end));
            }
        }
    }
    shared.sort_by(|a, b| a.2.cmp(&b.2).then(a.3.cmp(&b.3)));
    shared
}

fn arrangement_face_boundary_overlap_interval(
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    edge_start: ArrangementBoundaryPointKey,
    edge_end: ArrangementBoundaryPointKey,
) -> Option<(ArrangementSegmentParameter, ArrangementSegmentParameter)> {
    let segment_start = arrangement_key_boundary_point(segment_key.0, 0);
    let segment_end = arrangement_key_boundary_point(segment_key.1, 0);
    let edge_start_t = boundary_segment_parameter_xz(edge_start, segment_start, segment_end)?;
    let edge_end_t = boundary_segment_parameter_xz(edge_end, segment_start, segment_end)?;
    let start = edge_start_t
        .min(edge_end_t)
        .max(ArrangementSegmentParameter::zero());
    let end = edge_start_t
        .max(edge_end_t)
        .min(ArrangementSegmentParameter::one());
    (end > start).then_some((start, end))
}

pub(super) fn arrangement_face_boundary_interval_point_at(
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    interval: ArrangementFaceBoundaryInterval,
    parameter: ArrangementSegmentParameter,
) -> Option<ArrangementBoundaryPointKey> {
    let segment_start = arrangement_key_boundary_point(segment_key.0, 0);
    let segment_end = arrangement_key_boundary_point(segment_key.1, 0);
    arrangement_boundary_endpoint_at_parameter(
        interval.edge_start,
        segment_start,
        segment_end,
        parameter,
    )
    .or_else(|| {
        arrangement_boundary_endpoint_at_parameter(
            interval.edge_end,
            segment_start,
            segment_end,
            parameter,
        )
    })
    .or_else(|| {
        arrangement_face_boundary_interpolated_point_at(
            segment_start,
            segment_end,
            interval,
            parameter,
        )
    })
}

fn arrangement_boundary_endpoint_at_parameter(
    endpoint: ArrangementBoundaryPointKey,
    segment_start: ArrangementBoundaryPointKey,
    segment_end: ArrangementBoundaryPointKey,
    parameter: ArrangementSegmentParameter,
) -> Option<ArrangementBoundaryPointKey> {
    let endpoint_t = boundary_segment_parameter_xz(endpoint, segment_start, segment_end)?;
    (endpoint_t.cmp(&parameter) == std::cmp::Ordering::Equal).then_some(endpoint)
}

fn arrangement_face_boundary_interpolated_point_at(
    segment_start: ArrangementBoundaryPointKey,
    segment_end: ArrangementBoundaryPointKey,
    interval: ArrangementFaceBoundaryInterval,
    parameter: ArrangementSegmentParameter,
) -> Option<ArrangementBoundaryPointKey> {
    if parameter < interval.start || parameter > interval.end {
        return None;
    }
    let target = ArrangementBoundaryPointKey {
        x_key: parameter.interpolate_i64(segment_start.x_key, segment_end.x_key),
        z_key: parameter.interpolate_i64(segment_start.z_key, segment_end.z_key),
        y_mm: 0,
    };
    let edge_parameter =
        boundary_segment_parameter_xz(target, interval.edge_start, interval.edge_end)?;
    if edge_parameter < ArrangementSegmentParameter::zero()
        || edge_parameter > ArrangementSegmentParameter::one()
    {
        return None;
    }
    Some(ArrangementBoundaryPointKey {
        x_key: target.x_key,
        z_key: target.z_key,
        y_mm: edge_parameter.interpolate_i64(interval.edge_start.y_mm, interval.edge_end.y_mm),
    })
}

pub(super) fn arrangement_key_boundary_point(
    key: NodeArrangementKey,
    y_mm: i64,
) -> ArrangementBoundaryPointKey {
    ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm,
    }
}
