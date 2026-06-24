//! Baseline water runtime for authored still-water bodies.
//!
//! Water is authored as deterministic lake and open-water fills. The runtime
//! stores only the visible baseline depth above terrain and exposes render-patch
//! snapshots for Godot. Any future river or flow simulation should be designed as
//! a new system rather than extending this baseline renderer with legacy solver
//! state.

use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::sparse_chunk_grid::SparseChunkGrid;
use std::collections::HashSet;

const DEFAULT_WATER_CHUNK_CELLS: usize = 64;
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

/// Water runtime for one world.
///
/// The runtime stores flat authored still-water bodies as baseline depth above
/// terrain. Rendering reads this depth directly; no source/sink, velocity, or
/// flux state is maintained.
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
}
