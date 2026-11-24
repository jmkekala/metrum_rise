// SPDX-License-Identifier: GPL-3.0-or-later

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
    /// Physical size of one terrain and water sample cell in metres: 2, 4 or 8.
    pub terrain_cell_m: f32,
    /// Default base terrain elevation for untouched blank-world chunks.
    pub terrain_base_elevation_m: f32,
    /// Physical size of one environmental grid cell (pollution, noise, desirability) in metres.
    pub env_cell_m: f32,
    /// Physical size of one zoning/building footprint cell in metres.
    pub zone_cell_m: f32,
}

impl WorldConfig {
    /// Terrain sample spacings a world may use, in metres. Each divides the 64 m graded tile
    /// and the 512 m patch, and the far level's step of 8 cells stays within one tile.
    pub const ALLOWED_TERRAIN_CELLS_M: [f32; 3] = [2.0, 4.0, 8.0];
    /// World sides are whole multiples of this many metres, so the world edge lies on a 64 m
    /// graded tile line from the centred origin and every tile lies inside one render patch.
    pub const WORLD_SIDE_MULTIPLE_M: f32 = 128.0;
    /// The terrain render patch and storage chunk span in metres, fixed for every world.
    pub const TERRAIN_CHUNK_M: f32 = 512.0;
    /// Default terrain sample spacing used by the current gameplay fallback world.
    pub const DEFAULT_TERRAIN_CELL_M: f32 = 8.0;
    /// Default base elevation for untouched blank authored worlds.
    pub const DEFAULT_TERRAIN_BASE_ELEVATION_M: f32 = 0.0;
    /// Default gameplay fallback world width when no authored world is loaded.
    pub const DEFAULT_GAMEPLAY_WORLD_WIDTH_M: f32 = 20_480.0;
    /// Default gameplay fallback world height when no authored world is loaded.
    pub const DEFAULT_GAMEPLAY_WORLD_HEIGHT_M: f32 = 20_480.0;
    /// Small editor sandbox width used by non-gameplay tool scenes.
    pub const EDITOR_SANDBOX_WIDTH_M: f32 = 512.0;
    /// Small editor sandbox height used by non-gameplay tool scenes.
    pub const EDITOR_SANDBOX_HEIGHT_M: f32 = 512.0;

    /// Creates a new world configuration with the default terrain cell and base elevation.
    pub fn new(width_m: f32, height_m: f32, env_cell_m: f32, zone_cell_m: f32) -> Self {
        Self {
            width_m,
            height_m,
            terrain_cell_m: Self::DEFAULT_TERRAIN_CELL_M,
            terrain_base_elevation_m: Self::DEFAULT_TERRAIN_BASE_ELEVATION_M,
            env_cell_m,
            zone_cell_m,
        }
    }

    /// Overrides the base elevation of untouched blank-world terrain, in metres.
    pub fn with_base_elevation(mut self, terrain_base_elevation_m: f32) -> Self {
        self.terrain_base_elevation_m = terrain_base_elevation_m;
        self
    }

    /// Overrides the terrain sample spacing while preserving the current world extent metadata.
    pub fn with_terrain_resolution(mut self, terrain_cell_m: f32) -> Self {
        self.terrain_cell_m = terrain_cell_m;
        self
    }

