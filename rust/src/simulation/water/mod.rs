//! Water runtime split into authored baseline water plus dynamic flowing water.
//!
//! Baseline water owns flat hydrostatic still-water bodies authored by the world
//! editor. Dynamic water owns transient additional depth, velocity, and flux on
//! top of either dry terrain or one of those baseline water surfaces.

use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::sparse_chunk_grid::SparseChunkGrid;
use rayon::prelude::*;
use std::collections::HashSet;

const DEFAULT_WATER_CHUNK_CELLS: usize = 64;
const MAX_DYNAMIC_WATER_SUBSTEP_DT: f32 = 0.05;
const WATER_RENDER_PATCH_BORDER_TEXELS: usize = 1;
const WATER_DEBUG_VISIBLE_EPSILON: f32 = 0.001;

/// One deterministic render-patch snapshot of visible water.
pub(crate) struct WaterPatchSnapshot {
    /// Patch X index on the render-patch grid.
    pub patch_x: usize,
    /// Patch Z index on the render-patch grid.
    pub patch_z: usize,
    /// Number of owned samples across X without the border ring.
    pub sample_width: usize,
    /// Number of owned samples across Z without the border ring.
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
    /// Row-major visible water depth samples including the border ring.
    pub depth_data: Vec<f32>,
    /// Row-major dynamic-water velocity samples including the border ring.
    pub velocity_data: Vec<f32>,
}

/// Debug-only layer split for one visible water render patch.
pub(crate) struct WaterPatchLayerStats {
    /// Number of row-major samples in the patch texture including the border ring.
    pub(crate) total_samples: usize,
    /// Number of samples whose authored baseline depth is visibly non-zero.
    pub(crate) baseline_nonzero: usize,
    /// Maximum authored baseline depth in metres.
    pub(crate) baseline_max: f32,
    /// Sum of authored baseline depth samples in metres.
    pub(crate) baseline_sum: f32,
    /// Number of samples whose dynamic source/sink depth is visibly non-zero.
    pub(crate) dynamic_nonzero: usize,
    /// Maximum dynamic source/sink depth in metres.
    pub(crate) dynamic_max: f32,
    /// Sum of dynamic source/sink depth samples in metres.
    pub(crate) dynamic_sum: f32,
    /// Number of samples whose composed visible depth is visibly non-zero.
    pub(crate) combined_nonzero: usize,
    /// Maximum composed visible depth in metres.
    pub(crate) combined_max: f32,
    /// Sum of composed visible depth samples in metres.
    pub(crate) combined_sum: f32,
    /// Number of samples whose dynamic velocity is visibly non-zero.
    pub(crate) velocity_nonzero: usize,
    /// Maximum dynamic velocity magnitude in metres per second.
    pub(crate) velocity_max: f32,
    /// Sum of dynamic velocity magnitudes in metres per second.
    pub(crate) velocity_sum: f32,
    /// Number of authored source/sink boundary points inside the owned patch samples.
    pub(crate) source_count_in_patch: usize,
    /// Signed sum of authored source/sink rates inside the owned patch samples.
    pub(crate) source_rate_sum: f32,
    /// Absolute sum of authored source/sink rates inside the owned patch samples.
    pub(crate) source_rate_abs_sum: f32,
    /// Total authored source/sink boundary points in the runtime water layer.
    pub(crate) source_count_total: usize,
}

struct BaselineWaterState {
    depth: SparseChunkGrid<f32>,
}

impl BaselineWaterState {
    fn new(width: usize, height: usize, chunk_size: usize) -> Self {
        Self {
            depth: SparseChunkGrid::new(width, height, chunk_size.max(1), 0.0),
        }
    }

    fn clone_dense(&self) -> Vec<f32> {
        self.depth.clone_dense()
    }

    fn replace_from_dense(&mut self, dense: &[f32]) -> Result<(), String> {
        self.depth.replace_from_dense(dense)
    }
}

struct DynamicWaterState {
    depth: SparseChunkGrid<f32>,
    velocity: SparseChunkGrid<f32>,
    flux: SparseChunkGrid<[f32; 4]>,
    sources: Vec<(usize, usize, f32)>,
}

