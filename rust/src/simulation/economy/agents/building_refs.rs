//! Agent references to building allocator indices.

use super::data::AgentSystem;
use super::determinism::stable_index;
use super::tick::{BuiltTripPlan, plan_building_origin_trip, plan_building_to_border_trip};
use super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACTIVITY_HOME, MODE_CAR, MODE_WALK, TRANSIT_ACCESS_INGRESS,
    TRANSIT_IN_BUILDING, TRANSIT_NETWORK, age_group_can_work,
};
use crate::config::AGENT_DRIVEWAY_SPEED_MS;
use crate::simulation::buildings::allocator::{BuildingAllocator, baseline_private_zone_slot};
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{NodeType, TransitFlags, TransitType};
use crate::simulation::zoning::ZoneType;
use godot::prelude::Vector2;
use std::collections::HashMap;

impl AgentSystem {
    /// Clears schedule-derived building cache fields for one agent.
    pub(crate) fn clear_schedule_building_cache(&mut self, agent_idx: usize) {
        self.agents.cached_commute_minutes[agent_idx] = 0;
        self.agents.next_commute_refresh_time[agent_idx] = 0.0;
        self.agents.next_departure_day[agent_idx] = u32::MAX;
        self.agents.next_departure_minute[agent_idx] = 0;
        self.agents.next_departure_origin_building[agent_idx] = usize::MAX;
        self.agents.next_departure_target_building[agent_idx] = usize::MAX;
        self.agents.next_departure_activity[agent_idx] = 0;
        self.agents.cached_schedule_work_building[agent_idx] = usize::MAX;
        self.agents.cached_work_profile_index[agent_idx] = u16::MAX;
    }

    /// Assigns an agent to a household without touching movement state.
    pub(crate) fn assign_household_id(&mut self, agent_idx: usize, household_id: usize) {
        if agent_idx < self.agents.len() {
            self.agents.household_id[agent_idx] = household_id;
        }
    }

    /// Converts a border household carrier into an ordinary resident inside its home building.
    pub(crate) fn materialize_household_carrier(
        &mut self,
        agent_idx: usize,
        household_id: usize,
        age_group: u8,
        door_pos: Option<Vector2>,
    ) {
        if agent_idx >= self.agents.len() {
            return;
        }
        self.agents.household_id[agent_idx] = household_id;
        self.agents.age_group[agent_idx] = age_group;
        self.agents.pending_household_size[agent_idx] = 0;
        self.agents.target_building[agent_idx] = usize::MAX;
        self.agents.planned_target_building[agent_idx] = usize::MAX;
        self.clear_route_and_lane_state(agent_idx);
        self.agents.next_replan_time[agent_idx] = 0.0;
        self.agents.transit_mode[agent_idx] = MODE_WALK;
        self.agents.activity[agent_idx] = 0;
        self.agents.planned_activity[agent_idx] = 0;
        self.agents.transit[agent_idx] = TRANSIT_IN_BUILDING;
        if let Some(door) = door_pos {
            self.agents.pos_x[agent_idx] = door.x;
            self.agents.pos_y[agent_idx] = door.y;
        }
        self.clear_schedule_building_cache(agent_idx);
        self.invalidate_lane_bucket_snapshot();
    }

    /// Updates a household member after their household relocates to a new home.
    pub(crate) fn relocate_household_member_home(
        &mut self,
        agent_idx: usize,
        old_home: usize,
        new_home: usize,
        old_home_live: bool,
    ) {
        if agent_idx >= self.agents.len() {
            return;
        }

        if self.agents.home_building[agent_idx] != new_home {
            self.agents.home_building[agent_idx] = new_home;
            self.clear_schedule_building_cache(agent_idx);
        }

        if old_home_live && self.agents.current_building[agent_idx] == old_home {
            self.agents.current_building[agent_idx] = new_home;
            self.agents.target_building[agent_idx] = usize::MAX;
            self.agents.planned_target_building[agent_idx] = usize::MAX;
            self.agents.transit[agent_idx] = TRANSIT_IN_BUILDING;
            self.agents.activity[agent_idx] = 0;
            self.clear_route_and_lane_state(agent_idx);
            self.invalidate_lane_bucket_snapshot();
        } else {
            let mut needs_replan = false;
            if old_home_live && self.agents.target_building[agent_idx] == old_home {
                self.agents.target_building[agent_idx] = new_home;
                needs_replan = true;
            }
            if old_home_live && self.agents.planned_target_building[agent_idx] == old_home {
                self.agents.planned_target_building[agent_idx] = new_home;
                needs_replan = true;
            }
            if !old_home_live
                && self.agents.current_building[agent_idx] == usize::MAX
                && self.agents.target_building[agent_idx] == usize::MAX
            {
                self.agents.target_building[agent_idx] = new_home;
                self.agents.planned_target_building[agent_idx] = new_home;
                self.agents.activity[agent_idx] = 0;
                needs_replan = true;
            }
            if needs_replan {
                self.clear_access_plan_and_path(agent_idx);
                self.agents.next_replan_time[agent_idx] = 0.0;
                self.clear_schedule_building_cache(agent_idx);
            }
        }
    }

