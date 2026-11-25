// SPDX-License-Identifier: GPL-3.0-or-later

//! Authoritative heightmap terrain used for grading, raycasting, and rendering.

pub mod graded;
mod patch_masks;
pub(crate) mod rect_clip;

use godot::prelude::Vector3;
use std::collections::{HashMap, HashSet};

use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::sparse_chunk_grid::SparseChunkGrid;
use crate::simulation::network::graph::RegionGraph;

const DEFAULT_TERRAIN_CHUNK_CELLS: usize = 64;
/// Texels of neighbouring terrain each render patch texture carries beyond its own samples.
pub(crate) const TERRAIN_RENDER_PATCH_BORDER_TEXELS: usize = 4;
/// Metres per stored height unit in files written before heights were metres: saves before
/// format 131 and world definitions before format 6. Their loaders multiply by it once, at load;
/// nothing else uses it.
pub(crate) const LEGACY_HEIGHT_UNIT_M: f32 = 20.0;
const TERRAIN_LOCAL_MIN_SAMPLE_MARGIN_M: f32 = 8.0;
const TERRAIN_LOCAL_SAMPLE_MARGIN_RENDER_STEPS: f32 = 4.0;
const TERRAIN_LOCAL_SAMPLE_MARGIN_TERRAIN_CELLS: f32 = 2.0;
// Local height revisions use the shared world-zero-aligned 512 m chunk grid. The margin covers
// draped overlays owned by a chunk centre but reaching past it: an 80 m x 120 m parcel extends
// at most 72 m, a cell under 8 m. One terrain cell is added for bilinear sampling.
const HEIGHT_REVISION_REGION_M: f32 = RegionGraph::CHUNK_SIZE;
const HEIGHT_REVISION_MARGIN_M: f32 = 96.0;

/// Fine render step used by planned and committed road-clipped terrain patches.
pub(crate) const ROAD_LOCKED_TERRAIN_RENDER_STEP_M: f32 = 2.0;
/// Span of the fixed world-aligned terrain CDT tile, the CDT, cache and ledger unit inside a
/// render patch, in millimetres.
/// Returns the deterministic margin used by bounded terrain grading queries.
pub(crate) fn terrain_local_sample_margin_m(
    terrain: &TerrainSystem,
    render_step_m: f32,
) -> f32 {
    TERRAIN_LOCAL_MIN_SAMPLE_MARGIN_M
        .max(render_step_m.max(f32::EPSILON) * TERRAIN_LOCAL_SAMPLE_MARGIN_RENDER_STEPS)
        .max(terrain.cell_size_m() * TERRAIN_LOCAL_SAMPLE_MARGIN_TERRAIN_CELLS)
}

/// Expands a road grading margin by the patch-selection safety pad used around grading rays.
///
/// A patch can be selected by the pad even when the road seam lies just beyond the unpadded
/// grading margin. Clip-source queries must include the same pad or that selected patch can be
/// misclassified as road-owned with no road sources.
pub(crate) fn terrain_road_query_margin_m(
    terrain: &TerrainSystem,
    render_step_m: f32,
    grading_margin_m: f32,
) -> f32 {
    grading_margin_m.max(0.0) + render_step_m.max(terrain.cell_size_m())
}

/// One deterministic render-patch snapshot of visual terrain.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerrainPatchSnapshot {
    /// Patch X index on the render-patch grid.
    pub patch_x: usize,
    /// Patch Z index on the render-patch grid.
    pub patch_z: usize,
    /// Number of owned terrain samples across X without the border ring.
    pub sample_width: usize,
    /// Number of owned terrain samples across Z without the border ring.
    pub sample_height: usize,
    /// Number of texture samples across X including the border ring.
    pub texture_width: usize,
    /// Number of texture samples across Z including the border ring.
    pub texture_height: usize,
    /// Inner owned sample start inside the texture, in texels.
    pub inner_offset_x: usize,
    /// Inner owned sample start inside the texture, in texels.
    pub inner_offset_z: usize,
    /// Patch minimum world-space X in metres.
    pub world_origin_x: f32,
    /// Patch minimum world-space Z in metres.
    pub world_origin_z: f32,
    /// Patch width in world metres.
    pub world_size_x: f32,
    /// Patch height in world metres.
    pub world_size_z: f32,
    /// Row-major source-terrain samples including the border ring.
    pub height_data: Vec<f32>,
}

/// Centre of a render frame spanning `size_m` from `min_m`: the local origin of a terrain patch
/// and of the road render chunk over it (`ROAD-68-D`, open question 23).
pub(crate) fn render_frame_centre(min_m: f32, size_m: f32) -> f32 {
    min_m + size_m * 0.5
}

/// A world coordinate local to the render frame centred at `centre_m`: subtracted in f64 and
/// rounded once, so every mesh in one frame gives a world point the same bits.
pub(crate) fn render_frame_local(world_m: f64, centre_m: f32) -> f32 {
    (world_m - f64::from(centre_m)) as f32
}

/// Authoritative heightmap for the terrain surface.
#[derive(Clone)]
pub struct TerrainSystem {
    /// Map width in height samples.
    pub width: usize,
    /// Map height (depth) in height samples.
    pub height: usize,
    /// Terrain sample spacing in metres.
    cell_size: f32,
    /// Authored terrain-chunk span in metres used to derive render-patch ownership.
    chunk_span_m: f32,
    /// Number of terrain intervals owned by one render patch along one axis.
    render_patch_interval_cells: usize,
    /// Source heightmap sculpted by the player and never written by road placement.
    source_data: SparseChunkGrid<f32>,
    /// Monotonic revision of authoritative terrain samples used by derived-query caches.
    source_generation: u64,
    /// Render patches whose visible terrain textures must be refreshed.
    dirty_render_patches: HashSet<(usize, usize)>,
    /// Latest local height revision per 512 m region, including writes within the draping margin.
    height_region_revisions: HashMap<(i32, i32), u64>,
    /// Monotonic counter shared by local writes and whole-grid replacements.
    height_revision: u64,
    /// Revision of the latest whole-grid replacement; every region is at least this new.
    height_epoch: u64,
}

/// Bounded capture of source samples used to validate the site-feasibility cache.
#[derive(Debug)]
pub(crate) struct TerrainSourceCapture {
    data: SparseChunkGrid<f32>,
    // Explicit coverage distinguishes a reset-to-default chunk from an untouched sparse chunk.
    chunks: HashSet<(usize, usize)>,
}

impl TerrainSourceCapture {
    /// Starts an empty capture against the supplied terrain layout.
    pub(crate) fn new(terrain: &TerrainSystem) -> Self {
        Self {
            data: terrain.source_data.empty_like(),
            chunks: HashSet::new(),
        }
    }

    /// Captures the source storage chunks intersecting the world bounds.
    pub(crate) fn capture_region(
        &mut self,
        terrain: &TerrainSystem,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) {
        let Some((x0, x1, z0, z1)) = terrain.grid_rect_for_world_bounds(min_x, min_z, max_x, max_z)
        else {
            return;
        };
        let size = self.data.chunk_size();
        for z in z0 / size..=z1 / size {
            for x in x0 / size..=x1 / size {
                if self.chunks.insert((x, z)) {
                    self.data.copy_rect_from(
                        &terrain.source_data,
                        x * size,
                        (x + 1) * size - 1,
                        z * size,
                        (z + 1) * size - 1,
                    );
                }
            }
        }
    }

