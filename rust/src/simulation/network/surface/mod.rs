//! Authoritative road-surface ownership layer and the first live compiler slices.
//!
//! This module now owns both the Phase 1 cache / dirty-tracking shell and the
//! first Phase 2 compiler pass for deterministic edge sections plus explicit
//! visual road pieces. It now drives the shipped preview, committed render mesh,
//! earthworks, and world-surface query paths from one deterministic compiled
//! roadbed cache.

use godot::prelude::{Vector2, Vector3};
use std::collections::{HashMap, HashSet};
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
const SIDEWALK_SLOPE_RATE: f32 = 0.02;
const SAMPLE_EPSILON_M: f32 = 0.001;
const NODE_CONNECTOR_SAMPLE_STEP_M: f32 = 1.0;
const ROAD_POINT_SIMPLIFY_DISTANCE_M: f32 = 0.5;
const TAUBIN_SMOOTHING_ITERS: usize = 50;
const TAUBIN_LAMBDA: f32 = 0.5;
const TAUBIN_MU: f32 = -0.53;
const PREVIEW_MAX_GRADE: f32 = 0.41;
const PREVIEW_CLEARANCE_M: f32 = 1.0;
const PREVIEW_MESH_LIFT_M: f32 = 0.05;
const EARTHWORK_PAVEMENT_DEPTH_M: f32 = 0.04;
const EARTHWORK_MIN_MARGIN_M: f32 = 4.0;
const EARTHWORK_MAX_MARGIN_M: f32 = 18.0;
const EARTHWORK_MARGIN_SAMPLE_STEP_M: f32 = 1.0;
const EARTHWORK_CUT_SLOPE_RATE: f32 = 0.5;
const EARTHWORK_FILL_SLOPE_RATE: f32 = 0.5;
const EARTHWORK_RETAINING_WALL_SLOPE_THRESHOLD: f32 = 1.25;
const BRIDGE_ABUTMENT_LENGTH_M: f32 = 12.0;
const TUNNEL_PORTAL_STAMP_DEPTH_M: f32 = 1.0;

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
    /// Outer piece-owned boundaries used for debug and surface chunk bounds.
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
    /// Outer piece-owned boundaries used for debug and surface chunk bounds.
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
struct IncidentPieceSide {
    boundary_points_outer_to_inner: Vec<Vector3>,
    band_kinds_outer_to_inner: Vec<RoadSurfaceBandKind>,
    inner_point_world: Vector3,
    inner_surface_kind: RoadSurfaceBandKind,
}

#[derive(Clone, Debug, PartialEq)]
struct IncidentPieceMouth {
    left: IncidentPieceSide,
    right: IncidentPieceSide,
}

#[derive(Clone, Debug, PartialEq)]
struct OrderedIncidentPieceMouth {
    mouth: IncidentPieceMouth,
    direction_angle_ccw: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct BendSectorGeometry {
    outer_start_point_world: Vector3,
    outer_end_point_world: Vector3,
    road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
}

#[derive(Clone, Debug, PartialEq)]
struct JunctionGapSectorGeometry {
    outer_start_point_world: Vector3,
    outer_end_point_world: Vector3,
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
    surface_chunk_cache: HashMap<SurfaceChunkKey, RoadSurfaceChunkCacheEntry>,
    earthwork_chunk_cache: HashMap<SurfaceChunkKey, RoadEarthworkChunkCacheEntry>,
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
            surface_chunk_cache: HashMap::new(),
            earthwork_chunk_cache: HashMap::new(),
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

        Self::sort_visual_polygons(&mut polygons);
        polygons
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
        self.surface_chunk_cache.clear();
        self.earthwork_chunk_cache.clear();
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

        for edge_idx in edge_ids {
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
        for edge_idx in sorted_span_edges {
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
                self.compiled_visual_span_pieces
                    .insert(edge_idx, span_piece);
            } else {
                self.compiled_visual_span_pieces.remove(&edge_idx);
            }
        }

