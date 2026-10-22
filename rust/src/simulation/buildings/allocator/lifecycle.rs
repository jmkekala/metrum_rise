// SPDX-License-Identifier: GPL-2.0-only

//! Building removal, demand-owned household admission, and coordinate restoration.

use crate::config::SIDEWALK_WIDTH;
use crate::debug_log;
use crate::simulation::buildings::allocator::{
    BuildingAllocator, DemandSpawnPlacementRejection, baseline_private_zone_slot,
    resolve_building_economy_profile_binding_with_catalog, zone_class_to_zone_type,
};
use crate::simulation::economy::agents::{AgentSystem, HouseholdAgeComposition};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::demand::{
    DemandBuildingActionKey, DemandBuildingActionPlan, DemandLevelChangeAction,
    DemandLevelChangeGuard, DemandSpawnAction,
};
use crate::simulation::economy::households::{
    HouseholdSystem, applicant_offer, applicant_offer_limit,
};
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneSystem;
use crate::simulation::network::surface::RoadSurfaceSystem;
use crate::simulation::network::types::{NodeType, TransitFlags, TransitType};
use crate::simulation::pathing::cch::CchCostsFrom;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::zoning::ZoneType;
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::{Vector2, Vector3};
use rayon::prelude::*;

const REZONE_GRACE_DAYS: u8 = 3;
const FRONTAGE_ATTACHMENT_REPAIR_MIN_SEARCH_M: f32 = 50.0;
const FRONTAGE_ATTACHMENT_REPAIR_SEARCH_MARGIN_M: f32 = 20.0;
const FRONTAGE_ATTACHMENT_VALID_MIN_DISTANCE_M: f32 = 6.0;

/// Summary of building mutations performed by one demand-owned building action pass.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DemandBuildingActionExecution {
    /// Residential spawn execution and final placement rejection counters.
    pub(crate) residential: DemandUseBuildingActionExecution,
    /// Commercial spawn execution and final placement rejection counters.
    pub(crate) commercial: DemandUseBuildingActionExecution,
    /// Industrial spawn execution and final placement rejection counters.
    pub(crate) industrial: DemandUseBuildingActionExecution,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DemandUseBuildingActionExecution {
    /// Selected spawn actions submitted to allocator placement.
    pub(crate) spawn_attempted: usize,
    /// Selected spawn actions that committed a building.
    pub(crate) spawn_executed: usize,
    /// Selected spawn actions rejected by final allocator placement validation.
    pub(crate) spawn_rejections: DemandSpawnPlacementRejectionCounts,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DemandSpawnPlacementRejectionCounts {
    /// The selected asset no longer resolved to valid placement parameters.
    pub(crate) asset_unavailable: usize,
    /// The selected parcel no longer existed in the zoning system.
    pub(crate) parcel_unavailable: usize,
    /// The parcel geometry or occupancy no longer accepted the selected asset.
    pub(crate) slot_unavailable: usize,
    /// A driveway anchor could not resolve an adjacent road surface height.
    pub(crate) driveway_road_surface_missing: usize,
    /// Multiple driveway anchors required incompatible flat-site heights.
    pub(crate) driveway_height_conflict: usize,
    /// The asset has driveway anchors, but none touch the claimed road edge.
    pub(crate) driveway_connection_missing: usize,
    /// The frontage fallback could not resolve an adjacent road surface height.
    pub(crate) frontage_road_surface_missing: usize,
    /// The selected flat-site height conflicted with an existing neighboring site.
    pub(crate) neighbor_site_height_conflict: usize,
    /// The flat support footprint could not tie into terrain/roads within slope limits.
    pub(crate) site_support_tie_in_invalid: usize,
}

/// Why a building waits in the `building removals` queue (`SIM-02-J`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuildingRemovalKind {
    /// Planned clearance of a failed site from the hour's plan; removed if it may still be
    /// cleared ([`BuildingAllocator::can_demand_despawn`]).
    Clearance,
    /// Its attachment went or its rezoning grace ran out at its daily phase; removed if that
    /// still holds ([`BuildingAllocator::compatibility_expired`]).
    Incompatible,
}

impl BuildingRemovalKind {
    /// Saved code.
    pub(crate) fn to_saved(self) -> i64 {
        match self {
            Self::Clearance => 0,
            Self::Incompatible => 1,
        }
    }

    /// Inverse of [`Self::to_saved`].
    pub(crate) fn from_saved(code: i64) -> Option<Self> {
        match code {
            0 => Some(Self::Clearance),
            1 => Some(Self::Incompatible),
            _ => None,
        }
    }
}

/// A queued building removal: the slot that queued it, the building and why (`SIM-02-J`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BuildingRemovalRequest {
    /// Slot that queued it.
    pub(crate) slot: u64,
    /// The building to remove; dropped when it no longer resolves.
    pub(crate) building: crate::simulation::core::handles::Handle<super::Building>,
    /// Why it waits.
    pub(crate) kind: BuildingRemovalKind,
}

// Car costs from every car-connected border node: one sweep per border instead of one route
// query per border per household (`ALLOC-04`). Kept across admission batches while the routing
// hierarchy, its metric and the graph's node roles are those it was swept on (`SIM-02-K`).
#[derive(Clone, Default)]
pub(super) struct BorderArrivalCosts {
    // CCH build generation, metric revision and node-role revision of the tables; `None` when
    // never swept or swept while a topology edit waited for its CCH rebuild.
    key: Option<(u32, u64, u64)>,
    // Ascending node ids.
    borders: Vec<u32>,
    costs: Vec<CchCostsFrom>,
}

impl BorderArrivalCosts {
    // The revisions the tables follow, or `None` while edited chunks wait for the CCH rebuild:
    // the sweeps read the CCH alone and the border set the graph's node roles and edges, and
    // every topology edit marks chunks and rebuilds the CCH, moving its build generation.
    fn revision(transit_network: &TransitNetwork, graph: &RegionGraph) -> Option<(u32, u64, u64)> {
        let cch = &transit_network.cch_graph;
        transit_network.cch_dirty_chunks.is_empty().then_some((
            cch.build_generation,
            cch.metric_revision,
            graph.node_role_revision(),
        ))
    }

    // O(1) when the revisions match; otherwise the O(nodes) border scan, then
    // `CchGraph::costs_from_each` over the borders, as every admission batch did before.
    fn refresh(&mut self, transit_network: &TransitNetwork, graph: &RegionGraph) {
        let key = Self::revision(transit_network, graph);
        if key.is_some() && self.key == key {
            #[cfg(test)]
            if crate::debug::work::city_checks() {
                self.assert_current(transit_network, graph);
            }
            return;
        }
        *self = Self::sweep(transit_network, graph);
        self.key = key;
    }

    fn sweep(transit_network: &TransitNetwork, graph: &RegionGraph) -> Self {
        let borders: Vec<u32> = (0..graph.node_count() as u32)
            .into_par_iter()
            .filter(|&node| {
                graph.node(node).node_type == NodeType::Border
                    && graph.node_adjacency(node).iter().any(|&edge_idx| {
                        let edge = graph.edge(edge_idx);
                        !edge.deleted
                            && edge.primary_type == TransitType::Road
                            && (edge.allowed_types & TransitFlags::CAR) != 0
                    })
            })
            .collect();
        let costs = transit_network
            .cch_graph
            .costs_from_each(&borders, graph, TransitFlags::CAR);
        Self {
            key: None,
            borders,
            costs,
        }
    }

    // Test builds: the kept tables equal a fresh sweep, bit for bit.
    #[cfg(test)]
    fn assert_current(&self, transit_network: &TransitNetwork, graph: &RegionGraph) {
        let fresh = Self::sweep(transit_network, graph);
        assert_eq!(
            self.borders, fresh.borders,
            "kept arrival borders went stale"
        );
        for (kept, fresh) in self.costs.iter().zip(&fresh.costs) {
            for node in 0..graph.node_count() as u32 {
                assert_eq!(
                    kept.cost_to(node).map(f32::to_bits),
                    fresh.cost_to(node).map(f32::to_bits),
                    "kept arrival costs went stale"
                );
            }
        }
    }
}

