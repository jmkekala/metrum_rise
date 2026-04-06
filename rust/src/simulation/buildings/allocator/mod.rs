//! Building placement and lifecycle management.
//!
//! [`BuildingAllocator::tick`] runs once per simulation tick. It:
//! 1. Removes buildings whose zoning cell has been changed or whose road edge was deleted.
//! 2. Scans zoned, unoccupied cells with sufficient demand and spawns new buildings.
//! 3. Spawns immigrant agents up to the current residential capacity.

mod placement;
mod lifecycle;
mod index;
mod geometry;

#[cfg(test)]
mod tests;

use crate::assets::{AssetRegistry, ZoneClass};
use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::Vector2;
use std::collections::HashMap;

/// A placed building occupying a variable-footprint area on a zoning grid.
#[derive(Clone)]
pub struct Building {
    /// World-space X centre of the building footprint (metres, ground-plane X axis).
    pub center_x: f32,
    /// World-space Z centre of the building footprint (metres, ground-plane Z axis).
    pub center_y: f32,
    /// Width of the footprint in zoning grid cells.
    pub width_cells: u16,
    /// Depth of the footprint in zoning grid cells.
    pub depth_cells: u16,
    /// Zone category this building was spawned into.
    pub zone_type: ZoneType,
    /// Unit vector pointing from the road toward the building.
    pub facing_dir: Vector2,
    /// T-coordinate (0.0 to 1.0) along the road edge [`edge_idx`] for this building's frontage.
    pub frontage_t: f32,
    /// Signed side of the road: `+1.0` = left, `-1.0` = right.
    pub side_offset: f32,
    /// Ticks since this building lost its zoning. Currently unused.
    pub abandoned_timer: u32,
    /// Index into [`RegionGraph::edges`] for the road segment this building fronts.
    pub edge_idx: usize,
    /// Road side: `1` = left, `-1` = right.
    pub side: i8,
    /// Column index (along the road) of the building's leading cell.
    pub cell_x: usize,
    /// Depth offset of the building's leading cell (0 = frontage row).
    pub cell_y: u16,
    /// Total agents currently residing or working in this building.
    pub occupancy: u32,
    /// Qualified asset ID identifying the model for this building.
    pub asset_id: String,
    /// Current growth tier.
    pub level: u8,
    /// If true, the asset was missing from the registry during load.
    pub broken: bool,
}

/// Manages the full lifecycle of [`Building`]s.
#[derive(Clone)]
pub struct BuildingAllocator {
    /// All currently placed buildings.
    pub buildings: Vec<Building>,
    /// Set to `true` when the building list changes, signalling renderers to refresh.
    pub dirty: bool,
    /// Per-edge frontage occupancy tracker.
    pub edge_occupancy: HashMap<usize, EdgeOccupancy>,
    /// Inverted index: `zone_index[ZoneType as usize]` contains building indices.
    pub zone_index: [Vec<usize>; 6],
    /// Inverted index: buildings with occupancy below capacity.
    pub vacancy_index: [Vec<usize>; 6],
    /// Position of each building in its respective `vacancy_index` list for O(1) removal.
    pub vacancy_pos: Vec<usize>,
    /// Recalculates inverted indices if true.
    pub dirty_index: bool,
    /// Per-zone dirty flags set when buildings are spawned or removed.
    pub dirty_zones: [bool; 6],
    /// Registry of all loaded pack assets.
    pub registry: AssetRegistry,
}

/// Tracks which frontage columns along a road edge are claimed by placed buildings.
#[derive(Clone)]
pub struct EdgeOccupancy {
    /// Number of columns along this road edge.
    pub cells_long: usize,
    /// True if a building has its frontage in this column on the left side.
    pub left: Vec<bool>,
    /// True if a building has its frontage in this column on the right side.
    pub right: Vec<bool>,
}

