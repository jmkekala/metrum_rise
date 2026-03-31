use godot::prelude::*;
use std::collections::HashMap;

use super::graph::{Edge, RegionGraph};
use super::types::{NodeType, TransitFlags, TransitType};
#[cfg(test)]
use super::types::EdgeClass;
use crate::config;

/// Types of travel lanes supported by the network.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LaneType {
    /// Lane for motorized vehicles.
    Vehicle,
    /// Lane for pedestrians.
    Foot,
}

/// A single travel lane through a road or intersection.
#[derive(Clone)]
pub struct Lane {
    /// The parent road edge ID. `usize::MAX` for intersection connections.
    pub edge_id: usize,
    /// Direction relative to the edge geometry.
    pub is_fwd: bool,
    /// Lane index (0 is innermost).
    pub lane_idx: i8,
    /// The physical path of the lane.
    pub geometry: Vec<Vector3>,
    /// Total length in meters.
    pub length: f32,
    /// Cumulative distance at each geometry vertex: `cum_dist[i]` = distance from `geometry[0]` to `geometry[i]`.
    /// Used for O(log N) position interpolation via binary search.
    pub cum_dist: Vec<f32>,
    /// The travel type of this lane.
    pub lane_type: LaneType,
    /// Whether this is a visual crosswalk.
    pub is_crosswalk: bool,
    /// Reachable lanes from the end of this lane.
    pub next_lanes: Vec<usize>,
}

impl Default for Lane {
    fn default() -> Self {
        Self {
            edge_id: usize::MAX,
            is_fwd: true,
            lane_idx: 0,
            geometry: Vec::new(),
            length: 0.0,
            cum_dist: Vec::new(),
            lane_type: LaneType::Vehicle,
            is_crosswalk: false,
            next_lanes: Vec::new(),
        }
    }
}

// Builds the cumulative-distance prefix sum for a lane's geometry.
fn build_cum_dist(geometry: &[Vector3]) -> Vec<f32> {
    let mut v = Vec::with_capacity(geometry.len());
    let mut acc = 0.0;
    for i in 0..geometry.len() {
        if i > 0 {
            acc += geometry[i - 1].distance_to(geometry[i]);
        }
        v.push(acc);
    }
    v
}

/// System for managing road and intersection lanes.
pub struct LaneSystem {
    /// All active lanes.
    pub lanes: Vec<Lane>,
    /// Mapping of edge IDs to their constituent lanes.
    pub edge_lanes: HashMap<usize, Vec<usize>>,
}

impl LaneSystem {
    /// Creates a new, empty lane system.
    pub fn new() -> Self {
        Self {
            lanes: Vec::new(),
            edge_lanes: HashMap::new(),
        }
    }

    /// Clears all lanes and structural mappings.
    pub fn clear(&mut self) {
        self.lanes.clear();
        self.edge_lanes.clear();
    }

    /// Retrieve the global `lane_id` given an `edge_idx` and a local `lane_idx`.
    pub fn get_lane_id(&self, edge_idx: usize, lane_idx: usize) -> Option<usize> {
        self.edge_lanes.get(&edge_idx).and_then(|lanes| {
            lanes.iter().find(|&&id| self.lanes[id].lane_idx == lane_idx as i8).copied()
        })
    }

