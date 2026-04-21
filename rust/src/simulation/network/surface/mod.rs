//! Authoritative road-surface ownership layer and the first live compiler slices.
//!
//! This module now owns both the Phase 1 cache / dirty-tracking shell and the
//! first Phase 2 compiler pass for deterministic edge sections plus node-patch
//! inputs. It now drives the shipped preview, committed render mesh, earthworks,
//! and world-surface query paths from one deterministic compiled roadbed cache.

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
const BRIDGE_ABUTMENT_LENGTH_M: f32 = 12.0;
const TUNNEL_PORTAL_STAMP_DEPTH_M: f32 = 1.0;

/// Chunk key used by the road-surface and earthwork caches.
pub type SurfaceChunkKey = (i32, i32);

/// Classification of one compiled node patch in the replacement road-surface model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoadSurfaceNodePatchClass {
    /// One incident surface edge ends here and requires a terminal patch.
    Terminal,
    /// Two nearly anti-parallel compatible edges hand corridor ownership edge-to-edge.
    PassThrough,
    /// A nearly straight node still requires a transition patch because widths differ.
    WidthTransition,
    /// A multi-arm or otherwise non-pass-through node requires a full junction patch.
    Junction,
}

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

/// One boundary sample used while assembling a node patch loop.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceNodeBoundaryPoint {
    /// Surface band that owns this boundary point.
    pub band_kind: RoadSurfaceBandKind,
    /// Polar angle around the node center in radians.
    pub angle_rad: f32,
    /// World-space boundary point in metres.
    pub point_world: Vector3,
}

/// One ordered boundary loop for a compiled node patch.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceNodeBoundaryLoop {
    /// Ordered loop boundary points.
    pub points: Vec<RoadSurfaceNodeBoundaryPoint>,
}

/// Compiled node-patch shell for the replacement road-surface system.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceNodePatch {
    /// Owning node id.
    pub node_id: u32,
    /// Node patch classification.
    pub class: RoadSurfaceNodePatchClass,
    /// Ordered outer roadbed boundary loops used by later deterministic triangulation.
    pub boundary_loops: Vec<RoadSurfaceNodeBoundaryLoop>,
    /// Ordered carriageway-owned boundary loops used by standard-road top-surface rendering.
    pub carriageway_boundary_loops: Vec<RoadSurfaceNodeBoundaryLoop>,
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
    /// Compiled terminal node patches for the temporary preview edge.
    pub compiled_node_patches: Vec<RoadSurfaceNodePatch>,
    /// Triangulated top-surface preview mesh vertices, lifted slightly for editor visibility.
    pub surface_vertices: Vec<Vector3>,
    /// Preview validity after grade and bridge / tunnel clearance checks.
    pub is_valid: bool,
}

#[derive(Default)]
pub(crate) struct RoadSurfaceDebugData {
    pub(crate) section_lines: Vec<Vector3>,
    pub(crate) band_lines: Vec<Vector3>,
    pub(crate) node_patch_lines: Vec<Vector3>,
    pub(crate) earthwork_chunk_lines: Vec<Vector3>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum IncidentEdgeSide {
    Start,
    End,
}

#[derive(Clone, Copy)]
enum NodeBoundarySelector {
    OuterRoadbed,
    Carriageway,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ChunkCacheKind {
    Surface,
    Earthwork,
}

#[derive(Clone, Copy)]
struct IncidentSurfaceEdge {
    edge_idx: usize,
    side: IncidentEdgeSide,
    direction_xz: Vector2,
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
    compiled_node_patches: HashMap<u32, RoadSurfaceNodePatch>,
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
            compiled_node_patches: HashMap::new(),
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

    /// Returns the currently cached compiled node patches by node id.
    pub fn compiled_node_patches(&self) -> &HashMap<u32, RoadSurfaceNodePatch> {
        &self.compiled_node_patches
    }

    /// Returns the current per-chunk surface cache shell.
    pub fn surface_chunk_cache(&self) -> &HashMap<SurfaceChunkKey, RoadSurfaceChunkCacheEntry> {
        &self.surface_chunk_cache
    }

