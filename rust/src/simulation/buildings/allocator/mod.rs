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
mod site;

#[cfg(test)]
mod tests;

pub(crate) use placement::ExplicitServicePlacementPreview;
pub(crate) use site::BuildingSiteGradingRequest;

use crate::assets::{AssetRegistry, ZoneClass};
use crate::debug_log;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, ResourceRuntimeId, RuntimeEconomyCatalog, load_runtime_economy_catalog,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::VehicleFrontageAccess;
use crate::simulation::zoning::{ZoneType, ZoningSystem};
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

/// Final allocator-side reason a selected demand spawn could not be committed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DemandSpawnPlacementRejection {
    /// The selected asset no longer resolves to valid placement parameters.
    AssetUnavailable,
    /// The selected parcel no longer exists in the zoning system.
    ParcelUnavailable,
    /// The parcel exists, but its current geometry or occupancy no longer accepts the asset.
    SlotUnavailable,
    /// A driveway anchor could not resolve an adjacent road surface height.
    DrivewayRoadSurfaceMissing,
    /// Multiple driveway anchors required incompatible flat-site heights.
    DrivewayHeightConflict,
    /// The asset has driveway anchors, but none touch the claimed road edge.
    DrivewayConnectionMissing,
    /// The frontage fallback could not resolve an adjacent road surface height.
    FrontageRoadSurfaceMissing,
    /// The selected flat-site height conflicts with an already placed neighboring site.
    NeighborSiteHeightConflict,
    /// The flat support footprint cannot tie into surrounding terrain or roads within slope limits.
    SiteSupportTieInInvalid,
}

/// Final allocator-side reason an explicit service placement could not be committed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExplicitServicePlacementRejection {
    /// The selected asset is not present in the loaded registry.
    AssetUnavailable,
    /// The selected asset is not an explicit service-building asset.
    NotServiceBuilding,
    /// The selected asset is not an explicit resource-extractor building.
    NotIndustryBuilding,
    /// The selected utility asset references an unsupported or missing runtime profile.
    UtilityProfileUnavailable,
    /// The selected extractor asset references an unsupported or mismatched runtime profile.
    ExtractorProfileUnavailable,
    /// No road frontage near the requested point can accept the building footprint.
    RoadFrontageUnavailable,
    /// A driveway anchor could not resolve an adjacent road surface height.
    DrivewayRoadSurfaceMissing,
    /// Multiple driveway anchors required incompatible flat-site heights.
    DrivewayHeightConflict,
    /// The asset has driveway anchors, but none touch the claimed road edge.
    DrivewayConnectionMissing,
    /// The frontage fallback could not resolve an adjacent road surface height.
    FrontageRoadSurfaceMissing,
    /// The selected flat-site height conflicts with an already placed neighboring site.
    NeighborSiteHeightConflict,
    /// The flat support footprint cannot tie into surrounding terrain or roads within slope limits.
    SiteSupportTieInInvalid,
    /// The selected footprint overlaps an existing building site.
    SiteOverlap,
    /// The selected footprint overlaps an existing road corridor.
    RoadOverlap,
}

