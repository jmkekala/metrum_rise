//! Core data layout and lifecycle management for the agent simulation.
//!
//! # Memory layout
//!
//! [`AgentSystem`] uses a Structure-of-Arrays (SoA) layout — every field is a separate
//! `Vec<T>` indexed by agent ID. This enables cache-friendly bulk iteration and is a
//! prerequisite for future `rayon::par_iter_mut` parallelisation.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::graph::TransitGraph;
use crate::simulation::grid::pollution::PollutionSystem;
use super::{TRANSIT_IMMIGRATING, TRANSIT_IDLE, MODE_CAR};
use godot::prelude::*;
use rand::Rng;
use std::collections::HashMap;

/// Simulation-wide agent state stored in Structure-of-Arrays layout.
///
/// All `Vec` fields are parallel arrays indexed by agent ID `i` (0..`count`).
/// Add/remove agents only through [`spawn_agent`](Self::spawn_agent) and
/// [`kill_agent`](Self::kill_agent) to keep all arrays in sync.
pub struct AgentSystem {
    /// Number of live agents. All parallel `Vec` fields have exactly this many elements.
    pub count: usize,

    // Core Identity

    /// Index into `BuildingAllocator::buildings` for the agent's home. `usize::MAX` = homeless (immigrating).
    pub home_building: Vec<usize>,
    /// Index into `BuildingAllocator::buildings` for the agent's workplace. `usize::MAX` = unemployed.
    pub work_building: Vec<usize>,

    // Physics / Rendering

    /// World-space X position (metres).
    pub pos_x: Vec<f32>,
    /// World-space Z position (metres, Godot forward axis).
    pub pos_y: Vec<f32>,
    /// Whether the agent should be rendered this frame.
    pub is_visible: Vec<bool>,

    /// Current activity: `0` = Home, `1` = Work, `2` = Shop.
    pub activity: Vec<u8>,
    /// Current transit phase. One of the `TRANSIT_*` constants defined in this module.
    pub transit: Vec<u8>,
    /// Agent wellbeing in `[0, 100]`.
    pub happiness: Vec<f32>,
    /// Agent cash balance. Initialised at 100 for immigrants.
    pub money: Vec<f32>,
    /// Internal clock for journey duration calculation.
    pub journey_start_time: Vec<f32>,
    /// Global simulation time for this system.
    pub sim_time: f32,

    // Routing Geometry

    /// Building the agent is currently inside. `usize::MAX` = on the road.
    pub current_building: Vec<usize>,
    /// Building the agent is travelling toward. `usize::MAX` = no active destination.
    pub target_building: Vec<usize>,
    /// Graph node the agent is currently at or most recently passed through.
    pub current_node: Vec<u32>,
    /// Graph node the agent is navigating toward.
    pub target_node: Vec<u32>,

    // Spline Geometry

    /// Index into `TransitGraph::edges` for the edge the agent is currently traversing.
    pub current_edge: Vec<usize>,
    /// Progress along `current_edge` as a signed segment index. Positive = forward, negative = reverse.
    pub edge_progression: Vec<isize>,
    /// Lateral lane offset index on the current edge. Positive = forward lane, negative = backward lane.
    pub current_lane: Vec<i8>,
    /// Current transit mode. One of the `MODE_*` constants.
    pub transit_mode: Vec<u8>,

    // Traffic Lane Manager — Bezier Intersection Pathing

    /// Bezier control point 0 (start), world-space X/Z.
    pub bezier_p0_x: Vec<f32>,
    /// Bezier control point 0, world-space Z.
    pub bezier_p0_y: Vec<f32>,
    /// Bezier control point 1 (first handle), world-space X.
    pub bezier_p1_x: Vec<f32>,
    /// Bezier control point 1, world-space Z.
    pub bezier_p1_y: Vec<f32>,
    /// Bezier control point 2 (second handle), world-space X.
    pub bezier_p2_x: Vec<f32>,
    /// Bezier control point 2, world-space Z.
    pub bezier_p2_y: Vec<f32>,
    /// Bezier control point 3 (end), world-space X.
    pub bezier_p3_x: Vec<f32>,
    /// Bezier control point 3, world-space Z.
    pub bezier_p3_y: Vec<f32>,
    /// Normalised interpolation parameter `t ∈ [0, 1]` along the current bezier curve.
    pub bezier_t: Vec<f32>,
    /// Sequence of node IDs forming the planned route. Each inner `Vec` is one path segment.
    pub current_path: Vec<Vec<u32>>,
    /// Index into `current_path` of the node the agent is currently heading toward.
    pub current_path_index: Vec<usize>,