    /// Returns the current per-chunk earthwork cache shell.
    pub fn earthwork_chunk_cache(&self) -> &HashMap<SurfaceChunkKey, RoadEarthworkChunkCacheEntry> {
        &self.earthwork_chunk_cache
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

        for node_id in node_ids {
            let Some(patch) = self.compiled_node_patches.get(&node_id) else {
                continue;
            };
            self.visit_visible_node_triangles(graph, terrain, node_id, patch, &mut |triangle| {
                let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                else {
                    return;
                };
                let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
            });
        }

        for edge_idx in edge_indices {
            let Some(sections) = self.compiled_sections.get(&edge_idx) else {
                continue;
            };
            self.visit_visible_edge_triangles(
                graph,
                terrain,
                edge_idx,
                sections,
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

        for node_id in node_ids {
            let Some(patch) = self.compiled_node_patches.get(&node_id) else {
                continue;
            };
            self.visit_visible_node_triangles(graph, terrain, node_id, patch, &mut |triangle| {
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

        for edge_idx in edge_indices {
            let Some(sections) = self.compiled_sections.get(&edge_idx) else {
                continue;
            };
            self.visit_visible_edge_triangles(
                graph,
                terrain,
                edge_idx,
                sections,
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
        }

        let mut node_ids: Vec<u32> = self.compiled_node_patches.keys().copied().collect();
        node_ids.sort_unstable();
        for node_id in node_ids {
            let Some(patch) = self.compiled_node_patches.get(&node_id) else {
                continue;
            };
            for boundary_loop in &patch.boundary_loops {
                let points: Vec<Vector3> = boundary_loop
                    .points
                    .iter()
                    .map(|point| point.point_world + Vector3::UP * 0.24)
                    .collect();
                if points.len() < 2 {
                    continue;
                }
                for index in 0..points.len() {
                    data.node_patch_lines.push(points[index]);
                    data.node_patch_lines
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
            if self.tunnel_throat_is_visible(edge_idx, edge, at_start, terrain) {
                has_visible_surface_attachment = true;
            }
        }

        has_supported_surface && has_visible_surface_attachment
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
                compiled_node_patches: Vec::new(),
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
        let compiled_node_patches = [start_node, end_node]
            .into_iter()
            .filter_map(|node_id| {
                preview_surface
                    .compiled_node_patches()
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
            compiled_node_patches,
            surface_vertices,
            is_valid,
        }
    }

    /// Clears compiled caches and dirty tracking without changing the configured chunk span.
    pub fn clear(&mut self) {
        self.clear_dirty_tracking();
        self.compiled_sections.clear();
        self.compiled_node_patches.clear();
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

        for node_id in sorted_nodes {
            if self.node_has_surface_edges(graph, node_id) {
                let patch = self.compile_node_patch(graph, node_id);
                self.compiled_node_patches.insert(node_id, patch);
            } else {
                self.compiled_node_patches.remove(&node_id);
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

        let node_ids = self.all_surface_node_ids(graph);
        for node_id in node_ids {
            let patch = self.compile_node_patch(graph, node_id);
            self.compiled_node_patches.insert(node_id, patch);
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
                if edge_idx >= graph.edge_count() {
                    continue;
                }
                let edge = graph.edge(edge_idx);
                let Some(sections) = self.compiled_sections.get(&edge_idx) else {
                    continue;
                };
                self.stamp_edge_earthworks_for_chunk(edge, sections, chunk, terrain);
            }

            for &node_id in &entry.node_ids {
                if node_id as usize >= graph.node_count() {
                    continue;
                }
                let Some(patch) = self.compiled_node_patches.get(&node_id) else {
                    continue;
                };
                self.stamp_node_patch_earthworks_for_chunk(graph, node_id, patch, chunk, terrain);
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

    fn compile_node_patch(&self, graph: &RegionGraph, node_id: u32) -> RoadSurfaceNodePatch {
        let valid = graph.get_valid_node(node_id);
        let incidents = self.collect_incident_surface_edges(graph, valid);
        let class = self.classify_node_patch(graph, &incidents);
        let boundary_loops = self.build_node_patch_boundary_loops(
            graph,
            valid,
            class,
            &incidents,
            NodeBoundarySelector::OuterRoadbed,
        );
        let carriageway_boundary_loops = self.build_node_patch_boundary_loops(
            graph,
            valid,
            class,
            &incidents,
            NodeBoundarySelector::Carriageway,
        );

        RoadSurfaceNodePatch {
            node_id: valid,
            class,
            boundary_loops,
            carriageway_boundary_loops,
        }
    }

    fn classify_node_patch(
        &self,
        graph: &RegionGraph,
        incidents: &[IncidentSurfaceEdge],
    ) -> RoadSurfaceNodePatchClass {
        match incidents.len() {
            0 | 1 => RoadSurfaceNodePatchClass::Terminal,
            2 => {
                let a = incidents[0];
                let b = incidents[1];
                let straight = a.direction_xz.dot(b.direction_xz) <= -PASS_THROUGH_DOT_THRESHOLD;
                if !straight {
                    return RoadSurfaceNodePatchClass::Junction;
                }
                let compatible = self.sections_are_profile_compatible(graph, a, b);
                if compatible {
                    RoadSurfaceNodePatchClass::PassThrough
                } else {
                    RoadSurfaceNodePatchClass::WidthTransition
                }
            }
            _ => RoadSurfaceNodePatchClass::Junction,
        }
    }

    fn sections_are_profile_compatible(
        &self,
        graph: &RegionGraph,
        a: IncidentSurfaceEdge,
        b: IncidentSurfaceEdge,
    ) -> bool {
        let edge_a = graph.edge(a.edge_idx);
        let edge_b = graph.edge(b.edge_idx);
        if edge_a.class != edge_b.class {
            return false;
        }

        let Some(section_a) = self.throat_section_for_incident(graph, a) else {
            return false;
        };
        let Some(section_b) = self.throat_section_for_incident(graph, b) else {
            return false;
        };

        if section_a.bands.len() != section_b.bands.len() {
            return false;
        }

        section_a
            .bands
            .iter()
            .zip(&section_b.bands)
            .all(|(band_a, band_b)| {
                band_a.kind == band_b.kind
                    && ((band_a.lateral_end_m - band_a.lateral_start_m)
                        - (band_b.lateral_end_m - band_b.lateral_start_m))
                        .abs()
                        <= BAND_WIDTH_MATCH_EPSILON_M
            })
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
            if edge_idx >= graph.edge_count() || !self.compiled_sections.contains_key(&edge_idx) {
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
            let Some(section) = self.throat_section_for_side(edge_idx, edge, side) else {
                continue;
            };
            let mut direction_xz = section.tangent_xz;
            if side == IncidentEdgeSide::End {
                direction_xz = -direction_xz;
            }
            incidents.push(IncidentSurfaceEdge {
                edge_idx,
                side,
                direction_xz: direction_xz.normalized(),
            });
        }

        incidents.sort_by(|a, b| a.edge_idx.cmp(&b.edge_idx).then(a.side.cmp(&b.side)));
        incidents
    }

    fn build_node_patch_boundary_loops(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        class: RoadSurfaceNodePatchClass,
        incidents: &[IncidentSurfaceEdge],
        selector: NodeBoundarySelector,
    ) -> Vec<RoadSurfaceNodeBoundaryLoop> {
        if class == RoadSurfaceNodePatchClass::PassThrough {
            return Vec::new();
        }

        if class == RoadSurfaceNodePatchClass::Terminal {
            return incidents
                .first()
                .and_then(|incident| {
                    self.build_terminal_boundary_loop(graph, node_id, *incident, selector)
                })
                .into_iter()
                .collect();
        }

        let mut points = Vec::new();
        for incident in incidents {
            if let Some(mut throat_points) =
                self.build_throat_boundary_points(graph, node_id, *incident, selector)
            {
                points.append(&mut throat_points);
            }
        }
        self.finalize_boundary_loop(points).into_iter().collect()
    }

    fn finalize_boundary_loop(
        &self,
        mut points: Vec<RoadSurfaceNodeBoundaryPoint>,
    ) -> Option<RoadSurfaceNodeBoundaryLoop> {
        if points.len() < 3 {
            return None;
        }

        points.sort_by(|a, b| {
            a.angle_rad
                .total_cmp(&b.angle_rad)
                .then_with(|| a.point_world.x.total_cmp(&b.point_world.x))
                .then_with(|| a.point_world.z.total_cmp(&b.point_world.z))
                .then_with(|| a.point_world.y.total_cmp(&b.point_world.y))
        });
        points.dedup_by(|a, b| (a.point_world - b.point_world).length_squared() <= 0.0001);
        if points.len() < 3 {
            return None;
        }
        Some(RoadSurfaceNodeBoundaryLoop { points })
    }

    fn build_throat_boundary_points(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        incident: IncidentSurfaceEdge,
        selector: NodeBoundarySelector,
    ) -> Option<Vec<RoadSurfaceNodeBoundaryPoint>> {
        let section = self.throat_section_for_incident(graph, incident)?;
        let node_pos = graph.node(node_id).pos;
        let (first_kind, first_point, last_kind, last_point) =
            self.selected_boundary_points(section, selector)?;

        Some(vec![
            RoadSurfaceNodeBoundaryPoint {
                band_kind: first_kind,
                angle_rad: (first_point.z - node_pos.z).atan2(first_point.x - node_pos.x),
                point_world: first_point,
            },
            RoadSurfaceNodeBoundaryPoint {
                band_kind: last_kind,
                angle_rad: (last_point.z - node_pos.z).atan2(last_point.x - node_pos.x),
                point_world: last_point,
            },
        ])
    }

    fn build_terminal_boundary_loop(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        incident: IncidentSurfaceEdge,
        selector: NodeBoundarySelector,
    ) -> Option<RoadSurfaceNodeBoundaryLoop> {
        let section = self.throat_section_for_incident(graph, incident)?;
        let node_pos = graph.node(node_id).pos;
        let (first_kind, first_point, last_kind, last_point) =
            self.selected_boundary_points(section, selector)?;
        let cap_depth = Vector2::new(first_point.x - node_pos.x, first_point.z - node_pos.z)
            .length()
            .max(Vector2::new(last_point.x - node_pos.x, last_point.z - node_pos.z).length())
            .max(0.5);
        let cap_offset = -incident.direction_xz.normalized() * cap_depth;
        let midpoint_height = (first_point.y + last_point.y) * 0.5;
        let outside_left = Vector3::new(
            first_point.x + cap_offset.x,
            first_point.y,
            first_point.z + cap_offset.y,
        );
        let outside_mid = Vector3::new(
            node_pos.x + cap_offset.x,
            midpoint_height,
            node_pos.z + cap_offset.y,
        );
        let outside_right = Vector3::new(
            last_point.x + cap_offset.x,
            last_point.y,
            last_point.z + cap_offset.y,
        );

        self.finalize_boundary_loop(vec![
            RoadSurfaceNodeBoundaryPoint {
                band_kind: first_kind,
                angle_rad: (first_point.z - node_pos.z).atan2(first_point.x - node_pos.x),
                point_world: first_point,
            },
            RoadSurfaceNodeBoundaryPoint {
                band_kind: first_kind,
                angle_rad: (outside_left.z - node_pos.z).atan2(outside_left.x - node_pos.x),
                point_world: outside_left,
            },
            RoadSurfaceNodeBoundaryPoint {
                band_kind: first_kind,
                angle_rad: (outside_mid.z - node_pos.z).atan2(outside_mid.x - node_pos.x),
                point_world: outside_mid,
            },
            RoadSurfaceNodeBoundaryPoint {
                band_kind: last_kind,
                angle_rad: (outside_right.z - node_pos.z).atan2(outside_right.x - node_pos.x),
                point_world: outside_right,
            },
            RoadSurfaceNodeBoundaryPoint {
                band_kind: last_kind,
                angle_rad: (last_point.z - node_pos.z).atan2(last_point.x - node_pos.x),
                point_world: last_point,
            },
        ])
    }

    fn selected_boundary_points(
        &self,
        section: &RoadSurfaceSection,
        selector: NodeBoundarySelector,
    ) -> Option<(RoadSurfaceBandKind, Vector3, RoadSurfaceBandKind, Vector3)> {
        match selector {
            NodeBoundarySelector::OuterRoadbed => {
                let first_band = section.bands.first()?;
                let last_band = section.bands.last()?;
                Some((
                    first_band.kind,
                    self.section_boundary_world_point(
                        section,
                        first_band.lateral_start_m,
                        first_band.height_start_m,
                    ),
                    last_band.kind,
                    self.section_boundary_world_point(
                        section,
                        last_band.lateral_end_m,
                        last_band.height_end_m,
                    ),
                ))
            }
            NodeBoundarySelector::Carriageway => {
                let mut carriageway_bands = section
                    .bands
                    .iter()
                    .filter(|band| band.kind == RoadSurfaceBandKind::Carriageway);
                let first_band = carriageway_bands.next()?;
                let last_band = carriageway_bands.last().unwrap_or(first_band);
                Some((
                    first_band.kind,
                    self.section_boundary_world_point(
                        section,
                        first_band.lateral_start_m,
                        first_band.height_start_m,
                    ),
                    last_band.kind,
                    self.section_boundary_world_point(
                        section,
                        last_band.lateral_end_m,
                        last_band.height_end_m,
                    ),
                ))
            }
        }
    }

    fn throat_section_for_incident(
        &self,
        graph: &RegionGraph,
        incident: IncidentSurfaceEdge,
    ) -> Option<&RoadSurfaceSection> {
        if incident.edge_idx >= graph.edge_count() {
            return None;
        }
        let edge = graph.edge(incident.edge_idx);
        self.throat_section_for_side(incident.edge_idx, edge, incident.side)
    }

    fn throat_section_for_side(
        &self,
        edge_idx: usize,
        edge: &Edge,
        side: IncidentEdgeSide,
    ) -> Option<&RoadSurfaceSection> {
        let sections = self.compiled_sections.get(&edge_idx)?;
        if sections.is_empty() {
            return None;
        }
        let total_length = sections.last()?.s_m.max(0.0);
        let target_s = match side {
            IncidentEdgeSide::Start => edge.start_clip.clamp(0.0, total_length),
            IncidentEdgeSide::End => (total_length - edge.end_clip).clamp(0.0, total_length),
        };

        sections.iter().min_by(|a, b| {
            (a.s_m - target_s)
                .abs()
                .total_cmp(&(b.s_m - target_s).abs())
                .then_with(|| a.s_m.total_cmp(&b.s_m))
        })
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
                *edge_idx < graph.edge_count()
                    && self.compiled_sections.contains_key(edge_idx)
                    && self.edge_overlaps_chunk(graph.edge(*edge_idx), chunk, kind)
            })
            .collect();
        edge_indices.sort_unstable();
        edge_indices.dedup();

        let mut node_ids: Vec<u32> = self
            .compiled_node_patches
            .iter()
            .filter_map(|(&node_id, patch)| {
                self.node_patch_overlaps_chunk(graph, node_id, patch, chunk, kind)
                    .then_some(node_id)
            })
            .collect();
        node_ids.sort_unstable();
        node_ids.dedup();

        (edge_indices, node_ids)
    }

    fn node_patch_overlaps_chunk(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        patch: &RoadSurfaceNodePatch,
        chunk: SurfaceChunkKey,
        kind: ChunkCacheKind,
    ) -> bool {
        if patch.class == RoadSurfaceNodePatchClass::PassThrough {
            return false;
        }
        let Some((min, max)) = self.node_patch_bounds(graph, node_id, patch, kind) else {
            return false;
        };

        let min_chunk = self.chunk_coords_for_world(min.x, min.z);
        let max_chunk = self.chunk_coords_for_world(max.x, max.z);
        (node_id as usize) < graph.node_count()
            && chunk.0 >= min_chunk.0
            && chunk.0 <= max_chunk.0
            && chunk.1 >= min_chunk.1
            && chunk.1 <= max_chunk.1
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
            }
        }

        let mut edge_indices: Vec<usize> = edge_indices.into_iter().collect();
        edge_indices.sort_unstable();
        let mut node_ids: Vec<u32> = node_ids.into_iter().collect();
        node_ids.sort_unstable();
        (edge_indices, node_ids)
    }

    fn visit_visible_edge_triangles<F>(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_idx: usize,
        sections: &[RoadSurfaceSection],
        visitor: &mut F,
    ) where
        F: FnMut([Vector3; 3]),
    {
        for (start_index, end_index) in
            self.visible_section_ranges_for_edge(graph, terrain, edge_idx, sections)
        {
            if end_index <= start_index {
                continue;
            }
            for pair in sections[start_index..=end_index].windows(2) {
                let profile_a = self.section_profile_world_points(&pair[0], 0.0);
                let profile_b = self.section_profile_world_points(&pair[1], 0.0);
                if profile_a.len() < 2 || profile_a.len() != profile_b.len() {
                    continue;
                }

                for index in 0..profile_a.len() - 1 {
                    let a0 = profile_a[index];
                    let a1 = profile_a[index + 1];
                    let b0 = profile_b[index];
                    let b1 = profile_b[index + 1];
                    visitor([a0, b0, b1]);
                    visitor([a0, b1, a1]);
                }
            }
        }
    }

    fn visit_visible_node_triangles<F>(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        patch: &RoadSurfaceNodePatch,
        visitor: &mut F,
    ) where
        F: FnMut([Vector3; 3]),
    {
        if patch.class == RoadSurfaceNodePatchClass::PassThrough
            || !self.node_uses_visible_surface(graph, terrain, node_id)
        {
            return;
        }

        for boundary_loop in &patch.boundary_loops {
            let points: Vec<Vector3> = boundary_loop
                .points
                .iter()
                .map(|point| point.point_world)
                .collect();
            if points.len() < 3 {
                continue;
            }
            let mut center = Vector3::ZERO;
            for point in &points {
                center += *point;
            }
            center /= points.len() as f32;
            for index in 0..points.len() {
                visitor([center, points[index], points[(index + 1) % points.len()]]);
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
        let start_class = self
            .compiled_node_patches
            .get(&graph.get_valid_node(edge.start_node))
            .map(|patch| patch.class);
        let end_class = self
            .compiled_node_patches
            .get(&graph.get_valid_node(edge.end_node))
            .map(|patch| patch.class);
        let start_handoff = if matches!(
            start_class,
            Some(RoadSurfaceNodePatchClass::Junction | RoadSurfaceNodePatchClass::WidthTransition)
        ) {
            edge.start_clip.clamp(0.0, total_length)
        } else {
            0.0
        };
        let end_handoff = if matches!(
            end_class,
            Some(RoadSurfaceNodePatchClass::Junction | RoadSurfaceNodePatchClass::WidthTransition)
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

    fn outer_surface_bounds(
        &self,
        section: &RoadSurfaceSection,
    ) -> Option<((f32, f32), (f32, f32))> {
        let first_band = section.bands.first()?;
        let last_band = section.bands.last()?;
        Some((
            (first_band.lateral_start_m, first_band.height_start_m),
            (last_band.lateral_end_m, last_band.height_end_m),
        ))
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

    fn stamp_standard_edge_earthwork_margins_for_chunk(
        &self,
        sections: &[RoadSurfaceSection],
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
    ) {
        if sections.len() < 2 {
            return;
        }

        for pair in sections.windows(2) {
            let Some((left_a, right_a)) = self.outer_surface_bounds(&pair[0]) else {
                continue;
            };
            let Some((left_b, right_b)) = self.outer_surface_bounds(&pair[1]) else {
                continue;
            };

            self.stamp_edge_earthwork_margin_side(
                &pair[0], left_a.0, left_a.1, &pair[1], left_b.0, left_b.1, -1.0, chunk, terrain,
            );
            self.stamp_edge_earthwork_margin_side(
                &pair[0], right_a.0, right_a.1, &pair[1], right_b.0, right_b.1, 1.0, chunk, terrain,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stamp_edge_earthwork_margin_side(
        &self,
        section_a: &RoadSurfaceSection,
        lateral_a: f32,
        height_a: f32,
        section_b: &RoadSurfaceSection,
        lateral_b: f32,
        height_b: f32,
        outward_sign: f32,
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
    ) {
        let road_a = self.section_boundary_world_point(section_a, lateral_a, height_a);
        let road_b = self.section_boundary_world_point(section_b, lateral_b, height_b);
        let outer_a =
            self.earthwork_transition_point(road_a, section_a.lateral_xz * outward_sign, terrain);
        let outer_b =
            self.earthwork_transition_point(road_b, section_b.lateral_xz * outward_sign, terrain);

        self.stamp_triangle_to_chunk(terrain, chunk, [road_a, road_b, outer_b]);
        self.stamp_triangle_to_chunk(terrain, chunk, [road_a, outer_b, outer_a]);
    }

    fn stamp_node_patch_earthwork_margins_for_chunk(
        &self,
        boundary_loop: &RoadSurfaceNodeBoundaryLoop,
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
    ) {
        if boundary_loop.points.len() < 3 {
            return;
        }

        let mut centroid_xz = Vector2::ZERO;
        for point in &boundary_loop.points {
            centroid_xz += Vector2::new(point.point_world.x, point.point_world.z);
        }
        centroid_xz /= boundary_loop.points.len() as f32;

        let outer_points: Vec<Vector3> = boundary_loop
            .points
            .iter()
            .map(|point| {
                let outward = Vector2::new(
                    point.point_world.x - centroid_xz.x,
                    point.point_world.z - centroid_xz.y,
                );
                self.earthwork_transition_point(point.point_world, outward, terrain)
            })
            .collect();

        for index in 0..boundary_loop.points.len() {
            let current = boundary_loop.points[index].point_world;
            let next = boundary_loop.points[(index + 1) % boundary_loop.points.len()].point_world;
            let outer_current = outer_points[index];
            let outer_next = outer_points[(index + 1) % outer_points.len()];
            self.stamp_triangle_to_chunk(terrain, chunk, [current, next, outer_next]);
            self.stamp_triangle_to_chunk(terrain, chunk, [current, outer_next, outer_current]);
        }
    }

    fn stamp_edge_earthworks_for_chunk(
        &self,
        edge: &Edge,
        sections: &[RoadSurfaceSection],
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
    ) {
        for (start_index, end_index) in
            self.earthwork_section_ranges_for_edge(edge, sections, terrain)
        {
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

                    let a0 = self.section_boundary_world_point(
                        &pair[0],
                        band_a.lateral_start_m,
                        band_a.height_start_m,
                    );
                    let a1 = self.section_boundary_world_point(
                        &pair[0],
                        band_a.lateral_end_m,
                        band_a.height_end_m,
                    );
                    let b0 = self.section_boundary_world_point(
                        &pair[1],
                        band_b.lateral_start_m,
                        band_b.height_start_m,
                    );
                    let b1 = self.section_boundary_world_point(
                        &pair[1],
                        band_b.lateral_end_m,
                        band_b.height_end_m,
                    );
                    self.stamp_triangle_to_chunk(terrain, chunk, [a0, b0, b1]);
                    self.stamp_triangle_to_chunk(terrain, chunk, [a0, b1, a1]);
                }
            }

            if edge.class == EdgeClass::Standard {
                self.stamp_standard_edge_earthwork_margins_for_chunk(
                    &sections[start_index..=end_index],
                    chunk,
                    terrain,
                );
            }
        }
    }

    fn stamp_node_patch_earthworks_for_chunk(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        patch: &RoadSurfaceNodePatch,
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
    ) {
        if patch.class == RoadSurfaceNodePatchClass::PassThrough
            || !self.node_patch_uses_earthworks(graph, node_id, terrain)
        {
            return;
        }

        for boundary_loop in &patch.boundary_loops {
            let points: Vec<Vector3> = boundary_loop
                .points
                .iter()
                .map(|point| point.point_world)
                .collect();
            if points.len() < 3 {
                continue;
            }
            let mut center = Vector3::ZERO;
            for point in &points {
                center += *point;
            }
            center /= points.len() as f32;

            for index in 0..points.len() {
                self.stamp_triangle_to_chunk(
                    terrain,
                    chunk,
                    [center, points[index], points[(index + 1) % points.len()]],
                );
            }

            self.stamp_node_patch_earthwork_margins_for_chunk(boundary_loop, chunk, terrain);
        }
    }

    fn node_patch_uses_earthworks(
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
            if self.tunnel_throat_is_visible(edge_idx, edge, at_start, terrain) {
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
        edge: &Edge,
        at_start: bool,
        terrain: &TerrainSystem,
    ) -> bool {
        self.throat_section_for_side(
            edge_idx,
            edge,
            if at_start {
                IncidentEdgeSide::Start
            } else {
                IncidentEdgeSide::End
            },
        )
        .map(|section| self.section_is_tunnel_surface_visible(section, terrain))
        .unwrap_or(false)
    }

    fn stamp_triangle_to_chunk(
        &self,
        terrain: &mut TerrainSystem,
        chunk: SurfaceChunkKey,
        triangle: [Vector3; 3],
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
            .max(chunk_min.x);
        let max_x = triangle
            .iter()
            .map(|point| point.x)
            .fold(chunk_min.x, f32::max)
            .min(chunk_max.x);
        let min_z = triangle
            .iter()
            .map(|point| point.z)
            .fold(chunk_max.z, f32::min)
            .max(chunk_min.z);
        let max_z = triangle
            .iter()
            .map(|point| point.z)
            .fold(chunk_min.z, f32::max)
            .min(chunk_max.z);
        let Some((min_grid_x, max_grid_x, min_grid_z, max_grid_z)) =
            terrain.grid_rect_for_world_bounds(min_x, min_z, max_x, max_z)
        else {
            return;
        };

        for grid_z in min_grid_z..=max_grid_z {
            for grid_x in min_grid_x..=max_grid_x {
                let (world_x, world_z) = terrain.grid_to_world_coords(grid_x, grid_z);
                let Some((wa, wb, wc)) =
                    Self::triangle_barycentric_weights_xz(triangle, Vector2::new(world_x, world_z))
                else {
                    continue;
                };
                let support_height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                let target_sample =
                    (support_height_m - EARTHWORK_PAVEMENT_DEPTH_M) / config::HEIGHT_SCALE;
                let source_sample = terrain.get_height(grid_x, grid_z);
                let current_visual = terrain.visual_height_at_grid(grid_x, grid_z);
                let blended = if target_sample < source_sample {
                    current_visual.min(target_sample)
                } else {
                    current_visual.max(target_sample)
                };
                terrain.set_visual_height_at_grid(grid_x, grid_z, blended);
            }
        }
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
        let surface_chunks = self.edge_chunks(edge, ChunkCacheKind::Surface);
        let earthwork_chunks = self.edge_chunks(edge, ChunkCacheKind::Earthwork);

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
        self.compiled_node_patches
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
        graph: &RegionGraph,
        kind: ChunkCacheKind,
    ) -> Vec<SurfaceChunkKey> {
        let mut chunks = HashSet::new();
        for (&edge_idx, _) in &self.compiled_sections {
            if edge_idx < graph.edge_count() {
                for chunk in self.edge_chunks(graph.edge(edge_idx), kind) {
                    chunks.insert(chunk);
                }
            }
        }
        for (&node_id, patch) in &self.compiled_node_patches {
            if patch.class == RoadSurfaceNodePatchClass::PassThrough {
                continue;
            }
            if node_id as usize >= graph.node_count() {
                continue;
            }
            let Some((min, max)) = self.node_patch_bounds(graph, node_id, patch, kind) else {
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

    fn node_patch_bounds(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        patch: &RoadSurfaceNodePatch,
        kind: ChunkCacheKind,
    ) -> Option<(Vector3, Vector3)> {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_z = f32::MAX;
        let mut max_z = f32::MIN;
        let mut saw_point = false;

        if (node_id as usize) < graph.node_count() {
            let pos = graph.node(node_id).pos;
            min_x = min_x.min(pos.x);
            max_x = max_x.max(pos.x);
            min_z = min_z.min(pos.z);
            max_z = max_z.max(pos.z);
            saw_point = true;
        }

        for point in patch
            .boundary_loops
            .iter()
            .flat_map(|boundary_loop| boundary_loop.points.iter())
        {
            min_x = min_x.min(point.point_world.x);
            max_x = max_x.max(point.point_world.x);
            min_z = min_z.min(point.point_world.z);
            max_z = max_z.max(point.point_world.z);
            saw_point = true;
        }

        if !saw_point {
            return None;
        }

        if kind == ChunkCacheKind::Earthwork {
            min_x -= EARTHWORK_MAX_MARGIN_M;
            max_x += EARTHWORK_MAX_MARGIN_M;
            min_z -= EARTHWORK_MAX_MARGIN_M;
            max_z += EARTHWORK_MAX_MARGIN_M;
        }

        Some((
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

    fn edge_chunks(&self, edge: &Edge, kind: ChunkCacheKind) -> Vec<SurfaceChunkKey> {
        if !Self::is_surface_edge(edge) {
            return Vec::new();
        }
        let Some((min, max)) = self.edge_bounds(edge, kind) else {
            return Vec::new();
        };

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

    fn edge_overlaps_chunk(
        &self,
        edge: &Edge,
        chunk: SurfaceChunkKey,
        kind: ChunkCacheKind,
    ) -> bool {
        let Some((min, max)) = self.edge_bounds(edge, kind) else {
            return false;
        };

        let min_chunk = self.chunk_coords_for_world(min.x, min.z);
        let max_chunk = self.chunk_coords_for_world(max.x, max.z);
        chunk.0 >= min_chunk.0
            && chunk.0 <= max_chunk.0
            && chunk.1 >= min_chunk.1
            && chunk.1 <= max_chunk.1
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EARTHWORK_MAX_MARGIN_M, EARTHWORK_PAVEMENT_DEPTH_M, PreviewRoadSurfaceResult,
        RoadSurfaceNodePatch, RoadSurfaceNodePatchClass, RoadSurfaceSection, RoadSurfaceSystem,
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
                let visual_height_m = terrain.sample_visual_height_world(sample_x, sample_z)
                    * crate::config::HEIGHT_SCALE;
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
    ) -> (RoadSurfaceSystem, TerrainSystem, usize) {
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
        (surface, terrain, edge_idx)
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
        Vec<RoadSurfaceNodePatch>,
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
        let compiled_node_patches = [start_node, end_node]
            .into_iter()
            .filter_map(|node_id| committed.compiled_node_patches().get(&node_id).cloned())
            .collect();
        (preview, compiled_sections, compiled_node_patches)
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
            return Some(band.height_start_m + (band.height_end_m - band.height_start_m) * t);
        }

        None
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
    fn node_classification_matches_surface_profiles() {
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
        assert_eq!(
            pass_surface.compiled_node_patches().get(&pb).unwrap().class,
            RoadSurfaceNodePatchClass::PassThrough
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
        assert_eq!(
            width_surface
                .compiled_node_patches()
                .get(&wb)
                .unwrap()
                .class,
            RoadSurfaceNodePatchClass::WidthTransition
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
                .compiled_node_patches()
                .get(&jb)
                .unwrap()
                .class,
            RoadSurfaceNodePatchClass::Junction
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
                .compiled_node_patches()
                .get(&ta)
                .unwrap()
                .class,
            RoadSurfaceNodePatchClass::Terminal
        );
    }

    #[test]
    fn throat_boundary_loops_are_angle_sorted_and_stable() {
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

        let terrain = flat_terrain(64, 64);
        let mut surface_a = RoadSurfaceSystem::new(16.0);
        let mut surface_b = RoadSurfaceSystem::new(16.0);
        surface_a.compile_dirty(&graph, &terrain);
        surface_b.compile_dirty(&graph, &terrain);

        let patch_a = surface_a.compiled_node_patches().get(&center).unwrap();
        let patch_b = surface_b.compiled_node_patches().get(&center).unwrap();
        assert_eq!(patch_a.class, RoadSurfaceNodePatchClass::Junction);
        assert_eq!(patch_a, patch_b);
        assert_eq!(patch_a.boundary_loops.len(), 1);
        assert_eq!(patch_a.boundary_loops[0].points.len(), 4);
        assert!(
            patch_a.boundary_loops[0]
                .points
                .windows(2)
                .all(|pair| pair[0].angle_rad <= pair[1].angle_rad)
        );
    }

    #[test]
    fn preview_matches_committed_sections_on_flat_terrain() {
        let terrain = flat_terrain(64, 64);
        let surface = RoadSurfaceSystem::new(16.0);
        let raw_points = vec![Vector3::new(0.0, 0.2, 0.0), Vector3::new(24.0, 0.2, 0.0)];

        let (preview, committed_sections, committed_patches) =
            compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

        assert_eq!(preview.edge_class, EdgeClass::Standard);
        assert!(preview.is_valid);
        assert_eq!(preview.compiled_sections, committed_sections);
        assert_eq!(preview.compiled_node_patches, committed_patches);
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

        let (preview, committed_sections, committed_patches) =
            compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

        assert_eq!(preview.edge_class, EdgeClass::Standard);
        assert!(preview.is_valid);
        assert_eq!(preview.compiled_sections, committed_sections);
        assert_eq!(preview.compiled_node_patches, committed_patches);
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

        let (preview, committed_sections, committed_patches) =
            compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

        assert_eq!(preview.edge_class, EdgeClass::Bridge);
        assert!(preview.is_valid);
        assert_eq!(preview.compiled_sections, committed_sections);
        assert_eq!(preview.compiled_node_patches, committed_patches);
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

        let (preview, committed_sections, committed_patches) =
            compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

        assert_eq!(preview.edge_class, EdgeClass::Tunnel);
        assert!(preview.is_valid);
        assert_eq!(preview.compiled_sections, committed_sections);
        assert_eq!(preview.compiled_node_patches, committed_patches);
    }

    #[test]
    fn terrain_earthworks_match_compiled_roadbed_inside_paved_footprint() {
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
            let expected = section_height_at_lateral_offset(section, lateral_offset).unwrap()
                - EARTHWORK_PAVEMENT_DEPTH_M;
            let sample_x = section.center_xz.x + section.lateral_xz.x * lateral_offset;
            let sample_z = section.center_xz.y + section.lateral_xz.y * lateral_offset;
            let actual = terrain.sample_visual_height_world(sample_x, sample_z)
                * crate::config::HEIGHT_SCALE;
            assert!(
                (actual - expected).abs() <= 0.12,
                "expected stamped terrain to follow compiled roadbed at lateral_offset={lateral_offset:.1}: actual={actual:.3} expected={expected:.3}"
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

        for lateral_offset in [-half_carriageway * 0.8, 0.0, half_carriageway * 0.8] {
            let road_height = section_height_at_lateral_offset(section, lateral_offset).unwrap();
            let sample_x = section.center_xz.x + section.lateral_xz.x * lateral_offset;
            let sample_z = section.center_xz.y + section.lateral_xz.y * lateral_offset;
            let visual_height = terrain.sample_visual_height_world(sample_x, sample_z)
                * crate::config::HEIGHT_SCALE;
            assert!(
                visual_height <= road_height - 0.01,
                "expected earthworks to keep visual terrain below the bounded carriageway on a steep hillside: lateral_offset={lateral_offset:.2} visual_height={visual_height:.3} road_height={road_height:.3}"
            );
        }
    }

    #[test]
    fn coarse_10m_hillside_case_has_material_footprint_overlap() {
        let (surface, terrain, edge_idx) = build_coarse_grid_hillside_case(10.0);
        let metrics = measure_max_footprint_overflow(&surface, edge_idx, &terrain);

        assert!(
            metrics.max_overflow_m >= 0.20,
            "expected the current 10 m terrain grid characterization case to show material road-footprint overlap, got {metrics:?}"
        );
    }

    #[test]
    fn coarse_5m_hillside_case_improves_on_10m_overlap() {
        let (coarse_surface, coarse_terrain, coarse_edge_idx) =
            build_coarse_grid_hillside_case(10.0);
        let (fine_surface, fine_terrain, fine_edge_idx) = build_coarse_grid_hillside_case(5.0);
        let coarse_metrics =
            measure_max_footprint_overflow(&coarse_surface, coarse_edge_idx, &coarse_terrain);
        let fine_metrics =
            measure_max_footprint_overflow(&fine_surface, fine_edge_idx, &fine_terrain);

        assert!(
            coarse_metrics.max_overflow_m >= 0.20,
            "expected the coarse reference case to remain meaningfully bad, got coarse={coarse_metrics:?} fine={fine_metrics:?}"
        );
        assert!(
            fine_metrics.max_overflow_m + 0.05 < coarse_metrics.max_overflow_m,
            "expected the same hillside case to improve on a 5 m grid, got coarse={coarse_metrics:?} fine={fine_metrics:?}"
        );
    }

    #[test]
    fn grounded_hillside_earthworks_cut_uphill_and_fill_downhill_outside_footprint() {
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
        let side_a_delta = side_a_actual - side_a_source;
        let side_b_delta = side_b_actual - side_b_source;

        assert!(
            (side_a_delta <= -0.15 && side_b_delta >= 0.15)
                || (side_b_delta <= -0.15 && side_a_delta >= 0.15),
            "expected one hillside margin side to cut and the other to fill, got side_a_delta={side_a_delta:.3} side_b_delta={side_b_delta:.3}"
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
        assert!(!debug.node_patch_lines.is_empty());
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
