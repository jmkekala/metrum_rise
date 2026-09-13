// SPDX-License-Identifier: GPL-2.0-only

//! Full and local lane reconstruction with deterministic indexed publication.

use super::super::graph::{Edge, RegionGraph};
use super::super::types::{TransitFlags, TransitType};
use super::geometry::build_one_lane;
use super::pedestrian_junctions::build_pedestrian_connections_at_node;
use super::vehicle_junctions::build_vehicle_connections_at_node;
use super::{LaneSystem, LaneType};
use crate::config;
use std::collections::{HashMap, HashSet};

impl LaneSystem {
    /// Completely rebuilds all physical lane geometry and connection splines for the entire graph.
    /// To be called after the road network topology and physical geometries have been updated.
    pub fn rebuild(&mut self, graph: &mut RegionGraph) {
        self.clear();

        // Maps (edge_id, is_fwd, lane_idx) -> lane_index in self.lanes
        let mut lane_map: HashMap<(usize, bool, i8), usize> = HashMap::new();

        // Append in graph order because lane IDs are referenced by agents and connectors.
        for (edge_idx, edge) in graph.edges().iter().enumerate() {
            self.append_edge_lanes(&mut lane_map, edge_idx, edge);
        }

        // Build node connections after all physical lanes exist.
        for node_id in 0..graph.node_count() {
            self.append_node_connections(&lane_map, graph, node_id);
        }
    }

    fn append_node_connections(
        &mut self,
        lane_map: &HashMap<(usize, bool, i8), usize>,
        graph: &mut RegionGraph,
        node_id: usize,
    ) {
        build_vehicle_connections_at_node(
            &mut self.lanes,
            lane_map,
            graph,
            node_id,
            &mut self.node_lanes,
        );
        build_pedestrian_connections_at_node(
            &mut self.lanes,
            lane_map,
            graph,
            node_id,
            &mut self.node_lanes,
        );
    }

    fn append_edge_lanes(
        &mut self,
        lane_map: &mut HashMap<(usize, bool, i8), usize>,
        edge_idx: usize,
        edge: &Edge,
    ) {
        if edge.deleted || edge.physical_geometry.len() < 2 {
            return;
        }

        let mut edge_lane_indices = Vec::new();

        let mut build_lane = |is_fwd: bool, l_idx: i8, l_type: LaneType, l_off: f32| {
            build_one_lane(
                &mut self.lanes,
                lane_map,
                &mut edge_lane_indices,
                edge_idx,
                edge,
                is_fwd,
                l_idx,
                l_type,
                l_off,
            );
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
            build_lane(false, -(l as i8) - 1, LaneType::Vehicle, lane_offset);
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

    /// Returns the physical-edge closure rebuilt by an incremental lane update.
    ///
    /// Every live edge incident to an endpoint of `affected_edges` is included because a changed
    /// junction clip moves the lane mouth on all of its road arms.
    pub(crate) fn incremental_rebuild_edge_closure(
        graph: &RegionGraph,
        affected_edges: &HashSet<usize>,
    ) -> HashSet<usize> {
        let mut affected_nodes = HashSet::new();
        for &edge_id in affected_edges {
            if edge_id >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_id);
            affected_nodes.insert(edge.start_node as usize);
            affected_nodes.insert(edge.end_node as usize);
        }

        let mut rebuild_set = affected_edges.clone();
        for node_id in affected_nodes {
            if node_id >= graph.node_adjacency_count() {
                continue;
            }
            for &edge_id in graph.node_adjacency(node_id as u32) {
                if !graph.edge(edge_id).deleted {
                    rebuild_set.insert(edge_id);
                }
            }
        }
        rebuild_set
    }

    /// Incrementally rebuilds changed physical lanes and every connection lane touching them.
    ///
    /// Existing adjacency bounds discovery and lane-map work to local incident lanes. Sorting
    /// affected edges/nodes costs O(K log K); geometry and connector work remains per owner.
    /// Publication is ordered because appended lane IDs are part of simulation state.
    pub fn rebuild_edges_incremental(
        &mut self,
        graph: &mut RegionGraph,
        affected_edges: &HashSet<usize>,
    ) {
        if affected_edges.is_empty() {
            return;
        }

        // Expand to every road arm whose lane mouth can move with an affected junction.
        let mut rebuild_edges: Vec<_> =
            Self::incremental_rebuild_edge_closure(graph, affected_edges)
                .into_iter()
                .collect();
        rebuild_edges.sort_unstable();

        // Connection lanes must be rebuilt at both ends of every rebuilt edge. Restricting this
        // to the original dirty endpoints leaves the far end pointing at orphaned physical lanes.
        let mut affected_nodes: HashSet<usize> = HashSet::new();
        for &edge_id in &rebuild_edges {
            if edge_id < graph.edge_count() {
                let edge = graph.edge(edge_id);
                affected_nodes.insert(edge.start_node as usize);
                affected_nodes.insert(edge.end_node as usize);
            }
        }

        let mut affected_nodes: Vec<_> = affected_nodes.into_iter().collect();
        affected_nodes.sort_unstable();

        // Retire old physical-lane ownership before appending replacements.
        for &e_id in &rebuild_edges {
            self.edge_lanes.remove(&e_id);
        }

        // Only incident surviving lanes are needed by the local connection builders. Clear
        // arrivals at rebuilt nodes; the opposite direction still owns its remote connections.
        let mut lane_map = HashMap::new();
        for &node_id in &affected_nodes {
            if node_id >= graph.node_adjacency_count() {
                continue;
            }
            for &edge_idx in graph.node_adjacency(node_id as u32) {
                let edge = graph.edge(edge_idx);
                if let Some(lane_ids) = self.edge_lanes.get(&edge_idx) {
                    for &lane_id in lane_ids {
                        let lane = &mut self.lanes[lane_id];
                        let terminal = if lane.is_fwd {
                            edge.end_node
                        } else {
                            edge.start_node
                        };
                        if terminal as usize == node_id {
                            lane.next_lanes.clear();
                        }
                        lane_map.insert((edge_idx, lane.is_fwd, lane.lane_idx), lane_id);
                    }
                }
            }
        }

        for &edge_idx in &rebuild_edges {
            if let Some(edge) = graph.edges().get(edge_idx) {
                self.append_edge_lanes(&mut lane_map, edge_idx, edge);
            }
        }

        // Rebuild connections in stable node order.
        for &node_id in &affected_nodes {
            if node_id < graph.node_count() {
                // Tombstone old connection lanes at this node so the renderer skips them.
                if let Some(old_ids) = self.node_lanes.remove(&node_id) {
                    for lid in old_ids {
                        if lid < self.lanes.len() {
                            self.lanes[lid].crosswalk_edge_id = None;
                            self.lanes[lid].crosswalk_marking = None;
                            self.lanes[lid].geometry.clear();
                            self.lanes[lid].next_lanes.clear();
                        }
                    }
                }
                self.append_node_connections(&lane_map, graph, node_id);
            }
        }
    }
}
