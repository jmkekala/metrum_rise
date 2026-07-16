//! Refined road-clipped terrain build and cache payload contracts.

use crate::simulation::terrain::TerrainPatchSnapshot;
use crate::simulation::terrain::cdt::{
    TerrainCdtError, TerrainCdtInput, TerrainCdtMesh, TerrainCdtPatch,
};

/// Fine render step used for terrain patches whose topology is clipped by visible road surfaces.
pub(crate) const ROAD_LOCKED_TERRAIN_RENDER_STEP_M: f32 = 2.0;

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
    /// Road-surface generation captured when this input was assembled.
    pub(crate) surface_generation: u64,
    /// Base visual terrain patch snapshot.
    pub(crate) patch: TerrainPatchSnapshot,
    /// Local CDT windows assembled from source terrain samples and road footprint loops.
    pub(crate) windows: Vec<RefinedTerrainCdtWindowBuildInput>,
    /// Number of source road-boundary records found by the clip query.
    pub(crate) road_clip_source_count: usize,
    /// Terrain-clip setup error, if the road-boundary query failed before CDT input was built.
    pub(crate) clip_error_label: Option<&'static str>,
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
    pub(crate) previous: Option<CachedRefinedTerrainCdtWindow>,
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
    /// Time spent in CDT construction for this window.
    pub(crate) cdt_ms: f64,
    /// True when this window was reused from the previous patch cache.
    pub(crate) reused: bool,
}

/// Cached production refined terrain patch built away from the Godot frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CachedRefinedTerrainPatch {
    /// Cache key for this patch.
    pub(crate) key: RefinedTerrainPatchCacheKey,
    /// CDT contract revision used to build this cached patch.
    pub(crate) contract_revision: i64,
    /// Road-surface generation used to build this cached patch.
    pub(crate) surface_generation: u64,
    /// Base visual terrain patch snapshot.
    pub(crate) patch: TerrainPatchSnapshot,
    /// Number of road loops supplied to the CDT builder.
    pub(crate) input_road_loops: usize,
    /// Number of source terrain samples supplied to the CDT builder.
    pub(crate) input_source_samples: usize,
    /// Local CDT windows composed into this render patch.
    pub(crate) windows: Vec<CachedRefinedTerrainCdtWindow>,
    /// Number of source road-boundary records found by the clip query.
    pub(crate) road_clip_source_count: usize,
    /// Terrain-clip setup error, if the road-boundary query failed before CDT input was built.
    pub(crate) clip_error_label: Option<&'static str>,
    /// Time spent in CDT construction for this patch's rebuilt windows.
    pub(crate) cdt_ms: f64,
    /// Number of windows reused from the previous cache entry.
    pub(crate) reused_windows: usize,
}
