//! Public road-surface data model, ownership cache, and compile orchestration.
//!
//! The sibling modules own the concrete edge, span, node, overlay, query,
//! earthwork, geometry, cache, and debug implementations. This file keeps the
//! shared public contracts and deterministic rebuild flow in one place.

use godot::prelude::{Vector2, Vector3};
use rayon::prelude::*;
use spade::{ConstrainedDelaunayTriangulation, Point2};
use std::collections::{BTreeSet, HashMap, HashSet};

use super::graph::RegionGraph;
use super::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;

mod arrangement;
mod backend;
mod cache;
mod debug;
mod earthwork;
mod edge;
mod geometry;
mod height;
mod input;
mod node;
mod overlay;
mod ownership;
mod query;
mod rails;
mod span;
mod triangulation;
mod validation;

// Shared geometric tolerances used across surface compilation, overlay solving, and queries.
const SAMPLE_EPSILON_M: f32 = 0.001;
const WORLD_POINT_DEDUP_DISTANCE_M: f32 = 1.0e-4;
const WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2: f32 =
    WORLD_POINT_DEDUP_DISTANCE_M * WORLD_POINT_DEDUP_DISTANCE_M;
// Shared overlay/geometry area floor: one 1 mm quantized square keeps closure slivers visible.
const NODE_OVERLAY_MIN_AREA_M2: f32 = 1.0e-6;
// Self-checks that compare two backend results allow a small fixed-grid residual budget.
const NODE_OVERLAY_NUMERIC_AREA_EPS_M2: f32 = NODE_OVERLAY_MIN_AREA_M2 * 16.0;
const NODE_OVERLAY_NUMERIC_DUST_WIDTH_M: f32 = WORLD_POINT_DEDUP_DISTANCE_M;
const NODE_OVERLAY_NUMERIC_AREA_CAP_M2: f32 = 1.0e-3;
// Avoid Rayon setup overhead for the small edge/node sets common in single-edit rebuilds.
const PARALLEL_SURFACE_COMPILE_MIN_ITEMS: usize = 16;

type SurfaceCdt = ConstrainedDelaunayTriangulation<Point2<f64>>;
type NodeOverlayPoint = [f64; 2];
type NodeOverlayPointKey = (i64, i64);
type NodeOverlayContour = Vec<NodeOverlayPoint>;
type NodeOverlayShape = Vec<NodeOverlayContour>;
type NodeOverlayShapes = Vec<NodeOverlayShape>;

/// Chunk key used by the road-surface and earthwork caches.
pub type SurfaceChunkKey = (i32, i32);

/// Ordered lateral surface-band kinds supported by the compiled roadbed.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RoadSurfaceBandKind {
    /// Main drivable carriageway surface.
    Carriageway,
    /// Curb or shoulder transition surface adjacent to the carriageway.
    CurbOrShoulder,
    /// Walkable sidewalk surface.
    Sidewalk,
    /// Dedicated pedestrian corridor that is not a roadside sidewalk band.
    Footpath,
    /// Reserved central median or separator.
    Median,
    /// Reserved parking band.
    Parking,
    /// Reserved bicycle band.
    CycleTrack,
    /// Reserved tram corridor.
    TramReservation,
}

/// One ordered lateral band inside a compiled roadbed section.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceBand {
    /// Surface-band classification.
    pub kind: RoadSurfaceBandKind,
    /// Inclusive lateral start offset from the section centerline in world metres.
    pub lateral_start_m: f32,
    /// Inclusive lateral end offset from the section centerline in world metres.
    pub lateral_end_m: f32,
    /// Height in world metres at `lateral_start_m`.
    pub height_start_m: f32,
    /// Height in world metres at `lateral_end_m`.
    pub height_end_m: f32,
}

/// One sampled cross-section along an edge in the compiled roadbed model.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceSection {
    /// Owning edge id.
    pub edge_idx: usize,
    /// Longitudinal distance from the edge start in world metres.
    pub s_m: f32,
    /// Section center point in world-space XZ metres.
    pub center_xz: Vector2,
    /// Solved center height in world metres.
    pub center_height_m: f32,
    /// Unit tangent vector in XZ.
    pub tangent_xz: Vector2,
    /// Unit lateral axis in XZ.
    pub lateral_xz: Vector2,
    /// Ordered lateral bands for this section.
    pub bands: Vec<RoadSurfaceBand>,
}

