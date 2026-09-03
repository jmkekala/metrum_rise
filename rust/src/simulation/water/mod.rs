// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: mod.rs
//  script_path: rust/src/simulation/water/mod.rs
//  module_name: water
//  version: 0.2.0
//  author: [BantedHam]
//  description: Water runtime: flat authored still-water bodies as
//           baseline depth above terrain. Patch snapshots carry the sim
//           ground testimony on their own samples, so the shell can
//           composite sculpts and earthworks into the drawn shoreline.
//  kind: module
//  spec: none
//  internal_dependencies: [config, sparse_chunk_grid, terrain]
//  external_dependencies: [godot-rust]
//  features: [baseline-depth, render-patches, ground-testimony]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-09-02
// ========================================================================

//! Baseline water runtime for authored still-water bodies.
//!
//! Water is authored as deterministic lake and open-water fills. The runtime
//! stores only the visible baseline depth above terrain and exposes render-patch
//! snapshots for Godot. Any future river or flow simulation should be designed as
//! a new system rather than extending this baseline renderer with legacy solver
//! state.

use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::sparse_chunk_grid::SparseChunkGrid;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const DEFAULT_WATER_CHUNK_CELLS: usize = 64;
const WATER_RENDER_PATCH_BORDER_TEXELS: usize = 1;
const WATER_DEBUG_VISIBLE_EPSILON: f32 = 0.001;

/// One deterministic render-patch snapshot of visible water.
#[derive(Clone, Debug, PartialEq)]
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
    /// Row-major sim terrain heights on the same samples, the ground
    /// testimony a shoreline needs to follow sculpts and earthworks.
    pub ground_data: Vec<f32>,
    /// Number of texture samples with visible water depth.
    pub depth_nonzero_count: usize,
}

/// Debug-only baseline/visible split for one visible water render patch.
pub(crate) struct WaterPatchLayerStats {
    /// Number of row-major samples in the patch texture including the border ring.
    pub(crate) total_samples: usize,
    /// Number of samples whose authored baseline depth is visibly non-zero.
    pub(crate) baseline_nonzero: usize,
    /// Maximum authored baseline depth in metres.
    pub(crate) baseline_max: f32,
    /// Sum of authored baseline depth samples in metres.
    pub(crate) baseline_sum: f32,
    /// Number of samples whose final visible depth is visibly non-zero.
    pub(crate) visible_nonzero: usize,
    /// Maximum final visible depth in metres.
    pub(crate) visible_max: f32,
    /// Sum of final visible depth samples in metres.
    pub(crate) visible_sum: f32,
}

#[derive(Clone)]
struct BaselineWaterState {
    depth: Arc<SparseChunkGrid<f32>>,
}

impl BaselineWaterState {
    fn new(width: usize, height: usize, chunk_size: usize) -> Self {
        Self {
            depth: Arc::new(SparseChunkGrid::new(width, height, chunk_size.max(1), 0.0)),
        }
    }

    fn clone_dense(&self) -> Vec<f32> {
        self.depth.clone_dense()
    }

    fn replace_from_dense(&mut self, dense: &[f32]) -> Result<(), String> {
        Arc::make_mut(&mut self.depth).replace_from_dense(dense)
    }
}

