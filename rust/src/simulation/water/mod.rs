//! Water runtime split into authored baseline water plus dynamic flowing water.
//!
//! Baseline water owns flat hydrostatic still-water bodies authored by the world
//! editor. Dynamic water owns transient additional depth, velocity, and flux on
//! top of either dry terrain or one of those baseline water surfaces.

use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::sparse_chunk_grid::SparseChunkGrid;
use rayon::prelude::*;

const DEFAULT_WATER_CHUNK_CELLS: usize = 64;
const MAX_DYNAMIC_WATER_SUBSTEP_DT: f32 = 0.05;

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
    baseline: BaselineWaterState,
    dynamic: DynamicWaterState,
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
            baseline: BaselineWaterState::new(width, height, chunk_size),
            dynamic: DynamicWaterState::new(width, height, chunk_size),
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
    }

    /// Adds a discrete amount of dynamic water depth to a specific grid cell.
    pub fn add_water(&mut self, x: usize, y: usize, amount: f32) {
        if x < self.width && y < self.height {
            let mut dynamic_depth = self.dynamic.depth.get(x, y) + amount;
            if dynamic_depth < 0.0 {
                dynamic_depth = 0.0;
            }
            self.dynamic.depth.set(x, y, dynamic_depth);
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
        self.baseline.replace_from_dense(dense)
    }

    /// Replaces the dynamic water depth buffer from a dense row-major snapshot.
    pub(crate) fn replace_dynamic_depth_from_dense(&mut self, dense: &[f32]) -> Result<(), String> {
        self.dynamic.depth.replace_from_dense(dense)
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
        self.dynamic.velocity.replace_from_dense(dense)
    }

    /// Replaces the dynamic water flux buffer from a dense row-major snapshot.
    pub(crate) fn replace_flux_from_dense(&mut self, dense: &[[f32; 4]]) -> Result<(), String> {
        self.dynamic.flux.replace_from_dense(dense)
    }

    /// Clears all transient dynamic water depth, velocity, and flux while keeping authored sources.
    pub(crate) fn clear_dynamic_state(&mut self) {
        self.dynamic.clear_runtime_state();
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
    fn combined_depth_adds_baseline_and_dynamic_layers() {
        let mut water = WaterSystem::with_chunking(3, 3, 10.0, 4);
        let mut baseline = vec![0.0; 9];
        baseline[4] = 5.0;
        water
            .replace_baseline_depth_from_dense(&baseline)
            .expect("baseline depth dimensions should match");
        water.add_water(1, 1, 2.0);

        let combined = water.clone_depth_dense();
        assert_eq!(combined[4], 7.0);
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
}
