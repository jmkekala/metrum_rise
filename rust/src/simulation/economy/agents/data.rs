//! Core data layout and lifecycle management for the agent simulation.
//!
//! # Memory layout
//!
//! [`AgentSystem`] uses a Structure-of-Arrays (SoA) layout provided by the `soa_derive` crate.
//! This enables cache-friendly bulk iteration and ensures all fields are kept in sync.

use super::{MODE_CAR, TRANSIT_ARRIVING, TRANSIT_IDLE, TRANSIT_IMMIGRATING, TRANSIT_ON_ROAD};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::graph::RegionGraph;

use rand::Rng;
use soa_derive::StructOfArray;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU32, Ordering};

/// Single agent data structure used for SoA generation.
#[derive(StructOfArray)]
#[soa_derive(Debug, Clone)]
pub struct Agent {
    /// Index into `BuildingAllocator::buildings` for the agent's home. `usize::MAX` = homeless (immigrating).
    pub home_building: usize,
    /// Index into `BuildingAllocator::buildings` for the agent's workplace. `usize::MAX` = unemployed.
    pub work_building: usize,

    /// World-space X position (metres).
    pub pos_x: f32,
    /// World-space Z position (metres, Godot forward axis).
    pub pos_y: f32,
    /// Whether the agent should be rendered this frame.
    pub is_visible: bool,

    /// Current activity: `0` = Home, `1` = Work, `2` = Shop.
    pub activity: u8,
    /// Current transit phase. One of the `TRANSIT_*` constants defined in this module.
    pub transit: u8,
    /// Agent wellbeing in `[0, 100]`.
    pub happiness: f32,
    /// Agent cash balance. Initialised at 100 for immigrants.
    pub money: f32,
    /// Internal clock for journey duration calculation.
    pub journey_start_time: f32,

    /// Building the agent is currently inside. `usize::MAX` = on the road.
    pub current_building: usize,
    /// Building the agent is travelling toward. `usize::MAX` = no active destination.
    pub target_building: usize,
    /// Graph node the agent is currently at or most recently passed through.
    pub current_node: u32,
    /// Graph node the agent is navigating toward.
    pub target_node: u32,

    /// Index into `RegionGraph::edges` for the edge the agent is currently traversing.
    pub current_edge: usize,
    /// Currently traversed lane ID in `LaneSystem`. `usize::MAX` if off-network.
    pub current_lane_id: usize,
    /// Distance (in metres) travelled along the `current_lane_id`.
    pub lane_distance: f32,
    /// Current movement speed in m/s. Updated each tick by the IDM model (cars) or held constant
    /// (pedestrians). Initialised to the edge speed limit on first lane entry.
    pub speed: f32,
    /// Current transit mode. One of the `MODE_*` constants.
    pub transit_mode: u8,

    /// Sequence of node IDs forming the planned route.
    pub current_path: Vec<u32>,
    /// Index into the active route buffer: `current_path`.
    pub current_path_index: usize,

    /// `true` if the agent owns a car and drove to their current location.
    pub has_car: bool,
    /// Type of vehicle the agent uses (if driving). One of the `VEHICLE_*` constants.
    pub vehicle_type: u8,
}

/// Simulation-wide agent system.
///
/// Refactored to use `soa_derive` for robust parallel array management.
pub struct AgentSystem {
    /// The backing SoA storage.
    pub agents: AgentVec,
    /// Global simulation time for this system.
    pub sim_time: f32,
    /// Running count of pathfinding calls this session, used for benchmark logging.
    pub pathfind_count: AtomicU32,
    /// Scratch buffer: (lane_id, agent_idx, lane_distance) sorted by (lane_id, lane_distance).
    /// Rebuilt at the start of each tick for IDM gap calculations.
    pub lane_occupants: Vec<(usize, usize, f32)>,
    /// Scratch buffer: IDM double-buffer for next-tick speeds. Avoids read-write conflicts in
    /// the parallel IDM pass.
    pub new_speed: Vec<f32>,
    /// Scratch buffer: per-edge speed sum for congestion calculation, indexed by edge ID.
    pub edge_speed_sum: Vec<f32>,
    /// Scratch buffer: per-edge agent count for congestion calculation, indexed by edge ID.
    pub edge_agent_cnt: Vec<u32>,
}

impl Deref for AgentSystem {
    type Target = AgentVec;
    fn deref(&self) -> &Self::Target {
        &self.agents
    }
}

impl DerefMut for AgentSystem {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.agents
    }
}

impl AgentSystem {
    /// Creates a new, empty agent system.
    pub fn new() -> Self {
        Self {
            agents: AgentVec::new(),
            sim_time: 0.0,
            pathfind_count: AtomicU32::new(0),
            lane_occupants: Vec::new(),
            new_speed: Vec::new(),
            edge_speed_sum: Vec::new(),
            edge_agent_cnt: Vec::new(),
        }
    }

