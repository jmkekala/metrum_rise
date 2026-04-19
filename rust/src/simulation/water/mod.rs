//! Shallow-water simulation using the Saint-Venant equations.
//!
//! Water depth and flux are stored on the same grid as the terrain heightmap.
//! The tick is parallelised with `rayon` over grid rows.
//! Water boundary points (player-placed) inject or remove depth at a fixed rate.

use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::sparse_chunk_grid::SparseChunkGrid;
use rayon::prelude::*;

const DEFAULT_WATER_CHUNK_CELLS: usize = 64;

/// Shallow-water state for the entire map.
pub struct WaterSystem {
    /// Grid width in cells (matches terrain width).
    pub width: usize,
    /// Grid height in cells (matches terrain height).
    pub height: usize,
    /// Water sample spacing in metres.
    cell_size: f32,
    /// Water depth (metres) per cell.
    depth: SparseChunkGrid<f32>,
    /// Flow velocity magnitude per cell, used for rendering foam/current effects.
    velocity: SparseChunkGrid<f32>,
    /// Directional flux per cell: `[Left, Right, Top, Bottom]` (m³/s).
    flux: SparseChunkGrid<[f32; 4]>,
    /// Player-placed water boundary points: `(grid_x, grid_y, signed_rate)`.
    /// Positive values add water; negative values remove water.
    pub sources: Vec<(usize, usize, f32)>,
}

impl WaterSystem {
    /// Creates a new, dry water system of the given dimensions.
    pub fn new(width: usize, height: usize) -> Self {
        Self::with_chunking(width, height, 1.0, DEFAULT_WATER_CHUNK_CELLS)
    }

