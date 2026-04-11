//! Building removal, immigration admission, and coordinate restoration.

use crate::debug_log;
use crate::simulation::buildings::allocator::{BuildingAllocator, zone_class_to_zone_type};
use crate::simulation::economy::agents::{AgentSystem, MODE_WALK, TRANSIT_IN_BUILDING};
use crate::simulation::economy::demand::{
    DemandBuildingActionKey, DemandBuildingActionPlan, DemandLevelChangeAction,
};
use crate::simulation::economy::households::{DEFAULT_IMMIGRANT_HOUSEHOLD_SIZE, HouseholdSystem};
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::grid::zoning::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneSystem;
use godot::prelude::Vector2;

const REZONE_GRACE_DAYS: u8 = 3;

impl BuildingAllocator {
    /// Removes buildings if their zone category has changed or their road edge no longer exists.
    pub(super) fn cleanup_stale_buildings(
        &mut self,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        logistics: &mut ShipmentSystem,
        graph: &RegionGraph,
        lanes: &LaneSystem,
    ) {
        let zone_cell_m = zoning.config.zone_cell_m;
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
                    let half_depth = b.depth_cells as f32 * zone_cell_m * 0.5;
                    let road_dist = zoning.distance_to_road_world(b.center_x, b.center_y) as f32;
                    if road_dist < half_depth {
                        None
                    } else {
                        match self.registry.get(&b.asset_id) {
                            Some(entry) => match entry.manifest.building.as_ref() {
                                Some(asset_building) if asset_building.is_zoned_private() => {
                                    match (asset_building.zone_type, asset_building.density_key()) {
                                        (Some(asset_zone_class), Some(asset_density)) => {
                                            let expected_zone_type =
                                                zone_class_to_zone_type(asset_zone_class);
                                            let edge = graph.edge(b.edge_idx);
                                            let edge_len = edge.physical_length;
                                            let cells_long =
                                                (edge_len / zone_cell_m).floor() as usize;
                                            if cells_long == 0
                                                || b.cell_x + b.width_cells as usize > cells_long
                                            {
                                                None
                                            } else {
                                                let curb_dist = edge.width * 0.5
                                                    + crate::config::SIDEWALK_WIDTH;
                                                let side = b.side as f32;
                                                let mut footprint_profile_runtime_id = 0_u16;
                                                let mut compatible = true;
                                                'profile_check: for dx in 0..b.width_cells as usize
                                                {
                                                    let t_dx = (b.cell_x as f32 + dx as f32 + 0.5)
                                                        * zone_cell_m
                                                        / edge_len;
                                                    let wp = Self::sample_pos_on_edge(
                                                        graph, b.edge_idx, t_dx,
                                                    );
                                                    let tangent = Self::sample_tangent_on_edge(
                                                        graph, b.edge_idx, t_dx,
                                                    );
                                                    let normal =
                                                        Vector2::new(tangent.y, -tangent.x) * side;
                                                    for dy in 0..b.depth_cells as usize {
                                                        let cell_center = wp
                                                            + normal
                                                                * (curb_dist
                                                                    + (dy as f32 + 0.5)
                                                                        * zone_cell_m);
                                                        let runtime_id = zoning
                                                            .get_zone_profile_runtime_id_world(
                                                                cell_center.x,
                                                                cell_center.y,
                                                            );
                                                        if footprint_profile_runtime_id == 0 {
                                                            if runtime_id == 0
                                                                || !zoning.profiles.asset_is_legal(
                                                                    runtime_id,
                                                                    expected_zone_type,
                                                                    asset_density,
                                                                    &entry.manifest.tags,
                                                                )
                                                            {
                                                                compatible = false;
                                                                break 'profile_check;
                                                            }
                                                            footprint_profile_runtime_id =
                                                                runtime_id;
                                                        } else if runtime_id
                                                            != footprint_profile_runtime_id
                                                        {
                                                            compatible = false;
                                                            break 'profile_check;
                                                        }
                                                    }
                                                }
                                                Some(compatible)
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
                let b_edge_idx = b.edge_idx;
                let b_side = b.side;
                let b_cell_x = b.cell_x;
                let b_center_x = b.center_x;
                let b_center_y = b.center_y;
                let b_facing = b.facing_dir;
                let b_width = b.width_cells;
                let b_depth = b.depth_cells;
                let b_zone = b.zone_type;
                self.dirty_zones[b_zone as usize] = true;

                let tangent = Vector2::new(-b_facing.y, b_facing.x);
                let width_m = b_width as f32 * zone_cell_m;
                let depth_m = b_depth as f32 * zone_cell_m;
                zoning.mark_occupied_rect(b_center_x, b_center_y, tangent, width_m, depth_m, false);

                if let Some(occ) = self.edge_occupancy.get_mut(&b_edge_idx) {
                    let slot = if b_side > 0 {
                        &mut occ.left
                    } else {
                        &mut occ.right
                    };
                    if b_cell_x < slot.len() {
                        slot[b_cell_x] = false;
                    }
                }

                logistics.invalidate_building(i, self);
                let last_idx = self.buildings.len() - 1;
                if i < last_idx {
                    self.dirty_zones[self.buildings[last_idx].zone_type as usize] = true;
                    let mut mapping = std::collections::HashMap::new();
                    mapping.insert(last_idx, i);
                    agents.remap_building_indices(&mapping);
                    logistics.remap_building_indices(&mapping);
                }

                self.buildings.swap_remove(i);
                self.dirty_index = true;
                self.entrances_dirty = true;
                removed_any = true;
            } else {
                i += 1;
            }
        }
        if removed_any {
            self.rebuild_entrance_cache(graph, lanes);
        }
    }

    /// Admits the already-decided demand-owned household count and assigns those households to
    /// claimed homes.
    pub(super) fn admit_households_from_demand(
        &mut self,
        households_to_spawn: usize,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
    ) {
        if households_to_spawn == 0 {
            return;
        }
        debug_log!(
            "economy",
            "demand-owned household admission planning: households_to_spawn={}",
            households_to_spawn,
        );
        for _ in 0..households_to_spawn {
            let Some((home_idx, household_size)) =
                self.claim_home_for_household(DEFAULT_IMMIGRANT_HOUSEHOLD_SIZE as u32)
            else {
                debug_log!(
                    "economy",
                    "demand-owned household admission stopped early: could not claim a home from vacancy index"
                );
                break;
            };
            let home_door = self.entrances[home_idx].door_pos;
            let household_id = households.admit_immigrant_household(home_idx, household_size);
            debug_log!(
                "economy",
                "demand-owned household admission created household_id={} size={} home_building={}",
                household_id,
                household_size,
                home_idx,
            );

            for _ in 0..household_size {
                let agent_idx = agents.spawn_agent(
                    home_idx,
                    u32::MAX,
                    0.0,
                    0.0,
                    u32::MAX,
                    home_door.x,
                    home_door.y,
                );
                agents.household_id[agent_idx] = household_id;
                agents.transit[agent_idx] = TRANSIT_IN_BUILDING;
                agents.transit_mode[agent_idx] = MODE_WALK;
                agents.current_building[agent_idx] = home_idx;
                agents.target_building[agent_idx] = usize::MAX;
                agents.planned_target_building[agent_idx] = usize::MAX;
                agents.current_node[agent_idx] = u32::MAX;
                agents.planned_attach_node[agent_idx] = u32::MAX;
                agents.planned_detach_node[agent_idx] = u32::MAX;
                agents.planned_attach_lane_id[agent_idx] = u32::MAX;
                agents.planned_detach_lane_id[agent_idx] = u32::MAX;
                agents.planned_attach_lane_d[agent_idx] = 0.0;
                agents.planned_detach_lane_d[agent_idx] = 0.0;
                agents.access_flags[agent_idx] = 0;
                agents.next_replan_time[agent_idx] = 0.0;
                agents.current_edge[agent_idx] = usize::MAX;
                agents.current_lane_id[agent_idx] = usize::MAX;
                agents.lane_distance[agent_idx] = 0.0;
                agents.speed[agent_idx] = 0.0;
                agents.pos_x[agent_idx] = home_door.x;
                agents.pos_y[agent_idx] = home_door.y;
                agents.activity[agent_idx] = 0;
                agents.planned_activity[agent_idx] = 0;
                agents.current_path[agent_idx].clear();
                agents.current_path_index[agent_idx] = 0;
                debug_log!(
                    "economy",
                    "demand-owned household admission housed agent_idx={} household_id={} current_building={} pos=({:.1}, {:.1})",
                    agent_idx,
                    household_id,
                    home_idx,
                    home_door.x,
                    home_door.y
                );
            }
        }
    }

    pub(crate) fn execute_demand_building_actions(
        &mut self,
        plan: &DemandBuildingActionPlan,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
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
                if let Some((moved_key, moved_idx)) =
                    self.remove_building_at_index(building_idx, zoning, agents, logistics)
                {
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
            let width_cells = building.width_cells as f32;
            let depth_cells = building.depth_cells as f32;
            let along_offset = width_cells * 0.5 * zone_cell_m;
            let depth_offset = crate::config::SIDEWALK_WIDTH
                + (building.cell_y as f32 + depth_cells * 0.5) * zone_cell_m;
            let edge_t =
                (building.cell_x as f32 * zone_cell_m / edge.physical_length).clamp(0.0, 1.0);

            let world_pos_on_edge = Self::sample_pos_on_edge(graph, building.edge_idx, edge_t);
            let tangent = Self::sample_tangent_on_edge(graph, building.edge_idx, edge_t);
            let normal = Vector2::new(tangent.y, -tangent.x) * building.side as f32;
            let center_2d = world_pos_on_edge
                + normal * (edge.width * 0.5 + depth_offset)
                + tangent * along_offset;

            building.center_x = center_2d.x;
            building.center_y = center_2d.y;
            building.facing_dir = normal;
            building.side_offset = building.side as f32;
        }

        self.dirty = true;
        Ok(())
    }
}

impl BuildingAllocator {
    fn claim_home_for_household(&mut self, desired_size: u32) -> Option<(usize, u16)> {
        let target_zones = [ZoneType::Residential, ZoneType::Mixed];

        let mut fallback_idx = usize::MAX;
        let mut fallback_size = 0_u16;
        'fallback: for &zone in &target_zones {
            for &building_idx in &self.vacancy_index[zone as usize] {
                let free_slots = self
                    .resident_capacity(building_idx)
                    .saturating_sub(self.buildings[building_idx].occupancy);
                if free_slots == 0 {
                    continue;
                }
                fallback_idx = building_idx;
                fallback_size = free_slots.min(desired_size).max(1) as u16;
                break 'fallback;
            }
        }
        if fallback_idx == usize::MAX {
            return None;
        }
        for _ in 0..fallback_size {
            self.claim_vacancy(fallback_idx);
        }
        Some((fallback_idx, fallback_size))
    }
}

fn demand_building_action_key(
    building: &crate::simulation::buildings::allocator::Building,
) -> DemandBuildingActionKey {
    DemandBuildingActionKey {
        edge_idx: building.edge_idx,
        side: building.side,
        cell_x: building.cell_x,
        width_cells: building.width_cells,
        depth_cells: building.depth_cells,
        level: building.level,
        asset_id: building.asset_id.clone(),
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
            || self.registry.resident_capacity(&action.target_asset_id) < building.occupancy
            || self.registry.worker_capacity(&action.target_asset_id) < building.worker_count
        {
            return None;
        }

        let target_zone_type = zone_class_to_zone_type(target_building.zone_type?);
        let building = &mut self.buildings[building_idx];
        building.asset_id = action.target_asset_id.clone();
        building.level = target_building.level;
        building.zone_type = target_zone_type;
        building.pending_redevelopment = false;
        building.rezone_grace_days_remaining = 0;
        self.dirty = true;
        self.dirty_index = true;
        self.entrances_dirty = true;
        self.dirty_zones[building.zone_type as usize] = true;
        Some(demand_building_action_key(building))
    }

    fn remove_building_at_index(
        &mut self,
        building_idx: usize,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        logistics: &mut ShipmentSystem,
    ) -> Option<(DemandBuildingActionKey, usize)> {
        let zone_cell_m = zoning.config.zone_cell_m;
        let building = self.buildings.get(building_idx)?.clone();
        let tangent = Vector2::new(-building.facing_dir.y, building.facing_dir.x);
        let width_m = building.width_cells as f32 * zone_cell_m;
        let depth_m = building.depth_cells as f32 * zone_cell_m;
        zoning.mark_occupied_rect(
            building.center_x,
            building.center_y,
            tangent,
            width_m,
            depth_m,
            false,
        );

        if let Some(occ) = self.edge_occupancy.get_mut(&building.edge_idx) {
            let slot = if building.side > 0 {
                &mut occ.left
            } else {
                &mut occ.right
            };
            if building.cell_x < slot.len() {
                slot[building.cell_x] = false;
            }
        }

        agents.evict_building(building_idx);
        logistics.invalidate_building(building_idx, self);
        self.dirty_zones[building.zone_type as usize] = true;

        let last_idx = self.buildings.len().saturating_sub(1);
        let moved_key = if building_idx < last_idx {
            let moved_building = self.buildings[last_idx].clone();
            let moved_key = demand_building_action_key(&moved_building);
            self.dirty_zones[moved_building.zone_type as usize] = true;
            let mut mapping = std::collections::HashMap::new();
            mapping.insert(last_idx, building_idx);
            agents.remap_building_indices(&mapping);
            logistics.remap_building_indices(&mapping);
            Some((moved_key, building_idx))
        } else {
            None
        };

        self.buildings.swap_remove(building_idx);
        self.dirty = true;
        self.dirty_index = true;
        self.entrances_dirty = true;
        moved_key
    }
}
