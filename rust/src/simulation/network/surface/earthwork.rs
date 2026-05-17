//! Road-owned earthwork generation, terrain stamping, and structural visibility rules.

use super::{
    ChunkCacheKind, NodeOverlayContour, NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSection,
    RoadSurfaceSpanBandOwner, RoadSurfaceSpanOwnedRegion, RoadSurfaceSpanRegionRole,
    RoadSurfaceSystem, RoadSurfaceVisualNodePiece, RoadSurfaceVisualNodePieceKind,
    RoadSurfaceVisualPolygon, RoadSurfaceVisualSpanPiece, SAMPLE_EPSILON_M, SurfaceChunkKey,
    backend,
    band_semantics::band_kind_sort_key,
    keys::{SurfaceXzKey, SurfaceXzSegmentKey},
    node::boundary::NodeFootprintBoundarySegmentSource,
};
use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::{BTreeMap, BTreeSet, HashMap};

// Vertical roadbed offset applied when terrain earthworks need pavement clearance.
pub(super) const EARTHWORK_PAVEMENT_DEPTH_M: f32 = 0.04;

// Lateral terrain probing envelope and sampling cadence for slopes.
const EARTHWORK_MIN_MARGIN_M: f32 = 4.0;
pub(super) const EARTHWORK_MAX_MARGIN_M: f32 = 18.0;
const EARTHWORK_MARGIN_SAMPLE_STEP_M: f32 = 1.0;

// Earthwork slope rates and retaining-wall classification threshold.
const EARTHWORK_CUT_SLOPE_RATE: f32 = 0.5;
const EARTHWORK_FILL_SLOPE_RATE: f32 = 0.5;
const EARTHWORK_RETAINING_WALL_SLOPE_THRESHOLD: f32 = 1.25;