    /// Whether every captured source sample still holds its value, exactly.
    pub(crate) fn matches_current(&self, terrain: &TerrainSystem) -> bool {
        self.chunks
            .iter()
            .all(|&(x, z)| self.data.chunk_values_match(&terrain.source_data, x, z))
    }
}

impl TerrainSystem {
    /// Creates a new, flat terrain system of the given dimensions.
    pub fn new(width: usize, height: usize) -> Self {
        Self::with_chunking(width, height, 1.0, DEFAULT_TERRAIN_CHUNK_CELLS, 0.0)
    }

    /// Creates a new terrain system with explicit sparse chunk size and base elevation.
    pub fn with_chunking(
        width: usize,
        height: usize,
        cell_size: f32,
        chunk_size: usize,
        base_elevation: f32,
    ) -> Self {
        let safe_cell_size = cell_size.max(f32::EPSILON);
        let safe_chunk_size = chunk_size.max(1);
        Self {
            width,
            height,
            cell_size: safe_cell_size,
            chunk_span_m: safe_cell_size * safe_chunk_size.saturating_sub(1) as f32,
            render_patch_interval_cells: safe_chunk_size.saturating_sub(1).max(1),
            source_data: SparseChunkGrid::new(width, height, safe_chunk_size, base_elevation),
            source_generation: 0,
            dirty_render_patches: HashSet::new(),
            height_region_revisions: HashMap::new(),
            height_revision: 0,
            height_epoch: 0,
        }
    }

    /// Creates a terrain system from the current world configuration.
    ///
    /// The configuration must keep the world rules (`WorldConfig::validate`): the graded
    /// tiles and render patches rely on the aligned grid, so a test or loader that builds a
    /// world off the rules fails here in debug builds.
    pub fn from_world_config(config: &WorldConfig) -> Self {
        debug_assert!(
            config.validate().is_ok(),
            "world off the rules: {:?} ({config:?})",
            config.validate()
        );
        Self::with_chunking(
            config.terrain_grid_width(),
            config.terrain_grid_height(),
            config.terrain_cell_m,
            config.terrain_storage_chunk_cells(),
            config.terrain_base_elevation_m,
        )
        .with_render_chunk_span(config.terrain_render_chunk_span_m())
    }

    fn with_render_chunk_span(mut self, chunk_span_m: f32) -> Self {
        self.chunk_span_m = chunk_span_m.max(self.cell_size);
        self.render_patch_interval_cells =
            render_patch_interval_cells(self.cell_size, self.chunk_span_m);
        self
    }

    /// Sets the height at a specific grid coordinate and marks the render patches over it.
    pub fn set_height(&mut self, x: usize, y: usize, value: f32) {
        if x < self.width && y < self.height {
            self.source_generation = self.source_generation.wrapping_add(1);
            self.source_data.set(x, y, value);
            self.mark_render_patches_for_grid_rect(x, x, y, y);
        }
    }

    /// Gets the source height in metres at a grid coordinate.
    pub fn get_height(&self, x: usize, y: usize) -> f32 {
        self.source_data.get(x, y)
    }

    /// Bilinearly interpolates the source height at any fractional world coordinate.
    pub fn get_height_interpolated(&self, x: f32, z: f32) -> f32 {
        self.interpolate_grid_height(&self.source_data, x, z)
    }

    /// Samples the authoritative source terrain at one world-space position.
    pub fn sample_height_world(&self, world_x: f32, world_z: f32) -> f32 {
        let (grid_x, grid_z) = self.world_to_grid_coords(world_x, world_z);
        self.get_height_interpolated(grid_x, grid_z)
    }

    /// Returns the terrain sample spacing in metres.
    pub(crate) fn cell_size_m(&self) -> f32 {
        self.cell_size
    }

    /// Returns the current authoritative terrain revision for exact derived-cache validation.
    pub(crate) fn source_generation(&self) -> u64 {
        self.source_generation
    }

    /// Returns the latest source height change that can move a draped overlay owned by
    /// one world-zero-aligned 512 m chunk. O(1); unrelated edits elsewhere leave it unchanged.
    pub(crate) fn height_revision_for_chunk(&self, chunk: (i32, i32)) -> u64 {
        self.height_region_revisions
            .get(&chunk)
            .copied()
            .unwrap_or(0)
            .max(self.height_epoch)
    }

    // O(regions touched); called after every local source sample write.
    fn record_height_grid_rect(&mut self, min_x: usize, max_x: usize, min_z: usize, max_z: usize) {
        self.height_revision += 1;
        let (x0, z0) = self.grid_to_world_coords(min_x, min_z);
        let (x1, z1) = self.grid_to_world_coords(max_x, max_z);
        let margin = HEIGHT_REVISION_MARGIN_M + self.cell_size;
        let region = |value: f32| (value / HEIGHT_REVISION_REGION_M).floor() as i32;
        for z in region(z0 - margin)..=region(z1 + margin) {
            for x in region(x0 - margin)..=region(x1 + margin) {
                self.height_region_revisions
                    .insert((x, z), self.height_revision);
            }
        }
    }

    // Whole-grid replacements supersede every local entry.
    fn record_height_reset(&mut self) {
        self.height_revision += 1;
        self.height_epoch = self.height_revision;
        self.height_region_revisions.clear();
    }

    fn interpolate_grid_height(&self, grid: &SparseChunkGrid<f32>, x: f32, z: f32) -> f32 {
        self.interpolate_height_cells(x, z, |x0, x1, z0, z1| {
            grid.get_bilinear_cells(x0, x1, z0, z1)
        })
    }

    fn interpolate_height_cells(
        &self,
        x: f32,
        z: f32,
        cells: impl FnOnce(usize, usize, usize, usize) -> [f32; 4],
    ) -> f32 {
        let x_clamped = x.clamp(0.0, (self.width - 1) as f32);
        let z_clamped = z.clamp(0.0, (self.height - 1) as f32);

        let x0 = x_clamped.floor() as usize;
        let z0 = z_clamped.floor() as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let z1 = (z0 + 1).min(self.height - 1);

        let fx = x_clamped.fract();
        let fz = z_clamped.fract();

        let [h00, h10, h01, h11] = cells(x0, x1, z0, z1);

        let h0 = h00 * (1.0 - fx) + h10 * fx;
        let h1 = h01 * (1.0 - fx) + h11 * fx;

        h0 * (1.0 - fz) + h1 * fz
    }

    /// Casts a ray against the terrain surface and returns the intersection point.
    pub fn raycast_terrain(&self, ray_origin: Vector3, ray_dir: Vector3) -> Option<Vector3> {
        self.raycast_height_field(ray_origin, ray_dir)
    }