/// Water runtime for one world.
///
/// The runtime stores flat authored still-water bodies as baseline depth above
/// terrain. Rendering reads this depth directly; no source/sink, velocity, or
/// flux state is maintained.
#[derive(Clone)]
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
    /// Render patches whose visible water textures must be refreshed.
    dirty_render_patches: HashSet<(usize, usize)>,
    /// Monotonic allocator for render-patch source revisions.
    render_generation_counter: u64,
    /// Latest source revision for each water render patch.
    render_patch_generations: HashMap<(usize, usize), u64>,
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
            dirty_render_patches: HashSet::new(),
            render_generation_counter: 0,
            render_patch_generations: HashMap::new(),
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
        .with_render_chunk_span(config.terrain_render_chunk_span_m())
    }

    fn with_render_chunk_span(mut self, chunk_span_m: f32) -> Self {
        self.chunk_span_m = chunk_span_m.max(self.cell_size);
        self.render_patch_interval_cells =
            render_patch_interval_cells(self.cell_size, self.chunk_span_m);
        self
    }

    /// Returns a dense row-major snapshot of baseline water depth above terrain.
    pub(crate) fn clone_baseline_depth_dense(&self) -> Vec<f32> {
        self.baseline.clone_dense()
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

    /// Returns the current source revision for one water render patch.
    pub(crate) fn render_patch_generation(&self, patch_x: usize, patch_z: usize) -> u64 {
        self.render_patch_generations
            .get(&(patch_x, patch_z))
            .copied()
            .unwrap_or(0)
    }

    /// Returns the current global water payload revision.
    pub(crate) fn render_generation(&self) -> u64 {
        self.render_generation_counter
    }

    /// Returns sorted dirty patch keys paired with their source revisions.
    pub(crate) fn dirty_render_patch_states(&self) -> Vec<(usize, usize, u64)> {
        let mut states = self
            .dirty_render_patches
            .iter()
            .map(|&(patch_x, patch_z)| {
                (
                    patch_x,
                    patch_z,
                    self.render_patch_generation(patch_x, patch_z),
                )
            })
            .collect::<Vec<_>>();
        states.sort_unstable();
        states
    }

    /// Returns the owned sample bounds for one render patch, excluding its render border ring.
    pub(crate) fn render_patch_owned_sample_bounds(
        &self,
        patch_x: usize,
        patch_z: usize,
    ) -> Option<(usize, usize, usize, usize)> {
        self.render_patch_sample_bounds(patch_x, patch_z)
    }

    /// Acknowledges one rendered patch only when the renderer consumed its current revision.
    pub(crate) fn acknowledge_render_patch(
        &mut self,
        patch_x: usize,
        patch_z: usize,
        generation: u64,
    ) -> bool {
        if self.render_patch_generation(patch_x, patch_z) != generation {
            return false;
        }
        self.dirty_render_patches.remove(&(patch_x, patch_z))
    }

    /// Returns one visible-water render patch aligned to the terrain render
    /// grid. The ground closure answers the sim terrain height at a sample,
    /// so the snapshot carries the ground testimony on the same indices.
    pub(crate) fn visible_patch_snapshot(
        &self,
        patch_x: usize,
        patch_z: usize,
        ground: &dyn Fn(usize, usize) -> f32,
    ) -> Option<WaterPatchSnapshot> {
        let (start_x, end_x, start_z, end_z) = self.render_patch_sample_bounds(patch_x, patch_z)?;
        let sample_width = end_x - start_x + 1;
        let sample_height = end_z - start_z + 1;
        let texture_width = sample_width + WATER_RENDER_PATCH_BORDER_TEXELS * 2;
        let texture_height = sample_height + WATER_RENDER_PATCH_BORDER_TEXELS * 2;
        let mut depth_data = vec![0.0_f32; texture_width * texture_height];
        let mut ground_data = vec![0.0_f32; texture_width * texture_height];
        let mut depth_nonzero_count = 0;

        for local_z in 0..texture_height {
            let sample_z =
                border_clamped_index(start_z, end_z, local_z, WATER_RENDER_PATCH_BORDER_TEXELS);
            for local_x in 0..texture_width {
                let sample_x =
                    border_clamped_index(start_x, end_x, local_x, WATER_RENDER_PATCH_BORDER_TEXELS);
                let flat_idx = local_z * texture_width + local_x;
                let depth = self.visible_depth_at(sample_x, sample_z);
                if depth > WATER_DEBUG_VISIBLE_EPSILON {
                    depth_nonzero_count += 1;
                }
                depth_data[flat_idx] = depth;
                ground_data[flat_idx] = ground(sample_x, sample_z);
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
            ground_data,
            depth_nonzero_count,
        })
    }

    /// Returns debug-only baseline/visible water stats for one visible render patch.
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
            visible_nonzero: 0,
            visible_max: 0.0,
            visible_sum: 0.0,
        };

        for local_z in 0..texture_height {
            let sample_z =
                border_clamped_index(start_z, end_z, local_z, WATER_RENDER_PATCH_BORDER_TEXELS);
            for local_x in 0..texture_width {
                let sample_x =
                    border_clamped_index(start_x, end_x, local_x, WATER_RENDER_PATCH_BORDER_TEXELS);
                let baseline_depth = self.baseline.depth.get(sample_x, sample_z);
                let visible_depth = baseline_depth;

                accumulate_patch_sample(
                    baseline_depth,
                    &mut stats.baseline_nonzero,
                    &mut stats.baseline_max,
                    &mut stats.baseline_sum,
                );
                accumulate_patch_sample(
                    visible_depth,
                    &mut stats.visible_nonzero,
                    &mut stats.visible_max,
                    &mut stats.visible_sum,
                );
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

    /// Returns whether a sampled roadbed corridor touches visible authored water.
    ///
    /// Sampling uses a fixed quarter-cell lattice over the candidate footprint. Complexity is
    /// `O(ceil(length / step) * ceil(width / step))` with no allocation, where `step` is bounded
    /// to `0.5..=3.0 m`; the query exits on the first visible-water sample.
    pub(crate) fn road_corridor_overlaps_visible_water(
        &self,
        points: &[godot::prelude::Vector3],
        half_width_m: f32,
    ) -> bool {
        if points.len() < 2 || self.width == 0 || self.height == 0 {
            return false;
        }

        let sample_step_m = (self.cell_size * 0.25).clamp(0.5, 3.0);
        let half_width_m = half_width_m.max(0.0);
        let lateral_steps = ((half_width_m * 2.0) / sample_step_m).ceil().max(1.0) as usize;

        for pair in points.windows(2) {
            let dx = pair[1].x - pair[0].x;
            let dz = pair[1].z - pair[0].z;
            let length_m = dx.hypot(dz);
            if length_m <= f32::EPSILON {
                continue;
            }
            let lateral_x = -dz / length_m;
            let lateral_z = dx / length_m;
            let longitudinal_steps = (length_m / sample_step_m).ceil().max(1.0) as usize;

            for along_idx in 0..=longitudinal_steps {
                let along_t = along_idx as f32 / longitudinal_steps as f32;
                let center_x = pair[0].x + dx * along_t;
                let center_z = pair[0].z + dz * along_t;
                for lateral_idx in 0..=lateral_steps {
                    let lateral_t = lateral_idx as f32 / lateral_steps as f32;
                    let offset_m = -half_width_m + half_width_m * 2.0 * lateral_t;
                    if self.visible_depth_world(
                        center_x + lateral_x * offset_m,
                        center_z + lateral_z * offset_m,
                    ) > WATER_DEBUG_VISIBLE_EPSILON
                    {
                        return true;
                    }
                }
            }
        }
        false
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
        self.render_generation_counter = self.render_generation_counter.wrapping_add(1).max(1);
        let generation = self.render_generation_counter;
        for patch_z in 0..self.render_patch_rows() {
            for patch_x in 0..self.render_patch_cols() {
                self.dirty_render_patches.insert((patch_x, patch_z));
                self.render_patch_generations
                    .insert((patch_x, patch_z), generation);
            }
        }
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
        self.baseline.depth.get(grid_x, grid_z)
    }

    fn visible_depth_world(&self, world_x: f32, world_z: f32) -> f32 {
        let (world_w, world_h) = self.world_size();
        let grid_x = (world_x + world_w * 0.5) / self.cell_size;
        let grid_z = (world_z + world_h * 0.5) / self.cell_size;
        let max_x = self.width.saturating_sub(1) as f32;
        let max_z = self.height.saturating_sub(1) as f32;
        if grid_x < 0.0 || grid_z < 0.0 || grid_x > max_x || grid_z > max_z {
            return 0.0;
        }

        let x0 = grid_x.floor() as usize;
        let z0 = grid_z.floor() as usize;
        let x1 = (x0 + 1).min(self.width.saturating_sub(1));
        let z1 = (z0 + 1).min(self.height.saturating_sub(1));
        let tx = grid_x - x0 as f32;
        let tz = grid_z - z0 as f32;
        let depth_00 = self.visible_depth_at(x0, z0);
        let depth_10 = self.visible_depth_at(x1, z0);
        let depth_01 = self.visible_depth_at(x0, z1);
        let depth_11 = self.visible_depth_at(x1, z1);
        let top = depth_00 + (depth_10 - depth_00) * tx;
        let bottom = depth_01 + (depth_11 - depth_01) * tx;
        top + (bottom - top) * tz
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
    fn visible_patch_snapshot_uses_chunk_local_baseline_depth() {
        let mut water = WaterSystem::with_chunking(9, 9, 10.0, 4).with_render_chunk_span(30.0);
        let mut baseline = vec![0.0; 81];
        baseline[3 + 3 * 9] = 5.0;
        water
            .replace_baseline_depth_from_dense(&baseline)
            .expect("baseline depth dimensions should match");

        let patch = water
            .visible_patch_snapshot(1, 1, &|_, _| 0.0)
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
        assert_eq!(patch.depth_data[0], 5.0);
        assert_eq!(patch.depth_data[patch.texture_width + 1], 5.0);
        assert_eq!(patch.depth_nonzero_count, 4);
    }

    #[test]
    fn visible_patch_layer_stats_reports_baseline_depth() {
        let mut water = WaterSystem::with_chunking(9, 9, 10.0, 4).with_render_chunk_span(30.0);
        let mut baseline = vec![0.0; 81];
        baseline[3 + 3 * 9] = 5.0;
        water
            .replace_baseline_depth_from_dense(&baseline)
            .expect("baseline depth dimensions should match");

        let stats = water
            .visible_patch_layer_stats(1, 1)
            .expect("patch (1,1) should exist on a 9x9 water grid");

        assert_eq!(stats.total_samples, 36);
        assert_eq!(stats.baseline_nonzero, 4);
        assert!((stats.baseline_max - 5.0).abs() < 0.0001);
        assert!((stats.baseline_sum - 20.0).abs() < 0.0001);
        assert_eq!(stats.visible_nonzero, 4);
        assert!((stats.visible_max - 5.0).abs() < 0.0001);
        assert!((stats.visible_sum - 20.0).abs() < 0.0001);
    }

    #[test]
    fn replacing_baseline_depth_marks_all_render_patches_dirty() {
        let mut water = WaterSystem::with_chunking(17, 17, 10.0, 4).with_render_chunk_span(30.0);
        let mut baseline = vec![0.0; 17 * 17];
        baseline[3 + 3 * 17] = 1.0;

        water
            .replace_baseline_depth_from_dense(&baseline)
            .expect("baseline depth dimensions should match");

        let dirty: HashSet<(usize, usize)> = water.dirty_render_patches().iter().copied().collect();
        assert_eq!(
            dirty.len(),
            water.render_patch_cols() * water.render_patch_rows()
        );
    }

    #[test]
    fn stale_acknowledgement_preserves_newer_water_patch_dirtiness() {
        let mut water = WaterSystem::with_chunking(9, 9, 10.0, 4).with_render_chunk_span(30.0);
        let baseline = vec![1.0; 81];
        water
            .replace_baseline_depth_from_dense(&baseline)
            .expect("baseline depth dimensions should match");
        let stale_generation = water.render_patch_generation(0, 0);

        water
            .replace_baseline_depth_from_dense(&baseline)
            .expect("second baseline update should advance the revision");
        assert!(!water.acknowledge_render_patch(0, 0, stale_generation));
        assert!(water.dirty_render_patches().contains(&(0, 0)));

        let current_generation = water.render_patch_generation(0, 0);
        assert!(water.acknowledge_render_patch(0, 0, current_generation));
        assert!(!water.dirty_render_patches().contains(&(0, 0)));
    }
}
