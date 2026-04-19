//! Heightmap terrain system used for road grade, raycasting, and rendering.
//!
//! Two height arrays are maintained: `source_data` (user-sculpted, never modified by roads)
//! and `data` (final map with road beds stamped in). Road snapping and cost calculations
//! always read from `source_data` to avoid feedback loops.

pub mod chunks;

pub use chunks::{
    TerrainChunkAsset, TerrainChunkLoadError, TerrainChunkLodAsset, TerrainChunkLodManifest,
    TerrainChunkManifest, TerrainChunkManifestError,
};

use godot::prelude::Vector3;

use crate::config::HEIGHT_SCALE;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::sparse_chunk_grid::SparseChunkGrid;

const DEFAULT_TERRAIN_CHUNK_CELLS: usize = 64;

/// Dual-buffer heightmap for the terrain surface.
pub struct TerrainSystem {
    /// Map width in height samples.
    pub width: usize,
    /// Map height (depth) in height samples.
    pub height: usize,
    /// Terrain sample spacing in metres.
    cell_size: f32,
    /// Final/visual heightmap (metres). Road-bed depressions are baked into this buffer.
    data: SparseChunkGrid<f32>,
    /// Source heightmap as sculpted by the player, without road modifications.
    /// Used for road grade calculation and slope cost — never written by road placement.
    source_data: SparseChunkGrid<f32>,
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
        Self {
            width,
            height,
            cell_size: cell_size.max(f32::EPSILON),
            data: SparseChunkGrid::new(width, height, chunk_size.max(1), base_elevation),
            source_data: SparseChunkGrid::new(width, height, chunk_size.max(1), base_elevation),
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
    }

    /// Sets the height at a specific grid coordinate.
    ///
    /// Updates both source and visual buffers.
    pub fn set_height(&mut self, x: usize, y: usize, value: f32) {
        if x < self.width && y < self.height {
            self.source_data.set(x, y, value);
            self.data.set(x, y, value);
        }
    }

    /// Gets the raw source height at a grid coordinate.
    pub fn get_height(&self, x: usize, y: usize) -> f32 {
        self.source_data.get(x, y)
    }

    /// Bilinearly interpolates the source height at any fractional world coordinate.
    pub fn get_height_interpolated(&self, x: f32, z: f32) -> f32 {
        let x_clamped = x.clamp(0.0, (self.width - 1) as f32);
        let z_clamped = z.clamp(0.0, (self.height - 1) as f32);

        let x0 = x_clamped.floor() as usize;
        let z0 = z_clamped.floor() as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let z1 = (z0 + 1).min(self.height - 1);

        let fx = x_clamped.fract();
        let fz = z_clamped.fract();

        // Sample from SOURCE data for all interpolation
        let h00 = self.source_data.get(x0, z0);
        let h10 = self.source_data.get(x1, z0);
        let h01 = self.source_data.get(x0, z1);
        let h11 = self.source_data.get(x1, z1);

        let h0 = h00 * (1.0 - fx) + h10 * fx;
        let h1 = h01 * (1.0 - fx) + h11 * fx;

        h0 * (1.0 - fz) + h1 * fz
    }

    /// Samples the authoritative source terrain at one world-space position.
    pub fn sample_height_world(&self, world_x: f32, world_z: f32) -> f32 {
        let (grid_x, grid_z) = self.world_to_grid_coords(world_x, world_z);
        self.get_height_interpolated(grid_x, grid_z)
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

    /// Casts a ray against the terrain surface and returns the intersection point.
    pub fn raycast_terrain(&self, ray_origin: Vector3, ray_dir: Vector3) -> Option<Vector3> {
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
        let mut prev_diff =
            prev_point.y - self.sample_height_world(prev_point.x, prev_point.z) * HEIGHT_SCALE;

        while prev_t < max_t {
            let t = (prev_t + step).min(max_t);
            let p = ray_origin + ray_dir * t;
            let h = self.sample_height_world(p.x, p.z) * HEIGHT_SCALE;
            let diff = p.y - h;

            if diff == 0.0 || diff.signum() != prev_diff.signum() {
                // Binary search refinement for precision.
                let mut t_low = prev_t;
                let mut t_high = t;
                for _ in 0..8 {
                    let t_mid = (t_low + t_high) * 0.5;
                    let pm = ray_origin + ray_dir * t_mid;
                    let hm = self.sample_height_world(pm.x, pm.z) * HEIGHT_SCALE;
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
        self.data.replace_from_dense(dense)
    }

    /// Replaces the authoritative source terrain buffer from a dense row-major snapshot.
    pub(crate) fn replace_source_from_dense(&mut self, dense: &[f32]) -> Result<(), String> {
        self.source_data.replace_from_dense(dense)
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

    /// Returns the terrain sample spacing in metres.
    pub(crate) fn cell_size_m(&self) -> f32 {
        self.cell_size
    }

    fn raycast_search_limit(&self, ray_origin: Vector3, half_w: f32, half_h: f32) -> f32 {
        let world_diag = (half_w * 2.0).hypot(half_h * 2.0);
        ray_origin.length() + world_diag + 10_000.0
    }
}

fn terrain_chunk_cells_for_config(config: &WorldConfig) -> usize {
    ((config.terrain_chunk_m / config.terrain_cell_m).ceil() as usize).max(1)
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
}