    fn raycast_height_field(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<Vector3> {
        let (half_w, half_h) = self.half_world_extents();
        let (entry_t, exit_t) = raycast_xz_interval(ray_origin, ray_dir, half_w, half_h)?;
        let mut prev_t = entry_t.max(0.0);
        let step = (self.cell_size * 0.5).clamp(0.5, 5.0);
        let max_t = if exit_t.is_finite() {
            exit_t.max(prev_t)
        } else {
            prev_t + self.raycast_search_limit(ray_origin, half_w, half_h)
        };
        let prev_point = ray_origin + ray_dir * prev_t;
        let mut prev_diff = prev_point.y
            - self.sample_height_world(prev_point.x, prev_point.z);

        while prev_t < max_t {
            let t = (prev_t + step).min(max_t);
            let p = ray_origin + ray_dir * t;
            let h = self.sample_height_world(p.x, p.z);
            let diff = p.y - h;

            if diff == 0.0 || diff.signum() != prev_diff.signum() {
                // Binary search refinement for precision.
                let mut t_low = prev_t;
                let mut t_high = t;
                for _ in 0..8 {
                    let t_mid = (t_low + t_high) * 0.5;
                    let pm = ray_origin + ray_dir * t_mid;
                    let hm = self.sample_height_world(pm.x, pm.z);
                    if (pm.y - hm).signum() == prev_diff.signum() {
                        t_low = t_mid;
                    } else {
                        t_high = t_mid;
                    }
                }

                return Some(ray_origin + ray_dir * ((t_low + t_high) * 0.5));
            }

            prev_t = t;
            prev_diff = diff;
        }

        None
    }

    /// Modifies terrain height in a circular area with smooth falloff.
    pub fn sculpt(&mut self, center_x: f32, center_y: f32, radius: f32, strength: f32) {
        let Some((min_x, max_x, min_y, max_y)) = self.brush_grid_bounds(center_x, center_y, radius)
        else {
            return;
        };
        self.source_generation = self.source_generation.wrapping_add(1);
        self.mark_render_patches_for_grid_rect(min_x, max_x, min_y, max_y);
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let dx = x as f32 - center_x;
                let dy = y as f32 - center_y;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq <= radius * radius {
                    let dist = dist_sq.sqrt();
                    let normalized_dist = dist / radius;
                    let falloff = (1.0 + (normalized_dist * std::f32::consts::PI).cos()) * 0.5;

                    let current_h = self.source_data.get(x, y);
                    let next_h = current_h + strength * falloff;

                    self.source_data.set(x, y, next_h);
                }
            }
        }
    }

    /// Moves terrain toward one target height in a circular area with smooth falloff.
    pub fn level_to_height(
        &mut self,
        center_x: f32,
        center_y: f32,
        radius: f32,
        target_height: f32,
        strength: f32,
    ) {
        let Some((min_x, max_x, min_y, max_y)) = self.brush_grid_bounds(center_x, center_y, radius)
        else {
            return;
        };
        self.source_generation = self.source_generation.wrapping_add(1);
        self.mark_render_patches_for_grid_rect(min_x, max_x, min_y, max_y);
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let dx = x as f32 - center_x;
                let dy = y as f32 - center_y;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq <= radius * radius {
                    let dist = dist_sq.sqrt();
                    let normalized_dist = dist / radius;
                    let falloff = (1.0 + (normalized_dist * std::f32::consts::PI).cos()) * 0.5;

                    let current_h = self.source_data.get(x, y);
                    let delta = target_height - current_h;
                    let max_step = strength * falloff;
                    let next_h = if delta.abs() <= max_step {
                        target_height
                    } else {
                        current_h + delta.signum() * max_step
                    };

                    self.source_data.set(x, y, next_h);
                }
            }
        }
    }

    /// Smooths terrain toward the local neighborhood average in a circular area.
    ///
    /// This uses a local patch snapshot so the brush is not biased by scan order.
    /// Complexity is O(k) for k cells in the touched brush bounding box.
    pub fn smooth(&mut self, center_x: f32, center_y: f32, radius: f32, strength: f32) {
        let Some((min_x, max_x, min_y, max_y)) = self.brush_grid_bounds(center_x, center_y, radius)
        else {
            return;
        };
        self.source_generation = self.source_generation.wrapping_add(1);
        self.mark_render_patches_for_grid_rect(min_x, max_x, min_y, max_y);

        // The snapshot reaches one sample past the brush on each side, inside the grid.
        let patch_min_x = min_x.saturating_sub(1);
        let patch_max_x = (max_x + 1).min(self.width - 1);
        let patch_min_y = min_y.saturating_sub(1);
        let patch_max_y = (max_y + 1).min(self.height - 1);

        let patch_w = patch_max_x - patch_min_x + 1;
        let patch_h = patch_max_y - patch_min_y + 1;
        let mut patch = vec![0.0_f32; patch_w * patch_h];
        for local_y in 0..patch_h {
            for local_x in 0..patch_w {
                patch[local_y * patch_w + local_x] = self
                    .source_data
                    .get(patch_min_x + local_x, patch_min_y + local_y);
            }
        }

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let dx = x as f32 - center_x;
                let dy = y as f32 - center_y;
                let dist_sq = dx * dx + dy * dy;
                if dist_sq > radius * radius {
                    continue;
                }

                let dist = dist_sq.sqrt();
                let normalized_dist = dist / radius;
                let falloff = (1.0 + (normalized_dist * std::f32::consts::PI).cos()) * 0.5;
                let max_step = strength * falloff;

                let local_x = x - patch_min_x;
                let local_y = y - patch_min_y;
                let mut sum = 0.0_f32;
                let mut samples = 0.0_f32;

                let sample_min_x = local_x.saturating_sub(1);
                let sample_max_x = (local_x + 1).min(patch_w - 1);
                let sample_min_y = local_y.saturating_sub(1);
                let sample_max_y = (local_y + 1).min(patch_h - 1);
                for sample_y in sample_min_y..=sample_max_y {
                    for sample_x in sample_min_x..=sample_max_x {
                        sum += patch[sample_y * patch_w + sample_x];
                        samples += 1.0;
                    }
                }

                let target_height = sum / samples.max(1.0_f32);
                let current_h = patch[local_y * patch_w + local_x];
                let delta = target_height - current_h;
                let next_h = if delta.abs() <= max_step {
                    target_height
                } else {
                    current_h + delta.signum() * max_step
                };

                self.source_data.set(x, y, next_h);
            }
        }
    }

    /// Moves terrain toward a clamped linear grade defined by two clicked anchor points.
    ///
    /// The target height for each touched sample is the linear interpolation between the two
    /// anchor heights, projected onto the anchor segment and clamped to the segment endpoints.
    #[allow(clippy::too_many_arguments)]
    pub fn slope_to_segment(
        &mut self,
        center_x: f32,
        center_y: f32,
        radius: f32,
        start_x: f32,
        start_y: f32,
        start_height: f32,
        end_x: f32,
        end_y: f32,
        end_height: f32,
        strength: f32,
    ) {
        let Some((min_x, max_x, min_y, max_y)) = self.brush_grid_bounds(center_x, center_y, radius)
        else {
            return;
        };
        self.mark_render_patches_for_grid_rect(min_x, max_x, min_y, max_y);
        let seg_x = end_x - start_x;
        let seg_y = end_y - start_y;
        let seg_len_sq = seg_x * seg_x + seg_y * seg_y;
        if seg_len_sq <= f32::EPSILON {
            self.level_to_height(center_x, center_y, radius, start_height, strength);
            return;
        }
        self.source_generation = self.source_generation.wrapping_add(1);

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let dx = x as f32 - center_x;
                let dy = y as f32 - center_y;
                let dist_sq = dx * dx + dy * dy;
                if dist_sq > radius * radius {
                    continue;
                }

                let dist = dist_sq.sqrt();
                let normalized_dist = dist / radius;
                let falloff = (1.0 + (normalized_dist * std::f32::consts::PI).cos()) * 0.5;
                let max_step = strength * falloff;

                let sample_x = x as f32;
                let sample_y = y as f32;
                let along_t = (((sample_x - start_x) * seg_x + (sample_y - start_y) * seg_y)
                    / seg_len_sq)
                    .clamp(0.0, 1.0);
                let target_height = start_height + (end_height - start_height) * along_t;

                let current_h = self.source_data.get(x, y);
                let delta = target_height - current_h;
                let next_h = if delta.abs() <= max_step {
                    target_height
                } else {
                    current_h + delta.signum() * max_step
                };

                self.source_data.set(x, y, next_h);
            }
        }
    }

    /// Returns a dense row-major snapshot of the authoritative source terrain buffer (metres).
    pub(crate) fn clone_source_dense(&self) -> Vec<f32> {
        self.source_data.clone_dense()
    }

    /// Replaces the authoritative source terrain buffer from a dense row-major snapshot.
    pub(crate) fn replace_source_from_dense(&mut self, dense: &[f32]) -> Result<(), String> {
        self.source_data.replace_from_dense(dense)?;
        self.source_generation = self.source_generation.wrapping_add(1);
        self.record_height_reset();
        Ok(())
    }

    /// The source terrain grid itself, for a reader that keeps a [`Self::source_snapshot`] and
    /// compares it with the live grid later. O(1).
    pub(crate) fn source_grid(&self) -> &SparseChunkGrid<f32> {
        &self.source_data
    }

    /// The source terrain as a shared clone, for an undo record: `O(chunks)` pointer copies.
    /// The live grid's later writes copy only the chunks they touch (`Arc::make_mut` in
    /// `SparseChunkGrid::set`), so the record costs `O(edited chunks)` memory beyond the pointers.
    pub(crate) fn source_snapshot(&self) -> SparseChunkGrid<f32> {
        self.source_data.clone()
    }

    /// Puts back an undo record's [`Self::source_snapshot`], with the same generation bump and
    /// whole-grid height reset as a dense replacement. O(1) beyond dropping the replaced chunks.
    pub(crate) fn replace_source_from_snapshot(
        &mut self,
        snapshot: SparseChunkGrid<f32>,
    ) -> Result<(), String> {
        if !self.source_data.layout_matches(&snapshot) {
            return Err("terrain snapshot layout differs from the live terrain".to_owned());
        }
        self.source_data = snapshot;
        self.source_generation = self.source_generation.wrapping_add(1);
        self.record_height_reset();
        Ok(())
    }

    /// Returns the full terrain extent in metres.
    pub(crate) fn world_size(&self) -> (f32, f32) {
        (
            (self.width.saturating_sub(1)) as f32 * self.cell_size,
            (self.height.saturating_sub(1)) as f32 * self.cell_size,
        )
    }

    /// Returns half-width and half-height in metres.
    pub(crate) fn half_world_extents(&self) -> (f32, f32) {
        let (world_w, world_h) = self.world_size();
        (world_w * 0.5, world_h * 0.5)
    }

    /// Returns the authored terrain-chunk span used for render-patch layout.
    pub(crate) fn chunk_span_m(&self) -> f32 {
        self.chunk_span_m
    }

    /// Returns the number of render patches across the X axis.
    pub(crate) fn render_patch_cols(&self) -> usize {
        if self.width <= 1 {
            1
        } else {
            (self.width - 1).div_ceil(self.render_patch_interval_cells)
        }
    }

    /// Returns the number of terrain intervals owned by one render patch.
    pub(crate) fn render_patch_interval_cells(&self) -> usize {
        self.render_patch_interval_cells
    }

    /// Returns the world-space margin sampled around each render patch texture.
    pub(crate) fn render_patch_border_margin_m(&self) -> f32 {
        TERRAIN_RENDER_PATCH_BORDER_TEXELS as f32 * self.cell_size
    }

    /// Returns the number of render patches across the Z axis.
    pub(crate) fn render_patch_rows(&self) -> usize {
        if self.height <= 1 {
            1
        } else {
            (self.height - 1).div_ceil(self.render_patch_interval_cells)
        }
    }

    /// Returns the current set of dirty terrain render patches.
    pub(crate) fn dirty_render_patches(&self) -> &HashSet<(usize, usize)> {
        &self.dirty_render_patches
    }

    /// Marks one render patch dirty if it exists on the terrain patch grid.
    pub(crate) fn mark_render_patch_dirty(&mut self, patch_x: usize, patch_z: usize) {
        if patch_x < self.render_patch_cols() && patch_z < self.render_patch_rows() {
            self.dirty_render_patches.insert((patch_x, patch_z));
        }
    }

    /// Acknowledges one render patch without disturbing dirtiness added to other patches.
    pub(crate) fn clear_render_patch_dirty(&mut self, patch_x: usize, patch_z: usize) {
        self.dirty_render_patches.remove(&(patch_x, patch_z));
    }

    /// Returns the render-patch keys whose sample bounds overlap the given world-space rectangle.
    pub(crate) fn render_patch_keys_for_world_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<(usize, usize)> {
        let mut keys = Vec::new();
        self.append_render_patch_keys_for_world_bounds(min_x, min_z, max_x, max_z, &mut keys);
        keys
    }

    /// Appends the render-patch keys whose sample bounds overlap the given world-space
    /// rectangle, in [`Self::render_patch_keys_for_world_bounds`]'s order. O(patches).
    pub(crate) fn append_render_patch_keys_for_world_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        keys: &mut Vec<(usize, usize)>,
    ) {
        let Some((min_grid_x, max_grid_x, min_grid_z, max_grid_z)) =
            self.grid_rect_for_world_bounds(min_x, min_z, max_x, max_z)
        else {
            return;
        };

        let (min_patch_x, max_patch_x) =
            self.patch_range_for_sample_range(min_grid_x, max_grid_x, self.render_patch_cols());
        let (min_patch_z, max_patch_z) =
            self.patch_range_for_sample_range(min_grid_z, max_grid_z, self.render_patch_rows());
        keys.reserve((max_patch_x - min_patch_x + 1) * (max_patch_z - min_patch_z + 1));
        for patch_z in min_patch_z..=max_patch_z {
            for patch_x in min_patch_x..=max_patch_x {
                keys.push((patch_x, patch_z));
            }
        }
    }

    /// Returns the owned world-space rectangle for one terrain render patch.
    pub(crate) fn render_patch_world_bounds(
        &self,
        patch_x: usize,
        patch_z: usize,
    ) -> Option<(f32, f32, f32, f32)> {
        let (start_x, end_x, start_z, end_z) = self.render_patch_sample_bounds(patch_x, patch_z)?;
        let (min_x, min_z) = self.grid_to_world_coords(start_x, start_z);
        let (max_x, max_z) = self.grid_to_world_coords(end_x, end_z);
        Some((min_x, min_z, max_x, max_z))
    }

    /// Marks all render patches overlapping a world-space rectangle as dirty.
    pub(crate) fn mark_render_patches_for_world_bounds(
        &mut self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) {
        let Some((min_grid_x, max_grid_x, min_grid_z, max_grid_z)) =
            self.grid_rect_for_world_bounds(min_x, min_z, max_x, max_z)
        else {
            return;
        };
        self.mark_render_patches_for_grid_rect(min_grid_x, max_grid_x, min_grid_z, max_grid_z);
    }

    /// Marks every terrain render patch dirty after a whole-world replacement.
    pub(crate) fn mark_all_render_patches_dirty(&mut self) {
        self.record_height_reset();
        for patch_z in 0..self.render_patch_rows() {
            for patch_x in 0..self.render_patch_cols() {
                self.dirty_render_patches.insert((patch_x, patch_z));
            }
        }
    }

    /// Returns one source-terrain render patch with a neighboring-sample border ring.
    pub(crate) fn source_patch_snapshot(
        &self,
        patch_x: usize,
        patch_z: usize,
    ) -> Option<TerrainPatchSnapshot> {
        let (start_x, end_x, start_z, end_z) = self.render_patch_sample_bounds(patch_x, patch_z)?;
        let sample_width = end_x - start_x + 1;
        let sample_height = end_z - start_z + 1;
        let texture_width = sample_width + TERRAIN_RENDER_PATCH_BORDER_TEXELS * 2;
        let texture_height = sample_height + TERRAIN_RENDER_PATCH_BORDER_TEXELS * 2;
        let mut height_data = vec![0.0_f32; texture_width * texture_height];

        let source_min_x = start_x.saturating_sub(TERRAIN_RENDER_PATCH_BORDER_TEXELS);
        let source_min_z = start_z.saturating_sub(TERRAIN_RENDER_PATCH_BORDER_TEXELS);
        let source_max_x = end_x
            .saturating_add(TERRAIN_RENDER_PATCH_BORDER_TEXELS)
            .min(self.width - 1);
        let source_max_z = end_z
            .saturating_add(TERRAIN_RENDER_PATCH_BORDER_TEXELS)
            .min(self.height - 1);
        let destination_x = TERRAIN_RENDER_PATCH_BORDER_TEXELS.saturating_sub(start_x);
        let destination_z = TERRAIN_RENDER_PATCH_BORDER_TEXELS.saturating_sub(start_z);
        self.source_data.copy_rect_into(
            source_min_x,
            source_max_x,
            source_min_z,
            source_max_z,
            &mut height_data,
            texture_width,
            destination_x,
            destination_z,
        );

        let copied_width = source_max_x - source_min_x + 1;
        let copied_height = source_max_z - source_min_z + 1;
        let copied_end_x = destination_x + copied_width;
        for local_z in destination_z..destination_z + copied_height {
            let row_start = local_z * texture_width;
            let left_value = height_data[row_start + destination_x];
            height_data[row_start..row_start + destination_x].fill(left_value);
            let right_value = height_data[row_start + copied_end_x - 1];
            height_data[row_start + copied_end_x..row_start + texture_width].fill(right_value);
        }
        let first_copied_row = destination_z * texture_width;
        for local_z in 0..destination_z {
            let destination_start = local_z * texture_width;
            height_data.copy_within(
                first_copied_row..first_copied_row + texture_width,
                destination_start,
            );
        }
        let last_copied_row = (destination_z + copied_height - 1) * texture_width;
        for local_z in destination_z + copied_height..texture_height {
            let destination_start = local_z * texture_width;
            height_data.copy_within(
                last_copied_row..last_copied_row + texture_width,
                destination_start,
            );
        }

        let (world_origin_x, world_origin_z) = self.grid_to_world_coords(start_x, start_z);
        let world_size_x = (sample_width.saturating_sub(1)) as f32 * self.cell_size;
        let world_size_z = (sample_height.saturating_sub(1)) as f32 * self.cell_size;
        Some(TerrainPatchSnapshot {
            patch_x,
            patch_z,
            sample_width,
            sample_height,
            texture_width,
            texture_height,
            inner_offset_x: TERRAIN_RENDER_PATCH_BORDER_TEXELS,
            inner_offset_z: TERRAIN_RENDER_PATCH_BORDER_TEXELS,
            world_origin_x,
            world_origin_z,
            world_size_x,
            world_size_z,
            height_data,
        })
    }

    /// Returns the terrain-border perimeter loop as world-space top positions.
    pub(crate) fn border_loop_positions(&self) -> Vec<Vector3> {
        if self.width < 2 || self.height < 2 {
            return Vec::new();
        }

        let mut perimeter = Vec::with_capacity(self.width * 2 + self.height * 2 - 4);
        for x in 0..self.width {
            perimeter.push(self.border_position(x, 0));
        }
        for z in 1..self.height {
            perimeter.push(self.border_position(self.width - 1, z));
        }
        for x in (0..self.width.saturating_sub(1)).rev() {
            perimeter.push(self.border_position(x, self.height - 1));
        }
        for z in (1..self.height.saturating_sub(1)).rev() {
            perimeter.push(self.border_position(0, z));
        }
        perimeter
    }

    /// Converts one world-space position to fractional terrain-grid coordinates.
    pub(crate) fn world_to_grid_coords(&self, world_x: f32, world_z: f32) -> (f32, f32) {
        let (half_w, half_h) = self.half_world_extents();
        (
            (world_x + half_w) / self.cell_size,
            (world_z + half_h) / self.cell_size,
        )
    }

    /// Converts one terrain sample index back to world-space coordinates.
    pub(crate) fn grid_to_world_coords(&self, grid_x: usize, grid_z: usize) -> (f32, f32) {
        let (half_w, half_h) = self.half_world_extents();
        (
            grid_x as f32 * self.cell_size - half_w,
            grid_z as f32 * self.cell_size - half_h,
        )
    }

    /// Converts one world-space AABB to a clamped inclusive terrain-grid rectangle.
    pub(crate) fn grid_rect_for_world_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Option<(usize, usize, usize, usize)> {
        if self.width == 0 || self.height == 0 {
            return None;
        }

        let (grid_min_x, grid_min_z) =
            self.world_to_grid_coords(min_x.min(max_x), min_z.min(max_z));
        let (grid_max_x, grid_max_z) =
            self.world_to_grid_coords(min_x.max(max_x), min_z.max(max_z));
        let min_grid_x = grid_min_x.floor().clamp(0.0, (self.width - 1) as f32) as usize;
        let max_grid_x = grid_max_x.ceil().clamp(0.0, (self.width - 1) as f32) as usize;
        let min_grid_z = grid_min_z.floor().clamp(0.0, (self.height - 1) as f32) as usize;
        let max_grid_z = grid_max_z.ceil().clamp(0.0, (self.height - 1) as f32) as usize;
        Some((min_grid_x, max_grid_x, min_grid_z, max_grid_z))
    }

    fn raycast_search_limit(&self, ray_origin: Vector3, half_w: f32, half_h: f32) -> f32 {
        let world_diag = (half_w * 2.0).hypot(half_h * 2.0);
        ray_origin.length() + world_diag + 10_000.0
    }

    // Samples in this rectangle were written: refresh render patches and draped overlays.
    // Clamped to the grid first, so the marks are bounded by the world whatever the caller
    // passes (`TERRAIN-28`: a wrapped bound once made the region loop below run without end).
    fn mark_render_patches_for_grid_rect(
        &mut self,
        min_x: usize,
        max_x: usize,
        min_z: usize,
        max_z: usize,
    ) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let (max_x, max_z) = (max_x.min(self.width - 1), max_z.min(self.height - 1));
        if min_x > max_x || min_z > max_z {
            return;
        }
        self.record_height_grid_rect(min_x, max_x, min_z, max_z);
        self.dirty_render_patches_for_grid_rect(min_x, max_x, min_z, max_z);
    }

    // Render refresh only; the caller records the exact changed extent separately.
    fn dirty_render_patches_for_grid_rect(
        &mut self,
        min_x: usize,
        max_x: usize,
        min_z: usize,
        max_z: usize,
    ) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let (min_patch_x, max_patch_x) =
            self.patch_range_for_sample_range(min_x, max_x, self.render_patch_cols());
        let (min_patch_z, max_patch_z) =
            self.patch_range_for_sample_range(min_z, max_z, self.render_patch_rows());
        for patch_z in min_patch_z..=max_patch_z {
            for patch_x in min_patch_x..=max_patch_x {
                self.dirty_render_patches.insert((patch_x, patch_z));
            }
        }
    }

    fn patch_range_for_sample_range(
        &self,
        min_sample: usize,
        max_sample: usize,
        patch_count: usize,
    ) -> (usize, usize) {
        let mut patch_min = min_sample / self.render_patch_interval_cells;
        if min_sample > 0 && min_sample.is_multiple_of(self.render_patch_interval_cells) {
            patch_min = patch_min.saturating_sub(1);
        }
        let mut patch_max = max_sample / self.render_patch_interval_cells;
        if max_sample > 0 && max_sample.is_multiple_of(self.render_patch_interval_cells) {
            patch_max = patch_max.min(patch_count.saturating_sub(1));
        }
        (
            patch_min.min(patch_count.saturating_sub(1)),
            patch_max.min(patch_count.saturating_sub(1)),
        )
    }

    fn render_patch_sample_bounds(
        &self,
        patch_x: usize,
        patch_z: usize,
    ) -> Option<(usize, usize, usize, usize)> {
        if patch_x >= self.render_patch_cols() || patch_z >= self.render_patch_rows() {
            return None;
        }

        let start_x = patch_x * self.render_patch_interval_cells;
        let start_z = patch_z * self.render_patch_interval_cells;
        let end_x = (start_x + self.render_patch_interval_cells).min(self.width.saturating_sub(1));
        let end_z = (start_z + self.render_patch_interval_cells).min(self.height.saturating_sub(1));
        Some((start_x, end_x, start_z, end_z))
    }

    // The inclusive sample rectangle `(min_x, max_x, min_y, max_y)` a brush of `radius` cells
    // at grid (`center_x`, `center_y`) may write, clamped to the grid; `None` when it holds no
    // sample (`TERRAIN-28`): a centre or radius that is not finite, a radius not above zero, or
    // a footprint wholly outside the world. Every brush writes only inside it, so a brush costs
    // O(clamped samples) whatever its inputs. Inside the grid the rectangle is the one the
    // brushes always used: the centre truncated, the radius rounded up.
    fn brush_grid_bounds(
        &self,
        center_x: f32,
        center_y: f32,
        radius: f32,
    ) -> Option<(usize, usize, usize, usize)> {
        if self.width == 0
            || self.height == 0
            || !(center_x.is_finite() && center_y.is_finite() && radius.is_finite())
            || radius <= 0.0
        {
            return None;
        }
        // Saturating: a far centre or a huge radius clamps instead of wrapping.
        let r_int = radius.ceil() as i64;
        let axis = |center: f32, samples: usize| {
            let center = center as i64;
            let min = center.saturating_sub(r_int).max(0);
            let max = center.saturating_add(r_int).min(samples as i64 - 1);
            (min <= max).then_some((min as usize, max as usize))
        };
        let (min_x, max_x) = axis(center_x, self.width)?;
        let (min_y, max_y) = axis(center_y, self.height)?;
        Some((min_x, max_x, min_y, max_y))
    }

    /// The world rectangle `[min_x, min_z, max_x, max_z]` a terrain brush of `radius_m` at
    /// world (`world_x`, `world_z`) reaches, grown by `margin_m` and clamped to the world grown
    /// by the same margin (`TERRAIN-28`). `None` when the brush can write no sample: a position
    /// or radius that is not finite, a radius not above zero, or a footprint wholly outside the
    /// world. Inside the world the rectangle is the footprint grown by the margin. O(1).
    pub(crate) fn brush_world_bounds(
        &self,
        world_x: f32,
        world_z: f32,
        radius_m: f32,
        margin_m: f32,
    ) -> Option<[f32; 4]> {
        if self.width == 0
            || self.height == 0
            || !(world_x.is_finite() && world_z.is_finite() && radius_m.is_finite())
            || radius_m <= 0.0
        {
            return None;
        }
        let (half_w, half_h) = self.half_world_extents();
        if world_x + radius_m < -half_w
            || world_x - radius_m > half_w
            || world_z + radius_m < -half_h
            || world_z - radius_m > half_h
        {
            return None;
        }
        let reach = radius_m + margin_m;
        let (limit_x, limit_z) = (half_w + margin_m, half_h + margin_m);
        Some([
            (world_x - reach).max(-limit_x),
            (world_z - reach).max(-limit_z),
            (world_x + reach).min(limit_x),
            (world_z + reach).min(limit_z),
        ])
    }

    fn border_position(&self, grid_x: usize, grid_z: usize) -> Vector3 {
        let (world_x, world_z) = self.grid_to_world_coords(grid_x, grid_z);
        Vector3::new(
            world_x,
            self.source_data.get(grid_x, grid_z),
            world_z,
        )
    }
}

