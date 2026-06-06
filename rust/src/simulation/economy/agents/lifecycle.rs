//! Agent spawning, removal, and whole-store lifecycle helpers.

use super::data::{Agent, AgentSystem};
use super::{MODE_CAR, MODE_WALK, TRANSIT_IMMIGRATING, TRANSIT_IN_BUILDING};
use crate::config::DEFAULT_URBAN_ROAD_SPEED_MS;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use rand::Rng;
use std::sync::atomic::Ordering;

impl AgentSystem {
    /// Spawns one agent already housed inside a building.
    pub fn spawn_housed_agent(&mut self, home: usize, init_x: f32, init_y: f32) -> usize {
        let mut rng = rand::thread_rng();
        let schedule_seed = stable_schedule_seed(home, self.agents.len() as u32);
        let render_id = self.allocate_render_id();
        let agent = Agent {
            home_building: home,
            household_id: usize::MAX,
            pending_household_size: 0,
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
            current_node: u32::MAX,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
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
            vehicle_type: rng.gen_range(0..4) as u8,
            pedestrian_type: rng.gen_range(0..4) as u8,
            walk_phase: rng.gen_range(0.0..1.0),
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        };

        self.agents.push(agent);
        self.invalidate_lane_bucket_snapshot();
        self.agents.len() - 1
    }

    /// Spawns a single border-origin arrival agent for explicit immigration-visualization paths.
    pub fn spawn_border_arrival_agent(
        &mut self,
        home: usize,
        _home_node: u32,
        _target_x: f32,
        _target_y: f32,
        highway_node: u32,
        init_x: f32,
        init_y: f32,
    ) -> usize {
        let mut rng = rand::thread_rng();
        let schedule_seed = stable_schedule_seed(home, self.agents.len() as u32);
        let render_id = self.allocate_render_id();
        let agent = Agent {
            home_building: home,
            household_id: usize::MAX,
            pending_household_size: 0,
            work_building: usize::MAX,
            pos_x: init_x,
            pos_y: init_y,
            render_id,
            activity: 0,
            transit: TRANSIT_IMMIGRATING,
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
            current_building: usize::MAX,
            target_building: home,
            planned_target_building: usize::MAX,
            current_node: highway_node,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            current_edge: usize::MAX,
            current_lane_id: usize::MAX,
            lane_distance: 0.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            speed: DEFAULT_URBAN_ROAD_SPEED_MS,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: Vec::new(),
            current_path_index: 0,
            has_car: true,
            vehicle_type: rng.gen_range(0..4) as u8,
            pedestrian_type: rng.gen_range(0..4) as u8,
            walk_phase: rng.gen_range(0.0..1.0),
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        };

        self.agents.push(agent);
        self.invalidate_lane_bucket_snapshot();
        self.agents.len() - 1
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
        let agent_idx = self.spawn_border_arrival_agent(
            home,
            u32::MAX,
            init_x,
            init_y,
            border_node,
            init_x,
            init_y,
        );
        self.agents.pending_household_size[agent_idx] = household_size.max(1);
        agent_idx
    }

    /// Efficiency helper to spawn a large number of agents at once for testing or benchmarks.
    pub fn spawn_random_agents(
        &mut self,
        count: usize,
        graph: &RegionGraph,
        allocator: &BuildingAllocator,
    ) {
        let mut rng = rand::thread_rng();
        let node_count = graph.node_count();
        let bldg_count = allocator.buildings.len();
        if node_count == 0 || bldg_count == 0 {
            return;
        }

        for _ in 0..count {
            let home_idx = rng.gen_range(0..bldg_count);
            let start_node = rng.gen_range(0..graph.node_count()) as u32;
            let start_pos = graph.node(start_node).pos;

            self.spawn_housed_agent(home_idx, start_pos.x, start_pos.z);
        }
    }

    /// Clears all agents from the system.
    pub fn clear(&mut self) {
        self.agents.clear();
        self.sim_time = 0.0;
        self.pathfind_count.store(0, Ordering::Relaxed);
        self.next_render_id = 0;
        self.invalidate_lane_bucket_snapshot();
    }

    /// Permanently removes an agent from the simulation using O(1) swap-remove.
    pub fn kill_agent(&mut self, index: usize, allocator: &mut BuildingAllocator) {
        if index >= self.agents.len() {
            return;
        }

        // Vacancy for residential is now household-based and managed by the HouseholdSystem.
        // Worker count for non-residential remains agent-based.
        let work = self.agents.work_building[index];
        if work != usize::MAX && work < allocator.buildings.len() {
            allocator.buildings[work].worker_count =
                allocator.buildings[work].worker_count.saturating_sub(1);
        }

        self.agents.swap_remove(index);
        self.invalidate_lane_bucket_snapshot();
    }
}

fn stable_schedule_seed(home_building: usize, spawn_index: u32) -> u32 {
    let mixed = (home_building as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(u64::from(spawn_index).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    ((mixed >> 32) as u32) ^ (mixed as u32)
}
