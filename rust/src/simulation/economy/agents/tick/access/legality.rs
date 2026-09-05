// SPDX-License-Identifier: GPL-2.0-only

//! Planned local access topology validation.

use super::super::lane_nav::{lane_origin_node, lane_terminal_node};
use super::geometry::entrance_allows_lane;
use crate::simulation::buildings::allocator::BuildingEntrance;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

/// Returns whether a planned access attach still matches entrance and lane topology.
pub(in crate::simulation::economy::agents::tick) fn planned_attach_is_legal(
    mode: u8,
    entrance: &BuildingEntrance,
    lane_id: usize,
    lane_d: f32,
    planned_attach_node: u32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> bool {
    if !entrance_allows_lane(mode, entrance, lane_id) {
        return false;
    }
    let Some(lane) = transit_network.lane_system.lanes.get(lane_id) else {
        return false;
    };
    if lane_d < 0.0 || lane_d > lane.length + 1e-4 {
        return false;
    }
    lane_terminal_node(lane_id, transit_network, graph) == Some(planned_attach_node)
}

/// Returns whether a planned access detach still matches entrance and lane topology.
pub(in crate::simulation::economy::agents::tick) fn planned_detach_is_legal(
    mode: u8,
    entrance: &BuildingEntrance,
    lane_id: usize,
    lane_d: f32,
    planned_detach_node: u32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> bool {
    if !entrance_allows_lane(mode, entrance, lane_id) {
        return false;
    }
    let Some(lane) = transit_network.lane_system.lanes.get(lane_id) else {
        return false;
    };
    if lane_d < 0.0 || lane_d > lane.length + 1e-4 {
        return false;
    }
    lane_origin_node(lane_id, transit_network, graph) == Some(planned_detach_node)
}
