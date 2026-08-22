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
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use std::sync::atomic::{AtomicU32, Ordering};

/// Rebuilds a destination-side network plan for an agent already in transit.
#[allow(clippy::too_many_arguments)]
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
    debug_context: Option<(usize, &'static str)>,
) -> Option<BuiltNetworkReplan> {
    let debug_context = debug_context.filter(|_| crate::debug::is_traffic_enabled());
    let incoming_edge_valid = incoming_edge_is_incident_to_node(incoming_edge, start_node, graph);
    let mut candidate_diagnostics = [None, None];

    if target_building >= allocator.buildings.len() || target_building >= allocator.entrances.len()
    {
        emit_network_replan_diagnostics(
            debug_context,
            start_node,
            incoming_edge,
            incoming_edge_valid,
            target_building,
            mode,
            preserve_flags,
            "target_building_out_of_range",
            None,
            &candidate_diagnostics,
            transit_network,
            graph,
        );
        return None;
    }
    let destination_entrance = &allocator.entrances[target_building];
    if destination_entrance.edge_idx >= graph.edge_count() {
        emit_network_replan_diagnostics(
            debug_context,
            start_node,
            incoming_edge,
            incoming_edge_valid,
            target_building,
            mode,
            preserve_flags,
            "destination_edge_out_of_range",
            Some(destination_entrance),
            &candidate_diagnostics,
            transit_network,
            graph,
        );
        return None;
    }
    let destination_edge = graph.edge(destination_entrance.edge_idx);
    if destination_edge.deleted {
        emit_network_replan_diagnostics(
            debug_context,
            start_node,
            incoming_edge,
            incoming_edge_valid,
            target_building,
            mode,
            preserve_flags,
            "destination_edge_deleted",
            Some(destination_entrance),
            &candidate_diagnostics,
            transit_network,
            graph,
        );
        return None;
    }
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
            record_replan_candidate_diagnostic(
                &mut candidate_diagnostics,
                destination_rank,
                planned_detach_node,
                planned_detach_lane_id,
                "candidate_lane_missing",
            );
            continue;
        }
        let Some(detach_lane_origin) =
            lane_origin_node(planned_detach_lane_id, transit_network, graph)
        else {
            record_replan_candidate_diagnostic(
                &mut candidate_diagnostics,
                destination_rank,
                planned_detach_node,
                planned_detach_lane_id,
                "lane_origin_missing",
            );
            continue;
        };
        if detach_lane_origin != planned_detach_node {
            record_replan_candidate_diagnostic(
                &mut candidate_diagnostics,
                destination_rank,
                planned_detach_node,
                planned_detach_lane_id,
                "lane_origin_mismatch",
            );
            continue;
        }
        let Some(planned_detach_lane_d) = projected_lane_distance_for_entrance(
            destination_entrance,
            planned_detach_lane_id,
            transit_network,
            graph,
        ) else {
            record_replan_candidate_diagnostic(
                &mut candidate_diagnostics,
                destination_rank,
                planned_detach_node,
                planned_detach_lane_id,
                "projection_failed",
            );
            continue;
        };
        let Some(ingress_local_distance) = local_access_distance(
            mode,
            destination_entrance,
            planned_detach_lane_id,
            planned_detach_lane_d,
            transit_network,
            graph,
        ) else {
            record_replan_candidate_diagnostic(
                &mut candidate_diagnostics,
                destination_rank,
                planned_detach_node,
                planned_detach_lane_id,
                "local_access_failed",
            );
            continue;
        };
        let ingress_local_time_s = local_access_time_s(ingress_local_distance, mode);
        let Some(destination_frontage_time_s) = frontage_time_s(
            mode,
            planned_detach_lane_id,
            planned_detach_lane_d,
            false,
            transit_network,
            graph,
        ) else {
            record_replan_candidate_diagnostic(
                &mut candidate_diagnostics,
                destination_rank,
                planned_detach_node,
                planned_detach_lane_id,
                "frontage_failed",
            );
            continue;
        };
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
                    record_replan_candidate_diagnostic(
                        &mut candidate_diagnostics,
                        destination_rank,
                        planned_detach_node,
                        planned_detach_lane_id,
                        "zero_hop_connector_missing",
                    );
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
                record_replan_candidate_diagnostic(
                    &mut candidate_diagnostics,
                    destination_rank,
                    planned_detach_node,
                    planned_detach_lane_id,
                    "zero_hop_connector_missing",
                );
                continue;
            }
        } else {
            pathfind_count.fetch_add(1, Ordering::Relaxed);
            let Some((mut travel_seconds, _, mut path)) = transit_network.cch_graph.find_path(
                start_node,
                planned_detach_node,
                search_incoming_edge,
                graph,
                transit_flags_for_mode(mode),
            ) else {
                record_replan_candidate_diagnostic(
                    &mut candidate_diagnostics,
                    destination_rank,
                    planned_detach_node,
                    planned_detach_lane_id,
                    "cch_no_path",
                );
                continue;
            };
            if path.len() < 2 {
                record_replan_candidate_diagnostic(
                    &mut candidate_diagnostics,
                    destination_rank,
                    planned_detach_node,
                    planned_detach_lane_id,
                    "path_too_short",
                );
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
                let Some(fallback) = (if incoming_edge_valid {
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
                }) else {
                    record_replan_candidate_diagnostic(
                        &mut candidate_diagnostics,
                        destination_rank,
                        planned_detach_node,
                        planned_detach_lane_id,
                        "pedestrian_connector_fallback_missing",
                    );
                    continue;
                };
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

    let Some((
        _,
        _,
        planned_detach_lane_id,
        planned_detach_lane_d,
        planned_detach_node,
        current_path,
    )) = best
    else {
        emit_network_replan_diagnostics(
            debug_context,
            start_node,
            incoming_edge,
            incoming_edge_valid,
            target_building,
            mode,
            preserve_flags,
            "no_valid_destination_candidate",
            Some(destination_entrance),
            &candidate_diagnostics,
            transit_network,
            graph,
        );
        return None;
    };
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

#[derive(Clone, Copy)]
struct ReplanCandidateDiagnostic {
    destination_rank: u8,
    planned_detach_node: u32,
    planned_detach_lane_id: usize,
    reason: &'static str,
}

fn record_replan_candidate_diagnostic(
    diagnostics: &mut [Option<ReplanCandidateDiagnostic>; 2],
    destination_rank: u8,
    planned_detach_node: u32,
    planned_detach_lane_id: usize,
    reason: &'static str,
) {
    let idx = destination_rank as usize;
    if idx < diagnostics.len() {
        diagnostics[idx] = Some(ReplanCandidateDiagnostic {
            destination_rank,
            planned_detach_node,
            planned_detach_lane_id,
            reason,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_network_replan_diagnostics(
    debug_context: Option<(usize, &'static str)>,
    start_node: u32,
    incoming_edge: usize,
    incoming_edge_valid: bool,
    target_building: usize,
    mode: u8,
    preserve_flags: u8,
    rejection_reason: &'static str,
    destination_entrance: Option<&BuildingEntrance>,
    candidate_diagnostics: &[Option<ReplanCandidateDiagnostic>; 2],
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) {
    let Some((agent_idx, caller)) = debug_context else {
        return;
    };
    crate::traffic_log!(
        "[REPLAN_DIAG] agent={} caller={} result=reject reason={} start_node={} incoming_edge={} incoming_edge_valid={} target_bldg={} mode={} preserve_flags=0x{:02x} graph_nodes={} graph_edges={} lane_count={}",
        agent_idx,
        caller,
        rejection_reason,
        start_node,
        incoming_edge,
        incoming_edge_valid,
        target_building,
        mode,
        preserve_flags,
        graph.node_count(),
        graph.edge_count(),
        transit_network.lane_system.lanes.len(),
    );

    if let Some(entrance) = destination_entrance {
        let edge_status = if entrance.edge_idx >= graph.edge_count() {
            "missing"
        } else if graph.edge(entrance.edge_idx).deleted {
            "deleted"
        } else {
            "ok"
        };
        crate::traffic_log!(
            "[REPLAN_DIAG]   destination edge={} edge_status={} side={} flags=0x{:02x} entrance_s={:.2} foot_fwd={} foot_bkw={} car_fwd={} car_bkw={} door=({:.2},{:.2}) curb=({:.2},{:.2})",
            entrance.edge_idx,
            edge_status,
            entrance.side,
            entrance.flags,
            entrance.entrance_s_m,
            entrance.foot_lane_fwd,
            entrance.foot_lane_bkw,
            entrance.car_lane_fwd,
            entrance.car_lane_bkw,
            entrance.door_pos.x,
            entrance.door_pos.y,
            entrance.curb_pos.x,
            entrance.curb_pos.y,
        );
    }

    for diagnostic in candidate_diagnostics.iter().flatten() {
        let lane_edge = transit_network
            .lane_system
            .lanes
            .get(diagnostic.planned_detach_lane_id)
            .map(|lane| lane.edge_id)
            .unwrap_or(usize::MAX);
        let lane_origin =
            lane_origin_node(diagnostic.planned_detach_lane_id, transit_network, graph)
                .unwrap_or(u32::MAX);
        crate::traffic_log!(
            "[REPLAN_DIAG]   candidate rank={} detach_node={} detach_lane={} lane_edge={} lane_origin={} reason={}",
            diagnostic.destination_rank,
            diagnostic.planned_detach_node,
            diagnostic.planned_detach_lane_id,
            lane_edge,
            lane_origin,
            diagnostic.reason,
        );
    }
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
