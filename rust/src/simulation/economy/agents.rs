//! Agent simulation: state machine, movement, pathfinding decisions, and immigration.
//!
//! # Memory layout
//!
//! [`AgentSystem`] uses a Structure-of-Arrays (SoA) layout — every field is a separate
//! `Vec<T>` indexed by agent ID. This enables cache-friendly bulk iteration and is a
//! prerequisite for future `rayon::par_iter_mut` parallelisation.
//!
//! # Finite state machine
//!
//! Each agent has two independent state axes:
//! - **Activity** (`activity[i]`): what the agent is trying to do — Home, Work, or Shop.
//! - **Transit** (`transit[i]`): the agent's current movement phase (see `TRANSIT_*` constants).
//!
//! # Known issues (see `docs/project.md`)
//! - `AgentSystem::tick` is single-threaded — Rayon parallelisation is a v0.01 goal.
//! - `happiness` and `money` are never modified (bug B10).

use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::TransitGraph;
use crate::simulation::network::types::TransitType;
use crate::simulation::pathing::hpa::HpaGraph;
use godot::prelude::*;
use rand::Rng;


/// Agent is inside a building — not moving. Waiting for the next activity trigger.
pub const TRANSIT_IDLE: u8 = 0;
/// Agent is walking from building interior to the road edge (departure animation phase).
pub const TRANSIT_DEPARTING: u8 = 1;
/// Agent is actively moving along road edges toward `target_node`.
pub const TRANSIT_ON_ROAD: u8 = 2;
/// Agent has reached the road node nearest their destination and is walking the final distance to the building.
pub const TRANSIT_ARRIVING: u8 = 3;
/// Newly spawned agent travelling from a highway border node to their first home — has no current building.
pub const TRANSIT_IMMIGRATING: u8 = 4;
/// Agent is traversing a bezier curve through a road intersection (lane-change phase).
pub const TRANSIT_INTERSECTION: u8 = 5;

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
    /// `true` if the agent is currently in a car; `false` if walking.
    pub is_driving: Vec<bool>,

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
            is_driving: Vec::new(),
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
        self.is_driving.push(true); // Immigrants always arrive in cars!
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
        
        if home == usize::MAX {
            // println!("Immigrant spawned at border node {} (pos: {}, {})", highway_node, init_x, init_y);
        }
        self.count - 1
    }

    pub fn spawn_random_agents(&mut self, count: usize, graph: &TransitGraph, allocator: &crate::simulation::buildings::allocator::BuildingAllocator) {
        let mut rng = rand::thread_rng();
        let node_count = graph.nodes.len();
        let bldg_count = allocator.buildings.len();
        if node_count == 0 || bldg_count == 0 { return; }

        for _ in 0..count {
            let home_idx = rng.gen_range(0..bldg_count);
            let home_node = allocator.buildings[home_idx].frontage_node;
            let start_node = rng.gen_range(0..node_count) as u32;
            let start_pos = graph.nodes[start_node as usize].pos;

            self.spawn_agent(home_idx, home_node, 0.0, 0.0, start_node, start_pos.x, start_pos.z);
        }
    }

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
        self.is_driving.clear();
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
    pub fn update_edge_indices(&mut self, mapping: &std::collections::HashMap<usize, usize>) {
        for i in 0..self.count {
            if self.current_edge[i] != usize::MAX {
                if let Some(&new_id) = mapping.get(&self.current_edge[i]) {
                    self.current_edge[i] = new_id;
                } else {
                    // Current edge was removed or not part of the compaction.
                    // Recover by placing the agent back at the start of their search segment.
                    self.current_edge[i] = usize::MAX;
                    self.current_path[i].clear();
                }
            }
        }
    }
    
    pub fn kill_agent(&mut self, index: usize) {
        if index >= self.count { return; }
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
        self.is_driving.swap(index, last_idx);
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
        self.is_driving.pop();
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
    
    pub fn decide_transit_mode(
        &mut self,
        i: usize,
        target_node: u32,
        graph: &crate::simulation::network::graph::TransitGraph,
        hpa: &crate::simulation::pathing::hpa::HpaGraph,
    ) -> (u32, bool) {
        self.pathfind_count += 1;
        let current_node = self.current_node[i];
        let mut pedestrian_dist = 10000.0;
        if let Some((_cost, _dist, _path)) = hpa.find_path(current_node, target_node, usize::MAX, graph, true) {
            pedestrian_dist = _dist;
        }


        if pedestrian_dist > 500.0 && self.has_car[i] {
            // Far target and has car, but ONLY drive if a driving path actually exists!
            self.pathfind_count += 1;
            if hpa.find_path(current_node, target_node, usize::MAX, graph, false).is_some() {
                return (target_node, true);
            }
        }
        
        // Close target, no car, OR car path disconnected -> Walk
        return (target_node, false);
    }

    pub fn find_available_home(&self, allocator: &BuildingAllocator) -> Option<usize> {
        let mut occupancy = vec![0; allocator.buildings.len()];
        for i in 0..self.count {
            if self.home_building[i] != usize::MAX && self.home_building[i] < allocator.buildings.len() {
                occupancy[self.home_building[i]] += 1;
            }
        }
        
        for (idx, b) in allocator.buildings.iter().enumerate() {
            if b.zone_type == ZoneType::Residential && occupancy[idx] < 6 {
                return Some(idx);
            }
        }
        None
    }
    
    pub fn tick(&mut self, allocator: &BuildingAllocator, hpa_graph: &HpaGraph, graph: &mut TransitGraph, delta: f32) {
        self.sim_time += delta;
        let mut rng = rand::thread_rng();
        
        // 1. Safety Scrub: Building indices are volatile
        for i in 0..self.count {
            if self.home_building[i] != usize::MAX && self.home_building[i] >= allocator.buildings.len() {
                self.home_building[i] = usize::MAX;
            }
            if self.work_building[i] != usize::MAX && self.work_building[i] >= allocator.buildings.len() {
                self.work_building[i] = usize::MAX;
            }
            if self.current_building[i] != usize::MAX && self.current_building[i] >= allocator.buildings.len() {
                self.current_building[i] = usize::MAX;
                self.transit[i] = 3; // Dump onto street if interior disappears
            }
            if self.target_building[i] != usize::MAX && self.target_building[i] >= allocator.buildings.len() {
                self.target_building[i] = usize::MAX;
                if self.home_building[i] != usize::MAX {
                    self.target_building[i] = self.home_building[i];
                } else {
                    self.transit[i] = TRANSIT_ARRIVING;
                }
            }
        }
        


        macro_rules! initiate_journey {
            ($self:expr, $i:expr) => {
                let curr = $self.current_building[$i];
                if curr != usize::MAX && curr < allocator.buildings.len() {
                    let b = &allocator.buildings[curr];
                    $self.transit[$i] = TRANSIT_DEPARTING; 
                    $self.current_node[$i] = b.frontage_node;
                    $self.is_visible[$i] = true;
                    // Note: current_building is cleared AFTER reaching frontage in Transit 1
                } else {
                    $self.transit[$i] = 10;
                }
            }
        }

        // Swarm Iteration
        for i in 0..self.count {
            self.current_node[i] = graph.get_valid_node(self.current_node[i]);
            self.target_node[i] = graph.get_valid_node(self.target_node[i]);
            
            match self.transit[i] {
                TRANSIT_IDLE => { // INSIDE BUILDING
                    if rng.gen_bool((0.05 * delta) as f64) {
                        let mut next_act = self.activity[i];
                        let mut next_bldg = usize::MAX;

                        if self.activity[i] == 0 { // Heading to Work or Shop
                            if self.money[i] >= 20.0 && rng.gen_bool(0.4) {
                                // Go Shopping
                                let shops: Vec<usize> = allocator.buildings.iter().enumerate()
                                    .filter(|(_, b)| b.zone_type == ZoneType::Commercial)
                                    .map(|(idx, _)| idx).collect();
                                if !shops.is_empty() {
                                    next_bldg = shops[rng.gen_range(0..shops.len())];
                                    next_act = 2;
                                }
                            } else {
                                // Go to Work
                                if self.work_building[i] == usize::MAX {
                                    let jobs: Vec<usize> = allocator.buildings.iter().enumerate()
                                        .filter(|(_, b)| b.zone_type == ZoneType::Industrial || b.zone_type == ZoneType::Commercial)
                                        .map(|(idx, _)| idx).collect();
                                    if !jobs.is_empty() { self.work_building[i] = jobs[rng.gen_range(0..jobs.len())]; }
                                }
                                if self.work_building[i] != usize::MAX {
                                    next_bldg = self.work_building[i];
                                    next_act = 1;
                                }
                            }
                        } else { // At Work/Shop, go Home
                            if self.home_building[i] != usize::MAX {
                                next_bldg = self.home_building[i];
                                next_act = 0;
                            }
                        }

                        if next_bldg != usize::MAX && next_bldg < allocator.buildings.len() {
                            self.target_building[i] = next_bldg;
                            self.activity[i] = next_act;
                            self.journey_start_time[i] = self.sim_time;
                            let target_node = allocator.buildings[next_bldg].frontage_node;
                            let (_final_target, driving) = self.decide_transit_mode(i, target_node, graph, hpa_graph);
                            self.target_node[i] = target_node; // Simple: always go to frontage
                            self.is_driving[i] = driving;
                            self.current_path[i].clear();
                            self.current_path_index[i] = 0;
                            initiate_journey!(self, i);
                        }
                    }
                }
                TRANSIT_DEPARTING => { // FROM BUILDING TO ROAD
                    let node_idx = self.current_node[i];
                    if node_idx == u32::MAX { self.transit[i] = 2; continue; }
                    
                    // To avoid the "centerline detour", we target the actual lane/sidewalk offset point
                    let target_vec = {
                        let node_pos = graph.nodes[node_idx as usize].pos;
                        let mut base_vec = Vector2::new(node_pos.x, node_pos.z);
                        
                        // If path is empty, find it NOW so we can get the first edge
                        if self.current_path[i].is_empty() {
                            if let Some((_, _, path)) = hpa_graph.find_path(node_idx, self.target_node[i], usize::MAX, graph, !self.is_driving[i]) {
                                let mut final_p = path;
                                if !final_p.is_empty() && final_p[0] == node_idx { final_p.remove(0); }
                                self.current_path[i] = final_p;
                                self.current_path_index[i] = 0;
                            }
                        }

                        if !self.current_path[i].is_empty() {
                            let next_node = self.current_path[i][0];
                            if let Some(found_e) = graph.get_edge_between_nodes(node_idx, next_node) {
                                let edge = &graph.edges[found_e];
                                let is_fwd = edge.start_node == node_idx;
                                let tangent = if is_fwd {
                                    (edge.geometry[1] - edge.geometry[0]).normalized()
                                } else {
                                    (edge.geometry[edge.geometry.len()-1] - edge.geometry[edge.geometry.len()-2]).normalized()
                                };
                                let normal = Vector2::new(-tangent.z, tangent.x);
                                
                                if self.is_driving[i] {
                                     let total_lanes = (edge.fwd_lanes + edge.bkw_lanes) as f32;
                                     let lane_w = edge.width / total_lanes;
                                     let lane_idx = if is_fwd { self.current_lane[i] as f32 } else { (edge.fwd_lanes as i8 + (-self.current_lane[i] - 1)) as f32 };
                                    let offset = (total_lanes * 0.5 - lane_idx - 0.5) * lane_w;
                                    base_vec += normal * offset;
                                } else {
                                    let b_id = self.current_building[i];
                                    if b_id != usize::MAX && b_id < allocator.buildings.len() {
                                        let b = &allocator.buildings[b_id];
                                        let sw_w = crate::config::SIDEWALK_WIDTH;
                                        let offset_amt = edge.width * 0.5 + sw_w * 0.5;
                                        
                                        let b_side = b.side_offset;
                                        self.bezier_t[i] = b_side; // Store side preference
                                        base_vec += normal * (b_side * offset_amt);
                                    }
                                }
                            }
                        }
                        base_vec
                    };

                    let dir = target_vec - Vector2::new(self.pos_x[i], self.pos_y[i]);
                    let dist = dir.length();
                    let speed = if self.is_driving[i] { 10.0 } else { 4.0 };
                    let step = speed * delta;
                    if dist < step {
                        self.pos_x[i] = target_vec.x; self.pos_y[i] = target_vec.y;
                        self.transit[i] = TRANSIT_ON_ROAD; self.current_building[i] = usize::MAX;
                    } else {
                        let mv = dir.normalized() * step;
                        self.pos_x[i] += mv.x; self.pos_y[i] += mv.y;
                    }
                }
                TRANSIT_ON_ROAD | TRANSIT_IMMIGRATING => { // ON ROAD 
                    let mut remaining_dist = (if self.is_driving[i] { 20.0 } else { 4.0 }) * delta;

                    while remaining_dist > 0.0 {
                        // 1. Arrival Check
                        if self.current_node[i] == self.target_node[i] && self.current_edge[i] == usize::MAX {
                            if self.transit[i] == 4 && self.home_building[i] == usize::MAX {
                                if let Some(h) = self.find_available_home(allocator) {
                                    godot_print!("Agent {}: Settled in home {}", i, h);
                                    self.home_building[i] = h;
                                     self.target_building[i] = h;
                                     self.target_node[i] = allocator.buildings[h].frontage_node;
                                     // Set side preference for arriving at new home
                                     let b = &allocator.buildings[h];
                                     self.bezier_t[i] = b.side_offset;
                                     self.current_path[i].clear();
                                    self.current_path_index[i] = 0;
                                    self.transit[i] = TRANSIT_ON_ROAD;
                                } else {
                                    // Wander
                                    if graph.nodes.len() > 1 {
                                        let nt = rng.gen_range(0..graph.nodes.len()) as u32;
                                        if nt != self.current_node[i] {
                                            self.target_node[i] = nt;
                                            self.current_path[i].clear();
                                            self.current_path_index[i] = 0;
                                        }
                                    }
                                }
                            } else {
                                // Arrived at building frontage!
                                // Keep is_driving true for now so we "drive" into the driveway in Transit 3
                                self.transit[i] = TRANSIT_ARRIVING;
                                break;
                            }
                        }

                        // 2. Pathfinding / Select Edge
                        if self.current_edge[i] == usize::MAX {
                            if self.current_path[i].is_empty() {
                                self.pathfind_count += 1;
                                if let Some((_, _, path)) = hpa_graph.find_path(self.current_node[i], self.target_node[i], usize::MAX, graph, !self.is_driving[i]) {
                                    self.current_path[i] = path;
                                    self.current_path_index[i] = 1;
                                } else {
                                    // Agent is utterly stuck (graph disconnected / no path possible)
                                    // Abandon Journey to avoid infinite loop
                                    if self.transit[i] == TRANSIT_IMMIGRATING {
                                        // Wander to random node if immigrants can't reach home
                                        if graph.nodes.len() > 1 {
                                            self.target_node[i] = (self.target_node[i] as usize + 1).rem_euclid(graph.nodes.len()) as u32;
                                        }
                                    } else {
                                        // Give up, teleport home or become idle
                                        if self.home_building[i] != usize::MAX && self.home_building[i] < allocator.buildings.len() {
                                            self.target_building[i] = self.home_building[i];
                                            self.target_node[i] = allocator.buildings[self.home_building[i]].frontage_node;
                                            self.transit[i] = TRANSIT_ARRIVING; // Jump into arriving phase
                                        } else {
                                            self.transit[i] = TRANSIT_IDLE;
                                        }
                                        break;
                                    }
                                }
                            }

                            if self.current_path_index[i] < self.current_path[i].len() {
                                let next_node = self.current_path[i][self.current_path_index[i]];
                                self.current_path_index[i] += 1;

                                if let Some(best_e) = graph.get_edge_between_nodes(self.current_node[i], next_node) {
                                    let edge = &graph.edges[best_e];
                                    let is_fwd = edge.start_node == self.current_node[i];
                                    self.current_edge[i] = best_e;
                                    if is_fwd {
                                        self.edge_progression[i] = 0;
                                        self.current_lane[i] = if edge.fwd_lanes > 0 { rng.gen_range(0..edge.fwd_lanes) as i8 } else { 0 };
                                    } else {
                                        self.edge_progression[i] = edge.physical_geometry.len() as isize - 1;
                                        self.current_lane[i] = if edge.bkw_lanes > 0 { -(rng.gen_range(0..edge.bkw_lanes) as i8) - 1 } else { -1 };
                                    }
                                } else {
                                    self.current_path[i].clear();
                                    break;
                                }
                            } else {
                                break;
                            }
                        }

                        // 3. Move along edge
                        if self.current_edge[i] >= graph.edges.len() || graph.edges[self.current_edge[i]].deleted {
                            self.current_edge[i] = usize::MAX;
                            self.current_path[i].clear();
                            break;
                        }
                        let edge = &graph.edges[self.current_edge[i]];
                        
                        // DEFENSIVE: Edge splits can shrink physical_geometry while agents are on it.
                        let max_valid = edge.physical_geometry.len() as isize - 1;
                        if self.edge_progression[i] > max_valid {
                            self.edge_progression[i] = max_valid;
                        } else if self.edge_progression[i] < 0 {
                            self.edge_progression[i] = 0;
                        }

                        let is_fwd = edge.start_node == self.current_node[i];
                        let target_idx = if is_fwd { self.edge_progression[i] + 1 } else { self.edge_progression[i] - 1 };

                        if target_idx >= 0 && target_idx < edge.physical_geometry.len() as isize {
                            let p_target = edge.physical_geometry[target_idx as usize];
                            
                            // Calculate lane offset (Direction-independent)
                            let p_prev = edge.physical_geometry[self.edge_progression[i] as usize];
                            let diff = if is_fwd {
                                Vector2::new(p_target.x - p_prev.x, p_target.z - p_prev.z)
                            } else {
                                Vector2::new(p_prev.x - p_target.x, p_prev.z - p_target.z)
                            };
                            
                            if diff.length_squared() < 1e-6 {
                                self.edge_progression[i] = target_idx; // Skip zero-length segment
                                remaining_dist -= 0.001; // Tiny progress to avoid infinite loop
                                continue;
                            }
                            let tangent = diff.normalized();
                            let normal = Vector2::new(-tangent.y, tangent.x);
                                                        let mut offset_target = Vector2::new(p_target.x, p_target.z);
                             if self.is_driving[i] {
                                  let total_lanes = (edge.fwd_lanes + edge.bkw_lanes) as f32;
                                  let lane_w = edge.width / total_lanes;
                                  let lane_idx = if is_fwd { self.current_lane[i] as f32 } else { (edge.fwd_lanes as i8 + (-self.current_lane[i] - 1)) as f32 };
                                 let lane_offset = (total_lanes * 0.5 - lane_idx - 0.5) * lane_w;
                                 offset_target += normal * lane_offset;
                             } else {
                                 // Pedestrian: Use side preference (Sidewalk Loyalty)
                                 let side = if self.bezier_t[i].abs() > 0.1 { self.bezier_t[i] } else { 1.0 };
                                 let sw_w = crate::config::SIDEWALK_WIDTH;
                                 let sw_off = (edge.width * 0.5 + sw_w * 0.5) * side;
                                 offset_target += normal * sw_off;
                             }

                            let d = Vector2::new(self.pos_x[i], self.pos_y[i]).distance_to(offset_target);
                            if d < remaining_dist {
                                self.pos_x[i] = offset_target.x; self.pos_y[i] = offset_target.y;
                                self.edge_progression[i] = target_idx;
                                remaining_dist -= d;
                            } else {
                                let diff = offset_target - Vector2::new(self.pos_x[i], self.pos_y[i]);
                                if diff.length_squared() > 1e-8 {
                                    let mv = diff.normalized() * remaining_dist;
                                    self.pos_x[i] += mv.x; self.pos_y[i] += mv.y;
                                }
                                remaining_dist = 0.0;
                            }
                        } else {
                            // Reached Node!
                            self.current_node[i] = if is_fwd { edge.end_node } else { edge.start_node };
                            
                            self.current_edge[i] = usize::MAX;
                        }
                    }
                }
                TRANSIT_ARRIVING => { // FROM ROAD TO BUILDING
                    let b_id = self.target_building[i];
                    if b_id == usize::MAX { self.transit[i] = TRANSIT_IDLE; continue; }
                    let b = &allocator.buildings[b_id];
                    let center_vec = Vector2::new(b.center_x, b.center_y);
                    
                    // DIRECT ENTRY: Move directly from current pos (which is the lane/sidewalk offset)
                    // to the building center. No more gateway detour!
                    let dir_to_center = center_vec - Vector2::new(self.pos_x[i], self.pos_y[i]);
                    let dist = dir_to_center.length();
                    let speed = if self.is_driving[i] { 10.0 } else { 4.0 };
                    let step = speed * delta;
                    
                    if dist < step {
                        let prev_activity = self.activity[i];
                        self.pos_x[i] = center_vec.x; self.pos_y[i] = center_vec.y;
                        self.current_building[i] = b_id;
                        self.is_visible[i] = false;
                        self.transit[i] = TRANSIT_IDLE;
                        self.is_driving[i] = false;
                        self.current_edge[i] = usize::MAX;
                        self.edge_progression[i] = 0;

                        // Apply commute penalty and activity outcomes
                        let commute_time = self.sim_time - self.journey_start_time[i];
                        self.happiness[i] = (self.happiness[i] - commute_time / 60.0).clamp(0.0, 100.0);
                        if prev_activity == 2 { // Returned from Shopping
                            self.money[i] = (self.money[i] - 20.0).max(0.0);
                        }
                    } else if dist > 0.0001 {
                        let mv = dir_to_center.normalized() * step;
                        self.pos_x[i] += mv.x; self.pos_y[i] += mv.y;
                    }
                }
                TRANSIT_INTERSECTION => { 
                     self.transit[i] = TRANSIT_ON_ROAD; 
                }
                _ => { self.transit[i] = TRANSIT_IDLE; }
            }
        }
    }

    /// Update per-day agent state: home/work bonuses and pollution penalties.
    pub fn daily_update(&mut self, pollution: &crate::simulation::grid::pollution::PollutionSystem) {
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
}
