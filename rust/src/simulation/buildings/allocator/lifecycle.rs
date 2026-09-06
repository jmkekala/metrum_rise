// SPDX-License-Identifier: GPL-2.0-only

//! Building removal, demand-owned household admission, and coordinate restoration.

use crate::config::SIDEWALK_WIDTH;
use crate::debug_log;
use crate::simulation::buildings::allocator::{
    BuildingAllocator, DemandSpawnPlacementRejection, baseline_private_zone_slot,
    resolve_building_economy_profile_binding_with_catalog, zone_class_to_zone_type,
};
use crate::simulation::economy::agents::{AgentSystem, household_age_composition};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::demand::{
    DemandBuildingActionKey, DemandBuildingActionPlan, DemandLevelChangeAction, DemandSpawnAction,
    demand_building_action_key,
};
use crate::simulation::economy::households::{
    HouseholdSystem, candidate_immigrant_household_size_for_vacancy,
};
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneSystem;
use crate::simulation::network::surface::RoadSurfaceSystem;
use crate::simulation::network::types::{NodeType, TransitFlags, TransitType};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::zoning::ZoneType;
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::{Vector2, Vector3};

const REZONE_GRACE_DAYS: u8 = 3;
const FRONTAGE_ATTACHMENT_REPAIR_MIN_SEARCH_M: f32 = 50.0;
const FRONTAGE_ATTACHMENT_REPAIR_SEARCH_MARGIN_M: f32 = 20.0;
const FRONTAGE_ATTACHMENT_VALID_MIN_DISTANCE_M: f32 = 6.0;

/// Summary of building mutations performed by one demand-owned building action pass.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DemandBuildingActionExecution {
    /// World-space bounds of building-site terrain patches dirtied by this action pass.
    pub(crate) site_dirty_bounds: Option<(f32, f32, f32, f32)>,
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

    pub(crate) fn geometry_total(self) -> usize {
        self.total()
            .saturating_sub(self.asset_unavailable)
            .saturating_sub(self.parcel_unavailable)
    }
}

