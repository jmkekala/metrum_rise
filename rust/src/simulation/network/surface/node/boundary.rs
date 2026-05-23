//! Node-owned footprint boundary resolution and source-backed terrain seams.

use super::{
    NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind, arrangement,
    backend::{ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2},
    keys::{SurfaceSegmentParameter, SurfaceXzKey},
    piece::{
        NodeFootprintBoundaryDirectSource, NodeFootprintBoundarySegmentSource,
        NodeFootprintBoundaryVertexSource, NodeOwnedRegion, NodeTopSurfacePolygonSource,
    },
    segments::{exact_line_parameter, interpolate_height_i64, overlay_segment_parameter},
};
use godot::prelude::{Vector2, Vector3};
use std::collections::BTreeMap;

pub(super) use super::segments::{
    arrangement_key, arrangement_key_lies_on_segment as arrangement_key_lies_on_segment_xz,
    key_lies_exactly_on_segment,
};

mod earthwork_segments;
mod heights;
mod interpolation;
mod sources;
mod support;

pub(super) use earthwork_segments::node_earthwork_boundary_segments_from_footprint_loops;
pub(super) use earthwork_segments::same_winding_boundary_point_loops_from_loop;
#[cfg(test)]
use earthwork_segments::{
    NodeFootprintBoundarySplitPoint, insert_node_footprint_boundary_split_point,
    push_sourced_node_earthwork_boundary_segments,
};
#[cfg(test)]
use sources::node_footprint_boundary_vertex_source_for_edge_point;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct ArrangementBoundaryPointKey {
    pub(super) x_key: i64,
    pub(super) z_key: i64,
    pub(super) y_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NodeFootprintBoundaryPoint {
    pub(super) point_key: ArrangementBoundaryPointKey,
}

pub(super) type ArrangementSegmentParameter = SurfaceSegmentParameter;
const BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS: i64 = 128;

impl ArrangementBoundaryPointKey {
    pub(super) fn from_world(point: Vector3) -> Self {
        Self {
            x_key: (f64::from(point.x) * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
            z_key: (f64::from(point.z) * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
            y_mm: (point.y * 1000.0).round() as i64,
        }
    }

    pub(super) fn xz_key(self) -> arrangement::NodeArrangementKey {
        arrangement::NodeArrangementKey::from_point(RoadVec2::new(
            self.x_key as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
            self.z_key as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
        ))
    }
}

impl NodeFootprintBoundaryPoint {
    pub(super) fn new(point_key: ArrangementBoundaryPointKey) -> Self {
        Self { point_key }
    }

    pub(super) fn point_world(self) -> Vector3 {
        arrangement_boundary_point_to_world(self.point_key)
    }

    pub(super) fn xz_key(self) -> arrangement::NodeArrangementKey {
        self.point_key.xz_key()
    }
}

#[derive(Clone, Copy, Debug)]
struct NodeEarthworkBoundarySourceEdge {
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    start_key: arrangement::NodeArrangementKey,
    end_key: arrangement::NodeArrangementKey,
    final_footprint_boundary: bool,
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    owner_kind: RoadSurfaceBandKind,
    owner_index: usize,
    height_field_id: arrangement::NodeBandHeightFieldId,
    start_source: NodeFootprintBoundaryDirectSource,
    end_source: NodeFootprintBoundaryDirectSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeFootprintBoundaryDirectVertex {
    source: NodeFootprintBoundaryVertexSource,
    owner_kind: RoadSurfaceBandKind,
    owner_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeFootprintBoundaryHeightCandidate {
    height_mm: i64,
    source: NodeFootprintBoundaryDirectVertex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeFootprintBoundaryEdgeHeightCandidate {
    height: NodeFootprintBoundaryHeightCandidate,
    final_footprint_boundary: bool,
}

pub(super) struct NodeFootprintBoundaryExportSources {
    source_edges: Vec<NodeEarthworkBoundarySourceEdge>,
    direct_vertex_sources: BTreeMap<ArrangementBoundaryPointKey, NodeFootprintBoundaryDirectVertex>,
    direct_vertex_source_candidates:
        BTreeMap<ArrangementBoundaryPointKey, Vec<NodeFootprintBoundaryDirectVertex>>,
    direct_vertex_source_conflicts:
        BTreeMap<ArrangementBoundaryPointKey, NodeFootprintBoundaryDirectVertexConflict>,
    explicit_vertical_step_segments: Vec<arrangement::NodeExplicitVerticalStepSegment>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeFootprintBoundaryDirectVertexConflict {
    existing: NodeFootprintBoundaryDirectVertex,
    incoming: NodeFootprintBoundaryDirectVertex,
}

#[derive(Debug)]
pub(crate) enum NodeBoundaryExportError {
    EmptyOuterBoundary,
    MissingFootprintBoundaryHeight {
        x_key: i64,
        z_key: i64,
    },
    ConflictingFootprintBoundaryHeight {
        x_key: i64,
        z_key: i64,
        existing_y_mm: i64,
        incoming_y_mm: i64,
        existing_owner_kind: RoadSurfaceBandKind,
        existing_owner_index: usize,
        existing_source: NodeFootprintBoundaryVertexSource,
        incoming_owner_kind: RoadSurfaceBandKind,
        incoming_owner_index: usize,
        incoming_source: NodeFootprintBoundaryVertexSource,
    },
    ConflictingFootprintBoundarySplitHeight {
        x_key: i64,
        z_key: i64,
        existing_y_mm: i64,
        incoming_y_mm: i64,
    },
    AmbiguousEarthworkBoundarySegmentSource {
        start_x_key: i64,
        start_z_key: i64,
        end_x_key: i64,
        end_z_key: i64,
        existing_source: RoadSurfaceEarthworkFaceSource,
        incoming_source: RoadSurfaceEarthworkFaceSource,
    },
    AmbiguousFootprintBoundaryPointSource {
        x_key: i64,
        z_key: i64,
        y_mm: i64,
        existing_owner_kind: RoadSurfaceBandKind,
        existing_owner_index: usize,
        existing_source: NodeFootprintBoundaryVertexSource,
        incoming_owner_kind: RoadSurfaceBandKind,
        incoming_owner_index: usize,
        incoming_source: NodeFootprintBoundaryVertexSource,
    },
    DegenerateOuterBoundaryLoop,
    MissingEarthworkBoundarySource,
    MissingEarthworkBoundarySegmentSource {
        start_x_key: i64,
        start_z_key: i64,
        end_x_key: i64,
        end_z_key: i64,
        nearby_source_edges: Vec<((i64, i64), (i64, i64), RoadSurfaceBandKind, usize, bool)>,
    },
    MissingNodeTopSurfaceGradeAuthority,
}

pub(super) fn remove_subbudget_unsupported_numeric_boundary_vertices<F>(
    points: &mut Vec<NodeFootprintBoundaryPoint>,
    mut should_keep_vertex: F,
) where
    F: FnMut(ArrangementBoundaryPointKey, [Vector3; 3]) -> bool,
{
    loop {
        if points.len() < 4 {
            return;
        }
        let mut removed = false;
        for index in 0..points.len() {
            let previous = if index == 0 {
                points.len() - 1
            } else {
                index - 1
            };
            let next = if index + 1 == points.len() {
                0
            } else {
                index + 1
            };
            let local_points = [
                points[previous].point_world(),
                points[index].point_world(),
                points[next].point_world(),
            ];
            let current_point_key = points[index].point_key;
            if should_keep_vertex(current_point_key, local_points) {
                continue;
            }
            points.remove(index);
            removed = true;
            break;
        }
        if !removed {
            return;
        }
    }
}

fn arrangement_key_lies_exactly_on_segment(
    point: arrangement::NodeArrangementKey,
    start: arrangement::NodeArrangementKey,
    end: arrangement::NodeArrangementKey,
) -> bool {
    key_lies_exactly_on_segment(
        arrangement_key(point),
        arrangement_key(start),
        arrangement_key(end),
    )
}

fn arrangement_key_lies_on_segment(
    point: arrangement::NodeArrangementKey,
    start: arrangement::NodeArrangementKey,
    end: arrangement::NodeArrangementKey,
) -> bool {
    arrangement_key_lies_on_segment_xz(point, start, end)
}

fn arrangement_key_segment_parameter_xz(
    point: arrangement::NodeArrangementKey,
    start: arrangement::NodeArrangementKey,
    end: arrangement::NodeArrangementKey,
) -> Option<ArrangementSegmentParameter> {
    let point_key = arrangement_key(point);
    let start_key = arrangement_key(start);
    let end_key = arrangement_key(end);
    overlay_segment_parameter(point_key, start_key, end_key)
}

fn arrangement_key_segment_parameter_xz_with_endpoint_dust(
    point: arrangement::NodeArrangementKey,
    start: arrangement::NodeArrangementKey,
    end: arrangement::NodeArrangementKey,
) -> Option<ArrangementSegmentParameter> {
    arrangement_key_segment_parameter_xz(point, start, end).or_else(|| {
        endpoint_dust_segment_parameter(
            arrangement_key(point),
            arrangement_key(start),
            arrangement_key(end),
        )
    })
}

fn endpoint_dust_segment_parameter(
    point: SurfaceXzKey,
    start: SurfaceXzKey,
    end: SurfaceXzKey,
) -> Option<ArrangementSegmentParameter> {
    if start == end || !point.collinear_with_overlay_grid_segment(start, end) {
        return None;
    }
    if key_distance_squared(point, start)
        <= i128::from(BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS)
            * i128::from(BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS)
    {
        return Some(ArrangementSegmentParameter::zero());
    }
    if key_distance_squared(point, end)
        <= i128::from(BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS)
            * i128::from(BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS)
    {
        return Some(ArrangementSegmentParameter::one());
    }
    None
}

fn key_distance_squared(a: SurfaceXzKey, b: SurfaceXzKey) -> i128 {
    let dx = i128::from(a.x_key() - b.x_key());
    let dz = i128::from(a.z_key() - b.z_key());
    dx * dx + dz * dz
}

fn node_footprint_direct_vertices_share_source_identity(
    a: NodeFootprintBoundaryDirectVertex,
    b: NodeFootprintBoundaryDirectVertex,
) -> bool {
    node_footprint_direct_vertices_share_owner_identity(a, b)
        && node_footprint_boundary_vertex_sources_share_identity(a.source, b.source)
}

fn node_footprint_direct_vertices_share_boundary_point_authority(
    point_key: ArrangementBoundaryPointKey,
    a: NodeFootprintBoundaryDirectVertex,
    b: NodeFootprintBoundaryDirectVertex,
) -> bool {
    node_footprint_direct_vertices_share_source_identity(a, b)
        || (node_footprint_direct_vertices_share_owner_identity(a, b)
            && node_footprint_boundary_vertex_sources_share_boundary_point_authority(
                point_key, a.source, b.source,
            ))
}

fn node_footprint_direct_vertices_share_owner_identity(
    a: NodeFootprintBoundaryDirectVertex,
    b: NodeFootprintBoundaryDirectVertex,
) -> bool {
    a.owner_kind == b.owner_kind && a.owner_index == b.owner_index
}

fn node_footprint_boundary_vertex_sources_share_identity(
    a: NodeFootprintBoundaryVertexSource,
    b: NodeFootprintBoundaryVertexSource,
) -> bool {
    match (a, b) {
        (
            NodeFootprintBoundaryVertexSource::Direct(a),
            NodeFootprintBoundaryVertexSource::Direct(b),
        ) => a.grade_authority_index == b.grade_authority_index,
        (
            NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                x_key: a_x_key,
                z_key: a_z_key,
                y_mm: a_y_mm,
            },
            NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                x_key: b_x_key,
                z_key: b_z_key,
                y_mm: b_y_mm,
            },
        ) => a_x_key == b_x_key && a_z_key == b_z_key && a_y_mm == b_y_mm,
        (
            NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                owning_segment_start: a_start,
                owning_segment_end: a_end,
                height_mm: a_height_mm,
            },
            NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                owning_segment_start: b_start,
                owning_segment_end: b_end,
                height_mm: b_height_mm,
            },
        ) => {
            a_height_mm == b_height_mm
                && ((a_start.grade_authority_index == b_start.grade_authority_index
                    && a_end.grade_authority_index == b_end.grade_authority_index)
                    || (a_start.grade_authority_index == b_end.grade_authority_index
                        && a_end.grade_authority_index == b_start.grade_authority_index))
        }
        _ => false,
    }
}

fn node_footprint_boundary_vertex_sources_share_boundary_point_authority(
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
        (
            NodeFootprintBoundaryVertexSource::Direct(_),
            NodeFootprintBoundaryVertexSource::BoundaryInterpolation { height_mm, .. },
        )
        | (
            NodeFootprintBoundaryVertexSource::BoundaryInterpolation { height_mm, .. },
            NodeFootprintBoundaryVertexSource::Direct(_),
        ) => height_mm == point_key.y_mm,
        (
            NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                height_mm: a_height_mm,
                ..
            },
            NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                height_mm: b_height_mm,
                ..
            },
        ) => a_height_mm == point_key.y_mm && b_height_mm == point_key.y_mm,
        _ => false,
    }
}

