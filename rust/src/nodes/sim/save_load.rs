//! SQLite save/load bridge methods for the Godot simulation node.

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
                graph: &self.region_graph,
                zoning: &self.zoning,
                pollution: &self.pollution,
                noise: &self.noise,
                demand: &self.demand,
                pending_demand_spawns: &self.pending_demand_spawns,
                allocator: &self.allocator,
                households: &self.households,
                logistics: &self.logistics,
                agents: &self.agents,
                network: &self.transit_network,
                treasury: &self.treasury,
            },
        )
        .map_err(|err| err.to_string())
    }

    /// Loads a full simulation snapshot from a SQLite save file and replaces the live world.
    pub(crate) fn load_game_internal(&mut self, path: &str) -> Result<(), String> {
        let loaded = load_from_sqlite(&PathBuf::from(path), &self.allocator.registry)
            .map_err(|err| err.to_string())?;
        self.apply_loaded_simulation(loaded);
        Ok(())
    }

    fn apply_loaded_simulation(&mut self, loaded: LoadedSimulation) {
        self.config = loaded.config;
        self.time = loaded.time;
        self.heightmap = loaded.terrain;
        self.watermap = loaded.water;
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
            .rebuild_building_site_clients(self.zoning.config.zone_cell_m);
        self.households = loaded.households;
        self.logistics = loaded.logistics;
        self.agents = loaded.agents;
        self.treasury = loaded.treasury;
        self.service_policy = Default::default();
        self.budget_history.clear();
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
        self.road_locked_terrain_patch_keys.clear();
        self.road_locked_terrain_patch_margins.clear();
        self.building_site_owned_terrain_patch_keys.clear();
        self.engineered_terrain_patch_keys.clear();
        self.engineered_terrain_patch_margins.clear();
        self.terrain_payload_patch_generations.clear();
        self.refresh_road_locked_terrain_patch_state(
            crate::nodes::sim::core::ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
        );
        self.terrain_dirty = true;
        self.water_dirty = true;
        self.cached_road_mesh_data = None;
        self.mark_network_render_dirty();
    }
}
