use crate::simulation::grid::data_grid::DataGrid;

use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::TransitGraph;
use crate::simulation::pathing::flow::RoutingTable;
use rand::Rng;

pub struct AgentSystem {
    pub count: usize,
    
    // Core Identity
    pub home_building: Vec<usize>, 
    pub work_building: Vec<usize>, 
    
    // Physics / Rendering
    pub pos_x: Vec<f32>,
    pub pos_y: Vec<f32>,
    pub is_visible: Vec<bool>,
    
    pub activity: Vec<u8>, // 0=Home, 1=Work, 2=Shop
    pub transit: Vec<u8>,  // 0=Inside, 1=ToRoad, 2=OnRoad, 3=ToBldg, 4=Immigrating
    pub happiness: Vec<f32>, 
    pub money: Vec<f32>,
    
    // Routing Geometry
    pub current_building: Vec<usize>,
    pub target_building: Vec<usize>,
    pub current_node: Vec<u32>,
    pub target_node: Vec<u32>,
    
    // Spline Geometry
    pub current_edge: Vec<usize>,
    pub edge_progression: Vec<isize>,
    pub current_lane: Vec<i8>,
    
    // TM:PE Bezier Intersection Pathing
    pub bezier_p0_x: Vec<f32>,
    pub bezier_p0_y: Vec<f32>,
    pub bezier_p1_x: Vec<f32>,
    pub bezier_p1_y: Vec<f32>,
    pub bezier_p2_x: Vec<f32>,
    pub bezier_p2_y: Vec<f32>,
    pub bezier_p3_x: Vec<f32>,
    pub bezier_p3_y: Vec<f32>,
    pub bezier_t: Vec<f32>,
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
            bezier_p0_x: Vec::new(),
            bezier_p0_y: Vec::new(),
            bezier_p1_x: Vec::new(),
            bezier_p1_y: Vec::new(),
            bezier_p2_x: Vec::new(),
            bezier_p2_y: Vec::new(),
            bezier_p3_x: Vec::new(),
            bezier_p3_y: Vec::new(),
            bezier_t: Vec::new(),
        }
    }

    pub fn spawn_agent(&mut self, home: usize, home_node: u32, target_x: f32, target_y: f32, highway_node: u32, init_x: f32, init_y: f32) {
        self.home_building.push(home);
        self.work_building.push(usize::MAX);
        self.pos_x.push(init_x);
        self.pos_y.push(init_y);
        self.is_visible.push(true);
        self.activity.push(0); // Heading Home
        self.transit.push(4); // IMMIGRATING (Driving into the city from the border)
        self.happiness.push(50.0);
        self.money.push(100.0); // Immigrants bring $100
        
        self.current_building.push(usize::MAX);
        self.target_building.push(home);
        self.current_node.push(highway_node);
        self.target_node.push(home_node);
        self.current_edge.push(usize::MAX);
        self.edge_progression.push(0);
        self.current_lane.push(0);
        self.bezier_p0_x.push(0.0);
        self.bezier_p0_y.push(0.0);
        self.bezier_p1_x.push(0.0);
        self.bezier_p1_y.push(0.0);
        self.bezier_p2_x.push(0.0);
        self.bezier_p2_y.push(0.0);
        self.bezier_p3_x.push(0.0);
        self.bezier_p3_y.push(0.0);
        self.bezier_t.push(0.0);
        self.count += 1;
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
        self.bezier_p0_x.swap(index, last_idx);
        self.bezier_p0_y.swap(index, last_idx);
        self.bezier_p1_x.swap(index, last_idx);
        self.bezier_p1_y.swap(index, last_idx);
        self.bezier_p2_x.swap(index, last_idx);
        self.bezier_p2_y.swap(index, last_idx);
        self.bezier_p3_x.swap(index, last_idx);
        self.bezier_p3_y.swap(index, last_idx);
        self.bezier_t.swap(index, last_idx);

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
        self.bezier_p0_x.pop();
        self.bezier_p0_y.pop();
        self.bezier_p1_x.pop();
        self.bezier_p1_y.pop();
        self.bezier_p2_x.pop();
        self.bezier_p2_y.pop();
        self.bezier_p3_x.pop();
        self.bezier_p3_y.pop();
        self.bezier_t.pop();

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
    
    pub fn tick(&mut self, allocator: &BuildingAllocator, routing_table: &RoutingTable, graph: &TransitGraph, delta: f32) {
        let mut rng = rand::thread_rng();
        
        let w = allocator.occupancy_grid.width as f32;
        let h = allocator.occupancy_grid.height as f32;
        
        let get_bldg_pos = |b_id: usize| -> godot::prelude::Vector2 {
            let b = &allocator.buildings[b_id];
            let bx = b.x as f32 - (w - 1.0) * 0.5;
            let bz = b.y as f32 - (h - 1.0) * 0.5;
            godot::prelude::Vector2::new(bx, bz)
        };

        // Massive Data-Oriented Swarm Iteration
        for i in 0..self.count {
            match self.transit[i] {
                0 => { // INSIDE BUILDING
                    if self.activity[i] == 0 { // At Home
                        if rng.gen_bool((0.05 * delta) as f64) {
                            // Matchmaker: Find Work
                            if self.work_building[i] == usize::MAX {
                                let mut jobs = Vec::new();
                                for (b_id, b) in allocator.buildings.iter().enumerate() {
                                    if b.zone_type == ZoneType::Industrial || b.zone_type == ZoneType::Commercial {
                                        jobs.push(b_id);
                                    }
                                }
                                if !jobs.is_empty() {
                                    self.work_building[i] = jobs[rng.gen_range(0..jobs.len())];
                                } else {
                                    self.happiness[i] = (self.happiness[i] - 1.0 * delta).max(0.0);
                                }
                            }

                            if self.work_building[i] != usize::MAX {
                                self.target_building[i] = self.work_building[i];
                                self.target_node[i] = allocator.buildings[self.work_building[i]].road_node;
                                self.activity[i] = 1; // Heading to Work
                                self.transit[i] = 1; // To Road
                                self.is_visible[i] = true;
                                let p = get_bldg_pos(self.current_building[i]);
                                self.pos_x[i] = p.x;
                                self.pos_y[i] = p.y;
                            }
                        } else if self.money[i] >= 20.0 && rng.gen_bool((0.08 * delta) as f64) {
                            // Find a random Shop! (Go shopping more often than working!)
                            let mut shops = Vec::new();
                            for (b_id, b) in allocator.buildings.iter().enumerate() {
                                if b.zone_type == ZoneType::Commercial { shops.push(b_id); }
                            }
                            if !shops.is_empty() {
                                self.money[i] -= 20.0; // Spend money purely to go shopping!
                                let shop_id = shops[rng.gen_range(0..shops.len())];
                                self.target_building[i] = shop_id;
                                self.target_node[i] = allocator.buildings[shop_id].road_node;
                                self.activity[i] = 2; // Heading to Shop
                                self.transit[i] = 1; // To Road
                                self.is_visible[i] = true;
                                let p = get_bldg_pos(self.current_building[i]);
                                self.pos_x[i] = p.x;
                                self.pos_y[i] = p.y;
                            }
                        }
                    } else if self.activity[i] == 1 { // At Work
                        if rng.gen_bool((0.08 * delta) as f64) { // Clock out
                            self.money[i] += 40.0; // Earn wages (Balanced against shopping)
                            // 50% chance to go shopping after work IF wealth allows
                            if self.money[i] >= 20.0 && rng.gen_bool(0.5) {
                                let mut shops = Vec::new();
                                for (b_id, b) in allocator.buildings.iter().enumerate() {
                                    if b.zone_type == ZoneType::Commercial { shops.push(b_id); }
                                }
                                if !shops.is_empty() {
                                    self.money[i] -= 20.0; // Spend money
                                    let shop_id = shops[rng.gen_range(0..shops.len())];
                                    self.target_building[i] = shop_id;
                                    self.target_node[i] = allocator.buildings[shop_id].road_node;
                                    self.activity[i] = 2; // Heading to Shop
                                    self.transit[i] = 1; // To Road
                                    self.is_visible[i] = true;
                                    let p = get_bldg_pos(self.current_building[i]);
                                    self.pos_x[i] = p.x;
                                    self.pos_y[i] = p.y;
                                    continue; // Skip the default go-home behavior below!
                                }
                            }
                            
                            // Default: Head Home
                            if self.home_building[i] != usize::MAX {
                                self.target_building[i] = self.home_building[i];
                                self.target_node[i] = allocator.buildings[self.home_building[i]].road_node;
                                self.activity[i] = 0; // Heading To Home
                            } else {
                                // HOMELESS! Head to the streets and become Stranded!
                                self.target_building[i] = usize::MAX;
                                self.activity[i] = 0; 
                            }
                            self.transit[i] = 1; // To Road
                            self.is_visible[i] = true;
                            let p = get_bldg_pos(self.current_building[i]);
                            self.pos_x[i] = p.x;
                            self.pos_y[i] = p.y;
                        }
                    } else if self.activity[i] == 2 { // At Shop
                        if rng.gen_bool((0.15 * delta) as f64) { // Done shopping (faster than working)
                            // 30% chance to go to another store if they have enough money!
                            if self.money[i] >= 20.0 && rng.gen_bool(0.3) {
                                let mut shops = Vec::new();
                                for (b_id, b) in allocator.buildings.iter().enumerate() {
                                    if b.zone_type == ZoneType::Commercial { shops.push(b_id); }
                                }
                                if !shops.is_empty() {
                                    self.money[i] -= 20.0; // Spend money
                                    let shop_id = shops[rng.gen_range(0..shops.len())];
                                    self.target_building[i] = shop_id;
                                    self.target_node[i] = allocator.buildings[shop_id].road_node;
                                    self.activity[i] = 2; // Heading to another Shop
                                    self.transit[i] = 1; // To Road
                                    self.is_visible[i] = true;
                                    let p = get_bldg_pos(self.current_building[i]);
                                    self.pos_x[i] = p.x;
                                    self.pos_y[i] = p.y;
                                    continue; // Skip going home!
                                }
                            }
                            
                            // Default: Head Home
                            if self.home_building[i] != usize::MAX {
                                self.target_building[i] = self.home_building[i];
                                self.target_node[i] = allocator.buildings[self.home_building[i]].road_node;
                                self.activity[i] = 0; // Heading To Home
                            } else {
                                // HOMELESS! Head to the streets and become Stranded!
                                self.target_building[i] = usize::MAX;
                                self.activity[i] = 0; 
                            }
                            self.transit[i] = 1; // To Road
                            self.is_visible[i] = true;
                            let p = get_bldg_pos(self.current_building[i]);
                            self.pos_x[i] = p.x;
                            self.pos_y[i] = p.y;
                        }
                    }
                }
                1 => { // WALKING TO ROAD
                    let tgt_node_id = if self.current_building[i] != usize::MAX {
                        allocator.buildings[self.current_building[i]].road_node
                    } else {
                        self.current_node[i] // Used by stranded agents navigating street geometry directly!
                    };
                    
                    let target_pos = graph.nodes[tgt_node_id as usize].pos;
                    let target_vec = godot::prelude::Vector2::new(target_pos.x, target_pos.z);
                    let current_vec = godot::prelude::Vector2::new(self.pos_x[i], self.pos_y[i]);
                    
                    let dir = target_vec - current_vec;
                    let dist = dir.length();
                    let step_dist = 4.0 * delta; // Slower walking
                    
                    if dist < step_dist {
                        self.pos_x[i] = target_vec.x;
                        self.pos_y[i] = target_vec.y;
                        self.current_node[i] = tgt_node_id;
                        self.transit[i] = 2; // Step onto the road network!
                    } else {
                        let step = dir.normalized() * step_dist;
                        self.pos_x[i] += step.x;
                        self.pos_y[i] += step.y;
                    }
                }
                2 | 4 => { // ON ROAD OR IMMIGRATING
                    if self.current_node[i] != self.target_node[i] {
                        if self.current_edge[i] == usize::MAX {
                            if let Some(next) = routing_table.get_next_node(self.current_node[i], self.target_node[i]) {
                                let mut found = false;
                                for (idx, edge) in graph.edges.iter().enumerate() {
                                    if edge.start_node == self.current_node[i] && edge.end_node == next {
                                        self.current_edge[i] = idx;
                                        self.edge_progression[i] = 0; // Forward
                                        let mut r = 0;
                                        if edge.fwd_lanes > 0 { r = rng.gen_range(0..edge.fwd_lanes); }
                                        self.current_lane[i] = r as i8;
                                        found = true;
                                        break;
                                    } else if edge.end_node == self.current_node[i] && edge.start_node == next {
                                        self.current_edge[i] = idx;
                                        self.edge_progression[i] = edge.physical_geometry.len() as isize - 1; // Backward
                                        let mut r = 0;
                                        if edge.bkw_lanes > 0 { r = rng.gen_range(0..edge.bkw_lanes); }
                                        self.current_lane[i] = -(r as i8) - 1;
                                        found = true;
                                        break;
                                    }
                                }
                                if !found {
                                    // Route broken, teleport home and sleep
                                    self.transit[i] = 0;
                                    self.activity[i] = 0;
                                    self.is_visible[i] = false;
                                    self.current_building[i] = self.home_building[i];
                                }
                            } else {
                                // Route broken, teleport home and sleep
                                self.transit[i] = 0;
                                self.activity[i] = 0;
                                self.is_visible[i] = false;
                                self.current_building[i] = self.home_building[i];
                            }
                        }
                        
                        if self.current_edge[i] != usize::MAX {
                            let curr_edge = &graph.edges[self.current_edge[i]];
                            let mut remaining_dist = 12.0 * delta; // Constant movement speed
                            
                            while remaining_dist > 0.0 && self.current_edge[i] != usize::MAX {
                                let target_idx = if curr_edge.start_node == self.current_node[i] {
                                    self.edge_progression[i] + 1
                                } else {
                                    self.edge_progression[i] - 1
                                };
                                
                                if target_idx >= 0 && target_idx < curr_edge.physical_geometry.len() as isize {
                                    let target_pos = curr_edge.physical_geometry[target_idx as usize];
                                    
                                    // Tangent/Normal Offset Math
                                    let mut t_idx_1 = target_idx as usize;
                                    let mut t_idx_2 = target_idx as usize + 1;
                                    if t_idx_2 >= curr_edge.physical_geometry.len() {
                                        t_idx_1 = target_idx as usize - 1;
                                        t_idx_2 = target_idx as usize;
                                    }
                                    let p1 = curr_edge.physical_geometry[t_idx_1];
                                    let p2 = curr_edge.physical_geometry[t_idx_2];
                                    let tangent = godot::prelude::Vector2::new(p2.x - p1.x, p2.z - p1.z).normalized();
                                    let normal = godot::prelude::Vector2::new(-tangent.y, tangent.x); // Right Hand Normal
                                    
                                    let lane_raw = self.current_lane[i];
                                    let lane_offset = if lane_raw >= 0 {
                                        (lane_raw as f32 + 0.5) * 3.0 // Forward (Right)
                                    } else {
                                        (lane_raw as f32 + 0.5) * 3.0 // Backward (Left)
                                    };
                                    
                                    let mut target_vec = godot::prelude::Vector2::new(target_pos.x, target_pos.z);
                                    target_vec += normal * lane_offset;
                                    
                                    let current_vec = godot::prelude::Vector2::new(self.pos_x[i], self.pos_y[i]);
                                    
                                    let dir = target_vec - current_vec;
                                    let dist = dir.length();
                                    
                                    if dist < remaining_dist {
                                        // Push past this sub-point and continue consuming velocity
                                        self.pos_x[i] = target_vec.x;
                                        self.pos_y[i] = target_vec.y;
                                        self.edge_progression[i] = target_idx;
                                        remaining_dist -= dist;
                                    } else {
                                        // Interpolate geometrically towards it
                                        let step = dir.normalized() * remaining_dist;
                                        self.pos_x[i] += step.x;
                                        self.pos_y[i] += step.y;
                                        remaining_dist = 0.0;
                                    }
                                } else {
                                    // Arrived at the next real Road Node!
                                    let next_node = if curr_edge.start_node == self.current_node[i] { curr_edge.end_node } else { curr_edge.start_node };
                                    
                                    if next_node == self.target_node[i] {
                                        self.current_node[i] = next_node;
                                        self.current_edge[i] = usize::MAX; // Stop rendering on edge
                                        self.transit[i] = 3; // Disembark!
                                        remaining_dist = 0.0;
                                    } else {
                                        // Entering a TM:PE Intersection! Calculate Dynamic Vector Spline!
                                        if let Some(future_node) = routing_table.get_next_node(next_node, self.target_node[i]) {
                                            
                                            // Find incoming edge link
                                            let mut next_edge_idx = usize::MAX;
                                            let mut next_fwd = true;
                                            for (idx, e) in graph.edges.iter().enumerate() {
                                                if e.start_node == next_node && e.end_node == future_node {
                                                    next_edge_idx = idx; next_fwd = true; break;
                                                } else if e.end_node == next_node && e.start_node == future_node {
                                                    next_edge_idx = idx; next_fwd = false; break;
                                                }
                                            }
                                            
                                            if next_edge_idx != usize::MAX {
                                                let next_edge = &graph.edges[next_edge_idx];
                                                
                                                // 1. Evaluate Explicit User Lane Connections
                                                let mut target_lane = 0i8;
                                                let node_ref = &graph.nodes[next_node as usize];
                                                let tcon_key = (self.current_edge[i], self.current_lane[i]);
                                                if let Some(conns) = node_ref.lane_connections.get(&tcon_key) {
                                                    let mut valids = Vec::new();
                                                    for c in conns { if c.0 == next_edge_idx { valids.push(c.1); } }
                                                    if valids.len() > 0 {
                                                        target_lane = valids[rng.gen_range(0..valids.len())];
                                                    } else {
                                                        if next_fwd && next_edge.fwd_lanes > 0 { target_lane = rng.gen_range(0..next_edge.fwd_lanes) as i8; }
                                                        if !next_fwd && next_edge.bkw_lanes > 0 { target_lane = -(rng.gen_range(0..next_edge.bkw_lanes) as i32) as i8 - 1; }
                                                    }
                                                } else {
                                                    if next_fwd && next_edge.fwd_lanes > 0 { target_lane = rng.gen_range(0..next_edge.fwd_lanes) as i8; }
                                                    if !next_fwd && next_edge.bkw_lanes > 0 { target_lane = -(rng.gen_range(0..next_edge.bkw_lanes) as i32) as i8 - 1; }
                                                }
                                                
                                                // 2. Extrapolate Micro-Spline coordinates
                                                self.bezier_p0_x[i] = self.pos_x[i];
                                                self.bezier_p0_y[i] = self.pos_y[i];
                                                
                                                let exit_tangent = {
                                                    let l = curr_edge.physical_geometry.len();
                                                    if curr_edge.end_node == next_node {
                                                        godot::prelude::Vector2::new(curr_edge.physical_geometry[l-1].x - curr_edge.physical_geometry[l-2].x, curr_edge.physical_geometry[l-1].z - curr_edge.physical_geometry[l-2].z).normalized()
                                                    } else {
                                                        godot::prelude::Vector2::new(curr_edge.physical_geometry[0].x - curr_edge.physical_geometry[1].x, curr_edge.physical_geometry[0].z - curr_edge.physical_geometry[1].z).normalized()
                                                    }
                                                };
                                                
                                                let h_len = 8.0; // Bezier Control Vector Magnitude
                                                self.bezier_p1_x[i] = self.pos_x[i] + exit_tangent.x * h_len;
                                                self.bezier_p1_y[i] = self.pos_y[i] + exit_tangent.y * h_len;
                                                
                                                let e_geom = if next_fwd { next_edge.physical_geometry[0] } else { next_edge.physical_geometry[next_edge.physical_geometry.len() - 1] };
                                                let entry_tangent = if next_fwd {
                                                    godot::prelude::Vector2::new(next_edge.physical_geometry[1].x - next_edge.physical_geometry[0].x, next_edge.physical_geometry[1].z - next_edge.physical_geometry[0].z).normalized()
                                                } else {
                                                    let l = next_edge.physical_geometry.len();
                                                    godot::prelude::Vector2::new(next_edge.physical_geometry[l-2].x - next_edge.physical_geometry[l-1].x, next_edge.physical_geometry[l-2].z - next_edge.physical_geometry[l-1].z).normalized()
                                                };
                                                
                                                let entry_normal = godot::prelude::Vector2::new(-entry_tangent.y, entry_tangent.x);
                                                let e_off = (target_lane as f32 + 0.5) * 3.0; // Works universally for fwd >=0, bkw <0
                                                
                                                let p3_x = e_geom.x + entry_normal.x * e_off;
                                                let p3_y = e_geom.z + entry_normal.y * e_off;
                                                self.bezier_p3_x[i] = p3_x;
                                                self.bezier_p3_y[i] = p3_y;
                                                self.bezier_p2_x[i] = p3_x - entry_tangent.x * h_len;
                                                self.bezier_p2_y[i] = p3_y - entry_tangent.y * h_len;
                                                
                                                // 3. Initiate Spline Driving
                                                self.bezier_t[i] = 0.0;
                                                self.transit[i] = 5; // IN INTERSECTION
                                                
                                                // Lock-in next edge state proactively
                                                self.current_node[i] = next_node;
                                                self.current_edge[i] = next_edge_idx;
                                                self.current_lane[i] = target_lane;
                                                self.edge_progression[i] = if next_fwd { 0 } else { next_edge.physical_geometry.len() as isize - 1 };
                                                
                                            } else {
                                                self.transit[i] = 0; self.is_visible[i] = false; self.current_building[i] = self.home_building[i];
                                            }
                                        } else {
                                            self.transit[i] = 0; self.is_visible[i] = false; self.current_building[i] = self.home_building[i];
                                        }
                                        remaining_dist = 0.0;
                                    }
                                }
                            }
                        }
                    } else {
                        self.transit[i] = 3; // Already at road node
                    }
                }
                3 => { // WALKING TO target_building OR STRANDED
                    if self.target_building[i] == usize::MAX {
                        // HOMELESS & STRANDED! Stand in the street and look for a new house!
                        let mut homes = Vec::new();
                        for (b_id, b) in allocator.buildings.iter().enumerate() {
                            if b.zone_type == ZoneType::Residential {
                                homes.push(b_id);
                            }
                        }
                        if !homes.is_empty() {
                            let new_home = homes[rng.gen_range(0..homes.len())];
                            self.home_building[i] = new_home;
                            self.target_building[i] = new_home;
                            self.target_node[i] = allocator.buildings[new_home].road_node;
                            self.activity[i] = 0; // Going to new home
                            
                            // Discover nearest physical road node from their stranded position to begin routing
                            let mut best_id = 0;
                            let mut min_d = 200.0;
                            for (n_i, n) in graph.nodes.iter().enumerate() {
                                let dx = n.pos.x - self.pos_x[i];
                                let dz = n.pos.z - self.pos_y[i];
                                let d = (dx*dx + dz*dz).sqrt();
                                if d < min_d {
                                    min_d = d;
                                    best_id = n_i as u32;
                                }
                            }
                            self.current_node[i] = best_id;
                            self.transit[i] = 1; // Walk to the discovered road node
                        } else {
                            // Angry homeless citizens standing in the street!
                            self.happiness[i] = (self.happiness[i] - 10.0 * delta).max(0.0);
                        }
                        continue;
                    }
                    
                    let target_vec = get_bldg_pos(self.target_building[i]);
                    let current_vec = godot::prelude::Vector2::new(self.pos_x[i], self.pos_y[i]);
                    
                    let dir = target_vec - current_vec;
                    let dist = dir.length();
                    let step_dist = 4.0 * delta; // Slower walking
                    
                    if dist < step_dist {
                        // Arrived physically!
                        self.pos_x[i] = target_vec.x;
                        self.pos_y[i] = target_vec.y;
                        self.current_building[i] = self.target_building[i];
                        self.is_visible[i] = false; // Walked indoors!
                        self.transit[i] = 0; // Inside
                    } else {
                        let step = dir.normalized() * step_dist;
                        self.pos_x[i] += step.x;
                        self.pos_y[i] += step.y;
                    }
                }
                5 => { // INTERSECTION BEZIER
                    let t = self.bezier_t[i];
                    if t >= 1.0 { // Finished driving through the intersection void!
                        self.transit[i] = 2; // Arrived on the other side, ON ROAD
                        self.pos_x[i] = self.bezier_p3_x[i];
                        self.pos_y[i] = self.bezier_p3_y[i];
                    } else {
                        // Advance bezier_t based on roughly 12m/s
                        let p0_vec = godot::prelude::Vector2::new(self.bezier_p0_x[i], self.bezier_p0_y[i]);
                        let p3_vec = godot::prelude::Vector2::new(self.bezier_p3_x[i], self.bezier_p3_y[i]);
                        let approx_len = (p3_vec - p0_vec).length().max(1.0);
                        
                        let delta_t = (12.0 * delta) / approx_len;
                        self.bezier_t[i] = f32::min(1.0, self.bezier_t[i] + delta_t);
                        
                        let t = self.bezier_t[i]; // re-read
                        let nt = 1.0 - t;
                        let nt2 = nt * nt;
                        let nt3 = nt2 * nt;
                        let t2 = t * t;
                        let t3 = t2 * t;
                        
                        self.pos_x[i] = nt3 * self.bezier_p0_x[i] + 3.0 * nt2 * t * self.bezier_p1_x[i] + 3.0 * nt * t2 * self.bezier_p2_x[i] + t3 * self.bezier_p3_x[i];
                        self.pos_y[i] = nt3 * self.bezier_p0_y[i] + 3.0 * nt2 * t * self.bezier_p1_y[i] + 3.0 * nt * t2 * self.bezier_p2_y[i] + t3 * self.bezier_p3_y[i];
                    }
                }
                _ => {}
            }
        }
    }
}