fn render_patch_interval_cells(cell_size: f32, chunk_span_m: f32) -> usize {
    ((chunk_span_m / cell_size.max(f32::EPSILON)).round() as usize).max(1)
}

fn raycast_xz_interval(
    ray_origin: Vector3,
    ray_dir: Vector3,
    half_w: f32,
    half_h: f32,
) -> Option<(f32, f32)> {
    let (tx0, tx1) = raycast_axis_interval(ray_origin.x, ray_dir.x, -half_w, half_w)?;
    let (tz0, tz1) = raycast_axis_interval(ray_origin.z, ray_dir.z, -half_h, half_h)?;
    let entry_t = tx0.max(tz0);
    let exit_t = tx1.min(tz1);
    if exit_t < entry_t.max(0.0) {
        return None;
    }
    Some((entry_t, exit_t))
}

fn raycast_axis_interval(origin: f32, dir: f32, min: f32, max: f32) -> Option<(f32, f32)> {
    if dir.abs() <= f32::EPSILON {
        if origin < min || origin > max {
            return None;
        }
        return Some((f32::NEG_INFINITY, f32::INFINITY));
    }

    let mut t0 = (min - origin) / dir;
    let mut t1 = (max - origin) / dir;
    if t0 > t1 {
        std::mem::swap(&mut t0, &mut t1);
    }
    Some((t0, t1))
}