    /// Spawns a single agent arriving at the city as an immigrant.
    pub fn spawn_agent(
        &mut self,
        home: usize,
        home_node: u32,
        _target_x: f32,
        _target_y: f32,
        highway_node: u32,
        init_x: f32,
        init_y: f32,
    ) -> usize {
        let mut rng = rand::thread_rng();
        let agent = Agent {
            home_building: home,
            work_building: usize::MAX,
            pos_x: init_x,
            pos_y: init_y,
            is_visible: true,
            activity: 0, // Heading Home
            transit: TRANSIT_IMMIGRATING,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: self.sim_time,
            current_building: usize::MAX,
            target_building: home,
            current_node: highway_node,
            target_node: home_node,
            current_edge: usize::MAX,
            current_lane_id: usize::MAX,
            lane_distance: 0.0,
            speed: 20.0,
            transit_mode: MODE_CAR,
            current_path: Vec::new(),
            current_path_index: 0,
            has_car: true,
            vehicle_type: rng.gen_range(0..4) as u8,
        };

        self.agents.push(agent);
        self.agents.len() - 1
    }

    /// Efficiency helper to spawn a large number of agents at once for testing or benchmarks.
    pub fn spawn_random_agents(
        &mut self,
        count: usize,
        graph: &RegionGraph,
        allocator: &BuildingAllocator,
    ) {
        let mut rng = rand::thread_rng();
        let node_count = graph.nodes.len();
        let bldg_count = allocator.buildings.len();
        if node_count == 0 || bldg_count == 0 {
            return;
        }

        for _ in 0..count {
            let home_idx = rng.gen_range(0..bldg_count);
            let b = &allocator.buildings[home_idx];
            let home_node = b.frontage_node;
            let start_node = rng.gen_range(0..node_count) as u32;
            let start_pos = graph.nodes[start_node as usize].pos;

            self.spawn_agent(
                home_idx,
                home_node,
                0.0,
                0.0,
                start_node,
                start_pos.x,
                start_pos.z,
            );
        }
    }

    /// Clears all agents from the system.
    pub fn clear(&mut self) {
        self.agents.clear();
        self.sim_time = 0.0;
        self.pathfind_count.store(0, Ordering::Relaxed);
    }

    /// Remaps the edge indices stored in all agents from [Old ID] to [New ID].
    pub fn update_edge_indices(&mut self, mapping: &HashMap<usize, usize>) {
        for i in 0..self.agents.len() {
            if self.agents.current_edge[i] != usize::MAX {
                if let Some(&new_id) = mapping.get(&self.agents.current_edge[i]) {
                    self.agents.current_edge[i] = new_id;
                    self.agents.current_path[i].clear();
                } else {
                    self.agents.current_edge[i] = usize::MAX;
                    self.agents.current_path[i].clear();
                }
            }
        }
    }

    /// Permanently removes an agent from the simulation using O(1) swap-remove.
    pub fn kill_agent(&mut self, index: usize, allocator: &mut BuildingAllocator) {
        if index >= self.agents.len() {
            return;
        }

        // Release vacancy if they had a home
        let home = self.agents.home_building[index];
        if home != usize::MAX {
            allocator.release_vacancy(home);
        }

        self.agents.swap_remove(index);
    }

    /// Remaps building indices after a `swap_remove` in `BuildingAllocator`. O(A).
    pub fn remap_building_indices(&mut self, mapping: &HashMap<usize, usize>) {
        if mapping.is_empty() {
            return;
        }
        for i in 0..self.agents.len() {
            if let Some(&new_id) = mapping.get(&self.agents.home_building[i]) {
                self.agents.home_building[i] = new_id;
            }
            if let Some(&new_id) = mapping.get(&self.agents.work_building[i]) {
                self.agents.work_building[i] = new_id;
            }
            if let Some(&new_id) = mapping.get(&self.agents.current_building[i]) {
                self.agents.current_building[i] = new_id;
            }
            if let Some(&new_id) = mapping.get(&self.agents.target_building[i]) {
                self.agents.target_building[i] = new_id;
            }
        }
    }

