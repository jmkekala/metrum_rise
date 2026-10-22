// SPDX-License-Identifier: GPL-2.0-only

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
use super::candidate::{NODE_RANKS, RouteStart, TripQueryScratch, candidate_lane_id};
use super::types::BuiltNetworkReplan;
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
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
) -> Result<BuiltNetworkReplan, ReplanFailure> {
    let debug_context = debug_context.filter(|_| crate::debug::Channel::Traffic.enabled());
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
        return Err(ReplanFailure::new(
            start_node,
            target_building,
            usize::MAX,
            "target_building_out_of_range",
            candidate_diagnostics,
        ));
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
        return Err(ReplanFailure::new(
            start_node,
            target_building,
            destination_entrance.edge_idx,
            "destination_edge_out_of_range",
            candidate_diagnostics,
        ));
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
        return Err(ReplanFailure::new(
            start_node,
            target_building,
            destination_entrance.edge_idx,
            "destination_edge_deleted",
            candidate_diagnostics,
        ));
    }
    TripQueryScratch::with_routes(|scratch| {
        let route_start = if incoming_edge_valid {
            RouteStart::Edge(incoming_edge)
        } else {
            RouteStart::Node
        };
        let mut best: Option<(f32, u8, usize, f32, u32)> = None;
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
            // A car that may not turn at the shared node replans the legal loop below.
            let car_loop = mode == MODE_CAR
                && start_node == planned_detach_node
                && !transit_network.cch_graph.vehicle_turn_permitted(
                    start_node,
                    if incoming_edge_valid {
                        incoming_edge
                    } else {
                        usize::MAX
                    },
                    destination_entrance.edge_idx,
                );
            let network_time_s = if start_node == planned_detach_node && !car_loop {
                if mode == MODE_CAR
                    || scratch.has_connectors(
                        &[start_node],
                        start_node,
                        route_start,
                        planned_detach_lane_id,
                        transit_network,
                        graph,
                    )
                {
                    0.0
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
                match scratch.network_path(
                    mode,
                    start_node,
                    planned_detach_node,
                    route_start,
                    planned_detach_lane_id,
                    transit_network,
                    graph,
                    |_| (),
                ) {
                    Ok((seconds, ())) => seconds,
                    Err(reason) => {
                        record_replan_candidate_diagnostic(
                            &mut candidate_diagnostics,
                            destination_rank,
                            planned_detach_node,
                            planned_detach_lane_id,
                            reason,
                        );
                        continue;
                    }
                }
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
                Some((best_cost, best_rank, best_lane, best_d, _)) => {
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
                scratch.retain_network_path(start_node == planned_detach_node && !car_loop);
                best = Some((
                    total_cost_s,
                    destination_rank,
                    planned_detach_lane_id,
                    planned_detach_lane_d,
                    planned_detach_node,
                ));
            }
        }

        let Some((_, _, planned_detach_lane_id, planned_detach_lane_d, planned_detach_node)) = best
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
            return Err(ReplanFailure::new(
                start_node,
                target_building,
                destination_entrance.edge_idx,
                "no_valid_destination_candidate",
                candidate_diagnostics,
            ));
        };
        let current_path = scratch.finish_path();
        let mut access_flags = ACCESS_PLAN_VALID | (preserve_flags & ACCESS_IMMIGRATION_ORIGIN);
        if current_path.is_empty() {
            access_flags |= ACCESS_ZERO_HOP_NODE_PATH;
        }

        Ok(BuiltNetworkReplan {
            planned_detach_node,
            planned_detach_lane_id,
            planned_detach_lane_d,
            current_path,
            access_flags,
        })
    })
}

/// Why one exit candidate (either end of the destination's road) was rejected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::simulation::economy::agents::tick) struct ReplanCandidateDiagnostic {
    destination_rank: u8,
    /// Node the agent would leave the road network from.
    pub(in crate::simulation::economy::agents::tick) planned_detach_node: u32,
    /// Lane the agent would leave the road from; `usize::MAX` when there is none.
    pub(in crate::simulation::economy::agents::tick) planned_detach_lane_id: usize,
    /// The planner's rejection code, such as `cch_no_path`.
    pub(in crate::simulation::economy::agents::tick) reason: &'static str,
}

/// Why `plan_network_replan` found no route, from the planner's own checks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::simulation::economy::agents::tick) struct ReplanFailure {
    /// Node the search started from.
    pub(in crate::simulation::economy::agents::tick) start_node: u32,
    /// The destination building.
    pub(in crate::simulation::economy::agents::tick) building: usize,
    /// The destination's road edge; `usize::MAX` when the building is gone.
    pub(in crate::simulation::economy::agents::tick) edge: usize,
    /// The rejection code, such as `no_valid_destination_candidate`.
    pub(in crate::simulation::economy::agents::tick) reason: &'static str,
    /// Each exit candidate's rejection, by rank.
    pub(in crate::simulation::economy::agents::tick) candidates:
        [Option<ReplanCandidateDiagnostic>; 2],
}

impl ReplanFailure {
    fn new(
        start_node: u32,
        building: usize,
        edge: usize,
        reason: &'static str,
        candidates: [Option<ReplanCandidateDiagnostic>; 2],
    ) -> Self {
        Self {
            start_node,
            building,
            edge,
            reason,
            candidates,
        }
    }
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
        TripQueryScratch::with_routes(|scratch| {
            scratch
                .network_path(
                    MODE_CAR,
                    start_node,
                    border_node,
                    RouteStart::Edge(search_incoming_edge),
                    usize::MAX,
                    transit_network,
                    graph,
                    <[u32]>::to_vec,
                )
                .ok()
                .map(|(_, path)| path)
        })?
    };

    Some(BuiltNetworkReplan {
        planned_detach_node: border_node,
        planned_detach_lane_id: usize::MAX,
        planned_detach_lane_d: 0.0,
        current_path,
        access_flags: ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION,
    })
}