impl DynamicWaterState {
    fn new(width: usize, height: usize, chunk_size: usize) -> Self {
        Self {
            depth: SparseChunkGrid::new(width, height, chunk_size.max(1), 0.0),
            velocity: SparseChunkGrid::new(width, height, chunk_size.max(1), 0.0),
            flux: SparseChunkGrid::new(width, height, chunk_size.max(1), [0.0; 4]),
            sources: Vec::new(),
        }
    }

    fn tick(&mut self, support_surface: &[f32], dt: f32, width: usize, height: usize) {
        if dt <= 0.0 || support_surface.len() != width * height || width < 3 || height < 3 {
            return;
        }

        let substeps = ((dt / MAX_DYNAMIC_WATER_SUBSTEP_DT).ceil() as usize).max(1);
        let substep_dt = dt / substeps as f32;
        for _ in 0..substeps {
            self.tick_substep(support_surface, substep_dt, width, height);
        }
    }

    fn tick_substep(&mut self, support_surface: &[f32], dt: f32, width: usize, height: usize) {
        let mut depth = self.depth.clone_dense();
        let mut velocity = self.velocity.clone_dense();
        let mut flux = self.flux.clone_dense();

        for &(x, y, rate) in &self.sources {
            let idx = y * width + x;
            depth[idx] = (depth[idx] + rate * dt).max(0.0);
        }

        let l = 1.0;
        let a = 1.0;
        let g = 9.81;
        let w = width;
        let h = height;
        let depth_ref = &depth;
        let support_ref = support_surface;

        flux.par_chunks_mut(w)
            .enumerate()
            .for_each(|(y, row_flux)| {
                if y == 0 || y >= h - 1 {
                    return;
                }

                for x in 1..w - 1 {
                    let idx = y * w + x;
                    if depth_ref[idx] <= 1e-6 && row_flux[x].iter().all(|&f| f <= 0.0) {
                        let n_top = (y - 1) * w + x;
                        let n_bottom = (y + 1) * w + x;
                        if depth_ref[idx - 1] <= 1e-6
                            && depth_ref[idx + 1] <= 1e-6
                            && depth_ref[n_top] <= 1e-6
                            && depth_ref[n_bottom] <= 1e-6
                        {
                            continue;
                        }
                    }

                    let surface_self = support_ref[idx] + depth_ref[idx];
                    let mut cell_flux = row_flux[x];
                    let nx = [x - 1, x + 1, x, x];
                    let ny = [y, y, y - 1, y + 1];

                    for i in 0..4 {
                        let n_idx = ny[i] * w + nx[i];
                        let surface_neighbor = support_ref[n_idx] + depth_ref[n_idx];
                        let h_diff = surface_self - surface_neighbor;
                        cell_flux[i] = (cell_flux[i] + dt * g * a * (h_diff / l)).max(0.0);
                    }

                    let total_flux = cell_flux[0] + cell_flux[1] + cell_flux[2] + cell_flux[3];
                    if total_flux > 0.0 {
                        let k = (depth_ref[idx] * l * l / (total_flux * dt)).min(1.0);
                        for value in &mut cell_flux {
                            *value *= k;
                        }
                    }

                    row_flux[x] = cell_flux;
                }
            });

        let flux_ref = &flux;
        depth
            .par_chunks_mut(w)
            .enumerate()
            .for_each(|(y, row_depth)| {
                if y == 0 || y >= h - 1 {
                    return;
                }
                for x in 1..w - 1 {
                    let idx = y * w + x;
                    let fin = flux_ref[idx - 1][1]
                        + flux_ref[idx + 1][0]
                        + flux_ref[idx - w][3]
                        + flux_ref[idx + w][2];
                    let fout =
                        flux_ref[idx][0] + flux_ref[idx][1] + flux_ref[idx][2] + flux_ref[idx][3];

                    if fin <= 1e-8 && fout <= 1e-8 && row_depth[x] <= 1e-6 {
                        continue;
                    }

                    row_depth[x] += dt * (fin - fout) / (l * l);
                    if row_depth[x] < 0.0001 {
                        row_depth[x] = 0.0;
                    }
                }
            });

        let depth_ref_2 = &depth;
        velocity
            .par_chunks_mut(w)
            .enumerate()
            .for_each(|(y, row_vel)| {
                if y == 0 || y >= h - 1 {
                    return;
                }
                for x in 1..w - 1 {
                    let idx = y * w + x;
                    if depth_ref_2[idx] > 0.001 {
                        let fin = flux_ref[idx - 1][1]
                            + flux_ref[idx + 1][0]
                            + flux_ref[idx - w][3]
                            + flux_ref[idx + w][2];
                        let fout = flux_ref[idx][0]
                            + flux_ref[idx][1]
                            + flux_ref[idx][2]
                            + flux_ref[idx][3];
                        row_vel[x] = (fin + fout) / (2.0 * depth_ref_2[idx] * l);
                    } else {
                        row_vel[x] = 0.0;
                    }
                }
            });

        self.depth
            .replace_from_dense(&depth)
            .expect("dynamic water depth snapshot must match the live water dimensions");
        self.velocity
            .replace_from_dense(&velocity)
            .expect("dynamic water velocity snapshot must match the live water dimensions");
        self.flux
            .replace_from_dense(&flux)
            .expect("dynamic water flux snapshot must match the live water dimensions");
    }