        for &node_id in &sorted_nodes {
            if self.node_has_surface_edges(graph, node_id) {
                let visual_piece = self.compile_visual_node_piece(graph, terrain, node_id);
                if let Some(visual_piece) = visual_piece {
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
        self.rebuild_surface_chunk_cache(graph, &dirty_surface_chunks);
        self.rebuild_earthwork_chunk_cache(graph, &dirty_terrain_chunks);
        self.compiled_once = true;
        self.clear_dirty_tracking();
    }

    /// Rebuilds terrain earthworks only for the currently dirty road-surface chunks.
    pub fn rebuild_dirty_earthworks(
        &mut self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
    ) -> Vec<SurfaceChunkKey> {
        let dirty_chunks = if self.compiled_once {
            self.sorted_chunk_keys(&self.dirty_terrain_chunks)
        } else {
            Vec::new()
        };
        self.compile_dirty(graph, terrain);

        let chunks = if self.compiled_once && dirty_chunks.is_empty() {
            self.collect_all_chunks(graph, ChunkCacheKind::Earthwork)
        } else {
            dirty_chunks
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
        let chunks = self.collect_all_chunks(graph, ChunkCacheKind::Earthwork);
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

    /// Marks one edge dirty and tags all overlapping surface and terrain chunks.
    pub fn mark_edge_dirty(&mut self, graph: &RegionGraph, edge_idx: usize) {
        if edge_idx >= graph.edge_count() || graph.edge(edge_idx).deleted {
            return;
        }
        self.dirty_edges.insert(edge_idx);
        for chunk in self.edge_chunks(graph.edge(edge_idx), ChunkCacheKind::Surface) {
            self.dirty_surface_chunks.insert(chunk);
        }
        for chunk in self.edge_chunks(graph.edge(edge_idx), ChunkCacheKind::Earthwork) {
            self.dirty_terrain_chunks.insert(chunk);
        }
    }

    /// Marks one node dirty and tags the chunk containing its current world position.
    pub fn mark_node_dirty(&mut self, graph: &RegionGraph, node_id: u32) {
        if node_id as usize >= graph.node_count() {
            return;
        }
        let valid = graph.get_valid_node(node_id);
        self.dirty_nodes.insert(valid);
        self.mark_world_point_dirty(graph.node(valid).pos);
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

        let edge_ids = self.all_surface_edge_ids(graph);
        for &edge_idx in &edge_ids {
            self.compiled_sections.insert(
                edge_idx,
                self.compile_edge_sections(graph, terrain, edge_idx),
            );
        }

        for &edge_idx in &edge_ids {
            if let Some(span_piece) = self.compile_visual_span_piece(graph, terrain, edge_idx) {
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
                self.compiled_visual_node_pieces
                    .insert(node_id, visual_piece);
            } else {
                self.compiled_visual_node_pieces.remove(&node_id);
            }
        }

        let all_surface_chunks = self.collect_all_chunks(graph, ChunkCacheKind::Surface);
        let all_earthwork_chunks = self.collect_all_chunks(graph, ChunkCacheKind::Earthwork);
        self.rebuild_surface_chunk_cache(graph, &all_surface_chunks);
        self.rebuild_earthwork_chunk_cache(graph, &all_earthwork_chunks);
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
        let mouth = self.build_incident_mouth_profile(incident)?;
        if mouth.boundary_points_world.len() < 2 {
            return None;
        }

        let node_pos = graph.node(node_id).pos;
        let cap_depth = mouth
            .boundary_points_world
            .iter()
            .map(|point| Vector2::new(point.x - node_pos.x, point.z - node_pos.z).length())
            .fold(0.5, f32::max);
        let cap_offset = -incident.direction_xz.normalized() * cap_depth;
        let extruded_points: Vec<Vector3> = mouth
            .boundary_points_world
            .iter()
            .map(|point| Vector3::new(point.x + cap_offset.x, point.y, point.z + cap_offset.y))
            .collect();

        let mut road_surface_polygons = Vec::new();
        let mut sidewalk_surface_polygons = Vec::new();
        for (index, band) in mouth.bands.iter().enumerate() {
            let polygon = Self::make_visual_polygon(vec![
                band.start_point_world,
                extruded_points[index],
                extruded_points[index + 1],
                band.end_point_world,
            ])?;
            if band.kind == RoadSurfaceBandKind::Carriageway {
                road_surface_polygons.push(polygon);
            } else {
                sidewalk_surface_polygons.push(polygon);
            }
        }

        let outer_boundary_loops =
            Self::build_terminal_outer_boundary_loops(&mouth, &extruded_points);
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
        let sectors = Self::collect_bend_sectors(node_pos, &mouths);
        if sectors.is_empty() {
            return None;
        }
        let mut road_surface_polygons = Vec::new();
        let mut sidewalk_surface_polygons = Vec::new();
        for sector in &sectors {
            road_surface_polygons.extend(sector.road_surface_polygons.iter().cloned());
            sidewalk_surface_polygons.extend(sector.sidewalk_surface_polygons.iter().cloned());
        }
        let outer_boundary_loops = Self::build_bend_outer_boundary_loops(node_pos, &sectors);
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
        let gap_sectors = Self::collect_junction_gap_sectors(node_pos, &mouths);
        if gap_sectors.is_empty() {
            return None;
        }
        let mut road_surface_polygons = Vec::new();
        let mut sidewalk_surface_polygons = Vec::new();
        for sector in &gap_sectors {
            road_surface_polygons.extend(sector.road_surface_polygons.iter().cloned());
            sidewalk_surface_polygons.extend(sector.sidewalk_surface_polygons.iter().cloned());
        }
        let outer_boundary_loops =
            Self::build_junction_outer_boundary_loops(node_pos, &gap_sectors);
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
                mouth: self.build_incident_piece_mouth(incident)?,
                direction_angle_ccw: Self::normalized_angle_ccw(incident.direction_xz),
            });
        }
        Some(mouths)
    }

    fn build_terminal_outer_boundary_loops(
        mouth: &IncidentMouthProfile,
        extruded_points: &[Vector3],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut loop_points =
            Vec::with_capacity(mouth.boundary_points_world.len() + extruded_points.len());
        loop_points.extend(mouth.boundary_points_world.iter().copied());
        loop_points.extend(extruded_points.iter().rev().copied());
        let mut loops = Vec::new();
        if let Some(loop_polygon) = Self::make_visual_polygon(loop_points) {
            loops.push(loop_polygon);
        }
        Self::sort_visual_polygons(&mut loops);
        loops
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

    fn build_incident_piece_mouth(
        &self,
        incident: IncidentSurfaceEdge,
    ) -> Option<IncidentPieceMouth> {
        let mouth = self.build_incident_mouth_profile(incident)?;
        let carriageway_indices: Vec<usize> = mouth
            .bands
            .iter()
            .enumerate()
            .filter_map(|(index, band)| {
                (band.kind == RoadSurfaceBandKind::Carriageway).then_some(index)
            })
            .collect();

        if let (Some(&first_carriageway), Some(&last_carriageway)) =
            (carriageway_indices.first(), carriageway_indices.last())
        {
            // `boundary_points_world` is ordered from geometric right outer edge to geometric left
            // outer edge relative to the outward throat direction, so the first slice is the
            // right-side profile and the final slice is the left-side profile. Each side profile
            // must carry one carriageway half inward to the shared centerline so junction sectors
            // can classify deep connector strips as road instead of leaving only a tiny center
            // wedge owned by asphalt.
            let right = IncidentPieceSide {
                boundary_points_outer_to_inner: mouth.boundary_points_world
                    [0..=first_carriageway + 1]
                    .to_vec(),
                band_kinds_outer_to_inner: mouth.bands[0..=first_carriageway]
                    .iter()
                    .map(|band| band.kind)
                    .collect(),
                inner_point_world: mouth.boundary_points_world[first_carriageway + 1],
                inner_surface_kind: RoadSurfaceBandKind::Carriageway,
            };
            let left = IncidentPieceSide {
                boundary_points_outer_to_inner: mouth.boundary_points_world[last_carriageway..]
                    .iter()
                    .rev()
                    .copied()
                    .collect(),
                band_kinds_outer_to_inner: mouth.bands[last_carriageway..]
                    .iter()
                    .rev()
                    .map(|band| band.kind)
                    .collect(),
                inner_point_world: mouth.boundary_points_world[last_carriageway],
                inner_surface_kind: RoadSurfaceBandKind::Carriageway,
            };
            Some(IncidentPieceMouth { left, right })
        } else {
            let first_point = *mouth.boundary_points_world.first()?;
            let last_point = *mouth.boundary_points_world.last()?;
            let center_point = first_point.lerp(last_point, 0.5);
            let foot_kind = mouth
                .bands
                .first()
                .map(|band| band.kind)
                .unwrap_or(RoadSurfaceBandKind::Footpath);
            let right = IncidentPieceSide {
                boundary_points_outer_to_inner: vec![first_point, center_point],
                band_kinds_outer_to_inner: vec![foot_kind],
                inner_point_world: center_point,
                inner_surface_kind: foot_kind,
            };
            let left = IncidentPieceSide {
                boundary_points_outer_to_inner: vec![last_point, center_point],
                band_kinds_outer_to_inner: vec![foot_kind],
                inner_point_world: center_point,
                inner_surface_kind: foot_kind,
            };
            Some(IncidentPieceMouth { left, right })
        }
    }

    fn build_bend_connector_polygons(
        side_a: &IncidentPieceSide,
        side_b: &IncidentPieceSide,
    ) -> Vec<(RoadSurfaceBandKind, RoadSurfaceVisualPolygon)> {
        let mut polygons = Vec::new();
        let normalized_breaks = Self::bend_connector_sample_breaks(side_a, side_b);
        for interval in normalized_breaks.windows(2) {
            let t0 = interval[0];
            let t1 = interval[1];
            if t1 - t0 <= 0.0001 {
                continue;
            }
            let Some(a0) = Self::sample_side_point(side_a, t0) else {
                continue;
            };
            let Some(a1) = Self::sample_side_point(side_a, t1) else {
                continue;
            };
            let Some(b0) = Self::sample_side_point(side_b, t0) else {
                continue;
            };
            let Some(b1) = Self::sample_side_point(side_b, t1) else {
                continue;
            };
            let kind = Self::sector_surface_kind(side_a, side_b, (t0 + t1) * 0.5);
            let Some(polygon) = Self::make_visual_polygon(vec![a0, b0, b1, a1]) else {
                continue;
            };
            polygons.push((kind, polygon));
        }
        polygons
    }

    fn build_junction_gap_connector_polygons(
        side_a: &IncidentPieceSide,
        side_b: &IncidentPieceSide,
    ) -> Vec<(RoadSurfaceBandKind, RoadSurfaceVisualPolygon)> {
        let mut polygons = Vec::new();
        let normalized_breaks = Self::junction_gap_connector_sample_breaks(side_a, side_b);
        for interval in normalized_breaks.windows(2) {
            let t0 = interval[0];
            let t1 = interval[1];
            if t1 - t0 <= 0.0001 {
                continue;
            }
            let Some(a0) = Self::sample_side_point(side_a, t0) else {
                continue;
            };
            let Some(a1) = Self::sample_side_point(side_a, t1) else {
                continue;
            };
            let Some(b0) = Self::sample_side_point(side_b, t0) else {
                continue;
            };
            let Some(b1) = Self::sample_side_point(side_b, t1) else {
                continue;
            };
            let kind = Self::sector_surface_kind(side_a, side_b, (t0 + t1) * 0.5);
            let Some(polygon) = Self::make_visual_polygon(vec![a0, b0, b1, a1]) else {
                continue;
            };
            polygons.push((kind, polygon));
        }
        polygons
    }

    fn bend_connector_sample_breaks(
        side_a: &IncidentPieceSide,
        side_b: &IncidentPieceSide,
    ) -> Vec<f32> {
        let mut breaks = Self::merged_side_breaks(side_a, side_b);
        let max_length = Self::side_total_length(side_a).max(Self::side_total_length(side_b));
        if max_length > SAMPLE_EPSILON_M {
            let mut distance = NODE_CONNECTOR_SAMPLE_STEP_M;
            while distance < max_length - SAMPLE_EPSILON_M {
                breaks.push((distance / max_length).clamp(0.0, 1.0));
                distance += NODE_CONNECTOR_SAMPLE_STEP_M;
            }
        }
        breaks.sort_by(|a, b| a.total_cmp(b));
        breaks.dedup_by(|a, b| (*a - *b).abs() <= 0.0001);
        if breaks.first().copied().unwrap_or(1.0) > 0.0 {
            breaks.insert(0, 0.0);
        }
        if breaks.last().copied().unwrap_or(0.0) < 1.0 {
            breaks.push(1.0);
        }
        breaks
    }

    fn junction_gap_connector_sample_breaks(
        side_a: &IncidentPieceSide,
        side_b: &IncidentPieceSide,
    ) -> Vec<f32> {
        let mut breaks = Self::merged_side_breaks(side_a, side_b);
        let max_length = Self::side_total_length(side_a).max(Self::side_total_length(side_b));
        if max_length > SAMPLE_EPSILON_M {
            let mut distance = NODE_CONNECTOR_SAMPLE_STEP_M;
            while distance < max_length - SAMPLE_EPSILON_M {
                breaks.push((distance / max_length).clamp(0.0, 1.0));
                distance += NODE_CONNECTOR_SAMPLE_STEP_M;
            }
        }
        breaks.sort_by(|a, b| a.total_cmp(b));
        breaks.dedup_by(|a, b| (*a - *b).abs() <= 0.0001);
        if breaks.first().copied().unwrap_or(1.0) > 0.0 {
            breaks.insert(0, 0.0);
        }
        if breaks.last().copied().unwrap_or(0.0) < 1.0 {
            breaks.push(1.0);
        }
        breaks
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

    fn merged_side_breaks(side_a: &IncidentPieceSide, side_b: &IncidentPieceSide) -> Vec<f32> {
        let mut breaks = Self::side_depth_breaks(side_a);
        breaks.extend(Self::side_depth_breaks(side_b));
        breaks.sort_by(|a, b| a.total_cmp(b));
        breaks.dedup_by(|a, b| (*a - *b).abs() <= 0.0001);
        if breaks.first().copied().unwrap_or(1.0) > 0.0 {
            breaks.insert(0, 0.0);
        }
        if breaks.last().copied().unwrap_or(0.0) < 1.0 {
            breaks.push(1.0);
        }
        breaks
    }

    fn side_total_length(side: &IncidentPieceSide) -> f32 {
        *Self::side_cumulative_lengths(side).last().unwrap_or(&0.0)
    }

    fn side_depth_breaks(side: &IncidentPieceSide) -> Vec<f32> {
        let cumulative = Self::side_cumulative_lengths(side);
        let total_length = *cumulative.last().unwrap_or(&0.0);
        if total_length <= SAMPLE_EPSILON_M {
            return vec![0.0, 1.0];
        }

        cumulative
            .into_iter()
            .map(|distance| (distance / total_length).clamp(0.0, 1.0))
            .collect()
    }

    fn side_cumulative_lengths(side: &IncidentPieceSide) -> Vec<f32> {
        let mut cumulative = Vec::with_capacity(side.boundary_points_outer_to_inner.len());
        let mut running = 0.0;
        cumulative.push(0.0);
        for pair in side.boundary_points_outer_to_inner.windows(2) {
            running += pair[0].distance_to(pair[1]);
            cumulative.push(running);
        }
        cumulative
    }

    fn sample_side_point(side: &IncidentPieceSide, t: f32) -> Option<Vector3> {
        let cumulative = Self::side_cumulative_lengths(side);
        let total_length = *cumulative.last()?;
        let clamped_t = t.clamp(0.0, 1.0);
        if total_length <= SAMPLE_EPSILON_M {
            return side.boundary_points_outer_to_inner.first().copied();
        }

        let target = clamped_t * total_length;
        for (index, pair) in side.boundary_points_outer_to_inner.windows(2).enumerate() {
            let start_distance = cumulative[index];
            let end_distance = cumulative[index + 1];
            let segment_length = end_distance - start_distance;
            if segment_length <= SAMPLE_EPSILON_M {
                continue;
            }
            if target <= end_distance + SAMPLE_EPSILON_M || index + 2 == cumulative.len() {
                let factor = ((target - start_distance) / segment_length).clamp(0.0, 1.0);
                return Some(pair[0].lerp(pair[1], factor));
            }
        }

        side.boundary_points_outer_to_inner.last().copied()
    }

    fn surface_kind_at_depth(side: &IncidentPieceSide, t: f32) -> RoadSurfaceBandKind {
        let cumulative = Self::side_cumulative_lengths(side);
        let total_length = *cumulative.last().unwrap_or(&0.0);
        if total_length <= SAMPLE_EPSILON_M {
            return side
                .band_kinds_outer_to_inner
                .first()
                .copied()
                .unwrap_or(side.inner_surface_kind);
        }

        let target = t.clamp(0.0, 1.0) * total_length;
        for index in 0..side.band_kinds_outer_to_inner.len() {
            if index + 1 >= cumulative.len() {
                break;
            }
            if target <= cumulative[index + 1] + SAMPLE_EPSILON_M {
                return side.band_kinds_outer_to_inner[index];
            }
        }

        side.band_kinds_outer_to_inner
            .last()
            .copied()
            .unwrap_or(side.inner_surface_kind)
    }

    fn sector_surface_kind(
        side_a: &IncidentPieceSide,
        side_b: &IncidentPieceSide,
        t: f32,
    ) -> RoadSurfaceBandKind {
        let kind_a = Self::surface_kind_at_depth(side_a, t);
        let kind_b = Self::surface_kind_at_depth(side_b, t);
        if kind_a == RoadSurfaceBandKind::Carriageway || kind_b == RoadSurfaceBandKind::Carriageway
        {
            RoadSurfaceBandKind::Carriageway
        } else {
            RoadSurfaceBandKind::Sidewalk
        }
    }

    fn build_junction_gap_sector_geometry(
        node_pos: Vector3,
        current: &OrderedIncidentPieceMouth,
        next: &OrderedIncidentPieceMouth,
    ) -> Option<JunctionGapSectorGeometry> {
        let (current_side, next_side) = Self::select_adjacent_gap_sides(current, next)?;

        let mut road_surface_polygons = Vec::new();
        let mut sidewalk_surface_polygons = Vec::new();
        if current_side.inner_surface_kind == RoadSurfaceBandKind::Carriageway
            && next_side.inner_surface_kind == RoadSurfaceBandKind::Carriageway
        {
            if let Some(polygon) = Self::make_visual_polygon(vec![
                node_pos,
                current_side.inner_point_world,
                next_side.inner_point_world,
            ]) {
                road_surface_polygons.push(polygon);
            }
        }

        for (band_kind, polygon) in
            Self::build_junction_gap_connector_polygons(current_side, next_side)
        {
            if band_kind == RoadSurfaceBandKind::Carriageway {
                road_surface_polygons.push(polygon);
            } else {
                sidewalk_surface_polygons.push(polygon);
            }
        }

        (!road_surface_polygons.is_empty() || !sidewalk_surface_polygons.is_empty()).then_some(
            JunctionGapSectorGeometry {
                outer_start_point_world: *current_side.boundary_points_outer_to_inner.first()?,
                outer_end_point_world: *next_side.boundary_points_outer_to_inner.first()?,
                road_surface_polygons,
                sidewalk_surface_polygons,
            },
        )
    }

    fn build_bend_sector_geometry(
        node_pos: Vector3,
        current: &OrderedIncidentPieceMouth,
        next: &OrderedIncidentPieceMouth,
    ) -> Option<BendSectorGeometry> {
        let (current_side, next_side) = Self::select_adjacent_gap_sides(current, next)?;
        let mut road_surface_polygons = Vec::new();
        let mut sidewalk_surface_polygons = Vec::new();
        if current_side.inner_surface_kind == RoadSurfaceBandKind::Carriageway
            && next_side.inner_surface_kind == RoadSurfaceBandKind::Carriageway
        {
            if let Some(polygon) = Self::make_visual_polygon(vec![
                node_pos,
                current_side.inner_point_world,
                next_side.inner_point_world,
            ]) {
                road_surface_polygons.push(polygon);
            }
        }
        for (band_kind, polygon) in Self::build_bend_connector_polygons(current_side, next_side) {
            if band_kind == RoadSurfaceBandKind::Carriageway {
                road_surface_polygons.push(polygon);
            } else {
                sidewalk_surface_polygons.push(polygon);
            }
        }
        (!road_surface_polygons.is_empty() || !sidewalk_surface_polygons.is_empty()).then_some(
            BendSectorGeometry {
                outer_start_point_world: *current_side.boundary_points_outer_to_inner.first()?,
                outer_end_point_world: *next_side.boundary_points_outer_to_inner.first()?,
                road_surface_polygons,
                sidewalk_surface_polygons,
            },
        )
    }

    fn collect_bend_sectors(
        node_pos: Vector3,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Vec<BendSectorGeometry> {
        let mut sectors = Vec::new();
        for index in 0..mouths.len() {
            let next_index = (index + 1) % mouths.len();
            let Some(sector) =
                Self::build_bend_sector_geometry(node_pos, &mouths[index], &mouths[next_index])
            else {
                continue;
            };
            sectors.push(sector);
        }
        sectors
    }

    fn build_bend_outer_boundary_loops(
        node_pos: Vector3,
        sectors: &[BendSectorGeometry],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut loop_points = Vec::new();
        for sector in sectors {
            loop_points.push(sector.outer_start_point_world);
            loop_points.push(sector.outer_end_point_world);
        }
        loop_points.dedup_by(|a, b| (*a - *b).length_squared() <= 0.0001);
        Self::sort_points_around_node(node_pos, &mut loop_points);
        let mut loops = Vec::new();
        if let Some(loop_polygon) = Self::make_visual_polygon(loop_points) {
            loops.push(loop_polygon);
        }
        Self::sort_visual_polygons(&mut loops);
        loops
    }

    fn collect_junction_gap_sectors(
        node_pos: Vector3,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Vec<JunctionGapSectorGeometry> {
        let mut sectors = Vec::new();
        for index in 0..mouths.len() {
            let next_index = (index + 1) % mouths.len();
            let Some(sector) = Self::build_junction_gap_sector_geometry(
                node_pos,
                &mouths[index],
                &mouths[next_index],
            ) else {
                continue;
            };
            sectors.push(sector);
        }
        sectors
    }

    fn build_junction_outer_boundary_loops(
        node_pos: Vector3,
        sectors: &[JunctionGapSectorGeometry],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut loop_points = Vec::new();
        for sector in sectors {
            loop_points.push(sector.outer_start_point_world);
            loop_points.push(sector.outer_end_point_world);
        }
        loop_points.dedup_by(|a, b| (*a - *b).length_squared() <= 0.0001);
        Self::sort_points_around_node(node_pos, &mut loop_points);
        let mut loops = Vec::new();
        if let Some(loop_polygon) = Self::make_visual_polygon(loop_points) {
            loops.push(loop_polygon);
        }
        Self::sort_visual_polygons(&mut loops);
        loops
    }

    fn sort_points_around_node(node_pos: Vector3, points_world: &mut [Vector3]) {
        points_world.sort_by(|a, b| {
            (a.z - node_pos.z)
                .atan2(a.x - node_pos.x)
                .total_cmp(&(b.z - node_pos.z).atan2(b.x - node_pos.x))
                .then(a.x.total_cmp(&b.x))
                .then(a.z.total_cmp(&b.z))
                .then(a.y.total_cmp(&b.y))
        });
    }

    fn normalized_angle_ccw(direction_xz: Vector2) -> f32 {
        let angle = direction_xz.y.atan2(direction_xz.x);
        if angle < 0.0 {
            angle + std::f32::consts::TAU
        } else {
            angle
        }
    }

    fn select_adjacent_gap_sides<'a>(
        current: &'a OrderedIncidentPieceMouth,
        next: &'a OrderedIncidentPieceMouth,
    ) -> Option<(&'a IncidentPieceSide, &'a IncidentPieceSide)> {
        let gap_span = if next.direction_angle_ccw < current.direction_angle_ccw {
            next.direction_angle_ccw + std::f32::consts::TAU - current.direction_angle_ccw
        } else {
            next.direction_angle_ccw - current.direction_angle_ccw
        };
        (gap_span > 0.0001).then_some((&current.mouth.left, &next.mouth.right))
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
        if earthwork_outer_boundary_loops.is_empty() {
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
        let left_sidewalk_outer_height =
            left_curb_top_height - sidewalk_width * SIDEWALK_SLOPE_RATE;
        let right_sidewalk_outer_height =
            right_curb_top_height - sidewalk_width * SIDEWALK_SLOPE_RATE;

        let mut bands = Vec::new();
        if sidewalk_width > 0.0 {
            bands.push(RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Sidewalk,
                lateral_start_m: -(half_carriageway + curb_width + sidewalk_width),
                lateral_end_m: -(half_carriageway + curb_width),
                height_start_m: left_sidewalk_outer_height,
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
                height_end_m: right_sidewalk_outer_height,
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
        let start_throat = edge.start_clip.clamp(0.0, total_length);
        let end_throat = (total_length - edge.end_clip).clamp(0.0, total_length);
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

    fn rebuild_surface_chunk_cache(&mut self, graph: &RegionGraph, chunks: &[SurfaceChunkKey]) {
        for &chunk in chunks {
            let (edge_indices, node_ids) =
                self.chunk_contributors(graph, chunk, ChunkCacheKind::Surface);
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

    fn rebuild_earthwork_chunk_cache(&mut self, graph: &RegionGraph, chunks: &[SurfaceChunkKey]) {
        for &chunk in chunks {
            let (edge_indices, node_ids) =
                self.chunk_contributors(graph, chunk, ChunkCacheKind::Earthwork);
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

    fn chunk_contributors(
        &self,
        graph: &RegionGraph,
        chunk: SurfaceChunkKey,
        kind: ChunkCacheKind,
    ) -> (Vec<usize>, Vec<u32>) {
        let (query_min, query_max) = self.chunk_query_bounds(chunk, kind);
        let mut edge_indices: Vec<usize> = graph
            .get_edges_near_aabb(query_min, query_max)
            .into_iter()
            .filter(|edge_idx| {
                let Some(piece) = self.compiled_visual_span_pieces.get(edge_idx) else {
                    return false;
                };
                self.visual_span_piece_overlaps_chunk(*edge_idx, piece, chunk, kind)
            })
            .collect();
        edge_indices.sort_unstable();
        edge_indices.dedup();

        let mut node_ids: Vec<u32> = match kind {
            ChunkCacheKind::Surface | ChunkCacheKind::Earthwork => self
                .compiled_visual_node_pieces
                .iter()
                .filter_map(|(&node_id, piece)| {
                    self.visual_node_piece_overlaps_chunk(node_id, piece, chunk, kind)
                        .then_some(node_id)
                })
                .collect(),
        };
        node_ids.sort_unstable();
        node_ids.dedup();

        (edge_indices, node_ids)
    }

    fn visual_node_piece_overlaps_chunk(
        &self,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        chunk: SurfaceChunkKey,
        kind: ChunkCacheKind,
    ) -> bool {
        let Some((min, max)) = self.visual_node_piece_bounds(piece, kind) else {
            return false;
        };

        let min_chunk = self.chunk_coords_for_world(min.x, min.z);
        let max_chunk = self.chunk_coords_for_world(max.x, max.z);
        chunk.0 >= min_chunk.0
            && chunk.0 <= max_chunk.0
            && chunk.1 >= min_chunk.1
            && chunk.1 <= max_chunk.1
            && piece.node_id == node_id
    }

    fn visual_span_piece_overlaps_chunk(
        &self,
        edge_idx: usize,
        piece: &RoadSurfaceVisualSpanPiece,
        chunk: SurfaceChunkKey,
        kind: ChunkCacheKind,
    ) -> bool {
        let Some((min, max)) = self.visual_span_piece_bounds(piece, kind) else {
            return false;
        };

        let min_chunk = self.chunk_coords_for_world(min.x, min.z);
        let max_chunk = self.chunk_coords_for_world(max.x, max.z);
        chunk.0 >= min_chunk.0
            && chunk.0 <= max_chunk.0
            && chunk.1 >= min_chunk.1
            && chunk.1 <= max_chunk.1
            && piece.edge_idx == edge_idx
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
            edge.start_clip.clamp(0.0, total_length)
        } else {
            0.0
        };
        let end_handoff = if matches!(
            end_kind,
            Some(CompiledNodeKind::Bend | CompiledNodeKind::JunctionN)
        ) {
            (total_length - edge.end_clip).clamp(0.0, total_length)
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

    fn edge_earthwork_extent_m(&self, edge: &Edge) -> f32 {
        self.outer_roadbed_half_width_m(edge)
            + if edge.class == EdgeClass::Standard {
                EARTHWORK_MAX_MARGIN_M
            } else {
                0.0
            }
    }

    fn outer_roadbed_half_width_m(&self, edge: &Edge) -> f32 {
        if edge.primary_type == TransitType::Foot || (edge.allowed_types & TransitFlags::CAR) == 0 {
            return edge.width.max(2.0) * 0.5;
        }

        let half_carriageway = edge.width.max(config::LANE_WIDTH) * 0.5;
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
        half_carriageway + curb_width + sidewalk_width
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
        let height_offset_m = self.span_piece_integrated_surface_offset_m(piece);
        if piece.edge_class == EdgeClass::Standard {
            self.stamp_piece_surface_geometry_for_chunk(
                &piece.earthwork_surface_polygons,
                chunk,
                terrain,
                0.0,
            );
        }

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

        let height_offset_m = self.node_piece_integrated_surface_offset_m(graph, node_id, terrain);
        if !self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
            self.stamp_piece_surface_geometry_for_chunk(
                &piece.earthwork_surface_polygons,
                chunk,
                terrain,
                0.0,
            );
        }

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
        let start_handoff = edge.start_clip.clamp(0.0, total_length);
        let end_handoff = (total_length - edge.end_clip).clamp(0.0, total_length);
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
        let triangles_world = Self::triangulate_simple_polygon_xz(&points_world)?;
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

    fn triangulate_simple_polygon_xz(points_world: &[Vector3]) -> Option<Vec<[Vector3; 3]>> {
        if points_world.len() < 3 {
            return None;
        }
        if points_world.len() == 3 {
            let triangle = [points_world[0], points_world[1], points_world[2]];
            return Self::triangle_has_area_xz(triangle).then_some(vec![triangle]);
        }

        let mut remaining: Vec<usize> = (0..points_world.len()).collect();
        let mut triangles = Vec::with_capacity(points_world.len() - 2);
        let mut guard = 0usize;
        let guard_limit = points_world.len() * points_world.len();

        while remaining.len() > 3 && guard < guard_limit {
            let mut clipped_ear = false;
            for index in 0..remaining.len() {
                let prev = remaining[(index + remaining.len() - 1) % remaining.len()];
                let current = remaining[index];
                let next = remaining[(index + 1) % remaining.len()];
                let triangle = [
                    points_world[prev],
                    points_world[current],
                    points_world[next],
                ];
                let projected_cross = (triangle[1].x - triangle[0].x)
                    * (triangle[2].z - triangle[0].z)
                    - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
                if projected_cross <= 0.002 {
                    continue;
                }

                let contains_other_vertex = remaining.iter().copied().any(|candidate| {
                    if candidate == prev || candidate == current || candidate == next {
                        return false;
                    }
                    Self::triangle_barycentric_weights_xz(
                        triangle,
                        Vector2::new(points_world[candidate].x, points_world[candidate].z),
                    )
                    .is_some()
                });
                if contains_other_vertex {
                    continue;
                }

                triangles.push(triangle);
                remaining.remove(index);
                clipped_ear = true;
                break;
            }

            if !clipped_ear {
                return None;
            }
            guard += 1;
        }

        if remaining.len() == 3 {
            let triangle = [
                points_world[remaining[0]],
                points_world[remaining[1]],
                points_world[remaining[2]],
            ];
            if Self::triangle_has_area_xz(triangle) {
                triangles.push(triangle);
            }
        }

        (!triangles.is_empty()).then_some(triangles)
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

    #[cfg(test)]
    fn polygon_has_area_xz(points: &[Vector3]) -> bool {
        Self::signed_polygon_area_xz(points).abs() > 0.002
    }

    fn triangle_has_area_xz(triangle: [Vector3; 3]) -> bool {
        let projected_cross = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
            - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
        projected_cross.abs() > 0.002
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
            .compiled_visual_span_pieces
            .get(&edge_idx)
            .and_then(|piece| {
                self.visual_span_piece_bounds(piece, ChunkCacheKind::Surface)
                    .map(|(min, max)| self.bounds_to_chunk_keys(min, max))
            })
            .unwrap_or_else(|| self.edge_chunks(edge, ChunkCacheKind::Surface));
        let earthwork_chunks = self
            .compiled_visual_span_pieces
            .get(&edge_idx)
            .and_then(|piece| {
                self.visual_span_piece_bounds(piece, ChunkCacheKind::Earthwork)
                    .map(|(min, max)| self.bounds_to_chunk_keys(min, max))
            })
            .unwrap_or_else(|| self.edge_chunks(edge, ChunkCacheKind::Earthwork));

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

    fn collect_all_chunks(
        &self,
        _graph: &RegionGraph,
        kind: ChunkCacheKind,
    ) -> Vec<SurfaceChunkKey> {
        let mut chunks = HashSet::new();
        for piece in self.compiled_visual_span_pieces.values() {
            let Some((min, max)) = self.visual_span_piece_bounds(piece, kind) else {
                continue;
            };
            let min_chunk = self.chunk_coords_for_world(min.x, min.z);
            let max_chunk = self.chunk_coords_for_world(max.x, max.z);
            for cx in min_chunk.0..=max_chunk.0 {
                for cz in min_chunk.1..=max_chunk.1 {
                    chunks.insert((cx, cz));
                }
            }
        }
        match kind {
            ChunkCacheKind::Surface | ChunkCacheKind::Earthwork => {
                for piece in self.compiled_visual_node_pieces.values() {
                    let Some((min, max)) = self.visual_node_piece_bounds(piece, kind) else {
                        continue;
                    };
                    let min_chunk = self.chunk_coords_for_world(min.x, min.z);
                    let max_chunk = self.chunk_coords_for_world(max.x, max.z);
                    for cx in min_chunk.0..=max_chunk.0 {
                        for cz in min_chunk.1..=max_chunk.1 {
                            chunks.insert((cx, cz));
                        }
                    }
                }
            }
        }
        self.sorted_chunk_keys(&chunks)
    }

    fn chunk_query_bounds(
        &self,
        chunk: SurfaceChunkKey,
        kind: ChunkCacheKind,
    ) -> (Vector3, Vector3) {
        let (min, max) = self.chunk_bounds(chunk);
        if kind == ChunkCacheKind::Surface {
            return (min, max);
        }

        let padding = EARTHWORK_MAX_MARGIN_M
            + config::SIDEWALK_WIDTH
            + CURB_BAND_WIDTH_M
            + config::LANE_WIDTH * 2.0;
        (
            Vector3::new(min.x - padding, 0.0, min.z - padding),
            Vector3::new(max.x + padding, 0.0, max.z + padding),
        )
    }

    fn edge_bounds(&self, edge: &Edge, kind: ChunkCacheKind) -> Option<(Vector3, Vector3)> {
        let points = self.edge_points(edge);
        if points.is_empty() {
            return None;
        }

        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_z = f32::MAX;
        let mut max_z = f32::MIN;
        for point in points {
            min_x = min_x.min(point.x);
            max_x = max_x.max(point.x);
            min_z = min_z.min(point.z);
            max_z = max_z.max(point.z);
        }

        if kind == ChunkCacheKind::Earthwork {
            let padding = self.edge_earthwork_extent_m(edge);
            min_x -= padding;
            max_x += padding;
            min_z -= padding;
            max_z += padding;
        }

        Some((
            Vector3::new(min_x, 0.0, min_z),
            Vector3::new(max_x, 0.0, max_z),
        ))
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

    fn edge_chunks(&self, edge: &Edge, kind: ChunkCacheKind) -> Vec<SurfaceChunkKey> {
        if !Self::is_surface_edge(edge) {
            return Vec::new();
        }
        let Some((min, max)) = self.edge_bounds(edge, kind) else {
            return Vec::new();
        };

        self.bounds_to_chunk_keys(min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EARTHWORK_MAX_MARGIN_M, PreviewRoadSurfaceResult, RoadSurfaceEarthworkFaceKind,
        RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceVisualNodePiece,
        RoadSurfaceVisualNodePieceKind,
    };
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };
    use crate::simulation::terrain::TerrainSystem;
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
    fn mark_edge_dirty_tracks_edge_and_overlapping_chunks() {
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
        assert_eq!(surface.dirty_surface_chunks().len(), 3);
        assert!(surface.dirty_surface_chunks().contains(&(0, 0)));
        assert!(surface.dirty_surface_chunks().contains(&(1, 0)));
        assert!(surface.dirty_surface_chunks().contains(&(2, 0)));
        assert!(surface.dirty_terrain_chunks().len() > surface.dirty_surface_chunks().len());
        assert!(
            surface
                .dirty_terrain_chunks()
                .is_superset(surface.dirty_surface_chunks())
        );
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
        assert!(
            surface
                .dirty_terrain_chunks()
                .is_superset(surface.dirty_surface_chunks())
        );
        assert!(surface.dirty_terrain_chunks().contains(&(0, 1)));
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
            clip_polygons.len() <= 3,
            "expected piece outer footprint loops, not per-band terrain clip cutters"
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
    fn incident_piece_mouth_sides_match_geometric_left_and_right() {
        let terrain = flat_terrain(64, 64);
        let mut graph = RegionGraph::new();
        let west = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let east = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(test_edge(
            west,
            center,
            vec![Vector3::new(-20.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.add_edge(test_edge(
            center,
            east,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);

        for incident in surface.sorted_incident_surface_edges(&graph, center) {
            let mouth = surface.build_incident_piece_mouth(incident).unwrap();
            let node_pos = graph.node(center).pos;
            let left_outer = *mouth.left.boundary_points_outer_to_inner.first().unwrap();
            let right_outer = *mouth.right.boundary_points_outer_to_inner.first().unwrap();
            let left_vec = Vector2::new(left_outer.x - node_pos.x, left_outer.z - node_pos.z);
            let right_vec = Vector2::new(right_outer.x - node_pos.x, right_outer.z - node_pos.z);
            let left_cross =
                incident.direction_xz.x * left_vec.y - incident.direction_xz.y * left_vec.x;
            let right_cross =
                incident.direction_xz.x * right_vec.y - incident.direction_xz.y * right_vec.x;
            assert!(
                left_cross > 0.0,
                "expected left side to stay geometrically left of the outward throat direction, got left_cross={left_cross:.3} direction={:?}",
                incident.direction_xz
            );
            assert!(
                right_cross < 0.0,
                "expected right side to stay geometrically right of the outward throat direction, got right_cross={right_cross:.3} direction={:?}",
                incident.direction_xz
            );
        }
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
            piece_a.sidewalk_surface_polygons.len() >= 3,
            "expected explicit JunctionN builder to emit multiple sidewalk sectors"
        );
        assert!(
            !piece_a.earthwork_outer_boundary_loops.is_empty(),
            "expected explicit visual node pieces to expose deterministic earthwork boundaries"
        );
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
    fn terrain_earthworks_integrate_paved_footprint_with_compiled_roadbed() {
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
            let actual = terrain.sample_visual_height_world(sample_x, sample_z)
                * crate::config::HEIGHT_SCALE;
            assert!(
                (actual - road_height).abs() <= 0.05,
                "expected stamped terrain to match the compiled paved surface at lateral_offset={lateral_offset:.1}: actual={actual:.3} road_height={road_height:.3}"
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
            let visual_height = terrain.sample_visual_height_world(sample_x, sample_z)
                * crate::config::HEIGHT_SCALE;
            sampled_profile.push((lateral_offset, road_height, visual_height));
            assert!(
                visual_height <= road_height + 0.01,
                "expected integrated terrain to stay at or below the bounded carriageway on a steep hillside: lateral_offset={lateral_offset:.2} visual_height={visual_height:.3} road_height={road_height:.3}"
            );
            assert!(
                (road_height - visual_height).abs() <= 0.08,
                "expected grounded-road integrated terrain under the footprint to follow the solved road surface instead of remaining a lowered support slab: lateral_offset={lateral_offset:.2} visual_height={visual_height:.3} road_height={road_height:.3}"
            );
        }

        let left = sampled_profile.first().unwrap();
        let right = sampled_profile.last().unwrap();
        let road_profile_delta = right.1 - left.1;
        let support_profile_delta = right.2 - left.2;
        assert!(
            (support_profile_delta - road_profile_delta).abs() <= 0.05,
            "expected paved-footprint support to follow the solved road crossfall instead of a flat slab: road_profile_delta={road_profile_delta:.3} support_profile_delta={support_profile_delta:.3}"
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
    fn mark_edge_dirty_expands_terrain_chunks_for_outer_earthwork_margin() {
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
        surface.mark_edge_dirty(&graph, edge_idx);

        assert!(surface.dirty_surface_chunks().contains(&(0, 0)));
        assert!(!surface.dirty_surface_chunks().contains(&(0, 1)));
        assert!(surface.dirty_terrain_chunks().contains(&(0, 1)));
        assert!(surface.dirty_terrain_chunks().contains(&(0, -1)));
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
            "grounded standard terminal earthwork margin should now be owned by integrated terrain instead of a separate visible earthwork carrier"
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
            "grounded standard span earthwork margin should now be owned by integrated terrain instead of a separate visible earthwork carrier"
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