fn ambiguous_footprint_boundary_point_source_error(
    point_key: ArrangementBoundaryPointKey,
    existing: NodeFootprintBoundaryDirectVertex,
    incoming: NodeFootprintBoundaryDirectVertex,
) -> NodeBoundaryExportError {
    NodeBoundaryExportError::AmbiguousFootprintBoundaryPointSource {
        x_key: point_key.x_key,
        z_key: point_key.z_key,
        y_mm: point_key.y_mm,
        existing_owner_kind: existing.owner_kind,
        existing_owner_index: existing.owner_index,
        existing_source: existing.source,
        incoming_owner_kind: incoming.owner_kind,
        incoming_owner_index: incoming.owner_index,
        incoming_source: incoming.source,
    }
}

fn merge_node_footprint_boundary_point_source(
    point_key: ArrangementBoundaryPointKey,
    source: &mut Option<NodeFootprintBoundaryDirectVertex>,
    candidate: NodeFootprintBoundaryDirectVertex,
) -> Result<(), NodeBoundaryExportError> {
    let Some(existing) = *source else {
        *source = Some(candidate);
        return Ok(());
    };
    if node_footprint_direct_vertices_share_source_identity(existing, candidate) {
        return Ok(());
    }
    if node_footprint_direct_vertices_share_boundary_point_authority(point_key, existing, candidate)
    {
        *source = Some(NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                x_key: point_key.x_key,
                z_key: point_key.z_key,
                y_mm: point_key.y_mm,
            },
            owner_kind: existing.owner_kind,
            owner_index: existing.owner_index,
        });
        return Ok(());
    }
    Err(ambiguous_footprint_boundary_point_source_error(
        point_key, existing, candidate,
    ))
}