impl DemandBuildingActionExecution {
    fn use_mut(&mut self, zone_type: ZoneType) -> &mut DemandUseBuildingActionExecution {
        match zone_type {
            ZoneType::Residential => &mut self.residential,
            ZoneType::Commercial => &mut self.commercial,
            ZoneType::Industrial => &mut self.industrial,
            _ => unreachable!("demand building execution only tracks baseline private zones"),
        }
    }
}

impl DemandSpawnPlacementRejectionCounts {
    fn record(&mut self, reason: DemandSpawnPlacementRejection) {
        match reason {
            DemandSpawnPlacementRejection::AssetUnavailable => self.asset_unavailable += 1,
            DemandSpawnPlacementRejection::ParcelUnavailable => self.parcel_unavailable += 1,
            DemandSpawnPlacementRejection::SlotUnavailable => self.slot_unavailable += 1,
            DemandSpawnPlacementRejection::DrivewayRoadSurfaceMissing => {
                self.driveway_road_surface_missing += 1;
            }
            DemandSpawnPlacementRejection::DrivewayHeightConflict => {
                self.driveway_height_conflict += 1;
            }
            DemandSpawnPlacementRejection::DrivewayConnectionMissing => {
                self.driveway_connection_missing += 1;
            }
            DemandSpawnPlacementRejection::FrontageRoadSurfaceMissing => {
                self.frontage_road_surface_missing += 1;
            }
            DemandSpawnPlacementRejection::NeighborSiteHeightConflict => {
                self.neighbor_site_height_conflict += 1;
            }
            DemandSpawnPlacementRejection::SiteSupportTieInInvalid => {
                self.site_support_tie_in_invalid += 1;
            }
        }
    }

    pub(crate) fn total(self) -> usize {
        self.asset_unavailable
            + self.parcel_unavailable
            + self.slot_unavailable
            + self.driveway_road_surface_missing
            + self.driveway_height_conflict
            + self.driveway_connection_missing
            + self.frontage_road_surface_missing
            + self.neighbor_site_height_conflict
            + self.site_support_tie_in_invalid
    }
}