    /// Forcefully removes all agents from a building that has been deleted.
    pub fn evict_building(&mut self, building_id: usize) {
        for i in 0..self.agents.len() {
            if self.agents.work_building[i] == building_id {
                self.agents.work_building[i] = usize::MAX; // Lose Job
            }
            if self.agents.home_building[i] == building_id {
                self.agents.home_building[i] = usize::MAX; // Become Homeless
            }
            if self.agents.current_building[i] == building_id {
                // Building collapsed while they were inside!
                self.agents.current_building[i] = usize::MAX;
                self.agents.target_building[i] = usize::MAX;
                self.agents.transit[i] = TRANSIT_ARRIVING; // Dump them physically onto the sidewalk/rubble
                self.agents.is_visible[i] = true;
            } else if self.agents.target_building[i] == building_id {
                if self.agents.home_building[i] != usize::MAX {
                    // Target shop destroyed. Head back home!
                    self.agents.target_building[i] = self.agents.home_building[i];
                    self.agents.activity[i] = 0;
                } else {
                    // Target destroyed, AND homeless! Become stranded on the street!
                    self.agents.target_building[i] = usize::MAX;
                    self.agents.transit[i] = TRANSIT_ARRIVING;
                    self.agents.is_visible[i] = true;
                }
            }
        }
    }

    /// Finds a residential or mixed-use building with available vacancy.
    /// Uses the allocator's `vacancy_index` for O(1) random selection.
    pub fn find_available_home(&mut self, allocator: &mut BuildingAllocator) -> Option<usize> {
        let mut rng = rand::thread_rng();
        let target_zones = [ZoneType::Residential, ZoneType::Mixed];

        // Sum total vacancy across residential and mixed
        let mut total_vacant = 0;
        for &z in &target_zones {
            total_vacant += allocator.vacancy_index[z as usize].len();
        }

        if total_vacant == 0 {
            return None;
        }

        // Pick random building index from the combined vacancy lists
        let mut pick = rng.gen_range(0..total_vacant);
        let mut building_idx = usize::MAX;

        for &z in &target_zones {
            let list = &allocator.vacancy_index[z as usize];
            if pick < list.len() {
                building_idx = list[pick];
                break;
            }
            pick -= list.len();
        }

        if building_idx != usize::MAX {
            allocator.claim_vacancy(building_idx);
            return Some(building_idx);
        }
        None
    }

    /// Update per-day agent state: home/work bonuses and pollution penalties.
    pub fn daily_update(
        &mut self,
        pollution: &PollutionSystem,
        config: &crate::simulation::core::config::MapConfig,
    ) {
        let w = pollution.grid.width as f32;
        let h = pollution.grid.height as f32;

        for i in 0..self.agents.len() {
            // 1. Snapshot-based Activity Rewards
            if self.agents.transit[i] == TRANSIT_IDLE {
                if self.agents.activity[i] == 0 {
                    // Home
                    self.agents.happiness[i] += 1.0;
                } else if self.agents.activity[i] == 1 {
                    // Work
                    self.agents.money[i] += 10.0;
                }
            }

            // 2. Pollution Penalty
            let (gx_raw, gy_raw) = config.world_to_env_grid(self.agents.pos_x[i], self.agents.pos_y[i], w as usize, h as usize);
            let gx = gx_raw.round() as i32;
            let gy = gy_raw.round() as i32;
            if gx >= 0 && gx < w as i32 && gy >= 0 && gy < h as i32 {
                if let Some(p) = pollution.grid.get(gx as usize, gy as usize) {
                    self.agents.happiness[i] -= p * 0.1;
                }
            }

            // 3. Final Clamping
            self.agents.happiness[i] = self.agents.happiness[i].clamp(0.0, 100.0);
            self.agents.money[i] = self.agents.money[i].max(0.0);
        }
    }

    /// Aggregates per-agent speed into per-edge average speed and writes
    /// `Edge::current_congestion = 1 − avg_speed/speed_limit` for every edge that has at least
    /// one car on it. Edges with no cars are left unchanged (their congestion decays on road
    /// edits; this function does not reset them to zero).
    ///
    /// Must be called once per tick by `simulate_tick_internal` after `AgentSystem::tick()`.
    /// O(A + E) sequential.
    pub fn update_edge_congestion(&mut self, graph: &mut RegionGraph) {
        let edge_count = graph.edges.len();
        self.edge_speed_sum.clear();
        self.edge_speed_sum.resize(edge_count, 0.0_f32);
        self.edge_agent_cnt.clear();
        self.edge_agent_cnt.resize(edge_count, 0_u32);

        for i in 0..self.agents.len() {
            if self.agents.transit[i] == TRANSIT_ON_ROAD {
                let eid = self.agents.current_edge[i];
                if eid != usize::MAX && eid < edge_count {
                    self.edge_speed_sum[eid] += self.agents.speed[i];
                    self.edge_agent_cnt[eid] += 1;
                }
            }
        }

        for eid in 0..edge_count {
            if !graph.edges[eid].deleted && self.edge_agent_cnt[eid] > 0 {
                let avg = self.edge_speed_sum[eid] / self.edge_agent_cnt[eid] as f32;
                let limit = graph.edges[eid].speed_limit.max(1.0);
                graph.edges[eid].current_congestion = (1.0 - avg / limit).max(0.0);
            }
        }
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
}
