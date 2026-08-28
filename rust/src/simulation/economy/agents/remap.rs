// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: remap.rs
//  script_path: rust/src/simulation/economy/agents/remap.rs
//  module_name: remap
//  version: 0.1.0
//  description: Repairs agent-held indices after the graph, lanes
//  kind: module
//  spec: none
//  internal_dependencies: [graph, lanes, allocator]
//  external_dependencies: [godot]
//  features: [index-remap, compaction, lane-reattach, agent-repair]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// ========================================================================

//! Agent index repair after graph, lane, building, and household remaps.

use super::data::AgentSystem;
use super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, MODE_WALK, TRANSIT_IN_BUILDING,
    TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::{Lane, LaneSystem, LaneType};
use godot::prelude::Vector2;
use std::collections::{HashMap, HashSet};

// ========================================================================
// REATTACH REACH
// ========================================================================

const LANE_REATTACH_MAX_DIST_M: f32 = 30.0;

// ========================================================================
// SURVIVING A ROAD EDIT
// ========================================================================

impl AgentSystem {
    /// Remaps the edge indices stored in all agents from old IDs to new IDs.
    pub fn update_edge_indices(&mut self, mapping: &HashMap<usize, usize>) {
        for i in 0..self.agents.len() {
            if self.agents.current_edge[i] != usize::MAX {
                if let Some(&new_id) = mapping.get(&self.agents.current_edge[i]) {
                    self.agents.current_edge[i] = new_id;
                    self.agents.current_path[i].clear();
                    self.agents.network_replan_failures[i] = 0;
                } else {
                    self.agents.current_edge[i] = usize::MAX;
                    self.agents.current_path[i].clear();
                    self.agents.network_replan_failures[i] = 0;
                }
            }
        }
        self.invalidate_lane_bucket_snapshot();
    }

    /// Remaps household indices after a `swap_remove` in `HouseholdSystem`. O(A).
    pub fn remap_household_indices(&mut self, mapping: &HashMap<usize, usize>) {
        if mapping.is_empty() {
            return;
        }
        for i in 0..self.agents.len() {
            if let Some(&new_id) = mapping.get(&self.agents.household_id[i]) {
                self.agents.household_id[i] = new_id;
            }
        }
    }

    /// Remaps building indices after a `swap_remove` in `BuildingAllocator`. O(A).
    pub fn remap_building_indices(&mut self, mapping: &HashMap<usize, usize>) {
        if mapping.is_empty() {
            return;
        }
        for i in 0..self.agents.len() {
            remap_usize(&mut self.agents.home_building[i], mapping);
            remap_usize(&mut self.agents.work_building[i], mapping);
            remap_usize(&mut self.agents.current_building[i], mapping);
            remap_usize(&mut self.agents.target_building[i], mapping);
            remap_usize(&mut self.agents.planned_target_building[i], mapping);
            remap_usize(&mut self.agents.next_departure_origin_building[i], mapping);
            remap_usize(&mut self.agents.next_departure_target_building[i], mapping);
            remap_usize(&mut self.agents.cached_schedule_work_building[i], mapping);
        }
    }

    /// Sets `current_lane_id = usize::MAX` for every agent whose active lane belongs to the
    /// incremental rebuild closure of `affected_edges`, or whose connector leads into that closure.
    ///
    /// Must be called **before** `LaneSystem::rebuild_edges_incremental` so the old lane IDs
    /// are still valid for lookup — old orphaned lanes retain their original `edge_id` even
    /// after `rebuild_edges_incremental` removes them from `edge_lanes`.
    pub fn invalidate_lane_ids_for_edges(
        &mut self,
        affected_edges: &HashSet<usize>,
        lane_system: &LaneSystem,
        graph: &RegionGraph,
    ) {
        let input_edges = affected_edges;
        let affected_edges = LaneSystem::incremental_rebuild_edge_closure(graph, affected_edges);
        let affected_lane_ids = collect_lane_ids_for_edges(&affected_edges, lane_system);
        let traffic_debug = crate::debug::is_traffic_enabled();
        if traffic_debug {
            log_road_edit_affected(
                "lane-invalidate",
                input_edges,
                &affected_edges,
                &affected_lane_ids,
                graph,
            );
        }

        for i in 0..self.agents.len() {
            let lid = self.agents.current_lane_id[i];
            if lid == usize::MAX || lid >= lane_system.lanes.len() {
                continue;
            }
            let lane = &lane_system.lanes[lid];
            let Some(reattach_edge_id) = invalidated_lane_reattach_edge(
                lane,
                &affected_edges,
                &affected_lane_ids,
                lane_system,
            ) else {
                continue;
            };
            {
                let old_edge = self.agents.current_edge[i];
                let old_node = self.agents.current_node[i];
                let old_lane_d = self.agents.lane_distance[i];
                let old_pos_x = self.agents.pos_x[i];
                let old_pos_y = self.agents.pos_y[i];
                let sampled_pos = sample_lane_position_xz(lane, old_lane_d);
                if let Some(pos) = sampled_pos {
                    self.agents.pos_x[i] = pos.x;
                    self.agents.pos_y[i] = pos.y;
                }
                if traffic_debug {
                    let log_pos_x = sampled_pos.map(|pos| pos.x).unwrap_or(old_pos_x);
                    let log_pos_y = sampled_pos.map(|pos| pos.y).unwrap_or(old_pos_y);
                    crate::traffic_log!(
                        "[ROAD_EDIT_AGENT_INVALIDATE] agent={} transit={} mode={} old_edge={} old_node={} old_lane={} old_lane_d={:.2} reattach_edge={} pos=({:.2},{:.2})",
                        i,
                        self.agents.transit[i],
                        self.agents.transit_mode[i],
                        old_edge,
                        old_node,
                        lid,
                        old_lane_d,
                        reattach_edge_id,
                        log_pos_x,
                        log_pos_y,
                    );
                }
                self.agents.current_edge[i] = reattach_edge_id;
                self.agents.current_lane_id[i] = usize::MAX;
                self.agents.lane_change_from_lane_id[i] = u32::MAX;
                self.agents.lane_change_start_d[i] = 0.0;
                self.agents.lane_change_length_m[i] = 0.0;
                self.agents.overtake_blocked_time_s[i] = 0.0;
                self.agents.overtake_cooldown_s[i] = 0.0;
                // Preserve lane_distance so road edits do not visually snap agents to lane start
                // before the next tick can re-attach them.
            }
        }
        self.invalidate_lane_bucket_snapshot();
    }