    /// Creates a new water system with an explicit sparse chunk size.
    pub fn with_chunking(width: usize, height: usize, cell_size: f32, chunk_size: usize) -> Self {
        Self {
            width,
            height,
            cell_size: cell_size.max(f32::EPSILON),
            depth: SparseChunkGrid::new(width, height, chunk_size.max(1), 0.0),
            velocity: SparseChunkGrid::new(width, height, chunk_size.max(1), 0.0),
            flux: SparseChunkGrid::new(width, height, chunk_size.max(1), [0.0; 4]),
            sources: Vec::new(),
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
    }

    /// Advances the water simulation by `dt` seconds.
    ///
    /// Performs three parallel passes:
    /// 1. Flux calculation (Saint-Venant momentum)
    /// 2. Depth update (Saint-Venant mass conservation)
    /// 3. Velocity magnitude calculation for rendering
    pub fn tick(&mut self, terrain: &[f32], dt: f32) {
        if terrain.len() != self.width * self.height || self.width < 3 || self.height < 3 {
            return;
        }

        let mut depth = self.depth.clone_dense();
        let mut velocity = self.velocity.clone_dense();
        let mut flux = self.flux.clone_dense();

        // 0. Apply water boundary points (sequential but small count).
        for &(x, y, rate) in &self.sources {
            let idx = y * self.width + x;
            depth[idx] = (depth[idx] + rate * dt).max(0.0);
        }

        let l = 1.0; // Pipe length
        let a = 1.0; // Pipe area
        let g = 9.81;
        let w = self.width;
        let h = self.height;

        // --- 1. Calculate flux (Parallelized rows) ---
        // Pre-cloning or sharing depth/terrain for immutable read
        let depth_ref = &depth;
        let terrain_ref = terrain;

        flux.par_chunks_mut(w)
            .enumerate()
            .for_each(|(y, row_flux)| {
                if y == 0 || y >= h - 1 {
                    return;
                }

                for x in 1..w - 1 {
                    let idx = y * w + x;

                    // SKIPPING LOGIC: If cell is dry and has no existing flux, skip.
                    // Note: We check if it HAS flux because even if depth is 0,
                    // water might be leaving due to momentum (flux > 0).
                    if depth_ref[idx] <= 1e-6 && row_flux[x].iter().all(|&f| f <= 0.0) {
                        // Check neighbors to see if water might flow IN
                        let n1 = (y - 1) * w + x;
                        let n2 = (y + 1) * w + x;
                        if depth_ref[idx - 1] <= 1e-6
                            && depth_ref[idx + 1] <= 1e-6
                            && depth_ref[n1] <= 1e-6
                            && depth_ref[n2] <= 1e-6
                        {
                            continue;
                        }
                    }

                    let h_self = terrain_ref[idx] + depth_ref[idx];
                    let mut f = row_flux[x];

                    // Neighbors: [Left, Right, Top, Bottom]
                    let nx = [x - 1, x + 1, x, x];
                    let ny = [y, y, y - 1, y + 1];

                    for i in 0..4 {
                        let n_idx = ny[i] * w + nx[i];
                        let h_neighbor = terrain_ref[n_idx] + depth_ref[n_idx];
                        let h_diff = h_self - h_neighbor;
                        f[i] = (f[i] + dt * g * a * (h_diff / l)).max(0.0);
                    }

                    // Scale flux to prevent negative depth
                    let total_flux = f[0] + f[1] + f[2] + f[3];
                    if total_flux > 0.0 {
                        let k = (depth_ref[idx] * l * l / (total_flux * dt)).min(1.0);
                        for i in 0..4 {
                            f[i] *= k;
                        }
                    }

                    row_flux[x] = f;
                }
            });

        // --- 2. Update depth (Parallelized rows) ---
        // Capture a read-only view of flux
        let flux_ref = &flux;

        depth
            .par_chunks_mut(w)
            .enumerate()
            .enumerate()
            .for_each(|(_y_idx, (y, row_depth))| {
                if y == 0 || y >= h - 1 {
                    return;
                }

                // Using the velocity buffer to also find active rows
                let mut row_vel = vec![0.0; w]; // Temporary for this row, will write to self.velocity later

                for x in 1..w - 1 {
                    let idx = y * w + x;

                    let fin = flux_ref[idx - 1][1] // From left
                        + flux_ref[idx + 1][0] // From right
                        + flux_ref[idx - w][3] // From top
                        + flux_ref[idx + w][2]; // From bottom

                    let fout =
                        flux_ref[idx][0] + flux_ref[idx][1] + flux_ref[idx][2] + flux_ref[idx][3];

                    if fin <= 1e-8 && fout <= 1e-8 && row_depth[x] <= 1e-6 {
                        continue; // Skip dry land updates
                    }

                    // Update depth
                    row_depth[x] += dt * (fin - fout) / (l * l);
                    if row_depth[x] < 0.0001 {
                        row_depth[x] = 0.0;
                    }

                    // Calculate velocity magnitude (speed)
                    if row_depth[x] > 0.001 {
                        row_vel[x] = (fin + fout) / (2.0 * row_depth[x] * l);
                    } else {
                        row_vel[x] = 0.0;
                    }
                }

                // Note: We need to write row_vel back to self.velocity
                // But self.velocity is currently being borrowed by tick.
                // We'll do a separate pass for it or use unsafe (not recommended).
                // Actually, we can just process velocity in a separate par_iter.
            });

        // Pass 3: Velocity (only for active cells)
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
            .expect("dense water depth snapshot must match water grid dimensions");
        self.velocity
            .replace_from_dense(&velocity)
            .expect("dense water velocity snapshot must match water grid dimensions");
        self.flux
            .replace_from_dense(&flux)
            .expect("dense water flux snapshot must match water grid dimensions");
    }

    /// Adds a discrete amount of water depth to a specific grid cell.
    pub fn add_water(&mut self, x: usize, y: usize, amount: f32) {
        if x < self.width && y < self.height {
            let next_depth = self.depth.get(x, y) + amount;
            self.depth.set(x, y, next_depth);
        }
    }

    /// Updates or adds a water boundary point at a specific grid cell.
    pub fn update_source(&mut self, x: usize, y: usize, rate_add: f32) {
        if x >= self.width || y >= self.height {
            return;
        }

        if let Some(source) = self.sources.iter_mut().find(|s| s.0 == x && s.1 == y) {
            source.2 += rate_add;
        } else {
            self.sources.push((x, y, rate_add));
        }
    }

    /// Returns a dense row-major snapshot of water depth.
    pub(crate) fn clone_depth_dense(&self) -> Vec<f32> {
        self.depth.clone_dense()
    }

    /// Returns a dense row-major snapshot of water velocity magnitude.
    pub(crate) fn clone_velocity_dense(&self) -> Vec<f32> {
        self.velocity.clone_dense()
    }

    /// Returns a dense row-major snapshot of directional flux values.
    pub(crate) fn clone_flux_dense(&self) -> Vec<[f32; 4]> {
        self.flux.clone_dense()
    }

    /// Replaces the water depth buffer from a dense row-major snapshot.
    pub(crate) fn replace_depth_from_dense(&mut self, dense: &[f32]) -> Result<(), String> {
        self.depth.replace_from_dense(dense)
    }

    /// Replaces the water velocity buffer from a dense row-major snapshot.
    pub(crate) fn replace_velocity_from_dense(&mut self, dense: &[f32]) -> Result<(), String> {
        self.velocity.replace_from_dense(dense)
    }

    /// Replaces the water flux buffer from a dense row-major snapshot.
    pub(crate) fn replace_flux_from_dense(&mut self, dense: &[[f32; 4]]) -> Result<(), String> {
        self.flux.replace_from_dense(dense)
    }

    /// Returns the full water-map extent in metres.
    pub(crate) fn world_size(&self) -> (f32, f32) {
        (
            (self.width.saturating_sub(1)) as f32 * self.cell_size,
            (self.height.saturating_sub(1)) as f32 * self.cell_size,
        )
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
}

fn water_chunk_cells_for_config(config: &WorldConfig) -> usize {
    ((config.terrain_chunk_m / config.terrain_cell_m).ceil() as usize).max(1)
}

#[cfg(test)]
mod tests {
    use super::WaterSystem;

    #[test]
    fn tick_does_not_panic_when_water_reaches_boundary_rows() {
        let mut water = WaterSystem::with_chunking(5, 5, 10.0, 4);
        water.update_source(2, 0, 1.0);
        let terrain = vec![0.0; 25];

        water.tick(&terrain, 0.25);

        assert_eq!(water.clone_depth_dense().len(), 25);
        assert_eq!(water.clone_velocity_dense().len(), 25);
    }
}
