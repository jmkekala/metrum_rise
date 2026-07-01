//! Industrial pollution emission and diffusion system.

use super::data_grid::DataGrid;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

/// A grid-based system that simulates industrial pollution.
///
/// Pollution is emitted by industrial buildings, then diffuses and decays
/// across the environment grid using a finite-difference method.
pub struct PollutionSystem {
    /// The current pollution level grid (0-100).
    pub grid: DataGrid<f32>,
    /// Intermediate swap buffer used for parallel diffusion calculations.
    pub swap: DataGrid<f32>,
}

impl PollutionSystem {
    /// Creates a new pollution system derived from the world map configuration.
    pub fn new(config: &crate::simulation::core::config::WorldConfig) -> Self {
        let (w, h) = config.get_env_grid_size();
        Self {
            grid: DataGrid::new(w, h, 0.0),
            swap: DataGrid::new(w, h, 0.0),
        }
    }

    /// Advances the pollution simulation: swaps buffers, emits from industrial sources, and diffuses.
    pub fn tick(
        &mut self,
        allocator: &BuildingAllocator,
        config: &crate::simulation::core::config::WorldConfig,
    ) {
        // Swap buffers: current grid moves to swap (source),
        // swap (old data) moves to grid (target for this tick)
        std::mem::swap(&mut self.grid, &mut self.swap);
        self.grid.data.fill(0.0);

        let w = self.grid.width;
        let h = self.grid.height;

        // 1. Emission (Sequential as building count is small compared to grid)
        for b in &allocator.buildings {
            if b.zone_type == ZoneType::Industrial {
                let (gx_raw, gy_raw) = config.world_to_env_grid(b.center_x, b.center_y, w, h);
                let gx = gx_raw.round() as i32;
                let gy = gy_raw.round() as i32;

                if gx >= 0 && gx < w as i32 && gy >= 0 && gy < h as i32 {
                    if let Some(val) = self.grid.get_mut(gx as usize, gy as usize) {
                        *val += 5.0;
                    }
                }
            }
        }

        // 2. Diffusion & 3. Decay (Parallelized)
        let old_grid = &self.swap;

        self.grid
            .data
            .par_chunks_mut(w)
            .enumerate()
            .for_each(|(y, row)| {
                for x in 0..w {
                    let current = *old_grid.get(x, y).unwrap_or(&0.0);

                    let mut neighbor_sum = 0.0;
                    let mut count = 0.0;

                    // Neighborhood Sampling
                    if x > 0 {
                        neighbor_sum += *old_grid.get(x - 1, y).unwrap_or(&0.0);
                        count += 1.0;
                    }
                    if x < w - 1 {
                        neighbor_sum += *old_grid.get(x + 1, y).unwrap_or(&0.0);
                        count += 1.0;
                    }
                    if y > 0 {
                        neighbor_sum += *old_grid.get(x, y - 1).unwrap_or(&0.0);
                        count += 1.0;
                    }
                    if y < h - 1 {
                        neighbor_sum += *old_grid.get(x, y + 1).unwrap_or(&0.0);
                        count += 1.0;
                    }

                    let avg = if count > 0.0 {
                        neighbor_sum / count
                    } else {
                        0.0
                    };

                    // Diffuse: keep 60% of own, take 40% of avg neighbor. Decay: 99.5% retention.
                    let propagated = (current * 0.60 + avg * 0.40) * 0.995;

                    // Combine emission (already in row[x]) + diffused state
                    row[x] = (row[x] + propagated).min(100.0).max(0.0);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetManifest;
    use crate::assets::asset::{BuildingData, MeshPart, PlacementMode, ZoneClass};
    use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::zoning::ZoneType;
    use godot::prelude::Vector2;

    fn register_test_asset(
        allocator: &mut BuildingAllocator,
        pack_id: &str,
        asset_id: &str,
        zone: ZoneClass,
    ) -> String {
        let (household_capacity, worker_capacity) = match zone {
            ZoneClass::Residential => (Some(6), None),
            ZoneClass::Commercial | ZoneClass::Industrial | ZoneClass::Office => (None, Some(4)),
            ZoneClass::Mixed => (Some(4), Some(2)),
        };
        allocator.registry.register(
            pack_id,
            AssetManifest {
                asset_id: asset_id.to_owned(),
                display_name: "Test".to_owned(),
                asset_set: None,
                tags: vec![],
                thumbnail: None,
                lods: vec![],
                mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
                anchors: vec![],
                site_surfaces: vec![],
                building: Some(BuildingData {
                    flat_size_m2: None,
                    placement_mode: PlacementMode::ZonedPrivate,
                    zone_type: Some(zone),
                    density: Some("low".to_owned()),
                    lot_width_cells: 3,
                    lot_depth_cells: 3,
                    min_zone_width_cells: None,
                    min_zone_depth_cells: None,
                    level: 1,
                    household_capacity,
                    worker_capacity,
                    service_class: None,
                    economy_profile: None,
                }),
                prop: None,
                vehicle: None,
                character: None,
            },
            String::new(),
        );
        format!("{pack_id}:{asset_id}")
    }

    #[test]
    fn test_pollution_diffusion_and_decay() {
        let config = WorldConfig::default();
        let mut system = PollutionSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        let industrial_asset = register_test_asset(
            &mut allocator,
            "test",
            "pollution_industrial",
            ZoneClass::Industrial,
        );

        // 1. Add one industrial source at world (0,0)
        // Default gameplay WorldConfig is 20km x 20km, so (0,0) is at the center of the grid.
        let source_building = Building {
            center_x: 0.0,
            center_y: 0.0,
            support_height_m: 0.0,
            width_cells: 3,
            depth_cells: 3,
            zone_profile_runtime_id: 0,
            parcel_id: 0,
            zone_type: ZoneType::Industrial,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 0.0,
            budget_distress: false,
            is_deserted: false,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            asset_id: industrial_asset,
            level: 1,
            construction_total_hours: 0,
            construction_remaining_hours: 0,
            broken: false,
            economy_profile_runtime_id: 0,
            economy_broken: false,
            resource_inventory: Vec::new(),
            revenue: 0.0,
            operating_budget: 500.0,
            profit_tax_budget_baseline: 500.0,
            last_day_profit: 0.0,
            shipment_cooldown_hours: 0,
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            daily_city_funded_input_cost: 0.0,
            daily_household_sales_value: 0.0,
            daily_power_service_units: 0.0,
            daily_power_served_units: 0.0,
            recent_power_service_units: 0.0,
            recent_power_served_units: 0.0,
            recent_household_sales_value: 0.0,
            commercial_activity_floor_scale: 0.0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        };
        allocator.buildings.push(source_building);

        let (gw, gh) = config.get_env_grid_size();
        let source_gx = (gw / 2) as usize;
        let source_gy = (gh / 2) as usize;

        // 2. Tick 200 times to allow diffusion
        for _ in 0..200 {
            system.tick(&allocator, &config);
        }

        // 3. Assertions
        let source_val = *system.grid.get(source_gx, source_gy).unwrap();
        assert!(
            source_val > 0.0,
            "Source cell should have positive pollution. Got: {}",
            source_val
        );

        let diffused_val = *system.grid.get(source_gx + 5, source_gy).unwrap();
        assert!(
            diffused_val > 0.0,
            "Cell 5 steps away should have nonzero diffused pollution. Got: {}",
            diffused_val
        );

        // Check for NaN/Inf
        for y in 0..gh {
            for x in 0..gw {
                let val = *system.grid.get(x, y).unwrap();
                assert!(
                    val.is_finite(),
                    "Pollution value at ({}, {}) is not finite: {}",
                    x,
                    y,
                    val
                );
            }
        }

        // 4. Test Decay: Remove source and track average
        allocator.buildings.clear();

        let mut avg_before = 0.0;
        for &val in &system.grid.data {
            avg_before += val;
        }
        avg_before /= system.grid.data.len() as f32;

        // Tick 50 times more
        for _ in 0..50 {
            system.tick(&allocator, &config);
        }

        let mut avg_after = 0.0;
        for &val in &system.grid.data {
            avg_after += val;
        }
        avg_after /= system.grid.data.len() as f32;

        assert!(
            avg_after < avg_before,
            "Average pollution should decay after source removal. Before: {}, After: {}",
            avg_before,
            avg_after
        );
    }
}