#[cfg(test)]
mod tests {
    use super::TerrainSystem;
    use godot::prelude::Vector3;
    use std::collections::HashSet;

    #[test]
    fn height_revisions_follow_changed_samples_per_512m_chunk() {
        // 2048 m world centred on zero: chunks -2..=1 on each axis, 8 m samples.
        let mut terrain = TerrainSystem::with_chunking(257, 257, 8.0, 64, 0.0);
        let revision = |terrain: &TerrainSystem, chunk| terrain.height_revision_for_chunk(chunk);
        assert_eq!(revision(&terrain, (0, 0)), 0);

        // World (256, 256) is interior: the draping margin stays inside chunk (0, 0).
        terrain.set_height(160, 160, 2.0);
        let interior = revision(&terrain, (0, 0));
        assert!(interior > 0);
        for chunk in [(1, 0), (0, 1), (-1, 0), (0, -1), (1, 1)] {
            assert_eq!(revision(&terrain, chunk), 0);
        }

        // World x = 496 m is within the margin of x = 512 m, so the neighbour moves too.
        terrain.set_height(190, 160, 1.0);
        assert!(revision(&terrain, (0, 0)) > interior);
        assert_eq!(revision(&terrain, (1, 0)), revision(&terrain, (0, 0)));
        assert_eq!(revision(&terrain, (-1, 0)), 0);

        // Unchanged source samples do not move a draped overlay.
        let before: Vec<_> = [(0, 0), (1, 0), (-2, -2)]
            .map(|chunk| revision(&terrain, chunk))
            .into();
        let after: Vec<_> = [(0, 0), (1, 0), (-2, -2)]
            .map(|chunk| revision(&terrain, chunk))
            .into();
        assert_eq!(after, before);

        // Whole-grid replacement supersedes every local revision.
        terrain.mark_all_render_patches_dirty();
        let epoch = revision(&terrain, (-2, 1));
        assert!(epoch > after[0]);
        assert_eq!(revision(&terrain, (0, 0)), epoch);
    }