    fn clear_runtime_state(&mut self) {
        let zero_depth = vec![0.0; self.depth.clone_dense().len()];
        let zero_velocity = vec![0.0; self.velocity.clone_dense().len()];
        let zero_flux = vec![[0.0; 4]; self.flux.clone_dense().len()];
        self.depth
            .replace_from_dense(&zero_depth)
            .expect("zero dynamic water depth snapshot must match dimensions");
        self.velocity
            .replace_from_dense(&zero_velocity)
            .expect("zero dynamic water velocity snapshot must match dimensions");
        self.flux
            .replace_from_dense(&zero_flux)
            .expect("zero dynamic water flux snapshot must match dimensions");
    }
}

/// Water runtime for one world.
///
/// Baseline water stores flat authored still-water bodies. Dynamic water stores
/// transient additional depth, velocity, and flux on top of either terrain or
/// baseline-water support surfaces.
pub struct WaterSystem {
    /// Grid width in cells (matches terrain width).
    pub width: usize,
    /// Grid height in cells (matches terrain height).
    pub height: usize,
    /// Water sample spacing in metres.
    cell_size: f32,
    /// Authored terrain-chunk span in metres used to derive render-patch ownership.
    chunk_span_m: f32,
    /// Number of terrain intervals owned by one render patch along one axis.
    render_patch_interval_cells: usize,
    baseline: BaselineWaterState,
    dynamic: DynamicWaterState,
    /// Render patches whose visible water textures must be refreshed.
    dirty_render_patches: HashSet<(usize, usize)>,
}

impl WaterSystem {
    /// Creates a new, dry water system of the given dimensions.
    pub fn new(width: usize, height: usize) -> Self {
        Self::with_chunking(width, height, 1.0, DEFAULT_WATER_CHUNK_CELLS)
    }

    /// Creates a new water system with an explicit sparse chunk size.
    pub fn with_chunking(width: usize, height: usize, cell_size: f32, chunk_size: usize) -> Self {
        let safe_cell_size = cell_size.max(f32::EPSILON);
        let safe_chunk_size = chunk_size.max(1);
        Self {
            width,
            height,
            cell_size: safe_cell_size,
            chunk_span_m: safe_cell_size * safe_chunk_size.saturating_sub(1) as f32,
            render_patch_interval_cells: safe_chunk_size.saturating_sub(1).max(1),
            baseline: BaselineWaterState::new(width, height, safe_chunk_size),
            dynamic: DynamicWaterState::new(width, height, safe_chunk_size),
            dirty_render_patches: HashSet::new(),
        }
    }

    /// Creates a water system from the current world configuration.
    pub fn from_world_config(config: &WorldConfig) -> Self {
        Self::with_chunking(
            config.terrain_grid_width(),
            config.terrain_grid_height(),
            config.terrain_cell_m,
            water_chunk_cells_for_config(config),
        )
        .with_render_chunk_span(config.terrain_chunk_m)
    }

