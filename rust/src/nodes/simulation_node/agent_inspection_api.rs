// SPDX-License-Identifier: GPL-2.0-only

//! Agent inspector (`DEBUG-05`): pick an agent, read its details and its route.
//!
//! Called on a click and, while an agent is inspected, by the inspector overlay. The selection
//! lives in the agent system, which keeps its row valid through removals.

use super::*;
use crate::nodes::sim::render::agent_inspection::{
    inspection_details, pick_agent, ribbon, route_lines,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneType;
use crate::simulation::network::types::TransitFlags;

/// How far `spawn_debug_car` looks for a road node around each end point.
const DEBUG_CAR_NODE_REACH_M: f32 = 50.0;

#[godot_api(secondary)]
impl SimulationNode {
    /// Inspects the agent on a road or junction lane nearest `pos` (XZ) within `radius_m` and
    /// returns its render ID. A miss returns `-1` and leaves the inspection as it was. Cost
    /// follows the roads and agents near `pos`, not the city.
    #[func]
    pub fn pick_debug_agent(&self, pos: Vector3, radius_m: f32) -> i64 {
        let mut core = self.lock_core();
        let Some(idx) = pick_agent(
            &core.region_graph,
            &core.transit_network.lane_system,
            &core.agents,
            pos,
            radius_m,
        ) else {
            return -1;
        };
        core.agents
            .inspect_agent(idx)
            .map_or(-1, |render_id| render_id as i64)
    }

    /// Ends the agent inspection.
    #[func]
    pub fn clear_agent_inspection(&self) {
        self.lock_core().agents.clear_inspected_agent();
    }

    /// The inspected agent's details: `render_id`, `mode`, `state`, `speed_mps`, `activity`,
    /// `freight`, the buildings `home`, `work`, `destination` and `inside` (each `id`, `kind`,
    /// `profile`, `position`, or empty), `lane`, `lane_edge`, `lane_distance_m`,
    /// `lane_length_m`, `path_index`, `path_length` and `position`, plus `drawn_position` when
    /// the agent is a car drawn this frame. While the simulation core is busy, only
    /// `drawn_position` (if any), so the camera follow does not skip frames during a long tick;
    /// `active: false` and a `reason` once `render_id` is no longer inspected. O(1).
    #[func]
    pub fn get_agent_inspection(&self, render_id: i64) -> VarDictionary {
        let Some(core) = self.try_lock_core() else {
            let mut drawn = VarDictionary::new();
            if let Some(origin) = self.car_visuals.drawn_origin(render_id) {
                drawn.set("drawn_position", Vector3::new(origin.x, origin.y, origin.z));
            }
            return drawn;
        };
        let id = u64::try_from(render_id).unwrap_or(u64::MAX);
        let Some(idx) = core.agents.inspected_agent_index(id) else {
            let reason = core
                .agents
                .inspection_end_reason(id)
                .unwrap_or("is no longer in the simulation");
            return vdict! { "active": false, "reason": reason };
        };
        let mut details = inspection_details(&core, idx);
        drop(core);
        if let Some(origin) = self.car_visuals.drawn_origin(render_id) {
            details.set("drawn_position", Vector3::new(origin.x, origin.y, origin.z));
        }
        details
    }

    /// The inspected agent's remaining route as a line list: `points` and `colors`, the same
    /// lines as a ribbon for the `route_ribbon` shader (`ribbon_vertices`, `ribbon_uv`,
    /// `ribbon_uv2`, `ribbon_colors`: six corners per segment), plus `lanes` (lanes and
    /// connectors), `remaining_m` and `end` (how the route ends). Empty while the core is busy
    /// or when `render_id` is not inspected. O(route lanes).
    #[func]
    pub fn get_agent_route_lines(&self, render_id: i64) -> VarDictionary {
        let Some(core) = self.try_lock_core() else {
            return VarDictionary::new();
        };
        let Some(idx) = u64::try_from(render_id)
            .ok()
            .and_then(|id| core.agents.inspected_agent_index(id))
        else {
            return VarDictionary::new();
        };
        let lines = route_lines(&core, idx);
        drop(core);
        let ribbon = ribbon(&lines.points, &lines.colors);
        vdict! {
            "points": PackedVector3Array::from(lines.points.as_slice()),
            "colors": PackedColorArray::from(lines.colors.as_slice()),
            "ribbon_vertices": PackedVector3Array::from(ribbon.vertices.as_slice()),
            "ribbon_uv": PackedVector2Array::from(ribbon.uv.as_slice()),
            "ribbon_uv2": PackedVector2Array::from(ribbon.uv2.as_slice()),
            "ribbon_colors": PackedColorArray::from(ribbon.colors.as_slice()),
            "lanes": lines.lanes as i64,
            "remaining_m": lines.remaining_m,
            "end": lines.end,
        }
    }

    /// For debugging and bridge tests: places a car at the start of the shortest car route
    /// between the road nodes nearest `from` and `to` (XZ, within 50 m) and returns its render
    /// ID, or `-1` without a route. The car has no home or destination building and stops where
    /// the route ends. It changes the simulation outside the tick, so the session is no longer
    /// reproducible.
    #[func]
    pub fn spawn_debug_car(&self, from: Vector3, to: Vector3) -> i64 {
        let mut core = self.lock_core();
        let graph = &core.region_graph;
        let (Some(start), Some(end)) =
            (nearest_road_node(graph, from), nearest_road_node(graph, to))
        else {
            return -1;
        };
        let Some((_, _, path)) = core.transit_network.cch_graph.find_path(
            start,
            end,
            usize::MAX,
            graph,
            TransitFlags::CAR,
        ) else {
            return -1;
        };
        let Some(edge_id) = path
            .get(1)
            .and_then(|&next| graph.get_edge_between_nodes(start, next))
        else {
            return -1;
        };
        let is_fwd = graph.edge(edge_id).start_node == start;
        let lanes = &core.transit_network.lane_system;
        let Some(lane_id) = lanes
            .edge_lanes
            .get(&edge_id)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&id| {
                let lane = &lanes.lanes[id];
                lane.is_fwd == is_fwd && lane.lane_type == LaneType::Vehicle
            })
            .min_by_key(|&id| lanes.lanes[id].lane_idx.unsigned_abs())
        else {
            return -1;
        };
        let at = lanes.lanes[lane_id].geometry[0];
        let idx = core
            .agents
            .spawn_debug_car(path, edge_id, lane_id, Vector2::new(at.x, at.z));
        core.agents.render_id[idx] as i64
    }
}

// The road node nearest `pos` (XZ) within the debug car reach.
fn nearest_road_node(graph: &RegionGraph, pos: Vector3) -> Option<u32> {
    let reach = Vector3::new(DEBUG_CAR_NODE_REACH_M, 0.0, DEBUG_CAR_NODE_REACH_M);
    let mut best: Option<(f32, u32)> = None;
    graph.visit_nodes_near_aabb(pos - reach, pos + reach, |node_id| {
        let node = graph.node(node_id).pos;
        let distance_sq = (node.x - pos.x).powi(2) + (node.z - pos.z).powi(2);
        if graph.node_has_live_incident_edge(node_id)
            && best.is_none_or(|best| (distance_sq, node_id) < best)
        {
            best = Some((distance_sq, node_id));
        }
    });
    best.map(|(_, node_id)| node_id)
}