    #[test]
    fn raycast_reaches_large_world_from_high_altitude_camera() {
        let terrain = TerrainSystem::with_chunking(1801, 1801, 10.0, 64, 100.0);
        let target = Vector3::new(0.0, 100.0, 0.0);
        let origin = Vector3::new(0.0, 20_800.0, 20_800.0);
        let ray_dir = (target - origin).normalized();

        let hit = terrain
            .raycast_terrain(origin, ray_dir)
            .expect("high-altitude ray should still hit terrain on a large world");

        assert!(hit.x.abs() < 1.0);
        assert!(hit.z.abs() < 1.0);
        assert!((hit.y - target.y).abs() < 1.0);
    }

    #[test]
    fn level_brush_moves_samples_toward_target_without_overshoot() {
        let mut terrain = TerrainSystem::with_chunking(9, 9, 10.0, 4, 0.0);
        terrain.set_height(4, 4, 1.0);
        terrain.set_height(5, 4, 4.0);

        terrain.level_to_height(4.0, 4.0, 2.0, 3.0, 0.5);

        assert!((terrain.get_height(4, 4) - 1.5).abs() < 0.0001);
        assert!((terrain.get_height(5, 4) - 3.75).abs() < 0.0001);
        assert!((terrain.get_height(0, 0) - 0.0).abs() < 0.0001);

        for _ in 0..10 {
            terrain.level_to_height(4.0, 4.0, 2.0, 3.0, 0.5);
        }

        assert!((terrain.get_height(4, 4) - 3.0).abs() < 0.0001);
        assert!((terrain.get_height(5, 4) - 3.0).abs() < 0.0001);
    }