    /// Updates a household member after their household loses its home.
    pub(crate) fn evict_household_member_home(&mut self, agent_idx: usize, old_home: usize) {
        if agent_idx >= self.agents.len() {
            return;
        }

        if self.agents.home_building[agent_idx] != usize::MAX {
            self.agents.home_building[agent_idx] = usize::MAX;
            self.clear_schedule_building_cache(agent_idx);
        }

        if self.agents.current_building[agent_idx] == old_home {
            self.agents.current_building[agent_idx] = usize::MAX;
            self.agents.target_building[agent_idx] = usize::MAX;
            self.agents.planned_target_building[agent_idx] = usize::MAX;
            self.agents.transit[agent_idx] = TRANSIT_ACCESS_INGRESS;
            self.clear_route_and_lane_state(agent_idx);
            self.invalidate_lane_bucket_snapshot();
        } else {
            let mut needs_replan = false;
            if self.agents.target_building[agent_idx] == old_home {
                self.agents.target_building[agent_idx] = usize::MAX;
                needs_replan = true;
            }
            if self.agents.planned_target_building[agent_idx] == old_home {
                self.agents.planned_target_building[agent_idx] = usize::MAX;
                needs_replan = true;
            }
            if needs_replan {
                self.clear_access_plan_and_path(agent_idx);
                self.agents.next_replan_time[agent_idx] = 0.0;
                self.clear_schedule_building_cache(agent_idx);
            }
        }
    }

    /// Assigns or clears an agent's workplace and invalidates derived work-trip caches.
    pub(crate) fn assign_work_building(
        &mut self,
        agent_idx: usize,
        mut work_building: usize,
        mut job_lock_days: u8,
    ) {
        if agent_idx >= self.agents.len() {
            return;
        }
        if work_building != usize::MAX && !age_group_can_work(self.agents.age_group[agent_idx]) {
            work_building = usize::MAX;
            job_lock_days = 0;
        }

        let old_work = self.agents.work_building[agent_idx];
        if old_work != work_building {
            self.agents.work_building[agent_idx] = work_building;
            self.clear_schedule_building_cache(agent_idx);
            if old_work != usize::MAX {
                let mut needs_replan = false;
                if self.agents.target_building[agent_idx] == old_work {
                    self.agents.target_building[agent_idx] = usize::MAX;
                    needs_replan = true;
                }
                if self.agents.planned_target_building[agent_idx] == old_work {
                    self.agents.planned_target_building[agent_idx] = usize::MAX;
                    needs_replan = true;
                }
                if needs_replan {
                    self.clear_access_plan_and_path(agent_idx);
                    self.agents.next_replan_time[agent_idx] = 0.0;
                }
            }
        }
        self.agents.job_lock_days[agent_idx] = job_lock_days;
        self.agents.consecutive_unpaid_days[agent_idx] = 0;
    }