    /// Completely rebuilds all physical lane geometry and connection splines for the entire graph.
    /// To be called after the road network topology and physical geometries have been updated.
    pub fn rebuild(&mut self, graph: &mut RegionGraph) {
        self.clear();
        
        // Maps (edge_id, is_fwd, lane_idx) -> lane_index in self.lanes
        let mut lane_map: HashMap<(usize, bool, i8), usize> = HashMap::new();

        // 1. Build Straight Lanes for all active edges
        for (edge_idx, edge) in graph.edges.iter().enumerate() {
            if edge.deleted || edge.physical_geometry.len() < 2 {
                continue;
            }

            let mut edge_lane_indices = Vec::new();



            // Helper to build a lane
            let mut build_lane = |is_fwd: bool, lane_idx: i8, lane_type: LaneType, lane_offset: f32| {
                let mut geometry = Vec::with_capacity(edge.physical_geometry.len());
                let pts = &edge.physical_geometry;
                
                // Add first point
                let mut dir0 = pts[1] - pts[0];
                dir0.y = 0.0;
                let t0 = if dir0.length() > 1e-5 { dir0.normalized() } else { Vector3::new(1.0, 0.0, 0.0) };
                let n0 = Vector3::new(-t0.z, 0.0, t0.x);
                geometry.push(pts[0] + n0 * lane_offset);

                // Mid points: bisector normal
                for j in 1..pts.len()-1 {
                    let mut d1 = pts[j] - pts[j-1];
                    let mut d2 = pts[j+1] - pts[j];
                    d1.y = 0.0; d2.y = 0.0;
                    let t1 = if d1.length() > 1e-5 { d1.normalized() } else { t0 };
                    let t2 = if d2.length() > 1e-5 { d2.normalized() } else { t1 };
                    let n1 = Vector3::new(-t1.z, 0.0, t1.x);
                    let n2 = Vector3::new(-t2.z, 0.0, t2.x);
                    let bisect = (n1 + n2).normalized();
                    let dot = n1.dot(bisect).max(0.1);
                    geometry.push(pts[j] + bisect * (lane_offset / dot));
                }

                // Last point
                let mut d_last = pts[pts.len()-1] - pts[pts.len()-2];
                d_last.y = 0.0;
                let t_last = if d_last.length() > 1e-5 { d_last.normalized() } else { t0 };
                let n_last = Vector3::new(-t_last.z, 0.0, t_last.x);
                geometry.push(pts[pts.len()-1] + n_last * lane_offset);

                if !is_fwd {
                    geometry.reverse();
                }

                let mut length = 0.0;
                for j in 0..geometry.len()-1 {
                    length += geometry[j].distance_to(geometry[j+1]);
                }

                let cum_dist = build_cum_dist(&geometry);
                let new_lane_id = self.lanes.len();
                self.lanes.push(Lane {
                    edge_id: edge_idx,
                    is_fwd,
                    lane_idx,
                    geometry,
                    length,
                    cum_dist,
                    lane_type,
                    is_crosswalk: false,
                    next_lanes: Vec::new(),
                });
                
                lane_map.insert((edge_idx, is_fwd, lane_idx), new_lane_id);
                edge_lane_indices.push(new_lane_id);
            };

            let lane_w = config::LANE_WIDTH;
            let sidewalk_w = config::SIDEWALK_WIDTH;
            let asphalt_width = (edge.fwd_lanes + edge.bkw_lanes) as f32 * lane_w;
            let side_mul = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };

            // 1. Forward Lanes
            for l in 0..edge.fwd_lanes {
                // Lane 0 is closest to center
                let lane_offset = (l as f32 + 0.5) * lane_w * side_mul;
                build_lane(true, l as i8, LaneType::Vehicle, lane_offset);
            }

            // 2. Backward Lanes
            for l in 0..edge.bkw_lanes {
                // Lane 0 is closest to center
                let lane_offset = -(l as f32 + 0.5) * lane_w * side_mul;
                build_lane(false, - (l as i8) - 1, LaneType::Vehicle, lane_offset);
            }

            // 3. Sidewalks
            if (edge.allowed_types & TransitFlags::FOOT) != 0 {
                if edge.primary_type == TransitType::Foot {
                    // Dedicated Footpath: center lane
                    build_lane(true, 0, LaneType::Foot, 0.0);
                    build_lane(false, 0, LaneType::Foot, 0.0);
                } else {
                    // Left Sidewalk (idx 100)
                    let left_offset = -(asphalt_width * 0.5 + sidewalk_w * 0.5) * side_mul;
                    build_lane(true, 100, LaneType::Foot, left_offset);
                    build_lane(false, 100, LaneType::Foot, left_offset);

                    // Right Sidewalk (idx -100)
                    let right_offset = (asphalt_width * 0.5 + sidewalk_w * 0.5) * side_mul;
                    build_lane(true, -100, LaneType::Foot, right_offset);
                    build_lane(false, -100, LaneType::Foot, right_offset);
                }
            }

            self.edge_lanes.insert(edge_idx, edge_lane_indices);
        }

