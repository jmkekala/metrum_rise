//! Refined road-clipped terrain build and cache payload contracts.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::simulation::network::surface::SurfaceChunkKey;
use crate::simulation::terrain::TerrainPatchSnapshot;
use crate::simulation::terrain::cdt::{
    TerrainCdtError, TerrainCdtInput, TerrainCdtMesh, TerrainCdtPatch,
};
use godot::prelude::{Vector2, Vector3};

/// Fine render step used for terrain patches whose topology is clipped by visible road surfaces.
pub(crate) const ROAD_LOCKED_TERRAIN_RENDER_STEP_M: f32 = 2.0;

/// Pending refined-terrain invalidation stamps for one render patch.
#[derive(Clone, Debug, Default)]
pub(crate) struct RefinedTerrainAssemblyLedger {
    /// Latest generation requiring a full-patch input rebuild.
    pub(crate) full_dirty_at: Option<u64>,
    /// Latest local-road generation touching each fixed 32 m spatial-query chunk.
    pub(crate) road_query_chunk_dirty_at: BTreeMap<SurfaceChunkKey, u64>,
}

/// Input-assembly scope captured for one immutable refined-terrain request.
pub(crate) enum RefinedTerrainAssemblyScope {
    /// Query and assemble the complete render patch.
    FullPatch,
    /// Query and assemble only these fixed world-aligned 64 m tile coordinates.
    LocalTiles(Vec<(i64, i64)>),
}

/// Cache key for one production refined terrain patch mesh.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RefinedTerrainPatchCacheKey {
    /// Terrain render-patch X index.
    pub(crate) patch_x: usize,
    /// Terrain render-patch Z index.
    pub(crate) patch_z: usize,
    /// Refined render step quantized to millimetres.
    pub(crate) render_step_mm: u32,
}

/// Complete input needed to build a refined road-clipped terrain patch off the Godot frame.
pub(crate) struct RefinedTerrainPatchBuildInput {
    /// Cache key for the produced patch.
    pub(crate) key: RefinedTerrainPatchCacheKey,
    /// Patch-local terrain source generation captured when this input was assembled.
    pub(crate) surface_generation: u64,
    /// Base visual terrain patch snapshot.
    pub(crate) patch: TerrainPatchSnapshot,
    /// Local CDT windows assembled from source terrain samples and road footprint loops.
    pub(crate) windows: Vec<RefinedTerrainCdtWindowBuildInput>,
    /// Previous compiled windows carried directly without rebuilding their inputs.
    pub(crate) reused_windows: Vec<Arc<CachedRefinedTerrainCdtWindow>>,
    /// Number of unique authoritative clip loops represented by the window plan.
    pub(crate) input_clip_loop_count: usize,
    /// Padded-query loops whose exact grading influence does not intersect this patch.
    pub(crate) omitted_margin_clip_loop_count: usize,
    /// Stable road-contributor ids expected in the complete matching generation.
    pub(crate) expected_road_clip_fingerprints: Vec<u64>,
    /// Stable building-site contributor ids expected in the complete matching generation.
    pub(crate) expected_site_clip_fingerprints: Vec<u64>,
    /// True when a road or building site forbids raw terrain for this patch.
    pub(crate) requires_engineered_refinement: bool,
    /// True when this patch intersects authoritative grounded-road ownership.
    pub(crate) requires_road_clipping: bool,
    /// Total number of road and building-site clip sources found by the query.
    pub(crate) clip_source_count: usize,
    /// Number of authoritative grounded-road sources found by the query.
    pub(crate) road_clip_source_count: usize,
    /// Number of authoritative grounded-road loops emitted by the query.
    pub(crate) road_clip_loop_count: usize,
    /// Number of building-site loops emitted by the query.
    pub(crate) site_clip_loop_count: usize,
    /// Terrain-clip setup error, if the road-boundary query failed before CDT input was built.
    pub(crate) clip_error_label: Option<&'static str>,
    /// Query margin used to discover road contributors for this generation.
    pub(crate) clip_query_margin_m: f32,
    /// Derive complete-generation clip counts from the final cached-window manifests.
    pub(crate) derive_clip_counts_from_windows: bool,
}