    /// Reattaches invalidated on-road agents to rebuilt lanes near their preserved world position.
    ///
    /// Call this after rebuilding lane geometry for the same affected edge set passed to
    /// [`Self::invalidate_lane_ids_for_edges`]. Agents keep their physical progress along the road
    /// edit instead of falling back to a graph node start on the next movement tick.
    pub fn reattach_invalidated_lanes_for_edges(
        &mut self,
        affected_edges: &HashSet<usize>,
        lane_system: &LaneSystem,
        graph: &RegionGraph,
    ) {
        let input_edges = affected_edges;
        let affected_edges = LaneSystem::incremental_rebuild_edge_closure(graph, affected_edges);
        let affected_lane_ids = collect_lane_ids_for_edges(&affected_edges, lane_system);
        let affected_nodes = collect_nodes_for_edges(&affected_edges, graph);
        let max_dist_sq = LANE_REATTACH_MAX_DIST_M * LANE_REATTACH_MAX_DIST_M;
        let traffic_debug = crate::debug::is_traffic_enabled();
        if traffic_debug {
            log_road_edit_affected(
                "lane-reattach",
                input_edges,
                &affected_edges,
                &affected_lane_ids,
                graph,
            );
        }
        for i in 0..self.agents.len() {
            if self.agents.current_lane_id[i] != usize::MAX
                || !matches!(
                    self.agents.transit[i],
                    TRANSIT_NETWORK | TRANSIT_INTERSECTION
                )
                || !affected_edges.contains(&self.agents.current_edge[i])
            {
                continue;
            }

            let desired_lane_type = if self.agents.transit_mode[i] == MODE_WALK {
                LaneType::Foot
            } else {
                LaneType::Vehicle
            };
            let old_edge = self.agents.current_edge[i];
            let old_node = self.agents.current_node[i];
            let old_lane_d = self.agents.lane_distance[i];
            let old_pos_x = self.agents.pos_x[i];
            let old_pos_y = self.agents.pos_y[i];
            let agent_pos = Vector2::new(self.agents.pos_x[i], self.agents.pos_y[i]);
            let Some(candidate) = best_reattach_lane(
                agent_pos,
                desired_lane_type,
                &affected_edges,
                lane_system,
                graph,
                max_dist_sq,
            ) else {
                if traffic_debug {
                    crate::traffic_log!(
                        "[ROAD_EDIT_AGENT_REATTACH] agent={} action=no-candidate transit={} mode={} old_edge={} old_node={} old_lane_d={:.2} pos=({:.2},{:.2}) max_dist_m={:.2}",
                        i,
                        self.agents.transit[i],
                        self.agents.transit_mode[i],
                        old_edge,
                        old_node,
                        old_lane_d,
                        old_pos_x,
                        old_pos_y,
                        LANE_REATTACH_MAX_DIST_M,
                    );
                }
                continue;
            };

            if traffic_debug {
                crate::traffic_log!(
                    "[ROAD_EDIT_AGENT_REATTACH] agent={} action=reattach transit={} mode={} old_edge={} old_node={} old_lane_d={:.2} old_pos=({:.2},{:.2}) new_edge={} new_node={} new_lane={} new_lane_d={:.2} snap_m={:.2} new_pos=({:.2},{:.2})",
                    i,
                    self.agents.transit[i],
                    self.agents.transit_mode[i],
                    old_edge,
                    old_node,
                    old_lane_d,
                    old_pos_x,
                    old_pos_y,
                    candidate.edge_id,
                    candidate.origin_node,
                    candidate.lane_id,
                    candidate.lane_d,
                    candidate.dist_sq.sqrt(),
                    candidate.pos.x,
                    candidate.pos.y,
                );
            }
            self.agents.current_lane_id[i] = candidate.lane_id;
            self.agents.current_edge[i] = candidate.edge_id;
            self.agents.current_node[i] = candidate.origin_node;
            self.agents.lane_distance[i] = candidate.lane_d;
            self.agents.pos_x[i] = candidate.pos.x;
            self.agents.pos_y[i] = candidate.pos.y;
            self.agents.current_path[i].clear();
            self.agents.current_path_index[i] = 0;
            self.agents.network_replan_failures[i] = 0;
            self.agents.transit[i] = TRANSIT_NETWORK;
            self.agents.lane_change_from_lane_id[i] = u32::MAX;
            self.agents.lane_change_start_d[i] = 0.0;
            self.agents.lane_change_length_m[i] = 0.0;
            self.agents.overtake_blocked_time_s[i] = 0.0;
            self.agents.overtake_cooldown_s[i] = 0.0;
        }

        for i in 0..self.agents.len() {
            if self.agents.transit[i] == TRANSIT_IN_BUILDING {
                continue;
            }
            if agent_route_state_touches_road_edit(
                self,
                i,
                &affected_edges,
                &affected_nodes,
                &affected_lane_ids,
                lane_system,
            ) {
                if traffic_debug {
                    log_agent_route_clear_after_road_edit(self, i);
                }
                self.clear_route_plan_after_road_edit(i, lane_system);
            }
        }
        self.invalidate_lane_bucket_snapshot();
    }