pub(super) fn boundary_segment_parameter_xz(
    point: ArrangementBoundaryPointKey,
    start: ArrangementBoundaryPointKey,
    end: ArrangementBoundaryPointKey,
) -> Option<ArrangementSegmentParameter> {
    exact_line_parameter(
        boundary_point_surface_key(point),
        boundary_point_surface_key(start),
        boundary_point_surface_key(end),
    )
}

pub(super) fn interpolated_segment_height_mm(
    start: ArrangementBoundaryPointKey,
    end: ArrangementBoundaryPointKey,
    parameter: ArrangementSegmentParameter,
) -> i64 {
    interpolate_height_i64(start.y_mm, end.y_mm, parameter)
}

pub(super) fn arrangement_boundary_point_to_world(point: ArrangementBoundaryPointKey) -> Vector3 {
    Vector3::new(
        (point.x_key as f64 / ROAD_OVERLAY_COORDINATE_SCALE) as f32,
        point.y_mm as f32 / 1000.0,
        (point.z_key as f64 / ROAD_OVERLAY_COORDINATE_SCALE) as f32,
    )
}

pub(super) fn boundary_points_numeric_area_budget_m2(points: &[Vector3]) -> f32 {
    if points.len() < 2 {
        return NODE_OVERLAY_MIN_AREA_M2;
    }
    let perimeter_m = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(start, end)| Vector2::new(start.x - end.x, start.z - end.z).length())
        .sum::<f32>();
    RoadSurfaceSystem::overlay_numeric_area_budget_m2(perimeter_m, points.len())
}

fn boundary_point_surface_key(point: ArrangementBoundaryPointKey) -> SurfaceXzKey {
    SurfaceXzKey::from_raw_keys(point.x_key, point.z_key)
}

#[cfg(test)]
mod tests;