// Structural end caps that constrain bridge abutments and tunnel portal stamps.
const BRIDGE_ABUTMENT_LENGTH_M: f32 = 12.0;
const TUNNEL_PORTAL_STAMP_DEPTH_M: f32 = 1.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoadSurfaceEarthworkFaceKind {
    Slope,
    RetainingWall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoadSurfaceEarthworkSupportPolicy {
    StandardFullGroundedSpan,
    BridgeEndpointAbutments,
    TunnelVisiblePortals,
}

impl RoadSurfaceEarthworkSupportPolicy {
    pub(crate) fn from_edge_class(edge_class: EdgeClass) -> Self {
        match edge_class {
            EdgeClass::Standard => Self::StandardFullGroundedSpan,
            EdgeClass::Bridge => Self::BridgeEndpointAbutments,
            EdgeClass::Tunnel => Self::TunnelVisiblePortals,
        }
    }

    pub(crate) fn debug_name(self) -> &'static str {
        match self {
            Self::StandardFullGroundedSpan => "standard_full_grounded_span",
            Self::BridgeEndpointAbutments => "bridge_endpoint_abutments",
            Self::TunnelVisiblePortals => "tunnel_visible_portals",
        }
    }

    pub(crate) fn sort_key(self) -> u8 {
        match self {
            Self::StandardFullGroundedSpan => 0,
            Self::BridgeEndpointAbutments => 1,
            Self::TunnelVisiblePortals => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RoadSurfaceEarthworkFaceSource {
    SpanSupportBoundary {
        edge_idx: usize,
        edge_class: EdgeClass,
        support_policy: RoadSurfaceEarthworkSupportPolicy,
        owner: RoadSurfaceSpanBandOwner,
        role: RoadSurfaceSpanRegionRole,
        start_section_index: usize,
        end_section_index: usize,
        start_s_m: f32,
        end_s_m: f32,
    },
    NodeFootprintBoundary {
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        owner_kind: RoadSurfaceBandKind,
        owner_index: usize,
        boundary_source: Option<NodeFootprintBoundarySegmentSource>,
    },
}

impl RoadSurfaceEarthworkFaceSource {
    pub(crate) fn source_ordering(self, other: Self) -> std::cmp::Ordering {
        match (self, other) {
            (
                Self::SpanSupportBoundary {
                    edge_idx: edge_idx_a,
                    edge_class: edge_class_a,
                    support_policy: support_policy_a,
                    owner: owner_a,
                    role: role_a,
                    start_section_index: start_section_index_a,
                    end_section_index: end_section_index_a,
                    start_s_m: start_s_m_a,
                    end_s_m: end_s_m_a,
                },
                Self::SpanSupportBoundary {
                    edge_idx: edge_idx_b,
                    edge_class: edge_class_b,
                    support_policy: support_policy_b,
                    owner: owner_b,
                    role: role_b,
                    start_section_index: start_section_index_b,
                    end_section_index: end_section_index_b,
                    start_s_m: start_s_m_b,
                    end_s_m: end_s_m_b,
                },
            ) => edge_idx_a
                .cmp(&edge_idx_b)
                .then(edge_class_sort_key(edge_class_a).cmp(&edge_class_sort_key(edge_class_b)))
                .then(
                    support_policy_a
                        .sort_key()
                        .cmp(&support_policy_b.sort_key()),
                )
                .then(owner_a.sort_key().cmp(&owner_b.sort_key()))
                .then(role_a.sort_key().cmp(&role_b.sort_key()))
                .then(start_section_index_a.cmp(&start_section_index_b))
                .then(end_section_index_a.cmp(&end_section_index_b))
                .then(start_s_m_a.total_cmp(&start_s_m_b))
                .then(end_s_m_a.total_cmp(&end_s_m_b)),
            (
                Self::NodeFootprintBoundary {
                    node_id: node_id_a,
                    kind: kind_a,
                    owner_kind: owner_kind_a,
                    owner_index: owner_index_a,
                    boundary_source: boundary_source_a,
                },
                Self::NodeFootprintBoundary {
                    node_id: node_id_b,
                    kind: kind_b,
                    owner_kind: owner_kind_b,
                    owner_index: owner_index_b,
                    boundary_source: boundary_source_b,
                },
            ) => node_id_a
                .cmp(&node_id_b)
                .then(kind_a.sort_key().cmp(&kind_b.sort_key()))
                .then(band_kind_sort_key(owner_kind_a).cmp(&band_kind_sort_key(owner_kind_b)))
                .then(owner_index_a.cmp(&owner_index_b))
                .then(boundary_source_a.cmp(&boundary_source_b)),
            (Self::SpanSupportBoundary { .. }, Self::NodeFootprintBoundary { .. }) => {
                std::cmp::Ordering::Less
            }
            (Self::NodeFootprintBoundary { .. }, Self::SpanSupportBoundary { .. }) => {
                std::cmp::Ordering::Greater
            }
        }
    }
}

fn edge_class_sort_key(edge_class: EdgeClass) -> u8 {
    match edge_class {
        EdgeClass::Standard => 0,
        EdgeClass::Bridge => 1,
        EdgeClass::Tunnel => 2,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RoadSurfaceEarthworkBoundarySegment {
    pub(crate) inner_start: Vector3,
    pub(crate) inner_end: Vector3,
    pub(crate) source: RoadSurfaceEarthworkFaceSource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadSurfaceEarthworkRenderFace {
    pub(crate) kind: RoadSurfaceEarthworkFaceKind,
    pub(crate) source: RoadSurfaceEarthworkFaceSource,
    pub(crate) inner_start: Vector3,
    pub(crate) inner_end: Vector3,
    pub(crate) polygon: RoadSurfaceVisualPolygon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct EarthworkBoundaryPointKey {
    x_key: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct EarthworkBoundaryEdgeKey {
    start: EarthworkBoundaryPointKey,
    end: EarthworkBoundaryPointKey,
}

#[derive(Clone, Copy, Debug)]
struct IndexedEarthworkBoundarySegment {
    segment_index: usize,
    segment: RoadSurfaceEarthworkBoundarySegment,
    start_key: EarthworkBoundaryPointKey,
    end_key: EarthworkBoundaryPointKey,
}

impl EarthworkBoundaryPointKey {
    fn from_point(point: Vector3) -> Self {
        Self::from_surface_key(SurfaceXzKey::from_godot_world_xz(point))
    }

    fn from_surface_key(key: SurfaceXzKey) -> Self {
        Self {
            x_key: key.x_key(),
            z_key: key.z_key(),
        }
    }

    fn surface_key(self) -> SurfaceXzKey {
        SurfaceXzKey::from_raw_keys(self.x_key, self.z_key)
    }
}

impl EarthworkBoundaryEdgeKey {
    fn normalized(
        start: EarthworkBoundaryPointKey,
        end: EarthworkBoundaryPointKey,
    ) -> Option<Self> {
        let segment = SurfaceXzSegmentKey::non_degenerate(start.surface_key(), end.surface_key())?;
        Some(Self {
            start: EarthworkBoundaryPointKey::from_surface_key(segment.start()),
            end: EarthworkBoundaryPointKey::from_surface_key(segment.end()),
        })
    }
}

impl RoadSurfaceSystem {
    /// Rebuilds terrain earthworks only for the currently dirty road-surface chunks.
    pub fn rebuild_dirty_earthworks(
        &mut self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
    ) -> Vec<SurfaceChunkKey> {
        let had_dirty_work = !self.compiled_once
            || !self.dirty_edges.is_empty()
            || !self.dirty_nodes.is_empty()
            || !self.dirty_surface_chunks.is_empty()
            || !self.dirty_terrain_chunks.is_empty();
        self.compile_dirty(graph, terrain);

        let chunks = if had_dirty_work {
            self.last_rebuilt_terrain_chunks.clone()
        } else {
            self.collect_all_chunks(ChunkCacheKind::Earthwork)
        };
        self.apply_earthwork_chunks(graph, terrain, &chunks);
        chunks
    }

    /// Rebuilds terrain earthworks for the whole world from the current compiled roadbed cache.
    pub fn rebuild_all_earthworks(
        &mut self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
    ) -> Vec<SurfaceChunkKey> {
        terrain.reset_visuals_from_source();
        self.compile_dirty(graph, terrain);
        let chunks = self.collect_all_chunks(ChunkCacheKind::Earthwork);
        self.apply_earthwork_chunks(graph, terrain, &chunks);
        chunks
    }
    fn apply_earthwork_chunks(
        &self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
        chunks: &[SurfaceChunkKey],
    ) {
        for &chunk in chunks {
            let (chunk_min, chunk_max) = self.chunk_bounds(chunk);
            terrain.reset_visual_region_from_source_world(
                chunk_min.x,
                chunk_min.z,
                chunk_max.x,
                chunk_max.z,
            );

            let Some(entry) = self.earthwork_chunk_cache.get(&chunk) else {
                continue;
            };

            for &edge_idx in &entry.edge_indices {
                let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                    continue;
                };
                self.stamp_visual_span_piece_earthworks_for_chunk(piece, chunk, terrain);
            }

            for &node_id in &entry.node_ids {
                if node_id as usize >= graph.node_count() {
                    continue;
                }
                let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                    continue;
                };
                self.stamp_visual_node_piece_earthworks_for_chunk(
                    graph, node_id, piece, chunk, terrain,
                );
            }
        }
    }
    pub(super) fn span_earthwork_boundary_segment_loops_from_support_regions(
        regions: &[RoadSurfaceSpanOwnedRegion],
        edge_class: EdgeClass,
    ) -> Vec<Vec<RoadSurfaceEarthworkBoundarySegment>> {
        let support_policy = RoadSurfaceEarthworkSupportPolicy::from_edge_class(edge_class);
        let mut candidate_segments = Vec::new();
        for region in regions {
            let source = RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                edge_idx: region.edge_idx,
                edge_class,
                support_policy,
                owner: region.owner,
                role: region.role,
                start_section_index: region.start_section_index,
                end_section_index: region.end_section_index,
                start_s_m: region.start_s_m,
                end_s_m: region.end_s_m,
            };
            Self::push_region_polygon_boundary_segments(
                &region.polygon,
                source,
                &mut candidate_segments,
            );
        }
        Self::owned_region_boundary_segment_loops(candidate_segments)
    }

    fn push_region_polygon_boundary_segments(
        polygon: &RoadSurfaceVisualPolygon,
        source: RoadSurfaceEarthworkFaceSource,
        segments: &mut Vec<RoadSurfaceEarthworkBoundarySegment>,
    ) {
        let points = &polygon.points_world;
        if points.len() < 3 {
            return;
        }
        for index in 0..points.len() {
            let inner_start = points[index];
            let inner_end = points[(index + 1) % points.len()];
            let span_xz = Vector2::new(inner_end.x - inner_start.x, inner_end.z - inner_start.z);
            if span_xz.length_squared() <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
                continue;
            }
            segments.push(RoadSurfaceEarthworkBoundarySegment {
                inner_start,
                inner_end,
                source,
            });
        }
    }

    fn owned_region_boundary_segment_loops(
        candidate_segments: Vec<RoadSurfaceEarthworkBoundarySegment>,
    ) -> Vec<Vec<RoadSurfaceEarthworkBoundarySegment>> {
        let mut edge_counts = BTreeMap::<EarthworkBoundaryEdgeKey, usize>::new();
        for segment in &candidate_segments {
            let start_key = EarthworkBoundaryPointKey::from_point(segment.inner_start);
            let end_key = EarthworkBoundaryPointKey::from_point(segment.inner_end);
            let Some(edge_key) = EarthworkBoundaryEdgeKey::normalized(start_key, end_key) else {
                continue;
            };
            *edge_counts.entry(edge_key).or_insert(0) += 1;
        }

        let mut boundary_segments = Vec::new();
        for (segment_index, segment) in candidate_segments.into_iter().enumerate() {
            let start_key = EarthworkBoundaryPointKey::from_point(segment.inner_start);
            let end_key = EarthworkBoundaryPointKey::from_point(segment.inner_end);
            let Some(edge_key) = EarthworkBoundaryEdgeKey::normalized(start_key, end_key) else {
                continue;
            };
            if edge_counts.get(&edge_key).copied() != Some(1) {
                continue;
            }
            boundary_segments.push(IndexedEarthworkBoundarySegment {
                segment_index,
                segment,
                start_key,
                end_key,
            });
        }
        Self::assemble_earthwork_boundary_segment_loops(boundary_segments)
    }

    fn assemble_earthwork_boundary_segment_loops(
        mut boundary_segments: Vec<IndexedEarthworkBoundarySegment>,
    ) -> Vec<Vec<RoadSurfaceEarthworkBoundarySegment>> {
        boundary_segments.sort_by(|a, b| {
            a.start_key
                .cmp(&b.start_key)
                .then(a.end_key.cmp(&b.end_key))
                .then(a.segment_index.cmp(&b.segment_index))
        });

        let mut adjacency = BTreeMap::<EarthworkBoundaryPointKey, Vec<usize>>::new();
        for (index, segment) in boundary_segments.iter().enumerate() {
            adjacency.entry(segment.start_key).or_default().push(index);
            adjacency.entry(segment.end_key).or_default().push(index);
        }
        for (point_key, indices) in adjacency.iter_mut() {
            indices.sort_by(|a, b| {
                let segment_a = &boundary_segments[*a];
                let segment_b = &boundary_segments[*b];
                let other_a = if segment_a.start_key == *point_key {
                    segment_a.end_key
                } else {
                    segment_a.start_key
                };
                let other_b = if segment_b.start_key == *point_key {
                    segment_b.end_key
                } else {
                    segment_b.start_key
                };
                other_a
                    .cmp(&other_b)
                    .then(segment_a.segment_index.cmp(&segment_b.segment_index))
            });
        }

        let mut used = BTreeSet::<usize>::new();
        let mut loops = Vec::new();
        for start_index in 0..boundary_segments.len() {
            if used.contains(&start_index) {
                continue;
            }
            let start_key = boundary_segments[start_index].start_key;
            let mut current_key = boundary_segments[start_index].end_key;
            let mut loop_segments = vec![boundary_segments[start_index].segment];
            used.insert(start_index);

            while current_key != start_key {
                let Some(next_indices) = adjacency.get(&current_key) else {
                    break;
                };
                let Some(next_index) = next_indices
                    .iter()
                    .copied()
                    .find(|candidate| !used.contains(candidate))
                else {
                    break;
                };
                let indexed = boundary_segments[next_index];
                let (segment, next_key) = if indexed.start_key == current_key {
                    (indexed.segment, indexed.end_key)
                } else {
                    (
                        RoadSurfaceEarthworkBoundarySegment {
                            inner_start: indexed.segment.inner_end,
                            inner_end: indexed.segment.inner_start,
                            source: indexed.segment.source,
                        },
                        indexed.start_key,
                    )
                };
                loop_segments.push(segment);
                used.insert(next_index);
                current_key = next_key;
            }

            if current_key != start_key || loop_segments.len() < 3 {
                continue;
            }
            let point_loop = loop_segments
                .iter()
                .map(|segment| segment.inner_start)
                .collect::<Vec<_>>();
            if Self::signed_polygon_area_xz(&point_loop).abs() <= SAMPLE_EPSILON_M {
                continue;
            }
            loops.push(loop_segments);
        }

        Self::orient_earthwork_boundary_segment_loops_by_nesting(&mut loops);
        loops
    }

    pub(super) fn orient_earthwork_boundary_segment_loops_by_nesting(
        loops: &mut [Vec<RoadSurfaceEarthworkBoundarySegment>],
    ) {
        let point_loops = loops
            .iter()
            .map(|segments| {
                segments
                    .iter()
                    .map(|segment| segment.inner_start)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let samples = point_loops
            .iter()
            .map(|points| {
                points.iter().fold(Vector2::ZERO, |sum, point| {
                    sum + Vector2::new(point.x, point.z)
                }) / points.len().max(1) as f32
            })
            .collect::<Vec<_>>();
        let should_be_ccw = point_loops
            .iter()
            .enumerate()
            .map(|(loop_index, _)| {
                let depth = point_loops
                    .iter()
                    .enumerate()
                    .filter(|(candidate_index, candidate)| {
                        *candidate_index != loop_index
                            && RoadSurfaceSystem::polygon_contains_point_xz(
                                candidate,
                                samples[loop_index],
                            )
                    })
                    .count();
                depth % 2 == 0
            })
            .collect::<Vec<_>>();
        for (segments, should_be_ccw) in loops.iter_mut().zip(should_be_ccw) {
            let points = segments
                .iter()
                .map(|segment| segment.inner_start)
                .collect::<Vec<_>>();
            let is_ccw = Self::signed_polygon_area_xz(&points) > 0.0;
            if is_ccw != should_be_ccw {
                Self::reverse_earthwork_boundary_segment_loop(segments);
            }
        }
    }

    fn reverse_earthwork_boundary_segment_loop(
        segments: &mut Vec<RoadSurfaceEarthworkBoundarySegment>,
    ) {
        segments.reverse();
        for segment in segments {
            std::mem::swap(&mut segment.inner_start, &mut segment.inner_end);
        }
    }

    pub(super) fn build_closed_earthwork_geometry_from_boundary_segments(
        &self,
        boundary_segment_loops: &[Vec<RoadSurfaceEarthworkBoundarySegment>],
        terrain: &TerrainSystem,
        top_surface_shapes: Option<&NodeOverlayShapes>,
    ) -> (
        Vec<RoadSurfaceVisualPolygon>,
        Vec<RoadSurfaceVisualPolygon>,
        Vec<RoadSurfaceEarthworkRenderFace>,
    ) {
        let mut earthwork_surface_polygons = Vec::new();
        let mut earthwork_outer_boundary_loops = Vec::new();
        let mut render_earthwork_faces = Vec::new();

        for boundary_segments in boundary_segment_loops {
            let Some((outer_loop, side_polygons, render_faces)) = self
                .build_closed_earthwork_loop_geometry(
                    boundary_segments,
                    terrain,
                    top_surface_shapes,
                )
            else {
                continue;
            };
            if let Some(outer_loop) = outer_loop {
                earthwork_outer_boundary_loops.push(outer_loop);
            }
            earthwork_surface_polygons.extend(side_polygons);
            render_earthwork_faces.extend(render_faces);
        }

        Self::sort_visual_polygons(&mut earthwork_surface_polygons);
        Self::sort_visual_polygons(&mut earthwork_outer_boundary_loops);
        Self::sort_earthwork_render_faces(&mut render_earthwork_faces);
        (
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        )
    }

    fn build_closed_earthwork_loop_geometry(
        &self,
        boundary_segments: &[RoadSurfaceEarthworkBoundarySegment],
        terrain: &TerrainSystem,
        top_surface_shapes: Option<&NodeOverlayShapes>,
    ) -> Option<(
        Option<RoadSurfaceVisualPolygon>,
        Vec<RoadSurfaceVisualPolygon>,
        Vec<RoadSurfaceEarthworkRenderFace>,
    )> {
        if boundary_segments.len() < 3 {
            return None;
        }
        let boundary_points = boundary_segments
            .iter()
            .map(|segment| segment.inner_start)
            .collect::<Vec<_>>();

        let vertex_outer_points: Vec<Vector3> = boundary_points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let outward = Self::closed_loop_vertex_outward_xz(&boundary_points, index);
                self.earthwork_transition_point(*point, outward, terrain)
            })
            .collect();
        let outer_loop = Self::make_visual_polygon(vertex_outer_points);
        let mut side_polygons = Vec::new();
        let mut render_faces = Vec::new();
        let winding_ccw = Self::signed_polygon_area_xz(&boundary_points) > 0.0;
        for segment in boundary_segments {
            let current = segment.inner_start;
            let next = segment.inner_end;
            let (outer_current, outer_next) = self.earthwork_edge_transition_points(
                current,
                next,
                winding_ccw,
                terrain,
                top_surface_shapes,
            );
            // Handoff and internal seam edges can be part of a closed footprint loop. They are not
            // terrain tie-ins, so a skirt whose plan area enters solved top ownership is rejected.
            if let Some(top_surface_shapes) = top_surface_shapes
                && Self::earthwork_candidate_intrudes_top(
                    [current, next, outer_next, outer_current],
                    top_surface_shapes,
                )
            {
                continue;
            }
            let Some(polygon) =
                Self::make_visual_polygon(vec![current, next, outer_next, outer_current])
            else {
                continue;
            };
            let face_kind =
                Self::classify_earthwork_face_kind(current, next, outer_next, outer_current);
            render_faces.push(RoadSurfaceEarthworkRenderFace {
                kind: face_kind,
                source: segment.source,
                inner_start: current,
                inner_end: next,
                polygon: polygon.clone(),
            });
            side_polygons.push(polygon);
        }

        if outer_loop.is_none() && side_polygons.is_empty() {
            return None;
        }
        Some((outer_loop, side_polygons, render_faces))
    }

    fn earthwork_edge_transition_points(
        &self,
        current: Vector3,
        next: Vector3,
        winding_ccw: bool,
        terrain: &TerrainSystem,
        top_surface_shapes: Option<&NodeOverlayShapes>,
    ) -> (Vector3, Vector3) {
        let edge = Vector2::new(next.x - current.x, next.z - current.z);
        let outward = Self::edge_outward_normal_xz(edge, winding_ccw);
        let outer_current = self.earthwork_transition_point(current, outward, terrain);
        let outer_next = self.earthwork_transition_point(next, outward, terrain);
        let Some(top_surface_shapes) = top_surface_shapes else {
            return (outer_current, outer_next);
        };

        let opposite_outer_current = self.earthwork_transition_point(current, -outward, terrain);
        let opposite_outer_next = self.earthwork_transition_point(next, -outward, terrain);
        let nominal_overlap = Self::earthwork_candidate_top_overlap_area_m2(
            [current, next, outer_next, outer_current],
            top_surface_shapes,
        );
        let opposite_overlap = Self::earthwork_candidate_top_overlap_area_m2(
            [current, next, opposite_outer_next, opposite_outer_current],
            top_surface_shapes,
        );
        if opposite_overlap < nominal_overlap {
            (opposite_outer_current, opposite_outer_next)
        } else {
            (outer_current, outer_next)
        }
    }

    fn earthwork_candidate_intrudes_top(
        points: [Vector3; 4],
        top_surface_shapes: &NodeOverlayShapes,
    ) -> bool {
        let Some((overlap_area_m2, budget_m2)) =
            Self::earthwork_candidate_top_overlap_metrics_m2(points, top_surface_shapes)
        else {
            return true;
        };
        overlap_area_m2 > budget_m2
    }

    fn earthwork_candidate_top_overlap_area_m2(
        points: [Vector3; 4],
        top_surface_shapes: &NodeOverlayShapes,
    ) -> f32 {
        Self::earthwork_candidate_top_overlap_metrics_m2(points, top_surface_shapes)
            .map(|(overlap_area_m2, _)| overlap_area_m2)
            .unwrap_or(f32::INFINITY)
    }

    fn earthwork_candidate_top_overlap_metrics_m2(
        mut points: [Vector3; 4],
        top_surface_shapes: &NodeOverlayShapes,
    ) -> Option<(f32, f32)> {
        if Self::signed_polygon_area_xz(&points) < 0.0 {
            points.reverse();
        }
        let candidate_shapes =
            Self::overlay_union_contours(&[Self::earthwork_overlay_contour_from_points(&points)])?;
        let overlap = Self::overlay_binary_shapes(
            &candidate_shapes,
            top_surface_shapes,
            OverlayRule::Intersect,
        )?;
        let overlap_area_m2 = overlap.iter().map(Self::overlay_shape_area_m2).sum();
        let budget_m2 = Self::overlay_numeric_area_budget_for_shapes(&candidate_shapes).max(
            Self::overlay_numeric_area_budget_for_shapes(top_surface_shapes),
        );
        Some((overlap_area_m2, budget_m2))
    }

    fn earthwork_overlay_contour_from_points(points: &[Vector3]) -> NodeOverlayContour {
        let mut contour = Vec::with_capacity(points.len());
        for point in points {
            let point = backend::road_vec2_to_overlay_point(backend::godot_vec3_xz_to_road(*point));
            if contour.last().is_none_or(|last| *last != point) {
                contour.push(point);
            }
        }
        if contour.len() >= 2 && contour.first() == contour.last() {
            contour.pop();
        }
        contour
    }

    pub(super) fn top_surface_overlay_shapes<'a>(
        polygons: impl IntoIterator<Item = &'a RoadSurfaceVisualPolygon>,
    ) -> Option<NodeOverlayShapes> {
        let mut contours = Vec::new();
        for polygon in polygons {
            if polygon.points_world.len() >= 3 {
                contours.push(Self::earthwork_overlay_contour_from_points(
                    &polygon.points_world,
                ));
            }
        }
        Self::overlay_union_contours(&contours)
    }

    fn closed_loop_vertex_outward_xz(boundary_points: &[Vector3], index: usize) -> Vector2 {
        if boundary_points.len() < 3 {
            return Vector2::RIGHT;
        }

        let len = boundary_points.len();
        let prev = boundary_points[(index + len - 1) % len];
        let current = boundary_points[index];
        let next = boundary_points[(index + 1) % len];
        let incoming = Vector2::new(current.x - prev.x, current.z - prev.z);
        let outgoing = Vector2::new(next.x - current.x, next.z - current.z);
        let winding_ccw = Self::signed_polygon_area_xz(boundary_points) > 0.0;
        let outward_incoming = Self::edge_outward_normal_xz(incoming, winding_ccw);
        let outward_outgoing = Self::edge_outward_normal_xz(outgoing, winding_ccw);
        let mut outward = outward_incoming + outward_outgoing;
        if outward.length_squared() <= SAMPLE_EPSILON_M {
            outward = if outward_incoming.length_squared() > SAMPLE_EPSILON_M {
                outward_incoming
            } else {
                outward_outgoing
            };
        }
        if outward.length_squared() <= SAMPLE_EPSILON_M {
            let centroid = boundary_points.iter().fold(Vector2::ZERO, |sum, point| {
                sum + Vector2::new(point.x, point.z)
            }) / boundary_points.len() as f32;
            outward = Vector2::new(current.x - centroid.x, current.z - centroid.y);
        }
        if outward.length_squared() <= SAMPLE_EPSILON_M {
            Vector2::RIGHT
        } else {
            outward.normalized()
        }
    }

    fn edge_outward_normal_xz(edge_xz: Vector2, winding_ccw: bool) -> Vector2 {
        if edge_xz.length_squared() <= SAMPLE_EPSILON_M {
            return Vector2::ZERO;
        }
        let tangent = edge_xz.normalized();
        if winding_ccw {
            Vector2::new(tangent.y, -tangent.x)
        } else {
            Vector2::new(-tangent.y, tangent.x)
        }
    }

    pub(super) fn classify_earthwork_face_kind(
        inner_start: Vector3,
        inner_end: Vector3,
        outer_end: Vector3,
        outer_start: Vector3,
    ) -> RoadSurfaceEarthworkFaceKind {
        let setback_a =
            Vector2::new(outer_start.x - inner_start.x, outer_start.z - inner_start.z).length();
        let setback_b = Vector2::new(outer_end.x - inner_end.x, outer_end.z - inner_end.z).length();
        let avg_setback = (setback_a + setback_b) * 0.5;
        if avg_setback <= SAMPLE_EPSILON_M {
            return RoadSurfaceEarthworkFaceKind::RetainingWall;
        }

        let max_height_delta = (outer_start.y - inner_start.y)
            .abs()
            .max((outer_end.y - inner_end.y).abs());
        let slope_ratio = max_height_delta / avg_setback.max(SAMPLE_EPSILON_M);
        if slope_ratio >= EARTHWORK_RETAINING_WALL_SLOPE_THRESHOLD {
            RoadSurfaceEarthworkFaceKind::RetainingWall
        } else {
            RoadSurfaceEarthworkFaceKind::Slope
        }
    }
    pub(super) fn sort_earthwork_render_faces(faces: &mut [RoadSurfaceEarthworkRenderFace]) {
        faces.sort_by(|a, b| {
            let kind_order = match (a.kind, b.kind) {
                (
                    RoadSurfaceEarthworkFaceKind::Slope,
                    RoadSurfaceEarthworkFaceKind::RetainingWall,
                ) => std::cmp::Ordering::Less,
                (
                    RoadSurfaceEarthworkFaceKind::RetainingWall,
                    RoadSurfaceEarthworkFaceKind::Slope,
                ) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            };
            if kind_order != std::cmp::Ordering::Equal {
                return kind_order;
            }
            a.source
                .source_ordering(b.source)
                .then(
                    a.inner_start
                        .x
                        .total_cmp(&b.inner_start.x)
                        .then(a.inner_start.z.total_cmp(&b.inner_start.z))
                        .then(a.inner_start.y.total_cmp(&b.inner_start.y)),
                )
                .then(
                    a.polygon
                        .points_world
                        .len()
                        .cmp(&b.polygon.points_world.len()),
                )
                .then_with(|| {
                    a.polygon
                        .points_world
                        .iter()
                        .zip(&b.polygon.points_world)
                        .find_map(|(point_a, point_b)| {
                            let ordering = point_a
                                .x
                                .total_cmp(&point_b.x)
                                .then(point_a.z.total_cmp(&point_b.z))
                                .then(point_a.y.total_cmp(&point_b.y));
                            (ordering != std::cmp::Ordering::Equal).then_some(ordering)
                        })
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
    }

    pub(super) fn earthwork_transition_point(
        &self,
        road_point: Vector3,
        outward_xz: Vector2,
        terrain: &TerrainSystem,
    ) -> Vector3 {
        let outward_xz = if outward_xz.length_squared() <= SAMPLE_EPSILON_M {
            Vector2::RIGHT
        } else {
            outward_xz.normalized()
        };
        let distance_m = self.earthwork_transition_distance_m(road_point, outward_xz, terrain);
        let outer_xz = Vector2::new(road_point.x, road_point.z) + outward_xz * distance_m;
        let outer_height_m =
            terrain.sample_height_world(outer_xz.x, outer_xz.y) * config::HEIGHT_SCALE;
        Vector3::new(outer_xz.x, outer_height_m, outer_xz.y)
    }

    fn earthwork_transition_distance_m(
        &self,
        road_point: Vector3,
        outward_xz: Vector2,
        terrain: &TerrainSystem,
    ) -> f32 {
        let source_height_at_edge =
            terrain.sample_height_world(road_point.x, road_point.z) * config::HEIGHT_SCALE;
        let cut_side = source_height_at_edge > road_point.y;
        let slope_rate = if cut_side {
            EARTHWORK_CUT_SLOPE_RATE
        } else {
            EARTHWORK_FILL_SLOPE_RATE
        };

        let mut distance_m = EARTHWORK_MIN_MARGIN_M;
        while distance_m < EARTHWORK_MAX_MARGIN_M {
            let sample_x = road_point.x + outward_xz.x * distance_m;
            let sample_z = road_point.z + outward_xz.y * distance_m;
            let source_height =
                terrain.sample_height_world(sample_x, sample_z) * config::HEIGHT_SCALE;
            let transition_height = if cut_side {
                road_point.y + slope_rate * distance_m
            } else {
                road_point.y - slope_rate * distance_m
            };
            let rejoins_source = if cut_side {
                transition_height >= source_height
            } else {
                transition_height <= source_height
            };
            if rejoins_source {
                return distance_m;
            }
            distance_m += EARTHWORK_MARGIN_SAMPLE_STEP_M;
        }

        EARTHWORK_MAX_MARGIN_M
    }

    fn stamp_visual_span_piece_earthworks_for_chunk(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
    ) {
        if !self.span_piece_uses_visible_earthwork(piece) {
            return;
        }

        let height_offset_m = self.span_piece_integrated_surface_offset_m(piece);
        self.stamp_span_top_surface_support_for_chunk(
            &piece.span_earthwork_support_regions,
            chunk,
            terrain,
            height_offset_m,
        );
    }

    fn stamp_visual_node_piece_earthworks_for_chunk(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
    ) {
        if !self.node_piece_uses_earthworks(graph, node_id, terrain) {
            return;
        }
        if !self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
            return;
        }

        let height_offset_m = self.node_piece_integrated_surface_offset_m(graph, node_id, terrain);
        self.stamp_piece_top_surface_clearance_for_chunk(
            &piece.road_surface_polygons,
            &piece.curb_surface_polygons,
            &piece.sidewalk_surface_polygons,
            chunk,
            terrain,
            height_offset_m,
        );
    }

    pub(super) fn node_piece_uses_earthworks(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        terrain: &TerrainSystem,
    ) -> bool {
        if node_id as usize >= graph.node_adjacency_count() {
            return false;
        }

        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted || !Self::is_surface_edge(edge) {
                continue;
            }
            if edge.class != EdgeClass::Tunnel || edge.primary_type == TransitType::Foot {
                return true;
            }

            let at_start = graph.get_valid_node(edge.start_node) == node_id;
            if self.tunnel_throat_is_visible(edge_idx, at_start, terrain) {
                return true;
            }
        }

        false
    }

    pub(super) fn earthwork_section_ranges_for_edge(
        &self,
        edge: &Edge,
        sections: &[RoadSurfaceSection],
        terrain: &TerrainSystem,
    ) -> Vec<(usize, usize)> {
        let Some((start_index, end_index)) = self.corridor_index_range_for_edge(edge, sections)
        else {
            return Vec::new();
        };

        match edge.class {
            EdgeClass::Standard => vec![(start_index, end_index)],
            EdgeClass::Bridge => self.endpoint_limited_section_ranges(
                sections,
                start_index,
                end_index,
                BRIDGE_ABUTMENT_LENGTH_M,
            ),
            EdgeClass::Tunnel => {
                self.tunnel_visible_section_ranges(sections, start_index, end_index, terrain)
            }
        }
    }

    fn corridor_index_range_for_edge(
        &self,
        edge: &Edge,
        sections: &[RoadSurfaceSection],
    ) -> Option<(usize, usize)> {
        if sections.len() < 2 {
            return None;
        }

        // Bridge abutments and tunnel portals are structural endpoint regions; trimming them by
        // the ordinary road-width handoff can either erase portals or collapse short spans into
        // one full-length stamp.
        if edge.class != EdgeClass::Standard {
            return Some((0, sections.len().saturating_sub(1)));
        }

        let total_length = sections.last()?.s_m.max(0.0);
        let start_handoff = Self::visual_start_handoff_m(edge, total_length);
        let end_handoff = Self::visual_end_handoff_s_m(edge, total_length);
        Self::section_index_range_for_s_bounds(sections, start_handoff, end_handoff)
    }

    fn endpoint_limited_section_ranges(
        &self,
        sections: &[RoadSurfaceSection],
        start_index: usize,
        end_index: usize,
        endpoint_length_m: f32,
    ) -> Vec<(usize, usize)> {
        if end_index <= start_index {
            return Vec::new();
        }

        let start_s = sections[start_index].s_m;
        let end_s = sections[end_index].s_m;
        if end_s - start_s <= endpoint_length_m * 2.0 {
            return vec![(start_index, end_index)];
        }

        let mut ranges = Vec::new();
        if let Some(start_end) = sections[start_index..=end_index]
            .iter()
            .rposition(|section| section.s_m <= start_s + endpoint_length_m + SAMPLE_EPSILON_M)
            .map(|offset| start_index + offset)
        {
            if start_end > start_index {
                ranges.push((start_index, start_end));
            }
        }

        if let Some(end_start) = sections[start_index..=end_index]
            .iter()
            .position(|section| section.s_m >= end_s - endpoint_length_m - SAMPLE_EPSILON_M)
            .map(|offset| start_index + offset)
        {
            if end_index > end_start {
                ranges.push((end_start, end_index));
            }
        }

        ranges.sort_unstable();
        ranges.dedup();
        ranges
    }

    pub(super) fn tunnel_visible_section_ranges(
        &self,
        sections: &[RoadSurfaceSection],
        start_index: usize,
        end_index: usize,
        terrain: &TerrainSystem,
    ) -> Vec<(usize, usize)> {
        if end_index <= start_index {
            return Vec::new();
        }

        let mut ranges = Vec::new();

        if self.section_is_tunnel_surface_visible(&sections[start_index], terrain) {
            let mut visible_end = start_index;
            while visible_end < end_index
                && self.section_is_tunnel_surface_visible(&sections[visible_end + 1], terrain)
            {
                visible_end += 1;
            }
            let transition_end = (visible_end + 1).min(end_index);
            if transition_end > start_index {
                ranges.push((start_index, transition_end));
            }
        }

        if self.section_is_tunnel_surface_visible(&sections[end_index], terrain) {
            let mut visible_start = end_index;
            while visible_start > start_index
                && self.section_is_tunnel_surface_visible(&sections[visible_start - 1], terrain)
            {
                visible_start -= 1;
            }
            let transition_start = visible_start.saturating_sub(1).max(start_index);
            if end_index > transition_start {
                if let Some(last) = ranges.last_mut() {
                    if transition_start <= last.1 {
                        last.1 = end_index;
                    } else {
                        ranges.push((transition_start, end_index));
                    }
                } else {
                    ranges.push((transition_start, end_index));
                }
            }
        }

        ranges
    }

    fn section_is_tunnel_surface_visible(
        &self,
        section: &RoadSurfaceSection,
        terrain: &TerrainSystem,
    ) -> bool {
        let terrain_height = terrain.sample_height_world(section.center_xz.x, section.center_xz.y)
            * config::HEIGHT_SCALE;
        section.center_height_m >= terrain_height - TUNNEL_PORTAL_STAMP_DEPTH_M
    }

    pub(super) fn tunnel_throat_is_visible(
        &self,
        edge_idx: usize,
        at_start: bool,
        terrain: &TerrainSystem,
    ) -> bool {
        let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
            return false;
        };
        let mouth = if at_start {
            piece.start_mouth_profile.as_ref()
        } else {
            piece.end_mouth_profile.as_ref()
        };
        let Some(mouth) = mouth else {
            return false;
        };
        let mut average_point = Vector3::ZERO;
        for point in &mouth.boundary_points_world {
            average_point += *point;
        }
        average_point /= mouth.boundary_points_world.len() as f32;
        let terrain_height =
            terrain.sample_height_world(average_point.x, average_point.z) * config::HEIGHT_SCALE;
        average_point.y >= terrain_height - TUNNEL_PORTAL_STAMP_DEPTH_M
    }

    fn stamp_piece_top_surface_clearance_for_chunk(
        &self,
        road_surface_polygons: &[RoadSurfaceVisualPolygon],
        curb_surface_polygons: &[RoadSurfaceVisualPolygon],
        sidewalk_surface_polygons: &[RoadSurfaceVisualPolygon],
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
        height_offset_m: f32,
    ) {
        let conservative_margin_m = terrain.cell_size_m() * std::f32::consts::SQRT_2 * 0.5;
        let mut candidates: HashMap<(usize, usize), (f32, f32)> = HashMap::new();

        for polygon in road_surface_polygons
            .iter()
            .chain(curb_surface_polygons)
            .chain(sidewalk_surface_polygons)
        {
            Self::visit_visual_polygon_triangles(polygon, &mut |triangle| {
                self.collect_top_surface_support_triangle_candidates(
                    terrain,
                    chunk,
                    triangle,
                    conservative_margin_m,
                    height_offset_m,
                    &mut candidates,
                );
            });
        }

        for ((grid_x, grid_z), (_, height_sample)) in candidates {
            terrain.set_visual_height_at_grid(grid_x, grid_z, height_sample);
        }
    }

    fn stamp_span_top_surface_support_for_chunk(
        &self,
        regions: &[RoadSurfaceSpanOwnedRegion],
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
        height_offset_m: f32,
    ) {
        let conservative_margin_m = terrain.cell_size_m() * std::f32::consts::SQRT_2 * 0.5;
        let mut candidates: HashMap<(usize, usize), (f32, f32)> = HashMap::new();

        for region in regions {
            Self::visit_visual_polygon_triangles(&region.polygon, &mut |triangle| {
                self.collect_top_surface_support_triangle_candidates(
                    terrain,
                    chunk,
                    triangle,
                    conservative_margin_m,
                    height_offset_m,
                    &mut candidates,
                );
            });
        }

        for ((grid_x, grid_z), (_, height_sample)) in candidates {
            terrain.set_visual_height_at_grid(grid_x, grid_z, height_sample);
        }
    }

    fn collect_top_surface_support_triangle_candidates(
        &self,
        terrain: &TerrainSystem,
        chunk: SurfaceChunkKey,
        triangle: [Vector3; 3],
        conservative_margin_m: f32,
        height_offset_m: f32,
        candidates: &mut HashMap<(usize, usize), (f32, f32)>,
    ) {
        if !Self::triangle_has_area_xz(triangle) {
            return;
        }

        let (chunk_min, chunk_max) = self.chunk_bounds(chunk);
        let min_x = triangle
            .iter()
            .map(|point| point.x)
            .fold(chunk_max.x, f32::min)
            .max(chunk_min.x - conservative_margin_m);
        let max_x = triangle
            .iter()
            .map(|point| point.x)
            .fold(chunk_min.x, f32::max)
            .min(chunk_max.x + conservative_margin_m);
        let min_z = triangle
            .iter()
            .map(|point| point.z)
            .fold(chunk_max.z, f32::min)
            .max(chunk_min.z - conservative_margin_m);
        let max_z = triangle
            .iter()
            .map(|point| point.z)
            .fold(chunk_min.z, f32::max)
            .min(chunk_max.z + conservative_margin_m);
        let Some((min_grid_x, max_grid_x, min_grid_z, max_grid_z)) =
            terrain.grid_rect_for_world_bounds(min_x, min_z, max_x, max_z)
        else {
            return;
        };
        let (grid_width, grid_height) = terrain.grid_dimensions();
        if grid_width == 0 || grid_height == 0 {
            return;
        }
        let max_grid_x_index = grid_width.saturating_sub(1);
        let max_grid_z_index = grid_height.saturating_sub(1);
        let grid_min_x = min_grid_x.saturating_sub(1).min(max_grid_x_index);
        let grid_max_x = max_grid_x.saturating_add(1).min(max_grid_x_index);
        let grid_min_z = min_grid_z.saturating_sub(1).min(max_grid_z_index);
        let grid_max_z = max_grid_z.saturating_add(1).min(max_grid_z_index);

        for grid_z in grid_min_z..=grid_max_z {
            for grid_x in grid_min_x..=grid_max_x {
                let (world_x, world_z) = terrain.grid_to_world_coords(grid_x, grid_z);
                let point_xz = Vector2::new(world_x, world_z);
                if !Self::point_is_inside_or_near_triangle_xz(
                    triangle,
                    point_xz,
                    conservative_margin_m,
                ) {
                    continue;
                }
                let Some((distance_squared, height_sample)) =
                    Self::top_surface_support_candidate_from_triangle(
                        triangle,
                        point_xz,
                        height_offset_m,
                    )
                else {
                    continue;
                };
                let entry = candidates
                    .entry((grid_x, grid_z))
                    .or_insert((distance_squared, height_sample));
                if Self::top_surface_support_candidate_replaces(
                    *entry,
                    (distance_squared, height_sample),
                ) {
                    *entry = (distance_squared, height_sample);
                }
            }
        }
    }

    fn top_surface_support_candidate_replaces(existing: (f32, f32), candidate: (f32, f32)) -> bool {
        let (existing_distance_squared, existing_height_sample) = existing;
        let (candidate_distance_squared, candidate_height_sample) = candidate;
        candidate_distance_squared < existing_distance_squared - 0.0001
            || ((candidate_distance_squared - existing_distance_squared).abs() <= 0.0001
                && candidate_height_sample < existing_height_sample)
    }

    fn top_surface_support_candidate_from_triangle(
        triangle: [Vector3; 3],
        point_xz: Vector2,
        height_offset_m: f32,
    ) -> Option<(f32, f32)> {
        let sample_point_xz = Self::closest_point_on_triangle_xz(triangle, point_xz);
        let (wa, wb, wc) = Self::triangle_barycentric_weights_xz(triangle, sample_point_xz)?;
        let support_height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
        let clearance_sample = (support_height_m - height_offset_m) / config::HEIGHT_SCALE;
        Some((
            point_xz.distance_squared_to(sample_point_xz),
            clearance_sample,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earthwork_support_candidates_use_lower_envelope_for_overlapping_top_surfaces() {
        assert!(RoadSurfaceSystem::top_surface_support_candidate_replaces(
            (0.0, 0.30),
            (0.0, 0.10),
        ));
        assert!(!RoadSurfaceSystem::top_surface_support_candidate_replaces(
            (0.0, 0.10),
            (0.0, 0.30),
        ));
    }

    #[test]
    fn earthwork_support_candidates_keep_nearest_non_overlapping_surface() {
        assert!(RoadSurfaceSystem::top_surface_support_candidate_replaces(
            (0.50, 0.10),
            (0.10, 0.30),
        ));
        assert!(!RoadSurfaceSystem::top_surface_support_candidate_replaces(
            (0.10, 0.30),
            (0.50, 0.10),
        ));
    }

    #[test]
    fn earthwork_hardcut_has_no_per_material_sequential_stamping_path() {
        let source = include_str!("earthwork.rs");
        for forbidden in [
            concat!("stamp_piece_surface_", "geometry_for_chunk"),
            concat!("profile_clearance_", "candidate_from_triangle"),
            concat!("collect_profile_clearance_", "triangle_candidates"),
        ] {
            assert!(
                !source.contains(forbidden),
                "road-touched terrain support must use one canonical lower-envelope pass, not `{forbidden}`"
            );
        }
    }
}
