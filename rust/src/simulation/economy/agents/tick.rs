//! Main simulation loop for agents: transit state machine and movement.

use super::data::AgentSystem;
use super::{
    MODE_CAR, MODE_WALK, TRANSIT_ARRIVING, TRANSIT_DEPARTING, TRANSIT_IDLE, TRANSIT_IMMIGRATING,
    TRANSIT_INTERSECTION, TRANSIT_ON_ROAD,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::network::TransitNetwork;
use godot::prelude::*;
use rand::Rng;

impl AgentSystem {
    /// Advances the agent simulation by `delta` seconds.
    pub fn tick(
        &mut self,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &mut RegionGraph,
        delta: f32,
    ) {
        self.sim_time += delta;
        let mut rng = rand::rngs::ThreadRng::default();

        // 1. Safety Scrub
        for i in 0..self.count {
            if self.home_building[i] != usize::MAX
                && self.home_building[i] >= allocator.buildings.len()
            {
                self.home_building[i] = usize::MAX;
            }
            if self.work_building[i] != usize::MAX
                && self.work_building[i] >= allocator.buildings.len()
            {
                self.work_building[i] = usize::MAX;
            }
            if self.current_building[i] != usize::MAX
                && self.current_building[i] >= allocator.buildings.len()
            {
                self.current_building[i] = usize::MAX;
                self.transit[i] = TRANSIT_ARRIVING;
                self.is_visible[i] = true;
            }
            if self.target_building[i] != usize::MAX
                && self.target_building[i] >= allocator.buildings.len()
            {
                if self.home_building[i] != usize::MAX {
                    self.target_building[i] = self.home_building[i];
                } else {
                    self.target_building[i] = usize::MAX;
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
                    $self.transit[$i] = TRANSIT_DEPARTING;
                    let n_start = edge.start_node;
                    let n_end = edge.end_node;
                    
                    // Pick the node that is closer to the final destination, 
                    // but respect one-way constraints for cars.
                    let can_go_fwd = $self.transit_mode[$i] != MODE_CAR || edge.fwd_lanes > 0;
                    let can_go_bkw = $self.transit_mode[$i] != MODE_CAR || edge.bkw_lanes > 0;

                    let target_pos = graph.nodes[$self.target_node[$i] as usize].pos;
                    let d_start = graph.nodes[n_start as usize].pos.distance_to(target_pos);
                    let d_end = graph.nodes[n_end as usize].pos.distance_to(target_pos);
                    
                    if can_go_fwd && can_go_bkw {
                        $self.current_node[$i] = if d_start < d_end { n_start } else { n_end };
                    } else if can_go_fwd {
                        $self.current_node[$i] = n_end;
                    } else {
                        $self.current_node[$i] = n_start;
                    }
                    $self.is_visible[$i] = true;
                } else {
                    $self.transit[$i] = TRANSIT_ARRIVING;
                }
            };
        }

        // Swarm Iteration
        for i in 0..self.count {
            self.current_node[i] = graph.get_valid_node(self.current_node[i]);
            self.target_node[i] = graph.get_valid_node(self.target_node[i]);

            match self.transit[i] {
                TRANSIT_IDLE => {
                    // INSIDE BUILDING
                    if rng.gen_bool((0.05 * delta) as f64) {
                        let mut next_act = self.activity[i];
                        let mut next_bldg = usize::MAX;

                        if self.activity[i] == 0 {
                            if self.money[i] >= 20.0 && rng.gen_bool(0.4) {
                                if let Some(h) = allocator
                                    .get_random_building_by_zone(ZoneType::Commercial, &mut rng)
                                {
                                    next_bldg = h;
                                    next_act = 2;
                                }
                            } else {
                                if self.work_building[i] == usize::MAX {
                                    if let Some(h) = allocator.get_random_building_by_zones(
                                        &[ZoneType::Industrial, ZoneType::Commercial],
                                        &mut rng,
                                    ) {
                                        self.work_building[i] = h;
                                    }
                                }
                                if self.work_building[i] != usize::MAX {
                                    next_bldg = self.work_building[i];
                                    next_act = 1;
                                }
                            }
                        } else {
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
                            let target_edge = &graph.edges[b.edge_idx];
                            let target_node = if b.frontage_t < 0.5 {
                                target_edge.start_node
                            } else {
                                target_edge.end_node
                            };
                            let (_final_target, mode) = self.decide_transit_mode(
                                i,
                                target_node,
                                graph,
                                &transit_network.cch_graph,
                            );
                            self.target_node[i] = target_node;
                            self.transit_mode[i] = mode;
                            self.current_path[i].clear();
                            self.current_path_index[i] = 0;
                            self.current_lane_id[i] = usize::MAX;
                            self.lane_distance[i] = 0.0;
                            initiate_journey!(self, i);
                        }
                    }
                }
                TRANSIT_DEPARTING => {
                    let node_idx = self.current_node[i];
                    if node_idx == u32::MAX || node_idx as usize >= graph.nodes.len() {
                        self.transit[i] = TRANSIT_ON_ROAD;
                        continue;
                    }

                    let b_id = self.current_building[i];
                    if b_id == usize::MAX || b_id >= allocator.buildings.len() {
                        self.transit[i] = TRANSIT_ON_ROAD;
                        continue;
                    }

                    if self.current_lane_id[i] == usize::MAX {
                        let b = &allocator.buildings[b_id];
                        let edge = &graph.edges[b.edge_idx];
                        let is_fwd = node_idx == edge.end_node;
                        
                        if let Some(edge_lanes) = transit_network.lane_system.edge_lanes.get(&b.edge_idx) {
                            let mut valid_lanes = Vec::new();
                            for &l_id in edge_lanes {
                                let lane = &transit_network.lane_system.lanes[l_id];
                                if lane.is_fwd == is_fwd {
                                    if self.transit_mode[i] == MODE_WALK {
                                        let target_idx = b.side as i8 * 100;
                                        if lane.lane_type == crate::simulation::network::lanes::LaneType::Foot && (lane.lane_idx == target_idx || lane.lane_idx == 0) {
                                            valid_lanes.push(l_id);
                                        }
                                    } else if lane.lane_type == crate::simulation::network::lanes::LaneType::Vehicle {
                                        valid_lanes.push(l_id);
                                    }
                                }
                            }
                            if !valid_lanes.is_empty() {
                                // For cars, pick the outermost lane (highest absolute index) to avoid crossing traffic immediately
                                let chosen = if self.transit_mode[i] == MODE_CAR {
                                    valid_lanes.iter().max_by_key(|&&id| {
                                        transit_network.lane_system.lanes[id].lane_idx.abs()
                                    }).copied().unwrap()
                                } else {
                                    valid_lanes[rng.gen_range(0..valid_lanes.len())]
                                };
                                self.current_lane_id[i] = chosen;
                                let lane = &transit_network.lane_system.lanes[chosen];
                                self.lane_distance[i] = if is_fwd { b.frontage_t * lane.length } else { (1.0 - b.frontage_t) * lane.length };
                            } else {
                                self.transit[i] = TRANSIT_ON_ROAD;
                                continue;
                            }
                        } else {
                            self.transit[i] = TRANSIT_ON_ROAD;
                            continue;
                        }
                    }

                    let target_vec = {
                        let mut out_pos = graph.nodes[node_idx as usize].pos;
                        if self.current_lane_id[i] != usize::MAX && self.current_lane_id[i] < transit_network.lane_system.lanes.len() {
                            let lane = &transit_network.lane_system.lanes[self.current_lane_id[i]];
                            let dist = self.lane_distance[i];
                            let mut curr = 0.0;
                            for j in 0..lane.geometry.len() - 1 {
                                let p0 = lane.geometry[j];
                                let p1 = lane.geometry[j+1];
                                let d = p0.distance_to(p1);
                                if curr + d >= dist || j == lane.geometry.len() - 2 {
                                    let t = if d > 1e-5 { (dist - curr) / d } else { 0.0 };
                                    out_pos = p0.lerp(p1, t.clamp(0.0, 1.0));
                                    break;
                                }
                                curr += d;
                            }
                        }
                        Vector2::new(out_pos.x, out_pos.z)
                    };

                    let dir = target_vec - Vector2::new(self.pos_x[i], self.pos_y[i]);
                    let dist = dir.length();
                    let speed = if self.transit_mode[i] == MODE_CAR {
                        10.0
                    } else {
                        4.0
                    };
                    let step = speed * delta;
                    if dist < step {
                        self.pos_x[i] = target_vec.x;
                        self.pos_y[i] = target_vec.y;
                        self.transit[i] = TRANSIT_ON_ROAD;
                        self.current_edge[i] = allocator.buildings[self.current_building[i]].edge_idx;
                        self.current_building[i] = usize::MAX;
                        
                        // We must initialize the pathing fully so the loop traverses it 
                        if self.transit_mode[i] == MODE_CAR && self.current_path[i].is_empty() {
                            self.pathfind_count += 1;
                            if let Some((_, _, path)) = transit_network.cch_graph.find_path(
                                self.current_node[i],
                                self.target_node[i],
                                usize::MAX,
                                graph,
                                TransitFlags::CAR,
                            ) {
                                self.current_path[i] = path;
                                self.current_path_index[i] = 1; // start traversing towards node 1
                            } else {
                                self.transit[i] = TRANSIT_IDLE;
                            }
                        }
                        
                    } else {
                        let mv = dir.normalized() * step;
                        self.pos_x[i] += mv.x;
                        self.pos_y[i] += mv.y;
                    }
                }
                TRANSIT_ON_ROAD | TRANSIT_IMMIGRATING | TRANSIT_INTERSECTION => {
                    let speed = if self.transit_mode[i] == MODE_CAR {
                        if self.transit[i] == TRANSIT_INTERSECTION {
                            10.0
                        } else {
                            20.0
                        }
                    } else {
                        4.0
                    };
                    let mut remaining_dist = speed * delta;

                    // === UNIFIED LANE GRAPH LOGIC ===
                    while remaining_dist > 0.0 {
                        
                        // 1. Initialise Path if missing
                        if self.current_path[i].is_empty() {
                            self.pathfind_count += 1;
                            let mut path_found = false;
                            
                            let search_flags = if self.transit_mode[i] == MODE_WALK {
                                TransitFlags::FOOT
                            } else {
                                TransitFlags::CAR
                            };

                            if let Some((_, _, path)) = transit_network.cch_graph.find_path(
                                self.current_node[i],
                                self.target_node[i],
                                usize::MAX,
                                graph,
                                search_flags,
                            ) {
                                if path.len() > 1 {
                                    self.current_path[i] = path;
                                    self.current_path_index[i] = 1; // [0] = current_node, [1] = next_node
                                    self.current_lane_id[i] = usize::MAX;
                                    path_found = true;
                                }
                            }

                            if !path_found {
                                // Agent is stuck or already at destination
                                if self.home_building[i] != usize::MAX && self.home_building[i] < allocator.buildings.len() {
                                    self.target_building[i] = self.home_building[i];
                                    self.transit[i] = TRANSIT_ARRIVING;
                                    self.transit_mode[i] = MODE_WALK;
                                } else {
                                    self.transit[i] = TRANSIT_IDLE;
                                }
                                break;
                            }
                        }

                        // 2. Initialise Lane if turning on-network
                        if self.current_lane_id[i] == usize::MAX {
                            if self.current_path_index[i] < self.current_path[i].len() {
                                let next_node = self.current_path[i][self.current_path_index[i]];
                                if let Some(best_e) = graph.get_edge_between_nodes(self.current_node[i], next_node) {
                                    let edge = &graph.edges[best_e];
                                    let is_fwd = edge.start_node == self.current_node[i];
                                    
                                    // Choose a lane
                                    if let Some(edge_lanes) = transit_network.lane_system.edge_lanes.get(&best_e) {
                                        let mut valid_lanes = Vec::new();
                                        for &l_id in edge_lanes {
                                            let lane = &transit_network.lane_system.lanes[l_id];
                                            if lane.is_fwd == is_fwd {
                                                if self.transit_mode[i] == MODE_WALK {
                                                    if lane.lane_type == crate::simulation::network::lanes::LaneType::Foot {
                                                        valid_lanes.push(l_id);
                                                    }
                                                } else if lane.lane_type == crate::simulation::network::lanes::LaneType::Vehicle {
                                                    valid_lanes.push(l_id);
                                                }
                                            }
                                        }
                                        
                                        if !valid_lanes.is_empty() {
                                            let chosen = valid_lanes[rng.gen_range(0..valid_lanes.len())];
                                            self.current_lane_id[i] = chosen;
                                            self.lane_distance[i] = 0.0;
                                            self.current_edge[i] = best_e;
                                            self.transit[i] = TRANSIT_ON_ROAD;
                                        } else {
                                            self.current_path[i].clear();
                                            break;
                                        }
                                    } else {
                                        self.current_path[i].clear();
                                        break;
                                    }
                                } else {
                                    self.current_path[i].clear();
                                    break;
                                }
                            } else {
                                break;
                            }
                        }

                        // 3. Distance logic
                        let lane_id = self.current_lane_id[i];
                        if lane_id >= transit_network.lane_system.lanes.len() {
                            self.current_lane_id[i] = usize::MAX;
                            self.current_path[i].clear();
                            break;
                        }

                        let lane = &transit_network.lane_system.lanes[lane_id];
                        let dist_to_end = lane.length - self.lane_distance[i];

                        if remaining_dist < dist_to_end {
                            // Move cleanly along the current lane
                            self.lane_distance[i] += remaining_dist;
                            remaining_dist = 0.0;
                            
                            // Check for Arrival midway
                            let t_bldg_idx = self.target_building[i];
                            if t_bldg_idx != usize::MAX && t_bldg_idx < allocator.buildings.len() {
                                let b = &allocator.buildings[t_bldg_idx];
                                if lane.edge_id == b.edge_idx {
                                    // Check distance along edge
                                    let tgt_len = graph.edges[b.edge_idx].physical_length;
                                    let progress_ratio = self.lane_distance[i] / lane.length.max(0.001);
                                    let agent_prog = if lane.is_fwd {
                                        progress_ratio * tgt_len
                                    } else {
                                        (1.0 - progress_ratio) * tgt_len
                                    };
                                    if (agent_prog - (b.frontage_t * tgt_len)).abs() < 4.0 {
                                        self.transit[i] = TRANSIT_ARRIVING;
                                        self.transit_mode[i] = MODE_WALK;
                                        self.current_lane_id[i] = usize::MAX;
                                        remaining_dist = 0.0;
                                    }
                                }
                            }
                            
                        } else {
                            // Reached the end of the lane!
                            remaining_dist -= dist_to_end;

                            if lane.edge_id != usize::MAX {
                                // Reached the end of a ROAD.
                                // Update node and fetch next edge
                                self.current_node[i] = if lane.is_fwd {
                                    graph.edges[lane.edge_id].end_node
                                } else {
                                    graph.edges[lane.edge_id].start_node
                                };

                                self.current_path_index[i] += 1;
                                
                                if self.current_path_index[i] < self.current_path[i].len() {
                                    let next_node = self.current_path[i][self.current_path_index[i]];
                                    if let Some(best_e) = graph.get_edge_between_nodes(self.current_node[i], next_node) {
                                        
                                        // Pick a connection lane leading to best_e
                                        let mut valid_conns = Vec::new();
                                        for &c_id in &lane.next_lanes {
                                            if c_id < transit_network.lane_system.lanes.len() {
                                                let conn_lane = &transit_network.lane_system.lanes[c_id];
                                                if !conn_lane.next_lanes.is_empty() {
                                                    let tgt_road_lane = conn_lane.next_lanes[0];
                                                    if tgt_road_lane < transit_network.lane_system.lanes.len() {
                                                        if transit_network.lane_system.lanes[tgt_road_lane].edge_id == best_e {
                                                            valid_conns.push(c_id);
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        if !valid_conns.is_empty() {
                                            self.current_lane_id[i] = valid_conns[rng.gen_range(0..valid_conns.len())];
                                            self.lane_distance[i] = 0.0;
                                            self.transit[i] = TRANSIT_INTERSECTION;
                                            self.current_edge[i] = usize::MAX;
                                        } else {
                                            // Stuck / restricted turn rules
                                            self.current_path[i].clear();
                                            self.current_lane_id[i] = usize::MAX;
                                            break;
                                        }

                                    } else {
                                        self.current_path[i].clear();
                                        self.current_lane_id[i] = usize::MAX;
                                        break;
                                    }
                                } else {
                                    // Path complete? (Normally intercepted by ARRIVAL first)
                                    self.current_path[i].clear();
                                    self.current_lane_id[i] = usize::MAX;
                                    break;
                                }

                            } else {
                                // Reached the end of an INTERSECTION CONNECTION. Move directly to its next road lane.
                                if !lane.next_lanes.is_empty() {
                                    let tgt_road_lane = lane.next_lanes[0];
                                    if tgt_road_lane < transit_network.lane_system.lanes.len() {
                                        self.current_lane_id[i] = tgt_road_lane;
                                        self.lane_distance[i] = 0.0;
                                        self.transit[i] = TRANSIT_ON_ROAD;
                                        self.current_edge[i] = transit_network.lane_system.lanes[tgt_road_lane].edge_id;
                                    } else {
                                        self.current_path[i].clear();
                                        self.current_lane_id[i] = usize::MAX;
                                        break;
                                    }
                                } else {
                                    self.current_path[i].clear();
                                    self.current_lane_id[i] = usize::MAX;
                                    break;
                                }
                            }
                        }
                    }

                    // 4. Transform Output for Rendering (Extract pos_x / pos_y from the lane system)
                    let current_lane = self.current_lane_id[i];
                    if current_lane != usize::MAX && current_lane < transit_network.lane_system.lanes.len() {
                        let l = &transit_network.lane_system.lanes[current_lane];
                        let dist = self.lane_distance[i];
                        if dist <= 0.0 && !l.geometry.is_empty() {
                            self.pos_x[i] = l.geometry[0].x;
                            self.pos_y[i] = l.geometry[0].z;
                        } else if dist >= l.length && !l.geometry.is_empty() {
                            let end = l.geometry.last().unwrap();
                            self.pos_x[i] = end.x;
                            self.pos_y[i] = end.z;
                        } else if l.geometry.len() >= 2 {
                            let mut curr = 0.0;
                            let mut found = false;
                            for j in 0..l.geometry.len() - 1 {
                                let p0 = l.geometry[j];
                                let p1 = l.geometry[j+1];
                                let d = p0.distance_to(p1);
                                if curr + d >= dist {
                                    let t = if d > 1e-5 { (dist - curr) / d } else { 0.0 };
                                    let mut out = p0.lerp(p1, t.clamp(0.0, 1.0));
                                    
                                    if self.transit_mode[i] == MODE_WALK && d > 1e-5 {
                                        let tangent = (p1 - p0) / d;
                                        let normal = Vector3::new(-tangent.z, 0.0, tangent.x);
                                        let jitter = (f32::sin(i as f32 * 4.0) + f32::cos(i as f32 * 7.0)) * 0.7;
                                        out += normal * jitter;
                                    }
                                    
                                    self.pos_x[i] = out.x;
                                    self.pos_y[i] = out.z;
                                    found = true;
                                    break;
                                }
                                curr += d;
                            }
                            if !found {
                                let end = l.geometry.last().unwrap();
                                self.pos_x[i] = end.x;
                                self.pos_y[i] = end.z;
                            }
                        }
                    }

                }
                TRANSIT_ARRIVING => {
                    // FROM ROAD TO BUILDING
                    let b_id = self.target_building[i];
                    if b_id == usize::MAX {
                        self.transit[i] = TRANSIT_IDLE;
                        continue;
                    }
                    let b = &allocator.buildings[b_id];
                    let center_vec = Vector2::new(b.center_x, b.center_y);

                    let dir_to_center = center_vec - Vector2::new(self.pos_x[i], self.pos_y[i]);
                    let dist = dir_to_center.length();
                    let speed = if self.transit_mode[i] == MODE_CAR {
                        10.0
                    } else {
                        4.0
                    };
                    let step = speed * delta;

                    if dist < step {
                        let prev_activity = self.activity[i];
                        self.pos_x[i] = center_vec.x;
                        self.pos_y[i] = center_vec.y;
                        self.current_building[i] = b_id;
                        self.is_visible[i] = false;
                        self.transit[i] = TRANSIT_IDLE;
                        self.transit_mode[i] = MODE_WALK;
                        self.current_edge[i] = usize::MAX;
                        self.current_lane_id[i] = usize::MAX;
                        self.lane_distance[i] = 0.0;
                        self.current_path[i].clear();
                        self.current_path_index[i] = 0;

                        let commute_time = self.sim_time - self.journey_start_time[i];
                        self.happiness[i] =
                            (self.happiness[i] - commute_time / 60.0).clamp(0.0, 100.0);
                        if prev_activity == 2 {
                            self.money[i] = (self.money[i] - 20.0).max(0.0);
                        }
                    } else if dist > 0.0001 {
                        let mv = dir_to_center.normalized() * step;
                        self.pos_x[i] += mv.x;
                        self.pos_y[i] += mv.y;
                    }
                }
                _ => {
                    self.transit[i] = TRANSIT_IDLE;
                }
            }
        }
    }
}
