//! Node-owned boundary, vertical-face, and visual-piece DTOs.

use super::{
    NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon, arrangement,
    backend::{ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2},
    band_semantics::{band_kind_sort_key, raised_step_band_rank, raised_step_kinds_can_contact},
    earthwork::{RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkRenderFace},
    keys::{SurfaceSegmentParameter, SurfaceXzKey},
    node_grade::NodeGradeVertexAuthority,
    segments::{exact_line_parameter, interpolate_height_i64, interpolate_key},
    terrain_clip::RoadSurfaceTerrainClipLoop,
};
use godot::prelude::{Vector2, Vector3};
use std::collections::{BTreeMap, BTreeSet};

pub(super) use super::segments::{
    arrangement_key_lies_exactly_on_segment, arrangement_key_lies_on_segment,
    arrangement_key_overlay_segment_parameter as arrangement_key_segment_parameter_xz,
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

#[derive(Clone, Copy, Debug)]
struct NodeFootprintBoundaryHeightCandidate {
    height_mm: i64,
    source: NodeFootprintBoundaryDirectVertex,
    source_edge: Option<(
        arrangement::NodeArrangementKey,
        arrangement::NodeArrangementKey,
    )>,
}

#[derive(Clone, Copy, Debug)]
struct NodeFootprintBoundarySplitPoint {
    point_key: ArrangementBoundaryPointKey,
    source: Option<NodeFootprintBoundaryDirectVertex>,
}

pub(super) struct NodeFootprintBoundaryExportSources {
    source_edges: Vec<NodeEarthworkBoundarySourceEdge>,
    direct_vertex_sources: BTreeMap<ArrangementBoundaryPointKey, NodeFootprintBoundaryDirectVertex>,
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
    FinalOwnedBoundary {
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
            Self::FinalOwnedBoundary { .. } => None,
        }
    }

    pub(crate) fn segment(self) -> arrangement::NodeExplicitVerticalStepSegment {
        match self {
            Self::CanonicalStep { segment, .. } | Self::FinalOwnedBoundary { segment } => segment,
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
            Self::FinalOwnedBoundary { segment } => (1, segment, None),
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

impl NodeFootprintBoundaryExportSources {
    pub(super) fn from_owned_regions(
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        owned_regions: &[NodeOwnedRegion],
        node_top_surface_sources: &[NodeTopSurfacePolygonSource],
    ) -> Result<Self, NodeBoundaryExportError> {
        Ok(Self {
            source_edges: node_earthwork_boundary_source_edges_from_owned_regions(
                node_id,
                kind,
                owned_regions,
                node_top_surface_sources,
            )?,
            direct_vertex_sources: node_footprint_boundary_direct_vertex_sources(
                owned_regions,
                node_top_surface_sources,
            )?,
        })
    }

    pub(super) fn height_mm_at_key(
        &mut self,
        key: arrangement::NodeArrangementKey,
        adjacent_keys: [arrangement::NodeArrangementKey; 2],
    ) -> Result<Option<i64>, NodeBoundaryExportError> {
        let Some(candidate) = self.height_candidate_at_boundary_vertex(key, adjacent_keys)? else {
            return Ok(None);
        };
        self.insert_boundary_vertex_source(key, candidate.height_mm, candidate.source);
        Ok(Some(candidate.height_mm))
    }

    pub(super) fn has_final_owned_footprint_boundary_support_at_point(
        &self,
        point_key: ArrangementBoundaryPointKey,
    ) -> bool {
        self.direct_vertex_sources
            .get(&point_key)
            .is_some_and(|source| {
                matches!(source.source, NodeFootprintBoundaryVertexSource::Direct(_))
            })
            || self.source_edges.iter().any(|source_edge| {
                node_footprint_boundary_vertex_source_for_edge_point(source_edge, point_key)
                    .is_some()
            })
    }

    fn height_candidate_at_boundary_vertex(
        &self,
        key: arrangement::NodeArrangementKey,
        adjacent_keys: [arrangement::NodeArrangementKey; 2],
    ) -> Result<Option<NodeFootprintBoundaryHeightCandidate>, NodeBoundaryExportError> {
        let edge_candidates = self
            .boundary_edge_height_candidates_at_key(key)
            .filter(|candidate| {
                candidate.source_edge.is_some_and(|(start, end)| {
                    adjacent_keys.iter().copied().any(|adjacent_key| {
                        adjacent_key != key
                            && arrangement_key_lies_on_segment(adjacent_key, start, end)
                    })
                })
            })
            .collect::<Vec<_>>();
        if !edge_candidates.is_empty() {
            return self.unique_height_candidate_at_key(key, edge_candidates);
        }
        self.unique_height_candidate_at_key(
            key,
            self.direct_height_candidates_at_key(key)
                .collect::<Vec<_>>(),
        )
    }

    fn unique_height_candidate_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
        candidates: Vec<NodeFootprintBoundaryHeightCandidate>,
    ) -> Result<Option<NodeFootprintBoundaryHeightCandidate>, NodeBoundaryExportError> {
        if candidates.is_empty() {
            return Ok(None);
        }

        let mut heights = candidates
            .iter()
            .map(|candidate| candidate.height_mm)
            .collect::<Vec<_>>();
        heights.sort_unstable();
        heights.dedup();
        if heights.len() != 1 {
            if let Some(candidate) = raised_step_footprint_height_candidate(&candidates, &heights) {
                return Ok(Some(candidate));
            }
            return Err(
                NodeBoundaryExportError::ConflictingFootprintBoundaryHeight {
                    x_key: key.x_key(),
                    z_key: key.z_key(),
                },
            );
        }

        Ok(candidates
            .into_iter()
            .max_by(|a, b| node_footprint_direct_vertex_ordering(a.source, b.source))
            .map(|candidate| candidate))
    }

    pub(super) fn interpolate_missing_footprint_boundary_heights(
        &mut self,
        vertices: &mut [(arrangement::NodeArrangementKey, Option<i64>)],
    ) -> Result<(), NodeBoundaryExportError> {
        let Some(_first_missing_key) = vertices
            .iter()
            .find_map(|(key, height_mm)| height_mm.is_none().then_some(*key))
        else {
            return Ok(());
        };
        let Some(first_solved_index) = vertices
            .iter()
            .position(|(_, height_mm)| height_mm.is_some())
        else {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
        };
        if vertices
            .iter()
            .filter(|(_, height_mm)| height_mm.is_some())
            .count()
            < 2
        {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
        }

        let mut ordered_indices = Vec::with_capacity(vertices.len() + 1);
        ordered_indices.extend(first_solved_index..vertices.len());
        ordered_indices.extend(0..=first_solved_index);

        let mut start_pos = 0;
        while start_pos + 1 < ordered_indices.len() {
            let start_index = ordered_indices[start_pos];
            let Some(start_height_mm) = vertices[start_index].1 else {
                return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
            };
            let Some(end_pos) = (start_pos + 1..ordered_indices.len())
                .find(|pos| vertices[ordered_indices[*pos]].1.is_some())
            else {
                return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
            };
            if end_pos == start_pos + 1 {
                start_pos = end_pos;
                continue;
            }

            let end_index = ordered_indices[end_pos];
            let Some(end_height_mm) = vertices[end_index].1 else {
                return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
            };
            let Some(start_candidate) =
                self.height_candidate_at_point(vertices[start_index].0, start_height_mm)?
            else {
                return Err(NodeBoundaryExportError::MissingEarthworkBoundarySource);
            };
            let Some(end_candidate) =
                self.height_candidate_at_point(vertices[end_index].0, end_height_mm)?
            else {
                return Err(NodeBoundaryExportError::MissingEarthworkBoundarySource);
            };

            let mut cumulative_lengths = Vec::with_capacity(end_pos - start_pos + 1);
            cumulative_lengths.push(0.0);
            let mut total_length_m = 0.0;
            for pair_pos in start_pos..end_pos {
                total_length_m += arrangement_key_distance_m(
                    vertices[ordered_indices[pair_pos]].0,
                    vertices[ordered_indices[pair_pos + 1]].0,
                );
                cumulative_lengths.push(total_length_m);
            }
            if total_length_m <= f64::EPSILON {
                return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
            }

            for run_offset in 1..cumulative_lengths.len() - 1 {
                let index = ordered_indices[start_pos + run_offset];
                let t = cumulative_lengths[run_offset] / total_length_m;
                let height_mm = (start_height_mm as f64
                    + (end_height_mm - start_height_mm) as f64 * t)
                    .round() as i64;
                vertices[index].1 = Some(height_mm);
                self.insert_contour_interpolated_boundary_source(
                    vertices[index].0,
                    height_mm,
                    start_candidate,
                    end_candidate,
                );
            }
            start_pos = end_pos;
        }
        Ok(())
    }

    fn height_candidate_at_point(
        &self,
        key: arrangement::NodeArrangementKey,
        height_mm: i64,
    ) -> Result<Option<NodeFootprintBoundaryHeightCandidate>, NodeBoundaryExportError> {
        let point_key = ArrangementBoundaryPointKey::from_world(
            arrangement_boundary_point_to_world(ArrangementBoundaryPointKey {
                x_key: key.x_key(),
                z_key: key.z_key(),
                y_mm: height_mm,
            }),
        );
        if let Some(source) = self.direct_vertex_sources.get(&point_key).copied() {
            return Ok(Some(NodeFootprintBoundaryHeightCandidate {
                height_mm,
                source,
                source_edge: None,
            }));
        }
        Ok(self
            .unique_height_candidate_at_key(
                key,
                self.height_candidates_at_key(key)
                    .filter(|candidate| candidate.height_mm == height_mm)
                    .collect::<Vec<_>>(),
            )?
            .filter(|candidate| candidate.height_mm == height_mm))
    }

    fn insert_contour_interpolated_boundary_source(
        &mut self,
        key: arrangement::NodeArrangementKey,
        height_mm: i64,
        start: NodeFootprintBoundaryHeightCandidate,
        end: NodeFootprintBoundaryHeightCandidate,
    ) {
        let owner = if node_footprint_direct_vertex_ordering(start.source, end.source).is_ge() {
            start.source
        } else {
            end.source
        };
        let candidate = NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                owning_segment_start: node_footprint_boundary_source_start_direct_source(
                    start.source.source,
                ),
                owning_segment_end: node_footprint_boundary_source_end_direct_source(
                    end.source.source,
                ),
                height_mm,
            },
            owner_kind: owner.owner_kind,
            owner_index: owner.owner_index,
        };
        self.insert_boundary_vertex_source(key, height_mm, candidate);
    }

    fn insert_boundary_vertex_source(
        &mut self,
        key: arrangement::NodeArrangementKey,
        height_mm: i64,
        candidate: NodeFootprintBoundaryDirectVertex,
    ) {
        let point_key = ArrangementBoundaryPointKey::from_world(
            arrangement_boundary_point_to_world(ArrangementBoundaryPointKey {
                x_key: key.x_key(),
                z_key: key.z_key(),
                y_mm: height_mm,
            }),
        );
        self.direct_vertex_sources
            .entry(point_key)
            .and_modify(|current| {
                if node_footprint_direct_vertex_ordering(candidate, *current).is_gt() {
                    *current = candidate;
                }
            })
            .or_insert(candidate);
    }

    fn height_candidates_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> impl Iterator<Item = NodeFootprintBoundaryHeightCandidate> + '_ {
        self.direct_height_candidates_at_key(key)
            .chain(self.boundary_edge_height_candidates_at_key(key))
    }

    fn boundary_edge_height_candidates_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> impl Iterator<Item = NodeFootprintBoundaryHeightCandidate> + '_ {
        self.source_edges.iter().filter_map(move |source_edge| {
            if !arrangement_key_lies_on_segment(key, source_edge.start_key, source_edge.end_key) {
                return None;
            }
            let parameter = arrangement_key_segment_parameter_xz(
                key,
                source_edge.start_key,
                source_edge.end_key,
            )?;
            let height_mm = interpolated_segment_height_mm(
                source_edge.start_point_key,
                source_edge.end_point_key,
                parameter,
            );
            Some(NodeFootprintBoundaryHeightCandidate {
                height_mm,
                source: NodeFootprintBoundaryDirectVertex {
                    source: node_footprint_boundary_vertex_source_for_edge_point(
                        source_edge,
                        ArrangementBoundaryPointKey {
                            x_key: key.x_key(),
                            z_key: key.z_key(),
                            y_mm: height_mm,
                        },
                    )?,
                    owner_kind: source_edge.owner_kind,
                    owner_index: source_edge.owner_index,
                },
                source_edge: Some((source_edge.start_key, source_edge.end_key)),
            })
        })
    }

    fn direct_height_candidates_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> impl Iterator<Item = NodeFootprintBoundaryHeightCandidate> + '_ {
        self.direct_vertex_sources
            .iter()
            .filter(move |(point_key, _)| {
                point_key.x_key == key.x_key() && point_key.z_key == key.z_key()
            })
            .map(|(point_key, source)| NodeFootprintBoundaryHeightCandidate {
                height_mm: point_key.y_mm,
                source: *source,
                source_edge: None,
            })
    }
}