    /// Forcefully removes all agents from a building that has been deleted.
    pub fn evict_building(&mut self, building_id: usize) {
        for i in 0..self.agents.len() {
            let cache_touches_building = self.agents.next_departure_origin_building[i]
                == building_id
                || self.agents.next_departure_target_building[i] == building_id
                || self.agents.cached_schedule_work_building[i] == building_id;
            let mut clear_schedule_cache = cache_touches_building;

            if self.agents.work_building[i] == building_id {
                self.agents.work_building[i] = usize::MAX;
                clear_schedule_cache = true;
            }
            if self.agents.home_building[i] == building_id {
                self.agents.home_building[i] = usize::MAX;
                self.agents.household_id[i] = usize::MAX;
                self.agents.pending_household_size[i] = 0;
                clear_schedule_cache = true;
            }
            if self.agents.planned_target_building[i] == building_id {
                self.agents.planned_target_building[i] = usize::MAX;
                clear_schedule_cache = true;
            }

            if self.agents.current_building[i] == building_id {
                self.agents.current_building[i] = usize::MAX;
                self.agents.target_building[i] = usize::MAX;
                self.agents.transit[i] = TRANSIT_ACCESS_INGRESS;
                clear_schedule_cache = true;
            } else if self.agents.target_building[i] == building_id {
                if self.agents.home_building[i] != usize::MAX {
                    self.agents.target_building[i] = self.agents.home_building[i];
                    self.agents.planned_target_building[i] = self.agents.home_building[i];
                    self.agents.activity[i] = 0;
                } else {
                    self.agents.target_building[i] = usize::MAX;
                    self.agents.pending_household_size[i] = 0;
                    self.agents.transit[i] = TRANSIT_ACCESS_INGRESS;
                }
                clear_schedule_cache = true;
            }

            if clear_schedule_cache {
                self.clear_schedule_building_cache(i);
            }
        }
        self.invalidate_lane_bucket_snapshot();
    }

    /// Prepares agents inside a building for visible displacement before that building is removed.
    pub(crate) fn evacuate_building_for_removal(
        &mut self,
        building_id: usize,
        households: &mut HouseholdSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        if building_id >= allocator.buildings.len() {
            return;
        }

        let mut household_targets: HashMap<usize, Option<usize>> = HashMap::new();
        for agent_idx in 0..self.agents.len() {
            if self.agents.home_building[agent_idx] != building_id {
                continue;
            }
            let household_id = self.agents.household_id[agent_idx];
            if household_id >= households.households.len()
                || household_targets.contains_key(&household_id)
            {
                continue;
            }
            let target_home = Self::claim_available_home_except(allocator, building_id);
            if let Some(home_idx) = target_home {
                let household = &mut households.households[household_id];
                household.home_building_id = home_idx;
                household.stay_failure_days = 0;
                household.unhoused_days_elapsed = 0;
            } else {
                households.households[household_id].home_building_id = usize::MAX;
            }
            household_targets.insert(household_id, target_home);
        }

        let border_plan = self.best_border_exit_plan_from_building(
            building_id,
            allocator,
            transit_network,
            graph,
        );
        for agent_idx in 0..self.agents.len() {
            let household_id = self.agents.household_id[agent_idx];
            let mut home_after_removal = self.agents.home_building[agent_idx];
            if home_after_removal == building_id {
                home_after_removal = household_targets
                    .get(&household_id)
                    .copied()
                    .flatten()
                    .unwrap_or(usize::MAX);
                self.agents.home_building[agent_idx] = home_after_removal;
                self.clear_schedule_building_cache(agent_idx);
            }

            if self.agents.work_building[agent_idx] == building_id {
                self.agents.work_building[agent_idx] = usize::MAX;
                self.agents.job_lock_days[agent_idx] = 0;
                self.clear_schedule_building_cache(agent_idx);
            }

            if self.agents.current_building[agent_idx] == building_id {
                if home_after_removal < allocator.buildings.len() {
                    if let Some(plan) = plan_building_origin_trip(
                        building_id,
                        home_after_removal,
                        ACTIVITY_HOME,
                        self.agents.has_car[agent_idx],
                        allocator,
                        transit_network,
                        graph,
                        &self.pathfind_count,
                    ) {
                        if self.start_building_origin_plan_on_network(
                            agent_idx,
                            plan,
                            transit_network,
                            graph,
                        ) {
                            continue;
                        }
                    }
                    self.place_agent_at_building_street_handoff(
                        agent_idx,
                        building_id,
                        home_after_removal,
                        ACTIVITY_HOME,
                        allocator,
                        transit_network,
                        graph,
                    );
                } else if let Some(plan) = border_plan.clone() {
                    if !self.start_building_origin_plan_on_network(
                        agent_idx,
                        plan,
                        transit_network,
                        graph,
                    ) {
                        self.place_agent_at_building_street_handoff(
                            agent_idx,
                            building_id,
                            usize::MAX,
                            ACTIVITY_HOME,
                            allocator,
                            transit_network,
                            graph,
                        );
                    }
                } else {
                    self.place_agent_at_building_street_handoff(
                        agent_idx,
                        building_id,
                        usize::MAX,
                        ACTIVITY_HOME,
                        allocator,
                        transit_network,
                        graph,
                    );
                }
            } else if self.agents.target_building[agent_idx] == building_id
                || self.agents.planned_target_building[agent_idx] == building_id
            {
                let target = if home_after_removal < allocator.buildings.len() {
                    home_after_removal
                } else {
                    usize::MAX
                };
                self.agents.target_building[agent_idx] = target;
                self.agents.planned_target_building[agent_idx] = target;
                self.agents.planned_activity[agent_idx] = if target == usize::MAX {
                    0
                } else {
                    ACTIVITY_HOME
                };
                self.clear_access_plan_and_path(agent_idx);
                self.agents.next_replan_time[agent_idx] = 0.0;
            }
        }
        self.invalidate_lane_bucket_snapshot();
    }

