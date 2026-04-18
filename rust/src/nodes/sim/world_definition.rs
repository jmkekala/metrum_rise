//! Blank-world creation and reusable world-definition bridge methods.

use crate::nodes::sim::core::{CityTreasury, SimCore};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::load_runtime_economy_tuning;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::zoning::ZoningSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use crate::simulation::world_definition::{
    LoadedWorldDefinition, WorldDefinitionView, load_world_definition_from_sqlite,
    save_world_definition_to_sqlite,
};
use std::path::PathBuf;

impl SimCore {
    /// Resets the live runtime to one fresh blank world with the given terrain settings.
    pub(crate) fn create_blank_world_internal(
        &mut self,
        width_m: f32,
        height_m: f32,
        terrain_cell_m: f32,
        terrain_chunk_m: f32,
        base_elevation_m: f32,
    ) -> Result<(), String> {
        validate_positive_f32(width_m, "width_m")?;
        validate_positive_f32(height_m, "height_m")?;
        validate_positive_f32(terrain_cell_m, "terrain_cell_m")?;
        validate_positive_f32(terrain_chunk_m, "terrain_chunk_m")?;
        if !base_elevation_m.is_finite() {
            return Err("base_elevation_m must be finite".to_owned());
        }

        let config = WorldConfig::new(width_m, height_m, self.config.env_cell_m, self.config.zone_cell_m)
            .with_terrain_resolution(terrain_cell_m)
            .with_chunking(terrain_chunk_m, base_elevation_m);
        let terrain = TerrainSystem::from_world_config(&config);
        self.reset_to_blank_world_runtime(config, terrain);
        Ok(())
    }

    /// Saves the current authored world state as a reusable world-definition asset.
    pub(crate) fn save_world_definition_internal(&self, path: &str, name: &str) -> Result<(), String> {
        save_world_definition_to_sqlite(
            &PathBuf::from(path),
            WorldDefinitionView {
                name,
                config: &self.config,
                terrain: &self.heightmap,
            },
        )
        .map_err(|err| err.to_string())
    }

    /// Loads one reusable world-definition asset and resets runtime state to a fresh blank city.
    pub(crate) fn load_world_definition_internal(&mut self, path: &str) -> Result<(), String> {
        let loaded =
            load_world_definition_from_sqlite(&PathBuf::from(path)).map_err(|err| err.to_string())?;
        self.apply_loaded_world_definition(loaded);
        Ok(())
    }

    fn apply_loaded_world_definition(&mut self, loaded: LoadedWorldDefinition) {
        let LoadedWorldDefinition {
            name: _name,
            config,
            terrain,
        } = loaded;
        self.reset_to_blank_world_runtime(config, terrain);
    }

    fn reset_to_blank_world_runtime(&mut self, config: WorldConfig, terrain: TerrainSystem) {
        let registry = self.allocator.registry.clone();

        self.time = TimeSystem::new();
        self.config = config;
        self.heightmap = terrain;
        self.watermap = WaterSystem::from_world_config(&self.config);
        self.region_graph = crate::simulation::network::graph::RegionGraph::new();
        self.transit_network = TransitNetwork::new();
        self.zoning = ZoningSystem::new(&self.config);
        self.pollution = PollutionSystem::new(&self.config);
        self.noise = NoiseSystem::new(&self.config);
        self.desirability = DesirabilitySystem::new(&self.config);
        self.demand = DemandSystem::new();
        self.agents = AgentSystem::new();
        self.households = HouseholdSystem::new();
        self.logistics = ShipmentSystem::new();

        let mut allocator = BuildingAllocator::new();
        allocator.registry = registry;
        self.allocator = allocator;

        self.treasury = CityTreasury::new(startup_treasury_balance());
        self.undo_stack.clear();
        self.terrain_dirty = true;
        self.water_dirty = true;
        self.network_dirty = true;
        self.last_tick_duration = 0.0;
        self.last_agent_tick_us = 0;
        self.last_road_timing.clear();
        self.camera_aabb = (0.0, 0.0, 0.0, 0.0);
    }
}

fn startup_treasury_balance() -> f64 {
    load_runtime_economy_tuning()
        .map(|tuning| tuning.startup_treasury_balance)
        .unwrap_or(100_000.0)
}

fn validate_positive_f32(value: f32, label: &str) -> Result<(), String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{label} must be finite and > 0"));
    }
    Ok(())
}