/// Returns the road node an agent departing from `building` should walk toward.
pub(crate) fn building_depart_node(building: &Building, graph: &RegionGraph) -> u32 {
    let edge = graph.edge(building.edge_idx);
    if building.frontage_t < 0.5 {
        edge.start_node
    } else {
        edge.end_node
    }
}

/// Converts a simulation [`ZoneType`] to the corresponding [`ZoneClass`].
pub(crate) fn zone_type_to_zone_class(zt: ZoneType) -> Option<ZoneClass> {
    match zt {
        ZoneType::Residential => Some(ZoneClass::Residential),
        ZoneType::Commercial  => Some(ZoneClass::Commercial),
        ZoneType::Industrial  => Some(ZoneClass::Industrial),
        ZoneType::Office      => Some(ZoneClass::Office),
        ZoneType::Mixed       => Some(ZoneClass::Mixed),
        ZoneType::None        => None,
    }
}

impl BuildingAllocator {
    /// Creates an empty allocator.
    pub fn new() -> Self {
        Self {
            buildings: Vec::new(),
            dirty: false,
            edge_occupancy: HashMap::new(),
            zone_index: [const { Vec::new() }; 6],
            vacancy_index: [const { Vec::new() }; 6],
            vacancy_pos: Vec::new(),
            dirty_index: true,
            dirty_zones: [false; 6],
            registry: AssetRegistry::new(),
        }
    }

    /// Advances the building lifecycle by one simulation tick.
    pub fn tick(
        &mut self,
        demand: &mut crate::simulation::economy::demand::DemandSystem,
        zoning: &mut ZoningSystem,
        desirability: &crate::simulation::grid::desirability::DesirabilitySystem,
        _noise: &crate::simulation::grid::noise::NoiseSystem,
        agents: &mut crate::simulation::economy::agents::AgentSystem,
        network: &mut crate::simulation::network::TransitNetwork,
        graph: &mut RegionGraph,
        config: &crate::simulation::core::config::MapConfig,
    ) {
        // 1. Stale building cleanup.
        self.cleanup_stale_buildings(zoning, agents, graph);

        // 2. Growth logic.
        self.spawn_new_buildings(demand, zoning, desirability, graph, config);

        // 3. Upgrade pass.
        self.update_building_levels(demand, desirability, config);

        network.rebuild_pathing_if_dirty(graph);

        if self.dirty_index {
            self.rebuild_zone_index();
        }

        // 4. Immigration logic.
        self.spawn_immigrants(demand.residential, agents, graph);

        self.dirty = false;
    }

    /// Remaps all building edge indices after a road network compaction.
    pub fn update_edge_indices(&mut self, mapping: &HashMap<usize, usize>) {
        let old_len = self.buildings.len();
        for b in &mut self.buildings {
            if let Some(&new_id) = mapping.get(&b.edge_idx) {
                b.edge_idx = new_id;
            } else {
                b.edge_idx = usize::MAX;
            }
        }
        self.buildings.retain(|b| b.edge_idx != usize::MAX);
        if self.buildings.len() != old_len {
            self.dirty = true;
            self.dirty_index = true;
        }
        let mut new_occ = HashMap::new();
        for (old_idx, occ) in self.edge_occupancy.drain() {
            if let Some(&new_id) = mapping.get(&old_idx) {
                new_occ.insert(new_id, occ);
            }
        }
        self.edge_occupancy = new_occ;
    }

    /// Removes all buildings and resets the dirty flag.
    pub fn clear(&mut self) {
        self.buildings.clear();
        self.edge_occupancy.clear();
        for list in &mut self.zone_index {
            list.clear();
        }
        for list in &mut self.vacancy_index {
            list.clear();
        }
        self.vacancy_pos.clear();
        self.dirty = false;
        self.dirty_index = false;
    }

    /// Returns the occupant capacity for a building, from its registered manifest.
    pub fn building_capacity(&self, building_idx: usize) -> u32 {
        let b = &self.buildings[building_idx];
        if b.broken { return 0; }
        let cap = self.registry.capacity(&b.asset_id);
        if cap == 0 { 6 } else { cap }
    }
}
