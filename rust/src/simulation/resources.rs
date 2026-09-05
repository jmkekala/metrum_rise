// SPDX-License-Identifier: GPL-2.0-only

//! Authored natural-resource deposit layers.
//!
//! Resource deposits are map-authored data, not building/runtime economy state.
//! They are stored as sparse terrain-aligned grids so the world editor can paint
//! them cheaply and future extractors can query only the cells under their footprint.

use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::sparse_chunk_grid::SparseChunkGrid;
use godot::prelude::Vector2;

/// Resource id used by authored coal deposits and the economy catalog.
pub(crate) const COAL_RESOURCE_ID: &str = "coal";
/// Maximum stored deposit richness. Editor percentages are normalized into this range.
pub(crate) const RESOURCE_RICHNESS_MAX: u16 = 1000;

/// Terrain-aligned authored resource deposit storage for one world.
#[derive(Clone)]
pub(crate) struct ResourceDepositSystem {
    width: usize,
    height: usize,
    cell_size_m: f32,
    coal_richness: SparseChunkGrid<u16>,
}

impl ResourceDepositSystem {
    /// Creates an empty terrain-aligned resource grid with explicit dimensions.
    pub(crate) fn with_chunking(
        width: usize,
        height: usize,
        cell_size_m: f32,
        chunk_size: usize,
    ) -> Self {
        let safe_chunk_size = chunk_size.max(1);
        Self {
            width,
            height,
            cell_size_m: cell_size_m.max(f32::EPSILON),
            coal_richness: SparseChunkGrid::new(width, height, safe_chunk_size, 0u16),
        }
    }

    /// Creates empty authored resource storage for the current world config.
    pub(crate) fn from_world_config(config: &WorldConfig) -> Self {
        Self::with_chunking(
            config.terrain_grid_width(),
            config.terrain_grid_height(),
            config.terrain_cell_m,
            resource_chunk_cells_for_config(config),
        )
    }

    /// Returns the terrain-aligned resource grid dimensions.
    pub(crate) fn grid_dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Returns the authored resource cell spacing in world metres.
    pub(crate) fn cell_size_m(&self) -> f32 {
        self.cell_size_m
    }

    /// Paints coal richness into a circular world-space brush footprint.
    pub(crate) fn paint_coal_circle_world(
        &mut self,
        world_x: f32,
        world_z: f32,
        radius_m: f32,
        richness: u16,
    ) -> bool {
        self.paint_coal_circle_world_value(world_x, world_z, radius_m, richness)
    }

    /// Clears coal richness from a circular world-space brush footprint.
    pub(crate) fn erase_coal_circle_world(
        &mut self,
        world_x: f32,
        world_z: f32,
        radius_m: f32,
    ) -> bool {
        self.paint_coal_circle_world_value(world_x, world_z, radius_m, 0)
    }

    /// Returns the stored coal richness at one terrain-aligned grid cell.
    pub(crate) fn coal_richness_at(&self, x: usize, z: usize) -> u16 {
        self.coal_richness.get(x, z)
    }

    /// Sets one coal richness cell, clamping to the canonical storage range.
    pub(crate) fn set_coal_richness_at(&mut self, x: usize, z: usize, richness: u16) {
        self.coal_richness
            .set(x, z, richness.min(RESOURCE_RICHNESS_MAX));
    }

    /// Returns a dense row-major coal-richness snapshot for persistence and render upload.
    pub(crate) fn clone_coal_richness_dense(&self) -> Vec<u16> {
        self.coal_richness.clone_dense()
    }

    /// Replaces coal-richness storage from a dense row-major snapshot.
    #[cfg(test)]
    pub(crate) fn replace_coal_richness_from_dense(&mut self, dense: &[u16]) -> Result<(), String> {
        self.coal_richness.replace_from_dense(dense)
    }