    fn best_border_exit_plan_from_building(
        &self,
        building_id: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<BuiltTripPlan> {
        let mut best: Option<(u32, BuiltTripPlan)> = None;
        for (node_idx, node) in graph.nodes().iter().enumerate() {
            if node.node_type != NodeType::Border {
                continue;
            }
            let border_node = node_idx as u32;
            let has_car_connection = graph.node_adjacency(border_node).iter().any(|&edge_idx| {
                let edge = graph.edge(edge_idx);
                !edge.deleted
                    && edge.primary_type == TransitType::Road
                    && (edge.allowed_types & TransitFlags::CAR) != 0
            });
            if !has_car_connection {
                continue;
            }
            let Some(plan) = plan_building_to_border_trip(
                building_id,
                border_node,
                allocator,
                transit_network,
                graph,
                &self.pathfind_count,
            ) else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|(best_node, _)| border_node < *best_node)
            {
                best = Some((border_node, plan));
            }
        }
        best.map(|(_, plan)| plan)
    }

    fn start_building_origin_plan_on_network(
        &mut self,
        agent_idx: usize,
        plan: BuiltTripPlan,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        if agent_idx >= self.agents.len()
            || plan.planned_attach_lane_id >= transit_network.lane_system.lanes.len()
        {
            return false;
        }
        let lane = &transit_network.lane_system.lanes[plan.planned_attach_lane_id];
        if lane.edge_id >= graph.edge_count() || graph.edge(lane.edge_id).deleted {
            return false;
        }
        let lane_pos = BuildingAllocator::sample_pos_on_lane(lane, plan.planned_attach_lane_d);
        let edge = graph.edge(lane.edge_id);
        let origin_node = if lane.is_fwd {
            edge.start_node
        } else {
            edge.end_node
        };

        self.agents.pos_x[agent_idx] = lane_pos.x;
        self.agents.pos_y[agent_idx] = lane_pos.y;
        self.agents.current_building[agent_idx] = usize::MAX;
        self.agents.target_building[agent_idx] = plan.target_building;
        self.agents.planned_target_building[agent_idx] = usize::MAX;
        self.agents.activity[agent_idx] = plan.activity;
        self.agents.planned_activity[agent_idx] = 0;
        self.agents.journey_start_time[agent_idx] = self.sim_time;
        self.agents.transit_mode[agent_idx] = plan.mode;
        self.agents.transit[agent_idx] = TRANSIT_NETWORK;
        self.agents.planned_attach_node[agent_idx] = plan.planned_attach_node;
        self.agents.planned_detach_node[agent_idx] = plan.planned_detach_node;
        self.agents.planned_attach_lane_id[agent_idx] = plan.planned_attach_lane_id as u32;
        self.agents.planned_detach_lane_id[agent_idx] = plan.planned_detach_lane_id as u32;
        self.agents.planned_attach_lane_d[agent_idx] = plan.planned_attach_lane_d;
        self.agents.planned_detach_lane_d[agent_idx] = plan.planned_detach_lane_d;
        self.agents.access_flags[agent_idx] = plan.access_flags;
        self.agents.next_replan_time[agent_idx] = 0.0;
        self.agents.current_node[agent_idx] = origin_node;
        self.agents.current_edge[agent_idx] = lane.edge_id;
        self.agents.current_lane_id[agent_idx] = plan.planned_attach_lane_id;
        self.agents.lane_distance[agent_idx] = plan.planned_attach_lane_d;
        self.agents.lane_change_from_lane_id[agent_idx] = u32::MAX;
        self.agents.lane_change_start_d[agent_idx] = 0.0;
        self.agents.lane_change_length_m[agent_idx] = 0.0;
        self.agents.overtake_blocked_time_s[agent_idx] = 0.0;
        self.agents.overtake_cooldown_s[agent_idx] = 0.0;
        self.agents.speed[agent_idx] = if plan.mode == MODE_CAR {
            edge.speed_limit.min(AGENT_DRIVEWAY_SPEED_MS)
        } else {
            0.0
        };
        self.agents.current_path[agent_idx] = plan.current_path;
        self.agents.current_path_index[agent_idx] =
            if self.agents.current_path[agent_idx].len() >= 2 {
                1
            } else {
                0
            };
        if (self.agents.access_flags[agent_idx] & ACCESS_FREIGHT_BORDER_DESTINATION) != 0 {
            self.agents.target_building[agent_idx] = usize::MAX;
            self.agents.planned_detach_lane_id[agent_idx] = u32::MAX;
        }
        true
    }

