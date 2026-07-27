//! Network replan construction for agents already outside a building.

use super::super::super::MODE_CAR;
use super::super::super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_IMMIGRATION_ORIGIN, ACCESS_PLAN_VALID,
    ACCESS_ZERO_HOP_NODE_PATH,
};
use super::super::access::{
    frontage_time_s, local_access_distance, local_access_time_s,
    projected_lane_distance_for_entrance,
};
use super::super::lane_nav::lane_origin_node;
use super::candidate::{
    NODE_RANKS, candidate_lane_id, pedestrian_lane_connector_path_from_edge,
    pedestrian_lane_connector_path_from_node, pedestrian_path_has_lane_connectors_from_edge,
    pedestrian_path_has_lane_connectors_from_node, transit_flags_for_mode,
};
use super::types::BuiltNetworkReplan;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use std::sync::atomic::{AtomicU32, Ordering};

/// Rebuilds a destination-side network plan for an agent already in transit.
pub(in crate::simulation::economy::agents::tick) fn plan_network_replan(
    start_node: u32,
    incoming_edge: usize,
    target_building: usize,
    mode: u8,
    preserve_flags: u8,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<BuiltNetworkReplan> {
    if target_building >= allocator.buildings.len() || target_building >= allocator.entrances.len()
    {
        return None;
    }
    let destination_entrance = &allocator.entrances[target_building];
    if destination_entrance.edge_idx >= graph.edge_count() {
        return None;
    }
    let destination_edge = graph.edge(destination_entrance.edge_idx);
    if destination_edge.deleted {
        return None;
    }
    let incoming_edge_valid = incoming_edge_is_incident_to_node(incoming_edge, start_node, graph);
    let search_incoming_edge = if incoming_edge_valid {
        incoming_edge
    } else {
        usize::MAX
    };

    let mut best: Option<(f32, u8, usize, f32, u32, Vec<u32>)> = None;
    for destination_rank in NODE_RANKS {
        let planned_detach_node = if destination_rank == 0 {
            destination_edge.start_node
        } else {
            destination_edge.end_node
        };
        let planned_detach_lane_id =
            candidate_lane_id(mode, destination_entrance, destination_rank == 0, false);
        if planned_detach_lane_id == usize::MAX {
            continue;
        }
        if lane_origin_node(planned_detach_lane_id, transit_network, graph)? != planned_detach_node
        {
            continue;
        }
        let planned_detach_lane_d = projected_lane_distance_for_entrance(
            destination_entrance,
            planned_detach_lane_id,
            transit_network,
            graph,
        )?;
        let ingress_local_time_s = local_access_time_s(
            local_access_distance(
                mode,
                destination_entrance,
                planned_detach_lane_id,
                planned_detach_lane_d,
                transit_network,
                graph,
            )?,
            mode,
        );
        let destination_frontage_time_s = frontage_time_s(
            mode,
            planned_detach_lane_id,
            planned_detach_lane_d,
            false,
            transit_network,
            graph,
        )?;
        let (network_time_s, current_path) = if start_node == planned_detach_node {
            if mode == MODE_CAR {
                (0.0, Vec::new())
            } else if incoming_edge_valid {
                if pedestrian_path_has_lane_connectors_from_edge(
                    &[start_node],
                    start_node,
                    incoming_edge,
                    planned_detach_lane_id,
                    transit_network,
                    graph,
                ) {
                    (0.0, Vec::new())
                } else {
                    continue;
                }
            } else if pedestrian_path_has_lane_connectors_from_node(
                &[start_node],
                start_node,
                planned_detach_lane_id,
                transit_network,
                graph,
            ) {
                (0.0, Vec::new())
            } else {
                continue;
            }
        } else {
            pathfind_count.fetch_add(1, Ordering::Relaxed);
            let (mut travel_seconds, _, mut path) = transit_network.cch_graph.find_path(
                start_node,
                planned_detach_node,
                search_incoming_edge,
                graph,
                transit_flags_for_mode(mode),
            )?;
            if path.len() < 2 {
                continue;
            }
            let pedestrian_path_valid = mode == MODE_CAR
                || if incoming_edge_valid {
                    pedestrian_path_has_lane_connectors_from_edge(
                        &path,
                        start_node,
                        incoming_edge,
                        planned_detach_lane_id,
                        transit_network,
                        graph,
                    )
                } else {
                    pedestrian_path_has_lane_connectors_from_node(
                        &path,
                        start_node,
                        planned_detach_lane_id,
                        transit_network,
                        graph,
                    )
                };
            if !pedestrian_path_valid {
                let fallback = if incoming_edge_valid {
                    pedestrian_lane_connector_path_from_edge(
                        start_node,
                        planned_detach_node,
                        incoming_edge,
                        planned_detach_lane_id,
                        transit_network,
                        graph,
                    )
                } else {
                    pedestrian_lane_connector_path_from_node(
                        start_node,
                        planned_detach_node,
                        planned_detach_lane_id,
                        transit_network,
                        graph,
                    )
                }?;
                travel_seconds = fallback.0;
                path = fallback.1;
            }
            (travel_seconds, path)
        };
        let total_cost_s = network_time_s + destination_frontage_time_s + ingress_local_time_s;
        let new_key = (
            total_cost_s.to_bits(),
            destination_rank,
            planned_detach_lane_id,
            planned_detach_lane_d.to_bits(),
        );
        let replace = match &best {
            None => true,
            Some((best_cost, best_rank, best_lane, best_d, _, _)) => {
                new_key
                    < (
                        best_cost.to_bits(),
                        *best_rank,
                        *best_lane,
                        best_d.to_bits(),
                    )
            }
        };
        if replace {
            best = Some((
                total_cost_s,
                destination_rank,
                planned_detach_lane_id,
                planned_detach_lane_d,
                planned_detach_node,
                current_path,
            ));
        }
    }

    let (_, _, planned_detach_lane_id, planned_detach_lane_d, planned_detach_node, current_path) =
        best?;
    let mut access_flags = ACCESS_PLAN_VALID | (preserve_flags & ACCESS_IMMIGRATION_ORIGIN);
    if current_path.is_empty() {
        access_flags |= ACCESS_ZERO_HOP_NODE_PATH;
    }

    Some(BuiltNetworkReplan {
        planned_detach_node,
        planned_detach_lane_id,
        planned_detach_lane_d,
        current_path,
        access_flags,
    })
}

fn incoming_edge_is_incident_to_node(
    incoming_edge: usize,
    start_node: u32,
    graph: &RegionGraph,
) -> bool {
    graph.get_edge(incoming_edge).is_some_and(|edge| {
        !edge.deleted && (edge.start_node == start_node || edge.end_node == start_node)
    })
}

/// Rebuilds a network-only plan for a freight carrier already travelling to an OWA border.
pub(in crate::simulation::economy::agents::tick) fn plan_border_network_replan(
    start_node: u32,
    incoming_edge: usize,
    border_node: u32,
    graph: &RegionGraph,
    transit_network: &TransitNetwork,
    pathfind_count: &AtomicU32,
) -> Option<BuiltNetworkReplan> {
    if start_node as usize >= graph.node_count() || border_node as usize >= graph.node_count() {
        return None;
    }

    let current_path = if start_node == border_node {
        Vec::new()
    } else {
        let search_incoming_edge =
            if incoming_edge_is_incident_to_node(incoming_edge, start_node, graph) {
                incoming_edge
            } else {
                usize::MAX
            };
        pathfind_count.fetch_add(1, Ordering::Relaxed);
        let (_, _, path) = transit_network.cch_graph.find_path(
            start_node,
            border_node,
            search_incoming_edge,
            graph,
            TransitFlags::CAR,
        )?;
        if path.len() < 2 {
            return None;
        }
        path
    };

    Some(BuiltNetworkReplan {
        planned_detach_node: border_node,
        planned_detach_lane_id: usize::MAX,
        planned_detach_lane_d: 0.0,
        current_path,
        access_flags: ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION,
    })
}
