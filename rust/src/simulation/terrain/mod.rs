//! Heightmap terrain system used for road grade, raycasting, and rendering.
//!
//! Two height arrays are maintained: `source_data` (user-sculpted, never modified by roads)
//! and `data` (derived visual terrain outside client-owned surfaces). Ordinary grounded roads do
//! not stamp their footprint into either array; road-touched render patches receive stitched mesh
//! topology from the road surface runtime instead.

pub(crate) mod cdt;
pub mod chunks;

pub use chunks::{
    TerrainChunkAsset, TerrainChunkLoadError, TerrainChunkLodAsset, TerrainChunkLodManifest,
    TerrainChunkManifest, TerrainChunkManifestError,
};

use godot::prelude::Vector3;
use std::collections::HashSet;

use crate::config::HEIGHT_SCALE;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::sparse_chunk_grid::SparseChunkGrid;

const DEFAULT_TERRAIN_CHUNK_CELLS: usize = 64;
const TERRAIN_RENDER_PATCH_BORDER_TEXELS: usize = 4;
const TERRAIN_CDT_LOCAL_MIN_SAMPLE_MARGIN_M: f32 = 8.0;
const TERRAIN_CDT_LOCAL_SAMPLE_MARGIN_RENDER_STEPS: f32 = 4.0;
const TERRAIN_CDT_LOCAL_SAMPLE_MARGIN_TERRAIN_CELLS: f32 = 2.0;

/// Returns the deterministic seam margin used by local terrain-CDT windows.
pub(crate) fn terrain_cdt_local_sample_margin_m(
    terrain: &TerrainSystem,
    render_step_m: f32,
) -> f32 {
    TERRAIN_CDT_LOCAL_MIN_SAMPLE_MARGIN_M
        .max(render_step_m.max(f32::EPSILON) * TERRAIN_CDT_LOCAL_SAMPLE_MARGIN_RENDER_STEPS)
        .max(terrain.cell_size_m() * TERRAIN_CDT_LOCAL_SAMPLE_MARGIN_TERRAIN_CELLS)
}

/// Expands a road grading margin by the patch-selection safety pad used around grading rays.
///
/// A patch can be selected by the pad even when the road seam lies just beyond the unpadded
/// grading margin. Clip-source queries must include the same pad or that selected patch can be
/// misclassified as road-owned with no road sources.
pub(crate) fn terrain_cdt_road_query_margin_m(
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
    /// Row-major visual-terrain samples including the border ring.
    pub height_data: Vec<f32>,
}

