//! Building placement and lifecycle management.
//!
//! [`BuildingAllocator::tick`] runs once per simulation tick. It:
//! 1. Removes buildings whose zoning cell has been changed or whose road edge was deleted.
//! 2. Rebuilds derived indices and pathing after building mutations.
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
use crate::debug_log;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, ResourceRuntimeId, load_runtime_economy_catalog,
};
use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::VehicleFrontageAccess;
use godot::prelude::Vector2;
use std::collections::HashMap;

/// Shipped baseline private-use families supported by the live zoning-driven runtime.
pub(crate) const BASELINE_PRIVATE_ZONES: [ZoneType; 3] = [
    ZoneType::Residential,
    ZoneType::Commercial,
    ZoneType::Industrial,
];

/// Returns the compact baseline bucket index for one shipped private-use family.
pub(crate) fn baseline_private_zone_slot(zone: ZoneType) -> Option<usize> {
    match zone {
        ZoneType::Residential => Some(0),
        ZoneType::Commercial => Some(1),
        ZoneType::Industrial => Some(2),
        ZoneType::None | ZoneType::Office | ZoneType::Mixed => None,
    }
}

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
    /// Authoritative runtime zoning-profile id captured when this building was placed.
    pub zone_profile_runtime_id: u16,
    /// Cached broad baseline family derived from [`Self::zone_profile_runtime_id`].
    ///
    /// Kept as a hot-path cache for broad R/C/I grouping and economy lookups. Legality still
    /// comes from the authoritative zoning-profile id plus the painted world grid.
    pub zone_type: ZoneType,
    /// Unit vector pointing from the road toward the building.
    pub facing_dir: Vector2,
    /// T-coordinate (0.0 to 1.0) along the road edge [`edge_idx`] for this building's frontage.
    pub frontage_t: f32,
    /// Signed side of the road: `+1.0` = left, `-1.0` = right.
    pub side_offset: f32,
    /// True once this building has entered the permanent deserted state.
    ///
    /// A one-way latch: set by the daily bankruptcy check when budget was negative at end of the
    /// previous day and is still negative at the start of the current day. Never cleared.
    pub is_deserted: bool,
    /// True when this building's budget ended the previous daily settlement below zero.
    ///
    /// Checked at the start of the next daily settlement to declare bankruptcy if still negative.
    pub budget_distress: bool,
    /// Index into [`RegionGraph::edges`] for the road segment this building fronts.
    pub edge_idx: usize,
    /// Road side: `1` = left, `-1` = right.
    pub side: i8,
    /// Column index (along the road) of the building's leading cell.
    pub cell_x: usize,
    /// Depth offset of the building's leading cell (0 = frontage row).
    pub cell_y: u16,
    /// Total households (for residential) or general occupants currently in this building.
    ///
    /// For residential buildings, this is the count of assigned households (family slots),
    /// which must be <= `household_capacity`. Total residents (agents) are tracked
    /// by the AgentSystem referencing these households.
    pub occupancy: u32,
    /// Total workers currently assigned to this building.
    pub worker_count: u32,
    /// Qualified asset ID identifying the model for this building.
    pub asset_id: String,
    /// Current growth tier.
    pub level: u8,
    /// If true, the asset was missing from the registry during load.
    pub broken: bool,
    /// Compact runtime economy-profile id resolved from the asset's authored profile reference.
    pub economy_profile_runtime_id: u16,
    /// True when the asset references an unresolved or unsupported economy profile.
    pub economy_broken: bool,
    /// Current typed on-site inventory by runtime resource id.
    ///
    /// Slot `resource_runtime_id - 1` stores the amount for that resource.
    pub resource_inventory: Vec<f32>,
    /// Lifetime gross revenue collected by this building.
    pub revenue: f32,
    /// Current operating budget available for wages and utility costs.
    ///
    /// May go negative after utility payment; see `budget_distress` and the bankruptcy spec.
    pub operating_budget: f32,
    /// Remaining hourly cooldown steps before this building may open another freight request.
    pub shipment_cooldown_hours: u16,
    /// Currency value of input shipments received from OWA during the current day.
    ///
    /// Reset once per day after the demand snapshot is taken. Read by the demand system to
    /// compute the fraction of commercial input value sourced from OWA vs local industrial.
    pub daily_owa_input_value: f32,
    /// Currency value of input shipments received from local industrial during the current day.
    ///
    /// Reset once per day after the demand snapshot is taken.
    pub daily_local_input_value: f32,
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
    /// Inverted index for shipped baseline families in [`BASELINE_PRIVATE_ZONES`] order.
    pub zone_index: [Vec<usize>; 3],
    /// Inverted vacancy index for shipped residential/commercial/industrial buildings.
    pub vacancy_index: [Vec<usize>; 3],
    /// Position of each building in its respective `vacancy_index` list for O(1) removal.
    pub vacancy_pos: Vec<usize>,
    /// Coarse 512 m chunk index of building centers for bounded nearby-economy queries.
    pub building_chunks: HashMap<(i32, i32), Vec<usize>>,
    /// Recalculates inverted indices if true.
    pub dirty_index: bool,
    /// Per-family dirty flags in [`BASELINE_PRIVATE_ZONES`] order.
    pub dirty_zones: [bool; 3],
    /// True when the derived entrance cache must be rebuilt before use.
    pub(crate) entrances_dirty: bool,
    /// Registry of all loaded pack assets.
    pub registry: AssetRegistry,
    /// Derived building entrance/access cache keyed by building index.
    pub(crate) entrances: Vec<BuildingEntrance>,
}

