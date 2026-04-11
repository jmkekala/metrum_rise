//! Building placement and lifecycle management.
//!
//! [`BuildingAllocator::tick`] runs once per simulation tick. It:
//! 1. Removes buildings whose zoning cell has been changed or whose road edge was deleted.
//! 2. Runs the one-time founding bootstrap when the player has provided valid
//!    startup zoning and an external connection.
//! 3. Rebuilds derived indices and pathing after building mutations.
//!
//! Demand-owned household admission is executed separately after the daily economy settlement and
//! daily demand pass; allocator tick no longer recomputes immigration pressure locally.

mod entrance;
mod geometry;
mod index;
mod lifecycle;
mod placement;

#[cfg(test)]
mod tests;

use crate::assets::{AssetRegistry, ZoneClass};
use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::VehicleFrontageAccess;
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
    ///
    /// In the current foundation slice this is the residential occupant count
    /// used for housing capacity and household anchoring.
    pub occupancy: u32,
    /// Total workers currently assigned to this building.
    pub worker_count: u32,
    /// Qualified asset ID identifying the model for this building.
    pub asset_id: String,
    /// Current growth tier.
    pub level: u8,
    /// If true, the asset was missing from the registry during load.
    pub broken: bool,
    /// Current on-site stock buffer for the first-pass economy loop.
    pub stock: f32,
    /// Lifetime gross revenue collected by this building.
    pub revenue: f32,
    /// Current operating budget available for wages, utility fallback, and imports.
    pub operating_budget: f32,
    /// Whether the building has resolved utility availability for the current daily pass.
    pub utility_service_available: bool,
    /// Remaining daily cooldown steps before this building may open another freight request.
    pub shipment_cooldown_days: u8,
    /// True when the current painted zoning profile is incompatible and the building is waiting
    /// for the rezoning grace timer to expire.
    pub pending_redevelopment: bool,
    /// Remaining deterministic daily grace before incompatible rezoning forces removal.
    pub rezone_grace_days_remaining: u8,
}

#[derive(Clone)]
pub(crate) struct BuildingEntrance {
    pub edge_idx: usize,
    #[allow(dead_code)]
    pub side: i8,
    pub vehicle_frontage_access: VehicleFrontageAccess,
    pub entrance_s_m: f32,
    pub door_pos: Vector2,
    pub curb_pos: Vector2,
    pub foot_lane_fwd: usize,
    pub foot_lane_bkw: usize,
    pub car_lane_fwd: usize,
    pub car_lane_bkw: usize,
    pub flags: u8,
}

impl Default for BuildingEntrance {
    fn default() -> Self {
        Self {
            edge_idx: usize::MAX,
            side: 0,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
            entrance_s_m: 0.0,
            door_pos: Vector2::ZERO,
            curb_pos: Vector2::ZERO,
            foot_lane_fwd: usize::MAX,
            foot_lane_bkw: usize::MAX,
            car_lane_fwd: usize::MAX,
            car_lane_bkw: usize::MAX,
            flags: 0,
        }
    }
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
    /// Coarse 512 m chunk index of building centers for bounded nearby-economy queries.
    pub building_chunks: HashMap<(i32, i32), Vec<usize>>,
    /// Recalculates inverted indices if true.
    pub dirty_index: bool,
    /// Per-zone dirty flags set when buildings are spawned or removed.
    pub dirty_zones: [bool; 6],
    /// Prevents the one-time founding bootstrap from running more than once.
    pub founding_bootstrap_consumed: bool,
    /// True when the derived entrance cache must be rebuilt before use.
    pub(crate) entrances_dirty: bool,
    /// Registry of all loaded pack assets.
    pub registry: AssetRegistry,
    /// Derived building entrance/access cache keyed by building index.
    pub(crate) entrances: Vec<BuildingEntrance>,
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

/// Converts an asset-manifest [`ZoneClass`] to the matching simulation [`ZoneType`].
pub(crate) fn zone_class_to_zone_type(zone: ZoneClass) -> ZoneType {
    match zone {
        ZoneClass::Residential => ZoneType::Residential,
        ZoneClass::Commercial => ZoneType::Commercial,
        ZoneClass::Industrial => ZoneType::Industrial,
        ZoneClass::Office => ZoneType::Office,
        ZoneClass::Mixed => ZoneType::Mixed,
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
            building_chunks: HashMap::new(),
            dirty_index: true,
            dirty_zones: [false; 6],
            founding_bootstrap_consumed: false,
            entrances_dirty: false,
            registry: AssetRegistry::new(),
            entrances: Vec::new(),
        }
    }