/// A placed building occupying one authored parcel or an explicit non-zoned site.
#[derive(Clone)]
pub struct Building {
    /// World-space X centre of the building footprint (metres, ground-plane X axis).
    pub center_x: f32,
    /// World-space Z centre of the building footprint (metres, ground-plane Z axis).
    pub center_y: f32,
    /// Fixed support-surface height captured when the building was placed.
    ///
    /// Rendered building parts use this instead of re-sampling terrain after placement.
    pub support_height_m: f32,
    /// Width of the footprint in zoning cells.
    pub width_cells: u16,
    /// Depth of the footprint in zoning cells.
    pub depth_cells: u16,
    /// Authoritative runtime zoning-profile id captured when this building was placed.
    pub zone_profile_runtime_id: u16,
    /// Stable authored parcel id claimed by this private zoned building; `0` means no parcel.
    pub parcel_id: u64,
    /// Cached broad baseline family derived from [`Self::zone_profile_runtime_id`].
    ///
    /// Kept as a hot-path cache for broad R/C/I grouping and economy lookups. Legality comes
    /// from the parcel's authoritative zoning-profile id.
    pub zone_type: ZoneType,
    /// Unit vector pointing from the building frontage toward the road.
    pub facing_dir: Vector2,
    /// T-coordinate (0.0 to 1.0) along [`Self::edge_idx`] for this building's frontage.
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
    /// Per-building service funding override in `0.0..=1.0`; negative means inherit city policy.
    pub service_funding_override: f32,
    /// Qualified asset ID identifying the model for this building.
    pub asset_id: String,
    /// Current growth tier.
    pub level: u8,
    /// Authored duration of the current construction task in operational hours.
    ///
    /// `0` means this building was placed complete or has already finished construction.
    pub construction_total_hours: u16,
    /// Remaining operational hours before this building becomes live economy capacity.
    pub construction_remaining_hours: u16,
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
    /// Operating-budget baseline captured after the most recent daily profit-tax settlement.
    pub profit_tax_budget_baseline: f32,
    /// Completed previous-day operating-budget delta captured before the daily baseline reset.
    pub last_day_profit: f32,
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
    /// City treasury-funded input purchases committed during the current day.
    ///
    /// Reset once per day after the demand snapshot is taken. This is separate from
    /// received-input counters because treasury-backed service purchases are paid at shipment
    /// reservation time, before the cargo may arrive.
    pub daily_city_funded_input_cost: f32,
    /// Net sales revenue collected from household shopping during the current day.
    ///
    /// Rolled into [`Self::recent_household_sales_value`] at the daily economy reset.
    pub daily_household_sales_value: f32,
    /// Aggregate utility power service units produced during the current day.
    ///
    /// Daily utility settlement uses this to route electricity payments without re-reading
    /// end-of-day fuel inventory after the plant has already consumed it.
    pub daily_power_service_units: f32,
    /// Aggregate utility power service units consumed from this building during the current day.
    ///
    /// Daily settlement derives this from citywide power demand and the plant's share of total
    /// produced power.
    pub daily_power_served_units: f32,
    /// Aggregate utility power service units produced during the last completed day.
    ///
    /// Building summaries and inspectors read this after the daily economy reset has cleared the
    /// live accumulator for the next day.
    pub recent_power_service_units: f32,
    /// Aggregate utility power service units consumed from this building last completed day.
    ///
    /// The inspector uses this with [`Self::recent_power_service_units`] to show used vs unused
    /// production.
    pub recent_power_served_units: f32,
    /// Most recently completed day's household sales revenue.
    ///
    /// Commercial staffing and input targets use this as a cheap demand signal instead of
    /// assuming every shop should immediately operate at full authored capacity.
    pub recent_household_sales_value: f32,
    /// Runtime-only commercial activity floor derived from local household demand and stock gaps.
    ///
    /// This is rebuilt by the household economy before production, hiring, and demand accounting;
    /// it is not persisted because it is a deterministic aggregate of live city state.
    pub commercial_activity_floor_scale: f32,
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
    /// Maximum half-diagonal of placed lots in zoning cells, rebuilt with [`Self::building_chunks`].
    pub(crate) max_lot_radius_cells: f32,
    /// Maximum support-footprint distance from its indexed lot center, in world metres.
    pub(crate) max_site_radius_m: f32,
    /// Recalculates inverted indices if true.
    pub dirty_index: bool,
    /// Per-family dirty flags in [`BASELINE_PRIVATE_ZONES`] order.
    pub dirty_zones: [bool; 3],
    /// True when the derived entrance cache must be rebuilt before use.
    pub(crate) entrances_dirty: bool,
    /// Revision bumped whenever building indices may have become stale for external systems.
    pub(crate) building_ref_revision: u64,
    /// Revision bumped whenever derived building entrance/access data changes.
    pub(crate) entrance_ref_revision: u64,
    /// Derived building entrance/access cache keyed by building index.
    pub(crate) entrances: Vec<BuildingEntrance>,
    /// Derived flat building-site clients keyed by building index.
    pub(crate) building_sites: Vec<BuildingSiteClient>,
    /// Building-site terrain bounds dirtied by allocator-owned cleanup.
    pub(crate) building_site_dirty_bounds: Option<(f32, f32, f32, f32)>,
    /// Registry of all loaded pack assets.
    pub registry: AssetRegistry,
}

pub(crate) use site::{BuildingSiteClient, BuildingSiteTerrainSnapshot};