/// Derived runtime binding from an asset-side `economy_profile` reference.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EconomyProfileBinding {
    /// Compact runtime profile id, or `0` when no runtime profile is bound.
    pub runtime_id: u16,
    /// True when the asset referenced an economy profile that the live runtime cannot execute.
    pub economy_broken: bool,
}

impl Building {
    /// Returns the current inventory amount for one runtime resource.
    pub(crate) fn inventory_units(&self, resource_runtime_id: ResourceRuntimeId) -> f32 {
        if resource_runtime_id == 0 {
            return 0.0;
        }
        self.resource_inventory
            .get(resource_runtime_id as usize - 1)
            .copied()
            .unwrap_or(0.0)
    }

    /// Sets the current inventory amount for one runtime resource.
    pub(crate) fn set_inventory_units(
        &mut self,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        if resource_runtime_id == 0 {
            return;
        }
        let slot = resource_runtime_id as usize - 1;
        if self.resource_inventory.len() <= slot {
            self.resource_inventory.resize(slot + 1, 0.0);
        }
        self.resource_inventory[slot] = amount.max(0.0);
    }

    /// Adds one amount to the current inventory for one runtime resource.
    pub(crate) fn add_inventory_units(
        &mut self,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        let current = self.inventory_units(resource_runtime_id);
        self.set_inventory_units(resource_runtime_id, current + amount);
    }

    /// Removes up to one amount from the current inventory for one runtime resource.
    pub(crate) fn remove_inventory_units(
        &mut self,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        let current = self.inventory_units(resource_runtime_id);
        self.set_inventory_units(resource_runtime_id, (current - amount).max(0.0));
    }

    /// Drops any inventory slots not referenced by the resolved economy profile.
    pub(crate) fn retain_inventory_for_profile(
        &mut self,
        profile: Option<&EconomyProfileRuntime>,
        resource_count: usize,
    ) {
        let mut retained = vec![0.0; resource_count];
        if let Some(profile) = profile {
            for port in profile.inputs.iter().chain(profile.outputs.iter()) {
                let slot = port.resource_runtime_id as usize - 1;
                if slot < self.resource_inventory.len() && slot < retained.len() {
                    retained[slot] = self.resource_inventory[slot];
                }
            }
        }
        self.resource_inventory = retained;
    }
}