impl BuildingAllocator {
    /// Removes invalid attachments and advances incompatible-zoning grace by actual elapsed days.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn cleanup_stale_buildings(
        &mut self,
        elapsed_days: u8,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
        treasury_balance: &mut f64,
        graph: &RegionGraph,
        lanes: &LaneSystem,
    ) {
        let mut removed_any = false;
        let mut i = 0;
        while i < self.buildings.len() {
            if self.expire_incompatible_building(
                i,
                elapsed_days,
                zoning,
                agents,
                households,
                logistics,
                treasury_balance,
                graph,
            ) {
                removed_any = true;
            } else {
                i += 1;
            }
        }
        if removed_any {
            self.bump_building_ref_revision();
            self.rebuild_entrance_cache(graph, lanes);
        }
    }

    /// Checks building `i`'s attachment and zoning compatibility, advancing an incompatible
    /// building's grace by `elapsed_days`, and swap-removes it when its attachment is gone or
    /// its grace has run out. Editing paths call it through [`Self::maintain`]; the caller then
    /// bumps the building revision and rebuilds the entrance cache once. Returns whether the
    /// building was removed. O(1), plus the removal's reference repairs.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn expire_incompatible_building(
        &mut self,
        i: usize,
        elapsed_days: u8,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
        treasury_balance: &mut f64,
        graph: &RegionGraph,
    ) -> bool {
        if !self.compatibility_expired(i, elapsed_days, zoning, graph) {
            return false;
        }
        self.remove_incompatible_building(
            i,
            zoning,
            agents,
            households,
            logistics,
            treasury_balance,
        );
        true
    }

    /// Checks building `i`'s attachment and zoning compatibility and advances an incompatible
    /// building's grace by `elapsed_days`; returns whether it must be removed: its attachment is
    /// gone or its grace has run out. A building's daily phase calls it with one day and queues
    /// the removal (`SIM-02-J`); the removal queue calls it again with none, so a building that
    /// recovered in between stays. O(1).
    pub(crate) fn compatibility_expired(
        &mut self,
        i: usize,
        elapsed_days: u8,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
    ) -> bool {
        let compatibility = {
            let b = &self.buildings[i];
            let edge_ok = b.edge_idx < graph.edge_count() && !graph.edge(b.edge_idx).deleted;
            if !edge_ok || graph.edge(b.edge_idx).no_building_spawn {
                None
            } else {
                match self.registry.get(&b.asset_id) {
                    Some(entry) => match entry.manifest.building.as_ref() {
                        Some(asset_building) if asset_building.is_zoned_private() => {
                            match (asset_building.zone_type, asset_building.density_key()) {
                                (Some(asset_zone_class), Some(asset_density)) => {
                                    if let Some(parcel) = zoning.parcel_by_raw_id(b.parcel_id) {
                                        if parcel.edge_idx() != b.edge_idx {
                                            None
                                        } else {
                                            let expected_zone_type =
                                                zone_class_to_zone_type(asset_zone_class);
                                            let width_m =
                                                b.width_cells as f32 * zoning.config.zone_cell_m;
                                            let depth_m =
                                                b.depth_cells as f32 * zoning.config.zone_cell_m;
                                            let compatible = width_m
                                                <= parcel.frontage_m() + f32::EPSILON
                                                && depth_m <= parcel.depth_m() + f32::EPSILON
                                                && zoning.profiles.asset_is_legal(
                                                    parcel.zone_profile_runtime_id(),
                                                    expected_zone_type,
                                                    asset_density,
                                                    &entry.manifest.tags,
                                                );
                                            Some(compatible)
                                        }
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            }
                        }
                        Some(_) => Some(true),
                        None => None,
                    },
                    None => None,
                }
            }
        };
        let was_pending_redevelopment = self.buildings[i].pending_redevelopment;
        let remove = match compatibility {
            None => true,
            Some(true) => {
                let building = &mut self.buildings[i];
                building.pending_redevelopment = false;
                building.rezone_grace_days_remaining = 0;
                false
            }
            Some(false) => {
                let building = &mut self.buildings[i];
                if !building.pending_redevelopment {
                    building.pending_redevelopment = true;
                    building.rezone_grace_days_remaining = REZONE_GRACE_DAYS;
                    false
                } else {
                    building.rezone_grace_days_remaining = building
                        .rezone_grace_days_remaining
                        .saturating_sub(elapsed_days);
                    building.rezone_grace_days_remaining == 0
                }
            }
        };

        if !self.dirty_index && was_pending_redevelopment != self.buildings[i].pending_redevelopment
        {
            self.admission_occupancy_changed(i);
        }
        remove
    }

    /// Swap-removes building `i`, whose compatibility expired ([`Self::compatibility_expired`]),
    /// with every reference repair of a removal. The caller bumps the building revision and
    /// repairs the derived caches once. O(1) plus the removal's parallel O(A + H) remaps.
    pub(crate) fn remove_incompatible_building(
        &mut self,
        i: usize,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
        treasury_balance: &mut f64,
    ) {
        let b = &self.buildings[i];
        let b_parcel_id = b.parcel_id;
        let b_zone = b.zone_type;
        let removed_site_bounds = self.site_dirty_bounds(i);
        self.record_building_site_change(i, removed_site_bounds);
        if let Some(zone_idx) = baseline_private_zone_slot(b_zone) {
            self.dirty_zones[zone_idx] = true;
        }
        zoning.clear_parcel_occupancy(b_parcel_id);

        agents.evict_building(i);
        households.invalidate_building(i, self);
        logistics.invalidate_building(i, self, agents, treasury_balance);
        let city_owned = self.is_city_service_building(&self.buildings[i]);
        households.close_building_electricity(&mut self.buildings[i], city_owned, treasury_balance);
        let last_idx = self.buildings.len() - 1;
        if i < last_idx {
            if let Some(zone_idx) = baseline_private_zone_slot(self.buildings[last_idx].zone_type) {
                self.dirty_zones[zone_idx] = true;
            }
            let mut mapping = std::collections::HashMap::new();
            mapping.insert(last_idx, i);
            agents.remap_building_indices(&mapping);
            households.remap_building_indices(&mapping);
            logistics.remap_building_indices(&mapping);
            zoning.remap_parcel_occupancy(self.buildings[last_idx].parcel_id, last_idx, i);
        }

        self.field_clearance.remove_and_remap(i, last_idx);
        self.record_production_site_removal(i, last_idx);
        self.queue_yard_removal(i);
        crate::debug::money::record_change(
            crate::debug::money::MoneyFlow::RemovedBuildingCash,
            self.buildings[i].operating_budget,
            0.0,
        );
        self.building_handles.swap_remove(i, self.buildings.len());
        self.buildings.swap_remove(i);
        crate::debug::work::add(crate::debug::work::Work::BuildingsRemoved, 1);
        self.remove_building_site_client(i);
        if let Some(bounds) = removed_site_bounds {
            debug_log!(
                "economy",
                "building site removed by cleanup: building_idx={} bounds=({:.1},{:.1})-({:.1},{:.1})",
                i,
                bounds.0,
                bounds.1,
                bounds.2,
                bounds.3
            );
        }
        self.mark_index_stale();
        self.entrances_dirty = true;
    }

    /// Sweeps admission's border arrival costs for the loaded network (`SIM-02-K`), as the
    /// first admission batch after loading would: the tables are kept per routing revision, so
    /// that batch finds them current. O(1) when current, else O(nodes) plus one CCH sweep a
    /// car-connected border.
    pub(crate) fn prepare_border_arrival_costs(
        &mut self,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.border_arrival.refresh(transit_network, graph);
    }

    /// Admits bounded arrival candidates after demand claims their actual income requirements:
    /// each accepted family claims its home's vacancy and is handed to `launch` with its home,
    /// size, composition and arrival border.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn admit_households_from_demand(
        &mut self,
        households_to_spawn: usize,
        next_household_id: usize,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        mut claim_income: impl FnMut(HouseholdAgeComposition, u16, u8) -> bool,
        mut launch: impl FnMut(usize, u16, HouseholdAgeComposition, u32),
    ) -> usize {
        if households_to_spawn == 0 {
            return 0;
        }
        let Some(residential_slot) = baseline_private_zone_slot(ZoneType::Residential) else {
            return 0;
        };
        if self.dirty_index {
            self.rebuild_zone_index();
        }
        for category in [crate::debug::Channel::Economy, crate::debug::Channel::Spawn] {
            debug_log!(
                category,
                "demand-owned household admission planning: households_to_spawn={}",
                households_to_spawn,
            );
        }
        let mut launched = 0;
        // The kept sweeps, refreshed by the batch's first applicant offer as they were built.
        let mut arrival = std::mem::take(&mut self.border_arrival);
        let mut arrival_current = false;
        // Each lookup is O(1) over the existing vacancy index; rejection neither
        // reserves income nor prevents another home/family from being considered.
        let attempt_limit = applicant_offer_limit(households_to_spawn);
        let mut cursor = next_household_id;
        for attempt in 0..attempt_limit {
            if launched == households_to_spawn {
                break;
            }
            let vacancies = &self.vacancy_index[residential_slot];
            if vacancies.is_empty() {
                break;
            }
            let position = cursor % vacancies.len();
            let home_idx = vacancies[position];
            cursor = position + 1;
            let household_seed = next_household_id.saturating_add(attempt);
            let Some((household_size, composition)) = applicant_offer(
                self.flat_size_m2(home_idx),
                self.applicant_seed(home_idx),
                household_seed,
            ) else {
                continue;
            };
            if !arrival_current {
                arrival.refresh(transit_network, graph);
                arrival_current = true;
            }
            let Some(border_node) =
                self.household_arrival_border_node(home_idx, &arrival, transit_network, graph)
            else {
                for category in [crate::debug::Channel::Economy, crate::debug::Channel::Spawn] {
                    debug_log!(
                        category,
                        "demand-owned household admission waiting: no legal border-to-home car route for home_building={}",
                        home_idx
                    );
                }
                continue;
            };
            if !claim_income(composition, household_size, self.buildings[home_idx].level) {
                continue;
            }
            // One household consumes 1 slot of household_capacity regardless of size.
            self.claim_vacancy(home_idx);
            // A full home may swap-remove its vacancy; consider the moved entry next.
            cursor = position;
            launch(home_idx, household_size, composition, border_node);
            launched += 1;
            for category in [crate::debug::Channel::Economy, crate::debug::Channel::Spawn] {
                debug_log!(
                    category,
                    "demand-owned household admission accepted size={} home_building={} border_node={}",
                    household_size,
                    home_idx,
                    border_node,
                );
            }
        }
        self.border_arrival = arrival;
        launched
    }

    // The border with the fastest car arrival at the home, ties to the lowest node id. O(borders).
    fn household_arrival_border_node(
        &self,
        home_idx: usize,
        arrival: &BorderArrivalCosts,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<u32> {
        let mut best: Option<(u32, f32)> = None;
        for (&border_node, costs) in arrival.borders.iter().zip(&arrival.costs) {
            let Some(eta_s) = self.freight_car_eta_from_border_costs(
                border_node,
                home_idx,
                costs,
                transit_network,
                graph,
            ) else {
                continue;
            };
            if best.as_ref().is_none_or(|&(best_node, best_eta)| {
                eta_s < best_eta
                    || ((eta_s - best_eta).abs() <= f32::EPSILON && border_node < best_node)
            }) {
                best = Some((border_node, eta_s));
            }
        }
        best.map(|(border_node, _)| border_node)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute_demand_building_actions(
        &mut self,
        plan: &DemandBuildingActionPlan,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
        treasury_balance: &mut f64,
        graph: &RegionGraph,
        network: &TransitNetwork,
        terrain: &TerrainSystem,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        level_changes: &mut impl DemandLevelChangeGuard,
    ) -> DemandBuildingActionExecution {
        let road_surface = &network.road_surface;
        let mut mutated_any = false;
        let mut execution = DemandBuildingActionExecution::default();

        let uses = [
            (ZoneType::Residential, &plan.residential),
            (ZoneType::Commercial, &plan.commercial),
            (ZoneType::Industrial, &plan.industrial),
        ];
        // Every zone's guarded level changes come first, while the guard's proofs lend the
        // census's books (`SIM-02-I`); a despawn swap-removes a building under those books,
        // so the guard closes before the first one.
        if uses
            .iter()
            .any(|(_, use_plan)| !use_plan.downgrades.is_empty() || !use_plan.upgrades.is_empty())
            && self.entrances_dirty
        {
            self.rebuild_entrance_cache(graph, &network.lane_system);
        }
        for (_, use_plan) in uses {
            for action in use_plan.downgrades.iter().chain(&use_plan.upgrades) {
                let Some(building_idx) =
                    self.resolve_demand_building_action(&action.building, zoning)
                else {
                    continue;
                };
                let Some(permit) = level_changes.prepare(
                    building_idx,
                    action,
                    self,
                    households,
                    logistics,
                    zoning,
                    *treasury_balance,
                ) else {
                    continue;
                };
                let applied = self
                    .apply_level_change_action(
                        building_idx,
                        &action.target_asset_id,
                        catalog,
                        zoning.config.zone_cell_m,
                        graph,
                        super::BuildingSiteEnvironment {
                            road_surface,
                            terrain,
                        },
                    )
                    .is_some();
                level_changes.finish(permit, applied, self);
                mutated_any |= applied;
            }
        }
        level_changes.close(households, self);

        for (zone_type, use_plan) in uses {
            for action in &use_plan.despawns {
                let Some(building_idx) = self.resolve_demand_building_action(action, zoning) else {
                    continue;
                };
                if !self.can_demand_despawn(building_idx) {
                    continue;
                }
                mutated_any |= self.remove_evacuated_building(
                    building_idx,
                    zoning,
                    agents,
                    households,
                    logistics,
                    treasury_balance,
                    graph,
                    network,
                );
            }

            for action in &use_plan.spawns {
                execution.use_mut(zone_type).spawn_attempted += 1;
                // A stale index makes every site check scan all buildings; despawns and level
                // changes above cost one rebuild here, spawns below keep it clean by appending.
                if self.dirty_index || self.vacancy_pos.len() != self.buildings.len() {
                    self.rebuild_zone_index();
                }
                match self.execute_demand_spawn_action(
                    action,
                    zoning,
                    graph,
                    road_surface,
                    terrain,
                    catalog,
                    tuning,
                ) {
                    Ok(building_idx) => {
                        execution.use_mut(zone_type).spawn_executed += 1;
                        mutated_any = true;
                        // On failure the index stays dirty and the next spawn rebuilds it.
                        self.index_appended_building(building_idx);
                    }
                    Err(reason) => {
                        execution.use_mut(zone_type).spawn_rejections.record(reason);
                    }
                }
            }
        }

        if mutated_any {
            // Rebuilt even when appends kept it clean: claims since the last rebuild have
            // swap-removed vacancies out of building order, and admission breaks ties by
            // vacancy position (`ALLOC-03`). One O(N) pass per batch, not per spawn.
            self.rebuild_zone_index();
            self.rebuild_entrance_cache(graph, &network.lane_system);
        }
        execution
    }

    /// Proves the hour plan's level changes without applying them (`SIM-02-J`): every zone's
    /// downgrades and upgrades in plan order, each resolved to its building, proved by
    /// `level_changes` and checked to fit now. A proved change keeps its claims in the open
    /// epoch, so the changes after it see them, and joins `proved` with its building for the
    /// level-change queue, which applies it later. The guard closes after the last change.
    /// O(changes × proof).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prove_demand_level_changes<'a>(
        &mut self,
        plan: &'a DemandBuildingActionPlan,
        zoning: &ZoningSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
        treasury_balance: f64,
        graph: &RegionGraph,
        network: &TransitNetwork,
        terrain: &TerrainSystem,
        catalog: &RuntimeEconomyCatalog,
        level_changes: &mut impl DemandLevelChangeGuard,
        proved: &mut Vec<(usize, &'a DemandLevelChangeAction)>,
    ) {
        let uses = [&plan.residential, &plan.commercial, &plan.industrial];
        if uses
            .iter()
            .any(|use_plan| !use_plan.downgrades.is_empty() || !use_plan.upgrades.is_empty())
            && self.entrances_dirty
        {
            self.rebuild_entrance_cache(graph, &network.lane_system);
        }
        for use_plan in uses {
            for action in use_plan.downgrades.iter().chain(&use_plan.upgrades) {
                let Some(building_idx) =
                    self.resolve_demand_building_action(&action.building, zoning)
                else {
                    continue;
                };
                let Some(permit) = level_changes.prepare(
                    building_idx,
                    action,
                    self,
                    households,
                    logistics,
                    zoning,
                    treasury_balance,
                ) else {
                    continue;
                };
                let fits = self.level_change_fits(
                    building_idx,
                    &action.target_asset_id,
                    catalog,
                    zoning.config.zone_cell_m,
                    graph,
                    super::BuildingSiteEnvironment {
                        road_surface: &network.road_surface,
                        terrain,
                    },
                );
                level_changes.finish(permit, fits, self);
                if fits {
                    proved.push((building_idx, action));
                }
            }
        }
        level_changes.close(households, self);
    }

    /// Removes building `building_idx` for planned clearance (`SIM-02-J`) when it may still be
    /// cleared ([`Self::can_demand_despawn`]): its remaining occupants leave through the street
    /// handoff first, then the removal repairs every reference. Returns whether it was removed;
    /// the caller repairs the derived caches once. O(1) plus the removal's parallel remaps.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn remove_cleared_building(
        &mut self,
        building_idx: usize,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
        treasury_balance: &mut f64,
        graph: &RegionGraph,
        network: &TransitNetwork,
    ) -> bool {
        if !self.can_demand_despawn(building_idx) {
            return false;
        }
        self.remove_evacuated_building(
            building_idx,
            zoning,
            agents,
            households,
            logistics,
            treasury_balance,
            graph,
            network,
        )
    }

    // Removes building `building_idx` after evacuating its remaining occupants through the
    // street handoff (employment slots can be empty while former employees remain inside, so
    // a physical route home is kept before deletion invalidates the entrance). One parallel
    // scan finds the agents naming it or the last building, which its swap-remove moves into
    // its index; the evacuation, eviction and agent remap visit those alone (`SIM-02-K`).
    // Returns whether it was removed. O(A/p) plus the removal's other remaps.
    #[allow(clippy::too_many_arguments)]
    fn remove_evacuated_building(
        &mut self,
        building_idx: usize,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
        treasury_balance: &mut f64,
        graph: &RegionGraph,
        network: &TransitNetwork,
    ) -> bool {
        if building_idx >= self.buildings.len() {
            return false;
        }
        let candidates =
            agents.agents_referencing_buildings(building_idx, self.buildings.len() - 1);
        agents.evacuate_building_among(building_idx, &candidates, households, self, network, graph);
        self.remove_building_at_index(
            building_idx,
            zoning,
            agents,
            households,
            logistics,
            treasury_balance,
            Some(&candidates),
        )
    }

    /// Executes one queued demand spawn through the same final placement path as batch demand,
    /// seeding its applicants with `applicant_seed` (its spawn id; the city counter without).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute_single_demand_spawn_action(
        &mut self,
        zone_type: ZoneType,
        action: &DemandSpawnAction,
        applicant_seed: Option<u64>,
        zoning: &mut ZoningSystem,
        graph: &RegionGraph,
        lanes: &LaneSystem,
        road_surface: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> DemandBuildingActionExecution {
        let mut execution = DemandBuildingActionExecution::default();
        execution.use_mut(zone_type).spawn_attempted += 1;
        let zone_index_was_clean =
            !self.dirty_index && self.vacancy_pos.len() == self.buildings.len();
        let entrance_cache_was_clean =
            !self.entrances_dirty && self.entrances.len() == self.buildings.len();
        match self.execute_seeded_demand_spawn_action(
            action,
            applicant_seed,
            zoning,
            graph,
            road_surface,
            terrain,
            catalog,
            tuning,
        ) {
            Ok(building_idx) => {
                execution.use_mut(zone_type).spawn_executed += 1;
                if self.dirty_index
                    && (!zone_index_was_clean || !self.index_appended_building(building_idx))
                {
                    self.rebuild_zone_index();
                }
                if !entrance_cache_was_clean
                    || !self.append_entrance_cache_for_building(building_idx, graph, lanes)
                {
                    self.rebuild_entrance_cache(graph, lanes);
                }
            }
            Err(reason) => {
                execution.use_mut(zone_type).spawn_rejections.record(reason);
            }
        }
        execution
    }

    /// Recomputes world-space building transforms from saved frontage attachment data.
    pub(crate) fn recompute_derived_transforms(
        &mut self,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
    ) -> Result<(), String> {
        for building in &mut self.buildings {
            if building.edge_idx >= graph.edge_count() {
                return Err(format!(
                    "building edge {} out of bounds for {} edges",
                    building.edge_idx,
                    graph.edge_count()
                ));
            }

            let edge = graph.edge(building.edge_idx);
            if edge.physical_geometry.len() < 2 || edge.physical_length <= 1e-6 {
                return Err(format!(
                    "building edge {} has insufficient geometry for transform rebuild",
                    building.edge_idx
                ));
            }

            let zone_cell_m = zoning.config.zone_cell_m;
            let depth_cells = building.depth_cells as f32;
            let (center_2d, normal, side_offset) = if building.parcel_id != 0 {
                let Some(parcel) = zoning.parcel_by_raw_id(building.parcel_id) else {
                    return Err(format!("building parcel {} missing", building.parcel_id));
                };
                let depth_m = depth_cells * zone_cell_m;
                (
                    parcel.front_center() + parcel.normal() * (depth_m * 0.5),
                    parcel.normal(),
                    edge.width * 0.5 + crate::config::SIDEWALK_WIDTH,
                )
            } else {
                // Explicit placement has no frontage grid cell. Its saved normalized
                // attachment is the center of the frontage, including on curved roads.
                Self::explicit_frontage_transform(
                    graph,
                    building.edge_idx,
                    building.frontage_t,
                    building.side,
                    depth_cells * zone_cell_m,
                )
                .ok_or_else(|| {
                    format!(
                        "building edge {} has no frontage tangent",
                        building.edge_idx
                    )
                })?
            };

            building.center_x = center_2d.x;
            building.center_y = center_2d.y;
            building.facing_dir = normal * -1.0;
            building.side_offset = side_offset;
            building.front_gap_m = super::geometry::lot_front_gaps(
                graph,
                center_2d,
                building.facing_dir,
                f32::from(building.width_cells) * zone_cell_m * 0.5,
                depth_cells * zone_cell_m * 0.5,
            );
        }

        self.dirty = true;
        self.rebuild_building_site_clients(zoning.config.zone_cell_m);
        self.bump_building_ref_revision();
        Ok(())
    }

    /// Moves the buildings fronting `edge_id` onto the halves of its split at cell `split_x`:
    /// those at or past it front `new_edge_id` from its start, and each one's frontage
    /// parameter is measured along its half, `lengths_m` long. The moved rows are recorded, so
    /// row-indexed tables and the census input copy (`SIM-02-I`) take them again; `undo`
    /// receives each row's previous edge, cell and frontage. Returns the buildings on each
    /// half. O(B).
    pub(crate) fn migrate_split_frontages(
        &mut self,
        [edge_id, new_edge_id]: [usize; 2],
        split_x: usize,
        lengths_m: [f32; 2],
        cell_m: f32,
        mut undo: Option<&mut Vec<(usize, usize, usize, f32)>>,
    ) -> [usize; 2] {
        let traffic_debug = crate::debug::Channel::Traffic.enabled();
        let mut moved = Vec::new();
        let mut halves = [0; 2];
        for (building_idx, b) in self.buildings.iter_mut().enumerate() {
            if b.edge_idx != edge_id {
                continue;
            }
            if let Some(undo) = undo.as_deref_mut() {
                undo.push((building_idx, b.edge_idx, b.cell_x, b.frontage_t));
            }
            let (old_cell_x, old_frontage_t) = (b.cell_x, b.frontage_t);
            let half = usize::from(b.cell_x >= split_x);
            if half == 1 {
                b.edge_idx = new_edge_id;
                b.cell_x -= split_x;
            }
            let half_cells = b.width_cells as f32 * 0.5;
            b.frontage_t = (b.cell_x as f32 + half_cells) * cell_m / lengths_m[half].max(0.001);
            halves[half] += 1;
            moved.push(building_idx);
            if traffic_debug {
                crate::traffic_log!(
                    "[ROAD_SPLIT_DEPENDENTS] bldg={} action={} old_edge={} new_edge={} old_cell_x={} new_cell_x={} old_frontage_t={:.3} new_frontage_t={:.3}",
                    building_idx,
                    if half == 1 {
                        "move-to-new-edge"
                    } else {
                        "stay-on-old-edge"
                    },
                    edge_id,
                    b.edge_idx,
                    old_cell_x,
                    b.cell_x,
                    old_frontage_t,
                    b.frontage_t,
                );
            }
        }
        if !moved.is_empty() {
            self.record_building_changes(moved);
        }
        halves
    }

    /// Puts back the rows [`Self::migrate_split_frontages`] moved, from its `undo` entries,
    /// and records them. O(rows).
    pub(crate) fn restore_split_frontages(&mut self, rows: &[(usize, usize, usize, f32)]) {
        for &(index, edge_idx, cell_x, frontage_t) in rows {
            let building = &mut self.buildings[index];
            building.edge_idx = edge_idx;
            building.cell_x = cell_x;
            building.frontage_t = frontage_t;
        }
        if !rows.is_empty() {
            self.record_building_changes(rows.iter().map(|row| row.0));
        }
    }

    /// Sets building `index`'s service funding override and records the row, so the census
    /// input copy takes it (`SIM-02-I`). O(1).
    pub(crate) fn set_service_funding_override(&mut self, index: usize, funding: f32) {
        self.buildings[index].service_funding_override = funding;
        self.record_building_changes([index]);
    }

    /// Re-projects stale building frontage references onto nearby live road edges.
    pub(crate) fn repair_road_attachments_after_topology_edit(
        &mut self,
        graph: &RegionGraph,
        zoning: &mut ZoningSystem,
    ) -> usize {
        let zone_cell_m = zoning.config.zone_cell_m;
        let mut repaired = 0usize;
        for building_idx in 0..self.buildings.len() {
            if self.buildings[building_idx].parcel_id != 0 {
                if self.align_building_attachment_to_parcel(building_idx, graph, zoning) {
                    repaired += 1;
                }
                continue;
            }

            let frontage_center =
                building_frontage_center(&self.buildings[building_idx], zone_cell_m);
            if self.frontage_attachment_is_plausible(
                &self.buildings[building_idx],
                graph,
                frontage_center,
                zone_cell_m,
            ) {
                continue;
            }

            let width_m = self.buildings[building_idx].width_cells as f32 * zone_cell_m;
            let depth_m = self.buildings[building_idx].depth_cells as f32 * zone_cell_m;
            let Some(projection) =
                self.closest_repair_frontage(frontage_center, width_m, depth_m, graph)
            else {
                continue;
            };

            let edge_width = graph.edge(projection.edge_idx).width;
            let building = &mut self.buildings[building_idx];
            let changed = building.edge_idx != projection.edge_idx
                || building.side != projection.side
                || (building.frontage_t - projection.t).abs() > 0.001;
            if !changed {
                continue;
            }
            building.edge_idx = projection.edge_idx;
            building.side = projection.side;
            building.frontage_t = projection.t;
            building.side_offset = edge_width * 0.5 + SIDEWALK_WIDTH;
            repaired += 1;
        }

        if repaired > 0 {
            self.dirty = true;
            self.entrances_dirty = true;
            self.bump_building_ref_revision();
            debug_log!(
                "buildings",
                "frontage_attachment_repair repaired={}",
                repaired
            );
        }
        repaired
    }

    /// Updates only occupants of locally reattached cell lots; their transforms stay unchanged.
    pub(crate) fn sync_cell_lot_attachments(
        &mut self,
        parcels: &[crate::simulation::zoning::ParcelId],
        graph: &RegionGraph,
        zoning: &mut ZoningSystem,
    ) -> usize {
        let mut repaired = 0;
        // Publish the parallel zoning proposals in order through indexed owner lookups.
        // The mutable building/parcel stores are shared; only the changed local ids are visited.
        for &id in parcels {
            let Some(index) = zoning
                .parcels
                .get(id)
                .and_then(|parcel| parcel.cell_lot().and(parcel.occupied_building()))
            else {
                continue;
            };
            if self
                .buildings
                .get(index)
                .is_some_and(|building| building.parcel_id == id.raw())
                && self.align_building_attachment_to_parcel(index, graph, zoning)
            {
                repaired += 1;
            }
        }
        if repaired > 0 {
            self.dirty = true;
            self.entrances_dirty = true;
            self.bump_building_ref_revision();
        }
        repaired
    }

    fn align_building_attachment_to_parcel(
        &mut self,
        building_idx: usize,
        graph: &RegionGraph,
        zoning: &mut ZoningSystem,
    ) -> bool {
        let parcel_id = self.buildings[building_idx].parcel_id;
        let Some(parcel) = zoning.parcel_by_raw_id(parcel_id) else {
            return false;
        };
        let parcel_edge_idx = parcel.edge_idx();
        let parcel_side = parcel.side();
        let parcel_frontage_t = parcel.frontage_center_t();
        let parcel_frontage_m = parcel.frontage_m();
        let parcel_depth_m = parcel.depth_m();
        let cell_lot = parcel.cell_lot().is_some();
        let frontage_center =
            building_frontage_center(&self.buildings[building_idx], zoning.config.zone_cell_m);

        let mut target_edge_idx = parcel_edge_idx;
        let mut target_side = parcel_side;
        let mut target_frontage_t = parcel_frontage_t;
        let mut repaired_parcel = false;

        // Cell lots receive exact current suppliers from local zoning refresh. A nearby road
        // is not a substitute for missing frontage, and one lot may span several split edges.
        if !cell_lot
            && !self.parcel_attachment_is_plausible(
                graph,
                parcel_edge_idx,
                parcel_side,
                parcel_frontage_t,
                frontage_center,
                zoning.config.zone_cell_m,
            )
        {
            let Some(projection) = self.closest_repair_frontage(
                frontage_center,
                parcel_frontage_m,
                parcel_depth_m,
                graph,
            ) else {
                return false;
            };
            if zoning
                .repair_parcel_attachment(
                    parcel_id,
                    projection.edge_idx,
                    projection.side,
                    projection.t,
                    graph,
                )
                .is_err()
            {
                return false;
            }
            target_edge_idx = projection.edge_idx;
            target_side = projection.side;
            target_frontage_t = projection.t;
            repaired_parcel = true;
        };

        let Some(edge) = graph.get_edge(target_edge_idx) else {
            return false;
        };

        let building = &mut self.buildings[building_idx];
        let changed = building.edge_idx != target_edge_idx
            || building.side != target_side
            || if cell_lot {
                building.frontage_t != target_frontage_t
                    || building.side_offset != edge.width * 0.5 + SIDEWALK_WIDTH
            } else {
                (building.frontage_t - target_frontage_t).abs() > 0.001
            };
        if !changed {
            return repaired_parcel;
        }
        building.edge_idx = target_edge_idx;
        building.side = target_side;
        building.frontage_t = target_frontage_t;
        building.side_offset = edge.width * 0.5 + SIDEWALK_WIDTH;
        let zone_cell_m = zoning.config.zone_cell_m;
        let half_m = |cells: u16| f32::from(cells) * zone_cell_m * 0.5;
        let front_gap_m = super::geometry::lot_front_gaps(
            graph,
            Vector2::new(building.center_x, building.center_y),
            building.facing_dir,
            half_m(building.width_cells),
            half_m(building.depth_cells),
        );
        if building.front_gap_m != front_gap_m {
            building.front_gap_m = front_gap_m;
            let old_bounds = self.site_dirty_bounds(building_idx);
            self.rebuild_building_site_client(building_idx, zone_cell_m);
            self.record_building_site_change(building_idx, old_bounds);
            self.record_building_site_change(building_idx, self.site_dirty_bounds(building_idx));
        }
        true
    }

    fn parcel_attachment_is_plausible(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        side: i8,
        frontage_t: f32,
        frontage_center: Vector2,
        zone_cell_m: f32,
    ) -> bool {
        let Some(connection_pos) =
            Self::road_side_connection_pos(graph, edge_idx, side, frontage_t)
        else {
            return false;
        };
        let tolerance_m = (zone_cell_m * 2.0).max(FRONTAGE_ATTACHMENT_VALID_MIN_DISTANCE_M);
        connection_pos.distance_squared_to(frontage_center) <= tolerance_m * tolerance_m
    }

    fn frontage_attachment_is_plausible(
        &self,
        building: &crate::simulation::buildings::allocator::Building,
        graph: &RegionGraph,
        frontage_center: Vector2,
        zone_cell_m: f32,
    ) -> bool {
        let Some(connection_pos) = Self::road_side_connection_pos(
            graph,
            building.edge_idx,
            building.side,
            building.frontage_t,
        ) else {
            return false;
        };
        let tolerance_m = (zone_cell_m * 2.0).max(FRONTAGE_ATTACHMENT_VALID_MIN_DISTANCE_M);
        connection_pos.distance_squared_to(frontage_center) <= tolerance_m * tolerance_m
    }

    fn closest_repair_frontage(
        &self,
        frontage_center: Vector2,
        width_m: f32,
        depth_m: f32,
        graph: &RegionGraph,
    ) -> Option<super::geometry::RoadFrontageProjection> {
        let search_radius = (width_m * 0.5 + depth_m + FRONTAGE_ATTACHMENT_REPAIR_SEARCH_MARGIN_M)
            .max(FRONTAGE_ATTACHMENT_REPAIR_MIN_SEARCH_M);
        let mut candidates = graph.get_edges_near_point(
            Vector3::new(frontage_center.x, 0.0, frontage_center.y),
            search_radius,
        );
        candidates.sort_unstable();
        candidates.dedup();

        let mut best: Option<super::geometry::RoadFrontageProjection> = None;
        for edge_idx in candidates {
            let Some(edge) = graph.get_edge(edge_idx) else {
                continue;
            };
            if edge.deleted
                || edge.no_building_spawn
                || edge.physical_geometry.len() < 2
                || edge.physical_length <= 1e-6
            {
                continue;
            }
            let Some(projection) =
                Self::project_point_to_edge_centerline(edge_idx, edge, frontage_center)
            else {
                continue;
            };
            let Some(connection_pos) =
                Self::road_side_connection_pos(graph, edge_idx, projection.side, projection.t)
            else {
                continue;
            };
            let dist_sq = connection_pos.distance_squared_to(frontage_center);
            if dist_sq > search_radius * search_radius {
                continue;
            }
            let candidate = super::geometry::RoadFrontageProjection {
                edge_idx,
                t: projection.t,
                side: projection.side,
                dist_sq,
            };
            if best.as_ref().is_none_or(|best| {
                candidate
                    .dist_sq
                    .total_cmp(&best.dist_sq)
                    .then(candidate.edge_idx.cmp(&best.edge_idx))
                    .is_lt()
            }) {
                best = Some(candidate);
            }
        }
        best
    }

    fn road_side_connection_pos(
        graph: &RegionGraph,
        edge_idx: usize,
        side: i8,
        edge_t: f32,
    ) -> Option<Vector2> {
        let edge = graph.get_edge(edge_idx)?;
        if edge.deleted || edge.no_building_spawn || edge.physical_geometry.len() < 2 {
            return None;
        }
        let center = Self::sample_pos_on_edge(graph, edge_idx, edge_t);
        let tangent = Self::sample_tangent_on_edge(graph, edge_idx, edge_t);
        if tangent.length_squared() <= 1e-12 {
            return None;
        }
        let normal = Vector2::new(tangent.y, -tangent.x) * side as f32;
        Some(center + normal * (edge.width * 0.5 + SIDEWALK_WIDTH))
    }
}

fn building_frontage_center(
    building: &crate::simulation::buildings::allocator::Building,
    zone_cell_m: f32,
) -> Vector2 {
    let center = Vector2::new(building.center_x, building.center_y);
    let facing = if building.facing_dir.length_squared() > 1e-12 {
        building.facing_dir.normalized()
    } else {
        Vector2::ZERO
    };
    center + facing * (building.depth_cells as f32 * zone_cell_m * 0.5)
}

impl BuildingAllocator {
    /// Returns the next demand-owned household admission target without mutating occupancy.
    ///
    /// Selection uses the residential vacancy index, prefers the smallest deterministic starter
    /// household currently claimable, and preserves vacancy-index order as the tie-breaker.
    #[cfg(test)]
    pub(crate) fn next_household_admission_candidate(&self) -> Option<(usize, u16)> {
        self.next_household_admission_candidate_for_household(0, false)
    }

    /// Returns the next demand-owned household target for a specific future household id.
    ///
    /// Picks the smallest `(worker rank, offered size, vacancy position)` through the admission
    /// order (`ALLOC-03`). A stale index may list moved buildings, so it is scanned as before.
    #[cfg(test)]
    pub(crate) fn next_household_admission_candidate_for_household(
        &self,
        next_household_id: usize,
        prefer_worker_capable: bool,
    ) -> Option<(usize, u16)> {
        let residential_slot = baseline_private_zone_slot(ZoneType::Residential)?;
        let vacancies = &self.vacancy_index[residential_slot];
        if self.dirty_index || !self.admission_order.covers(vacancies.len()) {
            return self.scan_household_admission_candidate(
                vacancies,
                next_household_id,
                prefer_worker_capable,
            );
        }
        self.admission_order
            .pick(
                vacancies,
                |building| self.applicant_seed(building),
                next_household_id,
                prefer_worker_capable,
            )
            .map(|(position, size)| (vacancies[position], size))
    }

    // The admission rule as a walk over every vacancy. O(V).
    #[cfg(test)]
    pub(super) fn scan_household_admission_candidate(
        &self,
        vacancies: &[usize],
        next_household_id: usize,
        prefer_worker_capable: bool,
    ) -> Option<(usize, u16)> {
        let mut selected_home_idx = usize::MAX;
        let mut selected_size = u16::MAX;
        let mut selected_order = usize::MAX;
        let mut selected_worker_rank = u8::MAX;

        for (order, &building_idx) in vacancies.iter().enumerate() {
            let Some(candidate_size) = self.admission_size(building_idx) else {
                continue;
            };

            let worker_rank = if prefer_worker_capable {
                let composition = crate::simulation::economy::agents::household_age_composition(
                    self.applicant_seed(building_idx),
                    next_household_id,
                    candidate_size,
                );
                if composition.adult_count > 0 { 0 } else { 1 }
            } else {
                0
            };

            if (worker_rank, candidate_size, order)
                < (selected_worker_rank, selected_size, selected_order)
            {
                selected_home_idx = building_idx;
                selected_size = candidate_size;
                selected_order = order;
                selected_worker_rank = worker_rank;
            }
        }

        (selected_home_idx != usize::MAX).then_some((selected_home_idx, selected_size))
    }
}

impl BuildingAllocator {
    /// The building a planned action names, when it still stands as planned. Expected O(1).
    pub(crate) fn resolve_demand_building_action(
        &self,
        action: &DemandBuildingActionKey,
        zoning: &ZoningSystem,
    ) -> Option<usize> {
        // Stable parcel ownership already follows swap-remove and level changes. Reuse it
        // for expected O(1) lookup instead of cloning a city-wide action-key table.
        let building_idx = zoning
            .parcel_by_raw_id(action.parcel_id)?
            .occupied_building()?;
        action
            .matches_building(self.buildings.get(building_idx)?)
            .then_some(building_idx)
    }

    /// Whether planned clearance may remove building `building_idx` now: deserted, sound,
    /// complete, not awaiting redevelopment, with no occupants or workers. O(1).
    pub(crate) fn can_demand_despawn(&self, building_idx: usize) -> bool {
        let Some(building) = self.buildings.get(building_idx) else {
            return false;
        };
        building.is_deserted
            && !building.broken
            && !building.economy_broken
            && !building.is_under_construction()
            && !building.pending_redevelopment
            && building.occupancy == 0
            && building.worker_count == 0
    }

    /// Whether `target_asset_id` can replace building `building_idx` in place now: the same lot
    /// and zone, room for its occupants and workers, a valid economy profile for a business and
    /// a valid site. O(site check).
    pub(crate) fn level_change_fits(
        &self,
        building_idx: usize,
        target_asset_id: &str,
        catalog: &RuntimeEconomyCatalog,
        zone_cell_m: f32,
        graph: &RegionGraph,
        environment: super::BuildingSiteEnvironment<'_>,
    ) -> bool {
        self.level_change_target(
            building_idx,
            target_asset_id,
            catalog,
            zone_cell_m,
            graph,
            environment,
        )
        .is_some()
    }

    // The target's level and economy binding when it fits ([`Self::level_change_fits`]).
    fn level_change_target(
        &self,
        building_idx: usize,
        target_asset_id: &str,
        catalog: &RuntimeEconomyCatalog,
        zone_cell_m: f32,
        graph: &RegionGraph,
        environment: super::BuildingSiteEnvironment<'_>,
    ) -> Option<(u8, super::EconomyProfileBinding)> {
        let building = self.buildings.get(building_idx)?;
        if building.broken || building.pending_redevelopment {
            return None;
        }

        let target_entry = self.registry.get(target_asset_id)?;
        let target_building = target_entry.manifest.building.as_ref()?;
        if !target_building.is_zoned_private() {
            return None;
        }
        if target_building.lot_width_cells != building.width_cells
            || target_building.lot_depth_cells != building.depth_cells
            || self.registry.household_capacity(target_asset_id) < building.occupancy
        {
            return None;
        }

        let target_zone_type = zone_class_to_zone_type(target_building.zone_type?);
        if target_zone_type != building.zone_type {
            return None;
        }
        let economy_binding = resolve_building_economy_profile_binding_with_catalog(
            &self.registry,
            catalog,
            target_asset_id,
        );
        if matches!(
            target_zone_type,
            ZoneType::Commercial | ZoneType::Industrial
        ) && (economy_binding.economy_broken || economy_binding.runtime_id == 0)
        {
            return None;
        }
        let target_worker_capacity = self
            .worker_capacity_for_asset_with_catalog(target_asset_id, catalog)
            .unwrap_or(0);
        if target_worker_capacity < building.worker_count {
            return None;
        }
        if !self.replacement_site_is_valid(
            building_idx,
            target_asset_id,
            zone_cell_m,
            graph,
            environment,
        ) {
            return None;
        }
        Some((target_building.level, economy_binding))
    }

    /// Replaces building `building_idx` in place by `target_asset_id` when it fits
    /// ([`Self::level_change_fits`]): asset, level and economy profile, its site and yard; its
    /// occupants and workers stay. Records the row for the census input copy, an in-place
    /// change that restarts no census. O(site check).
    pub(crate) fn apply_level_change_action(
        &mut self,
        building_idx: usize,
        target_asset_id: &str,
        catalog: &RuntimeEconomyCatalog,
        zone_cell_m: f32,
        graph: &RegionGraph,
        environment: super::BuildingSiteEnvironment<'_>,
    ) -> Option<()> {
        let (target_level, economy_binding) = self.level_change_target(
            building_idx,
            target_asset_id,
            catalog,
            zone_cell_m,
            graph,
            environment,
        )?;
        // Yards are laid from the asset's hedge, planting, surfaces and structures, so a
        // replacement that touches any authored yard clears the old one and lays the target's.
        let refresh_yard = [
            self.buildings[building_idx].asset_id.as_str(),
            target_asset_id,
        ]
        .into_iter()
        .filter_map(|asset| self.registry.get(asset)?.manifest.building.as_ref())
        .any(|data| data.yard_hedge.is_some() || !data.yard_planting.is_empty());
        let old_site_bounds = self.site_dirty_bounds(building_idx);
        let building = &mut self.buildings[building_idx];
        building.asset_id = target_asset_id.to_owned();
        building.level = target_level;
        building.economy_profile_runtime_id = economy_binding.runtime_id;
        building.economy_broken = economy_binding.economy_broken;
        let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
        building.retain_inventory_for_profile(profile, catalog.resource_count());
        building.pending_redevelopment = false;
        building.rezone_grace_days_remaining = 0;
        let zone_type = building.zone_type;
        self.rebuild_building_site_client(building_idx, zone_cell_m);
        if refresh_yard {
            // The yard key is the parcel and generation, which a level change keeps.
            self.queue_yard_removal(building_idx);
            self.queue_yard(building_idx, zone_cell_m);
        }
        let building = &self.buildings[building_idx];
        self.level_changes
            .record((building.parcel_id, building.build_generation));
        self.record_building_site_change(building_idx, old_site_bounds);
        self.record_building_site_change(building_idx, self.site_dirty_bounds(building_idx));
        self.record_building_changes([building_idx]);
        self.dirty = true;
        self.mark_index_stale();
        self.entrances_dirty = true;
        if let Some(zone_idx) = baseline_private_zone_slot(zone_type) {
            self.dirty_zones[zone_idx] = true;
        }
        Some(())
    }

    // Removes building `building_idx` and repairs every reference to it and to the last
    // building, which the swap-remove moves into its index. `candidates`, when given, are the
    // agents `AgentSystem::agents_referencing_buildings` found naming either before anything
    // else changed agents' references; eviction and the agent remap then visit them alone,
    // unless a cancelled shipment removed a carrier meanwhile (the remap then scans every
    // agent).
    #[allow(clippy::too_many_arguments)]
    fn remove_building_at_index(
        &mut self,
        building_idx: usize,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
        treasury_balance: &mut f64,
        candidates: Option<&[usize]>,
    ) -> bool {
        let Some((parcel_id, zone_type)) = self
            .buildings
            .get(building_idx)
            .map(|building| (building.parcel_id, building.zone_type))
        else {
            return false;
        };
        // A clean entrance cache follows the swap-remove below: each entrance is derived from
        // its own building, the network and the asset registry alone (`SIM-02-K`).
        let entrances_clean = !self.entrances_dirty && self.entrances.len() == self.buildings.len();
        self.record_building_site_change(building_idx, self.site_dirty_bounds(building_idx));
        zoning.clear_parcel_occupancy(parcel_id);

        let last_idx = self.buildings.len().saturating_sub(1);
        // The households naming either building, found as the agents were: the invalidation
        // and the remap below change no other household.
        let household_candidates =
            candidates.map(|_| households.households_referencing_buildings(building_idx, last_idx));
        match candidates {
            Some(candidates) => agents.evict_building_among(building_idx, candidates),
            None => agents.evict_building(building_idx),
        }
        let agent_count = agents.agents.len();
        match &household_candidates {
            Some(candidates) => {
                households.invalidate_building_among(building_idx, candidates, self)
            }
            None => households.invalidate_building(building_idx, self),
        }
        logistics.invalidate_building(building_idx, self, agents, treasury_balance);
        let city_owned = self.is_city_service_building(&self.buildings[building_idx]);
        households.close_building_electricity(
            &mut self.buildings[building_idx],
            city_owned,
            treasury_balance,
        );
        if let Some(zone_idx) = baseline_private_zone_slot(zone_type) {
            self.dirty_zones[zone_idx] = true;
        }

        if building_idx < last_idx {
            let moved_building = &self.buildings[last_idx];
            if let Some(zone_idx) = baseline_private_zone_slot(moved_building.zone_type) {
                self.dirty_zones[zone_idx] = true;
            }
            let mut mapping = std::collections::HashMap::new();
            mapping.insert(last_idx, building_idx);
            match candidates {
                // A removed carrier renumbered agents; the candidates no longer name them.
                Some(candidates) if agents.agents.len() == agent_count => {
                    agents.remap_building_among(last_idx, building_idx, candidates)
                }
                _ => agents.remap_building_indices(&mapping),
            }
            match &household_candidates {
                Some(candidates) => {
                    households.remap_building_among(last_idx, building_idx, candidates)
                }
                None => households.remap_building_indices(&mapping),
            }
            logistics.remap_building_indices(&mapping);
            zoning.remap_parcel_occupancy(moved_building.parcel_id, last_idx, building_idx);
        }

        self.field_clearance
            .remove_and_remap(building_idx, last_idx);
        self.record_production_site_removal(building_idx, last_idx);
        self.queue_yard_removal(building_idx);
        crate::debug::money::record_change(
            crate::debug::money::MoneyFlow::RemovedBuildingCash,
            self.buildings[building_idx].operating_budget,
            0.0,
        );
        self.building_handles
            .swap_remove(building_idx, self.buildings.len());
        self.keep_index_across_swap_remove(building_idx);
        self.buildings.swap_remove(building_idx);
        crate::debug::work::add(crate::debug::work::Work::BuildingsRemoved, 1);
        self.remove_building_site_client(building_idx);
        self.record_building_changes([building_idx, last_idx]);
        self.dirty = true;
        self.dirty_index = true;
        if entrances_clean {
            // As `rebuild_entrance_cache` leaves it, without deriving every entrance again.
            self.entrances.swap_remove(building_idx);
            self.bump_entrance_ref_revision();
            self.dirty_zones = [true; 3];
        } else {
            self.entrances_dirty = true;
        }
        true
    }

    /// Removes one building through the normal lifecycle hooks used by demand redevelopment.
    pub(crate) fn remove_building_for_bulldoze(
        &mut self,
        building_idx: usize,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
        treasury_balance: &mut f64,
    ) -> bool {
        self.remove_building_at_index(
            building_idx,
            zoning,
            agents,
            households,
            logistics,
            treasury_balance,
            None,
        )
    }
}

#[cfg(test)]
mod clearance_tests {
    use super::*;
    use crate::simulation::buildings::allocator::indexed_test_building;

    #[test]
    fn clearance_execution_rechecks_abandonment_occupancy_and_construction() {
        let mut allocator = BuildingAllocator::new();
        let mut building = indexed_test_building("test:home".to_owned(), ZoneType::Residential, 0);
        building.is_deserted = true;
        allocator.buildings.push(building);
        assert!(allocator.can_demand_despawn(0));
        allocator.buildings[0].is_deserted = false;
        assert!(
            !allocator.can_demand_despawn(0),
            "a recovered site must survive a stale plan"
        );
        allocator.buildings[0].is_deserted = true;
        allocator.buildings[0].occupancy = 1;
        assert!(!allocator.can_demand_despawn(0));
        allocator.buildings[0].occupancy = 0;
        allocator.buildings[0].worker_count = 1;
        assert!(!allocator.can_demand_despawn(0));
        allocator.buildings[0].worker_count = 0;
        allocator.buildings[0].set_construction_hours_left(1);
        assert!(!allocator.can_demand_despawn(0));
    }
}

#[cfg(test)]
mod border_arrival_tests {
    use super::*;
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::network::types::EdgeClass;

    // The border list and every cost of kept tables, as bits.
    fn tables(arrival: &BorderArrivalCosts, graph: &RegionGraph) -> (Vec<u32>, Vec<Option<u32>>) {
        let costs = arrival
            .costs
            .iter()
            .flat_map(|costs| (0..graph.node_count() as u32).map(|node| costs.cost_to(node)))
            .map(|cost| cost.map(f32::to_bits))
            .collect();
        (arrival.borders.clone(), costs)
    }

    #[test]
    fn kept_border_arrival_costs_follow_the_routing_revisions() {
        let mut allocator = BuildingAllocator::new();
        let mut zoning = ZoningSystem::new(&WorldConfig::default());
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        network.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            1,
            1,
            EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
        graph.set_node_type(0, NodeType::Border);
        network.rebuild_pathing(&mut graph);
        let mut arrival = BorderArrivalCosts::default();
        arrival.refresh(&network, &graph);
        assert!(arrival.key.is_some(), "a current network keeps its sweeps");
        assert_eq!(arrival.borders, [0]);
        assert_eq!(
            tables(&arrival, &graph),
            tables(&BorderArrivalCosts::sweep(&network, &graph), &graph)
        );

        // A border role set without a topology edit moves the node-role revision only.
        let far_end = graph.edge(0).end_node;
        graph.set_node_type(far_end, NodeType::Border);
        arrival.refresh(&network, &graph);
        assert_eq!(arrival.borders, [0, far_end]);
        assert_eq!(
            tables(&arrival, &graph),
            tables(&BorderArrivalCosts::sweep(&network, &graph), &graph)
        );

        // While an edit waits for its CCH rebuild the sweeps are taken fresh and not kept.
        network.mark_point_dirty(Vector3::new(0.0, 0.0, 0.0));
        arrival.refresh(&network, &graph);
        assert!(arrival.key.is_none());
        network.rebuild_pathing(&mut graph);
        let generation = network.cch_graph.build_generation;
        arrival.refresh(&network, &graph);
        assert_eq!(
            arrival.key,
            Some((
                generation,
                network.cch_graph.metric_revision,
                graph.node_role_revision()
            ))
        );
    }
}
