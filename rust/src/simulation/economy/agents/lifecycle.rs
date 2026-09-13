// SPDX-License-Identifier: GPL-2.0-only

//! Agent spawning, removal, and whole-store lifecycle helpers.

use super::data::{Agent, AgentSystem};
use super::determinism::{stable_index, stable_unit_f32};
use super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, AGE_ADULT, AGE_CHILD, MODE_CAR, MODE_WALK,
    TRANSIT_ACCESS_EGRESS, TRANSIT_IMMIGRATING, TRANSIT_IN_BUILDING, TRANSIT_NETWORK,
    VEHICLE_FREIGHT_DELIVERY, age_group_can_work,
};
use crate::config::DEFAULT_URBAN_ROAD_SPEED_MS;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::tick::{
    plan_building_origin_trip, plan_building_to_border_trip, plan_immigration_trip,
};
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

impl AgentSystem {
    /// Spawns one agent already housed inside a building.
    pub fn spawn_housed_agent(&mut self, home: usize, init_x: f32, init_y: f32) -> usize {
        self.spawn_housed_agent_with_age_group(home, init_x, init_y, AGE_ADULT)
    }

    /// Spawns one housed resident with an explicit lifecycle class.
    pub(crate) fn spawn_housed_agent_with_age_group(
        &mut self,
        home: usize,
        init_x: f32,
        init_y: f32,
        age_group: u8,
    ) -> usize {
        let mut agent = self.new_agent(home, init_x, init_y);
        agent.age_group = age_group;
        self.insert_agent(agent)
    }

    /// Spawns a single border-origin arrival agent for explicit immigration-visualization paths.
    pub fn spawn_border_arrival_agent(
        &mut self,
        home: usize,
        border_node: u32,
        init_x: f32,
        init_y: f32,
    ) -> usize {
        let mut agent = self.new_agent(home, init_x, init_y);
        agent.transit = TRANSIT_IMMIGRATING;
        agent.current_building = usize::MAX;
        agent.target_building = home;
        agent.current_node = border_node;
        agent.speed = DEFAULT_URBAN_ROAD_SPEED_MS;
        agent.transit_mode = MODE_CAR;
        self.insert_agent(agent)
    }

    /// Spawns one visible border-origin car that represents a whole pending household.
    pub fn spawn_household_arrival_carrier(
        &mut self,
        home: usize,
        household_size: u16,
        border_node: u32,
        init_x: f32,
        init_y: f32,
    ) -> usize {
        let agent_idx = self.spawn_border_arrival_agent(home, border_node, init_x, init_y);
        self.agents.pending_household_size[agent_idx] = household_size.max(1);
        agent_idx
    }