    fn with_render_chunk_span(mut self, chunk_span_m: f32) -> Self {
        self.chunk_span_m = chunk_span_m.max(self.cell_size);
        self.render_patch_interval_cells =
            render_patch_interval_cells(self.cell_size, self.chunk_span_m);
        self
    }

    /// Advances dynamic water by `dt` seconds over the terrain or baseline-water support surface.
    pub fn tick(&mut self, terrain_world: &[f32], dt: f32) {
        if terrain_world.len() != self.width * self.height {
            return;
        }
        let baseline_depth = self.baseline.clone_dense();
        let mut support_surface = terrain_world.to_vec();
        for (support, baseline_depth) in support_surface.iter_mut().zip(baseline_depth.iter()) {
            *support += *baseline_depth;
        }
        self.dynamic
            .tick(&support_surface, dt, self.width, self.height);
        self.mark_all_render_patches_dirty();
    }

    /// Adds a discrete amount of dynamic water depth to a specific grid cell.
    pub fn add_water(&mut self, x: usize, y: usize, amount: f32) {
        if x < self.width && y < self.height {
            let mut dynamic_depth = self.dynamic.depth.get(x, y) + amount;
            if dynamic_depth < 0.0 {
                dynamic_depth = 0.0;
            }
            self.dynamic.depth.set(x, y, dynamic_depth);
            self.mark_render_patches_for_grid_rect(x, x, y, y);
        }
    }

    /// Updates or adds one dynamic water boundary point at a specific grid cell.
    pub fn update_source(&mut self, x: usize, y: usize, rate_add: f32) {
        if x >= self.width || y >= self.height {
            return;
        }

        if let Some(source) = self
            .dynamic
            .sources
            .iter_mut()
            .find(|source| source.0 == x && source.1 == y)
        {
            source.2 += rate_add;
        } else {
            self.dynamic.sources.push((x, y, rate_add));
        }
        self.mark_render_patches_for_grid_rect(x, x, y, y);
    }

    /// Returns `true` when at least one dynamic water boundary point exists.
    pub(crate) fn has_sources(&self) -> bool {
        !self.dynamic.sources.is_empty()
    }

    /// Returns a dense row-major snapshot of baseline water depth above terrain.
    pub(crate) fn clone_baseline_depth_dense(&self) -> Vec<f32> {
        self.baseline.clone_dense()
    }

    /// Returns a dense row-major snapshot of dynamic water depth above the local support surface.
    pub(crate) fn clone_dynamic_depth_dense(&self) -> Vec<f32> {
        self.dynamic.depth.clone_dense()
    }