pub(super) fn remove_unsupported_numeric_boundary_vertices<F>(
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

fn node_earthwork_boundary_source_edges_from_owned_regions(
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    owned_regions: &[NodeOwnedRegion],
    node_top_surface_sources: &[NodeTopSurfacePolygonSource],
) -> Result<Vec<NodeEarthworkBoundarySourceEdge>, NodeBoundaryExportError> {
    if owned_regions.len() != node_top_surface_sources.len() {
        return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
    }
    let mut source_edges = Vec::new();
    for (region_index, region) in owned_regions.iter().enumerate() {
        let Some(top_source) = node_top_surface_sources.get(region_index) else {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        };
        let points = &region.polygon.points_world;
        if points.len() < 3 {
            continue;
        }
        if top_source.vertex_sources.len() != points.len() {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        }
        for index in 0..points.len() {
            let start_point_key = ArrangementBoundaryPointKey::from_world(points[index]);
            let end_point_key =
                ArrangementBoundaryPointKey::from_world(points[(index + 1) % points.len()]);
            let start_key = start_point_key.xz_key();
            let end_key = end_point_key.xz_key();
            if start_key == end_key {
                continue;
            }
            source_edges.push(NodeEarthworkBoundarySourceEdge {
                start_point_key,
                end_point_key,
                start_key,
                end_key,
                node_id,
                kind,
                owner_kind: region.kind,
                owner_index: region.owner_index,
                start_source: NodeFootprintBoundaryDirectSource {
                    top_surface_source_index: region_index,
                    grade_authority_index: top_source.vertex_sources[index].grade_authority_index,
                },
                end_source: NodeFootprintBoundaryDirectSource {
                    top_surface_source_index: region_index,
                    grade_authority_index: top_source.vertex_sources[(index + 1) % points.len()]
                        .grade_authority_index,
                },
            });
        }
    }
    source_edges.sort_by(|a, b| {
        node_earthwork_source_edge_ordering(a, b)
            .then(a.start_key.cmp(&b.start_key))
            .then(a.end_key.cmp(&b.end_key))
    });
    Ok(source_edges)
}

fn node_footprint_boundary_direct_vertex_sources(
    owned_regions: &[NodeOwnedRegion],
    node_top_surface_sources: &[NodeTopSurfacePolygonSource],
) -> Result<
    BTreeMap<ArrangementBoundaryPointKey, NodeFootprintBoundaryDirectVertex>,
    NodeBoundaryExportError,
> {
    if owned_regions.len() != node_top_surface_sources.len() {
        return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
    }
    let mut sources = BTreeMap::new();
    for (region_index, region) in owned_regions.iter().enumerate() {
        let Some(top_source) = node_top_surface_sources.get(region_index) else {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        };
        if top_source.vertex_sources.len() != region.polygon.points_world.len() {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        }
        for (point_index, point) in region.polygon.points_world.iter().copied().enumerate() {
            let candidate = NodeFootprintBoundaryDirectVertex {
                source: NodeFootprintBoundaryVertexSource::Direct(
                    NodeFootprintBoundaryDirectSource {
                        top_surface_source_index: region_index,
                        grade_authority_index: top_source.vertex_sources[point_index]
                            .grade_authority_index,
                    },
                ),
                owner_kind: region.kind,
                owner_index: region.owner_index,
            };
            sources
                .entry(ArrangementBoundaryPointKey::from_world(point))
                .and_modify(|current| {
                    if node_footprint_direct_vertex_ordering(candidate, *current).is_gt() {
                        *current = candidate;
                    }
                })
                .or_insert(candidate);
        }
    }
    Ok(sources)
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

fn node_footprint_boundary_vertex_source_at_point(
    point_key: ArrangementBoundaryPointKey,
    direct_vertex_sources: &BTreeMap<
        ArrangementBoundaryPointKey,
        NodeFootprintBoundaryDirectVertex,
    >,
) -> Option<NodeFootprintBoundaryDirectVertex> {
    direct_vertex_sources.get(&point_key).copied()
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

fn node_footprint_boundary_vertex_source_for_edge_point(
    source_edge: &NodeEarthworkBoundarySourceEdge,
    point_key: ArrangementBoundaryPointKey,
) -> Option<NodeFootprintBoundaryVertexSource> {
    if point_key == source_edge.start_point_key {
        return Some(NodeFootprintBoundaryVertexSource::Direct(
            source_edge.start_source,
        ));
    }
    if point_key == source_edge.end_point_key {
        return Some(NodeFootprintBoundaryVertexSource::Direct(
            source_edge.end_source,
        ));
    }
    let parameter = arrangement_key_segment_parameter_xz(
        point_key.xz_key(),
        source_edge.start_key,
        source_edge.end_key,
    )?;
    let expected_height_mm = interpolated_segment_height_mm(
        source_edge.start_point_key,
        source_edge.end_point_key,
        parameter,
    );
    if (expected_height_mm - point_key.y_mm).abs() > 1 {
        return None;
    }
    Some(NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
        owning_segment_start: source_edge.start_source,
        owning_segment_end: source_edge.end_source,
        height_mm: point_key.y_mm,
    })
}

fn node_footprint_boundary_source_start_direct_source(
    source: NodeFootprintBoundaryVertexSource,
) -> NodeFootprintBoundaryDirectSource {
    match source {
        NodeFootprintBoundaryVertexSource::Direct(direct) => direct,
        NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
            owning_segment_start,
            ..
        } => owning_segment_start,
    }
}

fn node_footprint_boundary_source_end_direct_source(
    source: NodeFootprintBoundaryVertexSource,
) -> NodeFootprintBoundaryDirectSource {
    match source {
        NodeFootprintBoundaryVertexSource::Direct(direct) => direct,
        NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
            owning_segment_end, ..
        } => owning_segment_end,
    }
}