    /// `true` if the agent owns a car and drove to their current location.
    pub has_car: Vec<bool>,
    /// Running count of pathfinding calls this session, used for benchmark logging.
    pub pathfind_count: u32,
}

impl AgentSystem {
    /// Creates a new, empty agent system.
    pub fn new() -> Self {
        Self {
            count: 0,
            home_building: Vec::new(),
            work_building: Vec::new(),
            pos_x: Vec::new(),
            pos_y: Vec::new(),
            is_visible: Vec::new(),
            activity: Vec::new(),
            transit: Vec::new(),
            happiness: Vec::new(),
            money: Vec::new(),
            current_building: Vec::new(),
            target_building: Vec::new(),
            current_node: Vec::new(),
            target_node: Vec::new(),
            current_edge: Vec::new(),
            edge_progression: Vec::new(),
            current_lane: Vec::new(),
            transit_mode: Vec::new(),
            bezier_p0_x: Vec::new(),
            bezier_p0_y: Vec::new(),
            bezier_p1_x: Vec::new(),
            bezier_p1_y: Vec::new(),
            bezier_p2_x: Vec::new(),
            bezier_p2_y: Vec::new(),
            bezier_p3_x: Vec::new(),
            bezier_p3_y: Vec::new(),
            bezier_t: Vec::new(),
            current_path: Vec::new(),
            current_path_index: Vec::new(),
            has_car: Vec::new(),
            journey_start_time: Vec::new(),
            sim_time: 0.0,
            pathfind_count: 0,
        }
    }

    /// Spawns a single agent arriving at the city as an immigrant.
    pub fn spawn_agent(&mut self, home: usize, home_node: u32, _target_x: f32, _target_y: f32, highway_node: u32, init_x: f32, init_y: f32) -> usize {
        self.home_building.push(home);
        self.work_building.push(usize::MAX);
        self.pos_x.push(init_x);
        self.pos_y.push(init_y);
        self.is_visible.push(true);
        self.activity.push(0); // Heading Home
        self.transit.push(TRANSIT_IMMIGRATING); 
        self.happiness.push(50.0);
        self.money.push(100.0); // Immigrants bring $100
        self.journey_start_time.push(self.sim_time);
        
        self.current_building.push(usize::MAX);
        self.target_building.push(home);
        self.current_node.push(highway_node);
        self.target_node.push(home_node);
        self.current_edge.push(usize::MAX);
        self.edge_progression.push(0);
        self.current_lane.push(0);
        self.transit_mode.push(MODE_CAR); // Immigrants always arrive in cars!
        self.bezier_p0_x.push(0.0);
        self.bezier_p0_y.push(0.0);
        self.bezier_p1_x.push(0.0);
        self.bezier_p1_y.push(0.0);
        self.bezier_p2_x.push(0.0);
        self.bezier_p2_y.push(0.0);
        self.bezier_p3_x.push(0.0);
        self.bezier_p3_y.push(0.0);
        self.bezier_t.push(0.0);
        self.current_path.push(Vec::new());
        self.current_path_index.push(0);
        
        self.has_car.push(true); // Immigrants arrive with a car!
        self.count += 1;
        
        self.count - 1
    }

    /// Efficiency helper to spawn a large number of agents at once for testing or benchmarks.
    pub fn spawn_random_agents(&mut self, count: usize, graph: &TransitGraph, allocator: &BuildingAllocator) {
        let mut rng = rand::thread_rng();
        let node_count = graph.nodes.len();
        let bldg_count = allocator.buildings.len();
        if node_count == 0 || bldg_count == 0 { return; }

        for _ in 0..count {
            let home_idx = rng.gen_range(0..bldg_count);
            let b = &allocator.buildings[home_idx];
            let edge = &graph.edges[b.edge_idx];
            let home_node = if b.frontage_t < 0.5 { edge.start_node } else { edge.end_node };
            let start_node = rng.gen_range(0..node_count) as u32;
            let start_pos = graph.nodes[start_node as usize].pos;

            self.spawn_agent(home_idx, home_node, 0.0, 0.0, start_node, start_pos.x, start_pos.z);
        }
    }

