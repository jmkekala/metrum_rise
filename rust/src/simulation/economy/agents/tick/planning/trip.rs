// SPDX-License-Identifier: GPL-2.0-only

//! Building-origin trip planning and commute estimates.

use super::super::super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, MODE_CAR, MODE_WALK,
};
use super::super::access::{
    frontage_time_s, local_access_distance, local_access_time_s,
    projected_lane_distance_for_entrance,
};
use super::super::lane_nav::lane_terminal_node;
use super::candidate::{
    NODE_RANK_PAIRS, NODE_RANKS, PlannedTripCandidate, TripEnd, TripLeg,
    best_trip_candidate_for_mode, build_exact_path_for_candidate, candidate_better,
    candidate_lane_id, combine_trip_ends, entrance_pair_supports_mode, mode_choice_cost_for,
    price_trip_without_network,
};
use super::types::BuiltTripPlan;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use std::sync::atomic::{AtomicU32, Ordering};

const BOUND_ROUNDING_MARGIN: f32 = 1.0e-3;

/// Builds a full plan for a trip that starts inside a building.
pub(crate) fn plan_building_origin_trip(
    current_building: usize,
    target_building: usize,
    activity: u8,
    has_car: bool,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<BuiltTripPlan> {
    if current_building >= allocator.buildings.len()
        || target_building >= allocator.buildings.len()
        || current_building >= allocator.entrances.len()
        || target_building >= allocator.entrances.len()
    {
        return None;
    }

    let origin_entrance = &allocator.entrances[current_building];
    let destination_entrance = &allocator.entrances[target_building];
    let best_walk =
        if entrance_pair_supports_mode(MODE_WALK, has_car, origin_entrance, destination_entrance) {
            best_trip_candidate_for_mode(
                MODE_WALK,
                origin_entrance,
                destination_entrance,
                transit_network,
                graph,
                pathfind_count,
            )
        } else {
            None
        };
    let best_car =
        if entrance_pair_supports_mode(MODE_CAR, has_car, origin_entrance, destination_entrance) {
            best_trip_candidate_for_mode(
                MODE_CAR,
                origin_entrance,
                destination_entrance,
                transit_network,
                graph,
                pathfind_count,
            )
        } else {
            None
        };

    let mut chosen = match (best_walk, best_car) {
        (None, None) => return None,
        (Some(walk), None) => walk,
        (None, Some(car)) => car,
        (Some(walk), Some(car)) => {
            if walk.mode_choice_cost_s <= car.mode_choice_cost_s {
                walk
            } else {
                car
            }
        }
    };

    let target_zone = allocator.buildings[target_building].zone_type;
    let (current_path, access_flags) = build_exact_path_for_candidate(
        &mut chosen,
        target_building,
        target_zone,
        transit_network,
        graph,
        pathfind_count,
    )?;

    Some(BuiltTripPlan {
        mode: chosen.mode,
        target_building,
        activity,
        planned_attach_node: chosen.planned_attach_node,
        planned_detach_node: chosen.planned_detach_node,
        planned_attach_lane_id: chosen.planned_attach_lane_id,
        planned_detach_lane_id: chosen.planned_detach_lane_id,
        planned_attach_lane_d: chosen.planned_attach_lane_d,
        planned_detach_lane_d: chosen.planned_detach_lane_d,
        current_path,
        access_flags,
    })
}

/// Builds a full car plan from a building door to an outside-world border node.
pub(crate) fn plan_building_to_border_trip(
    current_building: usize,
    border_node: u32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<BuiltTripPlan> {
    if current_building >= allocator.buildings.len()
        || current_building >= allocator.entrances.len()
        || border_node as usize >= graph.node_count()
    {
        return None;
    }

    let origin_entrance = &allocator.entrances[current_building];
    if origin_entrance.edge_idx >= graph.edge_count()
        || graph.edge(origin_entrance.edge_idx).deleted
    {
        return None;
    }

    let origin_edge = graph.edge(origin_entrance.edge_idx);
    let mut best_candidate: Option<PlannedTripCandidate> = None;
    for origin_rank in super::candidate::NODE_RANKS {
        let planned_attach_node = if origin_rank == 0 {
            origin_edge.start_node
        } else {
            origin_edge.end_node
        };
        let planned_attach_lane_id =
            candidate_lane_id(MODE_CAR, origin_entrance, origin_rank == 0, true);
        if planned_attach_lane_id == usize::MAX {
            continue;
        }
        if lane_terminal_node(planned_attach_lane_id, transit_network, graph)?
            != planned_attach_node
        {
            continue;
        }

        let planned_attach_lane_d = projected_lane_distance_for_entrance(
            origin_entrance,
            planned_attach_lane_id,
            transit_network,
            graph,
        )?;
        let egress_local_time_s = local_access_time_s(
            local_access_distance(
                MODE_CAR,
                origin_entrance,
                planned_attach_lane_id,
                planned_attach_lane_d,
                transit_network,
                graph,
            )?,
            MODE_CAR,
        );
        let origin_frontage_time_s = frontage_time_s(
            MODE_CAR,
            planned_attach_lane_id,
            planned_attach_lane_d,
            true,
            transit_network,
            graph,
        )?;

        let mut network_path = None;
        let network_path_time_s = if planned_attach_node == border_node {
            0.0
        } else {
            pathfind_count.fetch_add(1, Ordering::Relaxed);
            let (travel_seconds, _, path) = transit_network.cch_graph.find_path(
                planned_attach_node,
                border_node,
                usize::MAX,
                graph,
                TransitFlags::CAR,
            )?;
            network_path = Some(path);
            travel_seconds
        };
        let total_cost_s = egress_local_time_s + origin_frontage_time_s + network_path_time_s;
        if !total_cost_s.is_finite() {
            continue;
        }

        let candidate = PlannedTripCandidate {
            total_cost_s,
            mode_choice_cost_s: mode_choice_cost_for(MODE_CAR, total_cost_s),
            origin_rank,
            destination_rank: 0,
            mode: MODE_CAR,
            planned_attach_node,
            planned_detach_node: border_node,
            planned_attach_lane_id,
            planned_detach_lane_id: usize::MAX,
            planned_attach_lane_d,
            planned_detach_lane_d: 0.0,
            network_path,
        };
        if best_candidate
            .as_ref()
            .is_none_or(|best| candidate_better(&candidate, best))
        {
            best_candidate = Some(candidate);
        }
    }

    let mut chosen = best_candidate?;
    let current_path = if chosen.planned_attach_node == border_node {
        Vec::new()
    } else if let Some(path) = chosen.network_path.take() {
        if path.len() < 2 {
            return None;
        }
        path
    } else {
        pathfind_count.fetch_add(1, Ordering::Relaxed);
        let path = transit_network
            .cch_graph
            .find_path(
                chosen.planned_attach_node,
                border_node,
                usize::MAX,
                graph,
                TransitFlags::CAR,
            )
            .map(|(_, _, path)| path)?;
        if path.len() < 2 {
            return None;
        }
        path
    };

    Some(BuiltTripPlan {
        mode: MODE_CAR,
        target_building: usize::MAX,
        activity: 2,
        planned_attach_node: chosen.planned_attach_node,
        planned_detach_node: border_node,
        planned_attach_lane_id: chosen.planned_attach_lane_id,
        planned_detach_lane_id: usize::MAX,
        planned_attach_lane_d: chosen.planned_attach_lane_d,
        planned_detach_lane_d: 0.0,
        current_path,
        access_flags: ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION,
    })
}

/// Returns whether the ordinary building-origin trip planner can build this trip.
pub(crate) fn building_origin_trip_is_feasible(
    current_building: usize,
    target_building: usize,
    activity: u8,
    has_car: bool,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> bool {
    plan_building_origin_trip(
        current_building,
        target_building,
        activity,
        has_car,
        allocator,
        transit_network,
        graph,
        pathfind_count,
    )
    .is_some()
}

/// Estimates physical travel seconds, rounded up and saturated to the commute-cache range.
pub(crate) fn estimate_building_origin_trip_seconds(
    current_building: usize,
    target_building: usize,
    has_car: bool,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<u16> {
    let origin = BuildingTripEnds::origin(current_building, allocator, transit_network, graph);
    let destination =
        BuildingTripEnds::destination(target_building, allocator, transit_network, graph);
    match estimate_trip_seconds_between(
        &origin,
        &destination,
        has_car,
        transit_network,
        graph,
        pathfind_count,
        f32::INFINITY,
        f32::INFINITY,
    ) {
        TripEstimate::Seconds(seconds) => seconds,
        TripEstimate::OverBudget => unreachable!("an unbounded estimate has no budget"),
    }
}

/// Every walking and driving end of one building entrance, by node rank.
///
/// Built once per building and reused for each trip estimate to or from it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BuildingTripEnds {
    // Indexed by mode (`MODE_WALK`, `MODE_CAR`), then node rank.
    ends: [[Option<TripEnd>; 2]; 2],
}

impl BuildingTripEnds {
    /// Ends of trips that leave `building`; empty when it has no exact entrance.
    pub(crate) fn origin(
        building: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Self {
        Self::new(building, true, allocator, transit_network, graph)
    }

    /// Ends of trips that arrive at `building`; empty when it has no exact entrance.
    pub(crate) fn destination(
        building: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Self {
        Self::new(building, false, allocator, transit_network, graph)
    }

    fn new(
        building: usize,
        origin: bool,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Self {
        let mut ends = Self::default();
        if building >= allocator.buildings.len() || building >= allocator.entrances.len() {
            return ends;
        }
        let entrance = &allocator.entrances[building];
        if entrance.edge_idx >= graph.edge_count() || graph.edge(entrance.edge_idx).deleted {
            return ends;
        }
        for mode in [MODE_WALK, MODE_CAR] {
            for rank in NODE_RANKS {
                ends.ends[usize::from(mode)][usize::from(rank)] =
                    TripEnd::new(mode, rank, entrance, origin, transit_network, graph);
            }
        }
        ends
    }

    fn get(&self, mode: u8, rank: u8) -> Option<&TripEnd> {
        self.ends[usize::from(mode)][usize::from(rank)].as_ref()
    }

    fn present(&self) -> impl Iterator<Item = &TripEnd> {
        self.ends.iter().flatten().flatten()
    }

    /// Largest distance from `(x, y)` to a network node this building's trips can use.
    pub(crate) fn max_node_distance(&self, x: f32, y: f32) -> f32 {
        self.present()
            .map(|end| node_distance(end, x, y))
            .fold(0.0, f32::max)
    }

    /// Lower bound of the travel time from this origin to any destination `distance_m` from
    /// `(x, y)` whose own network nodes lie within `destination_node_slack_m` of it.
    ///
    /// Network legs are bounded by straight-line travel at `network_speed_bound_ms`, which must
    /// not be below any speed the route cost uses. Destination access counts as zero.
    pub(crate) fn lower_bound_seconds_to_distance(
        &self,
        has_car: bool,
        x: f32,
        y: f32,
        distance_m: f32,
        destination_node_slack_m: f32,
        network_speed_bound_ms: f32,
    ) -> f32 {
        // Within this reach a destination can share a node or a frontage with the origin, and
        // those trips skip the network.
        if distance_m <= self.max_node_distance(x, y) + destination_node_slack_m {
            return 0.0;
        }
        let modes: &[u8] = if has_car {
            &[MODE_WALK, MODE_CAR]
        } else {
            &[MODE_WALK]
        };
        let mut bound = f32::INFINITY;
        for &mode in modes {
            for rank in NODE_RANKS {
                let Some(end) = self.get(mode, rank) else {
                    continue;
                };
                let Some(departure_s) = end.network_departure_s() else {
                    continue;
                };
                let network_m = distance_m - node_distance(end, x, y) - destination_node_slack_m;
                bound = bound.min(departure_s + network_m / network_speed_bound_ms);
            }
        }
        bound
    }
}

fn node_distance(end: &TripEnd, x: f32, y: f32) -> f32 {
    let (node_x, node_z) = end.node_pos();
    (node_x - x).hypot(node_z - y)
}

/// Result of a commute estimate that may stop once it cannot come in under a budget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TripEstimate {
    /// The exact estimate: rounded-up seconds, or `None` when no mode reaches the destination.
    Seconds(Option<u16>),
    /// The trip takes longer than the budget whichever mode wins.
    OverBudget,
}

/// Estimates door-to-door seconds between two prepared buildings, as
/// [`estimate_building_origin_trip_seconds`] does, without network queries that cannot change the
/// result.
///
/// Every candidate first gets a lower bound from its exact access terms and straight-line travel
/// at `network_speed_bound_ms` between its network nodes. Candidates are routed in bound order and
/// skipped once their bound loses to the best mode-choice cost found, so the chosen candidate is
/// the one an exhaustive search picks. When every bound exceeds `budget_s` the result is
/// [`TripEstimate::OverBudget`] without any query.
pub(crate) fn estimate_trip_seconds_between(
    origin: &BuildingTripEnds,
    destination: &BuildingTripEnds,
    has_car: bool,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    network_speed_bound_ms: f32,
    budget_s: f32,
) -> TripEstimate {
    let mut best: Option<PlannedTripCandidate> = None;
    let mut networked = [(0.0_f32, MODE_WALK, 0_u8, 0_u8); 8];
    let mut networked_count = 0;
    let mut min_bound_s = f32::INFINITY;
    for mode in [MODE_WALK, MODE_CAR] {
        if mode == MODE_CAR && !has_car {
            continue;
        }
        for (origin_rank, destination_rank) in NODE_RANK_PAIRS {
            let (Some(origin_end), Some(destination_end)) = (
                origin.get(mode, origin_rank),
                destination.get(mode, destination_rank),
            ) else {
                continue;
            };
            match price_trip_without_network(
                mode,
                origin_rank,
                destination_rank,
                origin_end,
                destination_end,
                transit_network,
                graph,
                network_speed_bound_ms,
            ) {
                TripLeg::Priced(Some(candidate)) => {
                    min_bound_s = min_bound_s.min(candidate.total_cost_s);
                    if best
                        .as_ref()
                        .is_none_or(|best| candidate_better(&candidate, best))
                    {
                        best = Some(candidate);
                    }
                }
                TripLeg::Priced(None) => {}
                TripLeg::Network { lower_bound_s } => {
                    min_bound_s = min_bound_s.min(lower_bound_s);
                    networked[networked_count] = (
                        mode_choice_cost_for(mode, lower_bound_s),
                        mode,
                        origin_rank,
                        destination_rank,
                    );
                    networked_count += 1;
                }
            }
        }
    }
    if min_bound_s.is_finite() && exceeds_with_margin(min_bound_s, budget_s) {
        return TripEstimate::OverBudget;
    }

    let networked = &mut networked[..networked_count];
    networked.sort_unstable_by(|left, right| left.0.total_cmp(&right.0));
    for &(bound_mode_choice_s, mode, origin_rank, destination_rank) in networked.iter() {
        if best
            .as_ref()
            .is_some_and(|best| exceeds_with_margin(bound_mode_choice_s, best.mode_choice_cost_s))
        {
            // Later candidates have larger bounds still.
            break;
        }
        let (Some(origin_end), Some(destination_end)) = (
            origin.get(mode, origin_rank),
            destination.get(mode, destination_rank),
        ) else {
            continue;
        };
        if let Some(candidate) = combine_trip_ends(
            mode,
            origin_rank,
            destination_rank,
            origin_end,
            destination_end,
            transit_network,
            graph,
            pathfind_count,
        ) && best
            .as_ref()
            .is_none_or(|best| candidate_better(&candidate, best))
        {
            best = Some(candidate);
        }
    }
    TripEstimate::Seconds(
        best.map(|candidate| candidate.total_cost_s.ceil().clamp(1.0, u16::MAX as f32) as u16),
    )
}

// Route costs are f32 sums over many edges; a bound must clear the threshold by more than their
// rounding before a candidate is skipped.
fn exceeds_with_margin(bound_s: f32, threshold_s: f32) -> bool {
    bound_s > threshold_s + BOUND_ROUNDING_MARGIN * threshold_s.abs().max(1.0)
}
