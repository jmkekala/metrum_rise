//! Authoritative road-surface ownership layer and the first live compiler slices.
//!
//! This module now owns both the Phase 1 cache / dirty-tracking shell and the
//! first Phase 2 compiler pass for deterministic edge sections plus explicit
//! visual road pieces. It now drives the shipped preview, committed render mesh,
//! earthworks, and world-surface query paths from one deterministic compiled
//! roadbed cache.

use godot::prelude::{Vector2, Vector3};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::scale::FixedScaleFloatOverlay;
use i_overlay::float::simplify::SimplifyShape;
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;

use super::graph::{Edge, RegionGraph};
use super::types::{EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess};
use crate::config;
use crate::simulation::terrain::TerrainSystem;

const STANDARD_SECTION_STEP_M: f32 = 8.0;
const BRIDGE_SECTION_STEP_M: f32 = 12.0;
const TUNNEL_SECTION_STEP_M: f32 = 10.0;
const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;
const BAND_WIDTH_MATCH_EPSILON_M: f32 = 0.05;
const CURB_BAND_WIDTH_M: f32 = 0.15;
const CURB_STEP_HEIGHT_M: f32 = 0.12;
const MAX_STANDARD_DESIGN_CROSSFALL_RATE: f32 = 0.03;
const STANDARD_CROSSFALL_DEADZONE_RATE: f32 = 0.005;
const SAMPLE_EPSILON_M: f32 = 0.001;
const SURFACE_MIN_TRIANGLE_ALTITUDE_M: f32 = 0.05;
const ROAD_POINT_SIMPLIFY_DISTANCE_M: f32 = 0.5;
const TAUBIN_SMOOTHING_ITERS: usize = 50;
const TAUBIN_LAMBDA: f32 = 0.5;
const TAUBIN_MU: f32 = -0.53;
const PREVIEW_MAX_GRADE: f32 = 0.41;
const PREVIEW_CLEARANCE_M: f32 = 1.0;
const PREVIEW_MESH_LIFT_M: f32 = 0.05;
const VISUAL_NODE_HANDOFF_PADDING_M: f32 = 1.0;
const BEND_JOIN_ARC_SAMPLE_STEP_M: f32 = 0.75;
const EARTHWORK_PAVEMENT_DEPTH_M: f32 = 0.04;
const EARTHWORK_MIN_MARGIN_M: f32 = 4.0;
const EARTHWORK_MAX_MARGIN_M: f32 = 18.0;
const EARTHWORK_MARGIN_SAMPLE_STEP_M: f32 = 1.0;
const EARTHWORK_CUT_SLOPE_RATE: f32 = 0.5;
const EARTHWORK_FILL_SLOPE_RATE: f32 = 0.5;
const EARTHWORK_RETAINING_WALL_SLOPE_THRESHOLD: f32 = 1.25;
const BRIDGE_ABUTMENT_LENGTH_M: f32 = 12.0;
const TUNNEL_PORTAL_STAMP_DEPTH_M: f32 = 1.0;
const NODE_OVERLAY_SCALE: f32 = 1000.0;
const NODE_OVERLAY_MIN_AREA_M2: f32 = 0.002;

type SurfaceCdt = ConstrainedDelaunayTriangulation<Point2<f64>>;
type NodeOverlayPoint = [f32; 2];
type NodeOverlayContour = Vec<NodeOverlayPoint>;
type NodeOverlayShape = Vec<NodeOverlayContour>;
type NodeOverlayShapes = Vec<NodeOverlayShape>;

/// Chunk key used by the road-surface and earthwork caches.
pub type SurfaceChunkKey = (i32, i32);

/// Ordered lateral surface-band kinds supported by the replacement roadbed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

/// One sampled cross-section along an edge in the replacement roadbed model.
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
    pub(crate) polygon: RoadSurfaceVisualPolygon,
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
    /// Explicit asphalt-owned polygons for the node piece.
    pub road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit sidewalk-owned polygons for the node piece.
    pub sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
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
    /// Explicit asphalt-owned polygons for the span piece.
    pub road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit sidewalk-owned polygons for the span piece.
    pub sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    edge_class: EdgeClass,
    start_mouth_profile: Option<IncidentMouthProfile>,
    end_mouth_profile: Option<IncidentMouthProfile>,
    clearance_road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
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
    direction_angle_ccw: f32,
    direction_xz: Vector2,
    edge_idx: usize,
    side: IncidentEdgeSide,
}

#[derive(Clone, Debug, PartialEq)]
struct NodeCorridorCandidates {
    road_candidate_polygons: Vec<RoadSurfaceVisualPolygon>,
    non_road_candidate_polygons: Vec<NodeNonRoadCandidatePolygon>,
    non_road_height_candidate_polygons: Vec<RoadSurfaceVisualPolygon>,
}

#[derive(Clone, Debug, PartialEq)]
struct NodeNonRoadCandidatePolygon {
    polygon: RoadSurfaceVisualPolygon,
}

#[derive(Clone, Debug, PartialEq)]
struct NodeSurfaceRegionResult {
    outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
}

/// Ownership cache and compiler for the replacement road-surface pipeline.
pub struct RoadSurfaceSystem {
    chunk_span_m: f32,
    compiled_once: bool,
    dirty_edges: HashSet<usize>,
    dirty_nodes: HashSet<u32>,
    dirty_surface_chunks: HashSet<SurfaceChunkKey>,
    dirty_terrain_chunks: HashSet<SurfaceChunkKey>,
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

    pub(crate) fn terrain_render_patch_keys_with_visible_road(
        &self,
        terrain: &TerrainSystem,
    ) -> Vec<(usize, usize)> {
        let mut patch_keys = HashSet::new();

        for piece in self.compiled_visual_span_pieces.values() {
            let Some((min, max)) = self.visual_span_piece_bounds(piece, ChunkCacheKind::Surface)
            else {
                continue;
            };
            for key in terrain.render_patch_keys_for_world_bounds(min.x, min.z, max.x, max.z) {
                patch_keys.insert(key);
            }
        }

        for piece in self.compiled_visual_node_pieces.values() {
            let Some((min, max)) = self.visual_node_piece_bounds(piece, ChunkCacheKind::Surface)
            else {
                continue;
            };
            for key in terrain.render_patch_keys_for_world_bounds(min.x, min.z, max.x, max.z) {
                patch_keys.insert(key);
            }
        }

        let mut keys: Vec<(usize, usize)> = patch_keys.into_iter().collect();
        keys.sort_unstable();
        keys
    }

    pub(crate) fn terrain_clip_polygons_for_world_bounds(
        &self,
        graph: &RegionGraph,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut polygons = Vec::new();

        for piece in self.compiled_visual_span_pieces.values() {
            if piece.edge_class != EdgeClass::Standard {
                continue;
            }
            Self::collect_terrain_clip_polygons_from_piece(
                &piece.outer_boundary_loops,
                min_x,
                min_z,
                max_x,
                max_z,
                &mut polygons,
            );
        }

        for (&node_id, piece) in &self.compiled_visual_node_pieces {
            if !self.node_has_standard_surface_edges(graph, node_id) {
                continue;
            }
            Self::collect_terrain_clip_polygons_from_piece(
                &piece.outer_boundary_loops,
                min_x,
                min_z,
                max_x,
                max_z,
                &mut polygons,
            );
        }

        Self::union_terrain_clip_polygons(&polygons)
    }

