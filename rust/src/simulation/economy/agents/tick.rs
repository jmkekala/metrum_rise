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
use crate::simulation::pathing::pedestrian::{
    find_path as find_pedestrian_path, PedestrianEndpoint,
};
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
                    $self.current_node[$i] = if b.frontage_t < 0.5 {
                        edge.start_node
                    } else {
                        edge.end_node
                    };
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
                            let start_ped = if self.current_building[i] != usize::MAX
                                && self.current_building[i] < allocator.buildings.len()
                            {
                                let curr_b = &allocator.buildings[self.current_building[i]];
                                let curr_edge = &graph.edges[curr_b.edge_idx];
                                let curr_node = if curr_b.frontage_t < 0.5 {
                                    curr_edge.start_node
                                } else {
                                    curr_edge.end_node
                                };
                                PedestrianEndpoint {
                                    node: curr_node,
                                    edge_idx: Some(curr_b.edge_idx),
                                    side: curr_b.side,
                                }
                            } else {
                                PedestrianEndpoint {
                                    node: self.current_node[i],
                                    edge_idx: None,
                                    side: 0,
                                }
                            };
                            let target_ped = PedestrianEndpoint {
                                node: target_node,
                                edge_idx: Some(b.edge_idx),
                                side: b.side,
                            };
                            let (_final_target, mode) = self.decide_transit_mode_with_endpoints(
                                i,
                                start_ped,
                                target_ped,
                                target_node,
                                graph,
                                &transit_network.cch_graph,
                            );
                            self.target_node[i] = target_node;
                            self.transit_mode[i] = mode;
                            self.current_path[i].clear();
                            self.current_ped_path[i].clear();
                            self.current_path_index[i] = 0;
                            self.current_lane_id[i] = usize::MAX;
                            self.lane_distance[i] = 0.0;

                            if mode == MODE_WALK {
                                if let Some((_, _, path)) =
                                    find_pedestrian_path(graph, start_ped, target_ped)
                                {
                                    self.current_ped_path[i] = path;
                                }
                                self.pedestrian_side[i] = if start_ped.side != 0 {
                                    start_ped.side
                                } else {
                                    target_ped.side
                                };
                            } else {
                                self.pedestrian_side[i] = 0;
                            }
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

                    if self.transit_mode[i] == MODE_CAR && self.current_lane_id[i] == usize::MAX {
                        let b = &allocator.buildings[b_id];
                        let edge = &graph.edges[b.edge_idx];
                        let is_fwd = node_idx == edge.end_node;
                        
                        if let Some(edge_lanes) = transit_network.lane_system.edge_lanes.get(&b.edge_idx) {
                            let mut valid_lanes = Vec::new();
                            for &l_id in edge_lanes {
                                if transit_network.lane_system.lanes[l_id].is_fwd == is_fwd {
                                    valid_lanes.push(l_id);
                                }
                            }
                            if !valid_lanes.is_empty() {
                                let l_id = valid_lanes[rng.gen_range(0..valid_lanes.len())];
                                self.current_lane_id[i] = l_id;
                                let lane = &transit_network.lane_system.lanes[l_id];
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
                        let b = &allocator.buildings[b_id];
                        let edge = &graph.edges[b.edge_idx];
                        if self.transit_mode[i] == MODE_WALK {
                            let world_pos_on_edge =
                                allocator.get_pos_on_edge(graph, b.edge_idx, b.frontage_t);
                            let tangent =
                                allocator.get_tangent_on_edge(graph, b.edge_idx, b.frontage_t);
                            let normal = Vector2::new(-tangent.y, tangent.x) * (b.side as f32);
                            
                            let sw_w = crate::config::SIDEWALK_WIDTH;
                            let offset_amt = edge.width * 0.5 + sw_w * 0.5;
                            self.pedestrian_side[i] = b.side;
                            world_pos_on_edge + normal * offset_amt
                        } else {
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
                        }
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

                    // PEDESTRIAN LOGIC DOES NOT USE LANESYSTEM YET.
                    if self.transit_mode[i] == MODE_WALK {
                        self.tick_pedestrian(i, &mut remaining_dist, allocator, graph);
                        continue;
                    }

                    // === LANE GRAPH LOGIC (CARS) ===
                    while remaining_dist > 0.0 {
                        
                        // 1. Initialise Path if missing
                        if self.current_path[i].is_empty() {
                            self.pathfind_count += 1;
                            let mut path_found = false;
                            if let Some((_, _, path)) = transit_network.cch_graph.find_path(
                                self.current_node[i],
                                self.target_node[i],
                                usize::MAX,
                                graph,
                                TransitFlags::CAR,
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
                                            if transit_network.lane_system.lanes[l_id].is_fwd == is_fwd {
                                                valid_lanes.push(l_id);
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
                                    let out = p0.lerp(p1, t.clamp(0.0, 1.0));
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
                        self.current_ped_path[i].clear();
                        self.current_path_index[i] = 0;
                        self.pedestrian_side[i] = 0;

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
    
    // Extracted pedestrian fallback logic
    fn tick_pedestrian(&mut self, i: usize, remaining_dist: &mut f32, allocator: &BuildingAllocator, graph: &RegionGraph) {
        // Pedestrians use simple snapping & lines until proper sidewalks are baked into LaneSystem.
        let mut rd = *remaining_dist;
        while rd > 0.0 {
            if self.current_ped_path[i].is_empty() {
                self.pathfind_count += 1;
                let t_bldg_idx = self.target_building[i];
                if t_bldg_idx != usize::MAX && t_bldg_idx < allocator.buildings.len() {
                    if let Some((_, _, path)) = find_pedestrian_path(
                        graph,
                        PedestrianEndpoint { node: self.current_node[i], edge_idx: None, side: 0 },
                        PedestrianEndpoint { 
                            node: self.target_node[i], 
                            edge_idx: Some(allocator.buildings[t_bldg_idx].edge_idx), 
                            side: allocator.buildings[t_bldg_idx].side 
                        },
                    ) {
                        self.current_ped_path[i] = path;
                        self.current_path_index[i] = 0;
                    }
                } else {
                    self.current_ped_path[i].clear();
                    break;
                }
            }
            
            if self.current_path_index[i] < self.current_ped_path[i].len() {
                let step = self.current_ped_path[i][self.current_path_index[i]];
                if step.edge_idx >= graph.edges.len() || graph.edges[step.edge_idx].deleted {
                    self.current_ped_path[i].clear();
                    break; 
                }
                
                let edge = &graph.edges[step.edge_idx];
                let is_fwd = step.forward;
                if edge.physical_geometry.len() < 2 {
                    self.current_ped_path[i].clear();
                    break;
                }
                                
                let tgt_idx = if is_fwd { edge.physical_geometry.len() - 1 } else { 0 };
                let p = edge.physical_geometry[tgt_idx];
                let tangent = if is_fwd { 
                    (edge.physical_geometry[tgt_idx] - edge.physical_geometry[tgt_idx-1]).normalized() 
                } else { 
                    (edge.physical_geometry[1] - edge.physical_geometry[0]).normalized() 
                };
                let mut target_pos = Vector2::new(p.x, p.z);
                if step.side != 0 {
                    let normal = Vector2::new(-tangent.z, tangent.x);
                    let offset_amt = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH * 0.5;
                    target_pos += normal * offset_amt * (step.side as f32);
                }
                
                // Arrival Check
                let t_bldg_idx = self.target_building[i];
                if t_bldg_idx != usize::MAX && t_bldg_idx < allocator.buildings.len() {
                    let b = &allocator.buildings[t_bldg_idx];
                    if step.edge_idx == b.edge_idx {
                        let fp = allocator.get_pos_on_edge(graph, b.edge_idx, b.frontage_t);
                        let to_agent = Vector2::new(self.pos_x[i], self.pos_y[i]) - fp;
                        if to_agent.length_squared() < 4.0 {
                            self.transit[i] = TRANSIT_ARRIVING;
                            rd = 0.0;
                            break;
                        }
                    }
                }
                
                let d = (target_pos - Vector2::new(self.pos_x[i], self.pos_y[i])).length();
                if d < rd {
                    self.pos_x[i] = target_pos.x;
                    self.pos_y[i] = target_pos.y;
                    self.current_path_index[i] += 1;
                    self.pedestrian_side[i] = step.side;
                    rd -= d;
                } else {
                    let mv = (target_pos - Vector2::new(self.pos_x[i], self.pos_y[i])).normalized() * rd;
                    self.pos_x[i] += mv.x;
                    self.pos_y[i] += mv.y;
                    rd = 0.0;
                }
            } else {
                self.current_ped_path[i].clear();
                break;
            }
        }
        *remaining_dist = rd;
    }
}
