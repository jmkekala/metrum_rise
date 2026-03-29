//! Global map configuration and grid dimension calculations.

/// Configuration for the simulation's spatial dimensions and grid resolutions.
///
/// This struct replaces hardcoded constants from `config.rs` with runtime values,
/// allowing for different map sizes (e.g. standard 10km vs huge 20km).
#[derive(Clone, Copy, Debug)]
pub struct MapConfig {
    /// Total physical width of the map in metres.
    pub width_m: f32,
    /// Total physical height of the map in metres.
    pub height_m: f32,
    /// Physical size of one environmental grid cell (pollution, noise, desirability) in metres.
    pub env_cell_m: f32,
    /// Physical size of one zoning grid cell in metres.
    pub zone_cell_m: f32,
}

impl MapConfig {
    /// Creates a new configuration with the specified dimensions.
    pub fn new(width_m: f32, height_m: f32, env_cell_m: f32, zone_cell_m: f32) -> Self {
        Self {
            width_m,
            height_m,
            env_cell_m,
            zone_cell_m,
        }
    }

    /// Returns the number of cells in the zoning grid along the X axis.
    pub fn zone_grid_width(&self) -> usize {
        (self.width_m / self.zone_cell_m).round() as usize
    }

    /// Returns the number of cells in the zoning grid along the Y axis.
    pub fn zone_grid_height(&self) -> usize {
        (self.height_m / self.zone_cell_m).round() as usize
    }

    /// Returns the number of cells in the environmental grid along the X axis.
    pub fn env_grid_width(&self) -> usize {
        (self.width_m / self.env_cell_m).round() as usize
    }

    /// Returns the number of cells in the environmental grid along the Y axis.
    pub fn env_grid_height(&self) -> usize {
        (self.height_m / self.env_cell_m).round() as usize
    }

    /// Helper to get (width, height) for environmental grid.
    pub fn get_env_grid_size(&self) -> (usize, usize) {
        (self.env_grid_width(), self.env_grid_height())
    }

    /// Helper to get (width, height) for zoning grid.
    pub fn get_zone_grid_size(&self) -> (usize, usize) {
        (self.zone_grid_width(), self.zone_grid_height())
    }

    /// Maps world-space coordinates (units) to environmental grid coordinates.
    pub fn world_to_env_grid(&self, x: f32, z: f32, env_w: usize, env_h: usize) -> (f32, f32) {
        let zw = self.zone_grid_width() as f32;
        let zh = self.zone_grid_height() as f32;
        let hw = (zw - 1.0) * 0.5;
        let hh = (zh - 1.0) * 0.5;

        // Map [-HW, HW] to [0, ZW] then scale to [0, ENV_W]
        let gx = (x + hw) * (env_w as f32 / zw);
        let gz = (z + hh) * (env_h as f32 / zh);
        (gx, gz)
    }
}

impl Default for MapConfig {
    /// Default 20km x 20km configuration with 40m env cells and 10m zoning cells.
    fn default() -> Self {
        Self {
            width_m: 20000.0,
            height_m: 20000.0,
            env_cell_m: 40.0,
            zone_cell_m: 10.0,
        }
    }
}
