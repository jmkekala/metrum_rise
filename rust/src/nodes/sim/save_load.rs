// SPDX-License-Identifier: GPL-2.0-only

//! SQLite save/load bridge methods for the Godot simulation node.

use crate::debug_log;
use crate::nodes::sim::core::SimCore;
use crate::simulation::save::{
    LoadedSimulation, SaveGameView, SavedCameraState, load_from_sqlite, save_to_sqlite,
};
use crate::simulation::vegetation::VegetationGenerator;
use std::path::PathBuf;

impl SimCore {
    /// Saves the current simulation state into a single-file SQLite snapshot.
    pub(crate) fn save_game_internal(
        &self,
        path: &str,
        camera: Option<SavedCameraState>,
    ) -> Result<(), String> {
        let path = PathBuf::from(path);
        save_to_sqlite(
            &path,
            SaveGameView {
                camera,
                config: &self.config,
                vegetation: &self.vegetation.config,
                vegetation_edits: &self.vegetation_edits,
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
                agriculture: &self.agriculture,
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
    pub(crate) fn load_game_internal(
        &mut self,
        path: &str,
    ) -> Result<Option<SavedCameraState>, String> {
        let loaded = load_from_sqlite(&PathBuf::from(path), &self.allocator.registry)
            .map_err(|err| err.to_string())?;
        let camera = loaded.camera;
        self.apply_loaded_simulation(loaded);
        Ok(camera)
    }

    fn apply_loaded_simulation(&mut self, loaded: LoadedSimulation) {
        self.config = loaded.config;
        self.vegetation = VegetationGenerator::resolve(loaded.vegetation, &self.config);
        self.vegetation_edits = loaded.vegetation_edits;
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
        self.zoning.invalidate_cell_lot_assets();
        // Transforms and entrances were restored before agent lane references.
        self.allocator
            .rebuild_building_site_clients(self.zoning.config.zone_cell_m);
        self.households = loaded.households;
        self.logistics = loaded.logistics;
        self.resource_extraction = loaded.resource_extraction;
        self.agriculture = loaded.agriculture;
        self.agents = loaded.agents;
        self.treasury = loaded.treasury;
        self.service_policy = loaded.service_policy;
        self.fiscal_policy = loaded.fiscal_policy;
        self.apply_service_funding_staffing_policy();
        self.refresh_demand_construction_queue(self.time.day_index, self.time.minute_of_day);
        self.refresh_loaded_demand_state_and_log();
        self.budget_history = loaded.budget_history;
        self.budget_last_lifetime_build_cost = self.treasury.lifetime_build_cost;
        self.debug_household_admissions_since_daily = 0;
        self.time.speed_multiplier = 0.0;
        self.transit_network.flow_fields.mark_all_dirty();
        self.clear_world_transient_state();
        self.heightmap.mark_all_render_patches_dirty();
        self.refresh_all_engineered_terrain_patch_state(
            crate::nodes::sim::core::ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
        );
        self.terrain_dirty = true;
        self.water_dirty = true;
        self.mark_network_render_dirty();
        // Loading advances the render-facing road generation after the persisted
        // surface was rebuilt. Stamp a matching mesh generation before water and
        // refined-terrain payload workers consume the refreshed snapshot.
        self.precompute_road_mesh_data();
    }

    fn refresh_loaded_demand_state_and_log(&mut self) {
        self.demand
            .set_admission_candidate_hour(self.time.day_index, self.time.minute_of_day);
        self.demand.refresh_household_diagnostics(
            &self.allocator,
            &self.households,
            &self.region_graph,
            self.treasury.balance,
            &self.fiscal_policy,
        );
        if !crate::debug::category_enabled("spawn") {
            return;
        }
        let admission = self.demand.last_admission_debug_summary();
        debug_log!(
            "spawn",
            "load growth diagnostics: admit={} service_funding=electricity:{:.2} buildings={} households={} vacant_slots={} open_jobs={} move_in_jobs={} incoming_need={:.2}",
            self.demand.households_to_admit_today,
            self.service_policy.electricity_funding,
            self.allocator.buildings.len(),
            self.households
                .households
                .iter()
                .filter(|household| household.member_count > 0)
                .count(),
            admission.vacant_household_slots,
            admission.open_job_slots,
            admission.move_in_job_slots,
            admission.incoming_household_need,
        );
    }
}
