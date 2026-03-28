//! SQLite save/load bridge methods for the Godot simulation node.

use crate::nodes::simulation_node::SimulationNode;
use crate::simulation::save::{LoadedSimulation, SaveGameView, load_from_sqlite, save_to_sqlite};
use std::path::PathBuf;

impl SimulationNode {
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
                agents: &self.agents,
            },
        )
        .map_err(|err| err.to_string())
    }

    /// Loads a full simulation snapshot from a SQLite save file and replaces the live world.
    pub(crate) fn load_game_internal(&mut self, path: &str) -> Result<(), String> {
        let loaded = load_from_sqlite(&PathBuf::from(path)).map_err(|err| err.to_string())?;
        self.apply_loaded_simulation(loaded);
        Ok(())
    }

    fn apply_loaded_simulation(&mut self, loaded: LoadedSimulation) {
        self.config = loaded.config;
        self.time = loaded.time;
        self.time_passed = 0.0;
        self.heightmap = loaded.terrain;
        self.watermap = loaded.water;
        self.region_graph = loaded.graph;
        self.transit_network = loaded.transit_network;
        self.zoning = loaded.zoning;
        self.pollution = loaded.pollution;
        self.noise = loaded.noise;
        self.desirability = loaded.desirability;
        self.demand = loaded.demand;
        self.allocator = loaded.allocator;
        self.agents = loaded.agents;
        self.undo_stack.clear();
        self.terrain_dirty = true;
        self.water_dirty = true;
    }
}