/// Piece classification for explicit visual node ownership during the graph/visual split.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoadSurfaceVisualNodePieceKind {
    /// One incident surface edge ends here and requires a terminal visual piece.
    Terminal,
    /// Two non-pass-through incident edges require one explicit bend visual piece.
    Bend,
    /// Three or more incident edges require an explicit multi-mouth junction visual piece.
    JunctionN,
}

/// One explicit polygon owned by the visual road carrier.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceVisualPolygon {
    /// Ordered world-space polygon points.
    pub points_world: Vec<Vector3>,
    /// Deterministic cached triangles covering the polygon in world space.
    pub triangles_world: Vec<[Vector3; 3]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoadSurfaceEarthworkFaceKind {
    Slope,
    RetainingWall,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadSurfaceEarthworkRenderFace {
    pub(crate) kind: RoadSurfaceEarthworkFaceKind,
    pub(crate) inner_start: Vector3,
    pub(crate) inner_end: Vector3,
    pub(crate) polygon: RoadSurfaceVisualPolygon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RoadSurfaceVerticalFaceSource {
    explicit_vertical_step_index: usize,
    segment: arrangement::NodeExplicitVerticalStepSegment,
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
    terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
    /// Explicit asphalt-owned polygons for the node piece.
    pub road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit curb / shoulder-owned polygons for the node piece.
    pub curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit vertical faces at raised non-road material contacts.
    pub curb_vertical_face_polygons: Vec<RoadSurfaceVisualPolygon>,
    curb_vertical_face_sources: Vec<RoadSurfaceVerticalFaceSource>,
    /// Explicit sidewalk-owned polygons for the node piece.
    pub sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    explicit_vertical_step_segments: Vec<arrangement::NodeExplicitVerticalStepSegment>,
    owned_regions: Vec<NodeOwnedRegion>,
    pub(crate) earthwork_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) earthwork_outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) render_earthwork_faces: Vec<RoadSurfaceEarthworkRenderFace>,
}

/// Explicit visual span piece compiled from one edge corridor.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceVisualSpanPiece {
    /// Owning edge id.
    pub edge_idx: usize,
    /// Outer piece-owned boundaries used for debug, surface chunk bounds, and terrain clipping.
    pub outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
    /// Explicit asphalt-owned polygons for the span piece.
    pub road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit curb / shoulder-owned polygons for the span piece.
    pub curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit vertical curb faces at asphalt / curb material contacts.
    pub curb_vertical_face_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit sidewalk-owned polygons for the span piece.
    pub sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    edge_class: EdgeClass,
    start_mouth_profile: Option<IncidentMouthProfile>,
    end_mouth_profile: Option<IncidentMouthProfile>,
    clearance_road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    clearance_curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    clearance_sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) earthwork_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) earthwork_outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) render_earthwork_faces: Vec<RoadSurfaceEarthworkRenderFace>,
}

/// Cached render-side surface ownership for one chunk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoadSurfaceChunkCacheEntry {
    /// Owning chunk key.
    pub chunk: SurfaceChunkKey,
    /// Surface edges contributing cached geometry to this chunk.
    pub edge_indices: Vec<usize>,
    /// Surface nodes contributing cached patches to this chunk.
    pub node_ids: Vec<u32>,
}

/// Cached terrain-earthwork ownership for one chunk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoadEarthworkChunkCacheEntry {
    /// Owning chunk key.
    pub chunk: SurfaceChunkKey,
    /// Surface edges contributing earthworks to this chunk.
    pub edge_indices: Vec<usize>,
    /// Surface nodes contributing earthworks to this chunk.
    pub node_ids: Vec<u32>,
}