    /// Clears all agents from the system.
    pub fn clear(&mut self) {
        self.home_building.clear();
        self.work_building.clear();
        self.pos_x.clear();
        self.pos_y.clear();
        self.is_visible.clear();
        self.activity.clear();
        self.transit.clear();
        self.happiness.clear();
        self.money.clear();
        self.current_building.clear();
        self.target_building.clear();
        self.current_node.clear();
        self.target_node.clear();
        self.current_edge.clear();
        self.edge_progression.clear();
        self.current_lane.clear();
        self.transit_mode.clear();
        self.bezier_p0_x.clear();
        self.bezier_p0_y.clear();
        self.bezier_p1_x.clear();
        self.bezier_p1_y.clear();
        self.bezier_p2_x.clear();
        self.bezier_p2_y.clear();
        self.bezier_p3_x.clear();
        self.bezier_p3_y.clear();
        self.bezier_t.clear();
        self.current_path.clear();
        self.current_path_index.clear();
        self.has_car.clear();
        self.journey_start_time.clear();
        self.sim_time = 0.0;
        self.count = 0;
        self.pathfind_count = 0;
    }

    /// Remaps the edge indices stored in all agents from [Old ID] to [New ID].
    /// Remaps the edge indices stored in all agents from [Old ID] to [New ID].
    pub fn update_edge_indices(&mut self, mapping: &HashMap<usize, usize>) {
        for i in 0..self.count {
            if self.current_edge[i] != usize::MAX {
                if let Some(&new_id) = mapping.get(&self.current_edge[i]) {
                    self.current_edge[i] = new_id;
                } else {
                    self.current_edge[i] = usize::MAX;
                    self.current_path[i].clear();
                }
            }
        }
    }
    
    /// Permanently removes an agent from the simulation using O(1) swap-remove.
    pub fn kill_agent(&mut self, index: usize, allocator: &mut BuildingAllocator) {
        if index >= self.count { return; }
        
        // Release vacancy if they had a home
        let home = self.home_building[index];
        if home != usize::MAX {
            allocator.release_vacancy(home);
        }

        let last_idx = self.count - 1;
        
        self.home_building.swap(index, last_idx);
        self.work_building.swap(index, last_idx);
        self.pos_x.swap(index, last_idx);
        self.pos_y.swap(index, last_idx);
        self.is_visible.swap(index, last_idx);
        self.activity.swap(index, last_idx);
        self.transit.swap(index, last_idx);
        self.happiness.swap(index, last_idx);
        self.money.swap(index, last_idx);
        self.current_building.swap(index, last_idx);
        self.target_building.swap(index, last_idx);
        self.current_node.swap(index, last_idx);
        self.target_node.swap(index, last_idx);
        self.current_edge.swap(index, last_idx);
        self.edge_progression.swap(index, last_idx);
        self.current_lane.swap(index, last_idx);
        self.transit_mode.swap(index, last_idx);
        self.bezier_p0_x.swap(index, last_idx);
        self.bezier_p0_y.swap(index, last_idx);
        self.bezier_p1_x.swap(index, last_idx);
        self.bezier_p1_y.swap(index, last_idx);
        self.bezier_p2_x.swap(index, last_idx);
        self.bezier_p2_y.swap(index, last_idx);
        self.bezier_p3_x.swap(index, last_idx);
        self.bezier_p3_y.swap(index, last_idx);
        self.bezier_t.swap(index, last_idx);
        self.current_path.swap(index, last_idx);
        self.current_path_index.swap(index, last_idx);
        self.journey_start_time.swap(index, last_idx);

        self.home_building.pop();
        self.work_building.pop();
        self.pos_x.pop();
        self.pos_y.pop();
        self.is_visible.pop();
        self.activity.pop();
        self.transit.pop();
        self.happiness.pop();
        self.money.pop();
        self.current_building.pop();
        self.target_building.pop();
        self.current_node.pop();
        self.target_node.pop();
        self.current_edge.pop();
        self.edge_progression.pop();
        self.current_lane.pop();
        self.transit_mode.pop();
        self.bezier_p0_x.pop();
        self.bezier_p0_y.pop();
        self.bezier_p1_x.pop();
        self.bezier_p1_y.pop();
        self.bezier_p2_x.pop();
        self.bezier_p2_y.pop();
        self.bezier_p3_x.pop();
        self.bezier_p3_y.pop();
        self.bezier_t.pop();
        self.current_path.pop();
        self.current_path_index.pop();
        self.journey_start_time.pop();
        
        self.count -= 1;
    }