/// Dual-buffer heightmap for the terrain surface.
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
    /// Derived visual heightmap outside client-owned surfaces.
    data: SparseChunkGrid<f32>,
    /// Source heightmap as sculpted by the player, without road modifications.
    /// Used for road grade calculation and slope cost — never written by road placement.
    source_data: SparseChunkGrid<f32>,
    /// Render patches whose visible terrain textures must be refreshed.
    dirty_render_patches: HashSet<(usize, usize)>,
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
            data: SparseChunkGrid::new(width, height, safe_chunk_size, base_elevation),
            source_data: SparseChunkGrid::new(width, height, safe_chunk_size, base_elevation),
            dirty_render_patches: HashSet::new(),
        }
    }

    /// Creates a terrain system from the current world configuration.
    pub fn from_world_config(config: &WorldConfig) -> Self {
        Self::with_chunking(
            config.terrain_grid_width(),
            config.terrain_grid_height(),
            config.terrain_cell_m,
            terrain_chunk_cells_for_config(config),
            config.terrain_base_elevation_m,
        )
        .with_render_chunk_span(config.terrain_chunk_m)
    }

    fn with_render_chunk_span(mut self, chunk_span_m: f32) -> Self {
        self.chunk_span_m = chunk_span_m.max(self.cell_size);
        self.render_patch_interval_cells =
            render_patch_interval_cells(self.cell_size, self.chunk_span_m);
        self
    }

    /// Sets the height at a specific grid coordinate.
    ///
    /// Updates both source and visual buffers.
    pub fn set_height(&mut self, x: usize, y: usize, value: f32) {
        if x < self.width && y < self.height {
            self.source_data.set(x, y, value);
            self.data.set(x, y, value);
            self.mark_render_patches_for_grid_rect(x, x, y, y);
        }
    }

    /// Gets the raw source height at a grid coordinate.
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

    /// Samples the current visual terrain buffer at one world-space position.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn sample_visual_height_world(&self, world_x: f32, world_z: f32) -> f32 {
        let (grid_x, grid_z) = self.world_to_grid_coords(world_x, world_z);
        self.interpolate_grid_height(&self.data, grid_x, grid_z)
    }

    /// Returns the terrain sample spacing in metres.
    pub(crate) fn cell_size_m(&self) -> f32 {
        self.cell_size
    }

    /// Calculates the surface normal at a fractional coordinate using gradient sampling.
    pub fn get_normal_interpolated(&self, x: f32, z: f32) -> Vector3 {
        let eps = 0.1;
        let h_x1 = self.get_height_interpolated(x + eps, z);
        let h_x0 = self.get_height_interpolated(x - eps, z);
        let h_z1 = self.get_height_interpolated(x, z + eps);
        let h_z0 = self.get_height_interpolated(x, z - eps);

        let dx = (h_x1 - h_x0) / (2.0 * eps);
        let dz = (h_z1 - h_z0) / (2.0 * eps);

        // This is the gradient in "unit" height units.
        // We need to scale it by our world height scale (20.0) for accurate slope.
        Vector3::new(-dx * 20.0, 1.0, -dz * 20.0).normalized()
    }

    fn interpolate_grid_height(&self, grid: &SparseChunkGrid<f32>, x: f32, z: f32) -> f32 {
        let x_clamped = x.clamp(0.0, (self.width - 1) as f32);
        let z_clamped = z.clamp(0.0, (self.height - 1) as f32);

        let x0 = x_clamped.floor() as usize;
        let z0 = z_clamped.floor() as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let z1 = (z0 + 1).min(self.height - 1);

        let fx = x_clamped.fract();
        let fz = z_clamped.fract();

        let h00 = grid.get(x0, z0);
        let h10 = grid.get(x1, z0);
        let h01 = grid.get(x0, z1);
        let h11 = grid.get(x1, z1);

        let h0 = h00 * (1.0 - fx) + h10 * fx;
        let h1 = h01 * (1.0 - fx) + h11 * fx;

        h0 * (1.0 - fz) + h1 * fz
    }

    /// Casts a ray against the terrain surface and returns the intersection point.
    pub fn raycast_terrain(&self, ray_origin: Vector3, ray_dir: Vector3) -> Option<Vector3> {
        self.raycast_height_field(ray_origin, ray_dir, false)
    }

    /// Casts a ray against the current visual terrain surface.
    pub(crate) fn raycast_visual_terrain(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<Vector3> {
        self.raycast_height_field(ray_origin, ray_dir, true)
    }

    fn raycast_height_field(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
        visual: bool,
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
            - self.sample_height_world_impl(prev_point.x, prev_point.z, visual) * HEIGHT_SCALE;

        while prev_t < max_t {
            let t = (prev_t + step).min(max_t);
            let p = ray_origin + ray_dir * t;
            let h = self.sample_height_world_impl(p.x, p.z, visual) * HEIGHT_SCALE;
            let diff = p.y - h;

            if diff == 0.0 || diff.signum() != prev_diff.signum() {
                // Binary search refinement for precision.
                let mut t_low = prev_t;
                let mut t_high = t;
                for _ in 0..8 {
                    let t_mid = (t_low + t_high) * 0.5;
                    let pm = ray_origin + ray_dir * t_mid;
                    let hm = self.sample_height_world_impl(pm.x, pm.z, visual) * HEIGHT_SCALE;
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

    fn sample_height_world_impl(&self, world_x: f32, world_z: f32, visual: bool) -> f32 {
        if visual {
            self.sample_visual_height_world(world_x, world_z)
        } else {
            self.sample_height_world(world_x, world_z)
        }
    }

    /// Modifies terrain height in a circular area with smooth falloff.
    pub fn sculpt(&mut self, center_x: f32, center_y: f32, radius: f32, strength: f32) {
        let Some((min_x, max_x, min_y, max_y)) = self.brush_grid_bounds(center_x, center_y, radius)
        else {
            return;
        };
        self.mark_render_patches_for_grid_rect(min_x, max_x, min_y, max_y);
        let r_int = radius.ceil() as i32;
        let cx_int = center_x as i32;
        let cy_int = center_y as i32;

        for y in (cy_int - r_int)..=(cy_int + r_int) {
            for x in (cx_int - r_int)..=(cx_int + r_int) {
                if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
                    continue;
                }

                let dx = x as f32 - center_x;
                let dy = y as f32 - center_y;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq <= radius * radius {
                    let dist = dist_sq.sqrt();
                    let normalized_dist = dist / radius;
                    let falloff = (1.0 + (normalized_dist * std::f32::consts::PI).cos()) * 0.5;

                    let ux = x as usize;
                    let uy = y as usize;
                    let current_h = self.source_data.get(ux, uy);
                    let next_h = current_h + strength * falloff;

                    self.source_data.set(ux, uy, next_h);
                    self.data.set(ux, uy, next_h);
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
        self.mark_render_patches_for_grid_rect(min_x, max_x, min_y, max_y);
        let r_int = radius.ceil() as i32;
        let cx_int = center_x as i32;
        let cy_int = center_y as i32;

        for y in (cy_int - r_int)..=(cy_int + r_int) {
            for x in (cx_int - r_int)..=(cx_int + r_int) {
                if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
                    continue;
                }

                let dx = x as f32 - center_x;
                let dy = y as f32 - center_y;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq <= radius * radius {
                    let dist = dist_sq.sqrt();
                    let normalized_dist = dist / radius;
                    let falloff = (1.0 + (normalized_dist * std::f32::consts::PI).cos()) * 0.5;

                    let ux = x as usize;
                    let uy = y as usize;
                    let current_h = self.source_data.get(ux, uy);
                    let delta = target_height - current_h;
                    let max_step = strength * falloff;
                    let next_h = if delta.abs() <= max_step {
                        target_height
                    } else {
                        current_h + delta.signum() * max_step
                    };

                    self.source_data.set(ux, uy, next_h);
                    self.data.set(ux, uy, next_h);
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
        self.mark_render_patches_for_grid_rect(min_x, max_x, min_y, max_y);
        let r_int = radius.ceil() as i32;
        let cx_int = center_x as i32;
        let cy_int = center_y as i32;

        let min_x = (cx_int - r_int - 1).max(0) as usize;
        let max_x = (cx_int + r_int + 1).min(self.width as i32 - 1) as usize;
        let min_y = (cy_int - r_int - 1).max(0) as usize;
        let max_y = (cy_int + r_int + 1).min(self.height as i32 - 1) as usize;

        let patch_w = max_x - min_x + 1;
        let patch_h = max_y - min_y + 1;
        let mut patch = vec![0.0_f32; patch_w * patch_h];
        for local_y in 0..patch_h {
            for local_x in 0..patch_w {
                patch[local_y * patch_w + local_x] =
                    self.source_data.get(min_x + local_x, min_y + local_y);
            }
        }

        for y in (cy_int - r_int)..=(cy_int + r_int) {
            for x in (cx_int - r_int)..=(cx_int + r_int) {
                if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
                    continue;
                }

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

                let ux = x as usize;
                let uy = y as usize;
                let local_x = ux - min_x;
                let local_y = uy - min_y;
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

                self.source_data.set(ux, uy, next_h);
                self.data.set(ux, uy, next_h);
            }
        }
    }

    /// Moves terrain toward a clamped linear grade defined by two clicked anchor points.
    ///
    /// The target height for each touched sample is the linear interpolation between the two
    /// anchor heights, projected onto the anchor segment and clamped to the segment endpoints.
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

        let r_int = radius.ceil() as i32;
        let cx_int = center_x as i32;
        let cy_int = center_y as i32;

        for y in (cy_int - r_int)..=(cy_int + r_int) {
            for x in (cx_int - r_int)..=(cx_int + r_int) {
                if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
                    continue;
                }

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

                let ux = x as usize;
                let uy = y as usize;
                let current_h = self.source_data.get(ux, uy);
                let delta = target_height - current_h;
                let next_h = if delta.abs() <= max_step {
                    target_height
                } else {
                    current_h + delta.signum() * max_step
                };

                self.source_data.set(ux, uy, next_h);
                self.data.set(ux, uy, next_h);
            }
        }
    }

    /// Synchronizes the visual data buffer with the source data.
    pub fn reset_visuals_from_source(&mut self) {
        self.data = self.source_data.clone();
        self.mark_all_render_patches_dirty();
    }

    /// Synchronizes one world-space visual region from the authoritative source terrain.
    pub(crate) fn reset_visual_region_from_source_world(
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

        for grid_z in min_grid_z..=max_grid_z {
            for grid_x in min_grid_x..=max_grid_x {
                self.data
                    .set(grid_x, grid_z, self.source_data.get(grid_x, grid_z));
            }
        }
    }

    /// Returns a dense row-major snapshot of the visual terrain buffer.
    pub(crate) fn clone_visual_dense(&self) -> Vec<f32> {
        self.data.clone_dense()
    }

    /// Returns a dense row-major snapshot of the authoritative source terrain buffer.
    pub(crate) fn clone_source_dense(&self) -> Vec<f32> {
        self.source_data.clone_dense()
    }

    /// Returns source terrain as rendered world-space metres for water and preview solves.
    pub(crate) fn clone_source_dense_world_heights(&self) -> Vec<f32> {
        self.source_data
            .clone_dense()
            .into_iter()
            .map(|sample| sample * HEIGHT_SCALE)
            .collect()
    }

    /// Replaces the visual terrain buffer from a dense row-major snapshot.
    pub(crate) fn replace_visual_from_dense(&mut self, dense: &[f32]) -> Result<(), String> {
        self.data.replace_from_dense(dense)?;
        self.mark_all_render_patches_dirty();
        Ok(())
    }

    /// Replaces the authoritative source terrain buffer from a dense row-major snapshot.
    pub(crate) fn replace_source_from_dense(&mut self, dense: &[f32]) -> Result<(), String> {
        self.source_data.replace_from_dense(dense)
    }

    /// Writes one visual terrain sample after the caller has marked the enclosing render region.
    pub(crate) fn set_visual_height_at_grid_unmarked(
        &mut self,
        grid_x: usize,
        grid_z: usize,
        value: f32,
    ) {
        self.data.set(grid_x, grid_z, value);
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
        let Some((min_grid_x, max_grid_x, min_grid_z, max_grid_z)) =
            self.grid_rect_for_world_bounds(min_x, min_z, max_x, max_z)
        else {
            return Vec::new();
        };

        let (min_patch_x, max_patch_x) =
            self.patch_range_for_sample_range(min_grid_x, max_grid_x, self.render_patch_cols());
        let (min_patch_z, max_patch_z) =
            self.patch_range_for_sample_range(min_grid_z, max_grid_z, self.render_patch_rows());
        let mut keys =
            Vec::with_capacity((max_patch_x - min_patch_x + 1) * (max_patch_z - min_patch_z + 1));
        for patch_z in min_patch_z..=max_patch_z {
            for patch_x in min_patch_x..=max_patch_x {
                keys.push((patch_x, patch_z));
            }
        }
        keys
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

    /// Returns the terrain grid dimensions in samples.
    pub(crate) fn grid_dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Marks every terrain render patch dirty after a whole-world replacement.
    pub(crate) fn mark_all_render_patches_dirty(&mut self) {
        for patch_z in 0..self.render_patch_rows() {
            for patch_x in 0..self.render_patch_cols() {
                self.dirty_render_patches.insert((patch_x, patch_z));
            }
        }
    }

    /// Returns one visual-terrain render patch with a neighboring-sample border ring.
    pub(crate) fn visual_patch_snapshot(
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

        for local_z in 0..texture_height {
            let sample_z = bordered_global_index(
                start_z,
                local_z,
                TERRAIN_RENDER_PATCH_BORDER_TEXELS,
                self.height,
            );
            for local_x in 0..texture_width {
                let sample_x = bordered_global_index(
                    start_x,
                    local_x,
                    TERRAIN_RENDER_PATCH_BORDER_TEXELS,
                    self.width,
                );
                height_data[local_z * texture_width + local_x] = self.data.get(sample_x, sample_z);
            }
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
        if min_sample > 0 && min_sample % self.render_patch_interval_cells == 0 {
            patch_min = patch_min.saturating_sub(1);
        }
        let mut patch_max = max_sample / self.render_patch_interval_cells;
        if max_sample > 0 && max_sample % self.render_patch_interval_cells == 0 {
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

    fn brush_grid_bounds(
        &self,
        center_x: f32,
        center_y: f32,
        radius: f32,
    ) -> Option<(usize, usize, usize, usize)> {
        if self.width == 0 || self.height == 0 {
            return None;
        }
        let r_int = radius.ceil() as i32;
        let cx_int = center_x as i32;
        let cy_int = center_y as i32;
        Some((
            (cx_int - r_int).max(0) as usize,
            (cx_int + r_int).min(self.width as i32 - 1) as usize,
            (cy_int - r_int).max(0) as usize,
            (cy_int + r_int).min(self.height as i32 - 1) as usize,
        ))
    }

    fn border_position(&self, grid_x: usize, grid_z: usize) -> Vector3 {
        let (world_x, world_z) = self.grid_to_world_coords(grid_x, grid_z);
        Vector3::new(
            world_x,
            self.data.get(grid_x, grid_z) * HEIGHT_SCALE,
            world_z,
        )
    }
}

fn terrain_chunk_cells_for_config(config: &WorldConfig) -> usize {
    ((config.terrain_chunk_m / config.terrain_cell_m).ceil() as usize).max(1)
}

fn render_patch_interval_cells(cell_size: f32, chunk_span_m: f32) -> usize {
    ((chunk_span_m / cell_size.max(f32::EPSILON)).round() as usize).max(1)
}

fn bordered_global_index(
    start: usize,
    bordered_index: usize,
    border_texels: usize,
    limit: usize,
) -> usize {
    let max_index = limit.saturating_sub(1) as isize;
    let index = start as isize + bordered_index as isize - border_texels as isize;
    index.clamp(0, max_index) as usize
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
    use crate::config::HEIGHT_SCALE;
    use godot::prelude::Vector3;
    use std::collections::HashSet;

    #[test]
    fn raycast_reaches_large_world_from_high_altitude_camera() {
        let terrain = TerrainSystem::with_chunking(1801, 1801, 10.0, 64, 5.0);
        let target = Vector3::new(0.0, 5.0 * HEIGHT_SCALE, 0.0);
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
    fn visual_patch_snapshot_border_ring_samples_neighboring_terrain() {
        let mut terrain =
            TerrainSystem::with_chunking(9, 9, 10.0, 4, 0.0).with_render_chunk_span(30.0);
        terrain.set_height(2, 3, 22.0);
        terrain.set_height(3, 3, 33.0);
        terrain.set_height(6, 6, 66.0);
        terrain.set_height(7, 7, 77.0);

        let patch = terrain
            .visual_patch_snapshot(1, 1)
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