    /// Returns a dense row-major snapshot of total visible water depth above terrain.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn clone_depth_dense(&self) -> Vec<f32> {
        let baseline = self.baseline.clone_dense();
        let dynamic = self.dynamic.depth.clone_dense();
        baseline
            .into_iter()
            .zip(dynamic)
            .map(|(baseline_depth, dynamic_depth)| baseline_depth + dynamic_depth)
            .collect()
    }

    /// Replaces the baseline water depth buffer from a dense row-major snapshot.
    pub(crate) fn replace_baseline_depth_from_dense(
        &mut self,
        dense: &[f32],
    ) -> Result<(), String> {
        self.baseline.replace_from_dense(dense)?;
        self.mark_all_render_patches_dirty();
        Ok(())
    }

    /// Replaces the dynamic water depth buffer from a dense row-major snapshot.
    pub(crate) fn replace_dynamic_depth_from_dense(&mut self, dense: &[f32]) -> Result<(), String> {
        self.dynamic.depth.replace_from_dense(dense)?;
        self.mark_all_render_patches_dirty();
        Ok(())
    }

    /// Returns a dense row-major snapshot of dynamic water velocity magnitude.
    pub(crate) fn clone_velocity_dense(&self) -> Vec<f32> {
        self.dynamic.velocity.clone_dense()
    }

    /// Returns a dense row-major snapshot of dynamic directional flux values.
    pub(crate) fn clone_flux_dense(&self) -> Vec<[f32; 4]> {
        self.dynamic.flux.clone_dense()
    }

    /// Replaces the dynamic water velocity buffer from a dense row-major snapshot.
    pub(crate) fn replace_velocity_from_dense(&mut self, dense: &[f32]) -> Result<(), String> {
        self.dynamic.velocity.replace_from_dense(dense)?;
        self.mark_all_render_patches_dirty();
        Ok(())
    }

    /// Replaces the dynamic water flux buffer from a dense row-major snapshot.
    pub(crate) fn replace_flux_from_dense(&mut self, dense: &[[f32; 4]]) -> Result<(), String> {
        self.dynamic.flux.replace_from_dense(dense)
    }

    /// Clears all transient dynamic water depth, velocity, and flux while keeping authored sources.
    pub(crate) fn clear_dynamic_state(&mut self) {
        self.dynamic.clear_runtime_state();
        self.mark_all_render_patches_dirty();
    }

    /// Returns a snapshot of all dynamic water boundary points.
    pub(crate) fn clone_sources(&self) -> Vec<(usize, usize, f32)> {
        self.dynamic.sources.clone()
    }

    /// Replaces the current dynamic water boundary point list.
    pub(crate) fn replace_sources(&mut self, sources: Vec<(usize, usize, f32)>) {
        self.dynamic.sources = sources;
    }

    /// Returns the full water-map extent in metres.
    pub(crate) fn world_size(&self) -> (f32, f32) {
        (
            (self.width.saturating_sub(1)) as f32 * self.cell_size,
            (self.height.saturating_sub(1)) as f32 * self.cell_size,
        )
    }

    /// Returns the number of render patches across the X axis.
    pub(crate) fn render_patch_cols(&self) -> usize {
        if self.width <= 1 {
            1
        } else {
            (self.width - 1).div_ceil(self.render_patch_interval_cells)
        }
    }

    /// Returns the number of render patches across the Z axis.
    pub(crate) fn render_patch_rows(&self) -> usize {
        if self.height <= 1 {
            1
        } else {
            (self.height - 1).div_ceil(self.render_patch_interval_cells)
        }
    }

    /// Returns the current set of dirty water render patches.
    pub(crate) fn dirty_render_patches(&self) -> &HashSet<(usize, usize)> {
        &self.dirty_render_patches
    }

    /// Returns the owned sample bounds for one render patch, excluding its render border ring.
    pub(crate) fn render_patch_owned_sample_bounds(
        &self,
        patch_x: usize,
        patch_z: usize,
    ) -> Option<(usize, usize, usize, usize)> {
        self.render_patch_sample_bounds(patch_x, patch_z)
    }

    /// Clears the water render-patch dirtiness set.
    pub(crate) fn clear_dirty_render_patches(&mut self) {
        self.dirty_render_patches.clear();
    }

    /// Returns one visible-water render patch aligned to the terrain render grid.
    pub(crate) fn visible_patch_snapshot(
        &self,
        patch_x: usize,
        patch_z: usize,
    ) -> Option<WaterPatchSnapshot> {
        let (start_x, end_x, start_z, end_z) = self.render_patch_sample_bounds(patch_x, patch_z)?;
        let sample_width = end_x - start_x + 1;
        let sample_height = end_z - start_z + 1;
        let texture_width = sample_width + WATER_RENDER_PATCH_BORDER_TEXELS * 2;
        let texture_height = sample_height + WATER_RENDER_PATCH_BORDER_TEXELS * 2;
        let mut depth_data = vec![0.0_f32; texture_width * texture_height];
        let mut velocity_data = vec![0.0_f32; texture_width * texture_height];

        for local_z in 0..texture_height {
            let sample_z =
                border_clamped_index(start_z, end_z, local_z, WATER_RENDER_PATCH_BORDER_TEXELS);
            for local_x in 0..texture_width {
                let sample_x =
                    border_clamped_index(start_x, end_x, local_x, WATER_RENDER_PATCH_BORDER_TEXELS);
                let flat_idx = local_z * texture_width + local_x;
                depth_data[flat_idx] = self.baseline.depth.get(sample_x, sample_z)
                    + self.dynamic.depth.get(sample_x, sample_z);
                velocity_data[flat_idx] = self.dynamic.velocity.get(sample_x, sample_z);
            }
        }

        let (world_origin_x, world_origin_z) = self.grid_to_world_coords(start_x, start_z);
        let world_size_x = (sample_width.saturating_sub(1)) as f32 * self.cell_size;
        let world_size_z = (sample_height.saturating_sub(1)) as f32 * self.cell_size;
        Some(WaterPatchSnapshot {
            patch_x,
            patch_z,
            sample_width,
            sample_height,
            texture_width,
            texture_height,
            inner_offset_x: WATER_RENDER_PATCH_BORDER_TEXELS,
            inner_offset_z: WATER_RENDER_PATCH_BORDER_TEXELS,
            world_origin_x,
            world_origin_z,
            world_size_x,
            world_size_z,
            depth_data,
            velocity_data,
        })
    }

    /// Returns debug-only baseline/dynamic/combined water stats for one visible render patch.
    pub(crate) fn visible_patch_layer_stats(
        &self,
        patch_x: usize,
        patch_z: usize,
    ) -> Option<WaterPatchLayerStats> {
        let (start_x, end_x, start_z, end_z) = self.render_patch_sample_bounds(patch_x, patch_z)?;
        let sample_width = end_x - start_x + 1;
        let sample_height = end_z - start_z + 1;
        let texture_width = sample_width + WATER_RENDER_PATCH_BORDER_TEXELS * 2;
        let texture_height = sample_height + WATER_RENDER_PATCH_BORDER_TEXELS * 2;

        let mut stats = WaterPatchLayerStats {
            total_samples: texture_width * texture_height,
            baseline_nonzero: 0,
            baseline_max: 0.0,
            baseline_sum: 0.0,
            dynamic_nonzero: 0,
            dynamic_max: 0.0,
            dynamic_sum: 0.0,
            combined_nonzero: 0,
            combined_max: 0.0,
            combined_sum: 0.0,
            velocity_nonzero: 0,
            velocity_max: 0.0,
            velocity_sum: 0.0,
            source_count_in_patch: 0,
            source_rate_sum: 0.0,
            source_rate_abs_sum: 0.0,
            source_count_total: self.dynamic.sources.len(),
        };

        for local_z in 0..texture_height {
            let sample_z =
                border_clamped_index(start_z, end_z, local_z, WATER_RENDER_PATCH_BORDER_TEXELS);
            for local_x in 0..texture_width {
                let sample_x =
                    border_clamped_index(start_x, end_x, local_x, WATER_RENDER_PATCH_BORDER_TEXELS);
                let baseline_depth = self.baseline.depth.get(sample_x, sample_z);
                let dynamic_depth = self.dynamic.depth.get(sample_x, sample_z);
                let combined_depth = baseline_depth + dynamic_depth;
                let velocity = self.dynamic.velocity.get(sample_x, sample_z);

                accumulate_patch_sample(
                    baseline_depth,
                    &mut stats.baseline_nonzero,
                    &mut stats.baseline_max,
                    &mut stats.baseline_sum,
                );
                accumulate_patch_sample(
                    dynamic_depth,
                    &mut stats.dynamic_nonzero,
                    &mut stats.dynamic_max,
                    &mut stats.dynamic_sum,
                );
                accumulate_patch_sample(
                    combined_depth,
                    &mut stats.combined_nonzero,
                    &mut stats.combined_max,
                    &mut stats.combined_sum,
                );
                accumulate_patch_sample(
                    velocity,
                    &mut stats.velocity_nonzero,
                    &mut stats.velocity_max,
                    &mut stats.velocity_sum,
                );
            }
        }

        for &(source_x, source_z, rate) in &self.dynamic.sources {
            if (start_x..=end_x).contains(&source_x) && (start_z..=end_z).contains(&source_z) {
                stats.source_count_in_patch += 1;
                stats.source_rate_sum += rate;
                stats.source_rate_abs_sum += rate.abs();
            }
        }

        Some(stats)
    }

    /// Returns the visible water depth along the world-edge perimeter loop.
    pub(crate) fn border_loop_depths(&self) -> Vec<f32> {
        if self.width < 2 || self.height < 2 {
            return Vec::new();
        }

        let mut depths = Vec::with_capacity(self.width * 2 + self.height * 2 - 4);
        for x in 0..self.width {
            depths.push(self.visible_depth_at(x, 0));
        }
        for z in 1..self.height {
            depths.push(self.visible_depth_at(self.width - 1, z));
        }
        for x in (0..self.width.saturating_sub(1)).rev() {
            depths.push(self.visible_depth_at(x, self.height - 1));
        }
        for z in (1..self.height.saturating_sub(1)).rev() {
            depths.push(self.visible_depth_at(0, z));
        }
        depths
    }

    /// Converts one world-space position to the nearest in-bounds water cell.
    pub(crate) fn world_to_grid_cell_clamped(&self, world_x: f32, world_z: f32) -> (usize, usize) {
        let (world_w, world_h) = self.world_size();
        let half_w = world_w * 0.5;
        let half_h = world_h * 0.5;
        let grid_x = ((world_x + half_w) / self.cell_size)
            .round()
            .clamp(0.0, (self.width.saturating_sub(1)) as f32) as usize;
        let grid_z = ((world_z + half_h) / self.cell_size)
            .round()
            .clamp(0.0, (self.height.saturating_sub(1)) as f32) as usize;
        (grid_x, grid_z)
    }

    fn grid_to_world_coords(&self, grid_x: usize, grid_z: usize) -> (f32, f32) {
        let (world_w, world_h) = self.world_size();
        let half_w = world_w * 0.5;
        let half_h = world_h * 0.5;
        (
            grid_x as f32 * self.cell_size - half_w,
            grid_z as f32 * self.cell_size - half_h,
        )
    }

    fn mark_all_render_patches_dirty(&mut self) {
        for patch_z in 0..self.render_patch_rows() {
            for patch_x in 0..self.render_patch_cols() {
                self.dirty_render_patches.insert((patch_x, patch_z));
            }
        }
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

        let (min_patch_x, max_patch_x) = self.patch_range_for_sample_range(min_x, max_x);
        let (min_patch_z, max_patch_z) = self.patch_range_for_sample_range(min_z, max_z);
        for patch_z in min_patch_z..=max_patch_z {
            for patch_x in min_patch_x..=max_patch_x {
                self.dirty_render_patches.insert((patch_x, patch_z));
            }
        }
    }

    fn patch_range_for_sample_range(&self, min_sample: usize, max_sample: usize) -> (usize, usize) {
        let mut patch_min = min_sample / self.render_patch_interval_cells;
        if min_sample > 0 && min_sample % self.render_patch_interval_cells == 0 {
            patch_min = patch_min.saturating_sub(1);
        }
        let patch_max = max_sample / self.render_patch_interval_cells;
        (
            patch_min.min(self.render_patch_cols().saturating_sub(1)),
            patch_max.min(self.render_patch_cols().saturating_sub(1)),
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

    fn visible_depth_at(&self, grid_x: usize, grid_z: usize) -> f32 {
        self.baseline.depth.get(grid_x, grid_z) + self.dynamic.depth.get(grid_x, grid_z)
    }
}

fn water_chunk_cells_for_config(config: &WorldConfig) -> usize {
    ((config.terrain_chunk_m / config.terrain_cell_m).ceil() as usize).max(1)
}

fn render_patch_interval_cells(cell_size: f32, chunk_span_m: f32) -> usize {
    ((chunk_span_m / cell_size.max(f32::EPSILON)).round() as usize).max(1)
}

fn border_clamped_index(
    start: usize,
    end: usize,
    bordered_index: usize,
    border_texels: usize,
) -> usize {
    let sample_count = end.saturating_sub(start) + 1;
    if bordered_index < border_texels {
        start
    } else if bordered_index >= border_texels + sample_count {
        end
    } else {
        start + bordered_index - border_texels
    }
}

fn accumulate_patch_sample(
    value: f32,
    nonzero_count: &mut usize,
    max_value: &mut f32,
    sum_value: &mut f32,
) {
    if value > WATER_DEBUG_VISIBLE_EPSILON {
        *nonzero_count += 1;
    }
    *max_value = (*max_value).max(value);
    *sum_value += value;
}

#[cfg(test)]
mod tests {
    use super::WaterSystem;
    use std::collections::HashSet;

    #[test]
    fn visible_patch_snapshot_uses_chunk_local_combined_depth() {
        let mut water = WaterSystem::with_chunking(9, 9, 10.0, 4).with_render_chunk_span(30.0);
        let mut baseline = vec![0.0; 81];
        baseline[3 + 3 * 9] = 5.0;
        water
            .replace_baseline_depth_from_dense(&baseline)
            .expect("baseline depth dimensions should match");
        water.add_water(3, 3, 2.0);

        let patch = water
            .visible_patch_snapshot(1, 1)
            .expect("patch (1,1) should exist on a 9x9 water grid");

        assert_eq!(patch.patch_x, 1);
        assert_eq!(patch.patch_z, 1);
        assert_eq!(patch.sample_width, 4);
        assert_eq!(patch.sample_height, 4);
        assert_eq!(patch.texture_width, 6);
        assert_eq!(patch.texture_height, 6);
        assert_eq!(patch.inner_offset_x, 1);
        assert_eq!(patch.inner_offset_z, 1);
        assert!((patch.world_origin_x + 10.0).abs() < 0.0001);
        assert!((patch.world_origin_z + 10.0).abs() < 0.0001);
        assert!((patch.world_size_x - 30.0).abs() < 0.0001);
        assert!((patch.world_size_z - 30.0).abs() < 0.0001);
        assert_eq!(patch.depth_data[0], 7.0);
        assert_eq!(patch.depth_data[patch.texture_width + 1], 7.0);
        assert_eq!(patch.velocity_data.len(), patch.depth_data.len());
    }

    #[test]
    fn visible_patch_layer_stats_split_baseline_dynamic_depth() {
        let mut water = WaterSystem::with_chunking(9, 9, 10.0, 4).with_render_chunk_span(30.0);
        let mut baseline = vec![0.0; 81];
        baseline[3 + 3 * 9] = 5.0;
        water
            .replace_baseline_depth_from_dense(&baseline)
            .expect("baseline depth dimensions should match");
        water.add_water(3, 3, 2.0);
        water.update_source(3, 3, 1.25);

        let stats = water
            .visible_patch_layer_stats(1, 1)
            .expect("patch (1,1) should exist on a 9x9 water grid");

        assert_eq!(stats.total_samples, 36);
        assert_eq!(stats.baseline_nonzero, 4);
        assert!((stats.baseline_max - 5.0).abs() < 0.0001);
        assert!((stats.baseline_sum - 20.0).abs() < 0.0001);
        assert_eq!(stats.dynamic_nonzero, 4);
        assert!((stats.dynamic_max - 2.0).abs() < 0.0001);
        assert!((stats.dynamic_sum - 8.0).abs() < 0.0001);
        assert_eq!(stats.combined_nonzero, 4);
        assert!((stats.combined_max - 7.0).abs() < 0.0001);
        assert!((stats.combined_sum - 28.0).abs() < 0.0001);
        assert_eq!(stats.source_count_in_patch, 1);
        assert_eq!(stats.source_count_total, 1);
        assert!((stats.source_rate_sum - 1.25).abs() < 0.0001);
        assert!((stats.source_rate_abs_sum - 1.25).abs() < 0.0001);
    }

    #[test]
    fn tick_does_not_panic_when_water_reaches_boundary_rows() {
        let mut water = WaterSystem::with_chunking(5, 5, 10.0, 4);
        water.update_source(2, 0, 1.0);
        let terrain = vec![0.0; 25];

        water.tick(&terrain, 0.25);

        assert_eq!(water.clone_depth_dense().len(), 25);
        assert_eq!(water.clone_velocity_dense().len(), 25);
    }

    #[test]
    fn point_water_edit_marks_all_overlapping_render_patches() {
        let mut water = WaterSystem::with_chunking(17, 17, 10.0, 4).with_render_chunk_span(30.0);

        water.add_water(3, 3, 1.0);

        let dirty: HashSet<(usize, usize)> = water.dirty_render_patches().iter().copied().collect();
        let expected = HashSet::from([(0, 0), (1, 0), (0, 1), (1, 1)]);
        assert_eq!(dirty, expected);
    }
}
