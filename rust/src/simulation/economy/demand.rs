//! Global R/C/I demand signals for zoning-driven growth.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::grid::zoning::ZoneType;

/// Tracks the global demand for Residential, Commercial, and Industrial zones.
///
/// Demand is consumed by the building allocator when spawning new buildings
/// and is currently derived from live economy pressure rather than from organic
/// background growth.
pub struct DemandSystem {
    /// Residential demand (0-100).
    pub residential: f32,
    /// Commercial demand (0-100).
    pub commercial: f32,
    /// Industrial demand (0-100).
    pub industrial: f32,
}

impl DemandSystem {
    /// Creates a new demand system with base starter values.
    pub fn new() -> Self {
        Self {
            residential: 50.0,
            commercial: 25.0,
            industrial: 25.0, // Base starter demand
        }
    }

    /// Rebuilds zoning demand from current household, job, and stock pressure.
    pub fn recalculate(&mut self, allocator: &BuildingAllocator, households: &HouseholdSystem) {
        const HOUSEHOLD_TARGET_STOCK_DAYS: f32 = 3.0;
        const COMMERCIAL_TARGET_STOCK_UNITS: f32 = 600.0;
        const INDUSTRIAL_TARGET_STOCK_UNITS: f32 = 320.0;

        let mut housing_capacity = 0.0;
        let mut worker_capacity = 0.0;
        let mut filled_workers = 0.0;

        let mut commercial_stock_pressure_sum = 0.0;
        let mut commercial_count = 0.0;
        let mut industrial_stock_pressure_sum = 0.0;
        let mut industrial_count = 0.0;

        for (idx, building) in allocator.buildings.iter().enumerate() {
            if building.broken {
                continue;
            }

            if matches!(building.zone_type, ZoneType::Residential | ZoneType::Mixed) {
                housing_capacity += allocator.resident_capacity(idx) as f32;
            }

            if matches!(
                building.zone_type,
                ZoneType::Commercial | ZoneType::Industrial | ZoneType::Office | ZoneType::Mixed
            ) {
                worker_capacity += allocator.worker_capacity(idx) as f32;
                filled_workers += building.worker_count as f32;
            }

            match building.zone_type {
                ZoneType::Commercial | ZoneType::Mixed => {
                    commercial_stock_pressure_sum +=
                        (1.0 - (building.stock / COMMERCIAL_TARGET_STOCK_UNITS).clamp(0.0, 1.0))
                            .clamp(0.0, 1.0);
                    commercial_count += 1.0;
                }
                ZoneType::Industrial => {
                    industrial_stock_pressure_sum +=
                        (1.0 - (building.stock / INDUSTRIAL_TARGET_STOCK_UNITS).clamp(0.0, 1.0))
                            .clamp(0.0, 1.0);
                    industrial_count += 1.0;
                }
                _ => {}
            }
        }

        let resident_count: f32 = households
            .households
            .iter()
            .filter(|household| household.member_count > 0)
            .map(|household| household.member_count as f32)
            .sum();

        let mut household_stock_pressure_sum = 0.0;
        let mut active_households = 0.0;
        for household in &households.households {
            if household.member_count == 0 {
                continue;
            }
            household_stock_pressure_sum +=
                (1.0 - (household.stock_days / HOUSEHOLD_TARGET_STOCK_DAYS).clamp(0.0, 1.0))
                    .clamp(0.0, 1.0);
            active_households += 1.0;
        }

        let household_stock_pressure = average_or_zero(
            household_stock_pressure_sum,
            active_households,
        );
        let commercial_stock_pressure =
            average_or_zero(commercial_stock_pressure_sum, commercial_count);
        let industrial_stock_pressure =
            average_or_zero(industrial_stock_pressure_sum, industrial_count);

        let housing_fill = if housing_capacity > 0.0 {
            (resident_count / housing_capacity).clamp(0.0, 1.0)
        } else if worker_capacity > 0.0 || resident_count > 0.0 {
            1.0
        } else {
            0.0
        };

        let open_job_pressure = if worker_capacity > 0.0 {
            ((worker_capacity - filled_workers).max(0.0) / worker_capacity).clamp(0.0, 1.0)
        } else {
            0.0
        };

        self.residential = to_percent(0.55 * open_job_pressure + 0.45 * housing_fill);
        self.commercial =
            to_percent(0.60 * household_stock_pressure + 0.40 * commercial_stock_pressure);
        self.industrial =
            to_percent(0.65 * commercial_stock_pressure + 0.35 * industrial_stock_pressure);
    }
}

fn average_or_zero(sum: f32, count: f32) -> f32 {
    if count > 0.0 {
        (sum / count).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn to_percent(v: f32) -> f32 {
    v.clamp(0.0, 1.0) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
    use godot::prelude::Vector2;
    use crate::simulation::economy::households::{Household, HouseholdSystem, REPLENISHMENT_STABLE};

    fn building(
        zone_type: ZoneType,
        stock: f32,
        occupancy: u32,
        worker_count: u32,
    ) -> Building {
        Building {
            center_x: 0.0,
            center_y: 0.0,
            width_cells: 2,
            depth_cells: 2,
            zone_type,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 1.0,
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy,
            worker_count,
            asset_id: String::new(),
            level: 1,
            broken: false,
            stock,
            revenue: 0.0,
            operating_budget: 500.0,
            utility_service_available: true,
            shipment_cooldown_days: 0,
        }
    }

    #[test]
    fn recalculate_raises_commercial_and_industrial_pressure_on_shortages() {
        let mut allocator = BuildingAllocator::new();
        allocator.buildings.push(building(ZoneType::Industrial, 20.0, 0, 1));
        allocator.buildings.push(building(ZoneType::Commercial, 80.0, 0, 1));
        allocator.buildings.push(building(ZoneType::Residential, 0.0, 0, 0));

        let mut households = HouseholdSystem::new();
        households.households.push(Household {
            home_building_id: 2,
            budget: 50.0,
            stock: 0.5,
            member_count: 2,
            consumption_rate: 1.0,
            stock_days: 0.25,
            replenishment_state: REPLENISHMENT_STABLE,
            cooldown_days: 0,
        });

        let mut demand = DemandSystem::new();
        demand.recalculate(&allocator, &households);

        assert!(demand.commercial > 60.0);
        assert!(demand.industrial > 60.0);
    }

    #[test]
    fn recalculate_raises_residential_demand_when_jobs_outrun_housing() {
        let mut allocator = BuildingAllocator::new();
        allocator.buildings.push(building(ZoneType::Industrial, 300.0, 0, 1));
        allocator.buildings.push(building(ZoneType::Commercial, 500.0, 0, 1));
        allocator.buildings.push(building(ZoneType::Residential, 0.0, 0, 0));

        let mut households = HouseholdSystem::new();
        for _ in 0..5 {
            households.households.push(Household {
                home_building_id: 2,
                budget: 100.0,
                stock: 3.0,
                member_count: 1,
                consumption_rate: 1.0,
                stock_days: 3.0,
                replenishment_state: REPLENISHMENT_STABLE,
                cooldown_days: 0,
            });
        }

        let mut demand = DemandSystem::new();
        demand.recalculate(&allocator, &households);

        assert!(demand.residential > 50.0);
    }
}
