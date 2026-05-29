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
        let mut new_allocator = loaded.allocator;
        std::mem::swap(&mut new_allocator.registry, &mut self.allocator.registry);
        self.allocator = new_allocator;
        self.households = loaded.households;
        self.logistics = loaded.logistics;
        self.agents = loaded.agents;
        self.treasury = loaded.treasury;
        self.time.speed_multiplier = 0.0;
        self.transit_network.flow_fields.mark_all_dirty();
        self.undo_stack.clear();
        self.world_water_boundary_points.clear();
        self.world_lake_fills.clear();
        self.world_open_water_fills.clear();
        self.world_lake_fill_preview = None;
        self.terrain_dirty = true;
        self.water_dirty = true;
        self.cached_road_mesh_data = None;
    }
}