/// Cache key for one local CDT window inside a refined render patch.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct RefinedTerrainCdtWindowKey {
    /// Window minimum X in quantized millimetres.
    pub(crate) min_x_mm: i64,
    /// Window minimum Z in quantized millimetres.
    pub(crate) min_z_mm: i64,
    /// Window maximum X in quantized millimetres.
    pub(crate) max_x_mm: i64,
    /// Window maximum Z in quantized millimetres.
    pub(crate) max_z_mm: i64,
    /// Stable fingerprint of road loops and terrain samples in this window.
    pub(crate) fingerprint: u64,
}

/// Build input for one local CDT window inside a refined render patch.
pub(crate) struct RefinedTerrainCdtWindowBuildInput {
    /// Window cache key.
    pub(crate) key: RefinedTerrainCdtWindowKey,
    /// CDT input for this local window.
    pub(crate) cdt_input: TerrainCdtInput,
    /// Previous compiled window when the fingerprint did not change.
    pub(crate) previous: Option<Arc<CachedRefinedTerrainCdtWindow>>,
    /// True when this core tile has an exact road or building-site contributor.
    pub(crate) has_engineered_contributor: bool,
    /// Stable tile-local road contributor fingerprints represented by this input.
    pub(crate) road_clip_fingerprints: Vec<u64>,
    /// Stable tile-local building-site contributor fingerprints represented by this input.
    pub(crate) site_clip_fingerprints: Vec<u64>,
}

/// Cached local CDT window built away from the Godot frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CachedRefinedTerrainCdtWindow {
    /// Window cache key.
    pub(crate) key: RefinedTerrainCdtWindowKey,
    /// Number of road loops supplied to the CDT builder.
    pub(crate) input_road_loops: usize,
    /// Number of source terrain samples supplied to the CDT builder.
    pub(crate) input_source_samples: usize,
    /// Local CDT window used inside the base render patch.
    pub(crate) cdt_patch: TerrainCdtPatch,
    /// CDT result for this window.
    pub(crate) mesh_result: Result<TerrainCdtMesh, TerrainCdtError>,
    /// Immutable render buffers derived from this window's successful CDT result.
    pub(crate) mesh_buffers: Option<Arc<CachedRefinedTerrainMeshBuffers>>,
    /// Time spent in CDT construction for this window.
    pub(crate) cdt_ms: f64,
    /// True when this core tile has an exact road or building-site contributor.
    pub(crate) has_engineered_contributor: bool,
    /// Stable tile-local road contributor fingerprints represented by this compiled window.
    pub(crate) road_clip_fingerprints: Vec<u64>,
    /// Stable tile-local building-site contributor fingerprints represented by this compiled window.
    pub(crate) site_clip_fingerprints: Vec<u64>,
}