/// Derived runtime binding from an asset-side `economy_profile` reference.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EconomyProfileBinding {
    /// Compact runtime profile id, or `0` when no runtime profile is bound.
    pub runtime_id: u16,
    /// True when the asset referenced an economy profile that the live runtime cannot execute.
    pub economy_broken: bool,
}

impl Building {
    /// Returns true while the building exists only as a construction site.
    pub(crate) fn is_under_construction(&self) -> bool {
        self.construction_remaining_hours > 0
    }

    /// Returns true when the building can participate in household, labor, and economy flows.
    pub(crate) fn is_operational(&self) -> bool {
        !self.is_under_construction()
    }

    /// Returns deterministic `0.0..=1.0` construction progress for rendering and diagnostics.
    pub(crate) fn construction_progress(&self) -> f32 {
        if self.construction_total_hours == 0 {
            return 1.0;
        }
        let remaining = self
            .construction_remaining_hours
            .min(self.construction_total_hours);
        1.0 - remaining as f32 / self.construction_total_hours as f32
    }

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
    resolve_building_economy_profile_binding_with_catalog(registry, catalog.as_ref(), asset_id)
}

/// Resolves an asset's economy profile using a caller-owned runtime catalog.
pub(crate) fn resolve_building_economy_profile_binding_with_catalog(
    registry: &AssetRegistry,
    catalog: &RuntimeEconomyCatalog,
    asset_id: &str,
) -> EconomyProfileBinding {
    let Some(profile_id) = registry.economy_profile(asset_id) else {
        return EconomyProfileBinding::default();
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
            max_lot_radius_cells: 0.0,
            max_site_radius_m: 0.0,
            dirty_index: true,
            dirty_zones: [false; 3],
            entrances_dirty: false,
            building_ref_revision: 0,
            entrance_ref_revision: 0,
            entrances: Vec::new(),
            building_sites: Vec::new(),
            building_site_dirty_bounds: None,
            registry: AssetRegistry::new(),
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
        if self.building_sites.len() != self.buildings.len() {
            self.rebuild_building_site_clients(zoning.config.zone_cell_m);
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
        transit_network: &crate::simulation::network::TransitNetwork,
        graph: &RegionGraph,
    ) -> u32 {
        self.admit_households_from_demand(
            households_to_admit_today as usize,
            agents,
            transit_network,
            graph,
        ) as u32
    }

    /// Advances private construction sites by one operational hour.
    pub(crate) fn advance_construction_hour(&mut self) {
        let mut completed_any = false;
        let mut completed_zone_dirty = [false; BASELINE_PRIVATE_ZONES.len()];
        for building in &mut self.buildings {
            if building.construction_remaining_hours == 0 {
                continue;
            }
            building.construction_remaining_hours -= 1;
            if building.construction_remaining_hours == 0 {
                completed_any = true;
                building.construction_total_hours = 0;
                if let Some(zone_idx) = baseline_private_zone_slot(building.zone_type) {
                    completed_zone_dirty[zone_idx] = true;
                }
                debug_log!(
                    "economy",
                    "building construction complete: asset={} zone={:?} level={}",
                    building.asset_id,
                    building.zone_type,
                    building.level
                );
            }
        }
        if completed_any {
            self.dirty = true;
            self.dirty_index = true;
            for (zone_idx, dirty) in completed_zone_dirty.into_iter().enumerate() {
                self.dirty_zones[zone_idx] |= dirty;
            }
            self.rebuild_zone_index();
        }
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
        let mut removed_site_bounds = None;
        if self.building_sites.len() == self.buildings.len() {
            let mut kept_buildings = Vec::with_capacity(self.buildings.len());
            let mut kept_sites = Vec::with_capacity(self.building_sites.len());
            for (building, site) in self.buildings.drain(..).zip(self.building_sites.drain(..)) {
                if building.edge_idx != usize::MAX {
                    kept_buildings.push(building);
                    kept_sites.push(site);
                } else {
                    accumulate_site_bounds(&mut removed_site_bounds, Some(site.bounds()));
                }
            }
            self.buildings = kept_buildings;
            self.building_sites = kept_sites;
            self.recompute_max_site_radius_m();
        } else {
            for idx in 0..self.buildings.len() {
                if self.buildings[idx].edge_idx == usize::MAX {
                    accumulate_site_bounds(&mut removed_site_bounds, self.site_world_bounds(idx));
                }
            }
            self.buildings.retain(|b| b.edge_idx != usize::MAX);
            self.building_sites.clear();
            self.max_site_radius_m = 0.0;
        }
        self.accumulate_pending_site_dirty_bounds(removed_site_bounds);
        if self.buildings.len() != old_len {
            self.dirty = true;
            self.dirty_index = true;
            self.bump_building_ref_revision();
        }
        self.entrances.clear();
        self.entrances_dirty = true;
        self.bump_entrance_ref_revision();
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
        let had_buildings = !self.buildings.is_empty();
        let had_entrances = !self.entrances.is_empty();
        let had_sites = !self.building_sites.is_empty();
        self.buildings.clear();
        self.building_sites.clear();
        self.edge_occupancy.clear();
        for list in &mut self.zone_index {
            list.clear();
        }
        for list in &mut self.vacancy_index {
            list.clear();
        }
        self.vacancy_pos.clear();
        self.building_chunks.clear();
        self.max_lot_radius_cells = 0.0;
        self.max_site_radius_m = 0.0;
        self.building_site_dirty_bounds = None;
        self.dirty = false;
        self.dirty_index = false;
        self.entrances.clear();
        self.entrances_dirty = false;
        if had_buildings || had_sites {
            self.bump_building_ref_revision();
        }
        if had_entrances {
            self.bump_entrance_ref_revision();
        }
    }

    /// Returns the current building-reference revision observed by dependent systems.
    pub(crate) fn building_ref_revision(&self) -> u64 {
        self.building_ref_revision
    }

    /// Returns the current derived entrance-reference revision observed by dependent systems.
    pub(crate) fn entrance_ref_revision(&self) -> u64 {
        self.entrance_ref_revision
    }

    fn bump_building_ref_revision(&mut self) {
        self.building_ref_revision = self.building_ref_revision.wrapping_add(1);
    }

    /// Advances the derived entrance-reference revision.
    pub(crate) fn bump_entrance_ref_revision(&mut self) {
        self.entrance_ref_revision = self.entrance_ref_revision.wrapping_add(1);
    }

    /// Returns the occupant capacity for a building, from its registered manifest.
    ///
    /// Unresolved assets or undeclared capacities count as zero.
    pub fn building_capacity(&self, building_idx: usize) -> u32 {
        let b = &self.buildings[building_idx];
        if b.broken || b.economy_broken || b.is_under_construction() {
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
        if b.broken || b.economy_broken || b.is_under_construction() {
            return 0;
        }
        self.registry.household_capacity(&b.asset_id)
    }

    /// Resets daily economy accumulators on every building.
    ///
    /// Called once per day after the demand snapshot has been taken, so the next day's
    /// logistics and household sales ticks accumulate against a clean baseline.
    pub(crate) fn reset_daily_input_accumulators(&mut self) {
        for building in &mut self.buildings {
            building.daily_owa_input_value = 0.0;
            building.daily_local_input_value = 0.0;
            building.daily_city_funded_input_cost = 0.0;
            building.recent_household_sales_value = building.daily_household_sales_value.max(0.0);
            building.daily_household_sales_value = 0.0;
            building.recent_power_service_units = building.daily_power_service_units.max(0.0);
            building.daily_power_service_units = 0.0;
            building.recent_power_served_units = building.daily_power_served_units.max(0.0);
            building.daily_power_served_units = 0.0;
        }
    }

    /// Returns the target floor area per household for a building.
    pub fn flat_size_m2(&self, building_idx: usize) -> f32 {
        let Some(b) = self.buildings.get(building_idx) else {
            return 0.0;
        };
        self.registry.flat_size_m2(&b.asset_id)
    }

    /// Returns the worker capacity authored on the asset manifest.
    pub fn worker_capacity_for_asset(&self, asset_id: &str) -> u32 {
        self.registry.worker_capacity(asset_id)
    }

    /// Returns the economy-profile worker capacity for an asset, failing safe on unresolved profiles.
    pub(crate) fn worker_capacity_for_asset_with_catalog(
        &self,
        asset_id: &str,
        catalog: &RuntimeEconomyCatalog,
    ) -> Option<u32> {
        if let Some(profile_id) = self.registry.economy_profile(asset_id) {
            if let Some(profile) = catalog.profile_for_id(profile_id) {
                if profile.runtime_supported {
                    return Some(profile.worker_capacity);
                }
            }
            return None;
        }
        Some(self.registry.worker_capacity(asset_id))
    }

    /// Returns the manifest worker capacity for a placed building when no runtime catalog is available.
    ///
    /// Unresolved assets, broken buildings, and deserted buildings count as zero.
    pub fn worker_capacity(&self, building_idx: usize) -> u32 {
        let Some(b) = self.buildings.get(building_idx) else {
            return 0;
        };
        if b.broken || b.economy_broken || b.is_deserted || b.is_under_construction() {
            return 0;
        }
        self.worker_capacity_for_asset(&b.asset_id)
    }

    /// Returns the live economy worker capacity for a placed building.
    pub(crate) fn worker_capacity_with_catalog(
        &self,
        building_idx: usize,
        catalog: &RuntimeEconomyCatalog,
    ) -> u32 {
        let Some(b) = self.buildings.get(building_idx) else {
            return 0;
        };
        if b.broken || b.economy_broken || b.is_deserted || b.is_under_construction() {
            return 0;
        }
        self.worker_capacity_for_asset_with_catalog(&b.asset_id, catalog)
            .unwrap_or(0)
    }

    /// Returns whether the placed building is a city-funded explicit service building.
    pub(crate) fn is_city_service_building(&self, building: &Building) -> bool {
        self.registry.is_city_service_asset(&building.asset_id)
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
        self.fill_nearby_buildings_by_zones(
            origin_x,
            origin_y,
            zones,
            max_chunk_radius,
            candidate_limit,
            &mut candidates,
        );
        candidates
    }

    /// Fills a reusable nearby candidate buffer for the requested zones.
    pub fn fill_nearby_buildings_by_zones(
        &self,
        origin_x: f32,
        origin_y: f32,
        zones: &[ZoneType],
        max_chunk_radius: i32,
        candidate_limit: usize,
        candidates: &mut Vec<usize>,
    ) {
        self.fill_nearby_buildings(
            origin_x,
            origin_y,
            max_chunk_radius,
            candidate_limit,
            candidates,
            |_, building| zones.contains(&building.zone_type),
        );
    }

    /// Fills a reusable nearby candidate buffer using a caller-provided eligibility predicate.
    pub fn fill_nearby_buildings(
        &self,
        origin_x: f32,
        origin_y: f32,
        max_chunk_radius: i32,
        candidate_limit: usize,
        candidates: &mut Vec<usize>,
        mut eligible: impl FnMut(usize, &Building) -> bool,
    ) {
        candidates.clear();
        if candidate_limit == 0 {
            return;
        }
        let origin_chunk =
            RegionGraph::get_chunk_coords(godot::prelude::Vector3::new(origin_x, 0.0, origin_y));

        for ring in 0..=max_chunk_radius {
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
                        if eligible(idx, &self.buildings[idx]) {
                            candidates.push(idx);
                        }
                    }
                }
            }
        }

        candidates.sort_unstable_by(|&a, &b| {
            let da = squared_distance(origin_x, origin_y, &self.buildings[a]);
            let db = squared_distance(origin_x, origin_y, &self.buildings[b]);
            da.total_cmp(&db).then_with(|| a.cmp(&b))
        });
        candidates.truncate(candidate_limit);
    }
}

fn squared_distance(origin_x: f32, origin_y: f32, building: &Building) -> f32 {
    let dx = building.center_x - origin_x;
    let dy = building.center_y - origin_y;
    dx * dx + dy * dy
}

fn accumulate_site_bounds(
    target: &mut Option<(f32, f32, f32, f32)>,
    bounds: Option<(f32, f32, f32, f32)>,
) {
    let Some(bounds) = bounds else {
        return;
    };
    if let Some(existing) = target {
        existing.0 = existing.0.min(bounds.0);
        existing.1 = existing.1.min(bounds.1);
        existing.2 = existing.2.max(bounds.2);
        existing.3 = existing.3.max(bounds.3);
    } else {
        *target = Some(bounds);
    }
}
