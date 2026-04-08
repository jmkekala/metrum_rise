//! Core data layout and lifecycle management for the agent simulation.
//!
//! # Memory layout
//!
//! [`AgentSystem`] uses a Structure-of-Arrays (SoA) layout provided by the `soa_derive` crate.
//! This enables cache-friendly bulk iteration and ensures all fields are kept in sync.

use super::{MODE_CAR, TRANSIT_ARRIVING, TRANSIT_IDLE, TRANSIT_IMMIGRATING};
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
    /// Index into `HouseholdSystem::households` for the agent's shared household record.
    pub household_id: usize,
    /// Index into `BuildingAllocator::buildings` for the agent's workplace. `usize::MAX` = unemployed.
    pub work_building: usize,

    /// World-space X position (metres).
    pub pos_x: f32,
    /// World-space Z position (metres, Godot forward axis).
    pub pos_y: f32,
    /// Whether the agent should be rendered this frame.
    pub is_visible: bool,

    /// Current activity: `0` = Home, `1` = Work, `2` = other non-home stop.
    pub activity: u8,
    /// Current transit phase. One of the `TRANSIT_*` constants defined in this module.
    pub transit: u8,
    /// Agent wellbeing in `[0, 100]`.
    pub happiness: f32,
    /// Per-agent view of household money. Synced from the shared household budget.
    pub money: f32,
    /// Internal clock for journey duration calculation.
    pub journey_start_time: f32,

    /// Building the agent is currently inside. `usize::MAX` = on the road.
    pub current_building: usize,
    /// Building the agent is travelling toward. `usize::MAX` = no active destination.
    pub target_building: usize,
    /// Economy-selected destination building for the next idle departure.
    pub planned_target_building: usize,
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
    /// Economy-selected next activity to execute on the next idle departure.
    pub planned_activity: u8,

    /// Sequence of node IDs forming the planned route.
    pub current_path: Vec<u32>,
    /// Index into the active route buffer: `current_path`.
    pub current_path_index: usize,

    /// `true` if the agent owns a car and drove to their current location.
    pub has_car: bool,
    /// Type of vehicle the agent uses (if driving). One of the `VEHICLE_*` constants.
    pub vehicle_type: u8,

    /// Variant for pedestrian models (0-3: Male A, Male B, Female A, Female B).
    pub pedestrian_type: u8,
    /// Current cycle of the walk animation [0, 1].
    pub walk_phase: f32,
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
    /// Scratch buffer: per-lane agent lists for IDM gap lookup and overlap correction.
    /// Indexed by lane ID; each entry is `(lane_distance, agent_idx)` sorted ascending by dist.
    /// Sized to `LaneSystem::lanes.len()`; grows but never shrinks.
    pub lane_buckets: Vec<Vec<(f32, usize)>>,
    /// Dedup flag array — `lane_is_dirty[lid] == true` if lane `lid` has been pushed to this
    /// tick. Sized alongside `lane_buckets`. Cleared via `dirty_lanes` rather than a full scan.
    pub lane_is_dirty: Vec<bool>,
    /// Compact list of lane IDs that were touched this tick (no duplicates).
    /// Used to clear only the dirty buckets at the start of the next tick.
    pub dirty_lanes: Vec<usize>,
    /// Scratch buffer: IDM double-buffer for next-tick speeds. Avoids read-write conflicts in
    /// the parallel IDM pass.
    pub new_speed: Vec<f32>,
    /// Scratch buffer: per-connection-lane occupancy snapshot.
    /// `conn_occupied[lane_id] == true` when at least one `TRANSIT_INTERSECTION` agent
    /// occupies that connection lane at the start of the current tick.
    /// Built sequentially before the parallel movement pass; shared as a read-only
    /// reference during it so no synchronisation is needed.
    pub conn_occupied: Vec<bool>,
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
            lane_buckets: Vec::new(),
            lane_is_dirty: Vec::new(),
            dirty_lanes: Vec::new(),
            new_speed: Vec::new(),
            conn_occupied: Vec::new(),
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
            household_id: usize::MAX,
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
            planned_target_building: usize::MAX,
            current_node: highway_node,
            target_node: home_node,
            current_edge: usize::MAX,
            current_lane_id: usize::MAX,
            lane_distance: 0.0,
            speed: 20.0,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: Vec::new(),
            current_path_index: 0,
            has_car: true,
            vehicle_type: rng.gen_range(0..4) as u8,
            pedestrian_type: rng.gen_range(0..4) as u8,
            walk_phase: rng.gen_range(0.0..1.0),
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
        let node_count = graph.node_count();
        let bldg_count = allocator.buildings.len();
        if node_count == 0 || bldg_count == 0 {
            return;
        }

        for _ in 0..count {
            let home_idx = rng.gen_range(0..bldg_count);
            let b = &allocator.buildings[home_idx];
            let home_node = crate::simulation::buildings::allocator::building_depart_node(b, graph);
            let start_node = rng.gen_range(0..graph.node_count()) as u32;
            let start_pos = graph.node(start_node).pos;

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
            if let Some(&new_id) = mapping.get(&self.agents.planned_target_building[i]) {
                self.agents.planned_target_building[i] = new_id;
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
                self.agents.household_id[i] = usize::MAX;
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
                    self.agents.planned_target_building[i] = self.agents.home_building[i];
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
                }
            }

            // 2. Pollution Penalty
            let (gx_raw, gy_raw) = config.world_to_env_grid(
                self.agents.pos_x[i],
                self.agents.pos_y[i],
                w as usize,
                h as usize,
            );
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

    /// Sets `current_lane_id = usize::MAX` for every agent whose active lane belongs to one
    /// of `affected_edges`, or whose connection lane leads directly into such a lane.
    ///
    /// Must be called **before** `LaneSystem::rebuild_edges_incremental` so the old lane IDs
    /// are still valid for lookup — old orphaned lanes retain their original `edge_id` even
    /// after `rebuild_edges_incremental` removes them from `edge_lanes`.
    pub fn invalidate_lane_ids_for_edges(
        &mut self,
        affected_edges: &std::collections::HashSet<usize>,
        lane_system: &crate::simulation::network::lanes::LaneSystem,
    ) {
        // Collect lane IDs that belong to affected edges (from the current edge_lanes).
        let mut affected_lane_ids: std::collections::HashSet<usize> =
            std::collections::HashSet::new();
        for &edge_id in affected_edges {
            if let Some(lane_ids) = lane_system.edge_lanes.get(&edge_id) {
                affected_lane_ids.extend(lane_ids);
            }
        }

        for i in 0..self.agents.len() {
            let lid = self.agents.current_lane_id[i];
            if lid == usize::MAX || lid >= lane_system.lanes.len() {
                continue;
            }
            let lane = &lane_system.lanes[lid];
            let should_invalidate = if lane.edge_id != usize::MAX {
                // Road lane: invalidate if its edge is affected.
                affected_edges.contains(&lane.edge_id)
            } else {
                // Connection lane: invalidate if it leads into an affected road lane.
                lane.next_lanes
                    .first()
                    .map_or(false, |&next| affected_lane_ids.contains(&next))
            };
            if should_invalidate {
                self.agents.current_lane_id[i] = usize::MAX;
                // lane_distance is intentionally NOT zeroed here: the agent keeps its
                // visual position until the next tick re-attaches it to a new lane.
                // Zeroing it caused agents to visually teleport to edge position 0
                // immediately when a nearby road was modified.
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

#[cfg(test)]
mod tests {
    use super::super::TRANSIT_ON_ROAD;
    use super::*;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::graph::data::Edge;
    use crate::simulation::network::lanes::LaneSystem;
    use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};
    use godot::prelude::Vector3;
    use std::collections::HashSet;

    fn make_simple_lane_system() -> (RegionGraph, LaneSystem) {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(
            Vector3::new(0.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n1 = graph.add_node(
            Vector3::new(100.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n2 = graph.add_node(
            Vector3::new(200.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );

        let _e0 = graph.add_edge(Edge {
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
            ..Default::default()
        });
        let _e1 = graph.add_edge(Edge {
            start_node: n1,
            end_node: n2,
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
            geometry: vec![Vector3::new(100.0, 0.0, 0.0), Vector3::new(200.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(100.0, 0.0, 0.0), Vector3::new(200.0, 0.0, 0.0)],
            ..Default::default()
        });
        graph.rebuild_adjacency_list();

        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        (graph, lanes)
    }

    #[test]
    fn test_invalidate_clears_agents_on_affected_edge() {
        let (_graph, lane_system) = make_simple_lane_system();

        // Place two agents: one on edge 0, one on edge 1.
        let e0_lane = lane_system.edge_lanes[&0][0];
        let e1_lane = lane_system.edge_lanes[&1][0];

        let mut sys = AgentSystem::new();
        // Spawn minimal agents and manually set their lane IDs.
        sys.agents.push(Agent {
            home_building: usize::MAX,
            household_id: usize::MAX,
            work_building: usize::MAX,
            pos_x: 0.0,
            pos_y: 0.0,
            is_visible: true,
            activity: 0,
            transit: TRANSIT_ON_ROAD,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: 0.0,
            current_building: usize::MAX,
            target_building: usize::MAX,
            planned_target_building: usize::MAX,
            current_node: 0,
            target_node: 1,
            current_edge: 0,
            current_lane_id: e0_lane,
            lane_distance: 10.0,
            speed: 10.0,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: vec![],
            current_path_index: 0,
            has_car: true,
            vehicle_type: 0,
            pedestrian_type: 0,
            walk_phase: 0.0,
        });
        sys.agents.push(Agent {
            home_building: usize::MAX,
            household_id: usize::MAX,
            work_building: usize::MAX,
            pos_x: 150.0,
            pos_y: 0.0,
            is_visible: true,
            activity: 0,
            transit: TRANSIT_ON_ROAD,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: 0.0,
            current_building: usize::MAX,
            target_building: usize::MAX,
            planned_target_building: usize::MAX,
            current_node: 1,
            target_node: 2,
            current_edge: 1,
            current_lane_id: e1_lane,
            lane_distance: 10.0,
            speed: 10.0,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: vec![],
            current_path_index: 0,
            has_car: true,
            vehicle_type: 0,
            pedestrian_type: 0,
            walk_phase: 0.0,
        });

        // Invalidate only edge 0.
        let mut affected = HashSet::new();
        affected.insert(0usize);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system);

        // Agent 0 (on edge 0) must be invalidated.
        assert_eq!(
            sys.agents.current_lane_id[0],
            usize::MAX,
            "Agent on affected edge should have lane_id cleared"
        );
        // lane_distance is intentionally preserved so the agent doesn't visually teleport
        // to position 0 before the next tick re-attaches it.
        assert_eq!(
            sys.agents.lane_distance[0], 10.0,
            "lane_distance should be preserved to avoid visual teleport on road edit"
        );

        // Agent 1 (on edge 1) must be untouched.
        assert_eq!(
            sys.agents.current_lane_id[1], e1_lane,
            "Agent on unaffected edge should keep its lane_id"
        );
    }

    #[test]
    fn test_invalidate_skips_already_invalid_agents() {
        let (_graph, lane_system) = make_simple_lane_system();

        let mut sys = AgentSystem::new();
        sys.agents.push(Agent {
            home_building: usize::MAX,
            household_id: usize::MAX,
            work_building: usize::MAX,
            pos_x: 0.0,
            pos_y: 0.0,
            is_visible: false,
            activity: 0,
            transit: TRANSIT_IDLE,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: 0.0,
            current_building: 0,
            target_building: 0,
            planned_target_building: usize::MAX,
            current_node: 0,
            target_node: 0,
            current_edge: usize::MAX,
            current_lane_id: usize::MAX,
            lane_distance: 0.0,
            speed: 0.0,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: vec![],
            current_path_index: 0,
            has_car: false,
            vehicle_type: 0,
            pedestrian_type: 0,
            walk_phase: 0.0,
        });

        let mut affected = HashSet::new();
        affected.insert(0usize);
        // Should not panic on agents already at usize::MAX.
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system);
        assert_eq!(sys.agents.current_lane_id[0], usize::MAX);
    }
}