        // 2. Build Connection Lanes (Intersections)
        // To build connections, we need to know all inbound lanes to a node, and all outbound lanes from a node.
        for node_id in 0..graph.nodes.len() {
            let mut inbound = Vec::new();
            let mut outbound = Vec::new();

            for e_idx in &graph.adjacency[node_id] {
                let edge = &graph.edges[*e_idx];
                if edge.deleted { continue; }
                
                if edge.start_node as usize == node_id {
                    // Edge leaves this node
                    for l in 0..edge.fwd_lanes {
                        if let Some(&lane_id) = lane_map.get(&(*e_idx, true, l as i8)) {
                            outbound.push((*e_idx, l as i8, lane_id));
                        }
                    }
                    for l in 0..edge.bkw_lanes {
                        if let Some(&lane_id) = lane_map.get(&(*e_idx, false, (- (l as i8) - 1))) {
                            inbound.push((*e_idx, (- (l as i8) - 1), lane_id));
                        }
                    }
                }
                
                if edge.end_node as usize == node_id {
                    // Edge enters this node
                    for l in 0..edge.fwd_lanes {
                        if let Some(&lane_id) = lane_map.get(&(*e_idx, true, l as i8)) {
                            inbound.push((*e_idx, l as i8, lane_id));
                        }
                    }
                    for l in 0..edge.bkw_lanes {
                        if let Some(&lane_id) = lane_map.get(&(*e_idx, false, (- (l as i8) - 1))) {
                            outbound.push((*e_idx, (- (l as i8) - 1), lane_id));
                        }
                    }
                }
            }

            let node = &graph.nodes[node_id];

            for &(in_edge_id, in_lane_idx, in_lane_id) in &inbound {
                // Check allowed turn restrictions
                let mut allowed_targets = node.lane_connections.get(&(in_edge_id, in_lane_idx)).cloned();
                
                // If no rules exist, generate all meaningful turns (no U-turns except at dead ends)
                if allowed_targets.is_none() {
                    let mut defaults = Vec::new();
                    let node_deg = graph.adjacency[node_id].len();
                    for &(out_edge_id, out_lane_idx, _) in &outbound {
                        if out_edge_id != in_edge_id || node_deg == 1 {
                            defaults.push((out_edge_id, out_lane_idx));
                        }
                    }
                    if !defaults.is_empty() {
                        allowed_targets = Some(defaults);
                    }
                }

                let mut valid_out_lanes = Vec::new();
                for &(out_edge_id, out_lane_idx, out_lane_id) in &outbound {
                    if let Some(rules) = &allowed_targets {
                        if rules.contains(&(out_edge_id, out_lane_idx)) {
                            valid_out_lanes.push(out_lane_id);
                        }
                    }
                }

                for out_lane_id in valid_out_lanes {
                    // Create connecting bezier
                    let p0 = *self.lanes[in_lane_id].geometry.last().unwrap();
                    let p1_base = if self.lanes[in_lane_id].geometry.len() >= 2 {
                        let len = self.lanes[in_lane_id].geometry.len();
                        let d = self.lanes[in_lane_id].geometry[len-1] - self.lanes[in_lane_id].geometry[len-2];
                        if d.length_squared() > 0.00001 { d.normalized() } else { Vector3::new(1.0, 0.0, 0.0) }
                    } else {
                        Vector3::new(1.0, 0.0, 0.0)
                    };

                    let p3 = self.lanes[out_lane_id].geometry[0];
                    let p2_base = if self.lanes[out_lane_id].geometry.len() >= 2 {
                        let d = self.lanes[out_lane_id].geometry[1] - self.lanes[out_lane_id].geometry[0];
                        if d.length_squared() > 0.00001 { d.normalized() } else { Vector3::new(1.0, 0.0, 0.0) }
                    } else {
                        Vector3::new(1.0, 0.0, 0.0)
                    };

                    let dist = p0.distance_to(p3);
                    
                    let curve_dist = dist * 0.35;

                    let p1 = p0 + p1_base * curve_dist;
                    let p2 = p3 - p2_base * curve_dist;

                    // Evaluate bezier into exactly 5 points
                    let steps = 5;
                    let mut conn_geom = Vec::with_capacity(steps + 1);
                    let mut conn_len = 0.0;
                    for i in 0..=steps {
                        let t = i as f32 / steps as f32;
                        let mut p = (1.0 - t).powi(3) * p0
                            + 3.0 * (1.0 - t).powi(2) * t * p1
                            + 3.0 * (1.0 - t) * t.powi(2) * p2
                            + t.powi(3) * p3;
                        p.y = p0.y + (p3.y - p0.y) * t; // Linear height interp
                        conn_geom.push(p);

                        if i > 0 {
                            conn_len += conn_geom[i-1].distance_to(p);
                        }
                    }

                    let conn_cum_dist = build_cum_dist(&conn_geom);
                    let conn_lane_id = self.lanes.len();
                    self.lanes.push(Lane {
                        edge_id: usize::MAX,
                        is_fwd: true,
                        lane_idx: 0,
                        geometry: conn_geom,
                        length: conn_len,
                        cum_dist: conn_cum_dist,
                        lane_type: LaneType::Vehicle,
                        is_crosswalk: false,
                        next_lanes: vec![out_lane_id], // points to the out road lane
                    });

                    // In_lane points to the connection lane
                    self.lanes[in_lane_id].next_lanes.push(conn_lane_id);
                }
            }

            // --- Build Pedestrian Crosswalks / Sidewalk connections (Refined) ---
            let mut mouths = Vec::new();

            let node_pos = graph.nodes[node_id].pos;
            for &e_idx in &graph.adjacency[node_id] {
                let edge = &graph.edges[e_idx];
                if edge.deleted || (edge.allowed_types & TransitFlags::FOOT) == 0 || edge.primary_type != TransitType::Road {
                    continue;
                }

                let is_start = edge.start_node as usize == node_id;
                let other_p = if is_start { edge.geometry[1] } else { edge.geometry[edge.geometry.len() - 2] };
                let diff = other_p - node_pos;
                let dist = other_p.distance_to(node_pos);
                let dir = if dist > 1e-4 { diff / dist } else { Vector3::ZERO };
                let side_vec = Vector3::new(-dir.z, 0.0, dir.x); // Normal to road

                for &l_idx in &[-100_i8, 100_i8] {
                    let side = (l_idx as f32) / 100.0;
                    let offset = -(road_half_width(edge) + config::SIDEWALK_WIDTH * 0.5) * side;
                    // Sample slightly further to ensure road direction handles sorting
                    let mouth_pos = node_pos + dir * 5.0 + side_vec * offset;
                    let mouth_angle = (mouth_pos.x - node_pos.x).atan2(mouth_pos.z - node_pos.z);
                    
                    let (inbound, outbound) = if is_start {
                        (lane_map.get(&(e_idx, false, l_idx)).copied(),
                         lane_map.get(&(e_idx, true, l_idx)).copied())
                    } else {
                        (lane_map.get(&(e_idx, true, l_idx)).copied(),
                         lane_map.get(&(e_idx, false, l_idx)).copied())
                    };

                    if let (Some(in_id), Some(out_id)) = (inbound, outbound) {
                        mouths.push(SidewalkMouth {
                            edge_idx: e_idx,
                            lane_idx: l_idx,
                            angle: mouth_angle,
                            in_id,
                            out_id,
                        });
                    }
                }
            }

            mouths.sort_by(|a, b| a.angle.partial_cmp(&b.angle).unwrap());
            let num_mouths = mouths.len();
            if num_mouths < 2 { continue; }

            let node_ref = &mut graph.nodes[node_id];
            node_ref.lane_connections.clear();

            let mut crosswalks_added = 0;
            for i in 0..num_mouths {
                let m_start = &mouths[i];
                for j in 0..num_mouths {
                    if i == j { continue; }
                    let m_end = &mouths[j];
                    let diff_cw = if j > i { j - i } else { j + num_mouths - i };
                    let diff_ccw = num_mouths - diff_cw;
                    let use_cw = diff_cw <= diff_ccw;
                    let num_steps = if use_cw { diff_cw } else { diff_ccw };
                    if num_steps > 1 { continue; }
                    
                    let mut steps = Vec::new();
                    let mut current = i;
                    for _ in 0..num_steps {
                        let next = if use_cw { (current + 1) % num_mouths } else { (current + num_mouths - 1) % num_mouths };
                        let p0 = *self.lanes[mouths[current].in_id].geometry.last().unwrap();
                        let p1 = self.lanes[mouths[next].out_id].geometry[0];
                        if steps.is_empty() { steps.push(p0); }
                        steps.push(p1);
                        current = next;
                    }

                    let is_same_edge = m_start.edge_idx == m_end.edge_idx;
                    let deg = graph.adjacency[node_id].len();
                    let node_type = node_ref.node_type;
                    
                    // Frontage Node Restriction: No crossings Allowed.
                    // Strictly forbid any side-switching (lane_idx change) at frontage nodes.
                    if node_type == NodeType::Frontage && (is_same_edge || m_start.lane_idx != m_end.lane_idx) {
                        continue;
                    }

                    let skip_visual = is_same_edge && deg <= 2 && crosswalks_added >= 2;
                    let is_crosswalk = is_same_edge && num_steps == 1 && !skip_visual;
                    if is_crosswalk { crosswalks_added += 1; }

                    let mut dist = 0.0;
                    for k in 0..steps.len().saturating_sub(1) { dist += steps[k].distance_to(steps[k+1]); }

                    let steps_cum_dist = build_cum_dist(&steps);
                    let conn_id = self.lanes.len();
                    self.lanes.push(Lane {
                        edge_id: usize::MAX,
                        is_fwd: true,
                        lane_idx: 0,
                        geometry: steps,
                        length: dist,
                        cum_dist: steps_cum_dist,
                        lane_type: LaneType::Foot,
                        is_crosswalk,
                        next_lanes: vec![m_end.out_id],
                    });
                    self.lanes[m_start.in_id].next_lanes.push(conn_id);

                    node_ref.lane_connections.entry((m_start.edge_idx, m_start.lane_idx)).or_default().push((m_end.edge_idx, m_end.lane_idx));
                }
            }
        }
    }
}

