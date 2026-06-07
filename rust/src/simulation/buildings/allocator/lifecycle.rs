//! Building removal, demand-owned household admission, and coordinate restoration.

use crate::debug_log;
use crate::simulation::buildings::allocator::{
    BuildingAllocator, baseline_private_zone_slot, resolve_building_economy_profile_binding,
    zone_class_to_zone_type,
};
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::load_runtime_economy_catalog;
use crate::simulation::economy::demand::{
    DemandBuildingActionKey, DemandBuildingActionPlan, DemandLevelChangeAction,
    demand_building_action_key,
};
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneSystem;
use crate::simulation::network::types::{NodeType, TransitFlags, TransitType};
use crate::simulation::zoning::ZoneType;
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::Vector2;

const REZONE_GRACE_DAYS: u8 = 3;

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
                if let Some(zone_idx) = baseline_private_zone_slot(b_zone) {
                    self.dirty_zones[zone_idx] = true;
                }
                zoning.clear_parcel_occupancy(b_parcel_id);

                logistics.invalidate_building(i, self);
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

                households.invalidate_building(i);

                self.buildings.swap_remove(i);
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
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> usize {
        if households_to_spawn == 0 {
            return 0;
        }
        debug_log!(
            "economy",
            "demand-owned household admission planning: households_to_spawn={}",
            households_to_spawn,
        );
        let mut launched = 0;
        for _ in 0..households_to_spawn {
            let Some((home_idx, household_size)) = self.claim_home_for_household() else {
                debug_log!(
                    "economy",
                    "demand-owned household admission stopped early: could not claim a home from vacancy index"
                );
                break;
            };
            let Some(border_node) =
                self.household_arrival_border_node(home_idx, transit_network, graph)
            else {
                debug_log!(
                    "economy",
                    "demand-owned household admission waiting: no legal border-to-home car route for home_building={}",
                    home_idx
                );
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
            debug_log!(
                "economy",
                "demand-owned household admission launched carrier_agent={} size={} home_building={} border_node={}",
                carrier_idx,
                household_size,
                home_idx,
                border_node,
            );
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
    ) {
        let mut action_lookup: std::collections::HashMap<DemandBuildingActionKey, usize> = self
            .buildings
            .iter()
            .enumerate()
            .map(|(idx, building)| (demand_building_action_key(building), idx))
            .collect();
        let mut mutated_any = false;

        for use_plan in [&plan.residential, &plan.commercial, &plan.industrial] {
            for action in &use_plan.despawns {
                let Some(building_idx) = action_lookup.remove(action) else {
                    continue;
                };
                if !self.can_demand_despawn(building_idx) {
                    continue;
                }
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
                if let Some(updated_key) = self.apply_level_change_action(building_idx, action) {
                    action_lookup.remove(&action.building);
                    action_lookup.insert(updated_key, building_idx);
                    mutated_any = true;
                }
            }

            for action in &use_plan.upgrades {
                let Some(&building_idx) = action_lookup.get(&action.building) else {
                    continue;
                };
                if let Some(updated_key) = self.apply_level_change_action(building_idx, action) {
                    action_lookup.remove(&action.building);
                    action_lookup.insert(updated_key, building_idx);
                    mutated_any = true;
                }
            }

            for action in &use_plan.spawns {
                if self.execute_demand_spawn_action(action, zoning, graph) {
                    mutated_any = true;
                }
            }
        }

        if mutated_any {
            if self.dirty_index {
                self.rebuild_zone_index();
            }
            self.rebuild_entrance_cache(graph, lanes);
        }
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
                let width_cells = building.width_cells as f32;
                let along_offset = width_cells * 0.5 * zone_cell_m;
                let depth_offset = crate::config::SIDEWALK_WIDTH
                    + (building.cell_y as f32 + depth_cells * 0.5) * zone_cell_m;
                let edge_t =
                    (building.cell_x as f32 * zone_cell_m / edge.physical_length).clamp(0.0, 1.0);

                let world_pos_on_edge = Self::sample_pos_on_edge(graph, building.edge_idx, edge_t);
                let tangent = Self::sample_tangent_on_edge(graph, building.edge_idx, edge_t);
                let normal = Vector2::new(tangent.y, -tangent.x) * building.side as f32;
                (
                    world_pos_on_edge
                        + normal * (edge.width * 0.5 + depth_offset)
                        + tangent * along_offset,
                    normal,
                    building.side as f32,
                )
            };

            building.center_x = center_2d.x;
            building.center_y = center_2d.y;
            building.facing_dir = normal * -1.0;
            building.side_offset = side_offset;
        }

        self.dirty = true;
        Ok(())
    }
}

impl BuildingAllocator {
    fn claim_home_for_household(&mut self) -> Option<(usize, u16)> {
        let target_zones = [ZoneType::Residential];

        let mut fallback_idx = usize::MAX;
        let mut fallback_size = 0_u16;
        'fallback: for &zone in &target_zones {
            let Some(zone_idx) = baseline_private_zone_slot(zone) else {
                continue;
            };
            for &building_idx in &self.vacancy_index[zone_idx] {
                let free_slots = self
                    .household_capacity(building_idx)
                    .saturating_sub(self.buildings[building_idx].occupancy);
                if free_slots == 0 {
                    continue;
                }
                fallback_idx = building_idx;

                // Derive household size from the building's authored flat size.
                // Baseline: 40m2 per person, minimum 1, maximum 5.
                let flat_size = self.flat_size_m2(building_idx);
                let derived_size = if flat_size > 1.0 {
                    ((flat_size / 40.0).ceil() as u16).clamp(1, 5)
                } else {
                    2 // Legacy absolute fallback
                };

                fallback_size = derived_size;
                break 'fallback;
            }
        }
        if fallback_idx == usize::MAX {
            return None;
        }
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
            || self.worker_capacity_for_asset(&action.target_asset_id) < building.worker_count
        {
            return None;
        }

        let target_zone_type = zone_class_to_zone_type(target_building.zone_type?);
        if target_zone_type != building.zone_type {
            return None;
        }
        let economy_binding =
            resolve_building_economy_profile_binding(&self.registry, &action.target_asset_id);
        if matches!(
            target_zone_type,
            ZoneType::Commercial | ZoneType::Industrial
        ) && (economy_binding.economy_broken || economy_binding.runtime_id == 0)
        {
            return None;
        }
        let building = &mut self.buildings[building_idx];
        building.asset_id = action.target_asset_id.clone();
        building.level = target_building.level;
        building.economy_profile_runtime_id = economy_binding.runtime_id;
        building.economy_broken = economy_binding.economy_broken;
        let profile = load_runtime_economy_catalog()
            .ok()
            .and_then(|catalog| {
                catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .cloned()
            })
            .map(Box::new);
        let resource_count = load_runtime_economy_catalog()
            .map(|catalog| catalog.resource_count())
            .unwrap_or(0);
        building.retain_inventory_for_profile(profile.as_deref(), resource_count);
        building.pending_redevelopment = false;
        building.rezone_grace_days_remaining = 0;
        self.dirty = true;
        self.dirty_index = true;
        self.entrances_dirty = true;
        if let Some(zone_idx) = baseline_private_zone_slot(building.zone_type) {
            self.dirty_zones[zone_idx] = true;
        }
        Some(demand_building_action_key(building))
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
        households.invalidate_building(building_idx);
        logistics.invalidate_building(building_idx, self);
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
        self.bump_building_ref_revision();
        self.dirty = true;
        self.dirty_index = true;
        self.entrances_dirty = true;
        moved_key
    }
}