    #[test]
    fn smooth_brush_moves_samples_toward_local_average_without_scan_bias() {
        let mut terrain = TerrainSystem::with_chunking(9, 9, 10.0, 4, 0.0);
        terrain.set_height(4, 4, 9.0);

        terrain.smooth(4.0, 4.0, 2.0, 1.0);

        assert!((terrain.get_height(4, 4) - 8.0).abs() < 0.0001);
        assert!((terrain.get_height(5, 4) - 0.5).abs() < 0.0001);
        assert!((terrain.get_height(4, 5) - 0.5).abs() < 0.0001);
        assert!((terrain.get_height(0, 0) - 0.0).abs() < 0.0001);
    }

    // `TERRAIN-28`: a brush at the world's corners and edges, partly or wholly outside it, or
    // with a radius larger than the world writes only samples of the grid and marks only its
    // patches and 512 m regions; one wholly outside (or with a centre or radius that is not
    // finite) writes and marks nothing. The marks' counts are bounded by the world.
    #[test]
    fn brushes_reaching_past_the_world_are_clamped_and_outside_it_refused() {
        // 1,020 m centred on zero, 10 m samples: with the draping margin, regions -2..=1.
        // Uneven, so that every brush (smooth too) moves the samples it reaches.
        let fresh = || {
            let mut terrain = TerrainSystem::with_chunking(103, 103, 10.0, 52, 0.0);
            let dense: Vec<f32> = (0..103 * 103).map(|i| (i % 7) as f32 * 0.25).collect();
            terrain
                .replace_source_from_dense(&dense)
                .expect("103 x 103 samples");
            terrain
        };
        let max_regions = 16;
        let max_patches = fresh().render_patch_cols() * fresh().render_patch_rows();
        type Brush = fn(&mut TerrainSystem, f32, f32, f32);
        let brushes: [Brush; 4] = [
            |t, x, y, r| t.sculpt(x, y, r, 0.5),
            |t, x, y, r| t.level_to_height(x, y, r, 2.0, 0.5),
            |t, x, y, r| t.smooth(x, y, r, 0.5),
            |t, x, y, r| t.slope_to_segment(x, y, r, x - 1.0, y, 4.0, x + 1.0, y, 6.0, 0.5),
        ];
        let heights = |t: &TerrainSystem| -> Vec<u32> {
            (0..t.height)
                .flat_map(|z| (0..t.width).map(move |x| (x, z)))
                .map(|(x, z)| t.get_height(x, z).to_bits())
                .collect()
        };
        // Grid centres 510 m (51 samples) out from each corner and edge, the `RENDER-32-Q`
        // sculpt's point among them; the same directions 1.5 samples out, which a 2.4-sample
        // radius still reaches.
        let around = |out: f32| {
            let (lo, mid, hi) = (-out, 51.0, 102.0 + out);
            [
                (lo, lo),
                (mid, lo),
                (hi, lo),
                (hi, mid),
                (hi, hi),
                (mid, hi),
                (lo, hi),
                (lo, mid),
            ]
        };
        for (index, brush) in brushes.into_iter().enumerate() {
            let outside = around(51.0).map(|(x, y)| (x, y, 2.4));
            let invalid = [
                (f32::NAN, 51.0, 2.4),
                (51.0, 51.0, f32::INFINITY),
                (51.0, 51.0, 0.0),
                (-1.0e30, 51.0, 2.4),
            ];
            for (x, y, radius) in outside.into_iter().chain(invalid) {
                let mut terrain = fresh();
                let (before, dirty) = (heights(&terrain), terrain.dirty_render_patches().clone());
                brush(&mut terrain, x, y, radius);
                assert_eq!(
                    heights(&terrain),
                    before,
                    "brush {index} ({x}, {y}) r {radius}: refused"
                );
                assert_eq!(terrain.dirty_render_patches(), &dirty);
                assert!(terrain.height_region_revisions.is_empty());
            }
            let reaching = around(1.5).map(|(x, y)| (x, y, 2.4));
            let larger = [(51.0, 51.0, 1.0e6), (-1.0e6, 51.0, 2.0e6)];
            for (x, y, radius) in reaching.into_iter().chain(larger) {
                let mut terrain = fresh();
                let before = heights(&terrain);
                brush(&mut terrain, x, y, radius);
                assert_ne!(
                    heights(&terrain),
                    before,
                    "brush {index} ({x}, {y}) r {radius}: writes"
                );
                assert!(terrain.dirty_render_patches().len() <= max_patches);
                assert!(terrain.height_region_revisions.len() <= max_regions);
            }
        }

        // The marks clamp whatever bounds a caller passes: here the wrapped bound the outside
        // sculpt derived before the clamp.
        let mut terrain = fresh();
        terrain.mark_render_patches_for_grid_rect(0, usize::MAX - 47, 0, usize::MAX - 47);
        assert_eq!(terrain.dirty_render_patches().len(), max_patches);
        assert!(terrain.height_region_revisions.len() <= max_regions);
    }

