//! Main simulation loop for agents: transit state machine and movement.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::graph::TransitGraph;
use crate::simulation::pathing::hpa::HpaGraph;
use godot::prelude::*;
use rand::Rng;
use super::data::AgentSystem;
use super::{TRANSIT_IDLE, TRANSIT_DEPARTING, TRANSIT_ON_ROAD, TRANSIT_ARRIVING, TRANSIT_IMMIGRATING, TRANSIT_INTERSECTION, MODE_CAR, MODE_WALK};

impl AgentSystem {
    /// Advances the agent simulation by `delta` seconds.
    ///
    /// This is the main simulation entry point for all agents. It handles:
    /// 1. Safety cleanup of volatile building indices.
    /// 2. Activity state transitions (Home -> Work -> Shop).
    /// 3. Pathfinding and movement along road edges.
    /// 4. Arrival and departure logic for buildings.
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
                self.transit[i] = TRANSIT_ARRIVING; // Dump onto street if interior disappears
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
                    let edge = &graph.edges[b.edge_idx];
                    $self.transit[$i] = TRANSIT_DEPARTING; 
                    // Start node for the path is the nearest end of current edge
                    $self.current_node[$i] = if b.frontage_t < 0.5 { edge.start_node } else { edge.end_node };
                    $self.is_visible[$i] = true;
                    // Note: current_building is cleared AFTER reaching frontage in Transit 1
                } else {
                    $self.transit[$i] = TRANSIT_ARRIVING;
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
                                if let Some(h) = allocator.get_random_building_by_zone(ZoneType::Commercial, &mut rng) {
                                    next_bldg = h;
                                    next_act = 2;
                                }
                            } else {
                                // Go to Work
                                if self.work_building[i] == usize::MAX {
                                    if let Some(h) = allocator.get_random_building_by_zones(&[ZoneType::Industrial, ZoneType::Commercial], &mut rng) {
                                        self.work_building[i] = h;
                                    }
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
                            let b = &allocator.buildings[next_bldg];
                            let edge = &graph.edges[b.edge_idx];
                            let target_node = if b.frontage_t < 0.5 { edge.start_node } else { edge.end_node };
                            let (_final_target, mode) = self.decide_transit_mode(i, target_node, graph, hpa_graph);
                            self.target_node[i] = target_node; // Target nearest node for routing
                            self.transit_mode[i] = mode;
                            self.current_path[i].clear();
                            self.current_path_index[i] = 0;
                            initiate_journey!(self, i);
                        }
                    }
                }
                TRANSIT_DEPARTING => { // FROM BUILDING TO ROAD
                    let node_idx = self.current_node[i];
                    if node_idx == u32::MAX { self.transit[i] = TRANSIT_ON_ROAD; continue; }
                    
                    // To avoid the "centerline detour", we target the actual lane/sidewalk offset point
                    let target_vec = {
                        let b_id = self.current_building[i];
                        if b_id == usize::MAX || b_id >= allocator.buildings.len() {
                            Vector2::new(self.pos_x[i], self.pos_y[i])
                        } else {
                            let b = &allocator.buildings[b_id];
                            let edge = &graph.edges[b.edge_idx];
                            let world_pos_on_edge = allocator.get_pos_on_edge(graph, b.edge_idx, b.frontage_t);
                            let tangent = allocator.get_tangent_on_edge(graph, b.edge_idx, b.frontage_t);
                            let normal = Vector2::new(tangent.y, -tangent.x) * (b.side as f32);
                            
                            // Target point on the road/sidewalk
                            let mut base_vec = world_pos_on_edge;
                            if self.transit_mode[i] == MODE_CAR {
                                let total_lanes = (edge.fwd_lanes + edge.bkw_lanes) as f32;
                                let lane_w = edge.width / total_lanes;
                                // For departure, we'll just target the first lane for now
                                let lane_offset = (total_lanes * 0.5 - 0.5) * lane_w;
                                base_vec += normal * (lane_offset);
                            } else {
                                let sw_w = crate::config::SIDEWALK_WIDTH;
                                let offset_amt = edge.width * 0.5 + sw_w * 0.5;
                                base_vec += normal * (offset_amt);
                            }
                            base_vec
                        }
                    };

                    let dir = target_vec - Vector2::new(self.pos_x[i], self.pos_y[i]);
                    let dist = dir.length();
                    let speed = if self.transit_mode[i] == MODE_CAR { 10.0 } else { 4.0 };
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
                    let mut remaining_dist = (if self.transit_mode[i] == MODE_CAR { 20.0 } else { 4.0 }) * delta;

                    while remaining_dist > 0.0 {
                        // 1. Arrival Check
                        let t_bldg_idx = self.target_building[i];
                        if t_bldg_idx != usize::MAX && t_bldg_idx < allocator.buildings.len() {
                            let b = &allocator.buildings[t_bldg_idx];
                            if self.current_edge[i] == b.edge_idx {
                                // We are on the target edge!
                                let edge = &graph.edges[b.edge_idx];
                                let frontage_world_pos = allocator.get_pos_on_edge(graph, b.edge_idx, b.frontage_t);
                                let tangent = allocator.get_tangent_on_edge(graph, b.edge_idx, b.frontage_t);
                                let current_pos = Vector2::new(self.pos_x[i], self.pos_y[i]);
                                let to_agent = current_pos - frontage_world_pos;
                                
                                // Projected distance along the edge longitudinal axis
                                let dist_along = to_agent.dot(tangent).abs();
                                if dist_along < 2.0 {
                                    self.transit[i] = TRANSIT_ARRIVING;
                                    break;
                                }
                            } else if self.current_node[i] == self.target_node[i] && self.current_edge[i] == usize::MAX {
                                // Reached the nearest node, now must enter the target edge
                                let b = &allocator.buildings[t_bldg_idx];
                                self.current_edge[i] = b.edge_idx;
                                let edge = &graph.edges[b.edge_idx];
                                let is_fwd = edge.start_node == self.current_node[i];
                                if is_fwd {
                                    self.edge_progression[i] = 0;
                                    self.current_lane[i] = if edge.fwd_lanes > 0 { rng.gen_range(0..edge.fwd_lanes) as i8 } else { 0 };
                                } else {
                                    self.edge_progression[i] = edge.physical_geometry.len() as isize - 1;
                                    self.current_lane[i] = if edge.bkw_lanes > 0 { -(rng.gen_range(0..edge.bkw_lanes) as i8) - 1 } else { -1 };
                                }
                                // Continue to move along this edge
                            }
                        } else if self.current_node[i] == self.target_node[i] && self.current_edge[i] == usize::MAX {
                            // Immigrating or wandering case (no target building)
                            if self.transit[i] == TRANSIT_IMMIGRATING && self.home_building[i] == usize::MAX {
                                if let Some(h) = self.find_available_home(allocator) {
                                    if cfg!(not(test)) {
                                        godot_print!("Agent {}: Settled in home {}", i, h);
                                    }
                                    self.home_building[i] = h;
                                    self.target_building[i] = h;
                                    let b = &allocator.buildings[h];
                                    let edge = &graph.edges[b.edge_idx];
                                    self.target_node[i] = if b.frontage_t < 0.5 { edge.start_node } else { edge.end_node };
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
                            }
                        }

                        // 2. Pathfinding / Select Edge
                        if self.current_edge[i] == usize::MAX {
                            if self.current_path[i].is_empty() {
                                self.pathfind_count += 1;
                                if let Some((_, _, path)) = hpa_graph.find_path(self.current_node[i], self.target_node[i], usize::MAX, graph, self.transit_mode[i] == MODE_WALK) {
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
                                            let b = &allocator.buildings[self.home_building[i]];
                                            let edge = &graph.edges[b.edge_idx];
                                            self.target_node[i] = if b.frontage_t < 0.5 { edge.start_node } else { edge.end_node };
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
                             if self.transit_mode[i] == MODE_CAR {
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
                    let speed = if self.transit_mode[i] == MODE_CAR { 10.0 } else { 4.0 };
                    let step = speed * delta;
                    
                    if dist < step {
                        let prev_activity = self.activity[i];
                        self.pos_x[i] = center_vec.x; self.pos_y[i] = center_vec.y;
                        self.current_building[i] = b_id;
                        self.is_visible[i] = false;
                        self.transit[i] = TRANSIT_IDLE;
                        self.transit_mode[i] = MODE_WALK;
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
}