    fn clear_route_plan_after_road_edit(&mut self, i: usize, lane_system: &LaneSystem) {
        let freight_border_node = self.agents.freight_target_border_node[i];
        let preserve_freight_border = freight_border_node != u32::MAX
            && (self.agents.access_flags[i] & ACCESS_FREIGHT_BORDER_DESTINATION) != 0;
        let should_drop_intersection_lane = self.agents.transit[i] == TRANSIT_INTERSECTION
            && match lane_system.lanes.get(self.agents.current_lane_id[i]) {
                Some(lane) => lane.edge_id == usize::MAX,
                None => true,
            };

        self.agents.current_path[i].clear();
        self.agents.current_path_index[i] = 0;
        if should_drop_intersection_lane {
            self.agents.transit[i] = TRANSIT_NETWORK;
            self.agents.current_edge[i] = usize::MAX;
            self.agents.current_lane_id[i] = usize::MAX;
            self.agents.lane_distance[i] = 0.0;
            self.agents.speed[i] = 0.0;
        }
        self.agents.planned_attach_node[i] = u32::MAX;
        self.agents.planned_attach_lane_id[i] = u32::MAX;
        self.agents.planned_attach_lane_d[i] = 0.0;
        self.agents.planned_detach_lane_d[i] = 0.0;
        if preserve_freight_border {
            self.agents.planned_detach_node[i] = freight_border_node;
            self.agents.planned_detach_lane_id[i] = u32::MAX;
            self.agents.access_flags[i] = ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION;
        } else {
            self.agents.planned_detach_node[i] = u32::MAX;
            self.agents.planned_detach_lane_id[i] = u32::MAX;
            self.agents.access_flags[i] = 0;
        }
        self.agents.next_replan_time[i] = 0.0;
        self.agents.network_replan_failures[i] = 0;
        self.agents.lane_change_from_lane_id[i] = u32::MAX;
        self.agents.lane_change_start_d[i] = 0.0;
        self.agents.lane_change_length_m[i] = 0.0;
        self.agents.overtake_blocked_time_s[i] = 0.0;
        self.agents.overtake_cooldown_s[i] = 0.0;
    }
}

// ========================================================================
// WHAT THE EDIT TOUCHED
// ========================================================================

fn collect_lane_ids_for_edges(
    affected_edges: &HashSet<usize>,
    lane_system: &LaneSystem,
) -> HashSet<usize> {
    let mut affected_lane_ids = HashSet::new();
    for &edge_id in affected_edges {
        if let Some(lane_ids) = lane_system.edge_lanes.get(&edge_id) {
            affected_lane_ids.extend(lane_ids);
        }
    }
    affected_lane_ids
}

fn collect_nodes_for_edges(affected_edges: &HashSet<usize>, graph: &RegionGraph) -> HashSet<u32> {
    let mut affected_nodes = HashSet::new();
    for &edge_id in affected_edges {
        let Some(edge) = graph.get_edge(edge_id) else {
            continue;
        };
        affected_nodes.insert(graph.get_valid_node(edge.start_node));
        affected_nodes.insert(graph.get_valid_node(edge.end_node));
    }
    affected_nodes
}

// ========================================================================
// LOGGING
// ========================================================================

fn log_road_edit_affected(
    phase: &str,
    input_edges: &HashSet<usize>,
    closure_edges: &HashSet<usize>,
    affected_lane_ids: &HashSet<usize>,
    graph: &RegionGraph,
) {
    let affected_nodes = collect_nodes_for_edges(closure_edges, graph);
    crate::traffic_log!(
        "[ROAD_EDIT_AFFECTED] phase={} input_edges={:?} closure_edges={:?} affected_nodes={:?} affected_lane_count={}",
        phase,
        sorted_usize_ids(input_edges),
        sorted_usize_ids(closure_edges),
        sorted_u32_ids(&affected_nodes),
        affected_lane_ids.len(),
    );
}

fn log_agent_route_clear_after_road_edit(agents: &AgentSystem, i: usize) {
    crate::traffic_log!(
        "[ROAD_EDIT_ROUTE_CLEAR] agent={} transit={} mode={} current_edge={} current_node={} current_lane={} target_bldg={} planned_attach_node={} planned_detach_node={} planned_attach_lane={} planned_detach_lane={} path_len={} path_idx={} flags_before=0x{:02x}",
        i,
        agents.agents.transit[i],
        agents.agents.transit_mode[i],
        agents.agents.current_edge[i],
        agents.agents.current_node[i],
        agents.agents.current_lane_id[i],
        agents.agents.target_building[i],
        agents.agents.planned_attach_node[i],
        agents.agents.planned_detach_node[i],
        agents.agents.planned_attach_lane_id[i],
        agents.agents.planned_detach_lane_id[i],
        agents.agents.current_path[i].len(),
        agents.agents.current_path_index[i],
        agents.agents.access_flags[i],
    );
}

fn sorted_usize_ids(values: &HashSet<usize>) -> Vec<usize> {
    let mut ids: Vec<usize> = values.iter().copied().collect();
    ids.sort_unstable();
    ids
}

fn sorted_u32_ids(values: &HashSet<u32>) -> Vec<u32> {
    let mut ids: Vec<u32> = values.iter().copied().collect();
    ids.sort_unstable();
    ids
}

// ========================================================================
// WHO IS AFFECTED
// ========================================================================

fn agent_route_state_touches_road_edit(
    agents: &AgentSystem,
    i: usize,
    affected_edges: &HashSet<usize>,
    affected_nodes: &HashSet<u32>,
    affected_lane_ids: &HashSet<usize>,
    lane_system: &LaneSystem,
) -> bool {
    let current_edge = agents.agents.current_edge[i];
    if current_edge != usize::MAX && affected_edges.contains(&current_edge) {
        return true;
    }
    let current_node = agents.agents.current_node[i];
    if current_node != u32::MAX && affected_nodes.contains(&current_node) {
        return true;
    }
    if lane_usize_touches_road_edit(
        agents.agents.current_lane_id[i],
        affected_edges,
        affected_lane_ids,
        lane_system,
    ) {
        return true;
    }
    if lane_u32_touches_road_edit(
        agents.agents.planned_attach_lane_id[i],
        affected_edges,
        affected_lane_ids,
        lane_system,
    ) || lane_u32_touches_road_edit(
        agents.agents.planned_detach_lane_id[i],
        affected_edges,
        affected_lane_ids,
        lane_system,
    ) {
        return true;
    }

    let attach_node = agents.agents.planned_attach_node[i];
    if attach_node != u32::MAX && affected_nodes.contains(&attach_node) {
        return true;
    }
    let detach_node = agents.agents.planned_detach_node[i];
    if detach_node != u32::MAX && affected_nodes.contains(&detach_node) {
        return true;
    }

    agents.agents.current_path[i]
        .iter()
        .any(|node| affected_nodes.contains(node))
}