    pub(crate) fn sample_visible_surface_height(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        world_x: f32,
        world_z: f32,
    ) -> Option<f32> {
        let chunk = self.chunk_coords_for_world(world_x, world_z);
        let (edge_indices, node_ids) = self.collect_query_contributors(chunk, chunk);
        let point = Vector2::new(world_x, world_z);
        let mut best_height_m: Option<f32> = None;

        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            self.visit_visible_node_piece_triangles(
                graph,
                terrain,
                node_id,
                piece,
                &mut |triangle| {
                    let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                    else {
                        return;
                    };
                    let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                    best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
                },
            );
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            self.visit_visible_span_piece_triangles(piece, &mut |triangle| {
                let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                else {
                    return;
                };
                let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
            });
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            if !self.span_piece_uses_visible_earthwork(piece) {
                continue;
            }
            self.visit_span_piece_earthwork_triangles(piece, &mut |triangle| {
                let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                else {
                    return;
                };
                let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
            });
        }

        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
                continue;
            }
            self.visit_node_piece_earthwork_triangles(
                graph,
                terrain,
                node_id,
                piece,
                &mut |triangle| {
                    let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                    else {
                        return;
                    };
                    let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                    best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
                },
            );
        }

        best_height_m
    }

    #[cfg(test)]
    pub(crate) fn sample_paved_support_height(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        world_x: f32,
        world_z: f32,
    ) -> Option<f32> {
        let chunk = self.chunk_coords_for_world(world_x, world_z);
        let (edge_indices, node_ids) = self.collect_query_contributors(chunk, chunk);
        let point = Vector2::new(world_x, world_z);
        let mut best_height_m: Option<f32> = None;

        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_piece_uses_earthworks(graph, node_id, terrain) {
                continue;
            }
            let height_offset_m =
                self.node_piece_integrated_surface_offset_m(graph, node_id, terrain);

            for polygon in piece
                .road_surface_polygons
                .iter()
                .chain(&piece.sidewalk_surface_polygons)
            {
                Self::visit_visual_polygon_triangles(polygon, &mut |triangle| {
                    let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                    else {
                        return;
                    };
                    let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc
                        - height_offset_m;
                    best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
                });
            }
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            let height_offset_m = self.span_piece_integrated_surface_offset_m(piece);
            self.visit_span_piece_clearance_triangles(piece, &mut |triangle| {
                let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                else {
                    return;
                };
                let height_m =
                    triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc - height_offset_m;
                best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
            });
        }

        best_height_m
    }

    pub(crate) fn raycast_visible_surface(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<Vector3> {
        if ray_dir.length_squared() <= f32::EPSILON {
            return None;
        }

        let terrain_hit = terrain.raycast_visual_terrain(ray_origin, ray_dir)?;
        let terrain_t =
            (terrain_hit - ray_origin).dot(ray_dir) / ray_dir.length_squared().max(f32::EPSILON);
        if terrain_t < 0.0 {
            return Some(terrain_hit);
        }

        let min_chunk = self.chunk_coords_for_world(
            ray_origin.x.min(terrain_hit.x),
            ray_origin.z.min(terrain_hit.z),
        );
        let max_chunk = self.chunk_coords_for_world(
            ray_origin.x.max(terrain_hit.x),
            ray_origin.z.max(terrain_hit.z),
        );
        let (edge_indices, node_ids) = self.collect_query_contributors(min_chunk, max_chunk);

        let mut best_t = terrain_t;
        let mut best_hit = None;

        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            self.visit_visible_node_piece_triangles(
                graph,
                terrain,
                node_id,
                piece,
                &mut |triangle| {
                    let Some(t) = Self::ray_triangle_intersection_t(triangle, ray_origin, ray_dir)
                    else {
                        return;
                    };
                    if t >= 0.0 && t <= best_t {
                        best_t = t;
                        best_hit = Some(ray_origin + ray_dir * t);
                    }
                },
            );
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            self.visit_visible_span_piece_triangles(piece, &mut |triangle| {
                let Some(t) = Self::ray_triangle_intersection_t(triangle, ray_origin, ray_dir)
                else {
                    return;
                };
                if t >= 0.0 && t <= best_t {
                    best_t = t;
                    best_hit = Some(ray_origin + ray_dir * t);
                }
            });
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            if !self.span_piece_uses_visible_earthwork(piece) {
                continue;
            }
            self.visit_span_piece_earthwork_triangles(piece, &mut |triangle| {
                let Some(t) = Self::ray_triangle_intersection_t(triangle, ray_origin, ray_dir)
                else {
                    return;
                };
                if t >= 0.0 && t <= best_t {
                    best_t = t;
                    best_hit = Some(ray_origin + ray_dir * t);
                }
            });
        }

        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
                continue;
            }
            self.visit_node_piece_earthwork_triangles(
                graph,
                terrain,
                node_id,
                piece,
                &mut |triangle| {
                    let Some(t) = Self::ray_triangle_intersection_t(triangle, ray_origin, ray_dir)
                    else {
                        return;
                    };
                    if t >= 0.0 && t <= best_t {
                        best_t = t;
                        best_hit = Some(ray_origin + ray_dir * t);
                    }
                },
            );
        }

        best_hit.or(Some(terrain_hit))
    }

    pub(crate) fn build_debug_line_data(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
    ) -> RoadSurfaceDebugData {
        let mut data = RoadSurfaceDebugData::default();

        let mut edge_indices: Vec<usize> = self.compiled_sections.keys().copied().collect();
        edge_indices.retain(|edge_idx| self.compiled_visual_span_pieces.contains_key(edge_idx));
        edge_indices.sort_unstable();
        for edge_idx in edge_indices {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let Some(sections) = self.compiled_sections.get(&edge_idx) else {
                continue;
            };
            for section in sections {
                let profile = self.section_profile_world_points(section, 0.18);
                if let (Some(first), Some(last)) = (profile.first(), profile.last()) {
                    data.section_lines.push(*first);
                    data.section_lines.push(*last);
                }
            }

            for pair in sections.windows(2) {
                let profile_a = self.section_profile_world_points(&pair[0], 0.12);
                let profile_b = self.section_profile_world_points(&pair[1], 0.12);
                if profile_a.len() < 2 || profile_a.len() != profile_b.len() {
                    continue;
                }
                for index in 0..profile_a.len() {
                    data.band_lines.push(profile_a[index]);
                    data.band_lines.push(profile_b[index]);
                }
            }

            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            for boundary_loop in &piece.outer_boundary_loops {
                let points: Vec<Vector3> = boundary_loop
                    .points_world
                    .iter()
                    .map(|point| *point + Vector3::UP * 0.22)
                    .collect();
                if points.len() < 2 {
                    continue;
                }
                for index in 0..points.len() {
                    data.piece_boundary_lines.push(points[index]);
                    data.piece_boundary_lines
                        .push(points[(index + 1) % points.len()]);
                }
            }
        }

        let mut node_ids: Vec<u32> = self.compiled_visual_node_pieces.keys().copied().collect();
        node_ids.sort_unstable();
        for node_id in node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            for boundary_loop in &piece.outer_boundary_loops {
                let points: Vec<Vector3> = boundary_loop
                    .points_world
                    .iter()
                    .map(|point| *point + Vector3::UP * 0.24)
                    .collect();
                if points.len() < 2 {
                    continue;
                }
                for index in 0..points.len() {
                    data.piece_boundary_lines.push(points[index]);
                    data.piece_boundary_lines
                        .push(points[(index + 1) % points.len()]);
                }
            }
        }

        let mut chunks: Vec<SurfaceChunkKey> = self.earthwork_chunk_cache.keys().copied().collect();
        chunks.sort_unstable();
        for chunk in chunks {
            let (min, max) = self.chunk_bounds(chunk);
            let corners = [
                Vector3::new(
                    min.x,
                    terrain.sample_visual_height_world(min.x, min.z) * config::HEIGHT_SCALE + 0.35,
                    min.z,
                ),
                Vector3::new(
                    max.x,
                    terrain.sample_visual_height_world(max.x, min.z) * config::HEIGHT_SCALE + 0.35,
                    min.z,
                ),
                Vector3::new(
                    max.x,
                    terrain.sample_visual_height_world(max.x, max.z) * config::HEIGHT_SCALE + 0.35,
                    max.z,
                ),
                Vector3::new(
                    min.x,
                    terrain.sample_visual_height_world(min.x, max.z) * config::HEIGHT_SCALE + 0.35,
                    max.z,
                ),
            ];
            for index in 0..corners.len() {
                data.earthwork_chunk_lines.push(corners[index]);
                data.earthwork_chunk_lines
                    .push(corners[(index + 1) % corners.len()]);
            }
        }

        data
    }

    pub(crate) fn build_edge_geometry_debug_dump(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_ids: &[usize],
    ) -> String {
        let mut sorted_edge_ids = edge_ids.to_vec();
        sorted_edge_ids.sort_unstable();
        sorted_edge_ids.dedup();

        let mut dump = String::new();
        let _ = writeln!(dump, "ROAD_GEOMETRY_DUMP_BEGIN");
        let _ = writeln!(dump, "{{");
        let _ = writeln!(dump, "  \"edge_ids\": {:?},", sorted_edge_ids);
        let _ = writeln!(dump, "  \"edges\": [");

        let mut first_edge = true;
        for edge_idx in sorted_edge_ids {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted {
                continue;
            }

            if !first_edge {
                let _ = writeln!(dump, ",");
            }
            first_edge = false;
            self.append_edge_geometry_debug_dump(&mut dump, graph, terrain, edge_idx, edge);
        }

        let _ = writeln!(dump);
        let _ = writeln!(dump, "  ]");
        let _ = writeln!(dump, "}}");
        let _ = write!(dump, "ROAD_GEOMETRY_DUMP_END");
        dump
    }

    pub(crate) fn node_uses_visible_surface(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
    ) -> bool {
        if node_id as usize >= graph.node_adjacency_count() {
            return false;
        }

        let mut has_supported_surface = false;
        let mut has_visible_surface_attachment = false;
        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                return false;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted {
                continue;
            }
            if !Self::is_surface_edge(edge) || !self.compiled_sections.contains_key(&edge_idx) {
                return false;
            }

            has_supported_surface = true;
            if edge.primary_type == TransitType::Foot || edge.class != EdgeClass::Tunnel {
                has_visible_surface_attachment = true;
                continue;
            }

            let at_start = graph.get_valid_node(edge.start_node) == node_id;
            if self.tunnel_throat_is_visible(edge_idx, at_start, terrain) {
                has_visible_surface_attachment = true;
            }
        }

        has_supported_surface && has_visible_surface_attachment
    }

    pub(crate) fn span_piece_uses_visible_earthwork(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
    ) -> bool {
        piece.edge_class != EdgeClass::Standard
    }

    pub(crate) fn node_piece_uses_visible_earthwork(
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

            match edge.class {
                EdgeClass::Standard => {}
                EdgeClass::Bridge => return true,
                EdgeClass::Tunnel => {
                    let at_start = graph.get_valid_node(edge.start_node) == node_id;
                    if self.tunnel_throat_is_visible(edge_idx, at_start, terrain) {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn span_piece_integrated_surface_offset_m(&self, piece: &RoadSurfaceVisualSpanPiece) -> f32 {
        if self.span_piece_uses_visible_earthwork(piece) {
            EARTHWORK_PAVEMENT_DEPTH_M
        } else {
            0.0
        }
    }

    fn node_piece_integrated_surface_offset_m(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        terrain: &TerrainSystem,
    ) -> f32 {
        if self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
            EARTHWORK_PAVEMENT_DEPTH_M
        } else {
            0.0
        }
    }

    pub(crate) fn visible_section_ranges_for_edge(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_idx: usize,
        sections: &[RoadSurfaceSection],
    ) -> Vec<(usize, usize)> {
        let Some((start_index, end_index)) =
            self.visible_corridor_index_range_for_edge(graph, edge_idx, sections)
        else {
            return Vec::new();
        };
        if graph.edge(edge_idx).class != EdgeClass::Tunnel {
            return vec![(start_index, end_index)];
        }

        self.tunnel_visible_section_ranges(sections, start_index, end_index, terrain)
    }

    /// Grounds standard-road input to terrain and classifies bridge / tunnel previews using the
    /// same threshold as committed placement.
    pub(crate) fn classify_and_ground_road_points(
        raw_points: &[Vector3],
        terrain: &TerrainSystem,
    ) -> (Vec<Vector3>, EdgeClass) {
        let mut fixed_points = raw_points.to_vec();
        let mut all_points_above_clearance = !fixed_points.is_empty();
        let mut all_points_below_clearance = !fixed_points.is_empty();

        for point in &fixed_points {
            let terrain_h = terrain.sample_height_world(point.x, point.z) * config::HEIGHT_SCALE;
            let clearance_m = point.y - terrain_h;
            if clearance_m <= PREVIEW_CLEARANCE_M {
                all_points_above_clearance = false;
            }
            if clearance_m >= -PREVIEW_CLEARANCE_M {
                all_points_below_clearance = false;
            }
        }

        let class = if all_points_above_clearance {
            EdgeClass::Bridge
        } else if all_points_below_clearance {
            EdgeClass::Tunnel
        } else {
            EdgeClass::Standard
        };

        if class == EdgeClass::Standard {
            for point in &mut fixed_points {
                point.y = terrain.sample_height_world(point.x, point.z) * config::HEIGHT_SCALE;
            }
        }

        (fixed_points, class)
    }

    /// Applies the same point simplification threshold used by committed road placement.
    pub(crate) fn simplify_road_input_points(points: &[Vector3]) -> Vec<Vector3> {
        let mut simplified_points = Vec::with_capacity(points.len());
        if !points.is_empty() {
            simplified_points.push(points[0]);
            for point in points.iter().skip(1) {
                if point.distance_to(*simplified_points.last().unwrap())
                    > ROAD_POINT_SIMPLIFY_DISTANCE_M
                {
                    simplified_points.push(*point);
                }
            }
            if simplified_points.len() > 1
                && simplified_points.last().unwrap() != points.last().unwrap()
            {
                simplified_points.pop();
                simplified_points.push(*points.last().unwrap());
            }
        }
        simplified_points
    }

    /// Applies the Taubin height-smoothing pass shared by committed placement and preview.
    pub(crate) fn taubin_smooth_road_heights(points: &mut [Vector3]) {
        if points.len() <= 2 {
            return;
        }

        let mut temp_h = vec![0.0; points.len()];
        for _ in 0..TAUBIN_SMOOTHING_ITERS {
            for index in 1..points.len() - 1 {
                let laplacian = 0.5 * (points[index - 1].y + points[index + 1].y) - points[index].y;
                temp_h[index] = points[index].y + TAUBIN_LAMBDA * laplacian;
            }
            for index in 1..points.len() - 1 {
                points[index].y = temp_h[index];
            }
            for index in 1..points.len() - 1 {
                let laplacian = 0.5 * (points[index - 1].y + points[index + 1].y) - points[index].y;
                temp_h[index] = points[index].y + TAUBIN_MU * laplacian;
            }
            for index in 1..points.len() - 1 {
                points[index].y = temp_h[index];
            }
        }
    }

    /// Compiles one temporary road preview using the same point conditioning and section compiler
    /// as committed placement while keeping preview cache lifetime transient.
    pub fn compile_preview_surface(
        &self,
        raw_points: &[Vector3],
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
    ) -> PreviewRoadSurfaceResult {
        let (conditioned_points, edge_class) =
            Self::classify_and_ground_road_points(raw_points, terrain);
        let mut prepared_points = Self::simplify_road_input_points(&conditioned_points);
        Self::taubin_smooth_road_heights(&mut prepared_points);

        if prepared_points.len() < 2 {
            return PreviewRoadSurfaceResult {
                edge_class,
                prepared_points,
                compiled_sections: Vec::new(),
                compiled_visual_node_pieces: Vec::new(),
                surface_vertices: Vec::new(),
                is_valid: true,
            };
        }

        let mut graph = RegionGraph::new();
        let start_node = graph.add_node(prepared_points[0], NodeType::Junction);
        let end_node = graph.add_node(*prepared_points.last().unwrap(), NodeType::Junction);
        let edge_idx = graph.add_edge(Self::build_preview_edge(
            start_node,
            end_node,
            prepared_points.clone(),
            fwd_lanes,
            bkw_lanes,
            edge_class,
        ));

        let mut preview_surface = RoadSurfaceSystem::new(self.chunk_span_m);
        preview_surface.compile_dirty(&graph, terrain);

        let compiled_sections = preview_surface
            .compiled_sections()
            .get(&edge_idx)
            .cloned()
            .unwrap_or_default();
        let compiled_visual_node_pieces = [start_node, end_node]
            .into_iter()
            .filter_map(|node_id| {
                preview_surface
                    .compiled_visual_node_pieces()
                    .get(&node_id)
                    .cloned()
            })
            .collect();
        let surface_vertices = self.build_preview_surface_vertices(&compiled_sections);
        let is_valid = Self::preview_surface_is_valid(
            edge_class,
            &prepared_points,
            &compiled_sections,
            terrain,
        );

        PreviewRoadSurfaceResult {
            edge_class,
            prepared_points,
            compiled_sections,
            compiled_visual_node_pieces,
            surface_vertices,
            is_valid,
        }
    }

    /// Clears compiled caches and dirty tracking without changing the configured chunk span.
    pub fn clear(&mut self) {
        self.clear_dirty_tracking();
        self.compiled_sections.clear();
        self.compiled_visual_span_pieces.clear();
        self.compiled_visual_node_pieces.clear();
        self.clear_piece_chunk_coverage();
        self.surface_chunk_cache.clear();
        self.earthwork_chunk_cache.clear();
        self.last_rebuilt_surface_chunks.clear();
        self.last_rebuilt_terrain_chunks.clear();
        self.compiled_once = false;
    }

    /// Clears only the dirty tracking sets.
    pub fn clear_dirty_tracking(&mut self) {
        self.dirty_edges.clear();
        self.dirty_nodes.clear();
        self.dirty_surface_chunks.clear();
        self.dirty_terrain_chunks.clear();
    }

    /// Reconfigures the chunk span and clears all caches and dirty sets.
    pub fn set_chunk_span_m(&mut self, chunk_span_m: f32) {
        self.chunk_span_m = chunk_span_m.max(f32::EPSILON);
        self.clear();
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
        for &edge_idx in &sorted_span_edges {
            if edge_idx >= graph.edge_count() {
                self.compiled_sections.remove(&edge_idx);
                continue;
            }
            let edge = graph.edge(edge_idx);
            if !Self::is_surface_edge(edge) {
                self.compiled_sections.remove(&edge_idx);
                continue;
            }
            self.compiled_sections.insert(
                edge_idx,
                self.compile_edge_sections(graph, terrain, edge_idx),
            );
        }
        for edge_idx in sorted_span_edges {
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
            if let Some(span_piece) = self.compile_visual_span_piece(graph, terrain, edge_idx) {
                self.insert_span_piece_coverage(&span_piece);
                self.compiled_visual_span_pieces
                    .insert(edge_idx, span_piece);
            } else {
                self.compiled_visual_span_pieces.remove(&edge_idx);
            }
        }

        for &node_id in &sorted_nodes {
            self.remove_node_piece_coverage(node_id);
            if self.node_has_surface_edges(graph, node_id) {
                let visual_piece = self.compile_visual_node_piece(graph, terrain, node_id);
                if let Some(visual_piece) = visual_piece {
                    self.insert_node_piece_coverage(&visual_piece);
                    self.compiled_visual_node_pieces
                        .insert(node_id, visual_piece);
                } else {
                    self.compiled_visual_node_pieces.remove(&node_id);
                }
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

    /// Marks one world-space point as dirty for both surface and terrain chunk rebuilds.
    pub fn mark_world_point_dirty(&mut self, pos: Vector3) {
        let chunk = self.chunk_coords_for_world(pos.x, pos.z);
        self.dirty_surface_chunks.insert(chunk);
        self.dirty_terrain_chunks.insert(chunk);
    }

    /// Marks one world-space AABB as dirty for both surface and terrain chunk rebuilds.
    pub fn mark_world_aabb_dirty(&mut self, min: Vector3, max: Vector3) {
        let min_chunk = self.chunk_coords_for_world(min.x.min(max.x), min.z.min(max.z));
        let max_chunk = self.chunk_coords_for_world(min.x.max(max.x), min.z.max(max.z));
        for cx in min_chunk.0..=max_chunk.0 {
            for cz in min_chunk.1..=max_chunk.1 {
                let chunk = (cx, cz);
                self.dirty_surface_chunks.insert(chunk);
                self.dirty_terrain_chunks.insert(chunk);
            }
        }
    }

    /// Marks one edge dirty; chunk invalidation is derived from compiled piece coverage.
    pub fn mark_edge_dirty(&mut self, graph: &RegionGraph, edge_idx: usize) {
        if edge_idx >= graph.edge_count() {
            return;
        }
        self.dirty_edges.insert(edge_idx);
    }

    /// Marks one node dirty; chunk invalidation is derived from compiled piece coverage.
    pub fn mark_node_dirty(&mut self, graph: &RegionGraph, node_id: u32) {
        if node_id as usize >= graph.node_count() {
            return;
        }
        let valid = graph.get_valid_node(node_id);
        self.dirty_nodes.insert(valid);
    }

    fn clear_piece_chunk_coverage(&mut self) {
        self.surface_span_chunks.clear();
        self.surface_node_chunks.clear();
        self.earthwork_span_chunks.clear();
        self.earthwork_node_chunks.clear();
        self.surface_chunk_spans.clear();
        self.surface_chunk_nodes.clear();
        self.earthwork_chunk_spans.clear();
        self.earthwork_chunk_nodes.clear();
    }

    fn remove_span_piece_coverage(
        &mut self,
        edge_idx: usize,
    ) -> (Vec<SurfaceChunkKey>, Vec<SurfaceChunkKey>) {
        let surface_chunks = self
            .surface_span_chunks
            .remove(&edge_idx)
            .unwrap_or_default();
        let terrain_chunks = self
            .earthwork_span_chunks
            .remove(&edge_idx)
            .unwrap_or_default();
        Self::remove_owner_chunk_coverage(&mut self.surface_chunk_spans, edge_idx, &surface_chunks);
        Self::remove_owner_chunk_coverage(
            &mut self.earthwork_chunk_spans,
            edge_idx,
            &terrain_chunks,
        );
        self.extend_dirty_piece_chunks(&surface_chunks, &terrain_chunks);
        (surface_chunks, terrain_chunks)
    }

    fn remove_node_piece_coverage(
        &mut self,
        node_id: u32,
    ) -> (Vec<SurfaceChunkKey>, Vec<SurfaceChunkKey>) {
        let surface_chunks = self
            .surface_node_chunks
            .remove(&node_id)
            .unwrap_or_default();
        let terrain_chunks = self
            .earthwork_node_chunks
            .remove(&node_id)
            .unwrap_or_default();
        Self::remove_owner_chunk_coverage(&mut self.surface_chunk_nodes, node_id, &surface_chunks);
        Self::remove_owner_chunk_coverage(
            &mut self.earthwork_chunk_nodes,
            node_id,
            &terrain_chunks,
        );
        self.extend_dirty_piece_chunks(&surface_chunks, &terrain_chunks);
        (surface_chunks, terrain_chunks)
    }

    fn insert_span_piece_coverage(
        &mut self,
        piece: &RoadSurfaceVisualSpanPiece,
    ) -> (Vec<SurfaceChunkKey>, Vec<SurfaceChunkKey>) {
        let surface_chunks = Self::canonical_chunk_vec(
            self.visual_span_piece_chunks(piece, ChunkCacheKind::Surface),
        );
        let terrain_chunks = Self::canonical_chunk_vec(
            self.visual_span_piece_chunks(piece, ChunkCacheKind::Earthwork),
        );
        self.surface_span_chunks
            .insert(piece.edge_idx, surface_chunks.clone());
        self.earthwork_span_chunks
            .insert(piece.edge_idx, terrain_chunks.clone());
        Self::insert_owner_chunk_coverage(
            &mut self.surface_chunk_spans,
            piece.edge_idx,
            &surface_chunks,
        );
        Self::insert_owner_chunk_coverage(
            &mut self.earthwork_chunk_spans,
            piece.edge_idx,
            &terrain_chunks,
        );
        self.extend_dirty_piece_chunks(&surface_chunks, &terrain_chunks);
        (surface_chunks, terrain_chunks)
    }

    fn insert_node_piece_coverage(
        &mut self,
        piece: &RoadSurfaceVisualNodePiece,
    ) -> (Vec<SurfaceChunkKey>, Vec<SurfaceChunkKey>) {
        let surface_chunks = Self::canonical_chunk_vec(
            self.visual_node_piece_chunks(piece, ChunkCacheKind::Surface),
        );
        let terrain_chunks = Self::canonical_chunk_vec(
            self.visual_node_piece_chunks(piece, ChunkCacheKind::Earthwork),
        );
        self.surface_node_chunks
            .insert(piece.node_id, surface_chunks.clone());
        self.earthwork_node_chunks
            .insert(piece.node_id, terrain_chunks.clone());
        Self::insert_owner_chunk_coverage(
            &mut self.surface_chunk_nodes,
            piece.node_id,
            &surface_chunks,
        );
        Self::insert_owner_chunk_coverage(
            &mut self.earthwork_chunk_nodes,
            piece.node_id,
            &terrain_chunks,
        );
        self.extend_dirty_piece_chunks(&surface_chunks, &terrain_chunks);
        (surface_chunks, terrain_chunks)
    }

    fn extend_dirty_piece_chunks(
        &mut self,
        surface_chunks: &[SurfaceChunkKey],
        terrain_chunks: &[SurfaceChunkKey],
    ) {
        self.dirty_surface_chunks
            .extend(surface_chunks.iter().copied());
        self.dirty_terrain_chunks
            .extend(terrain_chunks.iter().copied());
    }

    fn insert_owner_chunk_coverage<T: Copy + Ord>(
        chunk_owners: &mut HashMap<SurfaceChunkKey, BTreeSet<T>>,
        owner: T,
        chunks: &[SurfaceChunkKey],
    ) {
        for &chunk in chunks {
            chunk_owners.entry(chunk).or_default().insert(owner);
        }
    }

    fn remove_owner_chunk_coverage<T: Copy + Ord>(
        chunk_owners: &mut HashMap<SurfaceChunkKey, BTreeSet<T>>,
        owner: T,
        chunks: &[SurfaceChunkKey],
    ) {
        for &chunk in chunks {
            let Some(owners) = chunk_owners.get_mut(&chunk) else {
                continue;
            };
            owners.remove(&owner);
            let remove_chunk = owners.is_empty();
            if remove_chunk {
                chunk_owners.remove(&chunk);
            }
        }
    }

    fn visual_span_piece_chunks(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        kind: ChunkCacheKind,
    ) -> Vec<SurfaceChunkKey> {
        self.visual_span_piece_bounds(piece, kind)
            .map(|(min, max)| self.bounds_to_chunk_keys(min, max))
            .unwrap_or_default()
    }

    fn visual_node_piece_chunks(
        &self,
        piece: &RoadSurfaceVisualNodePiece,
        kind: ChunkCacheKind,
    ) -> Vec<SurfaceChunkKey> {
        self.visual_node_piece_bounds(piece, kind)
            .map(|(min, max)| self.bounds_to_chunk_keys(min, max))
            .unwrap_or_default()
    }

    /// Marks a terrain edit dirty using the brush center and radius in world metres.
    ///
    /// This marks both touched terrain chunks and any nearby road edges / nodes whose compiled
    /// roadbed may need recompilation when terrain-dependent grades are rebuilt.
    pub fn mark_terrain_edit_dirty(&mut self, graph: &RegionGraph, center: Vector2, radius_m: f32) {
        let radius_m = radius_m.max(0.0);
        let min = Vector3::new(center.x - radius_m, 0.0, center.y - radius_m);
        let max = Vector3::new(center.x + radius_m, 0.0, center.y + radius_m);
        self.mark_world_aabb_dirty(min, max);

        for edge_idx in graph.get_edges_near_aabb(min, max) {
            self.mark_edge_dirty(graph, edge_idx);
            let edge = graph.edge(edge_idx);
            self.mark_node_dirty(graph, edge.start_node);
            self.mark_node_dirty(graph, edge.end_node);
        }
    }

    fn compile_all(&mut self, graph: &RegionGraph, terrain: &TerrainSystem) {
        self.prune_stale_cache_entries(graph);
        self.clear_piece_chunk_coverage();
        self.surface_chunk_cache.clear();
        self.earthwork_chunk_cache.clear();

        let edge_ids = self.all_surface_edge_ids(graph);
        for &edge_idx in &edge_ids {
            self.compiled_sections.insert(
                edge_idx,
                self.compile_edge_sections(graph, terrain, edge_idx),
            );
        }

        for &edge_idx in &edge_ids {
            if let Some(span_piece) = self.compile_visual_span_piece(graph, terrain, edge_idx) {
                self.insert_span_piece_coverage(&span_piece);
                self.compiled_visual_span_pieces
                    .insert(edge_idx, span_piece);
            } else {
                self.compiled_visual_span_pieces.remove(&edge_idx);
            }
        }

        let node_ids = self.all_surface_node_ids(graph);
        for node_id in node_ids {
            let visual_piece = self.compile_visual_node_piece(graph, terrain, node_id);
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

    fn compile_edge_sections(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_idx: usize,
    ) -> Vec<RoadSurfaceSection> {
        let edge = graph.edge(edge_idx);
        let points = self.edge_points(edge);
        if points.is_empty() {
            return Vec::new();
        }
        if points.len() == 1 {
            let center = points[0];
            let center_height_m = self.solve_section_height(center);
            let tangent_xz = Vector2::RIGHT;
            let lateral_xz = Vector2::new(-tangent_xz.y, tangent_xz.x);
            return vec![RoadSurfaceSection {
                edge_idx,
                s_m: 0.0,
                center_xz: Vector2::new(center.x, center.z),
                center_height_m,
                tangent_xz,
                lateral_xz,
                bands: self.build_lateral_bands(
                    edge,
                    terrain,
                    Vector2::new(center.x, center.z),
                    lateral_xz,
                    center_height_m,
                ),
            }];
        }

        let cumulative = self.build_cumulative_distances(points);
        let sample_distances = self.build_section_sample_distances(edge, &cumulative);
        sample_distances
            .into_iter()
            .map(|s_m| {
                let (center, tangent_xz) = self.sample_polyline(points, &cumulative, s_m);
                let center_height_m = self.solve_section_height(center);
                let lateral_xz = Vector2::new(-tangent_xz.y, tangent_xz.x).normalized();
                RoadSurfaceSection {
                    edge_idx,
                    s_m,
                    center_xz: Vector2::new(center.x, center.z),
                    center_height_m,
                    tangent_xz,
                    lateral_xz,
                    bands: self.build_lateral_bands(
                        edge,
                        terrain,
                        Vector2::new(center.x, center.z),
                        lateral_xz,
                        center_height_m,
                    ),
                }
            })
            .collect()
    }

    fn build_preview_edge(
        start_node: u32,
        end_node: u32,
        points: Vec<Vector3>,
        fwd_lanes: u8,
        bkw_lanes: u8,
        class: EdgeClass,
    ) -> Edge {
        let is_walkway = fwd_lanes == 0 && bkw_lanes == 0;
        let mut allowed_types = TransitFlags::NONE;
        if fwd_lanes > 0 || bkw_lanes > 0 {
            allowed_types |= TransitFlags::CAR;
        }
        if is_walkway || fwd_lanes > 0 || bkw_lanes > 0 {
            allowed_types |= TransitFlags::FOOT;
        }
        let vehicle_frontage_access = if is_walkway {
            VehicleFrontageAccess::SameSideOnly
        } else {
            VehicleFrontageAccess::BothSides
        };
        let physical_length = points
            .windows(2)
            .map(|segment| segment[0].distance_to(segment[1]))
            .sum();

        Edge {
            start_node,
            end_node,
            primary_type: if is_walkway {
                TransitType::Foot
            } else {
                TransitType::Road
            },
            allowed_types,
            class,
            width: ((fwd_lanes + bkw_lanes) as f32 * config::LANE_WIDTH).max(2.0),
            fwd_lanes,
            bkw_lanes,
            speed_limit: 50.0,
            base_cost: 0.0,
            physical_length,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: points.clone(),
            physical_geometry: points,
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access,
        }
    }

    fn compile_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        let valid = graph.get_valid_node(node_id);
        let incidents = self.sorted_incident_surface_edges(graph, valid);
        match self.classify_visual_node_kind(&incidents) {
            CompiledNodeKind::Terminal => incidents.first().and_then(|incident| {
                self.build_terminal_visual_node_piece(graph, terrain, valid, *incident)
            }),
            CompiledNodeKind::PassThrough => None,
            CompiledNodeKind::Bend => {
                self.build_bend_visual_node_piece(graph, terrain, valid, &incidents)
            }
            CompiledNodeKind::JunctionN => {
                self.build_junction_visual_node_piece(graph, terrain, valid, &incidents)
            }
        }
    }

    fn compile_visual_span_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_idx: usize,
    ) -> Option<RoadSurfaceVisualSpanPiece> {
        if edge_idx >= graph.edge_count() {
            return None;
        }
        let edge = graph.edge(edge_idx);
        let sections = self.compiled_sections.get(&edge_idx)?;
        let visible_ranges =
            self.visible_section_ranges_for_edge(graph, terrain, edge_idx, sections);
        let (mut road_surface_polygons, mut sidewalk_surface_polygons) =
            self.compile_surface_polygons_for_ranges(sections, &visible_ranges);

        if road_surface_polygons.is_empty() && sidewalk_surface_polygons.is_empty() {
            return None;
        }

        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        let outer_boundary_loops = Self::build_span_outer_boundary_loops(sections, &visible_ranges);
        if outer_boundary_loops.is_empty() {
            return None;
        }

        let earthwork_ranges = self.earthwork_section_ranges_for_edge(edge, sections, terrain);
        let (mut clearance_road_surface_polygons, mut clearance_sidewalk_surface_polygons) =
            self.compile_surface_polygons_for_ranges(sections, &earthwork_ranges);
        Self::sort_visual_polygons(&mut clearance_road_surface_polygons);
        Self::sort_visual_polygons(&mut clearance_sidewalk_surface_polygons);
        let earthwork_boundary_loops =
            Self::build_span_outer_boundary_loops(sections, &earthwork_ranges);
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_loops(
                &earthwork_boundary_loops,
                terrain,
            );

        let start_mouth_profile =
            Self::section_range_mouth_profile(sections, &visible_ranges, IncidentEdgeSide::Start);
        let end_mouth_profile =
            Self::section_range_mouth_profile(sections, &visible_ranges, IncidentEdgeSide::End);

        Some(RoadSurfaceVisualSpanPiece {
            edge_idx,
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
            edge_class: edge.class,
            start_mouth_profile,
            end_mouth_profile,
            clearance_road_surface_polygons,
            clearance_sidewalk_surface_polygons,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        })
    }

    fn compile_surface_polygons_for_ranges(
        &self,
        sections: &[RoadSurfaceSection],
        ranges: &[(usize, usize)],
    ) -> (Vec<RoadSurfaceVisualPolygon>, Vec<RoadSurfaceVisualPolygon>) {
        let mut road_surface_polygons = Vec::new();
        let mut sidewalk_surface_polygons = Vec::new();

        for &(start_index, end_index) in ranges {
            if end_index <= start_index {
                continue;
            }
            for pair in sections[start_index..=end_index].windows(2) {
                if pair[0].bands.len() != pair[1].bands.len() {
                    continue;
                }
                for (band_a, band_b) in pair[0].bands.iter().zip(&pair[1].bands) {
                    let width_a = (band_a.lateral_end_m - band_a.lateral_start_m).abs();
                    let width_b = (band_b.lateral_end_m - band_b.lateral_start_m).abs();
                    if width_a <= BAND_WIDTH_MATCH_EPSILON_M
                        && width_b <= BAND_WIDTH_MATCH_EPSILON_M
                    {
                        continue;
                    }

                    let Some(polygon) = Self::make_visual_strip_polygon(vec![
                        self.section_boundary_world_point(
                            &pair[0],
                            band_a.lateral_start_m,
                            band_a.height_start_m,
                        ),
                        self.section_boundary_world_point(
                            &pair[1],
                            band_b.lateral_start_m,
                            band_b.height_start_m,
                        ),
                        self.section_boundary_world_point(
                            &pair[1],
                            band_b.lateral_end_m,
                            band_b.height_end_m,
                        ),
                        self.section_boundary_world_point(
                            &pair[0],
                            band_a.lateral_end_m,
                            band_a.height_end_m,
                        ),
                    ]) else {
                        continue;
                    };

                    if band_a.kind == RoadSurfaceBandKind::Carriageway
                        && band_b.kind == RoadSurfaceBandKind::Carriageway
                    {
                        road_surface_polygons.push(polygon);
                    } else {
                        sidewalk_surface_polygons.push(polygon);
                    }
                }
            }
        }

        (road_surface_polygons, sidewalk_surface_polygons)
    }

    fn build_span_outer_boundary_loops(
        sections: &[RoadSurfaceSection],
        ranges: &[(usize, usize)],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut loops = Vec::new();
        for &(start_index, end_index) in ranges {
            if end_index <= start_index {
                continue;
            }
            let mut left_points = Vec::new();
            let mut right_points = Vec::new();
            for section in &sections[start_index..=end_index] {
                let Some((left_point, right_point)) = Self::section_outer_boundary_pair(section)
                else {
                    continue;
                };
                left_points.push(left_point);
                right_points.push(right_point);
            }
            if left_points.len() < 2 || right_points.len() < 2 {
                continue;
            }
            right_points.reverse();
            let mut loop_points = left_points;
            loop_points.extend(right_points);
            if let Some(loop_polygon) = Self::make_visual_polygon(loop_points) {
                loops.push(loop_polygon);
            }
        }
        Self::sort_visual_polygons(&mut loops);
        loops
    }

    fn section_range_mouth_profile(
        sections: &[RoadSurfaceSection],
        ranges: &[(usize, usize)],
        side: IncidentEdgeSide,
    ) -> Option<IncidentMouthProfile> {
        let section = match side {
            IncidentEdgeSide::Start => {
                let &(start_index, _) = ranges.first()?;
                sections.get(start_index)?
            }
            IncidentEdgeSide::End => {
                let &(_, end_index) = ranges.last()?;
                sections.get(end_index)?
            }
        };
        Self::build_mouth_profile_from_section(section, side)
    }

    fn build_mouth_profile_from_section(
        section: &RoadSurfaceSection,
        side: IncidentEdgeSide,
    ) -> Option<IncidentMouthProfile> {
        let mut boundary_points_world = Vec::with_capacity(section.bands.len() + 1);
        let mut bands = Vec::with_capacity(section.bands.len());

        if side == IncidentEdgeSide::Start {
            for band in &section.bands {
                let start_point_world = Self::section_boundary_world_point_static(
                    section,
                    band.lateral_start_m,
                    band.height_start_m,
                );
                let end_point_world = Self::section_boundary_world_point_static(
                    section,
                    band.lateral_end_m,
                    band.height_end_m,
                );
                if boundary_points_world.is_empty() {
                    boundary_points_world.push(start_point_world);
                }
                boundary_points_world.push(end_point_world);
                bands.push(IncidentMouthBand {
                    kind: band.kind,
                    start_point_world,
                    end_point_world,
                });
            }
        } else {
            for band in section.bands.iter().rev() {
                let start_point_world = Self::section_boundary_world_point_static(
                    section,
                    band.lateral_end_m,
                    band.height_end_m,
                );
                let end_point_world = Self::section_boundary_world_point_static(
                    section,
                    band.lateral_start_m,
                    band.height_start_m,
                );
                if boundary_points_world.is_empty() {
                    boundary_points_world.push(start_point_world);
                }
                boundary_points_world.push(end_point_world);
                bands.push(IncidentMouthBand {
                    kind: band.kind,
                    start_point_world,
                    end_point_world,
                });
            }
        }

        Some(IncidentMouthProfile {
            inward_direction_xz: match side {
                IncidentEdgeSide::Start => section.tangent_xz,
                IncidentEdgeSide::End => -section.tangent_xz,
            },
            boundary_points_world,
            bands,
        })
    }

    fn build_terminal_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        incident: IncidentSurfaceEdge,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        let node_pos = graph.node(node_id).pos;
        let mouths = self.build_ordered_piece_mouths(&[incident])?;
        let node_candidates = Self::build_node_corridor_candidates(node_pos, &mouths)?;
        let node_regions = Self::resolve_node_surface_regions_with_overlay(
            &node_candidates.road_candidate_polygons,
            &node_candidates.non_road_candidate_polygons,
            &node_candidates.non_road_height_candidate_polygons,
        )?;
        let outer_boundary_loops = node_regions.outer_boundary_loops;
        let road_surface_polygons = node_regions.road_surface_polygons;
        let sidewalk_surface_polygons = node_regions.sidewalk_surface_polygons;
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_loops(
                &outer_boundary_loops,
                terrain,
            );

        self.assemble_explicit_node_piece(
            node_id,
            RoadSurfaceVisualNodePieceKind::Terminal,
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        )
    }

    fn build_bend_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if incidents.len() != 2 {
            return None;
        }
        let node_pos = graph.node(node_id).pos;
        let mouths = self.build_ordered_piece_mouths(incidents)?;
        let node_candidates = Self::build_node_corridor_candidates(node_pos, &mouths)?;
        let node_regions = Self::resolve_node_surface_regions_with_overlay(
            &node_candidates.road_candidate_polygons,
            &node_candidates.non_road_candidate_polygons,
            &node_candidates.non_road_height_candidate_polygons,
        )?;
        let outer_boundary_loops = node_regions.outer_boundary_loops;
        let road_surface_polygons = node_regions.road_surface_polygons;
        let sidewalk_surface_polygons = node_regions.sidewalk_surface_polygons;
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_loops(
                &outer_boundary_loops,
                terrain,
            );

        self.assemble_explicit_node_piece(
            node_id,
            RoadSurfaceVisualNodePieceKind::Bend,
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        )
    }

    fn build_junction_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if incidents.len() < 3 {
            return None;
        }
        let node_pos = graph.node(node_id).pos;
        let mouths = self.build_ordered_piece_mouths(incidents)?;
        let node_candidates = Self::build_node_corridor_candidates(node_pos, &mouths)?;
        let node_regions = Self::resolve_node_surface_regions_with_overlay(
            &node_candidates.road_candidate_polygons,
            &node_candidates.non_road_candidate_polygons,
            &node_candidates.non_road_height_candidate_polygons,
        )?;
        let outer_boundary_loops = node_regions.outer_boundary_loops;
        let road_surface_polygons = node_regions.road_surface_polygons;
        let sidewalk_surface_polygons = node_regions.sidewalk_surface_polygons;
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_loops(
                &outer_boundary_loops,
                terrain,
            );

        self.assemble_explicit_node_piece(
            node_id,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        )
    }

    fn build_ordered_piece_mouths(
        &self,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<Vec<OrderedIncidentPieceMouth>> {
        let mut mouths = Vec::with_capacity(incidents.len());
        for &incident in incidents {
            mouths.push(OrderedIncidentPieceMouth {
                profile: self.build_incident_mouth_profile(incident)?,
                direction_angle_ccw: Self::normalized_angle_ccw(incident.direction_xz),
                direction_xz: incident.direction_xz,
                edge_idx: incident.edge_idx,
                side: incident.side,
            });
        }
        mouths.sort_by(|a, b| {
            a.direction_angle_ccw
                .total_cmp(&b.direction_angle_ccw)
                .then(a.edge_idx.cmp(&b.edge_idx))
                .then(a.side.cmp(&b.side))
        });
        Some(mouths)
    }

    fn resolve_node_surface_regions_with_overlay(
        road_candidates: &[RoadSurfaceVisualPolygon],
        non_road_candidates: &[NodeNonRoadCandidatePolygon],
        non_road_height_candidates: &[RoadSurfaceVisualPolygon],
    ) -> Option<NodeSurfaceRegionResult> {
        let mut height_candidates = Vec::with_capacity(
            road_candidates
                .len()
                .saturating_add(non_road_candidates.len()),
        );
        height_candidates.extend(road_candidates.iter().cloned());
        height_candidates.extend(
            non_road_candidates
                .iter()
                .map(|candidate| candidate.polygon.clone()),
        );

        let road_contours = Self::overlay_contours_from_polygons(road_candidates);
        let mut road_shapes = Self::overlay_union_contours(&road_contours)?;

        let footprint_contours = Self::overlay_contours_from_polygons(&height_candidates);
        let mut footprint_shapes = Self::overlay_union_contours(&footprint_contours)?;

        let mut non_road_shapes = if road_shapes.is_empty() {
            footprint_shapes.clone()
        } else if footprint_shapes.is_empty() {
            Vec::new()
        } else {
            Self::overlay_binary_shapes(&footprint_shapes, &road_shapes, OverlayRule::Difference)?
        };
        let non_road_guide_points =
            Self::overlay_points_from_polygons(non_road_height_candidates);
        Self::insert_overlay_guide_points_into_shapes(
            &mut non_road_shapes,
            &non_road_guide_points,
        );

        Self::sort_overlay_shapes(&mut road_shapes);
        Self::sort_overlay_shapes(&mut non_road_shapes);
        Self::sort_overlay_shapes(&mut footprint_shapes);

        let mut road_surface_polygons =
            Self::visual_polygons_from_overlay_shapes(&road_shapes, road_candidates);
        let mut sidewalk_surface_polygons = Self::visual_polygons_from_overlay_shapes(
            &non_road_shapes,
            non_road_height_candidates,
        );
        let mut outer_boundary_loops = Self::outer_boundary_polygons_from_overlay_shapes(
            &footprint_shapes,
            &height_candidates,
        );

        if road_surface_polygons.is_empty() && sidewalk_surface_polygons.is_empty() {
            return None;
        }
        if outer_boundary_loops.is_empty() {
            return None;
        }

        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_visual_polygons(&mut outer_boundary_loops);
        Some(NodeSurfaceRegionResult {
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
        })
    }

    fn overlay_contours_from_polygons(
        polygons: &[RoadSurfaceVisualPolygon],
    ) -> Vec<NodeOverlayContour> {
        let mut contours = Vec::new();
        for polygon in polygons {
            let contour = Self::overlay_contour_from_world_points(&polygon.points_world);
            if Self::overlay_contour_area(&contour).abs() > NODE_OVERLAY_MIN_AREA_M2 {
                contours.push(contour);
            }
        }
        contours
    }

    fn overlay_points_from_polygons(
        polygons: &[RoadSurfaceVisualPolygon],
    ) -> Vec<NodeOverlayPoint> {
        let mut points = polygons
            .iter()
            .flat_map(|polygon| polygon.points_world.iter())
            .map(|point| Self::overlay_point_from_world_point(*point))
            .collect::<Vec<_>>();
        points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
        points.dedup();
        points
    }

    fn overlay_contour_from_world_points(points_world: &[Vector3]) -> NodeOverlayContour {
        let mut contour = Vec::with_capacity(points_world.len());
        for point in points_world {
            let overlay_point = Self::overlay_point_from_world_point(*point);
            if contour
                .last()
                .is_none_or(|last: &NodeOverlayPoint| *last != overlay_point)
            {
                contour.push(overlay_point);
            }
        }
        if contour.len() >= 2 && contour.first() == contour.last() {
            contour.pop();
        }
        contour
    }

    fn overlay_point_from_world_point(point: Vector3) -> NodeOverlayPoint {
        [
            (point.x * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
            (point.z * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
        ]
    }

    fn insert_overlay_guide_points_into_shapes(
        shapes: &mut NodeOverlayShapes,
        guide_points: &[NodeOverlayPoint],
    ) {
        if guide_points.is_empty() {
            return;
        }
        for shape in shapes {
            for contour in shape {
                *contour = Self::insert_overlay_guide_points_into_contour(contour, guide_points);
            }
        }
    }

    fn insert_overlay_guide_points_into_contour(
        contour: &[NodeOverlayPoint],
        guide_points: &[NodeOverlayPoint],
    ) -> NodeOverlayContour {
        if contour.len() < 2 {
            return contour.to_vec();
        }

        let mut enriched = Vec::with_capacity(contour.len());
        let tolerance_m = 2.0 / NODE_OVERLAY_SCALE;
        for index in 0..contour.len() {
            let current = contour[index];
            let next = contour[(index + 1) % contour.len()];
            enriched.push(current);

            let segment_x = next[0] - current[0];
            let segment_z = next[1] - current[1];
            let length_squared = segment_x * segment_x + segment_z * segment_z;
            if length_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
                continue;
            }

            let mut edge_points = Vec::new();
            for point in guide_points {
                if *point == current || *point == next {
                    continue;
                }
                let rel_x = point[0] - current[0];
                let rel_z = point[1] - current[1];
                let t = (rel_x * segment_x + rel_z * segment_z) / length_squared;
                if t <= 0.0 || t >= 1.0 {
                    continue;
                }
                let cross = rel_x * segment_z - rel_z * segment_x;
                let distance = cross.abs() / length_squared.sqrt();
                if distance <= tolerance_m {
                    edge_points.push((t, *point));
                }
            }
            edge_points.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (_, point) in edge_points {
                if enriched.last().is_none_or(|last| *last != point) {
                    enriched.push(point);
                }
            }
        }
        if enriched.len() >= 2 && enriched.first() == enriched.last() {
            enriched.pop();
        }
        enriched
    }

    fn overlay_union_contours(contours: &[NodeOverlayContour]) -> Option<NodeOverlayShapes> {
        if contours.is_empty() {
            return Some(Vec::new());
        }
        let shapes = contours.simplify_shape(FillRule::Positive);
        Some(Self::filter_overlay_shapes_by_area(shapes))
    }

    fn overlay_binary_shapes(
        subject: &NodeOverlayShapes,
        clip: &NodeOverlayShapes,
        rule: OverlayRule,
    ) -> Option<NodeOverlayShapes> {
        if subject.is_empty() {
            return Some(Vec::new());
        }
        if clip.is_empty() {
            return Some(subject.clone());
        }
        let shapes = subject
            .overlay_with_fixed_scale(clip, rule, FillRule::Positive, NODE_OVERLAY_SCALE)
            .ok()?;
        Some(Self::filter_overlay_shapes_by_area(shapes))
    }

    fn filter_overlay_shapes_by_area(shapes: NodeOverlayShapes) -> NodeOverlayShapes {
        shapes
            .into_iter()
            .filter_map(|shape| {
                let filtered = shape
                    .into_iter()
                    .filter(|contour| contour.len() >= 3)
                    .collect::<Vec<_>>();
                let outer = filtered.first()?;
                (Self::overlay_contour_area(outer).abs() > NODE_OVERLAY_MIN_AREA_M2)
                    .then_some(filtered)
            })
            .collect()
    }

    fn sort_overlay_shapes(shapes: &mut [NodeOverlayShape]) {
        shapes.sort_by(|a, b| {
            let area_a = a
                .first()
                .map(|contour| Self::overlay_contour_area(contour).abs())
                .unwrap_or(0.0);
            let area_b = b
                .first()
                .map(|contour| Self::overlay_contour_area(contour).abs())
                .unwrap_or(0.0);
            area_b
                .total_cmp(&area_a)
                .then_with(|| Self::overlay_shape_sort_key(a).cmp(&Self::overlay_shape_sort_key(b)))
        });
    }

    fn overlay_shape_sort_key(shape: &NodeOverlayShape) -> (i64, i64, usize) {
        let mut min_x = i64::MAX;
        let mut min_z = i64::MAX;
        let mut points = 0usize;
        for contour in shape {
            points += contour.len();
            for point in contour {
                min_x = min_x.min((point[0] * NODE_OVERLAY_SCALE).round() as i64);
                min_z = min_z.min((point[1] * NODE_OVERLAY_SCALE).round() as i64);
            }
        }
        (min_x, min_z, points)
    }

    fn overlay_contour_area(contour: &NodeOverlayContour) -> f32 {
        if contour.len() < 3 {
            return 0.0;
        }
        let mut signed_area = 0.0;
        for index in 0..contour.len() {
            let current = contour[index];
            let next = contour[(index + 1) % contour.len()];
            signed_area += current[0] * next[1] - next[0] * current[1];
        }
        signed_area * 0.5
    }

    fn visual_polygons_from_overlay_shapes(
        shapes: &[NodeOverlayShape],
        height_candidates: &[RoadSurfaceVisualPolygon],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut polygons = Vec::new();
        for shape in shapes {
            let Some(polygon) =
                Self::visual_polygon_from_overlay_shape(shape, height_candidates, true)
            else {
                continue;
            };
            polygons.push(polygon);
        }
        Self::sort_visual_polygons(&mut polygons);
        polygons
    }

    fn union_terrain_clip_polygons(
        polygons: &[RoadSurfaceVisualPolygon],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        if polygons.is_empty() {
            return Vec::new();
        }

        let contours = Self::overlay_contours_from_polygons(polygons);
        let Some(mut shapes) = Self::overlay_union_contours(&contours) else {
            return Vec::new();
        };
        Self::sort_overlay_shapes(&mut shapes);
        Self::outer_boundary_polygons_from_overlay_shapes(&shapes, polygons)
    }

    fn outer_boundary_polygons_from_overlay_shapes(
        shapes: &[NodeOverlayShape],
        height_candidates: &[RoadSurfaceVisualPolygon],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut polygons = Vec::new();
        for shape in shapes {
            let Some(polygon) =
                Self::visual_polygon_from_overlay_shape(shape, height_candidates, false)
            else {
                continue;
            };
            polygons.push(polygon);
        }
        Self::sort_visual_polygons(&mut polygons);
        polygons
    }

    fn visual_polygon_from_overlay_shape(
        shape: &NodeOverlayShape,
        height_candidates: &[RoadSurfaceVisualPolygon],
        preserve_holes: bool,
    ) -> Option<RoadSurfaceVisualPolygon> {
        let outer_contour = shape.first()?;
        let mut outer_points =
            Self::world_points_from_overlay_contour(outer_contour, height_candidates)?;
        if Self::signed_polygon_area_xz(&outer_points).abs() <= NODE_OVERLAY_MIN_AREA_M2 {
            return None;
        }
        if Self::signed_polygon_area_xz(&outer_points) < 0.0 {
            outer_points.reverse();
        }

        let mut hole_points = Vec::new();
        if preserve_holes {
            for contour in shape.iter().skip(1) {
                let mut points =
                    Self::world_points_from_overlay_contour(contour, height_candidates)?;
                if Self::signed_polygon_area_xz(&points).abs() <= NODE_OVERLAY_MIN_AREA_M2 {
                    continue;
                }
                if Self::signed_polygon_area_xz(&points) > 0.0 {
                    points.reverse();
                }
                hole_points.push(points);
            }
        }

        Self::canonicalize_world_loop(&mut outer_points)?;
        for hole in &mut hole_points {
            Self::canonicalize_world_loop(hole)?;
        }
        let triangles_world = Self::triangulate_constrained_shape_xz(&outer_points, &hole_points)?;
        Some(RoadSurfaceVisualPolygon {
            points_world: outer_points,
            triangles_world,
        })
    }

    fn world_points_from_overlay_contour(
        contour: &NodeOverlayContour,
        height_candidates: &[RoadSurfaceVisualPolygon],
    ) -> Option<Vec<Vector3>> {
        contour
            .iter()
            .map(|point| {
                let xz = Vector2::new(point[0], point[1]);
                let y = Self::sample_overlay_point_height_from_candidates(xz, height_candidates)?;
                Some(Vector3::new(point[0], y, point[1]))
            })
            .collect()
    }

    fn canonicalize_world_loop(points_world: &mut Vec<Vector3>) -> Option<()> {
        points_world.dedup_by(|a, b| (*a - *b).length_squared() <= 0.0001);
        if points_world.len() >= 2
            && (points_world.first().copied()? - points_world.last().copied()?).length_squared()
                <= 0.0001
        {
            points_world.pop();
        }
        if points_world.len() < 3 {
            return None;
        }
        let (start_index, _) = points_world.iter().enumerate().min_by(|(_, a), (_, b)| {
            a.x.total_cmp(&b.x)
                .then(a.z.total_cmp(&b.z))
                .then(a.y.total_cmp(&b.y))
        })?;
        points_world.rotate_left(start_index);
        Some(())
    }

    fn sample_overlay_point_height_from_candidates(
        point_xz: Vector2,
        candidates: &[RoadSurfaceVisualPolygon],
    ) -> Option<f32> {
        for polygon in candidates {
            for triangle in &polygon.triangles_world {
                if let Some((wa, wb, wc)) =
                    Self::triangle_barycentric_weights_xz(*triangle, point_xz)
                {
                    return Some(triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc);
                }
            }
        }

        let mut best_distance_squared = f32::INFINITY;
        let mut best_height = None;
        for polygon in candidates {
            if polygon.points_world.len() < 2 {
                continue;
            }
            for index in 0..polygon.points_world.len() {
                let start = polygon.points_world[index];
                let end = polygon.points_world[(index + 1) % polygon.points_world.len()];
                let start_xz = Vector2::new(start.x, start.z);
                let end_xz = Vector2::new(end.x, end.z);
                let segment = end_xz - start_xz;
                let length_squared = segment.length_squared();
                let t = if length_squared <= SAMPLE_EPSILON_M {
                    0.0
                } else {
                    ((point_xz - start_xz).dot(segment) / length_squared).clamp(0.0, 1.0)
                };
                let closest = start_xz + segment * t;
                let distance_squared = point_xz.distance_squared_to(closest);
                if distance_squared < best_distance_squared {
                    best_distance_squared = distance_squared;
                    best_height = Some(start.y + (end.y - start.y) * t);
                }
            }
        }
        best_height
    }

    fn triangulate_constrained_shape_xz(
        outer_points: &[Vector3],
        holes: &[Vec<Vector3>],
    ) -> Option<Vec<[Vector3; 3]>> {
        if outer_points.len() < 3 {
            return None;
        }

        let mut vertices = Vec::new();
        let mut vertex_lookup = BTreeMap::new();
        let mut constraints = BTreeSet::new();
        Self::push_surface_cdt_loop(
            outer_points,
            &mut vertices,
            &mut vertex_lookup,
            &mut constraints,
        );
        for hole in holes {
            Self::push_surface_cdt_loop(hole, &mut vertices, &mut vertex_lookup, &mut constraints);
        }

        let spade_vertices = vertices
            .iter()
            .map(|point| Point2::new(f64::from(point.x), f64::from(point.z)))
            .collect::<Vec<_>>();
        let mut invalid_constraints = 0usize;
        let cdt = SurfaceCdt::try_bulk_load_cdt(
            spade_vertices,
            constraints.into_iter().collect(),
            |_| invalid_constraints += 1,
        )
        .ok()?;
        if invalid_constraints > 0 {
            return None;
        }

        let mut triangles = Vec::new();
        for face in cdt.inner_faces() {
            let [a, b, c] = face.vertices();
            let triangle = [
                vertices[a.fix().index()],
                vertices[b.fix().index()],
                vertices[c.fix().index()],
            ];
            let centroid = Vector2::new(
                (triangle[0].x + triangle[1].x + triangle[2].x) / 3.0,
                (triangle[0].z + triangle[1].z + triangle[2].z) / 3.0,
            );
            if !Self::triangle_has_area_xz(triangle) {
                continue;
            }
            if !Self::polygon_contains_point_xz(outer_points, centroid) {
                continue;
            }
            if holes
                .iter()
                .any(|hole| Self::polygon_contains_point_xz(hole, centroid))
            {
                continue;
            }
            triangles.push(triangle);
        }

        (!triangles.is_empty()).then_some(triangles)
    }

    fn push_surface_cdt_loop(
        points_world: &[Vector3],
        vertices: &mut Vec<Vector3>,
        vertex_lookup: &mut BTreeMap<(i64, i64), usize>,
        constraints: &mut BTreeSet<[usize; 2]>,
    ) {
        if points_world.len() < 2 {
            return;
        }
        let indices = points_world
            .iter()
            .map(|point| Self::insert_surface_cdt_vertex(*point, vertices, vertex_lookup))
            .collect::<Vec<_>>();
        for index in 0..indices.len() {
            let edge = Self::normalize_surface_edge_array(
                indices[index],
                indices[(index + 1) % indices.len()],
            );
            if edge[0] != edge[1] {
                constraints.insert(edge);
            }
        }
    }

    fn insert_surface_cdt_vertex(
        point: Vector3,
        vertices: &mut Vec<Vector3>,
        vertex_lookup: &mut BTreeMap<(i64, i64), usize>,
    ) -> usize {
        let key = Self::surface_cdt_vertex_key(point);
        if let Some(index) = vertex_lookup.get(&key) {
            return *index;
        }
        let index = vertices.len();
        vertices.push(point);
        vertex_lookup.insert(key, index);
        index
    }

    fn surface_cdt_vertex_key(point: Vector3) -> (i64, i64) {
        (
            (point.x / SAMPLE_EPSILON_M).round() as i64,
            (point.z / SAMPLE_EPSILON_M).round() as i64,
        )
    }

    fn normalize_surface_edge_array(a: usize, b: usize) -> [usize; 2] {
        if a < b { [a, b] } else { [b, a] }
    }

    fn build_closed_earthwork_geometry_from_boundary_loops(
        &self,
        boundary_loops: &[RoadSurfaceVisualPolygon],
        terrain: &TerrainSystem,
    ) -> (
        Vec<RoadSurfaceVisualPolygon>,
        Vec<RoadSurfaceVisualPolygon>,
        Vec<RoadSurfaceEarthworkRenderFace>,
    ) {
        let mut earthwork_surface_polygons = Vec::new();
        let mut earthwork_outer_boundary_loops = Vec::new();
        let mut render_earthwork_faces = Vec::new();

        for boundary_loop in boundary_loops {
            let Some((outer_loop, side_polygons, render_faces)) =
                self.build_closed_earthwork_loop_geometry(&boundary_loop.points_world, terrain)
            else {
                continue;
            };
            earthwork_outer_boundary_loops.push(outer_loop);
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
        boundary_points: &[Vector3],
        terrain: &TerrainSystem,
    ) -> Option<(
        RoadSurfaceVisualPolygon,
        Vec<RoadSurfaceVisualPolygon>,
        Vec<RoadSurfaceEarthworkRenderFace>,
    )> {
        if boundary_points.len() < 3 {
            return None;
        }

        let outer_points: Vec<Vector3> = boundary_points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let outward = Self::closed_loop_vertex_outward_xz(boundary_points, index);
                self.earthwork_transition_point(*point, outward, terrain)
            })
            .collect();
        let outer_loop = Self::make_visual_polygon(outer_points.clone())?;
        let mut side_polygons = Vec::new();
        let mut render_faces = Vec::new();
        for index in 0..boundary_points.len() {
            let current = boundary_points[index];
            let next = boundary_points[(index + 1) % boundary_points.len()];
            let outer_current = outer_points[index];
            let outer_next = outer_points[(index + 1) % outer_points.len()];
            let Some(polygon) =
                Self::make_visual_polygon(vec![current, next, outer_next, outer_current])
            else {
                continue;
            };
            let face_kind =
                Self::classify_earthwork_face_kind(current, next, outer_next, outer_current);
            render_faces.push(RoadSurfaceEarthworkRenderFace {
                kind: face_kind,
                polygon: polygon.clone(),
            });
            side_polygons.push(polygon);
        }

        Some((outer_loop, side_polygons, render_faces))
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

    fn classify_earthwork_face_kind(
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

    fn build_incident_mouth_profile(
        &self,
        incident: IncidentSurfaceEdge,
    ) -> Option<IncidentMouthProfile> {
        let piece = self.compiled_visual_span_pieces.get(&incident.edge_idx)?;
        match incident.side {
            IncidentEdgeSide::Start => piece.start_mouth_profile.clone(),
            IncidentEdgeSide::End => piece.end_mouth_profile.clone(),
        }
    }

    fn section_outer_boundary_pair(section: &RoadSurfaceSection) -> Option<(Vector3, Vector3)> {
        let first_band = section.bands.first()?;
        let last_band = section.bands.last()?;
        let left_point = Self::section_boundary_world_point_static(
            section,
            first_band.lateral_start_m,
            first_band.height_start_m,
        );
        let right_point = Self::section_boundary_world_point_static(
            section,
            last_band.lateral_end_m,
            last_band.height_end_m,
        );
        Some((left_point, right_point))
    }

    fn build_node_corridor_candidates(
        node_pos: Vector3,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Option<NodeCorridorCandidates> {
        if mouths.len() == 2 {
            return Self::build_bend_corridor_candidates(node_pos, mouths);
        }

        let mut road_candidate_polygons = Vec::new();
        let mut non_road_candidate_polygons = Vec::new();
        let mut non_road_height_candidate_polygons = Vec::new();

        for mouth in mouths {
            let Some((outer_a, outer_b)) = Self::mouth_full_roadbed_segment(&mouth.profile) else {
                continue;
            };
            if let Some(polygon) =
                Self::build_mouth_corridor_polygon(node_pos, mouth.direction_xz, outer_a, outer_b)
            {
                non_road_candidate_polygons.push(NodeNonRoadCandidatePolygon { polygon });
            }

            if let Some((carriageway_a, carriageway_b)) =
                Self::mouth_carriageway_segment(&mouth.profile)
            {
                if let Some(polygon) = Self::build_mouth_corridor_polygon(
                    node_pos,
                    mouth.direction_xz,
                    carriageway_a,
                    carriageway_b,
                ) {
                    road_candidate_polygons.push(polygon);
                }
            }

            Self::append_mouth_non_road_height_candidates(
                node_pos,
                mouth,
                &mut non_road_height_candidate_polygons,
            );
        }

        (!road_candidate_polygons.is_empty() || !non_road_candidate_polygons.is_empty()).then_some(
            NodeCorridorCandidates {
                road_candidate_polygons,
                non_road_candidate_polygons,
                non_road_height_candidate_polygons,
            },
        )
    }

    fn build_bend_corridor_candidates(
        node_pos: Vector3,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Option<NodeCorridorCandidates> {
        let Some((start_index, end_index)) = Self::bend_join_mouth_order(mouths) else {
            return None;
        };
        let start_mouth = &mouths[start_index];
        let end_mouth = &mouths[end_index];
        let mut road_candidate_polygons = Vec::new();
        let mut non_road_candidate_polygons = Vec::new();
        let mut non_road_height_candidate_polygons = Vec::new();

        // Keep each bend corridor/join as a simple overlay candidate; merging them into one loop
        // can make the two throat caps cross at the node on tight bends.
        for mouth in [start_mouth, end_mouth] {
            if let Some((outer_a, outer_b)) = Self::mouth_full_roadbed_segment(&mouth.profile) {
                if let Some(polygon) = Self::build_mouth_corridor_polygon(
                    node_pos,
                    mouth.direction_xz,
                    outer_a,
                    outer_b,
                ) {
                    non_road_candidate_polygons.push(NodeNonRoadCandidatePolygon { polygon });
                }
            }

            if let Some((carriageway_a, carriageway_b)) =
                Self::mouth_carriageway_segment(&mouth.profile)
            {
                if let Some(polygon) = Self::build_mouth_corridor_polygon(
                    node_pos,
                    mouth.direction_xz,
                    carriageway_a,
                    carriageway_b,
                ) {
                    road_candidate_polygons.push(polygon);
                }
            }

            Self::append_mouth_non_road_height_candidates(
                node_pos,
                mouth,
                &mut non_road_height_candidate_polygons,
            );
        }

        for left_side in [true, false] {
            if let Some(polygon) = Self::build_bend_local_side_join_polygon(
                node_pos,
                start_mouth,
                end_mouth,
                Self::mouth_full_roadbed_segment,
                left_side,
            ) {
                non_road_candidate_polygons.push(NodeNonRoadCandidatePolygon { polygon });
            }

            if let Some(polygon) = Self::build_bend_local_side_join_polygon(
                node_pos,
                start_mouth,
                end_mouth,
                Self::mouth_carriageway_segment,
                left_side,
            ) {
                road_candidate_polygons.push(polygon);
            }
        }

        for (start_band, end_band) in start_mouth.profile.bands.iter().zip(&end_mouth.profile.bands)
        {
            if start_band.kind != end_band.kind
                || start_band.kind == RoadSurfaceBandKind::Carriageway
            {
                continue;
            }
            if let Some(polygon) = Self::build_bend_local_band_join_polygon(
                node_pos,
                start_mouth,
                end_mouth,
                start_band,
                end_band,
            ) {
                non_road_height_candidate_polygons.push(polygon);
            }
        }

        (!road_candidate_polygons.is_empty() || !non_road_candidate_polygons.is_empty()).then_some(
            NodeCorridorCandidates {
                road_candidate_polygons,
                non_road_candidate_polygons,
                non_road_height_candidate_polygons,
            },
        )
    }

    fn append_mouth_non_road_height_candidates(
        node_pos: Vector3,
        mouth: &OrderedIncidentPieceMouth,
        non_road_height_candidate_polygons: &mut Vec<RoadSurfaceVisualPolygon>,
    ) {
        for band in &mouth.profile.bands {
            if band.kind == RoadSurfaceBandKind::Carriageway {
                continue;
            }
            let Some(polygon) = Self::build_mouth_corridor_polygon(
                node_pos,
                mouth.direction_xz,
                band.start_point_world,
                band.end_point_world,
            ) else {
                continue;
            };
            non_road_height_candidate_polygons.push(polygon);
        }
    }

    fn mouth_full_roadbed_segment(profile: &IncidentMouthProfile) -> Option<(Vector3, Vector3)> {
        Some((
            *profile.boundary_points_world.first()?,
            *profile.boundary_points_world.last()?,
        ))
    }

    fn mouth_carriageway_segment(profile: &IncidentMouthProfile) -> Option<(Vector3, Vector3)> {
        let mut carriageway_indices =
            profile
                .bands
                .iter()
                .enumerate()
                .filter_map(|(index, band)| {
                    (band.kind == RoadSurfaceBandKind::Carriageway).then_some(index)
                });
        let first_carriageway = carriageway_indices.next()?;
        let last_carriageway = carriageway_indices.last().unwrap_or(first_carriageway);
        Some((
            *profile.boundary_points_world.get(first_carriageway)?,
            *profile.boundary_points_world.get(last_carriageway + 1)?,
        ))
    }

    fn bend_join_mouth_order(mouths: &[OrderedIncidentPieceMouth]) -> Option<(usize, usize)> {
        if mouths.len() != 2 {
            return None;
        }
        let angle_a = mouths[0].direction_angle_ccw;
        let angle_b = mouths[1].direction_angle_ccw;
        let diff_ab = (angle_b - angle_a).rem_euclid(std::f32::consts::TAU);
        if diff_ab <= SAMPLE_EPSILON_M {
            return None;
        }
        if diff_ab <= std::f32::consts::PI {
            Some((0, 1))
        } else {
            Some((1, 0))
        }
    }

    fn build_bend_local_side_join_polygon(
        node_pos: Vector3,
        start_mouth: &OrderedIncidentPieceMouth,
        end_mouth: &OrderedIncidentPieceMouth,
        segment_fn: fn(&IncidentMouthProfile) -> Option<(Vector3, Vector3)>,
        left_side: bool,
    ) -> Option<RoadSurfaceVisualPolygon> {
        let (start_a, start_b) = segment_fn(&start_mouth.profile)?;
        let (end_a, end_b) = segment_fn(&end_mouth.profile)?;
        let start_travel = -start_mouth.direction_xz;
        let end_travel = end_mouth.direction_xz;
        if start_travel.length_squared() <= SAMPLE_EPSILON_M
            || end_travel.length_squared() <= SAMPLE_EPSILON_M
        {
            return None;
        }
        let start_travel = start_travel.normalized();
        let end_travel = end_travel.normalized();
        let turn = Self::cross_xz(start_travel, end_travel);
        if turn.abs() <= SAMPLE_EPSILON_M {
            return None;
        }

        let (start_left, start_right) =
            Self::segment_left_right_for_travel(start_travel, start_a, start_b)?;
        let (end_left, end_right) =
            Self::segment_left_right_for_travel(end_travel, end_a, end_b)?;
        let start_center = Self::midpoint_world(start_left, start_right);
        let end_center = Self::midpoint_world(end_left, end_right);
        let (start_side, end_side) = if left_side {
            (start_left, end_left)
        } else {
            (start_right, end_right)
        };
        let start_node =
            Self::bend_node_side_point(node_pos, start_travel, start_center, start_side, left_side);
        let end_node =
            Self::bend_node_side_point(node_pos, end_travel, end_center, end_side, left_side);
        let ccw = Self::bend_short_arc_is_ccw(node_pos, start_node, end_node)?;
        let center_height = (start_node.y + end_node.y) * 0.5;
        let mut points_world = vec![
            Vector3::new(node_pos.x, center_height, node_pos.z),
            start_node,
        ];
        Self::append_bend_arc_points(&mut points_world, node_pos, start_node, end_node, ccw);
        Self::make_visual_polygon(points_world)
    }

    fn build_bend_local_band_join_polygon(
        node_pos: Vector3,
        start_mouth: &OrderedIncidentPieceMouth,
        end_mouth: &OrderedIncidentPieceMouth,
        start_band: &IncidentMouthBand,
        end_band: &IncidentMouthBand,
    ) -> Option<RoadSurfaceVisualPolygon> {
        let start_a = start_band.start_point_world;
        let start_b = start_band.end_point_world;
        let end_a = end_band.start_point_world;
        let end_b = end_band.end_point_world;
        Self::build_bend_local_segment_join_polygon(
            node_pos,
            start_mouth.direction_xz,
            end_mouth.direction_xz,
            start_a,
            start_b,
            end_a,
            end_b,
        )
    }

    fn build_bend_local_segment_join_polygon(
        node_pos: Vector3,
        start_direction_xz: Vector2,
        end_direction_xz: Vector2,
        start_a: Vector3,
        start_b: Vector3,
        end_a: Vector3,
        end_b: Vector3,
    ) -> Option<RoadSurfaceVisualPolygon> {
        let start_travel = -start_direction_xz;
        let end_travel = end_direction_xz;
        if start_travel.length_squared() <= SAMPLE_EPSILON_M
            || end_travel.length_squared() <= SAMPLE_EPSILON_M
        {
            return None;
        }
        let start_travel = start_travel.normalized();
        let end_travel = end_travel.normalized();
        let turn = Self::cross_xz(start_travel, end_travel);
        if turn.abs() <= SAMPLE_EPSILON_M {
            return None;
        }

        let (start_left, start_right) =
            Self::segment_left_right_for_travel(start_travel, start_a, start_b)?;
        let (end_left, end_right) =
            Self::segment_left_right_for_travel(end_travel, end_a, end_b)?;
        let start_center = Self::midpoint_world(start_left, start_right);
        let end_center = Self::midpoint_world(end_left, end_right);
        let start_left_node =
            Self::bend_node_side_point(node_pos, start_travel, start_center, start_left, true);
        let start_right_node =
            Self::bend_node_side_point(node_pos, start_travel, start_center, start_right, false);
        let end_left_node =
            Self::bend_node_side_point(node_pos, end_travel, end_center, end_left, true);
        let end_right_node =
            Self::bend_node_side_point(node_pos, end_travel, end_center, end_right, false);
        let ccw = Self::bend_short_arc_is_ccw(node_pos, start_left_node, end_left_node)?;
        let mut points_world = vec![start_left_node];
        Self::append_bend_arc_points(
            &mut points_world,
            node_pos,
            start_left_node,
            end_left_node,
            ccw,
        );
        points_world.push(end_right_node);
        Self::append_bend_arc_points(
            &mut points_world,
            node_pos,
            end_right_node,
            start_right_node,
            !ccw,
        );
        Self::make_visual_polygon(points_world)
    }

    fn bend_short_arc_is_ccw(node_pos: Vector3, from: Vector3, to: Vector3) -> Option<bool> {
        let from_vector = Vector2::new(from.x - node_pos.x, from.z - node_pos.z);
        let to_vector = Vector2::new(to.x - node_pos.x, to.z - node_pos.z);
        if from_vector.length_squared() <= SAMPLE_EPSILON_M
            || to_vector.length_squared() <= SAMPLE_EPSILON_M
        {
            return None;
        }
        let from_angle = Self::normalized_angle_ccw(from_vector);
        let to_angle = Self::normalized_angle_ccw(to_vector);
        let ccw_span = (to_angle - from_angle).rem_euclid(std::f32::consts::TAU);
        Some(ccw_span <= std::f32::consts::PI)
    }

    fn segment_left_right_for_travel(
        travel_xz: Vector2,
        a: Vector3,
        b: Vector3,
    ) -> Option<(Vector3, Vector3)> {
        if travel_xz.length_squared() <= SAMPLE_EPSILON_M {
            return None;
        }
        let center = Vector2::new((a.x + b.x) * 0.5, (a.z + b.z) * 0.5);
        let cross_a = Self::cross_xz(travel_xz, Vector2::new(a.x, a.z) - center);
        let cross_b = Self::cross_xz(travel_xz, Vector2::new(b.x, b.z) - center);
        if cross_a >= cross_b {
            Some((a, b))
        } else {
            Some((b, a))
        }
    }

    fn bend_node_side_point(
        node_pos: Vector3,
        travel_xz: Vector2,
        segment_center: Vector3,
        side_point: Vector3,
        left_side: bool,
    ) -> Vector3 {
        let left_normal = Self::left_normal_xz(travel_xz);
        let side_normal = if left_side { left_normal } else { -left_normal };
        let side_width = Vector2::new(
            side_point.x - segment_center.x,
            side_point.z - segment_center.z,
        )
        .length();
        Vector3::new(
            node_pos.x + side_normal.x * side_width,
            side_point.y,
            node_pos.z + side_normal.y * side_width,
        )
    }

    fn append_bend_arc_points(
        points_world: &mut Vec<Vector3>,
        node_pos: Vector3,
        from: Vector3,
        to: Vector3,
        ccw: bool,
    ) {
        let from_vector = Vector2::new(from.x - node_pos.x, from.z - node_pos.z);
        let to_vector = Vector2::new(to.x - node_pos.x, to.z - node_pos.z);
        let from_radius = from_vector.length();
        let to_radius = to_vector.length();
        if from_radius <= SAMPLE_EPSILON_M || to_radius <= SAMPLE_EPSILON_M {
            return;
        }
        if points_world.last().is_none_or(|point| {
            (*point - from).length_squared() > SAMPLE_EPSILON_M * SAMPLE_EPSILON_M
        }) {
            points_world.push(from);
        }
        let from_angle = Self::normalized_angle_ccw(from_vector);
        let to_angle = Self::normalized_angle_ccw(to_vector);
        let angle_span = if ccw {
            (to_angle - from_angle).rem_euclid(std::f32::consts::TAU)
        } else {
            (from_angle - to_angle).rem_euclid(std::f32::consts::TAU)
        };
        if angle_span <= SAMPLE_EPSILON_M || angle_span > std::f32::consts::PI {
            points_world.push(to);
            return;
        }
        let max_radius = from_radius.max(to_radius);
        let segment_count = ((angle_span * max_radius) / BEND_JOIN_ARC_SAMPLE_STEP_M)
            .ceil()
            .clamp(2.0, 96.0) as usize;
        for index in 1..=segment_count {
            let t = index as f32 / segment_count as f32;
            if index == segment_count {
                points_world.push(to);
                continue;
            }
            let angle = if ccw {
                from_angle + angle_span * t
            } else {
                from_angle - angle_span * t
            };
            let radius = from_radius + (to_radius - from_radius) * t;
            let height = from.y + (to.y - from.y) * t;
            points_world.push(Vector3::new(
                node_pos.x + angle.cos() * radius,
                height,
                node_pos.z + angle.sin() * radius,
            ));
        }
    }

    fn midpoint_world(a: Vector3, b: Vector3) -> Vector3 {
        Vector3::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5, (a.z + b.z) * 0.5)
    }

    fn left_normal_xz(direction_xz: Vector2) -> Vector2 {
        Vector2::new(-direction_xz.y, direction_xz.x)
    }

    fn cross_xz(a: Vector2, b: Vector2) -> f32 {
        a.x * b.y - a.y * b.x
    }

    fn build_mouth_corridor_polygon(
        node_pos: Vector3,
        direction_xz: Vector2,
        segment_a: Vector3,
        segment_b: Vector3,
    ) -> Option<RoadSurfaceVisualPolygon> {
        if direction_xz.length_squared() <= SAMPLE_EPSILON_M {
            return None;
        }
        let direction_xz = direction_xz.normalized();
        let node_xz = Vector2::new(node_pos.x, node_pos.z);
        let segment_center_xz = Vector2::new(
            (segment_a.x + segment_b.x) * 0.5,
            (segment_a.z + segment_b.z) * 0.5,
        );
        let mut depth_m = (segment_center_xz - node_xz).dot(direction_xz).max(0.0);
        if depth_m <= SAMPLE_EPSILON_M {
            depth_m = Vector2::new(segment_a.x - node_pos.x, segment_a.z - node_pos.z)
                .length()
                .max(Vector2::new(segment_b.x - node_pos.x, segment_b.z - node_pos.z).length());
        }
        if depth_m <= SAMPLE_EPSILON_M {
            return None;
        }

        let backtrack = Vector3::new(direction_xz.x * depth_m, 0.0, direction_xz.y * depth_m);
        let node_a = segment_a - backtrack;
        let node_b = segment_b - backtrack;
        Self::make_visual_polygon(vec![segment_a, segment_b, node_b, node_a])
    }

    fn normalized_angle_ccw(direction_xz: Vector2) -> f32 {
        let angle = direction_xz.y.atan2(direction_xz.x);
        if angle < 0.0 {
            angle + std::f32::consts::TAU
        } else {
            angle
        }
    }

    fn assemble_explicit_node_piece(
        &self,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
        mut road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut earthwork_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut earthwork_outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
        mut render_earthwork_faces: Vec<RoadSurfaceEarthworkRenderFace>,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if road_surface_polygons.is_empty() && sidewalk_surface_polygons.is_empty() {
            return None;
        }
        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_visual_polygons(&mut earthwork_surface_polygons);
        Self::sort_visual_polygons(&mut earthwork_outer_boundary_loops);
        Self::sort_earthwork_render_faces(&mut render_earthwork_faces);
        if outer_boundary_loops.is_empty() {
            return None;
        }
        Some(RoadSurfaceVisualNodePiece {
            node_id,
            kind,
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        })
    }

    fn sort_visual_polygons(polygons: &mut [RoadSurfaceVisualPolygon]) {
        polygons.sort_by(|a, b| {
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

    fn sort_earthwork_render_faces(faces: &mut [RoadSurfaceEarthworkRenderFace]) {
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

    fn classify_visual_node_kind(&self, incidents: &[IncidentSurfaceEdge]) -> CompiledNodeKind {
        match incidents.len() {
            0 | 1 => CompiledNodeKind::Terminal,
            2 => {
                let a = incidents[0];
                let b = incidents[1];
                let straight = a.direction_xz.dot(b.direction_xz) <= -PASS_THROUGH_DOT_THRESHOLD;
                if !straight {
                    return CompiledNodeKind::Bend;
                }
                CompiledNodeKind::PassThrough
            }
            _ => CompiledNodeKind::JunctionN,
        }
    }

    fn classify_surface_node_kind(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Option<CompiledNodeKind> {
        let incidents = self.sorted_incident_surface_edges(graph, node_id);
        (!incidents.is_empty()).then(|| self.classify_visual_node_kind(&incidents))
    }

    fn sorted_incident_surface_edges(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        let mut incidents = self.collect_incident_surface_edges(graph, node_id);
        incidents.sort_by(|a, b| {
            Self::normalized_angle_ccw(a.direction_xz)
                .total_cmp(&Self::normalized_angle_ccw(b.direction_xz))
                .then(a.edge_idx.cmp(&b.edge_idx))
                .then(a.side.cmp(&b.side))
        });
        incidents
    }

    fn collect_incident_surface_edges(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        if node_id as usize >= graph.node_adjacency_count() {
            return Vec::new();
        }

        let mut incidents = Vec::new();
        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if !Self::is_surface_edge(edge) {
                continue;
            }

            let side = if graph.get_valid_node(edge.start_node) == node_id {
                Some(IncidentEdgeSide::Start)
            } else if graph.get_valid_node(edge.end_node) == node_id {
                Some(IncidentEdgeSide::End)
            } else {
                None
            };
            let Some(side) = side else {
                continue;
            };
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            let Some(direction_xz) = (match side {
                IncidentEdgeSide::Start => piece
                    .start_mouth_profile
                    .as_ref()
                    .map(|mouth| mouth.inward_direction_xz),
                IncidentEdgeSide::End => piece
                    .end_mouth_profile
                    .as_ref()
                    .map(|mouth| mouth.inward_direction_xz),
            }) else {
                continue;
            };
            incidents.push(IncidentSurfaceEdge {
                edge_idx,
                side,
                direction_xz,
            });
        }

        incidents.sort_by(|a, b| a.edge_idx.cmp(&b.edge_idx).then(a.side.cmp(&b.side)));
        incidents
    }

    fn build_lateral_bands(
        &self,
        edge: &Edge,
        terrain: &TerrainSystem,
        center_xz: Vector2,
        lateral_xz: Vector2,
        center_height_m: f32,
    ) -> Vec<RoadSurfaceBand> {
        if edge.primary_type == TransitType::Foot || (edge.allowed_types & TransitFlags::CAR) == 0 {
            let half_width = edge.width.max(2.0) * 0.5;
            return vec![RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Footpath,
                lateral_start_m: -half_width,
                lateral_end_m: half_width,
                height_start_m: center_height_m,
                height_end_m: center_height_m,
            }];
        }

        let half_carriageway = edge.width.max(config::LANE_WIDTH) * 0.5;
        let (left_carriageway_height, center_carriageway_height, right_carriageway_height) = self
            .solve_standard_cross_section_profile(
                edge,
                terrain,
                center_xz,
                lateral_xz,
                center_height_m,
                half_carriageway,
            );
        let sidewalk_total = if edge.allowed_types & TransitFlags::FOOT != 0 {
            config::SIDEWALK_WIDTH
        } else {
            0.0
        };
        let curb_width = if sidewalk_total > 0.0 {
            CURB_BAND_WIDTH_M.min(sidewalk_total)
        } else {
            0.0
        };
        let sidewalk_width = (sidewalk_total - curb_width).max(0.0);
        let left_curb_top_height = left_carriageway_height
            + if curb_width > 0.0 {
                CURB_STEP_HEIGHT_M
            } else {
                0.0
            };
        let right_curb_top_height = right_carriageway_height
            + if curb_width > 0.0 {
                CURB_STEP_HEIGHT_M
            } else {
                0.0
            };
        let mut bands = Vec::new();
        if sidewalk_width > 0.0 {
            bands.push(RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Sidewalk,
                lateral_start_m: -(half_carriageway + curb_width + sidewalk_width),
                lateral_end_m: -(half_carriageway + curb_width),
                height_start_m: left_curb_top_height,
                height_end_m: left_curb_top_height,
            });
        }

        bands.push(RoadSurfaceBand {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            lateral_start_m: -(half_carriageway + curb_width),
            lateral_end_m: -half_carriageway,
            height_start_m: left_curb_top_height,
            height_end_m: left_carriageway_height,
        });
        bands.push(RoadSurfaceBand {
            kind: RoadSurfaceBandKind::Carriageway,
            lateral_start_m: -half_carriageway,
            lateral_end_m: 0.0,
            height_start_m: left_carriageway_height,
            height_end_m: center_carriageway_height,
        });
        bands.push(RoadSurfaceBand {
            kind: RoadSurfaceBandKind::Carriageway,
            lateral_start_m: 0.0,
            lateral_end_m: half_carriageway,
            height_start_m: center_carriageway_height,
            height_end_m: right_carriageway_height,
        });
        bands.push(RoadSurfaceBand {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            lateral_start_m: half_carriageway,
            lateral_end_m: half_carriageway + curb_width,
            height_start_m: right_carriageway_height,
            height_end_m: right_curb_top_height,
        });

        if sidewalk_width > 0.0 {
            bands.push(RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Sidewalk,
                lateral_start_m: half_carriageway + curb_width,
                lateral_end_m: half_carriageway + curb_width + sidewalk_width,
                height_start_m: right_curb_top_height,
                height_end_m: right_curb_top_height,
            });
        }

        bands
    }

    fn solve_standard_cross_section_profile(
        &self,
        edge: &Edge,
        terrain: &TerrainSystem,
        center_xz: Vector2,
        lateral_xz: Vector2,
        center_height_m: f32,
        half_carriageway: f32,
    ) -> (f32, f32, f32) {
        if edge.class != EdgeClass::Standard || half_carriageway <= SAMPLE_EPSILON_M {
            return (center_height_m, center_height_m, center_height_m);
        }

        let left_point = center_xz - lateral_xz * half_carriageway;
        let right_point = center_xz + lateral_xz * half_carriageway;
        let left_terrain_height =
            terrain.sample_height_world(left_point.x, left_point.y) * config::HEIGHT_SCALE;
        let right_terrain_height =
            terrain.sample_height_world(right_point.x, right_point.y) * config::HEIGHT_SCALE;
        let terrain_implied_crossfall_rate =
            (right_terrain_height - left_terrain_height) / (half_carriageway * 2.0);
        let crossfall_rate =
            if terrain_implied_crossfall_rate.abs() <= STANDARD_CROSSFALL_DEADZONE_RATE {
                0.0
            } else {
                terrain_implied_crossfall_rate.clamp(
                    -MAX_STANDARD_DESIGN_CROSSFALL_RATE,
                    MAX_STANDARD_DESIGN_CROSSFALL_RATE,
                )
            };

        (
            center_height_m - crossfall_rate * half_carriageway,
            center_height_m,
            center_height_m + crossfall_rate * half_carriageway,
        )
    }

    fn solve_section_height(&self, center: Vector3) -> f32 {
        center.y
    }

    fn build_section_sample_distances(&self, edge: &Edge, cumulative: &[f32]) -> Vec<f32> {
        let Some(&total_length) = cumulative.last() else {
            return vec![0.0];
        };
        if total_length <= SAMPLE_EPSILON_M {
            return vec![0.0];
        }

        let mut samples = vec![0.0, total_length];
        let start_throat = Self::visual_start_handoff_m(edge, total_length);
        let end_throat = Self::visual_end_handoff_s_m(edge, total_length);
        samples.push(start_throat);
        samples.push(end_throat);

        for &distance in cumulative {
            samples.push(distance);
        }

        let step_m = self.section_step_for_class(edge.class);
        for segment in cumulative.windows(2) {
            let start_s = segment[0];
            let end_s = segment[1];
            let mut sample_s = start_s + step_m;
            while sample_s < end_s - SAMPLE_EPSILON_M {
                samples.push(sample_s);
                sample_s += step_m;
            }
        }

        samples.sort_by(f32::total_cmp);
        samples.dedup_by(|a, b| (*a - *b).abs() <= SAMPLE_EPSILON_M);
        samples
    }

    fn visual_roadbed_half_width_m(edge: &Edge) -> f32 {
        if edge.primary_type == TransitType::Foot || (edge.allowed_types & TransitFlags::CAR) == 0 {
            return edge.width.max(2.0) * 0.5;
        }

        let sidewalk_total = if edge.allowed_types & TransitFlags::FOOT != 0 {
            config::SIDEWALK_WIDTH
        } else {
            0.0
        };
        edge.width.max(config::LANE_WIDTH) * 0.5 + sidewalk_total
    }

    fn visual_node_handoff_limit_m(edge: &Edge) -> f32 {
        Self::visual_roadbed_half_width_m(edge) + VISUAL_NODE_HANDOFF_PADDING_M
    }

    fn visual_start_handoff_m(edge: &Edge, total_length_m: f32) -> f32 {
        if edge.start_clip <= SAMPLE_EPSILON_M {
            0.0
        } else {
            edge.start_clip
                .max(Self::visual_node_handoff_limit_m(edge))
                .clamp(0.0, total_length_m)
        }
    }

    fn visual_end_handoff_m(edge: &Edge, total_length_m: f32) -> f32 {
        if edge.end_clip <= SAMPLE_EPSILON_M {
            0.0
        } else {
            edge.end_clip
                .max(Self::visual_node_handoff_limit_m(edge))
                .clamp(0.0, total_length_m)
        }
    }

    fn visual_end_handoff_s_m(edge: &Edge, total_length_m: f32) -> f32 {
        (total_length_m - Self::visual_end_handoff_m(edge, total_length_m))
            .clamp(0.0, total_length_m)
    }

    fn section_step_for_class(&self, class: EdgeClass) -> f32 {
        match class {
            EdgeClass::Standard => STANDARD_SECTION_STEP_M,
            EdgeClass::Bridge => BRIDGE_SECTION_STEP_M,
            EdgeClass::Tunnel => TUNNEL_SECTION_STEP_M,
        }
    }

    fn sample_polyline(
        &self,
        points: &[Vector3],
        cumulative: &[f32],
        s_m: f32,
    ) -> (Vector3, Vector2) {
        if points.len() == 1 {
            return (points[0], Vector2::RIGHT);
        }

        let total_length = cumulative.last().copied().unwrap_or(0.0);
        let clamped_s = s_m.clamp(0.0, total_length);

        for index in 0..points.len() - 1 {
            let start_s = cumulative[index];
            let end_s = cumulative[index + 1];
            if clamped_s > end_s && index + 2 < points.len() {
                continue;
            }

            let start = points[index];
            let end = points[index + 1];
            let segment_length = (end_s - start_s).max(SAMPLE_EPSILON_M);
            let local_t = ((clamped_s - start_s) / segment_length).clamp(0.0, 1.0);
            let point = start.lerp(end, local_t);
            let tangent_xz = self.segment_tangent_xz(points, index);
            return (point, tangent_xz);
        }

        (
            *points.last().unwrap(),
            self.segment_tangent_xz(points, points.len().saturating_sub(2)),
        )
    }

    fn segment_tangent_xz(&self, points: &[Vector3], preferred_index: usize) -> Vector2 {
        if points.len() < 2 {
            return Vector2::RIGHT;
        }

        let mut candidates = Vec::new();
        candidates.push(preferred_index.min(points.len() - 2));
        if preferred_index > 0 {
            candidates.push(preferred_index - 1);
        }
        if preferred_index + 1 < points.len() - 1 {
            candidates.push(preferred_index + 1);
        }

        for index in candidates {
            let delta = points[index + 1] - points[index];
            let tangent_xz = Vector2::new(delta.x, delta.z);
            if tangent_xz.length_squared() > 1e-8 {
                return tangent_xz.normalized();
            }
        }

        for window in points.windows(2) {
            let delta = window[1] - window[0];
            let tangent_xz = Vector2::new(delta.x, delta.z);
            if tangent_xz.length_squared() > 1e-8 {
                return tangent_xz.normalized();
            }
        }

        Vector2::RIGHT
    }

    fn build_cumulative_distances(&self, points: &[Vector3]) -> Vec<f32> {
        let mut cumulative = Vec::with_capacity(points.len());
        let mut running = 0.0;
        cumulative.push(0.0);
        for segment in points.windows(2) {
            running += segment[0].distance_to(segment[1]);
            cumulative.push(running);
        }
        cumulative
    }

    fn build_preview_surface_vertices(&self, sections: &[RoadSurfaceSection]) -> Vec<Vector3> {
        if sections.len() < 2 {
            return Vec::new();
        }

        let mut vertices = Vec::new();
        for pair in sections.windows(2) {
            let profile_a = self.section_profile_world_points(&pair[0], PREVIEW_MESH_LIFT_M);
            let profile_b = self.section_profile_world_points(&pair[1], PREVIEW_MESH_LIFT_M);
            if profile_a.len() < 2 || profile_a.len() != profile_b.len() {
                continue;
            }

            for index in 0..profile_a.len() - 1 {
                let a0 = profile_a[index];
                let a1 = profile_a[index + 1];
                let b0 = profile_b[index];
                let b1 = profile_b[index + 1];
                vertices.extend_from_slice(&[a0, b0, a1, a1, b0, b1]);
            }
        }

        vertices
    }

    fn section_profile_world_points(
        &self,
        section: &RoadSurfaceSection,
        y_lift_m: f32,
    ) -> Vec<Vector3> {
        let Some(first_band) = section.bands.first() else {
            return Vec::new();
        };

        let mut points = Vec::with_capacity(section.bands.len() + 1);
        let mut first_point = self.section_boundary_world_point(
            section,
            first_band.lateral_start_m,
            first_band.height_start_m,
        );
        first_point.y += y_lift_m;
        points.push(first_point);

        for band in &section.bands {
            let mut point =
                self.section_boundary_world_point(section, band.lateral_end_m, band.height_end_m);
            point.y += y_lift_m;
            points.push(point);
        }

        points
    }

    fn preview_surface_is_valid(
        edge_class: EdgeClass,
        prepared_points: &[Vector3],
        compiled_sections: &[RoadSurfaceSection],
        terrain: &TerrainSystem,
    ) -> bool {
        for pair in compiled_sections.windows(2) {
            let run = (pair[1].s_m - pair[0].s_m).abs();
            if run <= SAMPLE_EPSILON_M {
                continue;
            }
            let grade = (pair[1].center_height_m - pair[0].center_height_m).abs() / run;
            if grade > PREVIEW_MAX_GRADE {
                return false;
            }
        }

        if prepared_points.len() > 2 {
            if let Some(mid_section) = compiled_sections.get(compiled_sections.len() / 2) {
                let terrain_h = terrain
                    .sample_height_world(mid_section.center_xz.x, mid_section.center_xz.y)
                    * config::HEIGHT_SCALE;
                match edge_class {
                    EdgeClass::Bridge => {
                        if mid_section.center_height_m < terrain_h + PREVIEW_CLEARANCE_M {
                            return false;
                        }
                    }
                    EdgeClass::Tunnel => {
                        if mid_section.center_height_m > terrain_h - PREVIEW_CLEARANCE_M {
                            return false;
                        }
                    }
                    EdgeClass::Standard => {}
                }
            }
        }

        true
    }

    fn rebuild_surface_chunk_cache(&mut self, chunks: &[SurfaceChunkKey]) {
        for &chunk in chunks {
            let edge_indices = Self::sorted_owner_set(self.surface_chunk_spans.get(&chunk));
            let node_ids = Self::sorted_owner_set(self.surface_chunk_nodes.get(&chunk));
            if edge_indices.is_empty() && node_ids.is_empty() {
                self.surface_chunk_cache.remove(&chunk);
                continue;
            }
            self.surface_chunk_cache.insert(
                chunk,
                RoadSurfaceChunkCacheEntry {
                    chunk,
                    edge_indices,
                    node_ids,
                },
            );
        }
    }

    fn rebuild_earthwork_chunk_cache(&mut self, chunks: &[SurfaceChunkKey]) {
        for &chunk in chunks {
            let edge_indices = Self::sorted_owner_set(self.earthwork_chunk_spans.get(&chunk));
            let node_ids = Self::sorted_owner_set(self.earthwork_chunk_nodes.get(&chunk));
            if edge_indices.is_empty() && node_ids.is_empty() {
                self.earthwork_chunk_cache.remove(&chunk);
                continue;
            }
            self.earthwork_chunk_cache.insert(
                chunk,
                RoadEarthworkChunkCacheEntry {
                    chunk,
                    edge_indices,
                    node_ids,
                },
            );
        }
    }

    fn sorted_owner_set<T: Copy + Ord>(owners: Option<&BTreeSet<T>>) -> Vec<T> {
        match owners {
            Some(owners) => owners.iter().copied().collect(),
            None => Vec::new(),
        }
    }

    fn collect_query_contributors(
        &self,
        min_chunk: SurfaceChunkKey,
        max_chunk: SurfaceChunkKey,
    ) -> (Vec<usize>, Vec<u32>) {
        let mut edge_indices = HashSet::new();
        let mut node_ids = HashSet::new();
        for cx in (min_chunk.0 - 1)..=(max_chunk.0 + 1) {
            for cz in (min_chunk.1 - 1)..=(max_chunk.1 + 1) {
                let chunk = (cx, cz);
                if let Some(entry) = self.surface_chunk_cache.get(&chunk) {
                    edge_indices.extend(entry.edge_indices.iter().copied());
                    node_ids.extend(entry.node_ids.iter().copied());
                }
                if let Some(entry) = self.earthwork_chunk_cache.get(&chunk) {
                    edge_indices.extend(entry.edge_indices.iter().copied());
                    node_ids.extend(entry.node_ids.iter().copied());
                }
            }
        }

        let mut edge_indices: Vec<usize> = edge_indices.into_iter().collect();
        edge_indices.sort_unstable();
        let mut node_ids: Vec<u32> = node_ids.into_iter().collect();
        node_ids.sort_unstable();
        (edge_indices, node_ids)
    }

    fn visit_visible_span_piece_triangles<F>(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        visitor: &mut F,
    ) where
        F: FnMut([Vector3; 3]),
    {
        for polygon in piece
            .road_surface_polygons
            .iter()
            .chain(&piece.sidewalk_surface_polygons)
        {
            Self::visit_visual_polygon_triangles(polygon, visitor);
        }
    }

    fn visit_span_piece_earthwork_triangles<F>(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        visitor: &mut F,
    ) where
        F: FnMut([Vector3; 3]),
    {
        for polygon in &piece.earthwork_surface_polygons {
            Self::visit_visual_polygon_triangles(polygon, visitor);
        }
    }

    #[cfg(test)]
    fn visit_span_piece_clearance_triangles<F>(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        visitor: &mut F,
    ) where
        F: FnMut([Vector3; 3]),
    {
        for polygon in piece
            .clearance_road_surface_polygons
            .iter()
            .chain(&piece.clearance_sidewalk_surface_polygons)
        {
            Self::visit_visual_polygon_triangles(polygon, visitor);
        }
    }

    fn visit_visible_node_piece_triangles<F>(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        visitor: &mut F,
    ) where
        F: FnMut([Vector3; 3]),
    {
        if !self.node_uses_visible_surface(graph, terrain, node_id) {
            return;
        }

        for polygon in piece
            .road_surface_polygons
            .iter()
            .chain(&piece.sidewalk_surface_polygons)
        {
            Self::visit_visual_polygon_triangles(polygon, visitor);
        }
    }

    fn visit_node_piece_earthwork_triangles<F>(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        visitor: &mut F,
    ) where
        F: FnMut([Vector3; 3]),
    {
        if !self.node_piece_uses_earthworks(graph, node_id, terrain) {
            return;
        }

        for polygon in &piece.earthwork_surface_polygons {
            Self::visit_visual_polygon_triangles(polygon, visitor);
        }
    }

    fn visit_visual_polygon_triangles<F>(polygon: &RoadSurfaceVisualPolygon, visitor: &mut F)
    where
        F: FnMut([Vector3; 3]),
    {
        for &triangle in &polygon.triangles_world {
            if Self::triangle_has_area_xz(triangle) {
                visitor(triangle);
            }
        }
    }

    fn visible_corridor_index_range_for_edge(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        sections: &[RoadSurfaceSection],
    ) -> Option<(usize, usize)> {
        if sections.len() < 2 || edge_idx >= graph.edge_count() {
            return None;
        }

        let edge = graph.edge(edge_idx);
        let total_length = sections.last()?.s_m.max(0.0);
        let start_kind =
            self.classify_surface_node_kind(graph, graph.get_valid_node(edge.start_node));
        let end_kind = self.classify_surface_node_kind(graph, graph.get_valid_node(edge.end_node));
        let start_handoff = if matches!(
            start_kind,
            Some(CompiledNodeKind::Bend | CompiledNodeKind::JunctionN)
        ) {
            Self::visual_start_handoff_m(edge, total_length)
        } else {
            0.0
        };
        let end_handoff = if matches!(
            end_kind,
            Some(CompiledNodeKind::Bend | CompiledNodeKind::JunctionN)
        ) {
            Self::visual_end_handoff_s_m(edge, total_length)
        } else {
            total_length
        };
        if end_handoff - start_handoff <= SAMPLE_EPSILON_M {
            return None;
        }

        let start_index = sections
            .iter()
            .position(|section| section.s_m + SAMPLE_EPSILON_M >= start_handoff)
            .unwrap_or(0);
        let end_index = sections
            .iter()
            .rposition(|section| section.s_m - SAMPLE_EPSILON_M <= end_handoff)
            .unwrap_or(sections.len().saturating_sub(1));
        (end_index > start_index).then_some((start_index, end_index))
    }

    fn earthwork_transition_point(
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
        self.stamp_piece_top_surface_clearance_for_chunk(
            &piece.clearance_road_surface_polygons,
            &piece.clearance_sidewalk_surface_polygons,
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
            &piece.sidewalk_surface_polygons,
            chunk,
            terrain,
            height_offset_m,
        );
    }

    fn node_piece_uses_earthworks(
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

    fn earthwork_section_ranges_for_edge(
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

        let total_length = sections.last()?.s_m.max(0.0);
        let start_handoff = Self::visual_start_handoff_m(edge, total_length);
        let end_handoff = Self::visual_end_handoff_s_m(edge, total_length);
        if end_handoff - start_handoff <= SAMPLE_EPSILON_M {
            return None;
        }

        let start_index = sections
            .iter()
            .position(|section| section.s_m + SAMPLE_EPSILON_M >= start_handoff)
            .unwrap_or(0);
        let end_index = sections
            .iter()
            .rposition(|section| section.s_m - SAMPLE_EPSILON_M <= end_handoff)
            .unwrap_or(sections.len().saturating_sub(1));
        (end_index > start_index).then_some((start_index, end_index))
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

    fn tunnel_visible_section_ranges(
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

    fn tunnel_throat_is_visible(
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
        sidewalk_surface_polygons: &[RoadSurfaceVisualPolygon],
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
        height_offset_m: f32,
    ) {
        self.stamp_piece_surface_geometry_for_chunk(
            road_surface_polygons,
            chunk,
            terrain,
            height_offset_m,
        );
        self.stamp_piece_surface_geometry_for_chunk(
            sidewalk_surface_polygons,
            chunk,
            terrain,
            height_offset_m,
        );
    }

    fn stamp_piece_surface_geometry_for_chunk(
        &self,
        polygons: &[RoadSurfaceVisualPolygon],
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
        height_offset_m: f32,
    ) {
        let conservative_margin_m = terrain.cell_size_m() * std::f32::consts::SQRT_2 * 0.5;
        let mut candidates: HashMap<(usize, usize), (f32, f32)> = HashMap::new();

        for polygon in polygons {
            Self::visit_visual_polygon_triangles(polygon, &mut |triangle| {
                self.collect_profile_clearance_triangle_candidates(
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

    fn collect_profile_clearance_triangle_candidates(
        &self,
        terrain: &TerrainSystem,
        chunk: SurfaceChunkKey,
        triangle: [Vector3; 3],
        conservative_margin_m: f32,
        height_offset_m: f32,
        candidates: &mut HashMap<(usize, usize), (f32, f32)>,
    ) {
        let projected_cross = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
            - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
        if projected_cross.abs() <= 0.002 {
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
                    Self::profile_clearance_candidate_from_triangle(
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
                if distance_squared < entry.0 - 0.0001
                    || ((distance_squared - entry.0).abs() <= 0.0001 && height_sample > entry.1)
                {
                    *entry = (distance_squared, height_sample);
                }
            }
        }
    }

    fn profile_clearance_candidate_from_triangle(
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

    fn make_visual_polygon(mut points_world: Vec<Vector3>) -> Option<RoadSurfaceVisualPolygon> {
        points_world.dedup_by(|a, b| (*a - *b).length_squared() <= 0.0001);
        if points_world.len() >= 2
            && (points_world.first().copied()? - points_world.last().copied()?).length_squared()
                <= 0.0001
        {
            points_world.pop();
        }
        if Self::polygon_has_strict_edge_crossing_xz(&points_world) {
            return None;
        }
        let signed_area = Self::signed_polygon_area_xz(&points_world);
        if signed_area.abs() <= 0.002 {
            return None;
        }
        if signed_area < 0.0 {
            points_world.reverse();
        }
        let Some((start_index, _)) = points_world.iter().enumerate().min_by(|(_, a), (_, b)| {
            a.x.total_cmp(&b.x)
                .then(a.z.total_cmp(&b.z))
                .then(a.y.total_cmp(&b.y))
        }) else {
            return None;
        };
        points_world.rotate_left(start_index);
        let triangles_world = Self::triangulate_constrained_polygon_xz(&points_world)?;
        Some(RoadSurfaceVisualPolygon {
            points_world,
            triangles_world,
        })
    }

    fn make_visual_strip_polygon(
        mut points_world: Vec<Vector3>,
    ) -> Option<RoadSurfaceVisualPolygon> {
        points_world.dedup_by(|a, b| (*a - *b).length_squared() <= 0.0001);
        if points_world.len() >= 2
            && (points_world.first().copied()? - points_world.last().copied()?).length_squared()
                <= 0.0001
        {
            points_world.pop();
        }
        if points_world.len() < 3 {
            return None;
        }
        if Self::polygon_has_strict_edge_crossing_xz(&points_world) {
            return None;
        }
        let triangles_world = Self::triangulate_fan_polygon_xz(&points_world)?;
        Some(RoadSurfaceVisualPolygon {
            points_world,
            triangles_world,
        })
    }

    fn triangulate_fan_polygon_xz(points_world: &[Vector3]) -> Option<Vec<[Vector3; 3]>> {
        if points_world.len() < 3 {
            return None;
        }
        let anchor = points_world[0];
        let mut triangles = Vec::with_capacity(points_world.len().saturating_sub(2));
        for index in 1..points_world.len() - 1 {
            let triangle = [anchor, points_world[index], points_world[index + 1]];
            if Self::triangle_has_area_xz(triangle) {
                triangles.push(triangle);
            }
        }
        (!triangles.is_empty()).then_some(triangles)
    }

    fn triangulate_constrained_polygon_xz(points_world: &[Vector3]) -> Option<Vec<[Vector3; 3]>> {
        if points_world.len() < 3 {
            return None;
        }
        if points_world.len() == 3 {
            let triangle = [points_world[0], points_world[1], points_world[2]];
            return Self::triangle_has_area_xz(triangle).then_some(vec![triangle]);
        }

        let vertices = points_world
            .iter()
            .map(|point| Point2::new(f64::from(point.x), f64::from(point.z)))
            .collect::<Vec<_>>();
        let constraints = (0..points_world.len())
            .map(|index| [index, (index + 1) % points_world.len()])
            .collect::<Vec<_>>();
        let mut invalid_constraints = 0usize;
        let cdt = SurfaceCdt::try_bulk_load_cdt(vertices, constraints, |_| {
            invalid_constraints += 1;
        })
        .ok()?;
        if invalid_constraints > 0 {
            return None;
        }

        let mut triangles = Vec::new();
        for face in cdt.inner_faces() {
            let [a, b, c] = face.vertices();
            let triangle = [
                points_world[a.fix().index()],
                points_world[b.fix().index()],
                points_world[c.fix().index()],
            ];
            let centroid = Vector2::new(
                (triangle[0].x + triangle[1].x + triangle[2].x) / 3.0,
                (triangle[0].z + triangle[1].z + triangle[2].z) / 3.0,
            );
            if Self::triangle_has_area_xz(triangle)
                && Self::polygon_contains_point_xz(points_world, centroid)
            {
                triangles.push(triangle);
            }
        }

        (!triangles.is_empty()).then_some(triangles)
    }

    fn polygon_contains_point_xz(points_world: &[Vector3], point: Vector2) -> bool {
        if points_world.len() < 3 {
            return false;
        }
        let mut inside = false;
        for index in 0..points_world.len() {
            let start = points_world[index];
            let end = points_world[(index + 1) % points_world.len()];
            if Self::point_segment_distance_squared_xz(point, start, end) <= 0.0001 {
                return true;
            }
            let start_z = start.z;
            let end_z = end.z;
            if (start_z > point.y) != (end_z > point.y) {
                let edge_x_at_point_z =
                    (end.x - start.x) * (point.y - start_z) / (end_z - start_z) + start.x;
                if point.x < edge_x_at_point_z {
                    inside = !inside;
                }
            }
        }
        inside
    }

    fn point_segment_distance_squared_xz(point: Vector2, start: Vector3, end: Vector3) -> f32 {
        let start_xz = Vector2::new(start.x, start.z);
        let end_xz = Vector2::new(end.x, end.z);
        let segment = end_xz - start_xz;
        let length_squared = segment.length_squared();
        if length_squared <= SAMPLE_EPSILON_M {
            return point.distance_squared_to(start_xz);
        }
        let t = ((point - start_xz).dot(segment) / length_squared).clamp(0.0, 1.0);
        point.distance_squared_to(start_xz + segment * t)
    }

    fn signed_polygon_area_xz(points: &[Vector3]) -> f32 {
        if points.len() < 3 {
            return 0.0;
        }
        let mut signed_area = 0.0;
        for index in 0..points.len() {
            let current = points[index];
            let next = points[(index + 1) % points.len()];
            signed_area += current.x * next.z - next.x * current.z;
        }
        signed_area * 0.5
    }

    fn polygon_has_strict_edge_crossing_xz(points: &[Vector3]) -> bool {
        if points.len() < 4 {
            return false;
        }

        for edge_a in 0..points.len() {
            let edge_a_next = (edge_a + 1) % points.len();
            for edge_b in edge_a + 1..points.len() {
                let edge_b_next = (edge_b + 1) % points.len();
                if edge_a == edge_b
                    || edge_a == edge_b_next
                    || edge_a_next == edge_b
                    || edge_a_next == edge_b_next
                {
                    continue;
                }
                if Self::segments_strictly_intersect_xz(
                    points[edge_a],
                    points[edge_a_next],
                    points[edge_b],
                    points[edge_b_next],
                ) {
                    return true;
                }
            }
        }

        false
    }

    fn segments_strictly_intersect_xz(a: Vector3, b: Vector3, c: Vector3, d: Vector3) -> bool {
        let ab_c = Self::cross_points_xz(a, b, c);
        let ab_d = Self::cross_points_xz(a, b, d);
        let cd_a = Self::cross_points_xz(c, d, a);
        let cd_b = Self::cross_points_xz(c, d, b);
        ab_c * ab_d < -SAMPLE_EPSILON_M && cd_a * cd_b < -SAMPLE_EPSILON_M
    }

    fn cross_points_xz(a: Vector3, b: Vector3, c: Vector3) -> f32 {
        (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)
    }

    #[cfg(test)]
    fn polygon_has_area_xz(points: &[Vector3]) -> bool {
        Self::signed_polygon_area_xz(points).abs() > 0.002
    }

    fn triangle_has_area_xz(triangle: [Vector3; 3]) -> bool {
        let projected_cross = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
            - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
        let edge_ab = Vector2::new(triangle[1].x - triangle[0].x, triangle[1].z - triangle[0].z);
        let edge_bc = Vector2::new(triangle[2].x - triangle[1].x, triangle[2].z - triangle[1].z);
        let edge_ca = Vector2::new(triangle[0].x - triangle[2].x, triangle[0].z - triangle[2].z);
        let max_edge_m = edge_ab.length().max(edge_bc.length()).max(edge_ca.length());
        projected_cross.abs() > 0.002
            && projected_cross.abs() / max_edge_m.max(SAMPLE_EPSILON_M)
                >= SURFACE_MIN_TRIANGLE_ALTITUDE_M
    }

    fn triangle_barycentric_weights_xz(
        triangle: [Vector3; 3],
        point: Vector2,
    ) -> Option<(f32, f32, f32)> {
        let area = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
            - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
        if area.abs() <= SAMPLE_EPSILON_M {
            return None;
        }

        let w0 = ((triangle[1].x - point.x) * (triangle[2].z - point.y)
            - (triangle[1].z - point.y) * (triangle[2].x - point.x))
            / area;
        let w1 = ((triangle[2].x - point.x) * (triangle[0].z - point.y)
            - (triangle[2].z - point.y) * (triangle[0].x - point.x))
            / area;
        let w2 = 1.0 - w0 - w1;
        let epsilon = 0.001;
        if w0 < -epsilon || w1 < -epsilon || w2 < -epsilon {
            return None;
        }
        Some((w0, w1, w2))
    }

    fn point_is_inside_or_near_triangle_xz(
        triangle: [Vector3; 3],
        point: Vector2,
        margin_m: f32,
    ) -> bool {
        if Self::triangle_barycentric_weights_xz(triangle, point).is_some() {
            return true;
        }
        Self::distance_point_to_triangle_xz(triangle, point) <= margin_m
    }

    fn closest_point_on_triangle_xz(triangle: [Vector3; 3], point: Vector2) -> Vector2 {
        if Self::triangle_barycentric_weights_xz(triangle, point).is_some() {
            return point;
        }

        let triangle_points = [
            Vector2::new(triangle[0].x, triangle[0].z),
            Vector2::new(triangle[1].x, triangle[1].z),
            Vector2::new(triangle[2].x, triangle[2].z),
        ];
        let mut best = triangle_points[0];
        let mut best_distance_squared = point.distance_squared_to(best);

        for &(start, end) in &[
            (triangle_points[0], triangle_points[1]),
            (triangle_points[1], triangle_points[2]),
            (triangle_points[2], triangle_points[0]),
        ] {
            let candidate = Self::closest_point_on_segment_xz(point, start, end);
            let distance_squared = point.distance_squared_to(candidate);
            if distance_squared < best_distance_squared {
                best = candidate;
                best_distance_squared = distance_squared;
            }
        }

        best
    }

    fn distance_point_to_triangle_xz(triangle: [Vector3; 3], point: Vector2) -> f32 {
        Self::distance_point_to_segment_xz(
            point,
            Vector2::new(triangle[0].x, triangle[0].z),
            Vector2::new(triangle[1].x, triangle[1].z),
        )
        .min(Self::distance_point_to_segment_xz(
            point,
            Vector2::new(triangle[1].x, triangle[1].z),
            Vector2::new(triangle[2].x, triangle[2].z),
        ))
        .min(Self::distance_point_to_segment_xz(
            point,
            Vector2::new(triangle[2].x, triangle[2].z),
            Vector2::new(triangle[0].x, triangle[0].z),
        ))
    }

    fn distance_point_to_segment_xz(point: Vector2, start: Vector2, end: Vector2) -> f32 {
        point.distance_to(Self::closest_point_on_segment_xz(point, start, end))
    }

    fn closest_point_on_segment_xz(point: Vector2, start: Vector2, end: Vector2) -> Vector2 {
        let segment = end - start;
        let length_squared = segment.length_squared();
        if length_squared <= SAMPLE_EPSILON_M {
            return start;
        }
        let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
        start + segment * t
    }

    fn ray_triangle_intersection_t(
        triangle: [Vector3; 3],
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<f32> {
        let edge_ab = triangle[1] - triangle[0];
        let edge_ac = triangle[2] - triangle[0];
        let pvec = ray_dir.cross(edge_ac);
        let det = edge_ab.dot(pvec);
        if det.abs() <= SAMPLE_EPSILON_M {
            return None;
        }

        let inv_det = 1.0 / det;
        let tvec = ray_origin - triangle[0];
        let u = tvec.dot(pvec) * inv_det;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }

        let qvec = tvec.cross(edge_ab);
        let v = ray_dir.dot(qvec) * inv_det;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }

        let t = edge_ac.dot(qvec) * inv_det;
        (t >= 0.0).then_some(t)
    }

    fn section_boundary_world_point(
        &self,
        section: &RoadSurfaceSection,
        lateral_offset_m: f32,
        height_m: f32,
    ) -> Vector3 {
        Self::section_boundary_world_point_static(section, lateral_offset_m, height_m)
    }

    fn section_boundary_world_point_static(
        section: &RoadSurfaceSection,
        lateral_offset_m: f32,
        height_m: f32,
    ) -> Vector3 {
        Vector3::new(
            section.center_xz.x + section.lateral_xz.x * lateral_offset_m,
            height_m,
            section.center_xz.y + section.lateral_xz.y * lateral_offset_m,
        )
    }

    fn append_edge_geometry_debug_dump(
        &self,
        dump: &mut String,
        _graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_idx: usize,
        edge: &Edge,
    ) {
        let sections = self
            .compiled_sections
            .get(&edge_idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let surface_chunks = self
            .surface_span_chunks
            .get(&edge_idx)
            .cloned()
            .unwrap_or_default();
        let earthwork_chunks = self
            .earthwork_span_chunks
            .get(&edge_idx)
            .cloned()
            .unwrap_or_default();

        let _ = writeln!(dump, "    {{");
        let _ = writeln!(dump, "      \"edge_idx\": {edge_idx},");
        let _ = writeln!(dump, "      \"start_node\": {},", edge.start_node);
        let _ = writeln!(dump, "      \"end_node\": {},", edge.end_node);
        let _ = writeln!(dump, "      \"class\": \"{:?}\",", edge.class);
        let _ = writeln!(dump, "      \"primary_type\": \"{:?}\",", edge.primary_type);
        let _ = writeln!(dump, "      \"width_m\": {:.3},", edge.width);
        let _ = writeln!(dump, "      \"fwd_lanes\": {},", edge.fwd_lanes);
        let _ = writeln!(dump, "      \"bkw_lanes\": {},", edge.bkw_lanes);
        let _ = writeln!(
            dump,
            "      \"physical_length_m\": {:.3},",
            edge.physical_length
        );
        let _ = writeln!(dump, "      \"start_clip_m\": {:.3},", edge.start_clip);
        let _ = writeln!(dump, "      \"end_clip_m\": {:.3},", edge.end_clip);
        dump.push_str("      \"surface_chunks\": ");
        Self::append_chunk_key_list_literal(dump, &surface_chunks);
        dump.push_str(",\n");
        dump.push_str("      \"earthwork_chunks\": ");
        Self::append_chunk_key_list_literal(dump, &earthwork_chunks);
        dump.push_str(",\n");
        dump.push_str("      \"geometry_world\": ");
        Self::append_vector3_list_literal(dump, &edge.geometry);
        dump.push_str(",\n");
        dump.push_str("      \"physical_geometry_world\": ");
        Self::append_vector3_list_literal(dump, &edge.physical_geometry);
        dump.push_str(",\n");
        let _ = writeln!(dump, "      \"sections\": [");

        for (section_index, section) in sections.iter().enumerate() {
            if section_index > 0 {
                let _ = writeln!(dump, ",");
            }
            self.append_section_geometry_debug_dump(dump, terrain, section);
        }

        let _ = writeln!(dump);
        let _ = writeln!(dump, "      ]");
        let _ = write!(dump, "    }}");
    }

    fn append_section_geometry_debug_dump(
        &self,
        dump: &mut String,
        terrain: &TerrainSystem,
        section: &RoadSurfaceSection,
    ) {
        let center_world = Vector3::new(
            section.center_xz.x,
            section.center_height_m,
            section.center_xz.y,
        );
        let source_center_y_m = terrain
            .sample_height_world(section.center_xz.x, section.center_xz.y)
            * config::HEIGHT_SCALE;
        let visual_center_y_m = terrain
            .sample_visual_height_world(section.center_xz.x, section.center_xz.y)
            * config::HEIGHT_SCALE;

        let _ = writeln!(dump, "        {{");
        let _ = writeln!(dump, "          \"s_m\": {:.3},", section.s_m);
        dump.push_str("          \"center_world\": ");
        Self::append_vector3_literal(dump, center_world);
        dump.push_str(",\n");
        dump.push_str("          \"tangent_xz\": ");
        Self::append_vector2_literal(dump, section.tangent_xz);
        dump.push_str(",\n");
        dump.push_str("          \"lateral_xz\": ");
        Self::append_vector2_literal(dump, section.lateral_xz);
        dump.push_str(",\n");
        let _ = writeln!(
            dump,
            "          \"source_center_y_m\": {:.3},",
            source_center_y_m
        );
        let _ = writeln!(
            dump,
            "          \"visual_center_y_m\": {:.3},",
            visual_center_y_m
        );

        if let (Some(first_band), Some(last_band)) = (section.bands.first(), section.bands.last()) {
            let left_road = self.section_boundary_world_point(
                section,
                first_band.lateral_start_m,
                first_band.height_start_m,
            );
            let right_road = self.section_boundary_world_point(
                section,
                last_band.lateral_end_m,
                last_band.height_end_m,
            );
            let left_outer =
                self.earthwork_transition_point(left_road, section.lateral_xz * -1.0, terrain);
            let right_outer =
                self.earthwork_transition_point(right_road, section.lateral_xz, terrain);

            dump.push_str("          \"left_road_edge\": ");
            Self::append_surface_sample_literal(dump, terrain, left_road);
            dump.push_str(",\n");
            dump.push_str("          \"right_road_edge\": ");
            Self::append_surface_sample_literal(dump, terrain, right_road);
            dump.push_str(",\n");
            dump.push_str("          \"left_outer_margin\": ");
            Self::append_surface_sample_literal(dump, terrain, left_outer);
            dump.push_str(",\n");
            dump.push_str("          \"right_outer_margin\": ");
            Self::append_surface_sample_literal(dump, terrain, right_outer);
            dump.push_str(",\n");
        }

        let _ = writeln!(dump, "          \"bands\": [");
        for (band_index, band) in section.bands.iter().enumerate() {
            if band_index > 0 {
                let _ = writeln!(dump, ",");
            }
            let _ = write!(
                dump,
                "            {{\"kind\":\"{:?}\",\"lateral_start_m\":{:.3},\"lateral_end_m\":{:.3},\"height_start_m\":{:.3},\"height_end_m\":{:.3}}}",
                band.kind,
                band.lateral_start_m,
                band.lateral_end_m,
                band.height_start_m,
                band.height_end_m
            );
        }
        let _ = writeln!(dump);
        let _ = writeln!(dump, "          ]");
        let _ = write!(dump, "        }}");
    }

    fn append_surface_sample_literal(dump: &mut String, terrain: &TerrainSystem, point: Vector3) {
        let source_y_m = terrain.sample_height_world(point.x, point.z) * config::HEIGHT_SCALE;
        let visual_y_m =
            terrain.sample_visual_height_world(point.x, point.z) * config::HEIGHT_SCALE;
        dump.push('{');
        dump.push_str("\"world\":");
        Self::append_vector3_literal(dump, point);
        let _ = write!(
            dump,
            ",\"source_terrain_y_m\":{:.3},\"visual_terrain_y_m\":{:.3}",
            source_y_m, visual_y_m
        );
        dump.push('}');
    }

    fn append_vector3_list_literal(dump: &mut String, points: &[Vector3]) {
        dump.push('[');
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            Self::append_vector3_literal(dump, *point);
        }
        dump.push(']');
    }

    fn append_chunk_key_list_literal(dump: &mut String, chunks: &[SurfaceChunkKey]) {
        dump.push('[');
        for (index, chunk) in chunks.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(dump, "[{}, {}]", chunk.0, chunk.1);
        }
        dump.push(']');
    }

    fn append_vector3_literal(dump: &mut String, point: Vector3) {
        let _ = write!(dump, "[{:.3}, {:.3}, {:.3}]", point.x, point.y, point.z);
    }

    fn append_vector2_literal(dump: &mut String, point: Vector2) {
        let _ = write!(dump, "[{:.3}, {:.3}]", point.x, point.y);
    }

    fn prune_stale_cache_entries(&mut self, graph: &RegionGraph) {
        let stale_span_ids: Vec<usize> = self
            .surface_span_chunks
            .keys()
            .chain(self.earthwork_span_chunks.keys())
            .chain(self.compiled_visual_span_pieces.keys())
            .copied()
            .filter(|edge_idx| {
                *edge_idx >= graph.edge_count() || !Self::is_surface_edge(graph.edge(*edge_idx))
            })
            .collect();
        for edge_idx in stale_span_ids {
            self.remove_span_piece_coverage(edge_idx);
            self.compiled_visual_span_pieces.remove(&edge_idx);
            self.compiled_sections.remove(&edge_idx);
        }

        let stale_node_ids: Vec<u32> = self
            .surface_node_chunks
            .keys()
            .chain(self.earthwork_node_chunks.keys())
            .chain(self.compiled_visual_node_pieces.keys())
            .copied()
            .filter(|node_id| (*node_id as usize) >= graph.node_count())
            .collect();
        for node_id in stale_node_ids {
            self.remove_node_piece_coverage(node_id);
            self.compiled_visual_node_pieces.remove(&node_id);
        }

        self.compiled_sections.retain(|edge_idx, _| {
            *edge_idx < graph.edge_count() && Self::is_surface_edge(graph.edge(*edge_idx))
        });
        self.compiled_visual_span_pieces.retain(|edge_idx, _| {
            *edge_idx < graph.edge_count() && Self::is_surface_edge(graph.edge(*edge_idx))
        });
        self.compiled_visual_node_pieces
            .retain(|node_id, _| (*node_id as usize) < graph.node_count());
        self.surface_chunk_cache
            .retain(|_, entry| !entry.edge_indices.is_empty() || !entry.node_ids.is_empty());
        self.earthwork_chunk_cache
            .retain(|_, entry| !entry.edge_indices.is_empty() || !entry.node_ids.is_empty());
    }

    fn all_surface_edge_ids(&self, graph: &RegionGraph) -> Vec<usize> {
        graph
            .edges()
            .iter()
            .enumerate()
            .filter_map(|(edge_idx, edge)| Self::is_surface_edge(edge).then_some(edge_idx))
            .collect()
    }

    fn all_surface_node_ids(&self, graph: &RegionGraph) -> Vec<u32> {
        let mut node_ids = HashSet::new();
        for edge in graph.edges() {
            if !Self::is_surface_edge(edge) {
                continue;
            }
            node_ids.insert(graph.get_valid_node(edge.start_node));
            node_ids.insert(graph.get_valid_node(edge.end_node));
        }
        let mut node_ids: Vec<u32> = node_ids.into_iter().collect();
        node_ids.sort_unstable();
        node_ids
    }

    fn collect_all_chunks(&self, kind: ChunkCacheKind) -> Vec<SurfaceChunkKey> {
        let mut chunks = HashSet::new();
        match kind {
            ChunkCacheKind::Surface => {
                chunks.extend(self.surface_chunk_spans.keys().copied());
                chunks.extend(self.surface_chunk_nodes.keys().copied());
            }
            ChunkCacheKind::Earthwork => {
                chunks.extend(self.earthwork_chunk_spans.keys().copied());
                chunks.extend(self.earthwork_chunk_nodes.keys().copied());
            }
        }
        self.sorted_chunk_keys(&chunks)
    }

    fn visual_node_piece_bounds(
        &self,
        piece: &RoadSurfaceVisualNodePiece,
        kind: ChunkCacheKind,
    ) -> Option<(Vector3, Vector3)> {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_z = f32::MAX;
        let mut max_z = f32::MIN;
        let mut saw_point = false;

        match kind {
            ChunkCacheKind::Surface => {
                for point in piece
                    .outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(point.x);
                    max_x = max_x.max(point.x);
                    min_z = min_z.min(point.z);
                    max_z = max_z.max(point.z);
                    saw_point = true;
                }
            }
            ChunkCacheKind::Earthwork => {
                for point in piece
                    .earthwork_outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(point.x);
                    max_x = max_x.max(point.x);
                    min_z = min_z.min(point.z);
                    max_z = max_z.max(point.z);
                    saw_point = true;
                }
                if !saw_point {
                    for point in piece
                        .earthwork_surface_polygons
                        .iter()
                        .flat_map(|polygon| polygon.points_world.iter())
                    {
                        min_x = min_x.min(point.x);
                        max_x = max_x.max(point.x);
                        min_z = min_z.min(point.z);
                        max_z = max_z.max(point.z);
                        saw_point = true;
                    }
                }
            }
        }

        saw_point.then_some((
            Vector3::new(min_x, 0.0, min_z),
            Vector3::new(max_x, 0.0, max_z),
        ))
    }

    fn visual_span_piece_bounds(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        kind: ChunkCacheKind,
    ) -> Option<(Vector3, Vector3)> {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_z = f32::MAX;
        let mut max_z = f32::MIN;
        let mut saw_point = false;

        match kind {
            ChunkCacheKind::Surface => {
                for point in piece
                    .outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(point.x);
                    max_x = max_x.max(point.x);
                    min_z = min_z.min(point.z);
                    max_z = max_z.max(point.z);
                    saw_point = true;
                }
            }
            ChunkCacheKind::Earthwork => {
                for point in piece
                    .earthwork_outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(point.x);
                    max_x = max_x.max(point.x);
                    min_z = min_z.min(point.z);
                    max_z = max_z.max(point.z);
                    saw_point = true;
                }
                if !saw_point {
                    for point in piece
                        .earthwork_surface_polygons
                        .iter()
                        .flat_map(|polygon| polygon.points_world.iter())
                    {
                        min_x = min_x.min(point.x);
                        max_x = max_x.max(point.x);
                        min_z = min_z.min(point.z);
                        max_z = max_z.max(point.z);
                        saw_point = true;
                    }
                }
            }
        }

        saw_point.then_some((
            Vector3::new(min_x, 0.0, min_z),
            Vector3::new(max_x, 0.0, max_z),
        ))
    }

    fn sorted_chunk_keys(&self, chunks: &HashSet<SurfaceChunkKey>) -> Vec<SurfaceChunkKey> {
        let mut chunks: Vec<SurfaceChunkKey> = chunks.iter().copied().collect();
        chunks.sort_unstable();
        chunks
    }

    fn canonical_chunk_vec(mut chunks: Vec<SurfaceChunkKey>) -> Vec<SurfaceChunkKey> {
        chunks.sort_unstable();
        chunks.dedup();
        chunks
    }

    fn node_has_surface_edges(&self, graph: &RegionGraph, node_id: u32) -> bool {
        (node_id as usize) < graph.node_adjacency_count()
            && graph.node_adjacency(node_id).iter().any(|&edge_idx| {
                edge_idx < graph.edge_count() && Self::is_surface_edge(graph.edge(edge_idx))
            })
    }

    fn node_has_standard_surface_edges(&self, graph: &RegionGraph, node_id: u32) -> bool {
        (node_id as usize) < graph.node_adjacency_count()
            && graph.node_adjacency(node_id).iter().any(|&edge_idx| {
                if edge_idx >= graph.edge_count() {
                    return false;
                }
                let edge = graph.edge(edge_idx);
                Self::is_surface_edge(edge) && edge.class == EdgeClass::Standard
            })
    }

    fn collect_terrain_clip_polygons_from_piece(
        source: &[RoadSurfaceVisualPolygon],
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        out: &mut Vec<RoadSurfaceVisualPolygon>,
    ) {
        for polygon in source {
            if Self::visual_polygon_overlaps_bounds_xz(polygon, min_x, min_z, max_x, max_z) {
                out.push(polygon.clone());
            }
        }
    }

    fn visual_polygon_overlaps_bounds_xz(
        polygon: &RoadSurfaceVisualPolygon,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> bool {
        let mut polygon_min_x = f32::MAX;
        let mut polygon_max_x = f32::MIN;
        let mut polygon_min_z = f32::MAX;
        let mut polygon_max_z = f32::MIN;
        for point in &polygon.points_world {
            polygon_min_x = polygon_min_x.min(point.x);
            polygon_max_x = polygon_max_x.max(point.x);
            polygon_min_z = polygon_min_z.min(point.z);
            polygon_max_z = polygon_max_z.max(point.z);
        }

        polygon_min_x <= max_x
            && polygon_max_x >= min_x
            && polygon_min_z <= max_z
            && polygon_max_z >= min_z
    }

    fn is_surface_edge(edge: &Edge) -> bool {
        !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
    }

    fn edge_points<'a>(&self, edge: &'a Edge) -> &'a [Vector3] {
        if edge.physical_geometry.is_empty() {
            &edge.geometry
        } else {
            &edge.physical_geometry
        }
    }

    fn chunk_coords_for_world(&self, world_x: f32, world_z: f32) -> SurfaceChunkKey {
        (
            (world_x / self.chunk_span_m).floor() as i32,
            (world_z / self.chunk_span_m).floor() as i32,
        )
    }

    fn chunk_bounds(&self, chunk: SurfaceChunkKey) -> (Vector3, Vector3) {
        let min_x = chunk.0 as f32 * self.chunk_span_m;
        let min_z = chunk.1 as f32 * self.chunk_span_m;
        let max_x = min_x + self.chunk_span_m;
        let max_z = min_z + self.chunk_span_m;
        (
            Vector3::new(min_x, 0.0, min_z),
            Vector3::new(max_x, 0.0, max_z),
        )
    }

    fn bounds_to_chunk_keys(&self, min: Vector3, max: Vector3) -> Vec<SurfaceChunkKey> {
        let min_chunk = self.chunk_coords_for_world(min.x, min.z);
        let max_chunk = self.chunk_coords_for_world(max.x, max.z);
        let mut chunks = Vec::new();
        for cx in min_chunk.0..=max_chunk.0 {
            for cz in min_chunk.1..=max_chunk.1 {
                chunks.push((cx, cz));
            }
        }
        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChunkCacheKind, CURB_STEP_HEIGHT_M, EARTHWORK_MAX_MARGIN_M, PreviewRoadSurfaceResult,
        RoadSurfaceEarthworkFaceKind, RoadSurfaceSection, RoadSurfaceSystem,
        RoadSurfaceVisualNodePiece, RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon,
        SAMPLE_EPSILON_M, SurfaceChunkKey,
    };
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };
    use crate::simulation::terrain::TerrainSystem;
    use crate::simulation::terrain::cdt::{
        TerrainCdtInput, TerrainCdtPatch, TerrainCdtRoadLoop, TerrainCdtVertex,
        build_road_touched_terrain_patch,
    };
    use godot::prelude::{Vector2, Vector3};

    fn test_edge(
        start_node: u32,
        end_node: u32,
        points: Vec<Vector3>,
        width: f32,
        class: EdgeClass,
        primary_type: TransitType,
        allowed_types: u8,
    ) -> Edge {
        let length = points
            .windows(2)
            .map(|segment| segment[0].distance_to(segment[1]))
            .sum();
        Edge {
            start_node,
            end_node,
            primary_type,
            allowed_types,
            class,
            width,
            fwd_lanes: if (allowed_types & TransitFlags::CAR) != 0 {
                ((width / crate::config::LANE_WIDTH).round() as u8).max(1)
            } else {
                0
            },
            bkw_lanes: if (allowed_types & TransitFlags::CAR) != 0 {
                ((width / crate::config::LANE_WIDTH).round() as u8).max(1)
            } else {
                0
            },
            speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: length,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: points.clone(),
            physical_geometry: points,
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        }
    }

    fn flat_terrain(width: usize, height: usize) -> TerrainSystem {
        TerrainSystem::with_chunking(width, height, 1.0, 8, 0.0)
    }

    fn sloped_terrain(width: usize, height: usize) -> TerrainSystem {
        let mut terrain = TerrainSystem::with_chunking(width, height, 1.0, 8, 0.0);
        for z in 0..height {
            for x in 0..width {
                terrain.set_height(x, z, x as f32 * 0.05);
            }
        }
        terrain
    }

    fn ridge_terrain(width: usize, height: usize) -> TerrainSystem {
        let mut terrain = TerrainSystem::with_chunking(width, height, 1.0, 8, 0.0);
        let center_x = (width as f32 - 1.0) * 0.5;
        for z in 0..height {
            for x in 0..width {
                let dx = x as f32 - center_x;
                let ridge = (1.0 - (dx.abs() / 12.0).min(1.0)) * 6.0;
                terrain.set_height(x, z, ridge.max(0.0));
            }
        }
        terrain
    }

    #[allow(dead_code)]
    fn planar_world_terrain(
        width: usize,
        height: usize,
        cell_size_m: f32,
        base_height_m: f32,
        slope_x_m_per_m: f32,
        slope_z_m_per_m: f32,
    ) -> TerrainSystem {
        let mut terrain = TerrainSystem::with_chunking(width, height, cell_size_m, 8, 0.0);
        for z in 0..height {
            for x in 0..width {
                let (world_x, world_z) = terrain.grid_to_world_coords(x, z);
                let height_m =
                    base_height_m + world_x * slope_x_m_per_m + world_z * slope_z_m_per_m;
                terrain.set_height(x, z, height_m / crate::config::HEIGHT_SCALE);
            }
        }
        terrain
    }

    fn coarse_hillside_world_terrain(
        width: usize,
        height: usize,
        cell_size_m: f32,
    ) -> TerrainSystem {
        let mut terrain = TerrainSystem::with_chunking(width, height, cell_size_m, 8, 0.0);
        for z in 0..height {
            for x in 0..width {
                let (world_x, world_z) = terrain.grid_to_world_coords(x, z);
                let ridge_dx = world_x + 45.0;
                let ridge = 8.0 * (-(ridge_dx * ridge_dx) / (2.0 * 55.0 * 55.0)).exp();
                let shoulder_dx = world_x - world_z * 0.12 + 25.0;
                let shoulder = 4.0 * (-(shoulder_dx * shoulder_dx) / (2.0 * 85.0 * 85.0)).exp();
                let height_m = 150.0 + world_x * 0.06 - world_z * 0.012 + ridge + shoulder;
                terrain.set_height(x, z, height_m / crate::config::HEIGHT_SCALE);
            }
        }
        terrain
    }

    fn grounded_polyline_points_from_terrain(
        terrain: &TerrainSystem,
        start_xz: Vector2,
        end_xz: Vector2,
        segments: usize,
    ) -> Vec<Vector3> {
        let segments = segments.max(1);
        (0..=segments)
            .map(|idx| {
                let t = idx as f32 / segments as f32;
                let world_x = start_xz.x + (end_xz.x - start_xz.x) * t;
                let world_z = start_xz.y + (end_xz.y - start_xz.y) * t;
                let world_y =
                    terrain.sample_height_world(world_x, world_z) * crate::config::HEIGHT_SCALE;
                Vector3::new(world_x, world_y, world_z)
            })
            .collect()
    }

    #[allow(dead_code)]
    #[derive(Clone, Copy, Debug)]
    struct FootprintOverflowMetrics {
        max_overflow_m: f32,
        section_s_m: f32,
        lateral_offset_m: f32,
        road_height_m: f32,
        visual_height_m: f32,
    }

    fn footprint_sample_offsets(section: &RoadSurfaceSection) -> Vec<f32> {
        let mut offsets = Vec::new();
        for band in &section.bands {
            if !matches!(
                band.kind,
                super::RoadSurfaceBandKind::Carriageway
                    | super::RoadSurfaceBandKind::CurbOrShoulder
                    | super::RoadSurfaceBandKind::Sidewalk
                    | super::RoadSurfaceBandKind::Footpath
            ) {
                continue;
            }
            offsets.push(band.lateral_start_m);
            offsets.push((band.lateral_start_m + band.lateral_end_m) * 0.5);
            offsets.push(band.lateral_end_m);
        }
        offsets.sort_by(|a, b| a.total_cmp(b));
        offsets.dedup_by(|a, b| (*a - *b).abs() <= 0.001);
        offsets
    }

    fn measure_max_footprint_overflow(
        surface: &RoadSurfaceSystem,
        graph: &RegionGraph,
        edge_idx: usize,
        terrain: &TerrainSystem,
    ) -> FootprintOverflowMetrics {
        let mut best = FootprintOverflowMetrics {
            max_overflow_m: f32::NEG_INFINITY,
            section_s_m: 0.0,
            lateral_offset_m: 0.0,
            road_height_m: 0.0,
            visual_height_m: 0.0,
        };

        let sections = surface.compiled_sections().get(&edge_idx).unwrap();
        for section in sections {
            for lateral_offset_m in footprint_sample_offsets(section) {
                let Some(road_height_m) =
                    section_height_at_lateral_offset(section, lateral_offset_m)
                else {
                    continue;
                };
                let sample_x = section.center_xz.x + section.lateral_xz.x * lateral_offset_m;
                let sample_z = section.center_xz.y + section.lateral_xz.y * lateral_offset_m;
                let visual_height_m = surface
                    .sample_paved_support_height(graph, terrain, sample_x, sample_z)
                    .unwrap_or_else(|| {
                        terrain.sample_visual_height_world(sample_x, sample_z)
                            * crate::config::HEIGHT_SCALE
                    });
                let overflow_m = visual_height_m - road_height_m;
                if overflow_m > best.max_overflow_m {
                    best = FootprintOverflowMetrics {
                        max_overflow_m: overflow_m,
                        section_s_m: section.s_m,
                        lateral_offset_m,
                        road_height_m,
                        visual_height_m,
                    };
                }
            }
        }

        best
    }

    fn build_coarse_grid_hillside_case(
        cell_size_m: f32,
    ) -> (RoadSurfaceSystem, TerrainSystem, RegionGraph, usize) {
        let cells = ((800.0 / cell_size_m).round() as usize).max(2) + 1;
        let mut terrain = coarse_hillside_world_terrain(cells, cells, cell_size_m);
        let points = grounded_polyline_points_from_terrain(
            &terrain,
            Vector2::new(120.0, 40.0),
            Vector2::new(-180.0, -220.0),
            24,
        );

        let mut graph = RegionGraph::new();
        let start = graph.add_node(points[0], NodeType::Junction);
        let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            points,
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(128.0);
        surface.rebuild_all_earthworks(&graph, &mut terrain);
        (surface, terrain, graph, edge_idx)
    }

    fn compile_committed_preview_reference(
        surface: &RoadSurfaceSystem,
        raw_points: &[Vector3],
        terrain: &TerrainSystem,
        fwd_lanes: u8,
        bkw_lanes: u8,
    ) -> (
        PreviewRoadSurfaceResult,
        Vec<RoadSurfaceSection>,
        Vec<RoadSurfaceVisualNodePiece>,
    ) {
        let preview = surface.compile_preview_surface(raw_points, fwd_lanes, bkw_lanes, terrain);
        if preview.prepared_points.len() < 2 {
            return (preview, Vec::new(), Vec::new());
        }

        let mut graph = RegionGraph::new();
        let start_node = graph.add_node(preview.prepared_points[0], NodeType::Junction);
        let end_node = graph.add_node(*preview.prepared_points.last().unwrap(), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start_node,
            end_node,
            preview.prepared_points.clone(),
            ((fwd_lanes + bkw_lanes) as f32 * crate::config::LANE_WIDTH).max(2.0),
            preview.edge_class,
            if fwd_lanes == 0 && bkw_lanes == 0 {
                TransitType::Foot
            } else {
                TransitType::Road
            },
            if fwd_lanes == 0 && bkw_lanes == 0 {
                TransitFlags::FOOT
            } else {
                TransitFlags::CAR | TransitFlags::FOOT
            },
        ));

        let mut committed = RoadSurfaceSystem::new(surface.chunk_span_m());
        committed.compile_dirty(&graph, terrain);
        let compiled_sections = committed
            .compiled_sections()
            .get(&edge_idx)
            .cloned()
            .unwrap_or_default();
        let compiled_visual_node_pieces = [start_node, end_node]
            .into_iter()
            .filter_map(|node_id| {
                committed
                    .compiled_visual_node_pieces()
                    .get(&node_id)
                    .cloned()
            })
            .collect();
        (preview, compiled_sections, compiled_visual_node_pieces)
    }

    fn triangle_centroid_xz(triangle: [Vector3; 3]) -> Vector2 {
        Vector2::new(
            (triangle[0].x + triangle[1].x + triangle[2].x) / 3.0,
            (triangle[0].z + triangle[1].z + triangle[2].z) / 3.0,
        )
    }

    fn point_inside_visual_polygons(polygons: &[RoadSurfaceVisualPolygon], point: Vector2) -> bool {
        polygons.iter().any(|polygon| {
            polygon.triangles_world.iter().any(|&triangle| {
                RoadSurfaceSystem::triangle_barycentric_weights_xz(triangle, point).is_some()
            })
        })
    }

    fn assert_material_triangle_centroids_do_not_overlap(piece: &RoadSurfaceVisualNodePiece) {
        for sidewalk_polygon in &piece.sidewalk_surface_polygons {
            for &sidewalk_triangle in &sidewalk_polygon.triangles_world {
                let sidewalk_centroid = triangle_centroid_xz(sidewalk_triangle);
                for road_polygon in &piece.road_surface_polygons {
                    for &road_triangle in &road_polygon.triangles_world {
                        assert!(
                            RoadSurfaceSystem::triangle_barycentric_weights_xz(
                                road_triangle,
                                sidewalk_centroid,
                            )
                            .is_none(),
                            "sidewalk triangle centroid must not be owned by asphalt after overlay difference; centroid={sidewalk_centroid:?} sidewalk_triangle={sidewalk_triangle:?} road_triangle={road_triangle:?}"
                        );
                    }
                }
            }
        }

        for road_polygon in &piece.road_surface_polygons {
            for &road_triangle in &road_polygon.triangles_world {
                let road_centroid = triangle_centroid_xz(road_triangle);
                for sidewalk_polygon in &piece.sidewalk_surface_polygons {
                    for &sidewalk_triangle in &sidewalk_polygon.triangles_world {
                        assert!(
                            RoadSurfaceSystem::triangle_barycentric_weights_xz(
                                sidewalk_triangle,
                                road_centroid,
                            )
                            .is_none(),
                            "asphalt triangle centroid must not be owned by sidewalk after overlay difference"
                        );
                    }
                }
            }
        }
    }

    fn polygon_area_m2(polygon: &RoadSurfaceVisualPolygon) -> f32 {
        RoadSurfaceSystem::signed_polygon_area_xz(&polygon.points_world).abs()
    }

    fn polygon_triangle_area_m2(polygon: &RoadSurfaceVisualPolygon) -> f32 {
        polygon
            .triangles_world
            .iter()
            .map(|triangle| {
                RoadSurfaceSystem::signed_polygon_area_xz(&[triangle[0], triangle[1], triangle[2]])
                    .abs()
            })
            .sum()
    }

    #[test]
    fn hill_crossing_input_stays_standard_instead_of_auto_tunnel() {
        let terrain = ridge_terrain(97, 33);
        let raw_points = vec![
            Vector3::new(
                -20.0,
                terrain.sample_height_world(-20.0, 0.0) * crate::config::HEIGHT_SCALE,
                0.0,
            ),
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(
                20.0,
                terrain.sample_height_world(20.0, 0.0) * crate::config::HEIGHT_SCALE,
                0.0,
            ),
        ];

        let (grounded_points, class) =
            RoadSurfaceSystem::classify_and_ground_road_points(&raw_points, &terrain);

        assert_eq!(class, EdgeClass::Standard);
        for point in grounded_points {
            let terrain_y =
                terrain.sample_height_world(point.x, point.z) * crate::config::HEIGHT_SCALE;
            assert!(
                (point.y - terrain_y).abs() <= 0.001,
                "standard grounding should snap to terrain at x={:.2}: point_y={:.3} terrain_y={:.3}",
                point.x,
                point.y,
                terrain_y
            );
        }
    }

    #[test]
    fn uniformly_submerged_input_stays_auto_tunnel() {
        let terrain = flat_terrain(65, 33);
        let raw_points = vec![
            Vector3::new(-10.0, -2.5, 0.0),
            Vector3::new(0.0, -2.5, 0.0),
            Vector3::new(10.0, -2.5, 0.0),
        ];

        let (_points, class) =
            RoadSurfaceSystem::classify_and_ground_road_points(&raw_points, &terrain);
        assert_eq!(class, EdgeClass::Tunnel);
    }

    #[test]
    fn uniformly_elevated_input_stays_auto_bridge() {
        let terrain = flat_terrain(65, 33);
        let raw_points = vec![
            Vector3::new(-10.0, 2.5, 0.0),
            Vector3::new(0.0, 2.5, 0.0),
            Vector3::new(10.0, 2.5, 0.0),
        ];

        let (_points, class) =
            RoadSurfaceSystem::classify_and_ground_road_points(&raw_points, &terrain);
        assert_eq!(class, EdgeClass::Bridge);
    }

    fn section_height_at_lateral_offset(
        section: &RoadSurfaceSection,
        lateral_offset_m: f32,
    ) -> Option<f32> {
        let mut best_height_m: Option<f32> = None;
        for band in &section.bands {
            let start = band.lateral_start_m.min(band.lateral_end_m);
            let end = band.lateral_start_m.max(band.lateral_end_m);
            if lateral_offset_m < start - 0.001 || lateral_offset_m > end + 0.001 {
                continue;
            }

            let span = band.lateral_end_m - band.lateral_start_m;
            let t = if span.abs() <= 0.001 {
                0.0
            } else {
                ((lateral_offset_m - band.lateral_start_m) / span).clamp(0.0, 1.0)
            };
            let height_m = band.height_start_m + (band.height_end_m - band.height_start_m) * t;
            best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
        }

        best_height_m
    }

    fn outer_surface_lateral_bounds(section: &RoadSurfaceSection) -> Option<(f32, f32)> {
        Some((
            section.bands.first()?.lateral_start_m,
            section.bands.last()?.lateral_end_m,
        ))
    }

    #[test]
    fn mark_edge_dirty_tracks_edge_without_centerline_chunk_guess() {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(5.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            n0,
            n1,
            vec![Vector3::new(5.0, 0.0, 0.0), Vector3::new(25.0, 0.0, 0.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(10.0);
        surface.mark_edge_dirty(&graph, edge_idx);

        assert!(surface.dirty_edges().contains(&edge_idx));
        assert!(surface.dirty_surface_chunks().is_empty());
        assert!(surface.dirty_terrain_chunks().is_empty());
    }

    #[test]
    fn terrain_edit_marks_nearby_edges_nodes_and_chunks() {
        let mut graph = RegionGraph::new();
        let near_a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let near_b = graph.add_node(Vector3::new(8.0, 0.0, 0.0), NodeType::Junction);
        let far_a = graph.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
        let far_b = graph.add_node(Vector3::new(60.0, 0.0, 0.0), NodeType::Junction);
        let near_edge = graph.add_edge(test_edge(
            near_a,
            near_b,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(8.0, 0.0, 0.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        let far_edge = graph.add_edge(test_edge(
            far_a,
            far_b,
            vec![Vector3::new(50.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(10.0);
        surface.mark_terrain_edit_dirty(&graph, Vector2::new(4.0, 0.0), 5.0);

        assert!(surface.dirty_edges().contains(&near_edge));
        assert!(!surface.dirty_edges().contains(&far_edge));
        assert!(surface.dirty_nodes().contains(&near_a));
        assert!(surface.dirty_nodes().contains(&near_b));
        assert!(!surface.dirty_nodes().contains(&far_a));
        assert!(!surface.dirty_nodes().contains(&far_b));
        assert!(surface.dirty_surface_chunks().contains(&(-1, -1)));
        assert!(surface.dirty_surface_chunks().contains(&(0, 0)));
        assert_eq!(
            surface.dirty_surface_chunks(),
            surface.dirty_terrain_chunks()
        );
    }

    #[test]
    fn section_refinement_is_deterministic() {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            n0,
            n1,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let terrain = flat_terrain(64, 64);
        let mut surface_a = RoadSurfaceSystem::new(16.0);
        let mut surface_b = RoadSurfaceSystem::new(16.0);
        surface_a.compile_dirty(&graph, &terrain);
        surface_b.compile_dirty(&graph, &terrain);

        let sections_a = surface_a.compiled_sections().get(&edge_idx).unwrap();
        let sections_b = surface_b.compiled_sections().get(&edge_idx).unwrap();
        assert_eq!(sections_a, sections_b);
        let s_values: Vec<f32> = sections_a.iter().map(|section| section.s_m).collect();
        assert_eq!(s_values, vec![0.0, 8.0, 16.0, 20.0]);
    }

    #[test]
    fn standard_edge_sections_follow_solved_edge_profile_deterministically() {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(-16.0, 99.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(16.0, 99.0, 0.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            n0,
            n1,
            vec![
                Vector3::new(-16.0, 99.0, 0.0),
                Vector3::new(16.0, 99.0, 0.0),
            ],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let terrain = sloped_terrain(33, 9);
        let mut surface_a = RoadSurfaceSystem::new(16.0);
        let mut surface_b = RoadSurfaceSystem::new(16.0);
        surface_a.compile_dirty(&graph, &terrain);
        surface_b.compile_dirty(&graph, &terrain);

        let sections_a = surface_a.compiled_sections().get(&edge_idx).unwrap();
        let sections_b = surface_b.compiled_sections().get(&edge_idx).unwrap();
        assert_eq!(sections_a, sections_b);
        for section in sections_a {
            let expected = 99.0;
            assert!((section.center_height_m - expected).abs() <= 0.001);
        }
    }

    #[test]
    fn node_piece_classification_matches_surface_profiles() {
        let terrain = flat_terrain(64, 64);

        let mut pass_graph = RegionGraph::new();
        let pa = pass_graph.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
        let pb = pass_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let pc = pass_graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        pass_graph.add_edge(test_edge(
            pa,
            pb,
            vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        pass_graph.add_edge(test_edge(
            pb,
            pc,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        let mut pass_surface = RoadSurfaceSystem::new(16.0);
        pass_surface.compile_dirty(&pass_graph, &terrain);
        assert!(
            pass_surface
                .compiled_visual_node_pieces()
                .get(&pb)
                .is_none()
        );

        let mut width_graph = RegionGraph::new();
        let wa = width_graph.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
        let wb = width_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let wc = width_graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        width_graph.add_edge(test_edge(
            wa,
            wb,
            vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        width_graph.add_edge(test_edge(
            wb,
            wc,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            14.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        let mut width_surface = RoadSurfaceSystem::new(16.0);
        width_surface.compile_dirty(&width_graph, &terrain);
        assert!(
            width_surface
                .compiled_visual_node_pieces()
                .get(&wb)
                .is_none()
        );

        let mut junction_graph = RegionGraph::new();
        let ja = junction_graph.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
        let jb = junction_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let jc = junction_graph.add_node(Vector3::new(0.0, 0.0, 10.0), NodeType::Junction);
        junction_graph.add_edge(test_edge(
            ja,
            jb,
            vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        junction_graph.add_edge(test_edge(
            jb,
            jc,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 10.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        let mut junction_surface = RoadSurfaceSystem::new(16.0);
        junction_surface.compile_dirty(&junction_graph, &terrain);
        assert_eq!(
            junction_surface
                .compiled_visual_node_pieces()
                .get(&jb)
                .unwrap()
                .kind,
            RoadSurfaceVisualNodePieceKind::Bend
        );

        let mut terminal_graph = RegionGraph::new();
        let ta = terminal_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let tb = terminal_graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        terminal_graph.add_edge(test_edge(
            ta,
            tb,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        let mut terminal_surface = RoadSurfaceSystem::new(16.0);
        terminal_surface.compile_dirty(&terminal_graph, &terrain);
        assert_eq!(
            terminal_surface
                .compiled_visual_node_pieces()
                .get(&ta)
                .unwrap()
                .kind,
            RoadSurfaceVisualNodePieceKind::Terminal
        );
    }

    #[test]
    fn bend_and_terminal_visual_pieces_compile_explicit_band_polygons() {
        let terrain = flat_terrain(64, 64);

        let mut bend_graph = RegionGraph::new();
        let bend_center = bend_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let bend_a = bend_graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let bend_b = bend_graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);
        bend_graph.add_edge(test_edge(
            bend_center,
            bend_a,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        bend_graph.add_edge(test_edge(
            bend_center,
            bend_b,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 20.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        bend_graph.rebuild_intersection_clips();
        let mut bend_surface = RoadSurfaceSystem::new(16.0);
        bend_surface.compile_dirty(&bend_graph, &terrain);
        let bend_piece = bend_surface
            .compiled_visual_node_pieces()
            .get(&bend_center)
            .unwrap();
        assert_eq!(bend_piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
        assert!(!bend_piece.outer_boundary_loops.is_empty());
        assert!(!bend_piece.road_surface_polygons.is_empty());
        assert!(!bend_piece.sidewalk_surface_polygons.is_empty());
        assert!(
            bend_piece
                .outer_boundary_loops
                .iter()
                .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
        );
        assert!(
            bend_piece
                .road_surface_polygons
                .iter()
                .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
        );
        assert!(
            bend_piece
                .sidewalk_surface_polygons
                .iter()
                .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
        );
        assert!(
            point_inside_visual_polygons(&bend_piece.outer_boundary_loops, Vector2::new(3.0, 3.0)),
            "bend footprint must close the local round join between the two incident roadbeds"
        );
        assert!(
            point_inside_visual_polygons(
                &bend_piece.road_surface_polygons,
                Vector2::new(2.25, 2.25)
            ),
            "bend asphalt must close its own local join instead of leaving a road-surface gap"
        );
        assert!(
            bend_piece
                .outer_boundary_loops
                .iter()
                .any(|polygon| polygon.points_world.len() >= 12),
            "bend footprint should retain deterministic arc vertices instead of collapsing to only straight corridor corners"
        );
        assert!(!bend_piece.earthwork_surface_polygons.is_empty());
        assert!(!bend_piece.earthwork_outer_boundary_loops.is_empty());
        assert!(!bend_piece.render_earthwork_faces.is_empty());
        assert!(
            bend_piece
                .earthwork_surface_polygons
                .iter()
                .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
        );
        assert!(
            bend_piece
                .render_earthwork_faces
                .iter()
                .all(|face| RoadSurfaceSystem::polygon_has_area_xz(&face.polygon.points_world))
        );
        assert_ne!(
            bend_piece.earthwork_outer_boundary_loops,
            bend_piece.outer_boundary_loops
        );

        let mut terminal_graph = RegionGraph::new();
        let terminal_center =
            terminal_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let terminal_end =
            terminal_graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        terminal_graph.add_edge(test_edge(
            terminal_center,
            terminal_end,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        let mut terminal_surface = RoadSurfaceSystem::new(16.0);
        terminal_surface.compile_dirty(&terminal_graph, &terrain);
        let terminal_piece = terminal_surface
            .compiled_visual_node_pieces()
            .get(&terminal_center)
            .unwrap();
        assert_eq!(
            terminal_piece.kind,
            RoadSurfaceVisualNodePieceKind::Terminal
        );
        assert_eq!(terminal_piece.outer_boundary_loops.len(), 1);
        assert!(!terminal_piece.road_surface_polygons.is_empty());
        assert!(!terminal_piece.sidewalk_surface_polygons.is_empty());
        assert!(
            terminal_piece
                .road_surface_polygons
                .iter()
                .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
        );
        assert!(
            terminal_piece
                .sidewalk_surface_polygons
                .iter()
                .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
        );
        assert!(!terminal_piece.earthwork_surface_polygons.is_empty());
        assert!(!terminal_piece.earthwork_outer_boundary_loops.is_empty());
        assert!(!terminal_piece.render_earthwork_faces.is_empty());
        assert!(
            terminal_piece
                .earthwork_surface_polygons
                .iter()
                .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
        );
        assert!(
            terminal_piece
                .render_earthwork_faces
                .iter()
                .all(|face| RoadSurfaceSystem::polygon_has_area_xz(&face.polygon.points_world))
        );
        assert_ne!(
            terminal_piece.earthwork_outer_boundary_loops,
            terminal_piece.outer_boundary_loops
        );
    }

    #[test]
    fn span_visual_pieces_compile_explicit_band_polygons() {
        let terrain = flat_terrain(64, 64);
        let mut graph = RegionGraph::new();
        let a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let b = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            a,
            b,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);
        let span_piece = surface
            .compiled_visual_span_pieces()
            .get(&edge_idx)
            .unwrap();
        assert!(!span_piece.outer_boundary_loops.is_empty());
        assert!(!span_piece.road_surface_polygons.is_empty());
        assert!(!span_piece.sidewalk_surface_polygons.is_empty());
        assert!(
            span_piece
                .road_surface_polygons
                .iter()
                .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
        );
        assert!(
            span_piece
                .sidewalk_surface_polygons
                .iter()
                .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
        );
        assert!(!span_piece.earthwork_surface_polygons.is_empty());
        assert!(!span_piece.earthwork_outer_boundary_loops.is_empty());
        assert!(!span_piece.render_earthwork_faces.is_empty());
        assert!(
            span_piece
                .earthwork_surface_polygons
                .iter()
                .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
        );
        assert!(
            span_piece
                .render_earthwork_faces
                .iter()
                .all(|face| RoadSurfaceSystem::polygon_has_area_xz(&face.polygon.points_world))
        );
        assert_ne!(
            span_piece.earthwork_outer_boundary_loops,
            span_piece.outer_boundary_loops
        );
    }

    #[test]
    fn span_earthwork_outer_loops_stay_outside_paved_footprint() {
        let terrain = flat_terrain(97, 97);
        let mut graph = RegionGraph::new();
        let a = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
        let b = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            a,
            b,
            vec![Vector3::new(0.0, 0.0, -24.0), Vector3::new(0.0, 0.0, 24.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);
        let span_piece = surface
            .compiled_visual_span_pieces()
            .get(&edge_idx)
            .expect("standard edge should compile a visual span piece");
        let max_inner_abs_x = span_piece
            .outer_boundary_loops
            .iter()
            .flat_map(|polygon| polygon.points_world.iter())
            .map(|point| point.x.abs())
            .fold(0.0, f32::max);
        let min_outer_abs_x = span_piece
            .earthwork_outer_boundary_loops
            .iter()
            .flat_map(|polygon| polygon.points_world.iter())
            .map(|point| point.x.abs())
            .fold(f32::INFINITY, f32::min);
        assert!(
            min_outer_abs_x >= max_inner_abs_x + 0.5,
            "expected span earthwork tie-in to stay outside the paved footprint, got min_outer_abs_x={min_outer_abs_x:.3} max_inner_abs_x={max_inner_abs_x:.3}"
        );
    }

    #[test]
    fn terrain_clip_polygons_include_standard_grounded_footprints() {
        let terrain = flat_terrain(97, 97);
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
        graph.add_edge(test_edge(
            start,
            end,
            vec![Vector3::new(0.0, 0.0, -24.0), Vector3::new(0.0, 0.0, 24.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);

        let clip_polygons =
            surface.terrain_clip_polygons_for_world_bounds(&graph, -16.0, -32.0, 16.0, 32.0);

        assert!(
            !clip_polygons.is_empty(),
            "expected grounded standard road footprint polygons to clip terrain topology"
        );
        assert!(
            clip_polygons
                .iter()
                .flat_map(|polygon| polygon.points_world.iter())
                .any(|point| point.x.abs() > 5.0),
            "expected terrain clip polygons to include the full sidewalk / shoulder footprint"
        );
        assert!(
            clip_polygons
                .iter()
                .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world)),
            "expected every terrain clip cutter to be a valid road footprint polygon"
        );
        let expected_outer_boundary_loop_count: usize = surface
            .compiled_visual_span_pieces()
            .values()
            .map(|piece| piece.outer_boundary_loops.len())
            .sum::<usize>()
            + surface
                .compiled_visual_node_pieces()
                .values()
                .map(|piece| piece.outer_boundary_loops.len())
                .sum::<usize>();
        assert!(
            clip_polygons.len() <= expected_outer_boundary_loop_count,
            "expected terrain clip cutters to be the boolean-unioned piece footprint, got {} cutters for {} raw outer loops",
            clip_polygons.len(),
            expected_outer_boundary_loop_count
        );
    }

    #[test]
    fn terrain_clip_polygons_are_unioned_before_cdt_for_arbitrary_multiway_nodes() {
        let terrain = flat_terrain(257, 257);
        let mut graph = RegionGraph::new();
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        for angle_degrees in [0.0_f32, 23.0, 61.0, 137.0, 211.0, 304.0] {
            let angle = angle_degrees.to_radians();
            let endpoint = Vector3::new(angle.cos() * 64.0, 0.0, angle.sin() * 64.0);
            let node = graph.add_node(endpoint, NodeType::Junction);
            graph.add_edge(test_edge(
                center,
                node,
                vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
                14.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        }
        graph.rebuild_adjacency_list();

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);

        let clip_polygons =
            surface.terrain_clip_polygons_for_world_bounds(&graph, -96.0, -96.0, 96.0, 96.0);
        assert!(
            !clip_polygons.is_empty(),
            "expected arbitrary multiway node to produce terrain clip polygons"
        );

        let road_loops = clip_polygons
            .iter()
            .enumerate()
            .map(|(index, polygon)| {
                TerrainCdtRoadLoop::new(
                    index as u64,
                    0,
                    polygon
                        .points_world
                        .iter()
                        .map(|point| {
                            TerrainCdtVertex::new(f64::from(point.x), point.y, f64::from(point.z))
                        })
                        .collect(),
                )
            })
            .collect();
        let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
            TerrainCdtPatch::new(-96.0, -96.0, 96.0, 96.0, [0.0; 4]),
            road_loops,
            Vec::new(),
        ))
        .expect("unioned terrain clip footprint must be accepted by the terrain CDT");

        assert_eq!(
            mesh.stats.invalid_constraint_edges, 0,
            "terrain CDT must not see crossing constraints from arbitrary-angle piece loops"
        );
    }

    #[test]
    fn road_locked_terrain_patches_are_bounded_to_visible_footprint() {
        let terrain = flat_terrain(257, 257);
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(0.0, 0.0, -48.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(0.0, 0.0, 48.0), NodeType::Junction);
        graph.add_edge(test_edge(
            start,
            end,
            vec![Vector3::new(0.0, 0.0, -48.0), Vector3::new(0.0, 0.0, 48.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);

        let mut footprint_min_x = f32::MAX;
        let mut footprint_max_x = f32::MIN;
        let mut footprint_min_z = f32::MAX;
        let mut footprint_max_z = f32::MIN;
        for point in surface
            .compiled_visual_span_pieces()
            .values()
            .flat_map(|piece| piece.outer_boundary_loops.iter())
            .chain(
                surface
                    .compiled_visual_node_pieces()
                    .values()
                    .flat_map(|piece| piece.outer_boundary_loops.iter()),
            )
            .flat_map(|polygon| polygon.points_world.iter())
        {
            footprint_min_x = footprint_min_x.min(point.x);
            footprint_max_x = footprint_max_x.max(point.x);
            footprint_min_z = footprint_min_z.min(point.z);
            footprint_max_z = footprint_max_z.max(point.z);
        }

        let keys = surface.terrain_render_patch_keys_with_visible_road(&terrain);
        assert!(!keys.is_empty());
        assert!(
            keys.len() < terrain.render_patch_cols() * terrain.render_patch_rows() / 8,
            "road-locked render patches must stay local to the visible road footprint"
        );
        for (patch_x, patch_z) in keys {
            let patch = terrain.visual_patch_snapshot(patch_x, patch_z).unwrap();
            let patch_max_x = patch.world_origin_x + patch.world_size_x;
            let patch_max_z = patch.world_origin_z + patch.world_size_z;
            assert!(
                patch.world_origin_x <= footprint_max_x
                    && patch_max_x >= footprint_min_x
                    && patch.world_origin_z <= footprint_max_z
                    && patch_max_z >= footprint_min_z,
                "road-locked patch ({patch_x}, {patch_z}) must overlap the road footprint, not only the earthwork envelope"
            );
        }
    }

    #[test]
    fn terrain_clip_polygons_skip_bridge_midspans() {
        let terrain = flat_terrain(97, 97);
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
        graph.add_edge(test_edge(
            start,
            end,
            vec![Vector3::new(0.0, 8.0, -24.0), Vector3::new(0.0, 8.0, 24.0)],
            10.0,
            EdgeClass::Bridge,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);

        let clip_polygons =
            surface.terrain_clip_polygons_for_world_bounds(&graph, -16.0, -32.0, 16.0, 32.0);

        assert!(
            clip_polygons.is_empty(),
            "bridge midspans must not cut terrain topology like grounded standard roads"
        );
    }

    #[test]
    fn earthwork_face_classification_distinguishes_slopes_from_walls() {
        assert_eq!(
            RoadSurfaceSystem::classify_earthwork_face_kind(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1.0, 0.0, 0.0),
                Vector3::new(2.0, 0.5, 0.0),
                Vector3::new(1.0, 0.5, 0.0),
            ),
            RoadSurfaceEarthworkFaceKind::Slope
        );
        assert_eq!(
            RoadSurfaceSystem::classify_earthwork_face_kind(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1.0, 0.0, 0.0),
                Vector3::new(1.1, 3.0, 0.0),
                Vector3::new(0.1, 3.0, 0.0),
            ),
            RoadSurfaceEarthworkFaceKind::RetainingWall
        );
    }

    #[test]
    fn visual_node_pieces_are_deterministic_for_multi_arm_nodes() {
        let mut graph = RegionGraph::new();
        let left = graph.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let right = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let up = graph.add_node(Vector3::new(0.0, 0.0, 10.0), NodeType::Junction);
        graph.add_edge(test_edge(
            left,
            center,
            vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.add_edge(test_edge(
            center,
            right,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.add_edge(test_edge(
            center,
            up,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 10.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.rebuild_adjacency_list();

        let terrain = flat_terrain(64, 64);
        let mut surface_a = RoadSurfaceSystem::new(16.0);
        let mut surface_b = RoadSurfaceSystem::new(16.0);
        surface_a.compile_dirty(&graph, &terrain);
        surface_b.compile_dirty(&graph, &terrain);

        let piece_a = surface_a
            .compiled_visual_node_pieces()
            .get(&center)
            .unwrap();
        let piece_b = surface_b
            .compiled_visual_node_pieces()
            .get(&center)
            .unwrap();
        assert_eq!(piece_a.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
        assert_eq!(piece_a, piece_b);
        assert!(
            !piece_a.outer_boundary_loops.is_empty(),
            "expected explicit visual node pieces to expose deterministic outer boundaries"
        );
        assert!(
            !piece_a.road_surface_polygons.is_empty(),
            "expected explicit JunctionN builder to emit road-owned polygons"
        );
        assert!(
            !piece_a.sidewalk_surface_polygons.is_empty(),
            "expected explicit JunctionN builder to emit overlay-owned sidewalk polygons"
        );
        assert_material_triangle_centroids_do_not_overlap(piece_a);
    }

    #[test]
    fn oblique_t_junction_compiles_solid_cdt_owned_surface() {
        let mut graph = RegionGraph::new();
        let left = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let right = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        let oblique = graph.add_node(Vector3::new(12.0, 0.0, 20.784609), NodeType::Junction);
        graph.add_edge(test_edge(
            left,
            center,
            vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.add_edge(test_edge(
            center,
            right,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.add_edge(test_edge(
            center,
            oblique,
            vec![
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(12.0, 0.0, 20.784609),
            ],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.rebuild_adjacency_list();

        let terrain = flat_terrain(96, 96);
        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);

        let piece = surface
            .compiled_visual_node_pieces()
            .get(&center)
            .expect("60-degree T junction must compile an explicit JunctionN piece");
        assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
        assert!(!piece.outer_boundary_loops.is_empty());
        assert!(!piece.road_surface_polygons.is_empty());
        assert!(!piece.sidewalk_surface_polygons.is_empty());
        assert!(
            piece
                .road_surface_polygons
                .iter()
                .chain(piece.sidewalk_surface_polygons.iter())
                .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world)),
            "overlay-owned JunctionN polygons must be non-degenerate"
        );
        assert_material_triangle_centroids_do_not_overlap(piece);
    }

    #[test]
    fn arbitrary_six_way_junction_keeps_visible_ownership_disjoint() {
        let mut graph = RegionGraph::new();
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        for angle_degrees in [0.0_f32, 23.0, 61.0, 137.0, 211.0, 304.0] {
            let angle = angle_degrees.to_radians();
            let endpoint = Vector3::new(angle.cos() * 96.0, 0.0, angle.sin() * 96.0);
            let node = graph.add_node(endpoint, NodeType::Junction);
            graph.add_edge(test_edge(
                center,
                node,
                vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
                7.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        }
        graph.rebuild_intersection_clips();

        let terrain = flat_terrain(192, 192);
        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);

        let piece = surface
            .compiled_visual_node_pieces()
            .get(&center)
            .expect("arbitrary six-way node must compile one JunctionN piece");
        assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
        assert!(!piece.outer_boundary_loops.is_empty());
        assert!(!piece.road_surface_polygons.is_empty());
        assert!(!piece.sidewalk_surface_polygons.is_empty());

        let footprint_area: f32 = piece.outer_boundary_loops.iter().map(polygon_area_m2).sum();
        let asphalt_area: f32 = piece
            .road_surface_polygons
            .iter()
            .map(polygon_triangle_area_m2)
            .sum();
        let non_road_area: f32 = piece
            .sidewalk_surface_polygons
            .iter()
            .map(polygon_triangle_area_m2)
            .sum();
        assert!(
            (footprint_area - asphalt_area - non_road_area).abs() <= 0.1,
            "arbitrary JunctionN ownership must close the footprint without overlapping materials; footprint={footprint_area:.3} asphalt={asphalt_area:.3} non_road={non_road_area:.3}"
        );
        assert_material_triangle_centroids_do_not_overlap(piece);
    }

    #[test]
    fn arbitrary_five_way_junction_uses_conflict_bounded_footprint() {
        let mut graph = RegionGraph::new();
        let center_pos = Vector3::new(2.668, 0.0, 10.799);
        let center = graph.add_node(center_pos, NodeType::Junction);
        for endpoint in [
            Vector3::new(-58.540, 0.0, 6.220),
            Vector3::new(115.507, 0.0, 19.240),
            Vector3::new(96.186, 0.0, 60.070),
            Vector3::new(35.647, 0.0, -130.899),
            Vector3::new(-27.212, 0.0, 50.632),
        ] {
            let node = graph.add_node(endpoint, NodeType::Junction);
            graph.add_edge(test_edge(
                center,
                node,
                vec![center_pos, endpoint],
                7.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        }
        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();

        let terrain = flat_terrain(256, 256);
        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);

        let piece = surface
            .compiled_visual_node_pieces()
            .get(&center)
            .expect("arbitrary five-way node must compile one conflict-bounded JunctionN piece");
        assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);

        let max_expected_radius = graph
            .node_adjacency(center)
            .iter()
            .map(|&edge_idx| {
                let edge = graph.edge(edge_idx);
                let clip = if graph.get_valid_node(edge.start_node) == center {
                    edge.start_clip
                } else {
                    edge.end_clip
                };
                clip.max(RoadSurfaceSystem::visual_node_handoff_limit_m(edge))
                    + RoadSurfaceSystem::visual_roadbed_half_width_m(edge)
                    + 0.25
            })
            .fold(0.0_f32, f32::max);
        for point in piece
            .outer_boundary_loops
            .iter()
            .flat_map(|polygon| polygon.points_world.iter())
        {
            let radius = Vector2::new(point.x - center_pos.x, point.z - center_pos.z).length();
            assert!(
                radius <= max_expected_radius,
                "visual JunctionN footprint must stay inside the conflict-bounded handoff; point={point:?} radius={radius:.3} max={max_expected_radius:.3}"
            );
        }
        assert_material_triangle_centroids_do_not_overlap(piece);
    }

    #[test]
    fn dirty_node_recompile_refreshes_incident_span_sections_for_new_junction() {
        let mut graph = RegionGraph::new();
        let left = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let right = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        let left_edge = graph.add_edge(test_edge(
            left,
            center,
            vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.add_edge(test_edge(
            center,
            right,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();

        let terrain = flat_terrain(96, 96);
        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);

        let up = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
        let up_edge = graph.add_edge(test_edge(
            center,
            up,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 24.0)],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();

        surface.mark_node_dirty(&graph, center);
        surface.mark_node_dirty(&graph, up);
        surface.mark_edge_dirty(&graph, up_edge);
        surface.compile_dirty(&graph, &terrain);

        let edge = graph.edge(left_edge);
        let total_length: f32 = edge
            .geometry
            .windows(2)
            .map(|pair| pair[0].distance_to(pair[1]))
            .sum();
        let expected_handoff_s = RoadSurfaceSystem::visual_end_handoff_s_m(edge, total_length);
        let sections = surface.compiled_sections().get(&left_edge).unwrap();
        assert!(
            sections
                .iter()
                .any(|section| (section.s_m - expected_handoff_s).abs() <= SAMPLE_EPSILON_M),
            "dirty node recompilation must refresh incident span sections at the new visual handoff; expected_s={expected_handoff_s:.3} sections={:?}",
            sections
                .iter()
                .map(|section| section.s_m)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn dirty_recompile_marks_chunks_for_expanded_arbitrary_node_piece() {
        let terrain = flat_terrain(192, 192);
        let mut graph = RegionGraph::new();
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        for angle_degrees in [35.0_f32, 158.0, 276.0] {
            let angle = angle_degrees.to_radians();
            let endpoint = Vector3::new(angle.cos() * 88.0, 0.0, angle.sin() * 88.0);
            let node = graph.add_node(endpoint, NodeType::Junction);
            graph.add_edge(test_edge(
                center,
                node,
                vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
                7.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        }
        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();

        let mut surface = RoadSurfaceSystem::new(4.0);
        surface.compile_dirty(&graph, &terrain);

        let angle = 318.0_f32.to_radians();
        let endpoint = Vector3::new(angle.cos() * 88.0, 0.0, angle.sin() * 88.0);
        let new_node = graph.add_node(endpoint, NodeType::Junction);
        let new_edge = graph.add_edge(test_edge(
            center,
            new_node,
            vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();

        surface.mark_node_dirty(&graph, center);
        surface.mark_node_dirty(&graph, new_node);
        for &edge_idx in graph.node_adjacency(center) {
            surface.mark_edge_dirty(&graph, edge_idx);
        }
        surface.mark_edge_dirty(&graph, new_edge);
        surface.compile_dirty(&graph, &terrain);

        let piece = surface
            .compiled_visual_node_pieces()
            .get(&center)
            .expect("expanded arbitrary junction must have a compiled node piece");
        let (min, max) = surface
            .visual_node_piece_bounds(piece, ChunkCacheKind::Surface)
            .expect("expanded arbitrary junction must have surface bounds");

        for chunk in surface.bounds_to_chunk_keys(min, max) {
            let entry = surface
                .surface_chunk_cache
                .get(&chunk)
                .unwrap_or_else(|| panic!("expected rebuilt surface chunk {chunk:?}"));
            assert!(
                entry.node_ids.contains(&center),
                "surface chunk {chunk:?} must include the expanded junction node piece"
            );
        }
    }

    #[test]
    fn dirty_recompile_removes_node_from_old_chunks_after_topology_shrink() {
        let terrain = flat_terrain(192, 192);
        let mut graph = RegionGraph::new();
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let west = graph.add_node(Vector3::new(-64.0, 0.0, 0.0), NodeType::Junction);
        let east = graph.add_node(Vector3::new(64.0, 0.0, 0.0), NodeType::Junction);
        let north = graph.add_node(Vector3::new(0.0, 0.0, 64.0), NodeType::Junction);
        graph.add_edge(test_edge(
            west,
            center,
            vec![Vector3::new(-64.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.add_edge(test_edge(
            center,
            east,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(64.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        let removed_edge = graph.add_edge(test_edge(
            center,
            north,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 64.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();

        let mut surface = RoadSurfaceSystem::new(2.0);
        surface.compile_dirty(&graph, &terrain);
        let old_node_chunks = surface
            .surface_node_chunks
            .get(&center)
            .expect("three-way node must own chunks before shrink")
            .clone();
        assert!(
            old_node_chunks.len() > 1,
            "test requires node coverage wide enough to prove stale chunk removal"
        );

        graph.edges[removed_edge].deleted = true;
        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();
        surface.mark_edge_dirty(&graph, removed_edge);
        surface.mark_node_dirty(&graph, center);
        surface.compile_dirty(&graph, &terrain);

        let new_node_chunks = surface
            .surface_node_chunks
            .get(&center)
            .cloned()
            .unwrap_or_default();
        let removed_chunks: Vec<SurfaceChunkKey> = old_node_chunks
            .into_iter()
            .filter(|chunk| !new_node_chunks.contains(chunk))
            .collect();
        assert!(
            !removed_chunks.is_empty(),
            "topology shrink must remove at least one old node-owned chunk"
        );
        for chunk in removed_chunks {
            if let Some(entry) = surface.surface_chunk_cache.get(&chunk) {
                assert!(
                    !entry.node_ids.contains(&center),
                    "stale node contributor remained in removed chunk {chunk:?}"
                );
            }
        }
    }

    #[test]
    fn junction_node_non_road_surface_is_footprint_minus_asphalt() {
        let mut graph = RegionGraph::new();
        let left = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let right = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        let up = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
        graph.add_edge(test_edge(
            left,
            center,
            vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.add_edge(test_edge(
            center,
            right,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.add_edge(test_edge(
            center,
            up,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 24.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.rebuild_adjacency_list();

        let terrain = flat_terrain(96, 96);
        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);

        let piece = surface.compiled_visual_node_pieces().get(&center).unwrap();
        let footprint_area: f32 = piece.outer_boundary_loops.iter().map(polygon_area_m2).sum();
        let asphalt_area: f32 = piece
            .road_surface_polygons
            .iter()
            .map(polygon_triangle_area_m2)
            .sum();
        let non_road_area: f32 = piece
            .sidewalk_surface_polygons
            .iter()
            .map(polygon_triangle_area_m2)
            .sum();

        assert!(
            non_road_area > 0.0,
            "JunctionN must emit non-road node surface polygons"
        );
        let max_non_road_height = piece
            .sidewalk_surface_polygons
            .iter()
            .flat_map(|polygon| polygon.triangles_world.iter())
            .flat_map(|triangle| triangle.iter())
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            max_non_road_height >= CURB_STEP_HEIGHT_M - 0.001,
            "node non-road surfaces must sample curb/sidewalk band heights instead of flattened full-roadbed height; max_non_road_height={max_non_road_height:.3}"
        );
        assert!(
            (footprint_area - asphalt_area - non_road_area).abs() <= 0.05,
            "node non-road ownership must be exactly the resolved footprint minus asphalt; footprint={footprint_area:.3} asphalt={asphalt_area:.3} non_road={non_road_area:.3}"
        );
        assert_material_triangle_centroids_do_not_overlap(piece);
    }

    #[test]
    fn preview_matches_committed_sections_on_flat_terrain() {
        let terrain = flat_terrain(64, 64);
        let surface = RoadSurfaceSystem::new(16.0);
        let raw_points = vec![Vector3::new(0.0, 0.2, 0.0), Vector3::new(24.0, 0.2, 0.0)];

        let (preview, committed_sections, committed_visual_pieces) =
            compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

        assert_eq!(preview.edge_class, EdgeClass::Standard);
        assert!(preview.is_valid);
        assert_eq!(preview.compiled_sections, committed_sections);
        assert_eq!(preview.compiled_visual_node_pieces, committed_visual_pieces);
    }

    #[test]
    fn preview_matches_committed_sections_on_cross_slope() {
        let mut terrain = TerrainSystem::with_chunking(80, 16, 1.0, 8, 0.0);
        for z in 0..16 {
            for x in 0..80 {
                terrain.set_height(x, z, x as f32 * 0.005);
            }
        }
        let surface = RoadSurfaceSystem::new(16.0);
        let y0 = terrain.sample_height_world(-16.0, 0.0) * crate::config::HEIGHT_SCALE + 0.2;
        let y1 = terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE + 0.2;
        let y2 = terrain.sample_height_world(16.0, 0.0) * crate::config::HEIGHT_SCALE + 0.2;
        let raw_points = vec![
            Vector3::new(-16.0, y0, 0.0),
            Vector3::new(0.0, y1, 0.0),
            Vector3::new(16.0, y2, 0.0),
        ];

        let (preview, committed_sections, committed_visual_pieces) =
            compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

        assert_eq!(preview.edge_class, EdgeClass::Standard);
        assert!(preview.is_valid);
        assert_eq!(preview.compiled_sections, committed_sections);
        assert_eq!(preview.compiled_visual_node_pieces, committed_visual_pieces);
    }

    #[test]
    fn preview_matches_committed_sections_for_bridges() {
        let terrain = flat_terrain(96, 16);
        let surface = RoadSurfaceSystem::new(16.0);
        let raw_points = vec![
            Vector3::new(0.0, 3.0, 0.0),
            Vector3::new(16.0, 3.0, 0.0),
            Vector3::new(32.0, 3.0, 0.0),
        ];

        let (preview, committed_sections, committed_visual_pieces) =
            compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

        assert_eq!(preview.edge_class, EdgeClass::Bridge);
        assert!(preview.is_valid);
        assert_eq!(preview.compiled_sections, committed_sections);
        assert_eq!(preview.compiled_visual_node_pieces, committed_visual_pieces);
    }

    #[test]
    fn preview_matches_committed_sections_for_tunnels() {
        let terrain = flat_terrain(96, 16);
        let surface = RoadSurfaceSystem::new(16.0);
        let raw_points = vec![
            Vector3::new(0.0, -3.0, 0.0),
            Vector3::new(16.0, -3.0, 0.0),
            Vector3::new(32.0, -3.0, 0.0),
        ];

        let (preview, committed_sections, committed_visual_pieces) =
            compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

        assert_eq!(preview.edge_class, EdgeClass::Tunnel);
        assert!(preview.is_valid);
        assert_eq!(preview.compiled_sections, committed_sections);
        assert_eq!(preview.compiled_visual_node_pieces, committed_visual_pieces);
    }

    #[test]
    fn standard_road_footprint_uses_stitched_mesh_instead_of_visual_terrain_stamp() {
        let mut terrain = TerrainSystem::with_chunking(65, 65, 1.0, 8, 0.0);
        for z in 0..65 {
            for x in 0..65 {
                terrain.set_height(x, z, x as f32 * 0.01);
            }
        }

        let mut graph = RegionGraph::new();
        let grounded_height = terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
        let start = graph.add_node(
            Vector3::new(0.0, grounded_height, -16.0),
            NodeType::Junction,
        );
        let end = graph.add_node(Vector3::new(0.0, grounded_height, 16.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            vec![
                Vector3::new(0.0, grounded_height, -16.0),
                Vector3::new(0.0, grounded_height, 16.0),
            ],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.rebuild_all_earthworks(&graph, &mut terrain);

        let sections = surface.compiled_sections().get(&edge_idx).unwrap();
        let section = sections
            .iter()
            .min_by(|a, b| a.center_xz.y.abs().total_cmp(&b.center_xz.y.abs()))
            .unwrap();
        for lateral_offset in [-4.0_f32, 0.0, 4.0] {
            let road_height = section_height_at_lateral_offset(section, lateral_offset).unwrap();
            let sample_x = section.center_xz.x + section.lateral_xz.x * lateral_offset;
            let sample_z = section.center_xz.y + section.lateral_xz.y * lateral_offset;
            let source_height =
                terrain.sample_height_world(sample_x, sample_z) * crate::config::HEIGHT_SCALE;
            let visual_height = terrain.sample_visual_height_world(sample_x, sample_z)
                * crate::config::HEIGHT_SCALE;
            let support_height = surface
                .sample_paved_support_height(&graph, &terrain, sample_x, sample_z)
                .expect("standard paved footprint should expose a solved support surface");
            assert!(
                (visual_height - source_height).abs() <= 0.05,
                "ordinary standard roads must not stamp visual terrain at lateral_offset={lateral_offset:.1}: visual={visual_height:.3} source={source_height:.3}"
            );
            assert!(
                (support_height - road_height).abs() <= 0.05,
                "expected solved paved support to match the compiled road surface at lateral_offset={lateral_offset:.1}: support={support_height:.3} road_height={road_height:.3}"
            );
        }
    }

    #[test]
    fn grounded_standard_crossfall_is_bounded_and_footprint_stays_below_carriageway() {
        let mut terrain = TerrainSystem::with_chunking(129, 97, 1.0, 8, 0.0);
        for z in 0..97 {
            for x in 0..129 {
                terrain.set_height(x, z, x as f32 * 0.03);
            }
        }

        let mut graph = RegionGraph::new();
        let grounded_height = terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
        let start = graph.add_node(
            Vector3::new(0.0, grounded_height, -24.0),
            NodeType::Junction,
        );
        let end = graph.add_node(Vector3::new(0.0, grounded_height, 24.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            vec![
                Vector3::new(0.0, grounded_height, -24.0),
                Vector3::new(0.0, grounded_height, 24.0),
            ],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.rebuild_all_earthworks(&graph, &mut terrain);

        let section = surface
            .compiled_sections()
            .get(&edge_idx)
            .unwrap()
            .iter()
            .min_by(|a, b| a.center_xz.y.abs().total_cmp(&b.center_xz.y.abs()))
            .unwrap();
        let half_carriageway = graph.edge(edge_idx).width.max(crate::config::LANE_WIDTH) * 0.5;
        let left_height = section_height_at_lateral_offset(section, -half_carriageway).unwrap();
        let right_height = section_height_at_lateral_offset(section, half_carriageway).unwrap();
        let actual_crossfall_rate =
            (right_height - left_height) / (half_carriageway * 2.0).max(super::SAMPLE_EPSILON_M);

        assert!(
            actual_crossfall_rate.abs() <= super::MAX_STANDARD_DESIGN_CROSSFALL_RATE + 0.001,
            "expected grounded-road crossfall to stay within the design bound: actual_rate={actual_crossfall_rate:.4}"
        );

        let mut sampled_profile = Vec::new();
        for lateral_offset in [-half_carriageway * 0.8, 0.0, half_carriageway * 0.8] {
            let road_height = section_height_at_lateral_offset(section, lateral_offset).unwrap();
            let sample_x = section.center_xz.x + section.lateral_xz.x * lateral_offset;
            let sample_z = section.center_xz.y + section.lateral_xz.y * lateral_offset;
            let source_height =
                terrain.sample_height_world(sample_x, sample_z) * crate::config::HEIGHT_SCALE;
            let visual_height = terrain.sample_visual_height_world(sample_x, sample_z)
                * crate::config::HEIGHT_SCALE;
            let visible_surface_height = surface
                .sample_visible_surface_height(&graph, &terrain, sample_x, sample_z)
                .expect("standard road footprint should be owned by the road surface");
            sampled_profile.push((lateral_offset, road_height, visible_surface_height));
            assert!(
                (visual_height - source_height).abs() <= 0.05,
                "ordinary standard roads must not stamp visual terrain on a steep hillside: lateral_offset={lateral_offset:.2} visual_height={visual_height:.3} source_height={source_height:.3}"
            );
            assert!(
                (road_height - visible_surface_height).abs() <= 0.08,
                "expected grounded-road visible surface to follow the solved road surface: lateral_offset={lateral_offset:.2} visible_surface_height={visible_surface_height:.3} road_height={road_height:.3}"
            );
        }

        let left = sampled_profile.first().unwrap();
        let right = sampled_profile.last().unwrap();
        let road_profile_delta = right.1 - left.1;
        let support_profile_delta = right.2 - left.2;
        assert!(
            (support_profile_delta - road_profile_delta).abs() <= 0.05,
            "expected visible road footprint to follow the solved road crossfall instead of a flat slab: road_profile_delta={road_profile_delta:.3} support_profile_delta={support_profile_delta:.3}"
        );
    }

    #[test]
    fn flat_diagonal_10m_grid_keeps_paved_footprint_below_roadbed() {
        let terrain = TerrainSystem::with_chunking(129, 129, 10.0, 8, 0.0);
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(-160.0, 0.0, -160.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(160.0, 0.0, 160.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            vec![
                Vector3::new(-160.0, 0.0, -160.0),
                Vector3::new(160.0, 0.0, 160.0),
            ],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut terrain = terrain;
        let mut surface = RoadSurfaceSystem::new(128.0);
        surface.rebuild_all_earthworks(&graph, &mut terrain);
        let metrics = measure_max_footprint_overflow(&surface, &graph, edge_idx, &terrain);

        assert!(
            metrics.max_overflow_m <= 0.05,
            "expected a flat 45 degree road on a 10 m grid to keep the paved footprint below the roadbed, got {metrics:?}"
        );
    }

    #[test]
    fn shallow_angle_10m_grid_keeps_paved_footprint_below_roadbed() {
        let mut terrain = coarse_hillside_world_terrain(97, 97, 10.0);
        let points = grounded_polyline_points_from_terrain(
            &terrain,
            Vector2::new(-180.0, 5.0),
            Vector2::new(180.0, 1.0),
            28,
        );

        let mut graph = RegionGraph::new();
        let start = graph.add_node(points[0], NodeType::Junction);
        let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            points,
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(128.0);
        surface.rebuild_all_earthworks(&graph, &mut terrain);
        let metrics = measure_max_footprint_overflow(&surface, &graph, edge_idx, &terrain);

        assert!(
            metrics.max_overflow_m <= 0.05,
            "expected a shallow-angle road on a 10 m grid to keep the paved footprint below the roadbed, got {metrics:?}"
        );
    }

    #[test]
    fn coarse_10m_hillside_case_keeps_paved_footprint_below_roadbed() {
        let (surface, terrain, graph, edge_idx) = build_coarse_grid_hillside_case(10.0);
        let metrics = measure_max_footprint_overflow(&surface, &graph, edge_idx, &terrain);

        assert!(
            metrics.max_overflow_m <= 0.05,
            "expected the coarse 10 m hillside case to keep the paved footprint below the roadbed, got {metrics:?}"
        );
    }

    #[test]
    fn coarse_5m_hillside_case_stays_below_paved_roadbed_too() {
        let (coarse_surface, coarse_terrain, coarse_graph, coarse_edge_idx) =
            build_coarse_grid_hillside_case(10.0);
        let (fine_surface, fine_terrain, fine_graph, fine_edge_idx) =
            build_coarse_grid_hillside_case(5.0);
        let coarse_metrics = measure_max_footprint_overflow(
            &coarse_surface,
            &coarse_graph,
            coarse_edge_idx,
            &coarse_terrain,
        );
        let fine_metrics = measure_max_footprint_overflow(
            &fine_surface,
            &fine_graph,
            fine_edge_idx,
            &fine_terrain,
        );

        assert!(
            coarse_metrics.max_overflow_m <= 0.05,
            "expected the coarse reference case to stay below the paved roadbed, got coarse={coarse_metrics:?} fine={fine_metrics:?}"
        );
        assert!(
            fine_metrics.max_overflow_m <= 0.05,
            "expected the same hillside case on a 5 m grid to stay below the paved roadbed too, got coarse={coarse_metrics:?} fine={fine_metrics:?}"
        );
    }

    #[test]
    fn grounded_hillside_terrain_outside_paved_footprint_stays_near_source() {
        let mut terrain = TerrainSystem::with_chunking(129, 97, 1.0, 8, 0.0);
        for z in 0..97 {
            for x in 0..129 {
                terrain.set_height(x, z, x as f32 * 0.04);
            }
        }

        let mut graph = RegionGraph::new();
        let grounded_height = terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
        let start = graph.add_node(
            Vector3::new(0.0, grounded_height, -24.0),
            NodeType::Junction,
        );
        let end = graph.add_node(Vector3::new(0.0, grounded_height, 24.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            vec![
                Vector3::new(0.0, grounded_height, -24.0),
                Vector3::new(0.0, grounded_height, 24.0),
            ],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.rebuild_all_earthworks(&graph, &mut terrain);

        let sections = surface.compiled_sections().get(&edge_idx).unwrap();
        let section = sections
            .iter()
            .min_by(|a, b| a.center_xz.y.abs().total_cmp(&b.center_xz.y.abs()))
            .unwrap();
        let (left_outer, right_outer) = outer_surface_lateral_bounds(section).unwrap();

        let side_a_lateral = left_outer - 2.0;
        let side_b_lateral = right_outer + 2.0;
        let side_a_x = section.center_xz.x + section.lateral_xz.x * side_a_lateral;
        let side_a_z = section.center_xz.y + section.lateral_xz.y * side_a_lateral;
        let side_b_x = section.center_xz.x + section.lateral_xz.x * side_b_lateral;
        let side_b_z = section.center_xz.y + section.lateral_xz.y * side_b_lateral;
        let side_a_actual =
            terrain.sample_visual_height_world(side_a_x, side_a_z) * crate::config::HEIGHT_SCALE;
        let side_b_actual =
            terrain.sample_visual_height_world(side_b_x, side_b_z) * crate::config::HEIGHT_SCALE;
        let side_a_source =
            terrain.sample_height_world(side_a_x, side_a_z) * crate::config::HEIGHT_SCALE;
        let side_b_source =
            terrain.sample_height_world(side_b_x, side_b_z) * crate::config::HEIGHT_SCALE;
        assert!(
            (side_a_actual - side_a_source).abs() <= 0.12,
            "expected terrain outside the paved footprint to remain near source on hillside side A, got actual={side_a_actual:.3} source={side_a_source:.3}"
        );
        assert!(
            (side_b_actual - side_b_source).abs() <= 0.12,
            "expected terrain outside the paved footprint to remain near source on hillside side B, got actual={side_b_actual:.3} source={side_b_source:.3}"
        );

        let far_side_a_lateral = left_outer - EARTHWORK_MAX_MARGIN_M - 6.0;
        let far_side_b_lateral = right_outer + EARTHWORK_MAX_MARGIN_M + 6.0;
        let far_side_a_x = section.center_xz.x + section.lateral_xz.x * far_side_a_lateral;
        let far_side_a_z = section.center_xz.y + section.lateral_xz.y * far_side_a_lateral;
        let far_side_b_x = section.center_xz.x + section.lateral_xz.x * far_side_b_lateral;
        let far_side_b_z = section.center_xz.y + section.lateral_xz.y * far_side_b_lateral;
        let far_side_a_actual = terrain.sample_visual_height_world(far_side_a_x, far_side_a_z)
            * crate::config::HEIGHT_SCALE;
        let far_side_b_actual = terrain.sample_visual_height_world(far_side_b_x, far_side_b_z)
            * crate::config::HEIGHT_SCALE;
        let far_side_a_source =
            terrain.sample_height_world(far_side_a_x, far_side_a_z) * crate::config::HEIGHT_SCALE;
        let far_side_b_source =
            terrain.sample_height_world(far_side_b_x, far_side_b_z) * crate::config::HEIGHT_SCALE;

        assert!((far_side_a_actual - far_side_a_source).abs() <= 0.12);
        assert!((far_side_b_actual - far_side_b_source).abs() <= 0.12);
    }

    #[test]
    fn bridge_earthworks_do_not_flatten_under_the_span() {
        let mut terrain = flat_terrain(97, 33);
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(-24.0, 6.0, 0.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(24.0, 6.0, 0.0), NodeType::Junction);
        graph.add_edge(test_edge(
            start,
            end,
            vec![
                Vector3::new(-24.0, 6.0, 0.0),
                Vector3::new(0.0, 6.0, 0.0),
                Vector3::new(24.0, 6.0, 0.0),
            ],
            10.0,
            EdgeClass::Bridge,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.rebuild_all_earthworks(&graph, &mut terrain);

        let span_center =
            terrain.sample_visual_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
        let abutment = terrain.sample_visual_height_world(-20.0, 0.0) * crate::config::HEIGHT_SCALE;
        assert!(span_center.abs() <= 0.01);
        assert!(abutment >= 1.0);
    }

    #[test]
    fn tunnel_earthworks_only_stamp_portals() {
        let mut terrain = flat_terrain(97, 33);
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(test_edge(
            start,
            end,
            vec![
                Vector3::new(-24.0, 0.0, 0.0),
                Vector3::new(-10.0, -6.0, 0.0),
                Vector3::new(10.0, -6.0, 0.0),
                Vector3::new(24.0, 0.0, 0.0),
            ],
            10.0,
            EdgeClass::Tunnel,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.rebuild_all_earthworks(&graph, &mut terrain);

        let center = terrain.sample_visual_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
        let portal = terrain.sample_visual_height_world(-20.0, 0.0) * crate::config::HEIGHT_SCALE;
        assert!(center.abs() <= 0.01);
        assert!(portal <= -0.1);
    }

    #[test]
    fn dirty_terrain_earthworks_stay_bounded_to_touched_chunks() {
        let mut terrain = flat_terrain(161, 65);
        let mut graph = RegionGraph::new();
        let left_a = graph.add_node(Vector3::new(-56.0, 0.0, 0.0), NodeType::Junction);
        let left_b = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
        let right_a = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        let right_b = graph.add_node(Vector3::new(56.0, 0.0, 0.0), NodeType::Junction);
        let left_edge = graph.add_edge(test_edge(
            left_a,
            left_b,
            vec![Vector3::new(-56.0, 0.0, 0.0), Vector3::new(-24.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.add_edge(test_edge(
            right_a,
            right_b,
            vec![Vector3::new(24.0, 0.0, 0.0), Vector3::new(56.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.rebuild_all_earthworks(&graph, &mut terrain);
        let far_before =
            terrain.sample_visual_height_world(40.0, 0.0) * crate::config::HEIGHT_SCALE;

        surface.mark_edge_dirty(&graph, left_edge);
        let stamped_chunks = surface.rebuild_dirty_earthworks(&graph, &mut terrain);
        let far_after = terrain.sample_visual_height_world(40.0, 0.0) * crate::config::HEIGHT_SCALE;
        let right_chunk = surface.chunk_coords_for_world(40.0, 0.0);

        assert!(!stamped_chunks.is_empty());
        assert!(!stamped_chunks.contains(&right_chunk));
        assert!((far_after - far_before).abs() <= 0.001);
    }

    #[test]
    fn compile_dirty_derives_edge_chunks_from_compiled_piece_coverage() {
        let terrain = flat_terrain(64, 64);
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(5.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            n0,
            n1,
            vec![Vector3::new(5.0, 0.0, 0.0), Vector3::new(25.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(10.0);
        surface.compile_dirty(&graph, &terrain);

        let surface_chunks = surface
            .surface_span_chunks
            .get(&edge_idx)
            .expect("compiled span must own surface chunks")
            .clone();
        let terrain_chunks = surface
            .earthwork_span_chunks
            .get(&edge_idx)
            .expect("compiled span must own terrain chunks")
            .clone();
        assert!(!surface_chunks.is_empty());
        assert!(terrain_chunks.len() >= surface_chunks.len());

        surface.mark_edge_dirty(&graph, edge_idx);
        surface.compile_dirty(&graph, &terrain);

        for chunk in surface_chunks {
            let entry = surface
                .surface_chunk_cache
                .get(&chunk)
                .unwrap_or_else(|| panic!("surface chunk {chunk:?} must be rebuilt"));
            assert!(entry.edge_indices.contains(&edge_idx));
        }
        for chunk in terrain_chunks {
            let entry = surface
                .earthwork_chunk_cache
                .get(&chunk)
                .unwrap_or_else(|| panic!("terrain chunk {chunk:?} must be rebuilt"));
            assert!(entry.edge_indices.contains(&edge_idx));
        }
    }

    #[test]
    fn visible_surface_height_prefers_compiled_roadbed() {
        let mut terrain = TerrainSystem::with_chunking(65, 65, 1.0, 8, 0.0);
        for z in 0..65 {
            for x in 0..65 {
                terrain.set_height(x, z, x as f32 * 0.01);
            }
        }

        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(0.0, 0.0, -16.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(0.0, 0.0, 16.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            vec![Vector3::new(0.0, 0.0, -16.0), Vector3::new(0.0, 0.0, 16.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);

        let sampled = surface
            .sample_visible_surface_height(&graph, &terrain, 0.0, 0.0)
            .expect("standard road should own its paved footprint");
        let section = surface
            .compiled_sections()
            .get(&edge_idx)
            .unwrap()
            .iter()
            .min_by(|a, b| a.center_xz.y.abs().total_cmp(&b.center_xz.y.abs()))
            .unwrap();
        let expected = section_height_at_lateral_offset(section, 0.0).unwrap();
        assert!((sampled - expected).abs() <= 0.05);
    }

    #[test]
    fn paved_support_height_matches_grounded_visible_roadbed() {
        let mut terrain = TerrainSystem::with_chunking(65, 65, 1.0, 8, 0.0);
        for z in 0..65 {
            for x in 0..65 {
                terrain.set_height(x, z, x as f32 * 0.01);
            }
        }

        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(0.0, 0.0, -16.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(0.0, 0.0, 16.0), NodeType::Junction);
        graph.add_edge(test_edge(
            start,
            end,
            vec![Vector3::new(0.0, 0.0, -16.0), Vector3::new(0.0, 0.0, 16.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.rebuild_all_earthworks(&graph, &mut terrain);

        let visible_height = surface
            .sample_visible_surface_height(&graph, &terrain, 0.0, 0.0)
            .expect("grounded road should own its paved footprint");
        let support_height = surface
            .sample_paved_support_height(&graph, &terrain, 0.0, 0.0)
            .expect("grounded road should expose paved support clearance");

        assert!(
            (visible_height - support_height).abs() <= 0.05,
            "expected grounded-road integrated support height to match the visible roadbed instead of staying one pavement depth below it: visible_height={visible_height:.3} support_height={support_height:.3}"
        );
    }

    #[test]
    fn visible_surface_height_skips_grounded_terminal_earthwork_margin() {
        let terrain = flat_terrain(97, 97);
        let mut graph = RegionGraph::new();
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            end,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);
        let terminal_piece = surface
            .compiled_visual_node_pieces()
            .get(&center)
            .expect("terminal should compile a visual node piece");
        let inner_point = terminal_piece.outer_boundary_loops[0].points_world[0];
        let outer_point = terminal_piece.earthwork_outer_boundary_loops[0].points_world[0];
        let sample_x = (inner_point.x + outer_point.x) * 0.5;
        let sample_z = (inner_point.z + outer_point.z) * 0.5;

        assert!(
            surface
                .sample_visible_surface_height(&graph, &terrain, sample_x, sample_z)
                .is_none(),
            "grounded standard terminal earthwork margin stays outside visible-surface queries; Rust-generated terrain topology owns the ordinary seam"
        );
    }

    #[test]
    fn visible_surface_height_skips_grounded_span_earthwork_margin() {
        let terrain = flat_terrain(97, 97);
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            vec![Vector3::new(0.0, 0.0, -24.0), Vector3::new(0.0, 0.0, 24.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);
        let span_piece = surface
            .compiled_visual_span_pieces()
            .get(&edge_idx)
            .expect("standard edge should compile a visual span piece");
        let inner_point = span_piece.outer_boundary_loops[0].points_world[0];
        let outer_point = span_piece.earthwork_outer_boundary_loops[0].points_world[0];
        let sample_x = (inner_point.x + outer_point.x) * 0.5;
        let sample_z = (inner_point.z + outer_point.z) * 0.5;

        assert!(
            surface
                .sample_visible_surface_height(&graph, &terrain, sample_x, sample_z)
                .is_none(),
            "grounded standard span earthwork margin stays outside visible-surface queries; Rust-generated terrain topology owns the ordinary seam"
        );
    }

    #[test]
    fn visible_surface_height_skips_buried_tunnel_midspan() {
        let terrain = flat_terrain(97, 33);
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(test_edge(
            start,
            end,
            vec![
                Vector3::new(-24.0, 0.0, 0.0),
                Vector3::new(-10.0, -6.0, 0.0),
                Vector3::new(10.0, -6.0, 0.0),
                Vector3::new(24.0, 0.0, 0.0),
            ],
            10.0,
            EdgeClass::Tunnel,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);

        assert!(
            surface
                .sample_visible_surface_height(&graph, &terrain, 0.0, 0.0)
                .is_none()
        );
        assert!(
            surface
                .sample_visible_surface_height(&graph, &terrain, -20.0, 0.0)
                .is_some()
        );
    }

    #[test]
    fn visible_surface_raycast_hits_bridge_before_terrain() {
        let terrain = flat_terrain(97, 33);
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(-24.0, 6.0, 0.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(24.0, 6.0, 0.0), NodeType::Junction);
        graph.add_edge(test_edge(
            start,
            end,
            vec![
                Vector3::new(-24.0, 6.0, 0.0),
                Vector3::new(0.0, 6.0, 0.0),
                Vector3::new(24.0, 6.0, 0.0),
            ],
            10.0,
            EdgeClass::Bridge,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);

        let hit = surface
            .raycast_visible_surface(
                &graph,
                &terrain,
                Vector3::new(0.0, 20.0, 0.0),
                Vector3::DOWN,
            )
            .expect("bridge should be hittable by the combined world-surface ray");
        assert!((hit.y - 6.0).abs() <= 0.05);
    }

    #[test]
    fn debug_line_data_exposes_sections_bands_patches_and_earthwork_chunks() {
        let mut terrain = flat_terrain(65, 65);
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(0.0, 0.0, -16.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(0.0, 0.0, 16.0), NodeType::Junction);
        graph.add_edge(test_edge(
            start,
            end,
            vec![Vector3::new(0.0, 0.0, -16.0), Vector3::new(0.0, 0.0, 16.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.rebuild_all_earthworks(&graph, &mut terrain);
        let debug = surface.build_debug_line_data(&graph, &terrain);

        assert!(!debug.section_lines.is_empty());
        assert!(!debug.band_lines.is_empty());
        assert!(!debug.piece_boundary_lines.is_empty());
        assert!(!debug.earthwork_chunk_lines.is_empty());
    }

    #[test]
    fn debug_geometry_dump_exposes_edge_sections_and_terrain_samples() {
        let mut terrain = sloped_terrain(65, 65);
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(-16.0, 0.0, 0.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(16.0, 0.0, 0.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            vec![
                Vector3::new(-16.0, -0.8, 0.0),
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(16.0, 0.8, 0.0),
            ],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.rebuild_all_earthworks(&graph, &mut terrain);
        let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[edge_idx]);

        assert!(dump.contains("ROAD_GEOMETRY_DUMP_BEGIN"));
        assert!(dump.contains("\"edge_idx\": 0"));
        assert!(dump.contains("\"physical_geometry_world\""));
        assert!(dump.contains("\"sections\""));
        assert!(dump.contains("\"source_center_y_m\""));
        assert!(dump.contains("\"visual_center_y_m\""));
        assert!(dump.contains("\"left_outer_margin\""));
        assert!(dump.contains("\"right_outer_margin\""));
        assert!(dump.contains("ROAD_GEOMETRY_DUMP_END"));
    }

    #[test]
    fn transit_sync_to_terrain_invalidates_compiled_sections() {
        let terrain_before = flat_terrain(65, 65);
        let mut terrain_after = flat_terrain(65, 65);
        for z in 0..terrain_after.height {
            for x in 0..terrain_after.width {
                terrain_after.set_height(x, z, 0.5);
            }
        }

        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(0.0, 0.0, -16.0), NodeType::Junction);
        let end = graph.add_node(Vector3::new(0.0, 0.0, 16.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            vec![
                Vector3::new(0.0, 0.0, -16.0),
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(0.0, 0.0, 16.0),
            ],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.rebuild_adjacency_list();

        let mut network = TransitNetwork::new();
        network.road_surface.compile_dirty(&graph, &terrain_before);
        let before_height = network
            .road_surface
            .compiled_sections()
            .get(&edge_idx)
            .unwrap()[1]
            .center_height_m;

        network.sync_to_terrain(&mut graph, &terrain_after);
        assert!(
            graph.edge(edge_idx).geometry[1].y >= 9.5,
            "sync_to_terrain should resample edge geometry from terrain before recompilation"
        );

        network.road_surface.compile_dirty(&graph, &terrain_after);
        let after_height = network
            .road_surface
            .compiled_sections()
            .get(&edge_idx)
            .unwrap()[1]
            .center_height_m;

        assert!(
            after_height >= before_height + 9.5,
            "compiled roadbed cache should be invalidated after terrain sync, got before={before_height:.3} after={after_height:.3}"
        );
    }
}