struct SidewalkMouth {
    edge_idx: usize,
    lane_idx: i8,
    angle: f32,
    in_id: usize,
    out_id: usize,
}

fn road_half_width(edge: &Edge) -> f32 {
    ((edge.fwd_lanes + edge.bkw_lanes) as f32) * config::LANE_WIDTH * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::RegionGraph;

    #[test]
    fn test_lane_geometry_and_length() {
        let mut graph = RegionGraph::new();
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), crate::simulation::network::types::NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), crate::simulation::network::types::NodeType::Junction);

        graph.add_edge(crate::simulation::network::graph::data::Edge {
            start_node: n1,
            end_node: n2,
            primary_type: crate::simulation::network::types::TransitType::Road,
            allowed_types: crate::simulation::network::types::TransitFlags::CAR | crate::simulation::network::types::TransitFlags::FOOT,
            class: crate::simulation::network::types::EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 2.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });
        graph.rebuild_adjacency_list();

        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);

        // Should produce 6 physical lanes: 1 FWD Vehicle, 1 BKW Vehicle, and 4 Foot lanes (2 on each side).
        // Connection lanes (U-turns at dead ends, crosswalks) are also generated but are not physical lanes.
        let physical: Vec<_> = lanes.lanes.iter().filter(|l| l.edge_id != usize::MAX).collect();
        assert_eq!(physical.len(), 6, "Expected 2 vehicle and 4 foot physical lanes");

        let l_fwd = &lanes.lanes[0];
        assert!(l_fwd.is_fwd, "Lane 0 should be forward");
        assert!((l_fwd.length - 100.0).abs() < 1.0, "FWD lane length should be roughly 100m, was {}", l_fwd.length);
        assert!((l_fwd.geometry.first().unwrap().x - 0.0).abs() < 1.0, "FWD start X should be near 0");
        assert!((l_fwd.geometry.last().unwrap().x - 100.0).abs() < 1.0, "FWD end X should be near 100");

        let l_bkw = &lanes.lanes[1];
        assert!(!l_bkw.is_fwd, "Lane 1 should be backward");
        assert!((l_bkw.length - 100.0).abs() < 1.0, "BKW lane length should be roughly 100m, was {}", l_bkw.length);
        assert!((l_bkw.geometry.first().unwrap().x - 100.0).abs() < 1.0, "BKW start X should be near 100");
        assert!((l_bkw.geometry.last().unwrap().x - 0.0).abs() < 1.0, "BKW end X should be near 0");
    }

    #[test]
    fn test_highway_no_sidewalks() {
        let mut graph = RegionGraph::new();
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), crate::simulation::network::types::NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), crate::simulation::network::types::NodeType::Junction);

        graph.add_edge(crate::simulation::network::graph::data::Edge {
            start_node: n1,
            end_node: n2,
            primary_type: crate::simulation::network::types::TransitType::Road,
            allowed_types: crate::simulation::network::types::TransitFlags::CAR, // No FOOT
            class: crate::simulation::network::types::EdgeClass::Standard,
            width: 14.0,
            fwd_lanes: 2,
            bkw_lanes: 2,
            speed_limit: 100.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });
        graph.rebuild_adjacency_list();

        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);

        // Should produce 4 physical vehicle lanes and 0 foot lanes.
        // Connection lanes (U-turns at dead ends) are also generated but are not physical lanes.
        let physical: Vec<_> = lanes.lanes.iter().filter(|l| l.edge_id != usize::MAX).collect();
        assert_eq!(physical.len(), 4, "Highways should have no foot lanes");
        for lane in &physical {
            assert_eq!(lane.lane_type, LaneType::Vehicle);
        }
    }

    #[test]
    fn test_dedicated_footpath_centering() {
        let mut graph = RegionGraph::new();
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), crate::simulation::network::types::NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), crate::simulation::network::types::NodeType::Junction);

        graph.add_edge(crate::simulation::network::graph::data::Edge {
            start_node: n1,
            end_node: n2,
            primary_type: crate::simulation::network::types::TransitType::Foot,
            allowed_types: crate::simulation::network::types::TransitFlags::FOOT,
            class: crate::simulation::network::types::EdgeClass::Standard,
            width: 3.0,
            fwd_lanes: 0,
            bkw_lanes: 0,
            speed_limit: 5.0,
            base_cost: 20.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });
        graph.rebuild_adjacency_list();

        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);

        // Should produce 2 foot lanes (FWD/BKW) at center (idx 0).
        assert_eq!(lanes.lanes.len(), 2, "Dedicated footpaths should have 2 bidirectional lanes");
        for lane in &lanes.lanes {
            assert_eq!(lane.lane_type, LaneType::Foot);
            assert_eq!(lane.lane_idx, 0);
            // Check that offset was 0 (visually, start/end X should be on the centerline)
            assert!((lane.geometry[0].z - 0.0).abs() < 0.1);
        }
    }
    #[test]
    fn test_junction_pedestrian_connectivity() {
        let mut graph = RegionGraph::new();
        // n0 ---- n1 (Junction) ---- n2
        //          |
        //          n3
        let n0 = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(0.0, 0.0, 100.0), NodeType::Junction);

        let roads = [
            (n0, n1), (n1, n2), (n1, n3)
        ];

        for (s, e) in roads {
            graph.add_edge(crate::simulation::network::graph::data::Edge {
                start_node: s,
                end_node: e,
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
                geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
                physical_geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
                zoning_left: false,
                zoning_right: false,
                deleted: false,
            });
        }
        graph.rebuild_adjacency_list();

        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);

        // Check if n0-n1 left sidewalk (fwd) has connections to other roads at n1
        let l_id = *lanes.edge_lanes[&0].iter().find(|&&id| {
            let l = &lanes.lanes[id];
            l.lane_idx == 100 && l.is_fwd
        }).expect("Edge 0 should have a forward left sidewalk");

        let next = &lanes.lanes[l_id].next_lanes;
        assert!(!next.is_empty(), "Sidewalk should have connections at junction");
        
        // At least one connection should be a Foot lane (crosswalk)
        let has_crosswalk = next.iter().any(|&nid| lanes.lanes[nid].edge_id == usize::MAX); // edge_id == usize::MAX for connections
        assert!(has_crosswalk, "Should have a crosswalk connection at junction");
    }

    #[test]
    fn test_rht_lane_offsets() {
        let mut graph = RegionGraph::new();
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);

        graph.add_edge(Edge {
            start_node: n1,
            end_node: n2,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0, // asphalt
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
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });
        graph.rebuild_adjacency_list();

        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);

        // Forward lane (idx 0) should be on the RIGHT (Z > 0 in Godot with +X forward)
        let l_fwd = &lanes.lanes[0];
        assert!(l_fwd.is_fwd);
        assert!(l_fwd.geometry[0].z > 0.0, "Forward lane should be at positive Z (Right)");

        // Backward lane (idx -1) should be on the LEFT (Z < 0)
        let l_bkw = &lanes.lanes[1];
        assert!(!l_bkw.is_fwd);
        assert!(l_bkw.geometry[0].z < 0.0, "Backward lane should be at negative Z (Left of A->B, Right for the driver)");
        
        // Left Sidewalk (idx 100) should be at Negative Z
        let l_sidewalk_left = lanes.lanes.iter().find(|l| l.lane_idx == 100 && l.is_fwd).unwrap();
        assert!(l_sidewalk_left.geometry[0].z < l_bkw.geometry[0].z, "Left sidewalk should be further left than backward lane");

        // Right Sidewalk (idx -100) should be at Positive Z
        let l_sidewalk_right = lanes.lanes.iter().find(|l| l.lane_idx == -100 && l.is_fwd).unwrap();
        assert!(l_sidewalk_right.geometry[0].z > l_fwd.geometry[0].z, "Right sidewalk should be further right than forward lane");
    }

    #[test]
    fn test_crosswalk_counts() {
        let mut graph = RegionGraph::new();
        // n0 --- n1 --- n2 (T-Junction) --- n3
        //                |
        //                n4
        //                |
        //                n5 (4-way) --- n6
        //                |              |
        //                n7             n8
        let n0 = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(200.0, 0.0, 0.0), NodeType::Junction);
        let n4 = graph.add_node(Vector3::new(100.0, 0.0, 100.0), NodeType::Junction);
        let n5 = graph.add_node(Vector3::new(100.0, 0.0, 200.0), NodeType::Junction);
        let n6 = graph.add_node(Vector3::new(200.0, 0.0, 200.0), NodeType::Junction);
        let n7 = graph.add_node(Vector3::new(100.0, 0.0, 300.0), NodeType::Junction);
        let n8 = graph.add_node(Vector3::new(200.0, 0.0, 300.0), NodeType::Junction);
        let n9 = graph.add_node(Vector3::new(0.0, 0.0, 200.0), NodeType::Junction);

        let edges = [
            (n0, n1), (n1, n2), (n2, n3), (n2, n4), 
            (n4, n5), (n5, n6), (n5, n7), (n5, n9), (n6, n8)
        ];
        for (s, e) in edges {
            graph.add_edge(Edge {
                start_node: s,
                end_node: e,
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
                geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
                physical_geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
                zoning_left: false,
                zoning_right: false,
                deleted: false,
            });
        }
        graph.rebuild_adjacency_list();
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);

        // Helper to count crosswalks at a node
        fn count_crosswalks_at(lanes: &LaneSystem, graph: &RegionGraph, node_id: usize) -> usize {
            let mut inbound_lane_ids = Vec::new();
            for &e_idx in &graph.adjacency[node_id] {
                let edge = &graph.edges[e_idx];
                let is_end = edge.end_node as usize == node_id;
                let edge_lanes = &lanes.edge_lanes[&e_idx];
                for &lane_id in edge_lanes {
                    let l = &lanes.lanes[lane_id];
                    if l.lane_type == LaneType::Foot && l.is_fwd == is_end {
                        inbound_lane_ids.push(lane_id);
                    }
                }
            }
            
            let mut crosswalk_lanes = Vec::new();
            for &lid in &inbound_lane_ids {
                for &next_id in &lanes.lanes[lid].next_lanes {
                    let next_l = &lanes.lanes[next_id];
                    if next_l.is_crosswalk && next_l.lane_type == LaneType::Foot {
                        if !crosswalk_lanes.contains(&next_id) {
                            crosswalk_lanes.push(next_id);
                        }
                    }
                }
            }
            crosswalk_lanes.len() / 2 // Bidirectional = 2 lanes per crosswalk
        }

        assert_eq!(count_crosswalks_at(&lanes, &graph, n0 as usize), 1, "Dead end n0 should have 1 crosswalk");
        assert_eq!(count_crosswalks_at(&lanes, &graph, n1 as usize), 1, "Straight road n1 should have 1 crosswalk");
        assert_eq!(count_crosswalks_at(&lanes, &graph, n2 as usize), 3, "T-junction n2 should have 3 crosswalks");
        assert_eq!(count_crosswalks_at(&lanes, &graph, n5 as usize), 4, "4-way junction n5 should have 4 crosswalks");
    }

    #[test]
    fn test_vehicle_connections() {
        let mut graph = RegionGraph::new();
        let n_center = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n_north = graph.add_node(Vector3::new(0.0, 0.0, -100.0), NodeType::Junction);
        let n_east = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let n_south = graph.add_node(Vector3::new(0.0, 0.0, 100.0), NodeType::Junction);
        
        let road_params = (1, 1); // 2-lane road
        for &other in &[n_north, n_east, n_south] {
            graph.add_edge(Edge {
                start_node: n_center,
                end_node: other,
                primary_type: TransitType::Road,
                allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
                class: EdgeClass::Standard,
                width: 7.0,
                fwd_lanes: road_params.0,
                bkw_lanes: road_params.1,
                speed_limit: 50.0,
                base_cost: 1.0,
                physical_length: 100.0,
                current_congestion: 0.0,
                start_clip: 0.0,
                end_clip: 0.0,
                geometry: vec![graph.nodes[n_center as usize].pos, graph.nodes[other as usize].pos],
                physical_geometry: vec![graph.nodes[n_center as usize].pos, graph.nodes[other as usize].pos],
                zoning_left: false,
                zoning_right: false,
                deleted: false,
            });
        }
        graph.rebuild_adjacency_list();
        
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        
        // Node n_center should have 3 incoming vehicle lanes (one from each arm)
        // Each incoming lane should connect to 2 outgoing lanes (the other two arms)
        let node = &graph.nodes[n_center as usize];
        assert!(!node.lane_connections.is_empty(), "Node should have lane connections");
        
        for (&(e_in, l_in), targets) in &node.lane_connections {
            // Only check vehicle lanes (idx 0 for 1-lane roads)
            if l_in.abs() < 10 {
                 // Should connect to 2 other edges
                 let unique_edges: std::collections::HashSet<usize> = targets.iter().map(|(e, _)| *e).collect();
                 assert_eq!(unique_edges.len(), 2, "Vehicle lane on edge {} should connect to 2 other arms at T-junction", e_in);
            }
        }
    }

    #[test]
    fn test_building_frontage_no_crosswalks() {
        let mut graph = RegionGraph::new();
        // n0 ---- n1 (Frontage Node) ---- n2
        let n0 = graph.add_node(Vector3::new(-50.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Frontage);
        let n2 = graph.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);

        let edges = [(n0, n1), (n1, n2)];
        for (s, e) in edges {
            graph.add_edge(Edge {
                start_node: s,
                end_node: e,
                primary_type: TransitType::Road,
                allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
                class: EdgeClass::Standard,
                width: 7.0,
                fwd_lanes: 1,
                bkw_lanes: 1,
                speed_limit: 50.0,
                base_cost: 1.0,
                physical_length: 50.0,
                current_congestion: 0.0,
                start_clip: 0.0,
                end_clip: 0.0,
                geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
                physical_geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
                zoning_left: false,
                zoning_right: false,
                deleted: false,
            });
        }
        graph.rebuild_adjacency_list();
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);

        // Scan for crosswalks at node n1
        let mut has_crosswalk = false;
        
        // Find all inbound lanes to node n1
        for &e_idx in &graph.adjacency[n1 as usize] {
            let edge = &graph.edges[e_idx];
            let is_end = edge.end_node == n1;
            
            if let Some(lane_ids) = lanes.edge_lanes.get(&e_idx) {
                for &l_id in lane_ids {
                    let l = &lanes.lanes[l_id];
                    // If this lane enters node n1
                    if l.is_fwd == is_end {
                        // Check its next lanes for crosswalks
                        for &next_id in &l.next_lanes {
                            if lanes.lanes[next_id].is_crosswalk {
                                has_crosswalk = true;
                            }
                        }
                    }
                }
            }
        }

        assert!(!has_crosswalk, "Building frontage node should not have pedestrian crosswalks");
    }
}