    fn place_agent_at_building_street_handoff(
        &mut self,
        agent_idx: usize,
        source_building: usize,
        target_building: usize,
        activity: u8,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        if agent_idx >= self.agents.len() {
            return;
        }
        let Some(entrance) = allocator.entrances.get(source_building) else {
            return;
        };
        let mut current_node = u32::MAX;
        let mut current_edge = usize::MAX;
        if entrance.edge_idx < graph.edge_count() && !graph.edge(entrance.edge_idx).deleted {
            let edge = graph.edge(entrance.edge_idx);
            current_edge = entrance.edge_idx;
            let start_pos = graph.node(edge.start_node).pos;
            let end_pos = graph.node(edge.end_node).pos;
            let start_d =
                (start_pos.x - entrance.curb_pos.x).hypot(start_pos.z - entrance.curb_pos.y);
            let end_d = (end_pos.x - entrance.curb_pos.x).hypot(end_pos.z - entrance.curb_pos.y);
            current_node = if start_d <= end_d {
                edge.start_node
            } else {
                edge.end_node
            };
        }
        self.agents.pos_x[agent_idx] = entrance.curb_pos.x;
        self.agents.pos_y[agent_idx] = entrance.curb_pos.y;
        self.agents.current_building[agent_idx] = usize::MAX;
        self.agents.target_building[agent_idx] = target_building;
        self.agents.planned_target_building[agent_idx] = target_building;
        self.agents.activity[agent_idx] = activity;
        self.agents.planned_activity[agent_idx] = activity;
        self.agents.transit_mode[agent_idx] = MODE_WALK;
        self.agents.transit[agent_idx] = TRANSIT_NETWORK;
        self.agents.current_node[agent_idx] = current_node;
        self.agents.current_edge[agent_idx] = current_edge;
        self.agents.current_lane_id[agent_idx] = usize::MAX;
        self.agents.lane_distance[agent_idx] = 0.0;
        self.agents.access_flags[agent_idx] = 0;
        self.agents.next_replan_time[agent_idx] = 0.0;
        self.clear_access_plan_and_path(agent_idx);
        self.agents.current_node[agent_idx] = current_node;
        self.agents.current_edge[agent_idx] = current_edge;
        if current_node == u32::MAX {
            let lane_id = [
                entrance.foot_lane_fwd,
                entrance.foot_lane_bkw,
                entrance.car_lane_fwd,
                entrance.car_lane_bkw,
            ]
            .into_iter()
            .find(|&lane_id| lane_id < transit_network.lane_system.lanes.len());
            if let Some(lane_id) = lane_id {
                let lane = &transit_network.lane_system.lanes[lane_id];
                if lane.edge_id < graph.edge_count() {
                    let edge = graph.edge(lane.edge_id);
                    self.agents.current_node[agent_idx] = if lane.is_fwd {
                        edge.start_node
                    } else {
                        edge.end_node
                    };
                    self.agents.current_edge[agent_idx] = lane.edge_id;
                }
            }
        }
    }

