//! Node-owned footprint boundary resolution and source-backed terrain seams.

use super::{
    NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind, WORLD_POINT_DEDUP_DISTANCE_M, arrangement,
    backend::{ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2},
    band_semantics::band_kind_sort_key,
    keys::{SurfaceSegmentParameter, SurfaceXzKey},
    piece::{
        NodeFootprintBoundaryDirectSource, NodeFootprintBoundarySegmentSource,
        NodeFootprintBoundaryVertexSource, NodeOwnedRegion, NodeTopSurfacePolygonSource,
    },
    segments::{exact_line_parameter, interpolate_height_i64},
};
use godot::prelude::{Vector2, Vector3};
use std::collections::BTreeMap;

pub(super) use super::segments::{
    arrangement_key_lies_on_segment,
    arrangement_key_overlay_segment_parameter as arrangement_key_segment_parameter_xz,
};

mod earthwork_segments;
mod heights;
mod interpolation;
mod sources;
mod support;

#[cfg(test)]
use super::RoadSurfaceEarthworkFaceSource;
pub(super) use earthwork_segments::node_earthwork_boundary_segments_from_footprint_loops;
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

pub(super) type ArrangementSegmentParameter = SurfaceSegmentParameter;

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

#[derive(Clone, Copy, Debug)]
struct NodeEarthworkBoundarySourceEdge {
    start_point_key: ArrangementBoundaryPointKey,
    end_point_key: ArrangementBoundaryPointKey,
    start_key: arrangement::NodeArrangementKey,
    end_key: arrangement::NodeArrangementKey,
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    owner_kind: RoadSurfaceBandKind,
    owner_index: usize,
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

pub(super) struct NodeFootprintBoundaryExportSources {
    source_edges: Vec<NodeEarthworkBoundarySourceEdge>,
    direct_vertex_sources: BTreeMap<ArrangementBoundaryPointKey, NodeFootprintBoundaryDirectVertex>,
    explicit_vertical_step_segments: Vec<arrangement::NodeExplicitVerticalStepSegment>,
}

#[derive(Debug)]
pub(crate) enum NodeBoundaryExportError {
    EmptyOuterBoundary,
    MissingFootprintBoundaryHeight,
    ConflictingFootprintBoundaryHeight {
        x_key: i64,
        z_key: i64,
    },
    ConflictingFootprintBoundarySplitHeight {
        x_key: i64,
        z_key: i64,
        existing_y_mm: i64,
        incoming_y_mm: i64,
    },
    DegenerateOuterBoundaryLoop,
    MissingEarthworkBoundarySource,
    MissingNodeTopSurfaceGradeAuthority,
}

pub(super) fn remove_subbudget_unsupported_numeric_boundary_vertices<F>(
    points: &mut Vec<Vector3>,
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
            let local_points = [points[previous], points[index], points[next]];
            let current_point_key = ArrangementBoundaryPointKey::from_world(points[index]);
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

fn node_footprint_direct_vertex_ordering(
    a: NodeFootprintBoundaryDirectVertex,
    b: NodeFootprintBoundaryDirectVertex,
) -> std::cmp::Ordering {
    band_kind_sort_key(a.owner_kind)
        .cmp(&band_kind_sort_key(b.owner_kind))
        .then(a.owner_index.cmp(&b.owner_index))
        .then(a.source.cmp(&b.source))
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

fn arrangement_key_distance_m(
    a: arrangement::NodeArrangementKey,
    b: arrangement::NodeArrangementKey,
) -> f64 {
    let dx = (a.x_key() - b.x_key()) as f64 / ROAD_OVERLAY_COORDINATE_SCALE;
    let dz = (a.z_key() - b.z_key()) as f64 / ROAD_OVERLAY_COORDINATE_SCALE;
    (dx * dx + dz * dz).sqrt()
}

fn arrangement_key_segment_parameter_with_canonical_drift(
    point: arrangement::NodeArrangementKey,
    start: arrangement::NodeArrangementKey,
    end: arrangement::NodeArrangementKey,
) -> Option<ArrangementSegmentParameter> {
    // This is only for independent overlay projection drift around an already-owned source edge.
    // Interior drift must project inside the source segment; endpoint extension drift is accepted
    // only inside the project point-dedup radius of the actual source endpoint.
    let dx = i128::from(end.x_key() - start.x_key());
    let dz = i128::from(end.z_key() - start.z_key());
    let length_squared = dx * dx + dz * dz;
    if length_squared == 0 {
        return None;
    }
    let px = i128::from(point.x_key() - start.x_key());
    let pz = i128::from(point.z_key() - start.z_key());
    let projected_numerator = px * dx + pz * dz;
    let numerator = if projected_numerator < 0 {
        if arrangement_key_distance_m(point, start) > f64::from(WORLD_POINT_DEDUP_DISTANCE_M) {
            return None;
        }
        0
    } else if projected_numerator > length_squared {
        if arrangement_key_distance_m(point, end) > f64::from(WORLD_POINT_DEDUP_DISTANCE_M) {
            return None;
        }
        length_squared
    } else {
        if arrangement_key_segment_distance_m(point, start, end)
            > f64::from(WORLD_POINT_DEDUP_DISTANCE_M)
        {
            return None;
        }
        projected_numerator
    };
    ArrangementSegmentParameter::new(numerator, length_squared)
}

fn arrangement_key_segment_distance_m(
    point: arrangement::NodeArrangementKey,
    start: arrangement::NodeArrangementKey,
    end: arrangement::NodeArrangementKey,
) -> f64 {
    let px = point.x_key() as f64 / ROAD_OVERLAY_COORDINATE_SCALE;
    let pz = point.z_key() as f64 / ROAD_OVERLAY_COORDINATE_SCALE;
    let sx = start.x_key() as f64 / ROAD_OVERLAY_COORDINATE_SCALE;
    let sz = start.z_key() as f64 / ROAD_OVERLAY_COORDINATE_SCALE;
    let ex = end.x_key() as f64 / ROAD_OVERLAY_COORDINATE_SCALE;
    let ez = end.z_key() as f64 / ROAD_OVERLAY_COORDINATE_SCALE;
    let dx = ex - sx;
    let dz = ez - sz;
    let length_squared = dx * dx + dz * dz;
    let t = if length_squared > f64::EPSILON {
        (((px - sx) * dx + (pz - sz) * dz) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let closest_x = sx + dx * t;
    let closest_z = sz + dz * t;
    let distance_x = px - closest_x;
    let distance_z = pz - closest_z;
    (distance_x * distance_x + distance_z * distance_z).sqrt()
}

#[cfg(test)]
mod tests;