/// Temporary preview compile output for one road-tool stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewRoadSurfaceResult {
    /// Edge class inferred from the preview stroke before temporary compilation.
    pub edge_class: EdgeClass,
    /// Prepared centerline points after the same grounding, simplification, and smoothing rules
    /// used by committed placement.
    pub prepared_points: Vec<Vector3>,
    /// Compiled section cache for the temporary preview edge.
    pub compiled_sections: Vec<RoadSurfaceSection>,
    /// Explicit visual node pieces for the temporary preview edge endpoints.
    pub compiled_visual_node_pieces: Vec<RoadSurfaceVisualNodePiece>,
    /// Triangulated top-surface preview mesh vertices, lifted slightly for editor visibility.
    pub surface_vertices: Vec<Vector3>,
    /// Preview validity after grade and bridge / tunnel clearance checks.
    pub is_valid: bool,
}

#[derive(Default)]
pub(crate) struct RoadSurfaceDebugData {
    pub(crate) section_lines: Vec<Vector3>,
    pub(crate) band_lines: Vec<Vector3>,
    pub(crate) piece_boundary_lines: Vec<Vector3>,
    pub(crate) earthwork_chunk_lines: Vec<Vector3>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
enum IncidentEdgeSide {
    Start,
    End,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompiledNodeKind {
    Terminal,
    PassThrough,
    Bend,
    JunctionN,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ChunkCacheKind {
    Surface,
    Earthwork,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct IncidentSurfaceEdge {
    edge_idx: usize,
    side: IncidentEdgeSide,
    direction_xz: Vector2,
}

#[derive(Clone, Debug, PartialEq)]
struct IncidentMouthBand {
    kind: RoadSurfaceBandKind,
    start_point_world: Vector3,
    end_point_world: Vector3,
}

#[derive(Clone, Debug, PartialEq)]
struct IncidentMouthProfile {
    inward_direction_xz: Vector2,
    boundary_points_world: Vec<Vector3>,
    bands: Vec<IncidentMouthBand>,
}

#[derive(Clone, Debug, PartialEq)]
struct OrderedIncidentPieceMouth {
    profile: IncidentMouthProfile,
    endpoint_profile: IncidentMouthProfile,
    boundary_paths_world: Vec<Vec<Vector3>>,
    band_start_paths_world: Vec<Vec<Vector3>>,
    band_end_paths_world: Vec<Vec<Vector3>>,
    uses_sampled_band_domain_paths: bool,
    direction_angle_ccw: f32,
    direction_xz: Vector2,
    edge_idx: usize,
    side: IncidentEdgeSide,
}

#[derive(Clone, Debug, PartialEq)]
struct NodeOwnedRegion {
    kind: RoadSurfaceBandKind,
    owner_index: usize,
    polygon: RoadSurfaceVisualPolygon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum RoadSurfaceTerrainClipEdgeKind {
    SidewalkOuter,
    ShoulderOuter,
    FootprintBoundary,
    SpanHandoff,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RoadSurfaceTerrainClipSourceEdge {
    start: Vector3,
    end: Vector3,
    kind: RoadSurfaceTerrainClipEdgeKind,
}

#[derive(Clone, Debug, PartialEq)]
struct RoadSurfaceTerrainClipLoop {
    points_world: Vec<Vector3>,
    source_edges: Vec<RoadSurfaceTerrainClipSourceEdge>,
}

fn terrain_clip_edge_kind_for_band(kind: RoadSurfaceBandKind) -> RoadSurfaceTerrainClipEdgeKind {
    match kind {
        RoadSurfaceBandKind::Sidewalk => RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
        RoadSurfaceBandKind::CurbOrShoulder => RoadSurfaceTerrainClipEdgeKind::ShoulderOuter,
        _ => RoadSurfaceTerrainClipEdgeKind::FootprintBoundary,
    }
}

#[derive(Clone, Debug, PartialEq)]
struct NodeSurfaceRegionResult {
    outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    earthwork_boundary_point_loops: Vec<Vec<Vector3>>,
    terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
    road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    curb_vertical_faces: Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    explicit_vertical_step_segments: Vec<arrangement::NodeExplicitVerticalStepSegment>,
    owned_regions: Vec<NodeOwnedRegion>,
}

/// Ownership cache and compiler for the road-surface pipeline.
pub struct RoadSurfaceSystem {
    chunk_span_m: f32,
    compiled_once: bool,
    dirty_edges: HashSet<usize>,
    dirty_nodes: HashSet<u32>,
    dirty_surface_chunks: HashSet<SurfaceChunkKey>,
    dirty_terrain_chunks: HashSet<SurfaceChunkKey>,
    node_validation_logging_enabled: bool,
    compiled_sections: HashMap<usize, Vec<RoadSurfaceSection>>,
    compiled_visual_span_pieces: HashMap<usize, RoadSurfaceVisualSpanPiece>,
    compiled_visual_node_pieces: HashMap<u32, RoadSurfaceVisualNodePiece>,
    surface_span_chunks: HashMap<usize, Vec<SurfaceChunkKey>>,
    surface_node_chunks: HashMap<u32, Vec<SurfaceChunkKey>>,
    earthwork_span_chunks: HashMap<usize, Vec<SurfaceChunkKey>>,
    earthwork_node_chunks: HashMap<u32, Vec<SurfaceChunkKey>>,
    surface_chunk_spans: HashMap<SurfaceChunkKey, BTreeSet<usize>>,
    surface_chunk_nodes: HashMap<SurfaceChunkKey, BTreeSet<u32>>,
    earthwork_chunk_spans: HashMap<SurfaceChunkKey, BTreeSet<usize>>,
    earthwork_chunk_nodes: HashMap<SurfaceChunkKey, BTreeSet<u32>>,
    surface_chunk_cache: HashMap<SurfaceChunkKey, RoadSurfaceChunkCacheEntry>,
    earthwork_chunk_cache: HashMap<SurfaceChunkKey, RoadEarthworkChunkCacheEntry>,
    last_rebuilt_surface_chunks: Vec<SurfaceChunkKey>,
    last_rebuilt_terrain_chunks: Vec<SurfaceChunkKey>,
}

impl RoadSurfaceSystem {
    /// Creates an empty road-surface system using the given chunk span in world metres.
    pub fn new(chunk_span_m: f32) -> Self {
        Self {
            chunk_span_m: chunk_span_m.max(f32::EPSILON),
            compiled_once: false,
            dirty_edges: HashSet::new(),
            dirty_nodes: HashSet::new(),
            dirty_surface_chunks: HashSet::new(),
            dirty_terrain_chunks: HashSet::new(),
            node_validation_logging_enabled: true,
            compiled_sections: HashMap::new(),
            compiled_visual_span_pieces: HashMap::new(),
            compiled_visual_node_pieces: HashMap::new(),
            surface_span_chunks: HashMap::new(),
            surface_node_chunks: HashMap::new(),
            earthwork_span_chunks: HashMap::new(),
            earthwork_node_chunks: HashMap::new(),
            surface_chunk_spans: HashMap::new(),
            surface_chunk_nodes: HashMap::new(),
            earthwork_chunk_spans: HashMap::new(),
            earthwork_chunk_nodes: HashMap::new(),
            surface_chunk_cache: HashMap::new(),
            earthwork_chunk_cache: HashMap::new(),
            last_rebuilt_surface_chunks: Vec::new(),
            last_rebuilt_terrain_chunks: Vec::new(),
        }
    }

    /// Returns the configured chunk span in world metres.
    pub fn chunk_span_m(&self) -> f32 {
        self.chunk_span_m
    }

    /// Returns the set of edge ids that need road-surface recompilation.
    pub fn dirty_edges(&self) -> &HashSet<usize> {
        &self.dirty_edges
    }

    /// Returns the set of node ids that need node-patch recompilation.
    pub fn dirty_nodes(&self) -> &HashSet<u32> {
        &self.dirty_nodes
    }

    /// Returns the set of road-surface chunks that need render-cache rebuild.
    pub fn dirty_surface_chunks(&self) -> &HashSet<SurfaceChunkKey> {
        &self.dirty_surface_chunks
    }

    /// Returns the set of terrain chunks that need road-earthwork rebuild.
    pub fn dirty_terrain_chunks(&self) -> &HashSet<SurfaceChunkKey> {
        &self.dirty_terrain_chunks
    }

    /// Returns the currently cached compiled sections by edge id.
    pub fn compiled_sections(&self) -> &HashMap<usize, Vec<RoadSurfaceSection>> {
        &self.compiled_sections
    }

    /// Returns the currently cached explicit visual span pieces by edge id.
    pub fn compiled_visual_span_pieces(&self) -> &HashMap<usize, RoadSurfaceVisualSpanPiece> {
        &self.compiled_visual_span_pieces
    }

    /// Returns the currently cached explicit visual node pieces by node id.
    pub fn compiled_visual_node_pieces(&self) -> &HashMap<u32, RoadSurfaceVisualNodePiece> {
        &self.compiled_visual_node_pieces
    }

    /// Returns the current per-chunk surface cache shell.
    pub fn surface_chunk_cache(&self) -> &HashMap<SurfaceChunkKey, RoadSurfaceChunkCacheEntry> {
        &self.surface_chunk_cache
    }

    /// Returns the current per-chunk earthwork cache shell.
    pub fn earthwork_chunk_cache(&self) -> &HashMap<SurfaceChunkKey, RoadEarthworkChunkCacheEntry> {
        &self.earthwork_chunk_cache
    }

    /// Compiles the road-surface cache if it is dirty or has not been built yet.
    pub fn compile_dirty(&mut self, graph: &RegionGraph, terrain: &TerrainSystem) {
        if !self.compiled_once {
            self.compile_all(graph, terrain);
            return;
        }

        if self.dirty_edges.is_empty()
            && self.dirty_nodes.is_empty()
            && self.dirty_surface_chunks.is_empty()
            && self.dirty_terrain_chunks.is_empty()
        {
            return;
        }

        self.prune_stale_cache_entries(graph);

        let mut edge_ids: Vec<usize> = self.dirty_edges.iter().copied().collect();
        edge_ids.sort_unstable();

        let mut node_ids: HashSet<u32> = self
            .dirty_nodes
            .iter()
            .copied()
            .map(|node_id| graph.get_valid_node(node_id))
            .collect();
        for &edge_idx in &edge_ids {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            node_ids.insert(graph.get_valid_node(edge.start_node));
            node_ids.insert(graph.get_valid_node(edge.end_node));
        }

        let mut sorted_nodes: Vec<u32> = node_ids.into_iter().collect();
        sorted_nodes.sort_unstable();
        sorted_nodes.dedup();

        let mut span_edge_ids: HashSet<usize> = self.dirty_edges.iter().copied().collect();
        for &node_id in &sorted_nodes {
            if node_id as usize >= graph.node_adjacency_count() {
                continue;
            }
            for &edge_idx in graph.node_adjacency(node_id) {
                span_edge_ids.insert(edge_idx);
            }
        }
        let mut sorted_span_edges: Vec<usize> = span_edge_ids.into_iter().collect();
        sorted_span_edges.sort_unstable();
        sorted_span_edges.dedup();
        let section_results: Vec<(usize, Option<Vec<RoadSurfaceSection>>)> =
            Self::collect_surface_compile_work(&sorted_span_edges, |edge_idx| {
                if edge_idx >= graph.edge_count() {
                    return (edge_idx, None);
                }
                let edge = graph.edge(edge_idx);
                if !Self::is_surface_edge(edge) {
                    return (edge_idx, None);
                }
                (edge_idx, Some(self.compile_edge_sections(graph, edge_idx)))
            });
        for (edge_idx, sections) in section_results {
            if let Some(sections) = sections {
                self.compiled_sections.insert(edge_idx, sections);
            } else {
                self.compiled_sections.remove(&edge_idx);
            }
        }

        let mut span_candidates = Vec::new();
        for &edge_idx in &sorted_span_edges {
            self.remove_span_piece_coverage(edge_idx);
            if edge_idx >= graph.edge_count() {
                self.compiled_visual_span_pieces.remove(&edge_idx);
                continue;
            }
            let edge = graph.edge(edge_idx);
            if !Self::is_surface_edge(edge) {
                self.compiled_visual_span_pieces.remove(&edge_idx);
                continue;
            }
            span_candidates.push(edge_idx);
        }
        let span_results: Vec<(usize, Option<RoadSurfaceVisualSpanPiece>)> =
            Self::collect_surface_compile_work(&span_candidates, |edge_idx| {
                (
                    edge_idx,
                    self.compile_visual_span_piece(graph, terrain, edge_idx),
                )
            });
        for (edge_idx, span_piece) in span_results {
            if let Some(span_piece) = span_piece {
                self.insert_span_piece_coverage(&span_piece);
                self.compiled_visual_span_pieces
                    .insert(edge_idx, span_piece);
            } else {
                self.compiled_visual_span_pieces.remove(&edge_idx);
            }
        }

        let mut node_candidates = Vec::new();
        for &node_id in &sorted_nodes {
            self.remove_node_piece_coverage(node_id);
            if self.node_has_surface_edges(graph, node_id) {
                node_candidates.push(node_id);
            } else {
                self.compiled_visual_node_pieces.remove(&node_id);
            }
        }
        let node_results: Vec<(u32, Option<RoadSurfaceVisualNodePiece>)> =
            Self::collect_surface_compile_work(&node_candidates, |node_id| {
                (
                    node_id,
                    self.compile_visual_node_piece(graph, terrain, node_id),
                )
            });
        for (node_id, visual_piece) in node_results {
            if let Some(visual_piece) = visual_piece {
                self.insert_node_piece_coverage(&visual_piece);
                self.compiled_visual_node_pieces
                    .insert(node_id, visual_piece);
            } else {
                self.compiled_visual_node_pieces.remove(&node_id);
            }
        }

        let dirty_surface_chunks = self.sorted_chunk_keys(&self.dirty_surface_chunks);
        let dirty_terrain_chunks = self.sorted_chunk_keys(&self.dirty_terrain_chunks);
        self.rebuild_surface_chunk_cache(&dirty_surface_chunks);
        self.rebuild_earthwork_chunk_cache(&dirty_terrain_chunks);
        self.last_rebuilt_surface_chunks = dirty_surface_chunks;
        self.last_rebuilt_terrain_chunks = dirty_terrain_chunks;
        self.compiled_once = true;
        self.clear_dirty_tracking();
    }

    fn compile_all(&mut self, graph: &RegionGraph, terrain: &TerrainSystem) {
        self.prune_stale_cache_entries(graph);
        self.clear_piece_chunk_coverage();
        self.surface_chunk_cache.clear();
        self.earthwork_chunk_cache.clear();

        let edge_ids = self.all_surface_edge_ids(graph);
        let section_results: Vec<(usize, Vec<RoadSurfaceSection>)> =
            Self::collect_surface_compile_work(&edge_ids, |edge_idx| {
                (edge_idx, self.compile_edge_sections(graph, edge_idx))
            });
        for (edge_idx, sections) in section_results {
            self.compiled_sections.insert(edge_idx, sections);
        }

        let span_results: Vec<(usize, Option<RoadSurfaceVisualSpanPiece>)> =
            Self::collect_surface_compile_work(&edge_ids, |edge_idx| {
                (
                    edge_idx,
                    self.compile_visual_span_piece(graph, terrain, edge_idx),
                )
            });
        for (edge_idx, span_piece) in span_results {
            if let Some(span_piece) = span_piece {
                self.insert_span_piece_coverage(&span_piece);
                self.compiled_visual_span_pieces
                    .insert(edge_idx, span_piece);
            } else {
                self.compiled_visual_span_pieces.remove(&edge_idx);
            }
        }

        let node_ids = self.all_surface_node_ids(graph);
        let node_results: Vec<(u32, Option<RoadSurfaceVisualNodePiece>)> =
            Self::collect_surface_compile_work(&node_ids, |node_id| {
                (
                    node_id,
                    self.compile_visual_node_piece(graph, terrain, node_id),
                )
            });
        for (node_id, visual_piece) in node_results {
            if let Some(visual_piece) = visual_piece {
                self.insert_node_piece_coverage(&visual_piece);
                self.compiled_visual_node_pieces
                    .insert(node_id, visual_piece);
            } else {
                self.compiled_visual_node_pieces.remove(&node_id);
            }
        }

        let all_surface_chunks = self.collect_all_chunks(ChunkCacheKind::Surface);
        let all_earthwork_chunks = self.collect_all_chunks(ChunkCacheKind::Earthwork);
        self.rebuild_surface_chunk_cache(&all_surface_chunks);
        self.rebuild_earthwork_chunk_cache(&all_earthwork_chunks);
        self.last_rebuilt_surface_chunks = all_surface_chunks;
        self.last_rebuilt_terrain_chunks = all_earthwork_chunks;
        self.compiled_once = true;
        self.clear_dirty_tracking();
    }

    fn collect_surface_compile_work<I, O, F>(items: &[I], work: F) -> Vec<O>
    where
        I: Copy + Send + Sync,
        O: Send,
        F: Fn(I) -> O + Sync,
    {
        // Slice parallel iterators are indexed; collecting into Vec preserves input order, so
        // the serial commit phase remains deterministic without re-sorting by id.
        if items.len() >= PARALLEL_SURFACE_COMPILE_MIN_ITEMS {
            items.par_iter().copied().map(&work).collect()
        } else {
            items.iter().copied().map(&work).collect()
        }
    }

    fn section_index_range_for_s_bounds(
        sections: &[RoadSurfaceSection],
        start_s_m: f32,
        end_s_m: f32,
    ) -> Option<(usize, usize)> {
        if sections.len() < 2 || end_s_m - start_s_m <= SAMPLE_EPSILON_M {
            return None;
        }

        let start_index = sections
            .iter()
            .position(|section| section.s_m + SAMPLE_EPSILON_M >= start_s_m)
            .unwrap_or(0);
        let end_index = sections
            .iter()
            .rposition(|section| section.s_m - SAMPLE_EPSILON_M <= end_s_m)
            .unwrap_or(sections.len().saturating_sub(1));
        (end_index > start_index).then_some((start_index, end_index))
    }

    fn sort_visual_polygons(polygons: &mut [RoadSurfaceVisualPolygon]) {
        polygons.sort_by(Self::visual_polygon_ordering);
    }

    fn visual_polygon_ordering(
        a: &RoadSurfaceVisualPolygon,
        b: &RoadSurfaceVisualPolygon,
    ) -> std::cmp::Ordering {
        match (a.points_world.first(), b.points_world.first()) {
            (Some(point_a), Some(point_b)) => point_a
                .x
                .total_cmp(&point_b.x)
                .then(point_a.z.total_cmp(&point_b.z))
                .then(point_a.y.total_cmp(&point_b.y)),
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then(a.points_world.len().cmp(&b.points_world.len()))
        .then_with(|| {
            a.points_world
                .iter()
                .zip(&b.points_world)
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
    }

    fn sort_terrain_clip_loops(loops: &mut [RoadSurfaceTerrainClipLoop]) {
        loops.sort_by(|a, b| {
            match (a.points_world.first(), b.points_world.first()) {
                (Some(point_a), Some(point_b)) => point_a
                    .x
                    .total_cmp(&point_b.x)
                    .then(point_a.z.total_cmp(&point_b.z))
                    .then(point_a.y.total_cmp(&point_b.y)),
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then(a.points_world.len().cmp(&b.points_world.len()))
            .then_with(|| {
                a.points_world
                    .iter()
                    .zip(&b.points_world)
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

    fn sort_node_owned_regions(regions: &mut [NodeOwnedRegion]) {
        regions.sort_by(|a, b| {
            Self::band_kind_sort_key(a.kind)
                .cmp(&Self::band_kind_sort_key(b.kind))
                .then(a.owner_index.cmp(&b.owner_index))
                .then_with(|| {
                    match (
                        a.polygon.points_world.first(),
                        b.polygon.points_world.first(),
                    ) {
                        (Some(point_a), Some(point_b)) => point_a
                            .x
                            .total_cmp(&point_b.x)
                            .then(point_a.z.total_cmp(&point_b.z))
                            .then(point_a.y.total_cmp(&point_b.y)),
                        (None, Some(_)) => std::cmp::Ordering::Less,
                        (Some(_), None) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                })
                .then(
                    a.polygon
                        .points_world
                        .len()
                        .cmp(&b.polygon.points_world.len()),
                )
        });
    }
}

#[cfg(test)]
mod tests;