    /// Remaps building indices after a `swap_remove` in `BuildingAllocator`. O(A).
    pub fn remap_building_indices(&mut self, mapping: &HashMap<usize, usize>) {
        if mapping.is_empty() { return; }
        for i in 0..self.count {
            if let Some(&new_id) = mapping.get(&self.home_building[i]) {
                self.home_building[i] = new_id;
            }
            if let Some(&new_id) = mapping.get(&self.work_building[i]) {
                self.work_building[i] = new_id;
            }
            if let Some(&new_id) = mapping.get(&self.current_building[i]) {
                self.current_building[i] = new_id;
            }
            if let Some(&new_id) = mapping.get(&self.target_building[i]) {
                self.target_building[i] = new_id;
            }
        }
    }

    /// Forcefully removes all agents from a building that has been deleted.
    pub fn evict_building(&mut self, building_id: usize) {
        for i in 0..self.count {
            if self.work_building[i] == building_id {
                self.work_building[i] = usize::MAX; // Lose Job
            }
            if self.home_building[i] == building_id {
                self.home_building[i] = usize::MAX; // Become Homeless
            }
            if self.current_building[i] == building_id { // Building collapsed while they were inside!
                self.current_building[i] = usize::MAX;
                self.target_building[i] = usize::MAX;
                self.transit[i] = 3; // Dump them physically onto the sidewalk/rubble
                self.is_visible[i] = true;
            } else if self.target_building[i] == building_id {
                if self.home_building[i] != usize::MAX {
                    // Target shop destroyed. Head back home!
                    self.target_building[i] = self.home_building[i];
                    self.activity[i] = 0;
                } else {
                    // Target destroyed, AND homeless! Become stranded on the street!
                    self.target_building[i] = usize::MAX;
                    self.transit[i] = 3;
                    self.is_visible[i] = true;
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
        
        if total_vacant == 0 { return None; }
        
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
    /// Update per-day agent state: home/work bonuses and pollution penalties.
    pub fn daily_update(&mut self, pollution: &PollutionSystem) {
        let w = pollution.grid.width as f32;
        let h = pollution.grid.height as f32;
        let world_size_x = crate::config::MAP_WIDTH as f32 * crate::config::GRID_CELL_SIZE;
        let world_size_y = crate::config::MAP_HEIGHT as f32 * crate::config::GRID_CELL_SIZE;

        for i in 0..self.count {
            // 1. Snapshot-based Activity Rewards
            if self.transit[i] == TRANSIT_IDLE {
                if self.activity[i] == 0 { // Home
                    self.happiness[i] += 1.0;
                } else if self.activity[i] == 1 { // Work
                    self.money[i] += 10.0;
                }
            }

            // 2. Pollution Penalty
            let gx = (((self.pos_x[i] / world_size_x) + 0.5) * w).round() as i32;
            let gy = (((self.pos_y[i] / world_size_y) + 0.5) * h).round() as i32;
            if gx >= 0 && gx < w as i32 && gy >= 0 && gy < h as i32 {
                if let Some(p) = pollution.grid.get(gx as usize, gy as usize) {
                    self.happiness[i] -= p * 0.1;
                }
            }

            // 3. Final Clamping
            self.happiness[i] = self.happiness[i].clamp(0.0, 100.0);
            self.money[i] = self.money[i].max(0.0);
        }
    }

    /// Re-calculates building occupancy and vacancy index from scratch.
    pub fn recalculate_occupancy(&mut self, allocator: &mut BuildingAllocator) {
        for b in &mut allocator.buildings {
            b.occupancy = 0;
        }
        for i in 0..self.count {
            let h = self.home_building[i];
            if h != usize::MAX && h < allocator.buildings.len() {
                allocator.buildings[h].occupancy += 1;
            }
        }
        allocator.rebuild_zone_index();
    }
}
