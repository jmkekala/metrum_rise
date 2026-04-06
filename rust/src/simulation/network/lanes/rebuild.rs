use std::collections::{HashMap, HashSet};
use super::super::graph::RegionGraph;
use super::super::types::{TransitFlags, TransitType};
use super::{LaneSystem, LaneType};
use super::geometry::{build_one_lane};
use super::vehicle_junctions::build_vehicle_connections_at_node;
use super::pedestrian_junctions::build_pedestrian_connections_at_node;
use crate::config;

impl LaneSystem {
    /// Completely rebuilds all physical lane geometry and connection splines for the entire graph.
    /// To be called after the road network topology and physical geometries have been updated.
    pub fn rebuild(&mut self, graph: &mut RegionGraph) {
        self.clear();
        
        // Maps (edge_id, is_fwd, lane_idx) -> lane_index in self.lanes
        let mut lane_map: HashMap<(usize, bool, i8), usize> = HashMap::new();

        // 1. Build Straight Lanes for all active edges
        for (edge_idx, edge) in graph.edges().iter().enumerate() {
            if edge.deleted || edge.physical_geometry.len() < 2 {
                continue;
            }

            let mut edge_lane_indices = Vec::new();

            // Helper to build a lane (using the one from geometry module for consistency)
            let mut build_lane = |is_fwd: bool, l_idx: i8, l_type: LaneType, l_off: f32| {
                build_one_lane(&mut self.lanes, &mut lane_map, &mut edge_lane_indices, 
                    edge_idx, edge, is_fwd, l_idx, l_type, l_off);
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
        for node_id in 0..graph.node_count() {
            build_vehicle_connections_at_node(&mut self.lanes, &lane_map, graph, node_id, &mut self.node_lanes);
            build_pedestrian_connections_at_node(&mut self.lanes, &lane_map, graph, node_id, &mut self.node_lanes);
        }
    }

    /// Incrementally rebuilds lanes only for `affected_edges` and adjacent connection lanes.
    pub fn rebuild_edges_incremental(
        &mut self,
        graph: &mut RegionGraph,
        affected_edges: &HashSet<usize>,
    ) {
        if affected_edges.is_empty() { return; }

        // 1. Collect nodes at both ends of every affected edge.
        let mut affected_nodes: HashSet<usize> = HashSet::new();
        for &e_id in affected_edges {
            if e_id < graph.edge_count() && !graph.edge(e_id).deleted {
                let e = graph.edge(e_id);
                affected_nodes.insert(e.start_node as usize);
                affected_nodes.insert(e.end_node as usize);
            }
        }

        // 2. Expand: also rebuild edges incident to affected nodes.
        let mut rebuild_set: HashSet<usize> = affected_edges.clone();
        for &node_id in &affected_nodes {
            if node_id < graph.node_adjacency_count() {
                for &e_id in graph.node_adjacency(node_id as u32) {
                    if !graph.edge(e_id).deleted {
                        rebuild_set.insert(e_id);
                    }
                }
            }
        }

        // 3. Orphan old road lanes for every edge in rebuild_set.
        for &e_id in &rebuild_set {
            self.edge_lanes.remove(&e_id);
        }

        // 4. Clear next_lanes on non-orphaned lanes at affected nodes.
        for &node_id in &affected_nodes {
            if node_id >= graph.node_adjacency_count() { continue; }
            for &e_id in graph.node_adjacency(node_id as u32) {
                if let Some(lane_ids) = self.edge_lanes.get(&e_id) {
                    let ids: Vec<usize> = lane_ids.clone();
                    for lid in ids {
                        self.lanes[lid].next_lanes.clear();
                    }
                }
            }
        }

        let mut lane_map: HashMap<(usize, bool, i8), usize> = HashMap::new();

        // 6. Append new straight lanes for every edge in rebuild_set.
        for &edge_idx in &rebuild_set {
            if edge_idx >= graph.edge_count() { continue; }
            let edge = graph.edge(edge_idx);
            if edge.deleted || edge.physical_geometry.len() < 2 { continue; }

            let mut edge_lane_indices = Vec::new();
            let lane_w = config::LANE_WIDTH;
            let sidewalk_w = config::SIDEWALK_WIDTH;
            let asphalt_width = (edge.fwd_lanes + edge.bkw_lanes) as f32 * lane_w;
            let side_mul = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };

            for l in 0..edge.fwd_lanes {
                let off = (l as f32 + 0.5) * lane_w * side_mul;
                build_one_lane(&mut self.lanes, &mut lane_map, &mut edge_lane_indices,
                    edge_idx, edge, true, l as i8, LaneType::Vehicle, off);
            }
            for l in 0..edge.bkw_lanes {
                let off = -(l as f32 + 0.5) * lane_w * side_mul;
                build_one_lane(&mut self.lanes, &mut lane_map, &mut edge_lane_indices,
                    edge_idx, edge, false, -(l as i8) - 1, LaneType::Vehicle, off);
            }

            if (edge.allowed_types & TransitFlags::FOOT) != 0 {
                if edge.primary_type == TransitType::Foot {
                    build_one_lane(&mut self.lanes, &mut lane_map, &mut edge_lane_indices,
                        edge_idx, edge, true, 0, LaneType::Foot, 0.0);
                    build_one_lane(&mut self.lanes, &mut lane_map, &mut edge_lane_indices,
                        edge_idx, edge, false, 0, LaneType::Foot, 0.0);
                } else {
                    let left_off = -(asphalt_width * 0.5 + sidewalk_w * 0.5) * side_mul;
                    build_one_lane(&mut self.lanes, &mut lane_map, &mut edge_lane_indices,
                        edge_idx, edge, true, 100, LaneType::Foot, left_off);
                    build_one_lane(&mut self.lanes, &mut lane_map, &mut edge_lane_indices,
                        edge_idx, edge, false, 100, LaneType::Foot, left_off);
                    let right_off = (asphalt_width * 0.5 + sidewalk_w * 0.5) * side_mul;
                    build_one_lane(&mut self.lanes, &mut lane_map, &mut edge_lane_indices,
                        edge_idx, edge, true, -100, LaneType::Foot, right_off);
                    build_one_lane(&mut self.lanes, &mut lane_map, &mut edge_lane_indices,
                        edge_idx, edge, false, -100, LaneType::Foot, right_off);
                }
            }
            self.edge_lanes.insert(edge_idx, edge_lane_indices);
        }

        // 7 & 8. Rebuild connections for every affected node.
        for &node_id in &affected_nodes {
            if node_id < graph.node_count() {
                // Tombstone old connection lanes at this node so the renderer skips them.
                if let Some(old_ids) = self.node_lanes.remove(&node_id) {
                    for lid in old_ids {
                        if lid < self.lanes.len() {
                            self.lanes[lid].is_crosswalk = false;
                            self.lanes[lid].geometry.clear();
                            self.lanes[lid].next_lanes.clear();
                        }
                    }
                }
                build_vehicle_connections_at_node(&mut self.lanes, &lane_map, graph, node_id, &mut self.node_lanes);
                build_pedestrian_connections_at_node(&mut self.lanes, &lane_map, graph, node_id, &mut self.node_lanes);
            }
        }
    }
}
