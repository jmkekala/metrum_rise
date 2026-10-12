// SPDX-License-Identifier: GPL-2.0-only

//! Candidate scoring and exact path construction for building-origin trips.

use super::super::super::{
    ACCESS_PATH_FROM_FLOW_FIELD, ACCESS_PLAN_VALID, ACCESS_ZERO_HOP_NODE_PATH, MODE_CAR,
};
use super::super::access::{
    direct_frontage_segment_time_s, frontage_time_s, local_access_distance, local_access_time_s,
    projected_lane_distance_for_entrance,
};
use super::super::lane_nav::{lane_origin_node, lane_terminal_node};
use crate::simulation::buildings::allocator::BuildingEntrance;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneType;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::zoning::ZoneType;
use std::cmp::Ordering as CmpOrdering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU32, Ordering};

mod query;
pub(crate) use query::{SharedNetworkQuotes, TripQueryScratch};
pub(super) use query::RouteStart;

#[cfg(test)]
mod owned_oracle;

pub(super) const NODE_RANKS: [u8; 2] = [0, 1];
pub(super) const NODE_RANK_PAIRS: [(u8, u8); 4] = [(0, 0), (0, 1), (1, 0), (1, 1)];
const CAR_MODE_CHOICE_OVERHEAD_S: f32 = 180.0;
const WALK_CONNECTOR_COST_SPEED_MS: f32 = 1.4;

#[derive(Clone)]
pub(super) struct PlannedTripCandidate {
    pub(super) total_cost_s: f32,
    pub(super) mode_choice_cost_s: f32,
    pub(super) origin_rank: u8,
    pub(super) destination_rank: u8,
    pub(super) mode: u8,
    pub(super) planned_attach_node: u32,
    pub(super) planned_detach_node: u32,
    pub(super) planned_attach_lane_id: usize,
    pub(super) planned_detach_lane_id: usize,
    pub(super) planned_attach_lane_d: f32,
    pub(super) planned_detach_lane_d: f32,
    #[cfg(test)]
    pub(super) network_path: Option<Vec<u32>>,
}

pub(super) fn transit_flags_for_mode(mode: u8) -> u8 {
    if mode == MODE_CAR {
        TransitFlags::CAR
    } else {
        TransitFlags::FOOT
    }
}

pub(super) fn mode_choice_cost_for(mode: u8, travel_time_s: f32) -> f32 {
    if mode == MODE_CAR {
        travel_time_s + CAR_MODE_CHOICE_OVERHEAD_S
    } else {
        travel_time_s
    }
}

pub(super) fn candidate_better(
    new_candidate: &PlannedTripCandidate,
    best: &PlannedTripCandidate,
) -> bool {
    match new_candidate
        .mode_choice_cost_s
        .total_cmp(&best.mode_choice_cost_s)
    {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => {
            (
                new_candidate.total_cost_s.to_bits(),
                new_candidate.origin_rank,
                new_candidate.destination_rank,
                new_candidate.planned_attach_lane_id,
                new_candidate.planned_detach_lane_id,
                new_candidate.planned_attach_lane_d.to_bits(),
                new_candidate.planned_detach_lane_d.to_bits(),
            ) < (
                best.total_cost_s.to_bits(),
                best.origin_rank,
                best.destination_rank,
                best.planned_attach_lane_id,
                best.planned_detach_lane_id,
                best.planned_attach_lane_d.to_bits(),
                best.planned_detach_lane_d.to_bits(),
            )
        }
    }
}

pub(super) fn candidate_lane_id(
    mode: u8,
    entrance: &BuildingEntrance,
    toward_start: bool,
    origin: bool,
) -> usize {
    match (mode == MODE_CAR, toward_start, origin) {
        (false, true, true) => entrance.foot_lane_bkw,
        (false, false, true) => entrance.foot_lane_fwd,
        (false, true, false) => entrance.foot_lane_fwd,
        (false, false, false) => entrance.foot_lane_bkw,
        (true, true, true) => entrance.car_lane_bkw,
        (true, false, true) => entrance.car_lane_fwd,
        (true, true, false) => entrance.car_lane_fwd,
        (true, false, false) => entrance.car_lane_bkw,
    }
}

/// One end of a building trip for one mode and node rank: every access term that does not depend
/// on the other end.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TripEnd {
    edge_idx: usize,
    node: u32,
    node_x: f32,
    node_z: f32,
    lane_id: usize,
    lane_d: f32,
    // Door to lane for an origin, lane to door for a destination.
    local_time_s: f32,
    // Lane point to the terminal node for an origin, origin node to the lane point for a
    // destination; needed only when the trip leaves the shared frontage.
    frontage_time_s: Option<f32>,
}