fn raised_step_footprint_height_candidate(
    candidates: &[NodeFootprintBoundaryHeightCandidate],
    heights: &[i64],
) -> Option<NodeFootprintBoundaryHeightCandidate> {
    // Terminal cap corners can put a lower material edge and its raised neighbor at one
    // footprint key. Accept that only when the material ranks prove an adjacent raised-step
    // transition; unrelated cross-material conflicts still reject above.
    let [lower_height, raised_height] = heights else {
        return None;
    };
    let lower_candidates = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.height_mm == *lower_height)
        .collect::<Vec<_>>();
    let raised_candidates = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.height_mm == *raised_height)
        .collect::<Vec<_>>();
    if lower_candidates.is_empty() || raised_candidates.is_empty() {
        return None;
    }
    for lower in &lower_candidates {
        for raised in &raised_candidates {
            if !raised_step_kinds_can_contact(lower.source.owner_kind, raised.source.owner_kind) {
                return None;
            }
            let lower_rank = raised_step_band_rank(lower.source.owner_kind)?;
            let raised_rank = raised_step_band_rank(raised.source.owner_kind)?;
            if lower_rank >= raised_rank {
                return None;
            }
        }
    }
    raised_candidates
        .into_iter()
        .max_by(|a, b| node_footprint_direct_vertex_ordering(a.source, b.source))
}

