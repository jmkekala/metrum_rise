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

        // Linear search for entry/exit or surface intersection
        let mut t = 0.0;
        let max_dist = 10000.0; // Increased from 500 to support high-altitude camera
        let step = 0.5; // Half-meter steps for safety

        let mut prev_diff = ray_origin.y - self.sample_height_world(ray_origin.x, ray_origin.z) * 20.0;

        while t < max_dist {
            t += step;
            let p = ray_origin + ray_dir * t;

            // Bounds check
            if p.x < -half_w || p.x > half_w || p.z < -half_h || p.z > half_h {
                if t > 0.0 && p.y < -10.0 {
                    break;
                } // Went under map
                continue;
            }

            let h = self.sample_height_world(p.x, p.z) * 20.0;
            let diff = p.y - h;

            // Intersection detected (crossed the surface)
            if diff.signum() != prev_diff.signum() {
                // Binary search refinement for precision
                let mut t_low = t - step;
                let mut t_high = t;
                for _ in 0..8 {
                    let t_mid = (t_low + t_high) * 0.5;
                    let pm = ray_origin + ray_dir * t_mid;
                    let hm = self.sample_height_world(pm.x, pm.z) * 20.0;
                    if (pm.y - hm).signum() == prev_diff.signum() {
                        t_low = t_mid;
                    } else {
                        t_high = t_mid;
                    }
                }

                let final_p = ray_origin + ray_dir * ((t_low + t_high) * 0.5);
                return Some(final_p);
            }

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
}

fn terrain_chunk_cells_for_config(config: &WorldConfig) -> usize {
    ((config.terrain_chunk_m / config.terrain_cell_m).ceil() as usize).max(1)
}