/// Production mesh arrays prepared off-thread for one fixed window or a complete render patch.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CachedRefinedTerrainMeshBuffers {
    /// Ordinary terrain vertices in render-patch-local coordinates.
    pub(crate) terrain_vertices: Vec<Vector3>,
    /// Ordinary terrain vertex normals.
    pub(crate) terrain_normals: Vec<Vector3>,
    /// Magnitude of each pre-normalized tile-local normal sum; empty on complete patch buffers.
    pub(crate) terrain_normal_sum_lengths: Vec<f32>,
    /// Minimum-X seam samples for one fixed window; empty on complete patch buffers.
    pub(crate) window_min_x_side_zs: Vec<f32>,
    /// Maximum-X seam samples for one fixed window; empty on complete patch buffers.
    pub(crate) window_max_x_side_zs: Vec<f32>,
    /// Minimum-Z seam samples for one fixed window; empty on complete patch buffers.
    pub(crate) window_min_z_side_xs: Vec<f32>,
    /// Maximum-Z seam samples for one fixed window; empty on complete patch buffers.
    pub(crate) window_max_z_side_xs: Vec<f32>,
    /// Ordinary terrain UV coordinates.
    pub(crate) terrain_uvs: Vec<Vector2>,
    /// Ordinary terrain triangle indices.
    pub(crate) terrain_indices: Vec<i32>,
    /// Retaining-wall vertices in render-patch-local coordinates.
    pub(crate) retaining_vertices: Vec<Vector3>,
    /// Retaining-wall vertex normals.
    pub(crate) retaining_normals: Vec<Vector3>,
    /// Retaining-wall UV coordinates.
    pub(crate) retaining_uvs: Vec<Vector2>,
    /// Retaining-wall triangle indices.
    pub(crate) retaining_indices: Vec<i32>,
    /// Number of emitted ordinary terrain faces.
    pub(crate) terrain_emitted_faces: usize,
    /// Number of emitted retaining-wall faces.
    pub(crate) retaining_emitted_faces: usize,
    /// Number of pathological ordinary terrain faces suppressed from rendering.
    pub(crate) omitted_pathological_terrain_faces: usize,
    /// Largest ordinary terrain face height range.
    pub(crate) terrain_max_face_y_delta_m: f32,
    /// Largest ordinary terrain face slope ratio.
    pub(crate) terrain_max_face_slope_ratio: f32,
    /// Longest ordinary terrain triangle edge.
    pub(crate) terrain_longest_triangle_edge_m: f32,
    /// Largest retaining-wall face height range.
    pub(crate) retaining_max_face_y_delta_m: f32,
    /// Largest retaining-wall face slope ratio.
    pub(crate) retaining_max_face_slope_ratio: f32,
    /// Longest retaining-wall triangle edge.
    pub(crate) retaining_longest_triangle_edge_m: f32,
}

/// Cached production refined terrain patch built away from the Godot frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CachedRefinedTerrainPatch {
    /// Cache key for this patch.
    pub(crate) key: RefinedTerrainPatchCacheKey,
    /// CDT contract revision used to build this cached patch.
    pub(crate) contract_revision: i64,
    /// Patch-local terrain source generation used to build this cached patch.
    pub(crate) surface_generation: u64,
    /// Base visual terrain patch snapshot.
    pub(crate) patch: TerrainPatchSnapshot,
    /// Number of road loops supplied to the CDT builder.
    pub(crate) input_road_loops: usize,
    /// Number of source terrain samples supplied to the CDT builder.
    pub(crate) input_source_samples: usize,
    /// Local CDT windows composed into this render patch.
    pub(crate) windows: Vec<Arc<CachedRefinedTerrainCdtWindow>>,
    /// Complete render buffers prepared after all matching-generation windows succeed.
    pub(crate) mesh_buffers: Option<Arc<CachedRefinedTerrainMeshBuffers>>,
    /// True when a road or building site forbids raw terrain for this patch.
    pub(crate) requires_engineered_refinement: bool,
    /// True when this patch intersects authoritative grounded-road ownership.
    pub(crate) requires_road_clipping: bool,
    /// Total number of road and building-site clip sources found by the query.
    pub(crate) clip_source_count: usize,
    /// Number of authoritative grounded-road sources found by the clip query.
    pub(crate) road_clip_source_count: usize,
    /// Number of authoritative grounded-road loops emitted by the clip query.
    pub(crate) road_clip_loop_count: usize,
    /// Number of building-site loops emitted by the clip query.
    pub(crate) site_clip_loop_count: usize,
    /// Padded-query loops whose exact grading influence did not intersect this patch.
    pub(crate) omitted_margin_clip_loop_count: usize,
    /// Terrain-clip setup error, if the road-boundary query failed before CDT input was built.
    pub(crate) clip_error_label: Option<&'static str>,
    /// Query margin used to discover road contributors for this generation.
    pub(crate) clip_query_margin_m: f32,
    /// Time spent in CDT construction for this patch's rebuilt windows.
    pub(crate) cdt_ms: f64,
    /// Number of windows reused from the previous cache entry.
    pub(crate) reused_windows: usize,
}