impl TripEnd {
    /// Builds the origin (`origin`) or destination end of a trip at `entrance`.
    pub(super) fn new(
        mode: u8,
        rank: u8,
        entrance: &BuildingEntrance,
        origin: bool,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<Self> {
        if entrance.edge_idx >= graph.edge_count() {
            return None;
        }
        let edge = graph.edge(entrance.edge_idx);
        if edge.deleted {
            return None;
        }
        let node = if rank == 0 {
            edge.start_node
        } else {
            edge.end_node
        };
        let lane_id = candidate_lane_id(mode, entrance, rank == 0, origin);
        if lane_id == usize::MAX {
            return None;
        }
        let lane_node = if origin {
            lane_terminal_node(lane_id, transit_network, graph)?
        } else {
            lane_origin_node(lane_id, transit_network, graph)?
        };
        if lane_node != node {
            return None;
        }
        let lane_d =
            projected_lane_distance_for_entrance(entrance, lane_id, transit_network, graph)?;
        let local_time_s = local_access_time_s(
            local_access_distance(mode, entrance, lane_id, lane_d, transit_network, graph)?,
            mode,
        );
        let pos = graph.node(node).pos;
        Some(Self {
            edge_idx: entrance.edge_idx,
            node,
            node_x: pos.x,
            node_z: pos.z,
            lane_id,
            lane_d,
            local_time_s,
            frontage_time_s: frontage_time_s(
                mode,
                lane_id,
                lane_d,
                origin,
                transit_network,
                graph,
            ),
        })
    }

    /// Departure/arrival ranks for one door and mode, projecting each lane only once.
    /// Local geometry is direction-independent; node validity and frontage prices are not.
    pub(super) fn pair(mode: u8, entrance: &BuildingEntrance,
        network: &TransitNetwork, graph: &RegionGraph) -> [[Option<Self>; 2]; 2] {
        let origins = NODE_RANKS.map(|rank| Self::new(mode, rank, entrance, true, network, graph));
        let destinations = NODE_RANKS.map(|rank| {
            let Some(origin) = origins[usize::from(1 - rank)] else {
                return Self::new(mode, rank, entrance, false, network, graph);
            };
            let edge = graph.edge(entrance.edge_idx);
            let node = if rank == 0 { edge.start_node } else { edge.end_node };
            if lane_origin_node(origin.lane_id, network, graph) != Some(node) { return None; }
            let pos = graph.node(node).pos;
            Some(Self { node, node_x: pos.x, node_z: pos.z,
                frontage_time_s: frontage_time_s(mode, origin.lane_id, origin.lane_d,
                    false, network, graph), ..origin })
        });
        [origins, destinations]
    }

    /// Exact time an origin end spends before a network trip leaves its node; `None` when no
    /// network trip can leave from it.
    pub(super) fn network_departure_s(&self) -> Option<f32> {
        Some(self.local_time_s + self.frontage_time_s?)
    }

    pub(super) fn has_proven_walk_to(&self, destination: &Self,
        reachability: &super::reachability::PedestrianReachability) -> bool {
        self.network_departure_s().is_some_and(f32::is_finite)
            && destination.network_departure_s().is_some_and(f32::is_finite)
            // At a shared node the ordinary planner requires the direct connector;
            // a longer cycle in this component is not a substitute for that check.
            && self.node != destination.node
            && reachability.proves(self.lane_id, destination.lane_id)
    }

    pub(super) fn walking_group(&self, reachability: &super::reachability::PedestrianReachability) -> usize {
        if self.network_departure_s().is_some_and(f32::is_finite) {
            reachability.cyclic_group(self.lane_id)
        } else { usize::MAX }
    }

    /// Graph node used by this validated trip attachment.
    pub(super) fn network_node(&self) -> u32 { self.node }

    /// Network node this end attaches to or detaches from.
    pub(super) fn node_pos(&self) -> (f32, f32) {
        (self.node_x, self.node_z)
    }
}

/// A trip between two ends, priced when that needs no network query.
pub(super) enum TripLeg {
    /// The trip is fully priced: shared frontage, a shared node, or infeasible.
    Priced(Option<PlannedTripCandidate>),
    /// The trip needs a network path; `lower_bound_s` bounds its total cost from below.
    Network { lower_bound_s: f32 },
}

/// Prices a trip between two ends when that needs no network query, otherwise bounds its cost
/// from below with straight-line travel at `network_speed_bound_ms` between the two nodes.
///
/// `network_speed_bound_ms` must not be below any speed the route cost uses, or the bound is not
/// a bound.
pub(super) fn price_trip_without_network(
    mode: u8,
    origin_rank: u8,
    destination_rank: u8,
    origin: &TripEnd,
    destination: &TripEnd,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    network_speed_bound_ms: f32,
) -> TripLeg {
    if shares_direct_frontage(origin, destination) {
        return TripLeg::Priced(direct_frontage_candidate(
            mode,
            origin_rank,
            destination_rank,
            origin,
            destination,
            transit_network,
            graph,
        ));
    }
    let (Some(origin_frontage_time_s), Some(destination_frontage_time_s)) =
        (origin.frontage_time_s, destination.frontage_time_s)
    else {
        return TripLeg::Priced(None);
    };
    if origin.node == destination.node {
        return TripLeg::Priced(combine_trip_ends(
            mode,
            origin_rank,
            destination_rank,
            origin,
            destination,
            transit_network,
            graph,
            &AtomicU32::new(0),
        ));
    }
    let network_distance_m =
        (origin.node_x - destination.node_x).hypot(origin.node_z - destination.node_z);
    TripLeg::Network {
        lower_bound_s: origin.local_time_s
            + origin_frontage_time_s
            + network_distance_m / network_speed_bound_ms
            + destination_frontage_time_s
            + destination.local_time_s,
    }
}

fn shares_direct_frontage(origin: &TripEnd, destination: &TripEnd) -> bool {
    origin.edge_idx == destination.edge_idx
        && origin.lane_id == destination.lane_id
        && origin.lane_d <= destination.lane_d + 1e-6
}

/// Exact feasibility for a network leg, retaining CCH mode/turn restrictions and the
/// pedestrian connector fallback without constructing a path that the buyer never uses.
pub(super) fn network_leg_is_reachable(
    mode: u8,
    origin: &TripEnd,
    destination: &TripEnd,
    network: &TransitNetwork,
    graph: &RegionGraph,
    walking: &mut super::reachability::PedestrianReachability,
    query: &mut crate::simulation::pathing::cch::CchQueryScratch,
) -> bool {
    network.cch_graph.find_cost(origin.node, destination.node, origin.edge_idx,
        graph, transit_flags_for_mode(mode), query).is_some()
        && (mode == MODE_CAR || walking.reaches(origin.lane_id, destination.lane_id))
}

fn direct_frontage_candidate(
    mode: u8,
    origin_rank: u8,
    destination_rank: u8,
    origin: &TripEnd,
    destination: &TripEnd,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<PlannedTripCandidate> {
    let direct_frontage_time_s = direct_frontage_segment_time_s(
        mode,
        origin.lane_id,
        origin.lane_d,
        destination.lane_d,
        transit_network,
        graph,
    )?;
    finish_candidate(
        mode,
        origin_rank,
        destination_rank,
        origin,
        destination,
        origin.local_time_s + direct_frontage_time_s + destination.local_time_s,
    )
}

/// Prices one trip between two prepared ends, querying the network when the ends do not share a
/// frontage or a node.
pub(super) fn combine_trip_ends(
    mode: u8,
    origin_rank: u8,
    destination_rank: u8,
    origin: &TripEnd,
    destination: &TripEnd,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<PlannedTripCandidate> {
    if shares_direct_frontage(origin, destination) {
        return direct_frontage_candidate(
            mode,
            origin_rank,
            destination_rank,
            origin,
            destination,
            transit_network,
            graph,
        );
    }
    let origin_frontage_time_s = origin.frontage_time_s?;
    let destination_frontage_time_s = destination.frontage_time_s?;

    if origin.node != destination.node {
        return TripQueryScratch::with(transit_network, graph, |scratch| {
            scratch.combine(mode, origin_rank, destination_rank, origin, destination,
                transit_network, graph, pathfind_count, None)
        });
    }
    if mode != MODE_CAR
        && !connector_to_lane_exists(origin.lane_id, destination.lane_id, transit_network) {
        return None;
    }

    finish_candidate(
        mode,
        origin_rank,
        destination_rank,
        origin,
        destination,
        origin.local_time_s
            + origin_frontage_time_s
            + 0.0
            + destination_frontage_time_s
            + destination.local_time_s,
    )
}

fn finish_candidate(
    mode: u8,
    origin_rank: u8,
    destination_rank: u8,
    origin: &TripEnd,
    destination: &TripEnd,
    total_cost_s: f32,
) -> Option<PlannedTripCandidate> {
    if !total_cost_s.is_finite() {
        return None;
    }

    Some(PlannedTripCandidate {
        total_cost_s,
        mode_choice_cost_s: mode_choice_cost_for(mode, total_cost_s),
        origin_rank,
        destination_rank,
        mode,
        planned_attach_node: origin.node,
        planned_detach_node: destination.node,
        planned_attach_lane_id: origin.lane_id,
        planned_detach_lane_id: destination.lane_id,
        planned_attach_lane_d: origin.lane_d,
        planned_detach_lane_d: destination.lane_d,
        #[cfg(test)]
        network_path: None,
    })
}

#[cfg(test)]
fn evaluate_planned_trip_candidate(
    mode: u8,
    origin_rank: u8,
    destination_rank: u8,
    origin_entrance: &BuildingEntrance,
    destination_entrance: &BuildingEntrance,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<PlannedTripCandidate> {
    // Both entrances must sit on live edges before either end is priced.
    for entrance in [origin_entrance, destination_entrance] {
        if entrance.edge_idx >= graph.edge_count() || graph.edge(entrance.edge_idx).deleted {
            return None;
        }
    }
    let origin = TripEnd::new(
        mode,
        origin_rank,
        origin_entrance,
        true,
        transit_network,
        graph,
    )?;
    let destination = TripEnd::new(
        mode,
        destination_rank,
        destination_entrance,
        false,
        transit_network,
        graph,
    )?;
    combine_trip_ends(
        mode,
        origin_rank,
        destination_rank,
        &origin,
        &destination,
        transit_network,
        graph,
        pathfind_count,
    )
}

pub(super) fn build_exact_path_for_candidate(
    candidate: &mut PlannedTripCandidate,
    target_building: usize,
    target_zone: ZoneType,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<(Vec<u32>, u8)> {
    let direct_same_lane_frontage = candidate.planned_attach_lane_id
        == candidate.planned_detach_lane_id
        && candidate.planned_attach_lane_id != usize::MAX
        && candidate.planned_attach_lane_d <= candidate.planned_detach_lane_d + 1e-6;
    if direct_same_lane_frontage {
        return Some((Vec::new(), ACCESS_PLAN_VALID));
    }

    if candidate.planned_attach_node == candidate.planned_detach_node {
        return Some((Vec::new(), ACCESS_PLAN_VALID | ACCESS_ZERO_HOP_NODE_PATH));
    }

    TripQueryScratch::with_routes(|scratch| {
        let flow_field = if candidate.mode == MODE_CAR {
            transit_network.flow_fields.car(target_zone)
        } else {
            transit_network.flow_fields.foot(target_zone)
        };
        if let Some(field) = flow_field
            && field.nearest_building.get(candidate.planned_attach_node as usize) == Some(&target_building)
            && let Some(path) = scratch.flow_path(field, candidate, transit_network, graph, <[u32]>::to_vec) {
            return Some((path, ACCESS_PLAN_VALID | ACCESS_PATH_FROM_FLOW_FIELD));
        }
        pathfind_count.fetch_add(1, Ordering::Relaxed);
        scratch.network_path(candidate.mode, candidate.planned_attach_node,
            candidate.planned_detach_node, RouteStart::Lane(candidate.planned_attach_lane_id),
            candidate.planned_detach_lane_id, transit_network, graph, <[u32]>::to_vec)
            .ok().map(|(_, path)| (path, ACCESS_PLAN_VALID))
    })
}

/// Checks a stored path using prepared lane-frontier storage.
#[cfg(test)]
pub(super) fn pedestrian_path_has_lane_connectors(path: &[u32], attach: usize, detach: usize,
    network: &TransitNetwork, graph: &RegionGraph) -> bool {
    TripQueryScratch::with(network, graph, |scratch| scratch.has_connectors(path,
        path.first().copied().unwrap_or(u32::MAX), RouteStart::Lane(attach), detach, network, graph))
}


fn collect_connector_targets_to_edge(
    from_lane_id: usize,
    from_node: u32,
    target_edge_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    out: &mut Vec<usize>,
) {
    let Some(from_lane) = transit_network.lane_system.lanes.get(from_lane_id) else {
        return;
    };
    for &conn_lane_id in &from_lane.next_lanes {
        let Some(conn_lane) = transit_network.lane_system.lanes.get(conn_lane_id) else {
            continue;
        };
        if conn_lane.edge_id != usize::MAX || conn_lane.lane_type != LaneType::Foot {
            continue;
        }
        let Some(&target_lane_id) = conn_lane.next_lanes.first() else {
            continue;
        };
        let Some(target_lane) = transit_network.lane_system.lanes.get(target_lane_id) else {
            continue;
        };
        if target_lane.edge_id == target_edge_id
            && target_lane.lane_type == LaneType::Foot
            && lane_origin_node(target_lane_id, transit_network, graph) == Some(from_node)
        {
            out.push(target_lane_id);
        }
    }
}

fn connector_to_lane_exists(
    from_lane_id: usize,
    target_lane_id: usize,
    transit_network: &TransitNetwork,
) -> bool {
    let Some(from_lane) = transit_network.lane_system.lanes.get(from_lane_id) else {
        return false;
    };
    from_lane.next_lanes.iter().copied().any(|conn_lane_id| {
        transit_network
            .lane_system
            .lanes
            .get(conn_lane_id)
            .is_some_and(|conn_lane| {
                conn_lane.edge_id == usize::MAX
                    && conn_lane.lane_type == LaneType::Foot
                    && conn_lane.next_lanes.first().copied() == Some(target_lane_id)
            })
    })
}


fn visit_pedestrian_successors(
    state: &PedestrianSearchState,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    mut visit: impl FnMut(PedestrianSearchState),
) {
    let Some(from_lane) = transit_network.lane_system.lanes.get(state.incoming_lane) else {
        return;
    };
    for &conn_lane_id in &from_lane.next_lanes {
        let Some(conn_lane) = transit_network.lane_system.lanes.get(conn_lane_id) else {
            continue;
        };
        if conn_lane.edge_id != usize::MAX || conn_lane.lane_type != LaneType::Foot {
            continue;
        }
        let Some(&target_lane_id) = conn_lane.next_lanes.first() else {
            continue;
        };
        let Some(target_lane) = transit_network.lane_system.lanes.get(target_lane_id) else {
            continue;
        };
        if target_lane.lane_type != LaneType::Foot
            || lane_origin_node(target_lane_id, transit_network, graph) != Some(state.node)
        {
            continue;
        }
        let Some(next_node) = lane_terminal_node(target_lane_id, transit_network, graph) else {
            continue;
        };
        let travel_cost = graph
            .edges()
            .get(target_lane.edge_id)
            .map(|edge| edge.base_cost)
            .unwrap_or(target_lane.length / WALK_CONNECTOR_COST_SPEED_MS)
            + conn_lane.length / WALK_CONNECTOR_COST_SPEED_MS;
        visit(PedestrianSearchState {
            cost: state.cost + travel_cost.max(0.001),
            node: next_node,
            incoming_lane: target_lane_id,
        });
    }
}


#[derive(Copy, Clone, PartialEq)]
struct PedestrianSearchState {
    cost: f32,
    node: u32,
    incoming_lane: usize,
}

impl Eq for PedestrianSearchState {}

impl Ord for PedestrianSearchState {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(CmpOrdering::Equal)
    }
}

impl PartialOrd for PedestrianSearchState {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;

    pub(super) fn test_edge(start_node: u32, end_node: u32, start_x: f32, end_x: f32) -> Edge {
        Edge {
            start_node,
            end_node,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 14.0,
            base_cost: 1.0,
            physical_length: (end_x - start_x).abs(),
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![
                Vector3::new(start_x, 0.0, 0.0),
                Vector3::new(end_x, 0.0, 0.0),
            ],
            physical_geometry: vec![
                Vector3::new(start_x, 0.0, 0.0),
                Vector3::new(end_x, 0.0, 0.0),
            ],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        }
    }

    #[test]
    fn pedestrian_path_validation_rejects_missing_attach_edge_backtrack_connector() {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(200.0, 0.0, 0.0), NodeType::Junction);
        let edge0 = graph.add_edge(test_edge(n0, n1, 0.0, 100.0));
        let edge1 = graph.add_edge(test_edge(n1, n2, 100.0, 200.0));
        // Only the reverse car turn is allowed; walkers can still use this junction.
        graph.add_lane_connection(n1, edge1, 0, edge0, 0);
        graph.rebuild_adjacency_list();

        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);

        let attach_lane = network.lane_system.edge_lanes[&edge0]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &network.lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Foot
                    && lane_terminal_node(lane_id, &network, &graph) == Some(n0)
            })
            .expect("foot lane ending at n0");
        let detach_lane = network.lane_system.edge_lanes[&edge1]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &network.lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Foot
                    && lane_origin_node(lane_id, &network, &graph) == Some(n2)
            })
            .expect("foot lane starting at n2");

        network.flow_fields.rebuild_dirty(&graph, |zone, _| {
            if zone == ZoneType::Commercial {
                vec![(n2, 7)]
            } else {
                Vec::new()
            }
        });
        let mut candidate = PlannedTripCandidate {
            total_cost_s: 0.0,
            mode_choice_cost_s: 0.0,
            origin_rank: 0,
            destination_rank: 1,
            mode: crate::simulation::economy::agents::MODE_WALK,
            planned_attach_node: n0,
            planned_detach_node: n2,
            planned_attach_lane_id: attach_lane,
            planned_detach_lane_id: detach_lane,
            planned_attach_lane_d: 50.0,
            planned_detach_lane_d: 50.0,
            network_path: None,
        };
        let pathfind_count = AtomicU32::new(0);
        assert_eq!(
            build_exact_path_for_candidate(
                &mut candidate,
                7,
                ZoneType::Commercial,
                &network,
                &graph,
                &pathfind_count,
            ),
            Some((
                vec![n0, n1, n2],
                ACCESS_PLAN_VALID | ACCESS_PATH_FROM_FLOW_FIELD
            ))
        );
        assert_eq!(pathfind_count.load(Ordering::Relaxed), 0);

        network.lane_system.lanes[attach_lane].next_lanes.clear();

        assert!(
            !pedestrian_path_has_lane_connectors(
                &[n0, n1, n2],
                attach_lane,
                detach_lane,
                &network,
                &graph,
            ),
            "planner must reject a node path that movement cannot realize with sidewalk connectors"
        );
        assert!(
            build_exact_path_for_candidate(
                &mut candidate,
                7,
                ZoneType::Commercial,
                &network,
                &graph,
                &pathfind_count,
            )
            .is_none(),
            "the flow field must still reject the disconnected sidewalk"
        );
    }
}