    /// Spawns a physical freight carrier for one building-to-building shipment.
    pub(crate) fn spawn_city_freight_carrier(
        &mut self,
        shipment_id: u64,
        source_building: usize,
        destination_building: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<usize> {
        let plan = plan_building_origin_trip(
            source_building,
            destination_building,
            2,
            true,
            allocator,
            transit_network,
            graph,
            &self.pathfind_count,
        )?;
        let agent_idx = self.spawn_freight_carrier_base(shipment_id, source_building);
        self.apply_building_origin_freight_plan(agent_idx, source_building, plan, allocator);
        Some(agent_idx)
    }

    /// Reroutes an existing freight carrier from one building to another.
    pub(crate) fn route_freight_carrier_building_to_building(
        &mut self,
        agent_idx: usize,
        source_building: usize,
        destination_building: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        if agent_idx >= self.agents.len() {
            return false;
        }
        let Some(plan) = plan_building_origin_trip(
            source_building,
            destination_building,
            2,
            true,
            allocator,
            transit_network,
            graph,
            &self.pathfind_count,
        ) else {
            return false;
        };
        self.apply_building_origin_freight_plan(agent_idx, source_building, plan, allocator);
        self.agents.freight_target_border_node[agent_idx] = u32::MAX;
        true
    }

    /// Spawns a physical freight carrier entering from an outside-world border node.
    pub(crate) fn spawn_import_freight_carrier(
        &mut self,
        shipment_id: u64,
        border_node: u32,
        destination_building: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<usize> {
        let plan = plan_immigration_trip(
            border_node,
            destination_building,
            allocator,
            transit_network,
            graph,
            &self.pathfind_count,
        )?;
        if border_node as usize >= graph.node_count() {
            return None;
        }
        let agent_idx = self.spawn_freight_carrier_base(shipment_id, usize::MAX);
        self.apply_border_origin_freight_plan(
            agent_idx,
            border_node,
            destination_building,
            plan,
            graph,
        );
        Some(agent_idx)
    }

    /// Reroutes an existing freight carrier from an outside-world border node to a building.
    pub(crate) fn route_freight_carrier_border_to_building(
        &mut self,
        agent_idx: usize,
        border_node: u32,
        destination_building: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        if agent_idx >= self.agents.len() {
            return false;
        }
        let Some(plan) = plan_immigration_trip(
            border_node,
            destination_building,
            allocator,
            transit_network,
            graph,
            &self.pathfind_count,
        ) else {
            return false;
        };
        if border_node as usize >= graph.node_count() {
            return false;
        }
        self.apply_border_origin_freight_plan(
            agent_idx,
            border_node,
            destination_building,
            plan,
            graph,
        );
        true
    }

    /// Spawns a physical freight carrier for one building-to-border export shipment.
    pub(crate) fn spawn_export_freight_carrier(
        &mut self,
        shipment_id: u64,
        source_building: usize,
        border_node: u32,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<usize> {
        let plan = plan_building_to_border_trip(
            source_building,
            border_node,
            allocator,
            transit_network,
            graph,
            &self.pathfind_count,
        )?;
        let agent_idx = self.spawn_freight_carrier_base(shipment_id, source_building);
        self.apply_building_origin_freight_plan(agent_idx, source_building, plan, allocator);
        self.agents.freight_target_border_node[agent_idx] = border_node;
        Some(agent_idx)
    }

    /// Reroutes an existing freight carrier from a building to an outside-world border node.
    pub(crate) fn route_freight_carrier_building_to_border(
        &mut self,
        agent_idx: usize,
        source_building: usize,
        border_node: u32,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        if agent_idx >= self.agents.len() {
            return false;
        }
        let Some(plan) = plan_building_to_border_trip(
            source_building,
            border_node,
            allocator,
            transit_network,
            graph,
            &self.pathfind_count,
        ) else {
            return false;
        };
        self.apply_building_origin_freight_plan(agent_idx, source_building, plan, allocator);
        self.agents.freight_target_border_node[agent_idx] = border_node;
        true
    }

    /// Checks that an agent slot still belongs to the expected physical shipment.
    pub(crate) fn freight_carrier_matches(&self, index: usize, shipment_id: u64) -> bool {
        index < self.agents.len()
            && self.agents.vehicle_type[index] == VEHICLE_FREIGHT_DELIVERY
            && self.agents.freight_shipment_id[index] == shipment_id
    }

    /// Removes only the expected freight carrier and returns any swap-remap pair.
    pub(crate) fn remove_freight_carrier(
        &mut self,
        index: usize,
        shipment_id: u64,
        households: &mut HouseholdSystem,
    ) -> Option<(usize, usize)> {
        if !self.freight_carrier_matches(index, shipment_id) {
            return None;
        }
        self.remove_agent_record(index, households)
    }

    /// Permanently removes an agent from the simulation using O(1) swap-remove.
    ///
    /// Repairs an active shopper reference immediately. Returns `(old_idx, new_idx)` when another
    /// agent moved and freight index owners must remap.
    pub fn kill_agent(
        &mut self,
        index: usize,
        allocator: &mut BuildingAllocator,
        households: &mut HouseholdSystem,
    ) -> Option<(usize, usize)> {
        if index >= self.agents.len() {
            return None;
        }

        // Vacancy for residential is now household-based and managed by the HouseholdSystem.
        // Worker count for non-residential remains agent-based.
        let work = self.agents.work_building[index];
        if age_group_can_work(self.agents.age_group[index])
            && work != usize::MAX
            && work < allocator.buildings.len()
        {
            allocator.buildings[work].worker_count =
                allocator.buildings[work].worker_count.saturating_sub(1);
        }

        self.remove_agent_record(index, households)
    }

    fn remove_agent_record(
        &mut self,
        index: usize,
        households: &mut HouseholdSystem,
    ) -> Option<(usize, usize)> {
        let last = self.agents.len() - 1;
        self.agents.swap_remove(index);
        self.invalidate_lane_bucket_snapshot();
        if index != last {
            households.remap_shopper_agent_index(last, index, self);
            Some((last, index))
        } else {
            None
        }
    }
}

impl AgentSystem {
    fn new_agent(&mut self, home: usize, init_x: f32, init_y: f32) -> Agent {
        let spawn_index = self.agents.len() as u32;
        let schedule_seed = stable_schedule_seed(home, spawn_index);
        let render_id = self.allocate_render_id();
        let visual_seed = agent_visual_seed(home, spawn_index);
        Agent {
            home_building: home,
            household_id: usize::MAX,
            age_group: AGE_ADULT,
            pending_household_size: 0,
            freight_shipment_id: u64::MAX,
            work_building: usize::MAX,
            pos_x: init_x,
            pos_y: init_y,
            render_id,
            activity: 0,
            transit: TRANSIT_IN_BUILDING,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: self.sim_time,
            schedule_seed,
            cached_commute_minutes: 0,
            next_commute_refresh_time: 0.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            current_building: home,
            target_building: usize::MAX,
            planned_target_building: usize::MAX,
            freight_target_border_node: u32::MAX,
            current_node: u32::MAX,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            network_replan_failures: 0,
            current_edge: usize::MAX,
            current_lane_id: usize::MAX,
            lane_distance: 0.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            speed: 0.0,
            transit_mode: MODE_WALK,
            planned_activity: 0,
            current_path: Vec::new(),
            current_path_index: 0,
            has_car: true,
            vehicle_type: stable_index(visual_seed ^ 0xA11C_EC7A, 4) as u8,
            pedestrian_type: stable_index(visual_seed ^ 0x51DE_CAFE, 4) as u8,
            walk_phase: stable_unit_f32(visual_seed ^ 0xBEE5_100D),
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        }
    }

    fn insert_agent(&mut self, agent: Agent) -> usize {
        let index = self.agents.len();
        self.agents.push(agent);
        self.invalidate_lane_bucket_snapshot();
        index
    }

    fn spawn_freight_carrier_base(&mut self, shipment_id: u64, origin_building: usize) -> usize {
        let mut agent = self.new_agent(origin_building, 0.0, 0.0);
        agent.home_building = usize::MAX;
        // Freight is not a resident and must remain outside adult employment accounting.
        agent.age_group = AGE_CHILD;
        agent.freight_shipment_id = shipment_id;
        agent.activity = 2;
        agent.money = 0.0;
        agent.transit_mode = MODE_CAR;
        agent.vehicle_type = VEHICLE_FREIGHT_DELIVERY;
        self.insert_agent(agent)
    }

    fn apply_building_origin_freight_plan(
        &mut self,
        agent_idx: usize,
        source_building: usize,
        plan: crate::simulation::economy::agents::tick::BuiltTripPlan,
        allocator: &BuildingAllocator,
    ) {
        if let Some(origin_entrance) = allocator.entrances.get(source_building) {
            self.agents.pos_x[agent_idx] = origin_entrance.door_pos.x;
            self.agents.pos_y[agent_idx] = origin_entrance.door_pos.y;
        }
        self.agents.current_building[agent_idx] = source_building;
        self.agents.target_building[agent_idx] = plan.target_building;
        self.agents.activity[agent_idx] = plan.activity;
        self.agents.journey_start_time[agent_idx] = self.sim_time;
        self.agents.transit_mode[agent_idx] = plan.mode;
        self.agents.planned_attach_node[agent_idx] = plan.planned_attach_node;
        self.agents.planned_detach_node[agent_idx] = plan.planned_detach_node;
        self.agents.planned_attach_lane_id[agent_idx] = plan.planned_attach_lane_id as u32;
        self.agents.planned_detach_lane_id[agent_idx] = plan.planned_detach_lane_id as u32;
        self.agents.planned_attach_lane_d[agent_idx] = plan.planned_attach_lane_d;
        self.agents.planned_detach_lane_d[agent_idx] = plan.planned_detach_lane_d;
        self.agents.access_flags[agent_idx] = plan.access_flags;
        self.agents.next_replan_time[agent_idx] = 0.0;
        self.agents.network_replan_failures[agent_idx] = 0;
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
        if (plan.access_flags & ACCESS_FREIGHT_BORDER_DESTINATION) != 0 {
            self.agents.target_building[agent_idx] = usize::MAX;
            self.agents.planned_detach_lane_id[agent_idx] = u32::MAX;
        }
        self.agents.current_path[agent_idx] = plan.current_path;
        self.agents.current_path_index[agent_idx] =
            if self.agents.current_path[agent_idx].len() >= 2 {
                1
            } else {
                0
            };
        self.agents.transit[agent_idx] = TRANSIT_ACCESS_EGRESS;
    }

    fn apply_border_origin_freight_plan(
        &mut self,
        agent_idx: usize,
        border_node: u32,
        destination_building: usize,
        plan: crate::simulation::economy::agents::tick::BuiltTripPlan,
        graph: &RegionGraph,
    ) {
        let pos = graph.node(border_node).pos;
        self.agents.pos_x[agent_idx] = pos.x;
        self.agents.pos_y[agent_idx] = pos.z;
        self.agents.current_building[agent_idx] = usize::MAX;
        self.agents.target_building[agent_idx] = destination_building;
        self.agents.planned_target_building[agent_idx] = usize::MAX;
        self.agents.freight_target_border_node[agent_idx] = u32::MAX;
        self.agents.current_node[agent_idx] = border_node;
        self.agents.current_edge[agent_idx] = usize::MAX;
        self.agents.current_lane_id[agent_idx] = usize::MAX;
        self.agents.lane_distance[agent_idx] = 0.0;
        self.agents.lane_change_from_lane_id[agent_idx] = u32::MAX;
        self.agents.lane_change_start_d[agent_idx] = 0.0;
        self.agents.lane_change_length_m[agent_idx] = 0.0;
        self.agents.overtake_blocked_time_s[agent_idx] = 0.0;
        self.agents.overtake_cooldown_s[agent_idx] = 0.0;
        self.agents.transit[agent_idx] = TRANSIT_NETWORK;
        self.agents.transit_mode[agent_idx] = MODE_CAR;
        self.agents.speed[agent_idx] = 0.0;
        self.agents.planned_attach_node[agent_idx] = plan.planned_attach_node;
        self.agents.planned_detach_node[agent_idx] = plan.planned_detach_node;
        self.agents.planned_attach_lane_id[agent_idx] = u32::MAX;
        self.agents.planned_detach_lane_id[agent_idx] = plan.planned_detach_lane_id as u32;
        self.agents.planned_attach_lane_d[agent_idx] = 0.0;
        self.agents.planned_detach_lane_d[agent_idx] = plan.planned_detach_lane_d;
        self.agents.access_flags[agent_idx] = plan.access_flags;
        self.agents.next_replan_time[agent_idx] = 0.0;
        self.agents.network_replan_failures[agent_idx] = 0;
        self.agents.current_path[agent_idx] = plan.current_path;
        self.agents.current_path_index[agent_idx] =
            if self.agents.current_path[agent_idx].len() >= 2 {
                1
            } else {
                0
            };
    }
}

fn stable_schedule_seed(home_building: usize, spawn_index: u32) -> u32 {
    let mixed = (home_building as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(u64::from(spawn_index).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    ((mixed >> 32) as u32) ^ (mixed as u32)
}

fn agent_visual_seed(home_building: usize, spawn_index: u32) -> u64 {
    (home_building as u64)
        .wrapping_mul(0x94D0_49BB_1331_11EB)
        .wrapping_add(u64::from(spawn_index).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fmt::Write, hash::Hasher, hint::black_box, time::Instant};

    #[test]
    #[ignore = "manual matched release timing of agent initialization and SoA insertion"]
    fn benchmark_agent_spawning() {
        struct StateHash(std::collections::hash_map::DefaultHasher);
        impl std::fmt::Write for StateHash {
            fn write_str(&mut self, value: &str) -> std::fmt::Result {
                self.0.write(value.as_bytes());
                Ok(())
            }
        }

        for count in [1_024, 16_384, 131_072] {
            for mode in ["housed", "border", "freight", "mixed"] {
                let mut system = AgentSystem::new();
                system.agents.reserve(count);
                system.sim_time = 123.5;
                let mut samples = [0.0; 21];
                for sample in 0..24 {
                    // Retain column capacity; reset fixture identity outside measured spawning.
                    system.agents.clear();
                    system.next_render_id = 17;
                    let start = Instant::now();
                    for index in 0..black_box(count) {
                        let origin = if index % 7 == 0 {
                            usize::MAX
                        } else {
                            index % 97
                        };
                        let x = index as f32 * 0.25;
                        let kind = match mode {
                            "housed" => 0,
                            "border" => 1,
                            "freight" => 2,
                            _ => index % 3,
                        };
                        match kind {
                            0 => {
                                system.spawn_housed_agent_with_age_group(
                                    origin,
                                    x,
                                    -x,
                                    (index % 3) as u8,
                                );
                            }
                            1 => {
                                system.spawn_household_arrival_carrier(
                                    origin,
                                    (index % 6) as u16,
                                    (index % 31) as u32,
                                    x,
                                    -x,
                                );
                            }
                            _ => {
                                system.spawn_freight_carrier_base(index as u64 + 51, origin);
                            }
                        }
                    }
                    let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
                    if sample >= 3 {
                        samples[sample - 3] = elapsed;
                    }
                    black_box(&system);
                }
                // Hash every generated column, including schedule/visual seeds, outside timing.
                let mut hash = StateHash(Default::default());
                write!(&mut hash, "{:?}", system.agents).unwrap();
                samples.sort_by(f64::total_cmp);
                eprintln!(
                    "agent_spawning agents={count} mode={mode} median_ms={:.6} checksum={}",
                    samples[10],
                    hash.0.finish()
                );
            }
        }
    }
}