/// Resolves the live runtime economy-profile binding for one asset id.
pub(crate) fn resolve_building_economy_profile_binding(
    registry: &AssetRegistry,
    asset_id: &str,
) -> EconomyProfileBinding {
    let Some(profile_id) = registry.economy_profile(asset_id) else {
        return EconomyProfileBinding::default();
    };
    let catalog = match load_runtime_economy_catalog() {
        Ok(catalog) => catalog,
        Err(err) => {
            debug_log!(
                "economy",
                "asset_id={} economy profile '{}' could not be resolved because the runtime catalog failed to load: {}",
                asset_id,
                profile_id,
                err
            );
            return EconomyProfileBinding {
                runtime_id: 0,
                economy_broken: true,
            };
        }
    };
    let Some(profile) = catalog.profile_for_id(profile_id) else {
        debug_log!(
            "economy",
            "asset_id={} references missing economy profile '{}'; building will run economy-broken",
            asset_id,
            profile_id
        );
        return EconomyProfileBinding {
            runtime_id: 0,
            economy_broken: true,
        };
    };
    if !profile.runtime_supported {
        debug_log!(
            "economy",
            "asset_id={} references unsupported runtime economy profile '{}'; building will run economy-broken",
            asset_id,
            profile_id
        );
        return EconomyProfileBinding {
            runtime_id: 0,
            economy_broken: true,
        };
    }
    EconomyProfileBinding {
        runtime_id: profile.runtime_id,
        economy_broken: false,
    }
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

/// Converts a simulation [`ZoneType`] back to the authored [`ZoneClass`] when one exists.
pub(crate) fn zone_type_to_zone_class(zone: ZoneType) -> Option<ZoneClass> {
    match zone {
        ZoneType::Residential => Some(ZoneClass::Residential),
        ZoneType::Commercial => Some(ZoneClass::Commercial),
        ZoneType::Industrial => Some(ZoneClass::Industrial),
        ZoneType::Office => Some(ZoneClass::Office),
        ZoneType::Mixed => Some(ZoneClass::Mixed),
        ZoneType::None => None,
    }
}

impl BuildingAllocator {
    /// Creates an empty allocator.
    pub fn new() -> Self {
        Self {
            buildings: Vec::new(),
            dirty: false,
            edge_occupancy: HashMap::new(),
            zone_index: [const { Vec::new() }; 3],
            vacancy_index: [const { Vec::new() }; 3],
            vacancy_pos: Vec::new(),
            building_chunks: HashMap::new(),
            dirty_index: true,
            dirty_zones: [false; 3],
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
        households: &mut crate::simulation::economy::households::HouseholdSystem,
        logistics: &mut crate::simulation::economy::logistics::ShipmentSystem,
        network: &mut crate::simulation::network::TransitNetwork,
        graph: &mut RegionGraph,
    ) {
        // 1. Stale building cleanup.
        self.cleanup_stale_buildings(
            zoning,
            agents,
            households,
            logistics,
            graph,
            &network.lane_system,
        );

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
        if b.broken || b.economy_broken {
            return 0;
        }
        self.registry.capacity(&b.asset_id)
    }

    /// Returns the household capacity declared by a building asset.
    ///
    /// Unresolved assets or undeclared capacities count as zero.
    pub fn household_capacity(&self, building_idx: usize) -> u32 {
        let Some(b) = self.buildings.get(building_idx) else {
            return 0;
        };
        if b.broken || b.economy_broken {
            return 0;
        }
        self.registry.household_capacity(&b.asset_id)
    }

    /// Resets the daily OWA and local input value accumulators on every building.
    ///
    /// Called once per day after the demand snapshot has been taken, so the next day's
    /// logistics ticks accumulate against a clean baseline.
    pub(crate) fn reset_daily_input_accumulators(&mut self) {
        for building in &mut self.buildings {
            building.daily_owa_input_value = 0.0;
            building.daily_local_input_value = 0.0;
        }
    }

    /// Returns the target floor area per household for a building.
    pub fn flat_size_m2(&self, building_idx: usize) -> f32 {
        let Some(b) = self.buildings.get(building_idx) else {
            return 0.0;
        };
        self.registry.flat_size_m2(&b.asset_id)
    }

    /// Returns the worker capacity for a given asset, preferring the economy profile over the
    /// asset manifest. Falls back to the manifest value if no profile is linked or the profile
    /// has no capacity set.
    pub fn worker_capacity_for_asset(&self, asset_id: &str) -> u32 {
        if let Some(profile_id) = self.registry.economy_profile(asset_id) {
            if let Ok(catalog) = load_runtime_economy_catalog() {
                if let Some(p) = catalog.profile_for_id(profile_id) {
                    if p.worker_capacity > 0 {
                        return p.worker_capacity;
                    }
                }
            }
        }
        self.registry.worker_capacity(asset_id)
    }

    /// Returns the worker capacity declared for a placed building.
    ///
    /// Unresolved assets, broken buildings, and deserted buildings count as zero.
    pub fn worker_capacity(&self, building_idx: usize) -> u32 {
        let Some(b) = self.buildings.get(building_idx) else {
            return 0;
        };
        if b.broken || b.economy_broken || b.is_deserted {
            return 0;
        }
        self.worker_capacity_for_asset(&b.asset_id)
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