    /// Computes coal reserve units under one world-space extraction polygon.
    ///
    /// Richness is sampled at authored resource grid points and converted through
    /// `units_per_full_richness_m2`, so a full-richness 10 m cell contributes
    /// `10 * 10 * units_per_full_richness_m2` units.
    pub(crate) fn coal_reserve_units_for_polygon(
        &self,
        polygon: &[Vector2],
        units_per_full_richness_m2: f32,
    ) -> f32 {
        if polygon.len() < 3
            || units_per_full_richness_m2 <= 0.0
            || !units_per_full_richness_m2.is_finite()
            || self.width == 0
            || self.height == 0
        {
            return 0.0;
        }
        let Some((min_x, max_x, min_z, max_z)) = self.polygon_grid_bounds(polygon) else {
            return 0.0;
        };

        let cell_area = self.cell_size_m * self.cell_size_m;
        let mut reserve_units = 0.0f32;
        for z in min_z..=max_z {
            for x in min_x..=max_x {
                let richness = self.coal_richness.get(x, z);
                if richness == 0 {
                    continue;
                }
                let world_pos = self.grid_to_world_pos(x, z);
                if !point_in_polygon(world_pos, polygon) {
                    continue;
                }
                reserve_units += cell_area
                    * (f32::from(richness) / f32::from(RESOURCE_RICHNESS_MAX))
                    * units_per_full_richness_m2;
            }
        }
        reserve_units
    }

    /// Returns true when no authored coal cells exist.
    #[cfg(test)]
    pub(crate) fn coal_is_empty(&self) -> bool {
        self.clone_coal_richness_dense()
            .iter()
            .all(|value| *value == 0)
    }

    fn paint_coal_circle_world_value(
        &mut self,
        world_x: f32,
        world_z: f32,
        radius_m: f32,
        richness: u16,
    ) -> bool {
        if !world_x.is_finite()
            || !world_z.is_finite()
            || !radius_m.is_finite()
            || radius_m <= 0.0
            || self.width == 0
            || self.height == 0
        {
            return false;
        }

        let value = richness.min(RESOURCE_RICHNESS_MAX);
        let (center_x, center_z) = self.world_to_grid_coords(world_x, world_z);
        let radius_cells = radius_m / self.cell_size_m;
        let Some((min_x, max_x, min_z, max_z)) =
            self.circle_grid_bounds(center_x, center_z, radius_cells)
        else {
            return false;
        };

        let radius_sq = radius_cells * radius_cells;
        let mut changed = false;
        for z in min_z..=max_z {
            for x in min_x..=max_x {
                let dx = x as f32 - center_x;
                let dz = z as f32 - center_z;
                if dx * dx + dz * dz > radius_sq {
                    continue;
                }
                if self.coal_richness.get(x, z) == value {
                    continue;
                }
                self.coal_richness.set(x, z, value);
                changed = true;
            }
        }
        changed
    }

    fn world_to_grid_coords(&self, world_x: f32, world_z: f32) -> (f32, f32) {
        let half_w = self.world_width_m() * 0.5;
        let half_h = self.world_height_m() * 0.5;
        (
            (world_x + half_w) / self.cell_size_m,
            (world_z + half_h) / self.cell_size_m,
        )
    }

    fn grid_to_world_pos(&self, x: usize, z: usize) -> Vector2 {
        let half_w = self.world_width_m() * 0.5;
        let half_h = self.world_height_m() * 0.5;
        Vector2::new(
            x as f32 * self.cell_size_m - half_w,
            z as f32 * self.cell_size_m - half_h,
        )
    }

    fn world_width_m(&self) -> f32 {
        self.width.saturating_sub(1) as f32 * self.cell_size_m
    }

    fn world_height_m(&self) -> f32 {
        self.height.saturating_sub(1) as f32 * self.cell_size_m
    }

    fn circle_grid_bounds(
        &self,
        center_x: f32,
        center_z: f32,
        radius_cells: f32,
    ) -> Option<(usize, usize, usize, usize)> {
        let min_x = (center_x - radius_cells).floor();
        let max_x = (center_x + radius_cells).ceil();
        let min_z = (center_z - radius_cells).floor();
        let max_z = (center_z + radius_cells).ceil();
        if max_x < 0.0
            || max_z < 0.0
            || min_x > (self.width - 1) as f32
            || min_z > (self.height - 1) as f32
        {
            return None;
        }
        Some((
            min_x.clamp(0.0, (self.width - 1) as f32) as usize,
            max_x.clamp(0.0, (self.width - 1) as f32) as usize,
            min_z.clamp(0.0, (self.height - 1) as f32) as usize,
            max_z.clamp(0.0, (self.height - 1) as f32) as usize,
        ))
    }

