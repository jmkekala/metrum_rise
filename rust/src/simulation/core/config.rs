//! World configuration and bounded-grid sizing helpers.
//!
//! `WorldConfig` is the chunk-aware world-size config.
//! It owns authoritative world extents in metres plus the cell sizes used by
//! terrain, parcel zoning, and environmental grids.

/// Chunk-aware world configuration shared by gameplay, saves, and editor sandboxes.
///
/// The world extent is defined by physical dimensions plus terrain chunk metadata.
/// Bounded helper grids still derive their dimensions from this config, but their
/// world-space coordinates are canonical metres.
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
    /// Physical size of one zoning/building footprint cell in metres.
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

    /// Overrides the terrain chunk metadata while keeping current grid sizing.
    pub fn with_chunking(mut self, terrain_chunk_m: f32, terrain_base_elevation_m: f32) -> Self {
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

    /// Returns the shared world-space span used by terrain, water, and road render chunks.
    pub fn terrain_render_chunk_span_m(&self) -> f32 {
        let cell_m = self.terrain_cell_m.max(f32::EPSILON);
        let interval_count = (self.terrain_chunk_m.max(cell_m) / cell_m).round().max(1.0);
        interval_count * cell_m
    }

    /// Returns the world-space minimum corner of the runtime terrain sample grid.
    pub fn terrain_world_origin_m(&self) -> (f32, f32) {
        let width_m = self.terrain_grid_width().saturating_sub(1) as f32 * self.terrain_cell_m;
        let height_m = self.terrain_grid_height().saturating_sub(1) as f32 * self.terrain_cell_m;
        (-width_m * 0.5, -height_m * 0.5)
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
        ((self.width_m / self.terrain_cell_m).round() as usize) + 1
    }

    /// Returns the number of runtime terrain samples along the Y axis.
    pub fn terrain_grid_height(&self) -> usize {
        ((self.height_m / self.terrain_cell_m).round() as usize) + 1
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

    /// Maps world-space coordinates in metres to environmental-grid coordinates.
    pub fn world_to_env_grid(&self, x: f32, z: f32, env_w: usize, env_h: usize) -> (f32, f32) {
        let _ = (env_w, env_h);
        let gx = ((x + self.width_m * 0.5) / self.env_cell_m) - 0.5;
        let gz = ((z + self.height_m * 0.5) / self.env_cell_m) - 0.5;
        (gx, gz)
    }
}

impl Default for WorldConfig {
    /// Returns the default chunk-aware gameplay fallback world.
    fn default() -> Self {
        Self::gameplay_default()
    }
}

#[cfg(test)]
mod tests {
    use super::WorldConfig;

    #[test]
    fn render_chunk_span_never_falls_below_one_terrain_sample() {
        let config = WorldConfig::new(100.0, 100.0, 10.0, 10.0)
            .with_chunking(4.0, 0.0)
            .with_terrain_resolution(8.0);

        assert_eq!(config.terrain_render_chunk_span_m(), 8.0);
    }

    #[test]
    fn render_chunk_span_is_an_exact_terrain_interval_multiple() {
        let config = WorldConfig::default();

        assert_eq!(config.terrain_chunk_m, 512.0);
        assert_eq!(config.terrain_cell_m, 10.0);
        assert_eq!(config.terrain_render_chunk_span_m(), 510.0);
    }

    #[test]
    fn terrain_world_origin_matches_runtime_sample_extent() {
        assert_eq!(
            WorldConfig::gameplay_default().terrain_world_origin_m(),
            (-10_000.0, -10_000.0)
        );
        assert_eq!(
            WorldConfig::editor_sandbox().terrain_world_origin_m(),
            (-250.0, -250.0)
        );
    }
}
