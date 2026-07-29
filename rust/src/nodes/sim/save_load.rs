//! SQLite save/load bridge methods for the Godot simulation node.

use crate::debug_log;
use crate::nodes::sim::core::SimCore;
use crate::simulation::save::{LoadedSimulation, SaveGameView, load_from_sqlite, save_to_sqlite};
use std::path::PathBuf;

impl SimCore {
    /// Saves the current simulation state into a single-file SQLite snapshot.
    pub(crate) fn save_game_internal(&self, path: &str) -> Result<(), String> {
        let path = PathBuf::from(path);
        save_to_sqlite(
            &path,
            SaveGameView {
                config: &self.config,
                time: &self.time,
                terrain: &self.heightmap,
                water: &self.watermap,
                resource_deposits: &self.resource_deposits,
                graph: &self.region_graph,
                zoning: &self.zoning,
                pollution: &self.pollution,
                noise: &self.noise,
                demand: &self.demand,
                pending_demand_spawns: &self.pending_demand_spawns,
                allocator: &self.allocator,
                households: &self.households,
                logistics: &self.logistics,
                resource_extraction: &self.resource_extraction,
                agents: &self.agents,
                network: &self.transit_network,
                treasury: &self.treasury,
                service_policy: &self.service_policy,
                fiscal_policy: &self.fiscal_policy,
                budget_history: &self.budget_history,
            },
        )
        .map_err(|err| err.to_string())
    }

    /// Loads a full simulation snapshot from a SQLite save file and replaces the live world.
    pub(crate) fn load_game_internal(&mut self, path: &str) -> Result<(), String> {
        let loaded = load_from_sqlite(&PathBuf::from(path), &self.allocator.registry)
            .map_err(|err| err.to_string())?;
        self.apply_loaded_simulation(loaded)?;
        Ok(())
    }

    fn apply_loaded_simulation(&mut self, loaded: LoadedSimulation) -> Result<(), String> {
        self.config = loaded.config;
        self.time = loaded.time;
        self.heightmap = loaded.terrain;
        self.watermap = loaded.water;
        self.resource_deposits = loaded.resource_deposits;
        self.region_graph = loaded.graph;
        self.transit_network = loaded.transit_network;
        self.zoning = loaded.zoning;
        self.pollution = loaded.pollution;
        self.noise = loaded.noise;
        self.desirability = loaded.desirability;
        self.demand = loaded.demand;
        self.pending_demand_spawns = loaded.pending_demand_spawns;
        let mut new_allocator = loaded.allocator;
        std::mem::swap(&mut new_allocator.registry, &mut self.allocator.registry);
        self.allocator = new_allocator;
        self.allocator
            .recompute_derived_transforms(&self.region_graph, &self.zoning)?;
        self.allocator
            .rebuild_entrance_cache(&self.region_graph, &self.transit_network.lane_system);
        self.allocator
            .rebuild_building_site_clients(self.zoning.config.zone_cell_m);
        self.households = loaded.households;
        self.logistics = loaded.logistics;
        self.resource_extraction = loaded.resource_extraction;
        self.agents = loaded.agents;
        self.treasury = loaded.treasury;
        self.service_policy = loaded.service_policy;
        self.fiscal_policy = loaded.fiscal_policy;
        self.apply_service_funding_staffing_policy();
        self.refresh_loaded_demand_state_and_log();
        self.budget_history = loaded.budget_history;
        self.budget_last_lifetime_build_cost = self.treasury.lifetime_build_cost;
        self.debug_household_admissions_since_daily = 0;
        self.time.speed_multiplier = 0.0;
        self.transit_network.flow_fields.mark_all_dirty();
        self.undo_stack.clear();
        self.world_lake_fills.clear();
        self.world_open_water_fills.clear();
        self.world_lake_fill_preview = None;
        self.authored_water_patch_fill_debug_cache.clear();
        self.refined_terrain_patch_cache.clear();
        self.refined_terrain_assembly_ledgers.clear();
        self.road_locked_terrain_patch_keys.clear();
        self.road_locked_terrain_patch_margins.clear();
        self.building_site_owned_terrain_patch_keys.clear();
        self.engineered_terrain_patch_keys.clear();
        self.engineered_terrain_patch_margins.clear();
        self.terrain_payload_patch_generations.clear();
        self.heightmap.mark_all_render_patches_dirty();
        self.bump_global_terrain_payload_generation();
        self.refresh_all_engineered_terrain_patch_state(
            crate::nodes::sim::core::ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
        );
        self.terrain_dirty = true;
        self.water_dirty = true;
        self.cached_road_mesh_data = None;
        self.mark_network_render_dirty();
        // Loading advances the render-facing road generation after the persisted
        // surface was rebuilt. Stamp a matching mesh generation before water and
        // refined-terrain payload workers consume the refreshed snapshot.
        self.precompute_road_mesh_data();
        Ok(())
    }

    fn refresh_loaded_demand_state_and_log(&mut self) {
        let persisted_residential = self.demand.net_residential_pressure();
        let persisted_commercial = self.demand.net_commercial_pressure();
        let persisted_industrial = self.demand.net_industrial_pressure();
        let persisted_households_to_admit = self.demand.households_to_admit_today;
        if !crate::debug::category_enabled("spawn") {
            let service_funding_by_building = self.electricity_funding_by_building();
            self.demand.refresh_pressure_channels_with_service_funding(
                &self.allocator,
                &self.households,
                &self.region_graph,
                self.treasury.balance,
                &service_funding_by_building,
                &self.fiscal_policy,
            );
            return;
        }
        let service_funding_by_building = self.electricity_funding_by_building();
        self.demand.refresh_pressure_channels_with_service_funding(
            &self.allocator,
            &self.households,
            &self.region_graph,
            self.treasury.balance,
            &service_funding_by_building,
            &self.fiscal_policy,
        );
        let (
            vacant_household_slots,
            open_job_slots,
            move_in_job_slots,
            move_in_job_equivalent_slots,
            regional_growth_household_pull,
            open_job_household_pull,
            marginal_commercial_job_household_pull,
            incoming_household_need,
            move_in_acceptance,
            construction_move_in_acceptance,
            failure_factor,
        ) = self.demand.last_admission_debug_summary();
        debug_log!(
            "spawn",
            "load demand refresh: persisted=(R {:+.0}%, C {:+.0}%, I {:+.0}%, admit={}) refreshed=(R {:+.0}%, C {:+.0}%, I {:+.0}%, admit={}) service_funding=electricity:{:.2} buildings={} households={} vacant_slots={} open_jobs={} move_in_jobs={} move_in_job_equiv={:.2} regional_pull={:.2} job_pull={:.2} marginal_com_pull={:.2} incoming_need={:.2} move_in={:.2} construction_move_in={:.2} failure={:.2}",
            persisted_residential * 100.0,
            persisted_commercial * 100.0,
            persisted_industrial * 100.0,
            persisted_households_to_admit,
            self.demand.net_residential_pressure() * 100.0,
            self.demand.net_commercial_pressure() * 100.0,
            self.demand.net_industrial_pressure() * 100.0,
            self.demand.households_to_admit_today,
            self.service_policy.electricity_funding,
            self.allocator.buildings.len(),
            self.households
                .households
                .iter()
                .filter(|household| household.member_count > 0)
                .count(),
            vacant_household_slots,
            open_job_slots,
            move_in_job_slots,
            move_in_job_equivalent_slots,
            regional_growth_household_pull,
            open_job_household_pull,
            marginal_commercial_job_household_pull,
            incoming_household_need,
            move_in_acceptance,
            construction_move_in_acceptance,
            failure_factor,
        );
    }
}