    fn polygon_grid_bounds(&self, polygon: &[Vector2]) -> Option<(usize, usize, usize, usize)> {
        let mut min_world_x = f32::INFINITY;
        let mut max_world_x = f32::NEG_INFINITY;
        let mut min_world_z = f32::INFINITY;
        let mut max_world_z = f32::NEG_INFINITY;
        for point in polygon {
            if !point.x.is_finite() || !point.y.is_finite() {
                return None;
            }
            min_world_x = min_world_x.min(point.x);
            max_world_x = max_world_x.max(point.x);
            min_world_z = min_world_z.min(point.y);
            max_world_z = max_world_z.max(point.y);
        }
        if min_world_x > max_world_x || min_world_z > max_world_z {
            return None;
        }
        let (min_grid_x, min_grid_z) = self.world_to_grid_coords(min_world_x, min_world_z);
        let (max_grid_x, max_grid_z) = self.world_to_grid_coords(max_world_x, max_world_z);
        if max_grid_x < 0.0
            || max_grid_z < 0.0
            || min_grid_x > (self.width - 1) as f32
            || min_grid_z > (self.height - 1) as f32
        {
            return None;
        }
        Some((
            min_grid_x.floor().clamp(0.0, (self.width - 1) as f32) as usize,
            max_grid_x.ceil().clamp(0.0, (self.width - 1) as f32) as usize,
            min_grid_z.floor().clamp(0.0, (self.height - 1) as f32) as usize,
            max_grid_z.ceil().clamp(0.0, (self.height - 1) as f32) as usize,
        ))
    }
}

fn resource_chunk_cells_for_config(config: &WorldConfig) -> usize {
    ((config.terrain_chunk_m / config.terrain_cell_m).ceil() as usize).max(1)
}

fn point_in_polygon(point: Vector2, polygon: &[Vector2]) -> bool {
    let mut inside = false;
    let mut prev = polygon[polygon.len() - 1];
    for &curr in polygon {
        let crosses = (curr.y > point.y) != (prev.y > point.y);
        if crosses {
            let denom = prev.y - curr.y;
            if denom.abs() > f32::EPSILON {
                let intersection_x = (prev.x - curr.x) * (point.y - curr.y) / denom + curr.x;
                if point.x < intersection_x {
                    inside = !inside;
                }
            }
        }
        prev = curr;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coal_paint_and_erase_touch_only_brush_cells() {
        let config = WorldConfig::new(100.0, 100.0, 40.0, 10.0)
            .with_terrain_resolution(10.0)
            .with_chunking(40.0, 0.0);
        let mut deposits = ResourceDepositSystem::from_world_config(&config);

        assert!(deposits.paint_coal_circle_world(0.0, 0.0, 10.0, 700));
        assert_eq!(deposits.coal_richness_at(5, 5), 700);
        assert_eq!(deposits.coal_richness_at(0, 0), 0);

        assert!(deposits.erase_coal_circle_world(0.0, 0.0, 10.0));
        assert_eq!(deposits.coal_richness_at(5, 5), 0);
    }

    #[test]
    fn coal_dense_round_trip_preserves_sparse_values() {
        let mut deposits = ResourceDepositSystem::with_chunking(4, 4, 10.0, 2);
        deposits.set_coal_richness_at(1, 1, 350);
        deposits.set_coal_richness_at(3, 2, 1200);

        let dense = deposits.clone_coal_richness_dense();
        let mut loaded = ResourceDepositSystem::with_chunking(4, 4, 10.0, 2);
        loaded
            .replace_coal_richness_from_dense(&dense)
            .expect("dense resource grid should fit");

        assert_eq!(loaded.coal_richness_at(1, 1), 350);
        assert_eq!(loaded.coal_richness_at(3, 2), RESOURCE_RICHNESS_MAX);
    }
}
