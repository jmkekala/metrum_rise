//! World configuration and dense-grid sizing helpers.
//!
//! `WorldConfig` is the chunk-aware replacement for the legacy map-size config.
//! It owns authoritative world extents plus terrain chunk metadata even though
//! some current runtime systems still derive dense helper grids from it.

/// Chunk-aware world configuration shared by gameplay, saves, and editor sandboxes.
///
/// The world extent is defined by physical dimensions plus terrain chunk metadata.
/// Some current systems still consume dense whole-world helper grids; those
/// compatibility helpers remain here until terrain and water storage become sparse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldConfig {
    /// Total physical width of the authored world in metres.
    pub width_m: f32,
    /// Total physical height of the authored world in metres.
    pub height_m: f32,
    /// Physical size of one terrain and water sample cell in metres.
    pub terrain_cell_m: f32,
    /// Canonical authored terrain chunk span in metres.
    pub terrain_chunk_m: f32,
    /// Default base terrain elevation for untouched blank-world chunks.
    pub terrain_base_elevation_m: f32,
    /// Physical size of one environmental grid cell (pollution, noise, desirability) in metres.
    pub env_cell_m: f32,
    /// Physical size of one zoning grid cell in metres.
    pub zone_cell_m: f32,
}

impl WorldConfig {
    /// Default terrain sample spacing used by the current gameplay fallback world.
    pub const DEFAULT_TERRAIN_CELL_M: f32 = 10.0;
    /// Default canonical terrain chunk size for blank authored worlds.
    pub const DEFAULT_TERRAIN_CHUNK_M: f32 = 512.0;
    /// Default base elevation for untouched blank authored worlds.
    pub const DEFAULT_TERRAIN_BASE_ELEVATION_M: f32 = 0.0;
    /// Default gameplay fallback world width when no authored world is loaded.
    pub const DEFAULT_GAMEPLAY_WORLD_WIDTH_M: f32 = 20_000.0;
    /// Default gameplay fallback world height when no authored world is loaded.
    pub const DEFAULT_GAMEPLAY_WORLD_HEIGHT_M: f32 = 20_000.0;
    /// Small editor sandbox width used by non-gameplay tool scenes.
    pub const EDITOR_SANDBOX_WIDTH_M: f32 = 500.0;
    /// Small editor sandbox height used by non-gameplay tool scenes.
    pub const EDITOR_SANDBOX_HEIGHT_M: f32 = 500.0;

    /// Creates a new world configuration with default chunk metadata.
    pub fn new(width_m: f32, height_m: f32, env_cell_m: f32, zone_cell_m: f32) -> Self {
        Self {
            width_m,
            height_m,
            terrain_cell_m: Self::DEFAULT_TERRAIN_CELL_M,
            terrain_chunk_m: Self::DEFAULT_TERRAIN_CHUNK_M,
            terrain_base_elevation_m: Self::DEFAULT_TERRAIN_BASE_ELEVATION_M,
            env_cell_m,
            zone_cell_m,
        }
    }

    /// Overrides the terrain chunk metadata while keeping the current dense-grid sizing.
    pub fn with_chunking(
        mut self,
        terrain_chunk_m: f32,
        terrain_base_elevation_m: f32,
    ) -> Self {
        self.terrain_chunk_m = terrain_chunk_m;
        self.terrain_base_elevation_m = terrain_base_elevation_m;
        self
    }

    /// Overrides the terrain sample spacing while preserving the current world extent metadata.
    pub fn with_terrain_resolution(mut self, terrain_cell_m: f32) -> Self {
        self.terrain_cell_m = terrain_cell_m.max(f32::EPSILON);
        self
    }

    /// Returns the fallback gameplay world used before authored-world selection exists.
    pub fn gameplay_default() -> Self {
        Self::new(
            Self::DEFAULT_GAMEPLAY_WORLD_WIDTH_M,
            Self::DEFAULT_GAMEPLAY_WORLD_HEIGHT_M,
            40.0,
            10.0,
        )
    }

    /// Returns the stripped-down editor sandbox world used by tool-only scenes.
    pub fn editor_sandbox() -> Self {
        Self::new(
            Self::EDITOR_SANDBOX_WIDTH_M,
            Self::EDITOR_SANDBOX_HEIGHT_M,
            40.0,
            10.0,
        )
    }

    /// Returns the runtime world width in the current gameplay coordinate units.
    ///
    /// The live runtime still uses one world-space unit per zoning cell width rather than one
    /// world-space unit per physical metre. This helper makes that compatibility layer explicit.
    pub fn world_width_units(&self) -> f32 {
        (self.zone_grid_width().saturating_sub(1)) as f32
    }

    /// Returns the runtime world height in the current gameplay coordinate units.
    pub fn world_height_units(&self) -> f32 {
        (self.zone_grid_height().saturating_sub(1)) as f32
    }

    /// Returns the terrain sample spacing in current gameplay coordinate units.
    pub fn terrain_cell_world_units(&self) -> f32 {
        (self.terrain_cell_m / self.zone_cell_m).max(f32::EPSILON)
    }

    /// Returns the number of authored terrain chunks along the X axis.
    pub fn terrain_chunk_columns(&self) -> usize {
        (self.width_m / self.terrain_chunk_m).ceil() as usize
    }

    /// Returns the number of authored terrain chunks along the Y axis.
    pub fn terrain_chunk_rows(&self) -> usize {
        (self.height_m / self.terrain_chunk_m).ceil() as usize
    }

    /// Returns the number of runtime terrain samples along the X axis.
    pub fn terrain_grid_width(&self) -> usize {
        ((self.world_width_units() / self.terrain_cell_world_units()).round() as usize) + 1
    }

    /// Returns the number of runtime terrain samples along the Y axis.
    pub fn terrain_grid_height(&self) -> usize {
        ((self.world_height_units() / self.terrain_cell_world_units()).round() as usize) + 1
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

    /// Helper to get (width, height) for the runtime terrain grid.
    pub fn get_terrain_grid_size(&self) -> (usize, usize) {
        (self.terrain_grid_width(), self.terrain_grid_height())
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

impl Default for WorldConfig {
    /// Returns the default chunk-aware gameplay fallback world.
    fn default() -> Self {
        Self::gameplay_default()
    }
}