    /// Validates authored metadata and derived grid layouts before world allocation.
    ///
    /// The world rules: the terrain cell is 2, 4 or 8 m and both sides are whole multiples of
    /// 128 m (the patch and storage chunk are 512 m and not configurable). Grid axes must fit
    /// signed runtime coordinates and dense buffers must fit Rust's allocation layout. This
    /// does not guarantee that available memory can hold the world.
    pub fn validate(&self) -> Result<(), String> {
        for (value, label) in [
            (self.width_m, "width_m"),
            (self.height_m, "height_m"),
            (self.terrain_cell_m, "terrain_cell_m"),
            (self.env_cell_m, "env_cell_m"),
            (self.zone_cell_m, "zone_cell_m"),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!("{label} must be finite and > 0"));
            }
        }
        if !self.terrain_base_elevation_m.is_finite() {
            return Err("terrain_base_elevation_m must be finite".to_owned());
        }
        if !Self::ALLOWED_TERRAIN_CELLS_M.contains(&self.terrain_cell_m) {
            return Err(format!(
                "terrain_cell_m must be 2, 4 or 8 m (found {} m)",
                self.terrain_cell_m
            ));
        }
        for (side_m, label) in [(self.width_m, "width_m"), (self.height_m, "height_m")] {
            if side_m % Self::WORLD_SIDE_MULTIPLE_M != 0.0 {
                return Err(format!(
                    "{label} must be a whole multiple of 128 m (found {side_m} m)"
                ));
            }
        }
        validate_grid_layout(
            (self.width_m / self.terrain_cell_m).round(),
            (self.height_m / self.terrain_cell_m).round(),
            1,
            "terrain grid",
        )?;
        validate_grid_layout(
            (self.width_m / self.env_cell_m).round(),
            (self.height_m / self.env_cell_m).round(),
            0,
            "environment grid",
        )?;
        for extent in [self.width_m, self.height_m] {
            validate_grid_axis((extent / self.zone_cell_m).ceil(), 0, "zoning coordinates")?;
        }
        Ok(())
    }

    /// Refuses a stored world or save off the world rules: a terrain chunk span other than the
    /// fixed 512 m, then [`Self::validate`]. The message names the broken rule and why the file
    /// is refused, for the player who opened it.
    ///
    /// Stored files still carry the span they were written with; the storage chunk rows of a
    /// world definition are laid out on it. The writers validate, so a file off the rules is
    /// one from before the aligned grid (or edited by hand).
    pub fn validate_stored(&self, stored_chunk_m: f32) -> Result<(), String> {
        let rule = if stored_chunk_m == Self::TERRAIN_CHUNK_M {
            self.validate()
        } else {
            Err(format!(
                "terrain chunk span must be 512 m (found {stored_chunk_m} m)"
            ))
        };
        rule.map_err(|rule| {
            format!("{rule}; worlds and saves from before the aligned grid do not load")
        })
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

    /// Returns the square storage-chunk side used by terrain, water and deposit grids:
    /// 512 m of cells (256, 128 or 64).
    pub(crate) fn terrain_storage_chunk_cells(&self) -> usize {
        ((Self::TERRAIN_CHUNK_M / self.terrain_cell_m).ceil() as usize).max(1)
    }

    /// Returns the shared world-space span used by terrain, water, and road render chunks:
    /// 512 m on every legal cell.
    pub fn terrain_render_chunk_span_m(&self) -> f32 {
        let cell_m = self.terrain_cell_m.max(f32::EPSILON);
        let interval_count = (Self::TERRAIN_CHUNK_M.max(cell_m) / cell_m).round().max(1.0);
        interval_count * cell_m
    }

    /// Returns the world-space minimum corner of the runtime terrain sample grid.
    pub fn terrain_world_origin_m(&self) -> (f32, f32) {
        let width_m = self.terrain_grid_width().saturating_sub(1) as f32 * self.terrain_cell_m;
        let height_m = self.terrain_grid_height().saturating_sub(1) as f32 * self.terrain_cell_m;
        (-width_m * 0.5, -height_m * 0.5)
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
    pub fn world_to_env_grid(&self, x: f32, z: f32) -> (f32, f32) {
        let gx = ((x + self.width_m * 0.5) / self.env_cell_m) - 0.5;
        let gz = ((z + self.height_m * 0.5) / self.env_cell_m) - 0.5;
        (gx, gz)
    }
}

fn validate_grid_axis(samples: f32, extra: usize, label: &str) -> Result<usize, String> {
    let dimension = f64::from(samples) + extra as f64;
    if !dimension.is_finite() || dimension < 1.0 || dimension > f64::from(i32::MAX) {
        return Err(format!(
            "{label} dimensions must fit positive signed grid coordinates"
        ));
    }
    Ok(dimension as usize)
}

fn validate_grid_layout(width: f32, height: f32, extra: usize, label: &str) -> Result<(), String> {
    let width = validate_grid_axis(width, extra, label)?;
    let height = validate_grid_axis(height, extra, label)?;
    let bytes = width
        .checked_mul(height)
        .and_then(|cells| cells.checked_mul(std::mem::size_of::<f32>()));
    if bytes.is_none_or(|bytes| bytes > isize::MAX as usize) {
        return Err(format!(
            "{label} storage exceeds the supported allocation layout"
        ));
    }
    Ok(())
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
    fn render_chunk_span_is_512_m_on_every_allowed_cell() {
        for cell_m in WorldConfig::ALLOWED_TERRAIN_CELLS_M {
            let config = WorldConfig::default().with_terrain_resolution(cell_m);
            assert!(config.validate().is_ok());
            assert_eq!(config.terrain_render_chunk_span_m(), 512.0);
            assert_eq!(
                config.terrain_storage_chunk_cells() as f32 * cell_m,
                WorldConfig::TERRAIN_CHUNK_M
            );
        }
        let config = WorldConfig::default();
        assert_eq!(config.terrain_cell_m, 8.0);
        assert_eq!((config.width_m, config.height_m), (20_480.0, 20_480.0));
    }

    #[test]
    fn world_rules_refuse_other_cells_and_sides() {
        let legal = WorldConfig::new(640.0, 1_024.0, 40.0, 10.0);
        assert!(legal.validate().is_ok());
        for (config, rule) in [
            (legal.with_terrain_resolution(10.0), "terrain_cell_m must be 2, 4 or 8 m"),
            (legal.with_terrain_resolution(1.0), "terrain_cell_m must be 2, 4 or 8 m"),
            (legal.with_terrain_resolution(16.0), "terrain_cell_m must be 2, 4 or 8 m"),
            (
                WorldConfig::new(500.0, 512.0, 40.0, 10.0),
                "width_m must be a whole multiple of 128 m",
            ),
            (
                WorldConfig::new(512.0, 4_620.0, 40.0, 10.0),
                "height_m must be a whole multiple of 128 m",
            ),
        ] {
            let error = config.validate().unwrap_err();
            assert!(error.contains(rule), "{error}");
        }
        assert!(legal.validate_stored(512.0).is_ok());
        let error = legal.validate_stored(520.0).unwrap_err();
        assert!(error.contains("terrain chunk span must be 512 m"), "{error}");
        let error = legal.with_terrain_resolution(10.0).validate_stored(512.0).unwrap_err();
        assert!(
            error.contains("terrain_cell_m must be 2, 4 or 8 m")
                && error.ends_with("worlds and saves from before the aligned grid do not load"),
            "{error}"
        );
    }

    #[test]
    fn terrain_world_origin_matches_runtime_sample_extent() {
        assert_eq!(
            WorldConfig::gameplay_default().terrain_world_origin_m(),
            (-10_240.0, -10_240.0)
        );
        assert_eq!(
            WorldConfig::editor_sandbox().terrain_world_origin_m(),
            (-256.0, -256.0)
        );
    }
}