fn lane_u32_touches_road_edit(
    lane_id: u32,
    affected_edges: &HashSet<usize>,
    affected_lane_ids: &HashSet<usize>,
    lane_system: &LaneSystem,
) -> bool {
    if lane_id == u32::MAX {
        return false;
    }
    lane_usize_touches_road_edit(
        lane_id as usize,
        affected_edges,
        affected_lane_ids,
        lane_system,
    )
}

fn lane_usize_touches_road_edit(
    lane_id: usize,
    affected_edges: &HashSet<usize>,
    affected_lane_ids: &HashSet<usize>,
    lane_system: &LaneSystem,
) -> bool {
    if lane_id == usize::MAX || affected_lane_ids.contains(&lane_id) {
        return lane_id != usize::MAX;
    }
    lane_system.lanes.get(lane_id).is_some_and(|lane| {
        (lane.edge_id != usize::MAX && affected_edges.contains(&lane.edge_id))
            || (lane.edge_id == usize::MAX
                && lane
                    .next_lanes
                    .iter()
                    .any(|next_lane_id| affected_lane_ids.contains(next_lane_id)))
    })
}

fn invalidated_lane_reattach_edge(
    lane: &Lane,
    affected_edges: &HashSet<usize>,
    affected_lane_ids: &HashSet<usize>,
    lane_system: &LaneSystem,
) -> Option<usize> {
    if lane.edge_id != usize::MAX {
        return affected_edges
            .contains(&lane.edge_id)
            .then_some(lane.edge_id);
    }

    lane.next_lanes.iter().copied().find_map(|next_lane_id| {
        if !affected_lane_ids.contains(&next_lane_id) {
            return None;
        }
        lane_system.lanes.get(next_lane_id).and_then(|target_lane| {
            (target_lane.edge_id != usize::MAX).then_some(target_lane.edge_id)
        })
    })
}

fn remap_usize(value: &mut usize, mapping: &HashMap<usize, usize>) {
    if let Some(&new_id) = mapping.get(value) {
        *value = new_id;
    }
}

struct LaneReattachCandidate {
    lane_id: usize,
    edge_id: usize,
    origin_node: u32,
    lane_d: f32,
    pos: Vector2,
    dist_sq: f32,
}

// ========================================================================
// FINDING A NEW LANE
// ========================================================================

fn best_reattach_lane(
    agent_pos: Vector2,
    desired_lane_type: LaneType,
    affected_edges: &HashSet<usize>,
    lane_system: &LaneSystem,
    graph: &RegionGraph,
    max_dist_sq: f32,
) -> Option<LaneReattachCandidate> {
    let mut best: Option<LaneReattachCandidate> = None;
    for &edge_id in affected_edges {
        if graph
            .get_edge(edge_id)
            .is_none_or(|edge| edge.deleted || edge.physical_geometry.len() < 2)
        {
            continue;
        }
        let Some(lane_ids) = lane_system.edge_lanes.get(&edge_id) else {
            continue;
        };
        for &lane_id in lane_ids {
            let Some(lane) = lane_system.lanes.get(lane_id) else {
                continue;
            };
            if lane.edge_id != edge_id || lane.lane_type != desired_lane_type {
                continue;
            }
            let Some(origin_node) = lane_origin_node_for(lane, graph) else {
                continue;
            };
            let Some((lane_d, pos, dist_sq)) = project_point_to_lane(agent_pos, lane) else {
                continue;
            };
            if dist_sq > max_dist_sq {
                continue;
            }
            let replace = match &best {
                None => true,
                Some(best) => {
                    dist_sq < best.dist_sq - 1e-4
                        || ((dist_sq - best.dist_sq).abs() <= 1e-4 && lane_id < best.lane_id)
                }
            };
            if replace {
                best = Some(LaneReattachCandidate {
                    lane_id,
                    edge_id,
                    origin_node,
                    lane_d,
                    pos,
                    dist_sq,
                });
            }
        }
    }
    best
}

fn lane_origin_node_for(lane: &Lane, graph: &RegionGraph) -> Option<u32> {
    let edge = graph.get_edge(lane.edge_id)?;
    Some(if lane.is_fwd {
        edge.start_node
    } else {
        edge.end_node
    })
}

fn sample_lane_position_xz(lane: &Lane, lane_d: f32) -> Option<Vector2> {
    if lane.geometry.len() < 2 {
        return None;
    }
    Some(BuildingAllocator::sample_pos_on_lane(lane, lane_d))
}

fn project_point_to_lane(point: Vector2, lane: &Lane) -> Option<(f32, Vector2, f32)> {
    if lane.geometry.len() < 2 {
        return None;
    }

    let lane_d = BuildingAllocator::project_point_to_polyline_s(&lane.geometry, point)
        .clamp(0.0, lane.length.max(0.0));
    let pos = BuildingAllocator::sample_pos_on_lane(lane, lane_d);
    let dist_sq = point.distance_squared_to(pos);
    Some((lane_d, pos, dist_sq))
}

// ========================================================================
// TESTS
// ========================================================================