    /// Finds a residential building with available vacancy.
    /// Uses the allocator's `vacancy_index` for O(1) deterministic selection.
    pub fn find_available_home(&mut self, allocator: &mut BuildingAllocator) -> Option<usize> {
        let Some(residential_slot) = baseline_private_zone_slot(ZoneType::Residential) else {
            return None;
        };

        let total_vacant = allocator.vacancy_index[residential_slot].len();
        if total_vacant == 0 {
            return None;
        }

        let seed = (self.sim_time.to_bits() as u64)
            ^ ((self.agents.len() as u64) << 32)
            ^ (total_vacant as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let pick = stable_index(seed, total_vacant);
        let list = &allocator.vacancy_index[residential_slot];
        let building_idx = list.get(pick).copied().unwrap_or(usize::MAX);
        if building_idx != usize::MAX {
            allocator.claim_vacancy(building_idx);
            return Some(building_idx);
        }
        None
    }

    fn claim_available_home_except(
        allocator: &mut BuildingAllocator,
        excluded_building: usize,
    ) -> Option<usize> {
        let residential_slot = baseline_private_zone_slot(ZoneType::Residential)?;
        let building_idx = allocator.vacancy_index[residential_slot]
            .iter()
            .copied()
            .find(|&idx| idx != excluded_building && idx < allocator.buildings.len())?;
        allocator.claim_vacancy(building_idx);
        Some(building_idx)
    }

    /// Re-calculates building occupancy and vacancy index from scratch.
    pub fn recalculate_occupancy(&mut self, allocator: &mut BuildingAllocator) {
        for b in &mut allocator.buildings {
            b.occupancy = 0;
        }
        for i in 0..self.agents.len() {
            let h = self.agents.home_building[i];
            if h != usize::MAX && h < allocator.buildings.len() {
                allocator.buildings[h].occupancy += 1;
            }
        }
        allocator.rebuild_zone_index();
    }

    fn clear_route_and_lane_state(&mut self, agent_idx: usize) {
        self.clear_access_plan_and_path(agent_idx);
        self.agents.current_node[agent_idx] = u32::MAX;
        self.agents.current_edge[agent_idx] = usize::MAX;
        self.agents.current_lane_id[agent_idx] = usize::MAX;
        self.agents.lane_distance[agent_idx] = 0.0;
        self.agents.lane_change_from_lane_id[agent_idx] = u32::MAX;
        self.agents.lane_change_start_d[agent_idx] = 0.0;
        self.agents.lane_change_length_m[agent_idx] = 0.0;
        self.agents.overtake_blocked_time_s[agent_idx] = 0.0;
        self.agents.overtake_cooldown_s[agent_idx] = 0.0;
        self.agents.speed[agent_idx] = 0.0;
    }

    fn clear_access_plan_and_path(&mut self, agent_idx: usize) {
        self.agents.planned_attach_node[agent_idx] = u32::MAX;
        self.agents.planned_detach_node[agent_idx] = u32::MAX;
        self.agents.planned_attach_lane_id[agent_idx] = u32::MAX;
        self.agents.planned_detach_lane_id[agent_idx] = u32::MAX;
        self.agents.planned_attach_lane_d[agent_idx] = 0.0;
        self.agents.planned_detach_lane_d[agent_idx] = 0.0;
        self.agents.access_flags[agent_idx] = 0;
        self.agents.current_path[agent_idx].clear();
        self.agents.current_path_index[agent_idx] = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetManifest;
    use crate::assets::asset::{
        Anchor, AnchorType, BuildingData, MeshPart, PlacementMode, ZoneClass,
    };
    use crate::simulation::buildings::allocator::Building;
    use crate::simulation::economy::definitions::{
        load_runtime_economy_catalog, load_runtime_economy_tuning,
    };
    use crate::simulation::network::graph::Edge;
    use crate::simulation::network::types::EdgeClass;
    use crate::simulation::pathing::cch::CchGraph;
    use godot::prelude::Vector3;

    fn register_residential_test_asset(
        allocator: &mut BuildingAllocator,
        asset_id: &str,
    ) -> String {
        let manifest = AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test Home".to_owned(),
            asset_set: None,
            tags: vec![],
            thumbnail: None,
            lods: vec![],
            mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
            anchors: vec![Anchor {
                anchor_type: AnchorType::Entrance,
                name: "main".to_owned(),
                position: [0.0, 0.0, 0.5],
                forward: [0.0, 0.0, 1.0],
                width_m: None,
                length_m: None,
                vehicle_class: None,
            }],
            site_surfaces: vec![],
            building: Some(BuildingData {
                flat_size_m2: None,
                placement_mode: PlacementMode::ZonedPrivate,
                zone_type: Some(ZoneClass::Residential),
                density: Some("low".to_owned()),
                lot_width_cells: 1,
                lot_depth_cells: 1,
                frontage_forward: None,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity: Some(6),
                worker_capacity: None,
                service_class: None,
                economy_profile: None,
            }),
            prop: None,
            vehicle: None,
            character: None,
        };
        allocator.registry.register("test", manifest, String::new());
        format!("test:{asset_id}")
    }

    fn create_test_edge(n0: u32, n1: u32) -> Edge {
        Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        }
    }