    #[test]
    fn slope_brush_moves_samples_toward_segment_grade_and_clamps_to_endpoints() {
        let mut terrain = TerrainSystem::with_chunking(9, 9, 10.0, 4, 0.0);

        terrain.slope_to_segment(4.0, 4.0, 3.0, 2.0, 4.0, 2.0, 6.0, 4.0, 6.0, 1.0);

        assert!((terrain.get_height(4, 4) - 1.0).abs() < 0.0001);
        assert!((terrain.get_height(2, 4) - 0.25).abs() < 0.0001);
        assert!((terrain.get_height(3, 4) - 0.75).abs() < 0.0001);
        assert!((terrain.get_height(6, 4) - 0.25).abs() < 0.0001);

        for _ in 0..30 {
            terrain.slope_to_segment(4.0, 4.0, 3.0, 2.0, 4.0, 2.0, 6.0, 4.0, 6.0, 1.0);
        }

        assert!((terrain.get_height(2, 4) - 2.0).abs() < 0.0001);
        assert!((terrain.get_height(3, 4) - 3.0).abs() < 0.0001);
        assert!((terrain.get_height(4, 4) - 4.0).abs() < 0.0001);
        assert!((terrain.get_height(5, 4) - 5.0).abs() < 0.0001);
        assert!((terrain.get_height(6, 4) - 6.0).abs() < 0.0001);
        assert!((terrain.get_height(1, 4) - 0.0).abs() < 0.0001);
        assert!((terrain.get_height(7, 4) - 0.0).abs() < 0.0001);
    }

    #[test]
    fn source_patch_snapshot_border_ring_samples_neighboring_terrain() {
        let mut terrain =
            TerrainSystem::with_chunking(9, 9, 10.0, 4, 0.0).with_render_chunk_span(30.0);
        terrain.set_height(2, 3, 22.0);
        terrain.set_height(3, 3, 33.0);
        terrain.set_height(6, 6, 66.0);
        terrain.set_height(7, 7, 77.0);

        let patch = terrain
            .source_patch_snapshot(1, 1)
            .expect("patch (1,1) should exist on a 9x9 terrain");

        assert_eq!(patch.patch_x, 1);
        assert_eq!(patch.patch_z, 1);
        assert_eq!(patch.sample_width, 4);
        assert_eq!(patch.sample_height, 4);
        assert_eq!(patch.texture_width, 12);
        assert_eq!(patch.texture_height, 12);
        assert_eq!(patch.inner_offset_x, 4);
        assert_eq!(patch.inner_offset_z, 4);
        assert!((patch.world_origin_x + 10.0).abs() < 0.0001);
        assert!((patch.world_origin_z + 10.0).abs() < 0.0001);
        assert!((patch.world_size_x - 30.0).abs() < 0.0001);
        assert!((patch.world_size_z - 30.0).abs() < 0.0001);
        assert_eq!(patch.height_data[0], 0.0);
        assert_eq!(patch.height_data[patch.texture_width * 4 + 3], 22.0);
        assert_eq!(patch.height_data[patch.texture_width * 4 + 4], 33.0);
        assert_eq!(patch.height_data[patch.texture_width * 8 + 8], 77.0);
    }

    #[test]
    fn point_edit_marks_all_overlapping_render_patches() {
        let mut terrain =
            TerrainSystem::with_chunking(17, 17, 10.0, 4, 0.0).with_render_chunk_span(30.0);

        terrain.set_height(3, 3, 5.0);

        let dirty: HashSet<(usize, usize)> =
            terrain.dirty_render_patches().iter().copied().collect();
        let expected = HashSet::from([(0, 0), (1, 0), (0, 1), (1, 1)]);
        assert_eq!(dirty, expected);
    }

    #[test]
    fn world_bounds_on_patch_boundary_return_all_overlapping_render_patches() {
        let terrain =
            TerrainSystem::with_chunking(17, 17, 10.0, 4, 0.0).with_render_chunk_span(30.0);

        let (world_x, world_z) = terrain.grid_to_world_coords(3, 3);
        let keys: HashSet<(usize, usize)> = terrain
            .render_patch_keys_for_world_bounds(world_x, world_z, world_x, world_z)
            .into_iter()
            .collect();

        let expected = HashSet::from([(0, 0), (1, 0), (0, 1), (1, 1)]);
        assert_eq!(keys, expected);
    }

    #[test]
    fn world_bounds_use_axis_specific_patch_counts_on_rectangular_terrain() {
        let terrain =
            TerrainSystem::with_chunking(9, 17, 10.0, 4, 0.0).with_render_chunk_span(30.0);

        let (world_x, world_z) = terrain.grid_to_world_coords(4, 14);
        let keys: HashSet<(usize, usize)> = terrain
            .render_patch_keys_for_world_bounds(world_x, world_z, world_x, world_z)
            .into_iter()
            .collect();

        assert_eq!(terrain.render_patch_cols(), 3);
        assert_eq!(terrain.render_patch_rows(), 6);
        assert_eq!(keys, HashSet::from([(1, 4)]));
    }
}
