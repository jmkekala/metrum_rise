//! Node-owned boundary, vertical-face, and visual-piece DTOs.

use super::{
    NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon,
    WORLD_POINT_DEDUP_DISTANCE_M, arrangement,
    backend::{ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2},
    band_semantics::band_kind_sort_key,
    earthwork::{RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkRenderFace},
    keys::{SurfaceSegmentParameter, SurfaceXzKey},
    node_grade::NodeGradeVertexAuthority,
    segments::{exact_line_parameter, interpolate_height_i64, interpolate_key},
    terrain_clip::RoadSurfaceTerrainClipLoop,
};
use godot::prelude::{Vector2, Vector3};
use std::collections::{BTreeMap, BTreeSet};

pub(super) use super::segments::{
    arrangement_key_lies_on_segment,
    arrangement_key_overlay_segment_parameter as arrangement_key_segment_parameter_xz,
};

mod heights;
mod interpolation;
mod sources;
mod support;

use sources::{
    node_footprint_boundary_vertex_source_at_point,
    node_footprint_boundary_vertex_source_for_edge_point,
};

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

#[derive(Clone, Copy, Debug)]
struct NodeFootprintBoundarySplitPoint {
    point_key: ArrangementBoundaryPointKey,
    source: Option<NodeFootprintBoundaryDirectVertex>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoadSurfaceVerticalFaceSource {
    CanonicalStep {
        explicit_vertical_step_index: usize,
        segment: arrangement::NodeExplicitVerticalStepSegment,
    },
}

impl RoadSurfaceVerticalFaceSource {
    pub(crate) fn explicit_vertical_step_index(self) -> Option<usize> {
        match self {
            Self::CanonicalStep {
                explicit_vertical_step_index,
                ..
            } => Some(explicit_vertical_step_index),
        }
    }

    pub(crate) fn segment(self) -> arrangement::NodeExplicitVerticalStepSegment {
        match self {
            Self::CanonicalStep { segment, .. } => segment,
        }
    }

    pub(crate) fn sort_key(
        self,
    ) -> (
        u8,
        arrangement::NodeExplicitVerticalStepSegment,
        Option<usize>,
    ) {
        match self {
            Self::CanonicalStep {
                explicit_vertical_step_index,
                segment,
            } => (0, segment, Some(explicit_vertical_step_index)),
        }
    }
}

/// Explicit visual node piece compiled from the solved roadbed.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceVisualNodePiece {
    /// Owning node id.
    pub node_id: u32,
    /// Piece classification for rendering and debug.
    pub kind: RoadSurfaceVisualNodePieceKind,
    /// Outer piece-owned boundaries used for debug, surface chunk bounds, and terrain clipping.
    pub outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
    /// Explicit asphalt-owned polygons for the node piece.
    pub road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit curb / shoulder-owned polygons for the node piece.
    pub curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit vertical faces at raised owner-pair material contacts.
    pub raised_step_face_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) raised_step_face_sources: Vec<RoadSurfaceVerticalFaceSource>,
    /// Explicit sidewalk-owned polygons for the node piece.
    pub sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) explicit_vertical_step_segments: Vec<arrangement::NodeExplicitVerticalStepSegment>,
    pub(crate) node_grade_authorities: Vec<NodeGradeVertexAuthority>,
    pub(crate) node_top_surface_sources: Vec<NodeTopSurfacePolygonSource>,
    pub(crate) owned_regions: Vec<NodeOwnedRegion>,
    pub(crate) earthwork_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) earthwork_outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) render_earthwork_faces: Vec<RoadSurfaceEarthworkRenderFace>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeOwnedRegion {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner_index: usize,
    pub(crate) polygon: RoadSurfaceVisualPolygon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeTopSurfaceVertexSource {
    pub(crate) grade_authority_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeFootprintBoundaryDirectSource {
    pub(crate) top_surface_source_index: usize,
    pub(crate) grade_authority_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum NodeFootprintBoundaryVertexSource {
    Direct(NodeFootprintBoundaryDirectSource),
    BoundaryInterpolation {
        owning_segment_start: NodeFootprintBoundaryDirectSource,
        owning_segment_end: NodeFootprintBoundaryDirectSource,
        height_mm: i64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeFootprintBoundarySegmentSource {
    pub(crate) start: NodeFootprintBoundaryVertexSource,
    pub(crate) end: NodeFootprintBoundaryVertexSource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTopSurfacePolygonSource {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner_index: usize,
    pub(crate) height_field_id: arrangement::NodeBandHeightFieldId,
    pub(crate) vertex_sources: Vec<NodeTopSurfaceVertexSource>,
    pub(crate) triangle_sources: Vec<[NodeTopSurfaceVertexSource; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeSurfaceRegionResult {
    pub(crate) outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) earthwork_boundary_segments: Vec<Vec<RoadSurfaceEarthworkBoundarySegment>>,
    pub(crate) terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
    pub(crate) road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) raised_step_faces: Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    pub(crate) sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) explicit_vertical_step_segments: Vec<arrangement::NodeExplicitVerticalStepSegment>,
    pub(crate) node_grade_authorities: Vec<NodeGradeVertexAuthority>,
    pub(crate) node_top_surface_sources: Vec<NodeTopSurfacePolygonSource>,
    pub(crate) owned_regions: Vec<NodeOwnedRegion>,
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

pub(super) fn node_earthwork_boundary_segments_from_footprint_loops(
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

fn push_sourced_node_earthwork_boundary_segments(
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
            <= super::SAMPLE_EPSILON_M * super::SAMPLE_EPSILON_M
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

fn insert_node_footprint_boundary_split_point(
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

fn node_footprint_direct_vertex_ordering(
    a: NodeFootprintBoundaryDirectVertex,
    b: NodeFootprintBoundaryDirectVertex,
) -> std::cmp::Ordering {
    band_kind_sort_key(a.owner_kind)
        .cmp(&band_kind_sort_key(b.owner_kind))
        .then(a.owner_index.cmp(&b.owner_index))
        .then(a.source.cmp(&b.source))
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

pub(super) fn interpolated_segment_point_key(
    start: ArrangementBoundaryPointKey,
    end: ArrangementBoundaryPointKey,
    parameter: ArrangementSegmentParameter,
) -> ArrangementBoundaryPointKey {
    let point = interpolate_key(
        boundary_point_surface_key(start),
        boundary_point_surface_key(end),
        parameter,
    );
    ArrangementBoundaryPointKey {
        x_key: point.x_key(),
        z_key: point.z_key(),
        y_mm: interpolated_segment_height_mm(start, end, parameter),
    }
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