    /// Advances the building lifecycle by one simulation tick.
    pub fn tick(
        &mut self,
        zoning: &mut ZoningSystem,
        agents: &mut crate::simulation::economy::agents::AgentSystem,
        _households: &mut crate::simulation::economy::households::HouseholdSystem,
        logistics: &mut crate::simulation::economy::logistics::ShipmentSystem,
        network: &mut crate::simulation::network::TransitNetwork,
        graph: &mut RegionGraph,
    ) {
        // 1. Stale building cleanup.
        self.cleanup_stale_buildings(zoning, agents, logistics, graph, &network.lane_system);

        // 2. One-time founding bootstrap from zoning + border connection.
        self.place_founding_bootstrap_if_ready(zoning, graph, &network.lane_system);

        network.rebuild_pathing_if_dirty(graph);

        if self.entrances_dirty || self.entrances.len() != self.buildings.len() {
            self.rebuild_entrance_cache(graph, &network.lane_system);
        }

        if self.dirty_index {
            self.rebuild_zone_index();
        }

        self.dirty = false;
    }

    pub(crate) fn execute_demand_household_admission(
        &mut self,
        households_to_admit_today: u32,
        agents: &mut crate::simulation::economy::agents::AgentSystem,
        households: &mut crate::simulation::economy::households::HouseholdSystem,
    ) {
        self.admit_households_from_demand(households_to_admit_today as usize, agents, households);
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
        self.entrances.clear();
        self.entrances_dirty = true;
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
        self.building_chunks.clear();
        self.founding_bootstrap_consumed = false;
        self.dirty = false;
        self.dirty_index = false;
        self.entrances.clear();
        self.entrances_dirty = false;
    }

    /// Returns the occupant capacity for a building, from its registered manifest.
    ///
    /// Unresolved assets or undeclared capacities count as zero.
    pub fn building_capacity(&self, building_idx: usize) -> u32 {
        let b = &self.buildings[building_idx];
        if b.broken {
            return 0;
        }
        self.registry.capacity(&b.asset_id)
    }

    /// Returns the residential capacity declared by a building asset.
    ///
    /// Unresolved assets or undeclared capacities count as zero.
    pub fn resident_capacity(&self, building_idx: usize) -> u32 {
        let Some(b) = self.buildings.get(building_idx) else {
            return 0;
        };
        if b.broken {
            return 0;
        }
        self.registry
            .get(&b.asset_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .and_then(|building| building.residents_capacity)
            .unwrap_or(0)
    }

    /// Returns the worker capacity declared by a building asset.
    ///
    /// Unresolved assets or undeclared capacities count as zero.
    pub fn worker_capacity(&self, building_idx: usize) -> u32 {
        let Some(b) = self.buildings.get(building_idx) else {
            return 0;
        };
        if b.broken {
            return 0;
        }
        self.registry
            .get(&b.asset_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .and_then(|building| building.worker_capacity)
            .unwrap_or(0)
    }

    /// Returns a bounded nearby candidate list for the requested zones, sorted by distance.
    pub fn find_nearby_buildings_by_zones(
        &self,
        origin_x: f32,
        origin_y: f32,
        zones: &[ZoneType],
        max_chunk_radius: i32,
        candidate_limit: usize,
    ) -> Vec<usize> {
        let mut candidates = Vec::with_capacity(candidate_limit);
        let origin_chunk =
            RegionGraph::get_chunk_coords(godot::prelude::Vector3::new(origin_x, 0.0, origin_y));

        'rings: for ring in 0..=max_chunk_radius {
            for dx in -ring..=ring {
                for dz in -ring..=ring {
                    if ring > 0 && dx.abs() != ring && dz.abs() != ring {
                        continue;
                    }
                    let chunk_key = (origin_chunk.0 + dx, origin_chunk.1 + dz);
                    let Some(indices) = self.building_chunks.get(&chunk_key) else {
                        continue;
                    };
                    for &idx in indices {
                        if idx >= self.buildings.len() {
                            continue;
                        }
                        if zones.contains(&self.buildings[idx].zone_type) {
                            candidates.push(idx);
                            if candidates.len() >= candidate_limit {
                                break 'rings;
                            }
                        }
                    }
                }
            }
        }

        candidates.sort_unstable_by(|&a, &b| {
            let da = squared_distance(origin_x, origin_y, &self.buildings[a]);
            let db = squared_distance(origin_x, origin_y, &self.buildings[b]);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates.truncate(candidate_limit);
        candidates
    }
}

fn squared_distance(origin_x: f32, origin_y: f32, building: &Building) -> f32 {
    let dx = building.center_x - origin_x;
    let dy = building.center_y - origin_y;
    dx * dx + dy * dy
}