fn node_earthwork_source_edge_ordering(
    a: &NodeEarthworkBoundarySourceEdge,
    b: &NodeEarthworkBoundarySourceEdge,
) -> std::cmp::Ordering {
    a.node_id
        .cmp(&b.node_id)
        .then(a.kind.sort_key().cmp(&b.kind.sort_key()))
        .then(band_kind_sort_key(a.owner_kind).cmp(&band_kind_sort_key(b.owner_kind)))
        .then(a.owner_index.cmp(&b.owner_index))
        .then(a.start_source.cmp(&b.start_source))
        .then(a.end_source.cmp(&b.end_source))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_only_vertex_source_records_explicit_interpolation() {
        let start_source = NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 3,
            grade_authority_index: 30,
        };
        let end_source = NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 3,
            grade_authority_index: 31,
        };
        let start_point_key = ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 1.0, 0.0));
        let end_point_key = ArrangementBoundaryPointKey::from_world(Vector3::new(2.0, 3.0, 0.0));
        let source_edge = NodeEarthworkBoundarySourceEdge {
            start_point_key,
            end_point_key,
            start_key: start_point_key.xz_key(),
            end_key: end_point_key.xz_key(),
            node_id: 11,
            kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
            start_source,
            end_source,
        };

        let direct =
            node_footprint_boundary_vertex_source_for_edge_point(&source_edge, start_point_key)
                .expect("source edge endpoint should preserve direct top provenance");
        assert_eq!(
            direct,
            NodeFootprintBoundaryVertexSource::Direct(start_source)
        );

        let midpoint_key = ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 2.0, 0.0));
        let interpolated =
            node_footprint_boundary_vertex_source_for_edge_point(&source_edge, midpoint_key)
                .expect("boundary-only midpoint should be authorized by owning source edge");
        assert_eq!(
            interpolated,
            NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                owning_segment_start: start_source,
                owning_segment_end: end_source,
                height_mm: midpoint_key.y_mm,
            }
        );

        let wrong_height_key =
            ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 2.25, 0.0));
        assert!(
            node_footprint_boundary_vertex_source_for_edge_point(&source_edge, wrong_height_key)
                .is_none(),
            "boundary source recovery must block height drift instead of picking nearest top"
        );
    }

    #[test]
    fn numeric_cleanup_support_ignores_contour_only_interpolation_sources() {
        let source_edge = test_source_edge(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(2.0, 0.0, 0.0),
            3,
            30,
            3,
            31,
        );
        let start_source = source_edge.start_source;
        let end_source = source_edge.end_source;
        let unsupported_point = Vector3::new(1.0, 0.0, 0.00001);
        let unsupported_key = ArrangementBoundaryPointKey::from_world(unsupported_point);
        let mut direct_vertex_sources = BTreeMap::new();
        direct_vertex_sources.insert(
            unsupported_key,
            NodeFootprintBoundaryDirectVertex {
                source: NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                    owning_segment_start: start_source,
                    owning_segment_end: end_source,
                    height_mm: unsupported_key.y_mm,
                },
                owner_kind: RoadSurfaceBandKind::Sidewalk,
                owner_index: 5,
            },
        );
        let sources = NodeFootprintBoundaryExportSources {
            source_edges: vec![source_edge],
            direct_vertex_sources,
        };
        let supported_midpoint =
            ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 0.0, 0.0));

        assert!(
            sources.has_final_owned_footprint_boundary_support_at_point(supported_midpoint),
            "source-edge interpolation is valid footprint boundary support"
        );
        assert!(
            !sources.has_final_owned_footprint_boundary_support_at_point(unsupported_key),
            "contour-only interpolation must not make numeric cleanup depend on hidden support"
        );

        let mut points = vec![
            Vector3::new(0.0, 0.0, 0.0),
            unsupported_point,
            Vector3::new(2.0, 0.0, 0.0),
            Vector3::new(2.0, 0.0, 1.0),
            Vector3::new(0.0, 0.0, 1.0),
        ];
        remove_unsupported_numeric_boundary_vertices(&mut points, |point_key, local_points| {
            sources.has_final_owned_footprint_boundary_support_at_point(point_key)
                || RoadSurfaceSystem::signed_polygon_area_xz(&local_points).abs()
                    > boundary_points_numeric_area_budget_m2(&local_points)
        });

        assert_eq!(points.len(), 4);
        assert!(
            points
                .iter()
                .copied()
                .all(|point| ArrangementBoundaryPointKey::from_world(point) != unsupported_key)
        );
    }

    #[test]
    fn duplicate_split_point_same_height_preserves_sourced_subsegments() {
        let first_mid_source = NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 3,
            grade_authority_index: 31,
        };
        let second_mid_source = NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 4,
            grade_authority_index: 40,
        };
        let source_edges = vec![
            test_source_edge(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1.0, 1.0, 0.0),
                3,
                30,
                3,
                31,
            ),
            test_source_edge(
                Vector3::new(1.0, 1.0, 0.0),
                Vector3::new(2.0, 2.0, 0.0),
                4,
                40,
                4,
                41,
            ),
        ];
        let mut segments = Vec::new();

        push_sourced_node_earthwork_boundary_segments(
            11,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(2.0, 2.0, 0.0),
            &source_edges,
            &BTreeMap::new(),
            &mut segments,
        )
        .expect("same-height duplicate split points should merge without height repair");

        assert_eq!(segments.len(), 2);
        assert!(matches!(
            segments[0].source,
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                boundary_source: Some(NodeFootprintBoundarySegmentSource {
                    end: NodeFootprintBoundaryVertexSource::Direct(source),
                    ..
                }),
                ..
            } if source == first_mid_source
        ));
        assert!(matches!(
            segments[1].source,
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                boundary_source: Some(NodeFootprintBoundarySegmentSource {
                    start: NodeFootprintBoundaryVertexSource::Direct(source),
                    ..
                }),
                ..
            } if source == second_mid_source
        ));
    }

    #[test]
    fn duplicate_split_point_conflicting_height_is_rejected() {
        let parameter =
            ArrangementSegmentParameter::new(1, 2).expect("test parameter should be canonical");
        let mut split_points = BTreeMap::new();
        insert_node_footprint_boundary_split_point(
            &mut split_points,
            parameter,
            NodeFootprintBoundarySplitPoint {
                point_key: ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 1.0, 0.0)),
                source: None,
            },
        )
        .expect("first split point should insert");

        let error = insert_node_footprint_boundary_split_point(
            &mut split_points,
            parameter,
            NodeFootprintBoundarySplitPoint {
                point_key: ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 2.0, 0.0)),
                source: None,
            },
        )
        .expect_err("duplicate split points with different heights must not pick max Y");

        assert!(matches!(
            error,
            NodeBoundaryExportError::ConflictingFootprintBoundarySplitHeight {
                x_key: 1_000_000,
                z_key: 0,
                existing_y_mm: 1000,
                incoming_y_mm: 2000,
            }
        ));
    }

    #[test]
    fn duplicate_split_point_height_uses_source_order_not_max_y() {
        let parameter =
            ArrangementSegmentParameter::new(1, 2).expect("test parameter should be canonical");
        let lower_order_source = NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 1,
                grade_authority_index: 10,
            }),
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
        };
        let higher_order_source = NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 2,
                grade_authority_index: 20,
            }),
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
        };
        let mut split_points = BTreeMap::new();
        insert_node_footprint_boundary_split_point(
            &mut split_points,
            parameter,
            NodeFootprintBoundarySplitPoint {
                point_key: ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 2.0, 0.0)),
                source: Some(lower_order_source),
            },
        )
        .expect("first split point should insert");

        insert_node_footprint_boundary_split_point(
            &mut split_points,
            parameter,
            NodeFootprintBoundarySplitPoint {
                point_key: ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 1.0, 0.0)),
                source: Some(higher_order_source),
            },
        )
        .expect("explicit source ordering should resolve duplicate split point height");

        let selected = split_points
            .get(&parameter)
            .expect("merged split point should remain");
        assert_eq!(selected.point_key.y_mm, 1000);
        assert_eq!(selected.source, Some(higher_order_source));
    }

    fn test_source_edge(
        start: Vector3,
        end: Vector3,
        start_top_surface_source_index: usize,
        start_grade_authority_index: usize,
        end_top_surface_source_index: usize,
        end_grade_authority_index: usize,
    ) -> NodeEarthworkBoundarySourceEdge {
        let start_point_key = ArrangementBoundaryPointKey::from_world(start);
        let end_point_key = ArrangementBoundaryPointKey::from_world(end);
        NodeEarthworkBoundarySourceEdge {
            start_point_key,
            end_point_key,
            start_key: start_point_key.xz_key(),
            end_key: end_point_key.xz_key(),
            node_id: 11,
            kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
            start_source: NodeFootprintBoundaryDirectSource {
                top_surface_source_index: start_top_surface_source_index,
                grade_authority_index: start_grade_authority_index,
            },
            end_source: NodeFootprintBoundaryDirectSource {
                top_surface_source_index: end_top_surface_source_index,
                grade_authority_index: end_grade_authority_index,
            },
        }
    }
}