    fn create_test_building(
        edge_idx: usize,
        asset_id: &str,
        center_x: f32,
        frontage_t: f32,
        occupancy: u32,
    ) -> Building {
        Building {
            center_x,
            center_y: 8.0,
            support_height_m: 0.0,
            width_cells: 1,
            depth_cells: 1,
            zone_profile_runtime_id: 0,
            parcel_id: 0,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(0.0, -1.0),
            frontage_t,
            side_offset: 8.0,
            is_deserted: false,
            budget_distress: false,
            edge_idx,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy,
            worker_count: 0,
            service_funding_override: -1.0,
            asset_id: asset_id.to_owned(),
            level: 1,
            construction_total_hours: 0,
            construction_remaining_hours: 0,
            broken: false,
            economy_profile_runtime_id: 0,
            economy_broken: false,
            resource_inventory: Vec::new(),
            revenue: 0.0,
            operating_budget: 500.0,
            profit_tax_budget_baseline: 500.0,
            last_day_profit: 0.0,
            shipment_cooldown_hours: 0,
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            daily_city_funded_input_cost: 0.0,
            daily_household_sales_value: 0.0,
            daily_power_service_units: 0.0,
            daily_power_served_units: 0.0,
            recent_power_service_units: 0.0,
            recent_power_served_units: 0.0,
            recent_household_sales_value: 0.0,
            commercial_activity_floor_scale: 0.0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        }
    }

    #[test]
    fn evacuating_deleted_home_rehomes_and_makes_inside_agent_visible() {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let edge_idx = graph.add_edge(create_test_edge(n0, n1));
        graph.rebuild_adjacency_list();

        let mut transit_network = TransitNetwork::new();
        transit_network.lane_system.rebuild(&mut graph);
        transit_network.cch_graph = CchGraph::build(&graph);

        let mut allocator = BuildingAllocator::new();
        let asset_id = register_residential_test_asset(&mut allocator, "eviction_home");
        allocator
            .buildings
            .push(create_test_building(edge_idx, &asset_id, 25.0, 0.25, 1));
        allocator
            .buildings
            .push(create_test_building(edge_idx, &asset_id, 75.0, 0.75, 0));
        allocator.rebuild_entrance_cache(&graph, &transit_network.lane_system);
        allocator.rebuild_zone_index();

        assert_eq!(
            AgentSystem::claim_available_home_except(&mut allocator, 0),
            Some(1),
            "forced relocation must not claim the building being removed"
        );
        allocator.buildings[1].occupancy = 0;
        allocator.rebuild_zone_index();

        let catalog = load_runtime_economy_catalog().expect("built-in economy catalog loads");
        let tuning = load_runtime_economy_tuning().expect("built-in economy tuning loads");
        let mut households = HouseholdSystem::new();
        let household_id = households.admit_immigrant_household(&catalog, &tuning, 0, 1);

        let mut agents = AgentSystem::new();
        let agent_idx = agents.spawn_housed_agent(0, 25.0, 8.0);
        agents.assign_household_id(agent_idx, household_id);
        agents.agents.work_building[agent_idx] = 0;

        agents.evacuate_building_for_removal(
            0,
            &mut households,
            &mut allocator,
            &transit_network,
            &graph,
        );

        assert_eq!(households.households[household_id].home_building_id, 1);
        assert_eq!(agents.agents.household_id[agent_idx], household_id);
        assert_eq!(agents.agents.home_building[agent_idx], 1);
        assert_eq!(agents.agents.work_building[agent_idx], usize::MAX);
        assert_eq!(agents.agents.current_building[agent_idx], usize::MAX);
        assert_eq!(agents.agents.target_building[agent_idx], 1);
        assert_eq!(agents.agents.transit[agent_idx], TRANSIT_NETWORK);
        assert_eq!(agents.agents.current_edge[agent_idx], edge_idx);
    }