pub(super) fn entrance_pair_supports_mode(
    mode: u8,
    has_car: bool,
    origin_entrance: &BuildingEntrance,
    destination_entrance: &BuildingEntrance,
) -> bool {
    if mode == MODE_CAR {
        has_car
            && (origin_entrance.car_lane_fwd != usize::MAX
                || origin_entrance.car_lane_bkw != usize::MAX)
            && (destination_entrance.car_lane_fwd != usize::MAX
                || destination_entrance.car_lane_bkw != usize::MAX)
    } else {
        (origin_entrance.foot_lane_fwd != usize::MAX || origin_entrance.foot_lane_bkw != usize::MAX)
            && (destination_entrance.foot_lane_fwd != usize::MAX
                || destination_entrance.foot_lane_bkw != usize::MAX)
    }
}

pub(super) fn best_trip_candidate_for_mode(
    mode: u8,
    origin_entrance: &BuildingEntrance,
    destination_entrance: &BuildingEntrance,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<PlannedTripCandidate> {
    for entrance in [origin_entrance, destination_entrance] {
        if entrance.edge_idx >= graph.edge_count() || graph.edge(entrance.edge_idx).deleted {
            return None;
        }
    }
    // These four lane ends are immutable throughout the four rank combinations. Keep
    // the original candidate order and arithmetic while avoiding repeated projection.
    let origins = NODE_RANKS.map(|rank|
        TripEnd::new(mode, rank, origin_entrance, true, transit_network, graph));
    let destinations = NODE_RANKS.map(|rank|
        TripEnd::new(mode, rank, destination_entrance, false, transit_network, graph));
    let mut best: Option<PlannedTripCandidate> = None;
    for (origin_rank, destination_rank) in NODE_RANK_PAIRS {
        let (Some(origin), Some(destination)) =
            (&origins[usize::from(origin_rank)], &destinations[usize::from(destination_rank)])
            else { continue; };
        if let Some(candidate) = combine_trip_ends(
            mode,
            origin_rank,
            destination_rank,
            origin,
            destination,
            transit_network,
            graph,
            pathfind_count,
        ) {
            if best
                .as_ref()
                .is_none_or(|best| candidate_better(&candidate, best))
            {
                best = Some(candidate);
            }
        }
    }
    best
}