#[cfg(test)]
mod tests {
    use super::super::data::{Agent, AgentSystem};
    use super::super::{
        ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PATH_FROM_FLOW_FIELD, ACCESS_PLAN_VALID,
        ACCESS_ZERO_HOP_NODE_PATH, AGE_ADULT, MODE_CAR, TRANSIT_IN_BUILDING, TRANSIT_INTERSECTION,
        TRANSIT_NETWORK,
    };
    use super::*;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::graph::data::Edge;
    use crate::simulation::network::lanes::LaneSystem;
    use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};
    use godot::prelude::Vector3;

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
            lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
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
            lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
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

    struct TurnConnectorFixture {
        graph: RegionGraph,
        lane_system: LaneSystem,
        n_west: u32,
        n_center: u32,
        n_north: u32,
        inbound_lane: usize,
        connector_lane: usize,
    }

    fn make_turn_connector_lane_system() -> TurnConnectorFixture {
        let mut graph = RegionGraph::new();
        let n_west = graph.add_node(
            Vector3::new(-100.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n_center = graph.add_node(
            Vector3::ZERO,
            crate::simulation::network::types::NodeType::Junction,
        );
        let n_east = graph.add_node(
            Vector3::new(100.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n_north = graph.add_node(
            Vector3::new(0.0, 0.0, -100.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let add_edge = |graph: &mut RegionGraph, start_node: u32, end_node: u32| {
            let start = graph.node(start_node).pos;
            let end = graph.node(end_node).pos;
            graph.add_edge(Edge {
                start_node,
                end_node,
                primary_type: TransitType::Road,
                allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
                class: EdgeClass::Standard,
                width: 7.0,
                lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
                speed_limit: 50.0,
                base_cost: 1.0,
                physical_length: start.distance_to(end),
                current_congestion: 0.0,
                start_clip: 0.0,
                end_clip: 0.0,
                geometry: vec![start, end],
                physical_geometry: vec![start, end],
                ..Default::default()
            })
        };
        let west_edge = add_edge(&mut graph, n_west, n_center);
        let _east_edge = add_edge(&mut graph, n_center, n_east);
        let north_edge = add_edge(&mut graph, n_center, n_north);
        graph.rebuild_adjacency_list();

        let mut lane_system = LaneSystem::new();
        lane_system.rebuild(&mut graph);

        let inbound_lane = lane_system.edge_lanes[&west_edge]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd
            })
            .expect("west inbound vehicle lane");
        let connector_lane = lane_system.lanes[inbound_lane]
            .next_lanes
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.edge_id == usize::MAX
                    && lane.lane_type == LaneType::Vehicle
                    && lane.next_lanes.first().is_some_and(|&target_lane_id| {
                        lane_system
                            .lanes
                            .get(target_lane_id)
                            .is_some_and(|target_lane| target_lane.edge_id == north_edge)
                    })
            })
            .expect("west-to-north vehicle connector");

        TurnConnectorFixture {
            graph,
            lane_system,
            n_west,
            n_center,
            n_north,
            inbound_lane,
            connector_lane,
        }
    }

    #[test]
    fn remaps_cached_schedule_building_ids() {
        let mut sys = AgentSystem::new();
        let idx = sys.spawn_housed_agent(3, 0.0, 0.0);
        sys.agents.work_building[idx] = 5;
        sys.agents.current_building[idx] = 3;
        sys.agents.target_building[idx] = 5;
        sys.agents.planned_target_building[idx] = 5;
        sys.agents.next_departure_origin_building[idx] = 3;
        sys.agents.next_departure_target_building[idx] = 5;
        sys.agents.cached_schedule_work_building[idx] = 5;

        let mapping = HashMap::from([(3usize, 8usize), (5usize, 9usize)]);
        sys.remap_building_indices(&mapping);

        assert_eq!(sys.agents.home_building[idx], 8);
        assert_eq!(sys.agents.work_building[idx], 9);
        assert_eq!(sys.agents.current_building[idx], 8);
        assert_eq!(sys.agents.target_building[idx], 9);
        assert_eq!(sys.agents.planned_target_building[idx], 9);
        assert_eq!(sys.agents.next_departure_origin_building[idx], 8);
        assert_eq!(sys.agents.next_departure_target_building[idx], 9);
        assert_eq!(sys.agents.cached_schedule_work_building[idx], 9);
    }

    #[test]
    fn test_invalidate_clears_agents_on_incremental_rebuild_closure() {
        let (graph, lane_system) = make_simple_lane_system();
        let e0_lane = lane_system.edge_lanes[&0][0];
        let e1_lane = lane_system.edge_lanes[&1][0];

        let mut sys = AgentSystem::new();
        let render_id_0 = sys.allocate_render_id();
        sys.agents.push(Agent {
            home_building: usize::MAX,
            household_id: usize::MAX,
            age_group: AGE_ADULT,
            pending_household_size: 0,
            freight_shipment_id: u64::MAX,
            work_building: usize::MAX,
            pos_x: 0.0,
            pos_y: 0.0,
            render_id: render_id_0,
            activity: 0,
            transit: TRANSIT_NETWORK,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: 0.0,
            schedule_seed: 0,
            cached_commute_minutes: 0,
            next_commute_refresh_time: 0.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            current_building: usize::MAX,
            target_building: usize::MAX,
            planned_target_building: usize::MAX,
            freight_target_border_node: u32::MAX,
            current_node: 0,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            network_replan_failures: 0,
            current_edge: 0,
            current_lane_id: e0_lane,
            lane_distance: 10.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            junction_release_time_s: f32::MIN,
            next_reroute_time_s: 0.0,
            speed: 10.0,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: vec![],
            current_path_index: 0,
            has_car: true,
            vehicle_type: 0,
            pedestrian_type: 0,
            walk_phase: 0.0,
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        });
        let render_id_1 = sys.allocate_render_id();
        sys.agents.push(Agent {
            home_building: usize::MAX,
            household_id: usize::MAX,
            age_group: AGE_ADULT,
            pending_household_size: 0,
            freight_shipment_id: u64::MAX,
            work_building: usize::MAX,
            pos_x: 150.0,
            pos_y: 0.0,
            render_id: render_id_1,
            activity: 0,
            transit: TRANSIT_NETWORK,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: 0.0,
            schedule_seed: 1,
            cached_commute_minutes: 0,
            next_commute_refresh_time: 0.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            current_building: usize::MAX,
            target_building: usize::MAX,
            planned_target_building: usize::MAX,
            freight_target_border_node: u32::MAX,
            current_node: 1,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            network_replan_failures: 0,
            current_edge: 1,
            current_lane_id: e1_lane,
            lane_distance: 10.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            junction_release_time_s: f32::MIN,
            next_reroute_time_s: 0.0,
            speed: 10.0,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: vec![],
            current_path_index: 0,
            has_car: true,
            vehicle_type: 0,
            pedestrian_type: 0,
            walk_phase: 0.0,
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        });

        let mut affected = HashSet::new();
        affected.insert(0usize);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system, &graph);

        assert_eq!(sys.agents.current_lane_id[0], usize::MAX);
        assert_eq!(sys.agents.lane_distance[0], 10.0);
        assert_eq!(
            sys.agents.current_lane_id[1],
            usize::MAX,
            "the adjacent edge is physically rebuilt because it shares the changed junction"
        );
        assert_eq!(sys.agents.lane_distance[1], 10.0);
    }

    #[test]
    fn test_invalidate_skips_already_invalid_agents() {
        let (graph, lane_system) = make_simple_lane_system();

        let mut sys = AgentSystem::new();
        let render_id = sys.allocate_render_id();
        sys.agents.push(Agent {
            home_building: usize::MAX,
            household_id: usize::MAX,
            age_group: AGE_ADULT,
            pending_household_size: 0,
            freight_shipment_id: u64::MAX,
            work_building: usize::MAX,
            pos_x: 0.0,
            pos_y: 0.0,
            render_id,
            activity: 0,
            transit: TRANSIT_IN_BUILDING,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: 0.0,
            schedule_seed: 0,
            cached_commute_minutes: 0,
            next_commute_refresh_time: 0.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            current_building: 0,
            target_building: 0,
            planned_target_building: usize::MAX,
            freight_target_border_node: u32::MAX,
            current_node: 0,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            network_replan_failures: 0,
            current_edge: usize::MAX,
            current_lane_id: usize::MAX,
            lane_distance: 0.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            junction_release_time_s: f32::MIN,
            next_reroute_time_s: 0.0,
            speed: 0.0,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: vec![],
            current_path_index: 0,
            has_car: false,
            vehicle_type: 0,
            pedestrian_type: 0,
            walk_phase: 0.0,
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        });

        let mut affected = HashSet::new();
        affected.insert(0usize);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system, &graph);
        assert_eq!(sys.agents.current_lane_id[0], usize::MAX);
    }

    #[test]
    fn test_reattach_preserves_agent_position_after_edge_split() {
        let (mut graph, mut lane_system) = make_simple_lane_system();
        let old_lane_id = lane_system.edge_lanes[&0]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd
            })
            .expect("forward vehicle lane on edge 0");
        let old_lane_d = 75.0;
        let old_pos = sample_lane_position_xz(&lane_system.lanes[old_lane_id], old_lane_d).unwrap();
        let mut sys = AgentSystem::new();
        let idx = sys.spawn_border_arrival_agent(usize::MAX, 0, 0.0, 0.0, 0, old_pos.x, old_pos.y);
        sys.agents.transit[idx] = TRANSIT_NETWORK;
        sys.agents.current_node[idx] = 0;
        sys.agents.current_edge[idx] = 0;
        sys.agents.current_lane_id[idx] = old_lane_id;
        sys.agents.lane_distance[idx] = old_lane_d;
        sys.agents.speed[idx] = 10.0;
        sys.agents.transit_mode[idx] = MODE_CAR;

        let split_node = graph.add_node(
            Vector3::new(50.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let old_edge = graph.edge(0).clone();
        {
            let first = graph.edge_mut(0);
            first.end_node = split_node;
            first.geometry = vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, 0.0, 0.0)];
            first.physical_geometry = first.geometry.clone();
            first.physical_length = 50.0;
            first.base_cost = 0.5;
        }
        let mut second = old_edge;
        second.start_node = split_node;
        second.geometry = vec![Vector3::new(50.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)];
        second.physical_geometry = second.geometry.clone();
        second.physical_length = 50.0;
        second.base_cost = 0.5;
        let second_edge = graph.add_edge(second);
        graph.rebuild_adjacency_list();

        let affected = HashSet::from([0usize, second_edge]);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system, &graph);
        assert_eq!(sys.agents.current_lane_id[idx], usize::MAX);

        lane_system.rebuild_edges_incremental(&mut graph, &affected);
        sys.reattach_invalidated_lanes_for_edges(&affected, &lane_system, &graph);

        let new_lane_id = sys.agents.current_lane_id[idx];
        assert_ne!(new_lane_id, usize::MAX);
        let new_lane = &lane_system.lanes[new_lane_id];
        assert_eq!(new_lane.edge_id, second_edge);
        assert!(new_lane.is_fwd);
        assert_eq!(sys.agents.current_node[idx], split_node);
        assert!(
            (sys.agents.lane_distance[idx] - 25.0).abs() < 0.5,
            "expected preserved progress on the second split edge, got lane_d={:.2}",
            sys.agents.lane_distance[idx],
        );
        let new_pos = sample_lane_position_xz(new_lane, sys.agents.lane_distance[idx]).unwrap();
        assert!(
            old_pos.distance_to(new_pos) < 0.5,
            "reattach moved the car too far: old={old_pos:?} new={new_pos:?}",
        );
    }

    #[test]
    fn test_reattach_clears_stale_access_plan_after_road_edit() {
        let (graph, lane_system) = make_simple_lane_system();
        let current_lane = lane_system.edge_lanes[&0]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd
            })
            .expect("forward vehicle lane on edge 0");
        let detach_lane = lane_system.edge_lanes[&1]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd
            })
            .expect("forward vehicle lane on edge 1");

        let mut sys = AgentSystem::new();
        let idx = sys.spawn_border_arrival_agent(usize::MAX, 0, 0.0, 0.0, 0, 10.0, 0.0);
        sys.agents.transit[idx] = TRANSIT_NETWORK;
        sys.agents.transit_mode[idx] = MODE_CAR;
        sys.agents.current_node[idx] = 0;
        sys.agents.current_edge[idx] = 0;
        sys.agents.current_lane_id[idx] = current_lane;
        sys.agents.lane_distance[idx] = 10.0;
        sys.agents.target_building[idx] = 123;
        sys.agents.planned_attach_node[idx] = 1;
        sys.agents.planned_detach_node[idx] = 2;
        sys.agents.planned_attach_lane_id[idx] = current_lane as u32;
        sys.agents.planned_detach_lane_id[idx] = detach_lane as u32;
        sys.agents.planned_attach_lane_d[idx] = 10.0;
        sys.agents.planned_detach_lane_d[idx] = 20.0;
        sys.agents.access_flags[idx] =
            ACCESS_PLAN_VALID | ACCESS_ZERO_HOP_NODE_PATH | ACCESS_PATH_FROM_FLOW_FIELD;
        sys.agents.next_replan_time[idx] = 123.0;
        sys.agents.current_path[idx] = vec![0, 1, 2];
        sys.agents.current_path_index[idx] = 1;

        let affected = HashSet::from([0usize]);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system, &graph);
        sys.reattach_invalidated_lanes_for_edges(&affected, &lane_system, &graph);

        assert_ne!(sys.agents.current_lane_id[idx], usize::MAX);
        assert_eq!(sys.agents.transit[idx], TRANSIT_NETWORK);
        assert!(sys.agents.current_path[idx].is_empty());
        assert_eq!(sys.agents.current_path_index[idx], 0);
        assert_eq!(sys.agents.planned_attach_node[idx], u32::MAX);
        assert_eq!(sys.agents.planned_attach_lane_id[idx], u32::MAX);
        assert_eq!(sys.agents.planned_attach_lane_d[idx], 0.0);
        assert_eq!(sys.agents.planned_detach_node[idx], u32::MAX);
        assert_eq!(sys.agents.planned_detach_lane_id[idx], u32::MAX);
        assert_eq!(sys.agents.planned_detach_lane_d[idx], 0.0);
        assert_eq!(sys.agents.access_flags[idx], 0);
        assert_eq!(sys.agents.next_replan_time[idx], 0.0);
    }

    #[test]
    fn test_route_clear_drops_intersection_connector_lane_after_road_edit() {
        let TurnConnectorFixture {
            graph: _graph,
            lane_system,
            n_west,
            n_center,
            n_north,
            inbound_lane,
            connector_lane,
        } = make_turn_connector_lane_system();

        let mut sys = AgentSystem::new();
        let idx = sys.spawn_border_arrival_agent(usize::MAX, n_north, 0.0, 0.0, n_west, 100.0, 0.0);
        sys.agents.transit[idx] = TRANSIT_INTERSECTION;
        sys.agents.transit_mode[idx] = MODE_CAR;
        sys.agents.current_node[idx] = n_center;
        sys.agents.current_edge[idx] = usize::MAX;
        sys.agents.current_lane_id[idx] = connector_lane;
        sys.agents.lane_distance[idx] = lane_system.lanes[connector_lane].length * 0.5;
        sys.agents.speed[idx] = 3.5;
        sys.agents.target_building[idx] = 123;
        sys.agents.planned_attach_node[idx] = u32::MAX;
        sys.agents.planned_detach_node[idx] = n_center;
        sys.agents.planned_attach_lane_id[idx] = u32::MAX;
        sys.agents.planned_detach_lane_id[idx] = inbound_lane as u32;
        sys.agents.planned_detach_lane_d[idx] = 10.0;
        sys.agents.access_flags[idx] = ACCESS_PLAN_VALID | ACCESS_ZERO_HOP_NODE_PATH;
        sys.agents.next_replan_time[idx] = 77.0;
        sys.agents.network_replan_failures[idx] = 2;
        sys.agents.current_path[idx] = vec![n_center];
        sys.agents.current_path_index[idx] = 1;

        sys.clear_route_plan_after_road_edit(idx, &lane_system);

        assert_eq!(sys.agents.transit[idx], TRANSIT_NETWORK);
        assert_eq!(sys.agents.current_node[idx], n_center);
        assert_eq!(sys.agents.current_edge[idx], usize::MAX);
        assert_eq!(sys.agents.current_lane_id[idx], usize::MAX);
        assert_eq!(sys.agents.lane_distance[idx], 0.0);
        assert_eq!(sys.agents.speed[idx], 0.0);
        assert!(sys.agents.current_path[idx].is_empty());
        assert_eq!(sys.agents.current_path_index[idx], 0);
        assert_eq!(sys.agents.planned_attach_node[idx], u32::MAX);
        assert_eq!(sys.agents.planned_attach_lane_id[idx], u32::MAX);
        assert_eq!(sys.agents.planned_detach_node[idx], u32::MAX);
        assert_eq!(sys.agents.planned_detach_lane_id[idx], u32::MAX);
        assert_eq!(sys.agents.planned_detach_lane_d[idx], 0.0);
        assert_eq!(sys.agents.access_flags[idx], 0);
        assert_eq!(sys.agents.next_replan_time[idx], 0.0);
        assert_eq!(sys.agents.network_replan_failures[idx], 0);
    }

    #[test]
    fn test_reattach_preserves_freight_border_replan_marker_after_road_edit() {
        let (graph, lane_system) = make_simple_lane_system();
        let current_lane = lane_system.edge_lanes[&0]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd
            })
            .expect("forward vehicle lane on edge 0");
        let border_node = graph.edge(1).end_node;

        let mut sys = AgentSystem::new();
        let idx = sys.spawn_border_arrival_agent(usize::MAX, 0, 0.0, 0.0, 0, 10.0, 0.0);
        sys.agents.transit[idx] = TRANSIT_NETWORK;
        sys.agents.transit_mode[idx] = MODE_CAR;
        sys.agents.current_node[idx] = 0;
        sys.agents.current_edge[idx] = 0;
        sys.agents.current_lane_id[idx] = current_lane;
        sys.agents.lane_distance[idx] = 10.0;
        sys.agents.target_building[idx] = usize::MAX;
        sys.agents.freight_target_border_node[idx] = border_node;
        sys.agents.planned_detach_node[idx] = border_node;
        sys.agents.planned_detach_lane_id[idx] = u32::MAX;
        sys.agents.access_flags[idx] = ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION;
        sys.agents.next_replan_time[idx] = 123.0;
        sys.agents.current_path[idx] = vec![0, 1, border_node];
        sys.agents.current_path_index[idx] = 1;

        let affected = HashSet::from([0usize]);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system, &graph);
        sys.reattach_invalidated_lanes_for_edges(&affected, &lane_system, &graph);

        assert_ne!(sys.agents.current_lane_id[idx], usize::MAX);
        assert!(sys.agents.current_path[idx].is_empty());
        assert_eq!(sys.agents.current_path_index[idx], 0);
        assert_eq!(sys.agents.planned_attach_node[idx], u32::MAX);
        assert_eq!(sys.agents.planned_attach_lane_id[idx], u32::MAX);
        assert_eq!(sys.agents.planned_detach_node[idx], border_node);
        assert_eq!(sys.agents.planned_detach_lane_id[idx], u32::MAX);
        assert_eq!(
            sys.agents.access_flags[idx],
            ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION
        );
        assert_eq!(sys.agents.next_replan_time[idx], 0.0);
    }

    #[test]
    fn test_reattach_repairs_agent_invalidated_inside_connector() {
        let mut graph = RegionGraph::new();
        let n_west = graph.add_node(
            Vector3::new(-100.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n_center = graph.add_node(
            Vector3::ZERO,
            crate::simulation::network::types::NodeType::Junction,
        );
        let n_east = graph.add_node(
            Vector3::new(100.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n_north = graph.add_node(
            Vector3::new(0.0, 0.0, -100.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let add_edge = |graph: &mut RegionGraph, start_node: u32, end_node: u32| {
            let start = graph.node(start_node).pos;
            let end = graph.node(end_node).pos;
            graph.add_edge(Edge {
                start_node,
                end_node,
                primary_type: TransitType::Road,
                allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
                class: EdgeClass::Standard,
                width: 7.0,
                lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
                speed_limit: 50.0,
                base_cost: 1.0,
                physical_length: start.distance_to(end),
                current_congestion: 0.0,
                start_clip: 0.0,
                end_clip: 0.0,
                geometry: vec![start, end],
                physical_geometry: vec![start, end],
                ..Default::default()
            })
        };
        let west_edge = add_edge(&mut graph, n_west, n_center);
        let _east_edge = add_edge(&mut graph, n_center, n_east);
        let north_edge = add_edge(&mut graph, n_center, n_north);
        graph.rebuild_adjacency_list();

        let mut lane_system = LaneSystem::new();
        lane_system.rebuild(&mut graph);

        let inbound_lane = lane_system.edge_lanes[&west_edge]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd
            })
            .expect("west inbound vehicle lane");
        let connector_lane = lane_system.lanes[inbound_lane]
            .next_lanes
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.edge_id == usize::MAX
                    && lane.lane_type == LaneType::Vehicle
                    && lane.next_lanes.first().is_some_and(|&target_lane_id| {
                        lane_system
                            .lanes
                            .get(target_lane_id)
                            .is_some_and(|target_lane| target_lane.edge_id == north_edge)
                    })
            })
            .expect("west-to-north vehicle connector");
        let old_lane_d = lane_system.lanes[connector_lane].length * 0.5;
        let old_pos =
            sample_lane_position_xz(&lane_system.lanes[connector_lane], old_lane_d).unwrap();

        let mut sys = AgentSystem::new();
        let idx = sys.spawn_border_arrival_agent(
            usize::MAX,
            n_north,
            0.0,
            0.0,
            n_west,
            old_pos.x,
            old_pos.y,
        );
        sys.agents.transit[idx] = TRANSIT_INTERSECTION;
        sys.agents.transit_mode[idx] = MODE_CAR;
        sys.agents.current_node[idx] = n_center;
        sys.agents.current_edge[idx] = usize::MAX;
        sys.agents.current_lane_id[idx] = connector_lane;
        sys.agents.lane_distance[idx] = old_lane_d;
        sys.agents.speed[idx] = 6.0;

        let affected = HashSet::from([north_edge]);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system, &graph);
        assert_eq!(sys.agents.current_lane_id[idx], usize::MAX);
        assert_ne!(
            sys.agents.current_edge[idx],
            usize::MAX,
            "connector invalidation must preserve a physical edge for reattach"
        );

        lane_system.rebuild_edges_incremental(&mut graph, &affected);
        sys.reattach_invalidated_lanes_for_edges(&affected, &lane_system, &graph);

        let new_lane_id = sys.agents.current_lane_id[idx];
        assert_ne!(
            new_lane_id,
            usize::MAX,
            "agent invalidated inside a connector should reattach to a rebuilt physical lane"
        );
        assert_ne!(lane_system.lanes[new_lane_id].edge_id, usize::MAX);
        let new_pos = sample_lane_position_xz(
            &lane_system.lanes[new_lane_id],
            sys.agents.lane_distance[idx],
        )
        .unwrap();
        assert!(
            old_pos.distance_to(new_pos) <= LANE_REATTACH_MAX_DIST_M,
            "reattach moved connector agent too far: old={old_pos:?} new={new_pos:?}",
        );
    }
}