    #[test]
    fn evict_building_clears_schedule_cache_references() {
        let mut sys = AgentSystem::new();
        let agent_idx = sys.spawn_housed_agent(1, 0.0, 0.0);
        sys.agents.work_building[agent_idx] = 2;
        sys.agents.target_building[agent_idx] = 2;
        sys.agents.planned_target_building[agent_idx] = 2;
        sys.agents.cached_commute_minutes[agent_idx] = 18;
        sys.agents.next_commute_refresh_time[agent_idx] = 12.0;
        sys.agents.next_departure_day[agent_idx] = 3;
        sys.agents.next_departure_minute[agent_idx] = 540;
        sys.agents.next_departure_origin_building[agent_idx] = 1;
        sys.agents.next_departure_target_building[agent_idx] = 2;
        sys.agents.next_departure_activity[agent_idx] = 1;
        sys.agents.cached_schedule_work_building[agent_idx] = 2;
        sys.agents.cached_work_profile_index[agent_idx] = 4;

        sys.evict_building(2);

        assert_eq!(sys.agents.work_building[agent_idx], usize::MAX);
        assert_eq!(sys.agents.target_building[agent_idx], 1);
        assert_eq!(sys.agents.planned_target_building[agent_idx], 1);
        assert_eq!(sys.agents.cached_commute_minutes[agent_idx], 0);
        assert_eq!(sys.agents.next_commute_refresh_time[agent_idx], 0.0);
        assert_eq!(sys.agents.next_departure_day[agent_idx], u32::MAX);
        assert_eq!(
            sys.agents.next_departure_origin_building[agent_idx],
            usize::MAX
        );
        assert_eq!(
            sys.agents.next_departure_target_building[agent_idx],
            usize::MAX
        );
        assert_eq!(
            sys.agents.cached_schedule_work_building[agent_idx],
            usize::MAX
        );
        assert_eq!(sys.agents.cached_work_profile_index[agent_idx], u16::MAX);
    }

    #[test]
    fn assigning_work_clears_schedule_cache_and_stale_work_trip() {
        let mut sys = AgentSystem::new();
        let agent_idx = sys.spawn_housed_agent(1, 0.0, 0.0);
        sys.agents.work_building[agent_idx] = 2;
        sys.agents.target_building[agent_idx] = 2;
        sys.agents.planned_target_building[agent_idx] = 2;
        sys.agents.access_flags[agent_idx] = crate::simulation::economy::agents::ACCESS_PLAN_VALID;
        sys.agents.current_path[agent_idx] = vec![10, 20];
        sys.agents.next_departure_day[agent_idx] = 1;
        sys.agents.next_departure_origin_building[agent_idx] = 1;
        sys.agents.next_departure_target_building[agent_idx] = 2;
        sys.agents.cached_schedule_work_building[agent_idx] = 2;

        sys.assign_work_building(agent_idx, 3, 7);

        assert_eq!(sys.agents.work_building[agent_idx], 3);
        assert_eq!(sys.agents.job_lock_days[agent_idx], 7);
        assert_eq!(sys.agents.target_building[agent_idx], usize::MAX);
        assert_eq!(sys.agents.planned_target_building[agent_idx], usize::MAX);
        assert_eq!(sys.agents.access_flags[agent_idx], 0);
        assert!(sys.agents.current_path[agent_idx].is_empty());
        assert_eq!(sys.agents.next_departure_day[agent_idx], u32::MAX);
        assert_eq!(
            sys.agents.cached_schedule_work_building[agent_idx],
            usize::MAX
        );
    }

    #[test]
    fn relocating_home_clears_schedule_cache_and_stale_home_trip() {
        let mut sys = AgentSystem::new();
        let agent_idx = sys.spawn_housed_agent(1, 0.0, 0.0);
        sys.agents.target_building[agent_idx] = 1;
        sys.agents.planned_target_building[agent_idx] = 1;
        sys.agents.access_flags[agent_idx] = crate::simulation::economy::agents::ACCESS_PLAN_VALID;
        sys.agents.current_path[agent_idx] = vec![10, 20];
        sys.agents.next_departure_day[agent_idx] = 1;
        sys.agents.next_departure_origin_building[agent_idx] = 1;
        sys.agents.next_departure_target_building[agent_idx] = 2;
        sys.agents.cached_schedule_work_building[agent_idx] = 2;

        sys.relocate_household_member_home(agent_idx, 1, 4, true);

        assert_eq!(sys.agents.home_building[agent_idx], 4);
        assert_eq!(sys.agents.current_building[agent_idx], 4);
        assert_eq!(sys.agents.target_building[agent_idx], usize::MAX);
        assert_eq!(sys.agents.planned_target_building[agent_idx], usize::MAX);
        assert_eq!(sys.agents.access_flags[agent_idx], 0);
        assert!(sys.agents.current_path[agent_idx].is_empty());
        assert_eq!(sys.agents.next_departure_day[agent_idx], u32::MAX);
        assert_eq!(
            sys.agents.cached_schedule_work_building[agent_idx],
            usize::MAX
        );
    }
}