impl BuildingAllocator {
    /// Removes buildings if their zone category has changed or their road edge no longer exists.
    pub(super) fn cleanup_stale_buildings(
        &mut self,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
        graph: &RegionGraph,
        lanes: &LaneSystem,
    ) {
        let mut removed_any = false;
        let mut i = 0;
        while i < self.buildings.len() {
            let compatibility = {
                let b = &self.buildings[i];
                let edge_ok = b.edge_idx < graph.edge_count() && !graph.edge(b.edge_idx).deleted;
                if !edge_ok {
                    None
                } else if graph.edge(b.edge_idx).no_building_spawn {
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
                                                let width_m = b.width_cells as f32
                                                    * zoning.config.zone_cell_m;
                                                let depth_m = b.depth_cells as f32
                                                    * zoning.config.zone_cell_m;
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
                        if building.rezone_grace_days_remaining > 0 {
                            building.rezone_grace_days_remaining -= 1;
                        }
                        building.rezone_grace_days_remaining == 0
                    }
                }
            };

            if remove {
                let b = &self.buildings[i];
                let b_parcel_id = b.parcel_id;
                let b_zone = b.zone_type;
                let removed_site_bounds = self.site_world_bounds(i);
                if let Some(zone_idx) = baseline_private_zone_slot(b_zone) {
                    self.dirty_zones[zone_idx] = true;
                }
                zoning.clear_parcel_occupancy(b_parcel_id);

                agents.evict_building(i);
                households.invalidate_building(i, self);
                logistics.invalidate_building(i, self, agents);
                let last_idx = self.buildings.len() - 1;
                if i < last_idx {
                    if let Some(zone_idx) =
                        baseline_private_zone_slot(self.buildings[last_idx].zone_type)
                    {
                        self.dirty_zones[zone_idx] = true;
                    }
                    let mut mapping = std::collections::HashMap::new();
                    mapping.insert(last_idx, i);
                    agents.remap_building_indices(&mapping);
                    households.remap_building_indices(&mapping);
                    logistics.remap_building_indices(&mapping);
                    zoning.remap_parcel_occupancy(last_idx, i);
                }

                self.buildings.swap_remove(i);
                if self.building_sites.len() > i {
                    self.building_sites.swap_remove(i);
                    self.recompute_max_site_radius_m();
                }
                if let Some(bounds) = removed_site_bounds {
                    self.accumulate_pending_site_dirty_bounds(Some(bounds));
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
                self.dirty_index = true;
                self.entrances_dirty = true;
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

    /// Admits the already-decided demand-owned household count as border-origin arrival carriers.
    pub(super) fn admit_households_from_demand(
        &mut self,
        households_to_spawn: usize,
        next_household_id: usize,
        prefer_worker_capable: bool,
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> usize {
        if households_to_spawn == 0 {
            return 0;
        }
        for category in ["economy", "spawn"] {
            debug_log!(
                category,
                "demand-owned household admission planning: households_to_spawn={}",
                households_to_spawn,
            );
        }
        let mut launched = 0;
        for _ in 0..households_to_spawn {
            let Some((home_idx, household_size)) = self.claim_home_for_household(
                next_household_id.saturating_add(launched),
                prefer_worker_capable,
            ) else {
                for category in ["economy", "spawn"] {
                    debug_log!(
                        category,
                        "demand-owned household admission stopped early: could not claim a home from vacancy index"
                    );
                }
                break;
            };
            let Some(border_node) =
                self.household_arrival_border_node(home_idx, transit_network, graph)
            else {
                for category in ["economy", "spawn"] {
                    debug_log!(
                        category,
                        "demand-owned household admission waiting: no legal border-to-home car route for home_building={}",
                        home_idx
                    );
                }
                break;
            };
            // One household consumes 1 slot of household_capacity regardless of size.
            self.claim_vacancy(home_idx);
            let border_pos = graph.node(border_node).pos;
            let carrier_idx = agents.spawn_household_arrival_carrier(
                home_idx,
                household_size,
                border_node,
                border_pos.x,
                border_pos.z,
            );
            launched += 1;
            for category in ["economy", "spawn"] {
                debug_log!(
                    category,
                    "demand-owned household admission launched carrier_agent={} size={} home_building={} border_node={}",
                    carrier_idx,
                    household_size,
                    home_idx,
                    border_node,
                );
            }
        }
        launched
    }

    fn household_arrival_border_node(
        &self,
        home_idx: usize,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<u32> {
        let mut best: Option<(u32, f32)> = None;
        for (idx, node) in graph.nodes().iter().enumerate() {
            if node.node_type != NodeType::Border {
                continue;
            }
            let border_node = idx as u32;
            let has_car_connection = graph.node_adjacency(border_node).iter().any(|&edge_idx| {
                let edge = graph.edge(edge_idx);
                !edge.deleted
                    && edge.primary_type == TransitType::Road
                    && (edge.allowed_types & TransitFlags::CAR) != 0
            });
            if !has_car_connection {
                continue;
            }
            let Some(eta_s) = self.freight_car_eta_from_border_node(
                border_node,
                home_idx,
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

    pub(crate) fn execute_demand_building_actions(
        &mut self,
        plan: &DemandBuildingActionPlan,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
        graph: &RegionGraph,
        lanes: &LaneSystem,
        road_surface: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> DemandBuildingActionExecution {
        let mut action_lookup: std::collections::HashMap<DemandBuildingActionKey, usize> = self
            .buildings
            .iter()
            .enumerate()
            .map(|(idx, building)| (demand_building_action_key(building), idx))
            .collect();
        let mut mutated_any = false;
        let mut execution = DemandBuildingActionExecution {
            site_dirty_bounds: None,
            residential: DemandUseBuildingActionExecution::default(),
            commercial: DemandUseBuildingActionExecution::default(),
            industrial: DemandUseBuildingActionExecution::default(),
        };

        for (zone_type, use_plan) in [
            (ZoneType::Residential, &plan.residential),
            (ZoneType::Commercial, &plan.commercial),
            (ZoneType::Industrial, &plan.industrial),
        ] {
            for action in &use_plan.despawns {
                let Some(building_idx) = action_lookup.remove(action) else {
                    continue;
                };
                if !self.can_demand_despawn(building_idx) {
                    continue;
                }
                accumulate_site_dirty_bounds(
                    &mut execution.site_dirty_bounds,
                    self.site_world_bounds(building_idx),
                );
                if let Some((moved_key, moved_idx)) = self.remove_building_at_index(
                    building_idx,
                    zoning,
                    agents,
                    households,
                    logistics,
                ) {
                    action_lookup.insert(moved_key, moved_idx);
                }
                mutated_any = true;
            }

            for action in &use_plan.downgrades {
                let Some(&building_idx) = action_lookup.get(&action.building) else {
                    continue;
                };
                let old_site_bounds = self.site_world_bounds(building_idx);
                if let Some(updated_key) = self.apply_level_change_action(
                    building_idx,
                    action,
                    catalog,
                    zoning.config.zone_cell_m,
                ) {
                    accumulate_site_dirty_bounds(&mut execution.site_dirty_bounds, old_site_bounds);
                    accumulate_site_dirty_bounds(
                        &mut execution.site_dirty_bounds,
                        self.site_world_bounds(building_idx),
                    );
                    action_lookup.remove(&action.building);
                    action_lookup.insert(updated_key, building_idx);
                    mutated_any = true;
                }
            }

            for action in &use_plan.upgrades {
                let Some(&building_idx) = action_lookup.get(&action.building) else {
                    continue;
                };
                let old_site_bounds = self.site_world_bounds(building_idx);
                if let Some(updated_key) = self.apply_level_change_action(
                    building_idx,
                    action,
                    catalog,
                    zoning.config.zone_cell_m,
                ) {
                    accumulate_site_dirty_bounds(&mut execution.site_dirty_bounds, old_site_bounds);
                    accumulate_site_dirty_bounds(
                        &mut execution.site_dirty_bounds,
                        self.site_world_bounds(building_idx),
                    );
                    action_lookup.remove(&action.building);
                    action_lookup.insert(updated_key, building_idx);
                    mutated_any = true;
                }
            }

            for action in &use_plan.spawns {
                execution.use_mut(zone_type).spawn_attempted += 1;
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
                        self.buildings[building_idx].profit_tax_budget_baseline =
                            self.buildings[building_idx].operating_budget;
                        accumulate_site_dirty_bounds(
                            &mut execution.site_dirty_bounds,
                            self.site_world_bounds(building_idx),
                        );
                        mutated_any = true;
                    }
                    Err(reason) => {
                        execution.use_mut(zone_type).spawn_rejections.record(reason);
                    }
                }
            }
        }

        if mutated_any {
            if self.dirty_index {
                self.rebuild_zone_index();
            }
            self.rebuild_entrance_cache(graph, lanes);
        }
        execution
    }

    /// Executes one queued demand spawn through the same final placement path as batch demand.
    pub(crate) fn execute_single_demand_spawn_action(
        &mut self,
        zone_type: ZoneType,
        action: &DemandSpawnAction,
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
                self.buildings[building_idx].profit_tax_budget_baseline =
                    self.buildings[building_idx].operating_budget;
                accumulate_site_dirty_bounds(
                    &mut execution.site_dirty_bounds,
                    self.site_world_bounds(building_idx),
                );
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
        }

        self.dirty = true;
        self.rebuild_building_site_clients(zoning.config.zone_cell_m);
        self.bump_building_ref_revision();
        Ok(())
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
        let frontage_center =
            building_frontage_center(&self.buildings[building_idx], zoning.config.zone_cell_m);

        let mut target_edge_idx = parcel_edge_idx;
        let mut target_side = parcel_side;
        let mut target_frontage_t = parcel_frontage_t;
        let mut repaired_parcel = false;

        if !self.parcel_attachment_is_plausible(
            graph,
            parcel_edge_idx,
            parcel_side,
            parcel_frontage_t,
            frontage_center,
            zoning.config.zone_cell_m,
        ) {
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
            || (building.frontage_t - target_frontage_t).abs() > 0.001;
        if !changed {
            return repaired_parcel;
        }
        building.edge_idx = target_edge_idx;
        building.side = target_side;
        building.frontage_t = target_frontage_t;
        building.side_offset = edge.width * 0.5 + SIDEWALK_WIDTH;
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
    pub(crate) fn next_household_admission_candidate_for_household(
        &self,
        next_household_id: usize,
        prefer_worker_capable: bool,
    ) -> Option<(usize, u16)> {
        let residential_slot = baseline_private_zone_slot(ZoneType::Residential)?;
        let mut selected_home_idx = usize::MAX;
        let mut selected_size = u16::MAX;
        let mut selected_order = usize::MAX;
        let mut selected_worker_rank = u8::MAX;

        for (order, &building_idx) in self.vacancy_index[residential_slot].iter().enumerate() {
            let Some(building) = self.buildings.get(building_idx) else {
                continue;
            };
            let free_slots = self
                .household_capacity(building_idx)
                .saturating_sub(building.occupancy);
            if free_slots == 0 {
                continue;
            }

            let Some(candidate_size) = candidate_immigrant_household_size_for_vacancy(
                self.flat_size_m2(building_idx),
                building_idx,
                building.occupancy,
            ) else {
                continue;
            };

            let worker_rank = if prefer_worker_capable {
                let composition =
                    household_age_composition(building_idx, next_household_id, candidate_size);
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

    fn claim_home_for_household(
        &mut self,
        next_household_id: usize,
        prefer_worker_capable: bool,
    ) -> Option<(usize, u16)> {
        let (fallback_idx, fallback_size) = self.next_household_admission_candidate_for_household(
            next_household_id,
            prefer_worker_capable,
        )?;
        // Note: vacancy count for residential is now household-based.
        // The vacancy is claimed by the caller in admit_households_from_demand or relocation.
        Some((fallback_idx, fallback_size))
    }
}

impl BuildingAllocator {
    fn can_demand_despawn(&self, building_idx: usize) -> bool {
        let Some(building) = self.buildings.get(building_idx) else {
            return false;
        };
        !building.broken
            && !building.pending_redevelopment
            && building.occupancy == 0
            && building.worker_count == 0
    }

    fn apply_level_change_action(
        &mut self,
        building_idx: usize,
        action: &DemandLevelChangeAction,
        catalog: &RuntimeEconomyCatalog,
        zone_cell_m: f32,
    ) -> Option<DemandBuildingActionKey> {
        let building = self.buildings.get(building_idx)?;
        if building.broken || building.pending_redevelopment {
            return None;
        }
        if demand_building_action_key(building) != action.building {
            return None;
        }

        let target_entry = self.registry.get(&action.target_asset_id)?;
        let target_building = target_entry.manifest.building.as_ref()?;
        if !target_building.is_zoned_private() {
            return None;
        }
        if target_building.lot_width_cells != building.width_cells
            || target_building.lot_depth_cells != building.depth_cells
            || self.registry.household_capacity(&action.target_asset_id) < building.occupancy
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
            &action.target_asset_id,
        );
        if matches!(
            target_zone_type,
            ZoneType::Commercial | ZoneType::Industrial
        ) && (economy_binding.economy_broken || economy_binding.runtime_id == 0)
        {
            return None;
        }
        let target_worker_capacity = self
            .worker_capacity_for_asset_with_catalog(&action.target_asset_id, catalog)
            .unwrap_or(0);
        if target_worker_capacity < building.worker_count {
            return None;
        }
        let building = &mut self.buildings[building_idx];
        building.asset_id = action.target_asset_id.clone();
        building.level = target_building.level;
        building.economy_profile_runtime_id = economy_binding.runtime_id;
        building.economy_broken = economy_binding.economy_broken;
        let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
        building.retain_inventory_for_profile(profile, catalog.resource_count());
        building.pending_redevelopment = false;
        building.rezone_grace_days_remaining = 0;
        let zone_type = building.zone_type;
        let updated_key = demand_building_action_key(building);
        let _ = building;
        self.rebuild_building_site_client(building_idx, zone_cell_m);
        self.bump_building_ref_revision();
        self.dirty = true;
        self.dirty_index = true;
        self.entrances_dirty = true;
        if let Some(zone_idx) = baseline_private_zone_slot(zone_type) {
            self.dirty_zones[zone_idx] = true;
        }
        Some(updated_key)
    }

    fn remove_building_at_index(
        &mut self,
        building_idx: usize,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
    ) -> Option<(DemandBuildingActionKey, usize)> {
        let building = self.buildings.get(building_idx)?.clone();
        zoning.clear_parcel_occupancy(building.parcel_id);

        agents.evict_building(building_idx);
        households.invalidate_building(building_idx, self);
        logistics.invalidate_building(building_idx, self, agents);
        if let Some(zone_idx) = baseline_private_zone_slot(building.zone_type) {
            self.dirty_zones[zone_idx] = true;
        }

        let last_idx = self.buildings.len().saturating_sub(1);
        let moved_key = if building_idx < last_idx {
            let moved_building = self.buildings[last_idx].clone();
            let moved_key = demand_building_action_key(&moved_building);
            if let Some(zone_idx) = baseline_private_zone_slot(moved_building.zone_type) {
                self.dirty_zones[zone_idx] = true;
            }
            let mut mapping = std::collections::HashMap::new();
            mapping.insert(last_idx, building_idx);
            agents.remap_building_indices(&mapping);
            households.remap_building_indices(&mapping);
            logistics.remap_building_indices(&mapping);
            zoning.remap_parcel_occupancy(last_idx, building_idx);
            Some((moved_key, building_idx))
        } else {
            None
        };

        self.buildings.swap_remove(building_idx);
        if self.building_sites.len() > building_idx {
            self.building_sites.swap_remove(building_idx);
            self.recompute_max_site_radius_m();
        }
        self.bump_building_ref_revision();
        self.dirty = true;
        self.dirty_index = true;
        self.entrances_dirty = true;
        moved_key
    }

    /// Removes one building through the normal lifecycle hooks used by demand redevelopment.
    pub(crate) fn remove_building_for_bulldoze(
        &mut self,
        building_idx: usize,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
    ) -> bool {
        if building_idx >= self.buildings.len() {
            return false;
        }
        let _ = self.remove_building_at_index(building_idx, zoning, agents, households, logistics);
        true
    }
}

fn accumulate_site_dirty_bounds(
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
