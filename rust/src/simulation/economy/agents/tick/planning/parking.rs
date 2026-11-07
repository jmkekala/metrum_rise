// SPDX-License-Identifier: GPL-2.0-only

//! Parking endpoint quotes and reusable-route materialization using ordinary exact routing.

use super::super::super::{ACCESS_PLAN_VALID, ACCESS_ZERO_HOP_NODE_PATH, MODE_CAR};
use super::candidate::{
    PlannedTripCandidate, RouteStart, SharedNetworkQuotes, TripQueryScratch, parking_candidate,
};
use super::types::BuiltTripPlan;
use crate::simulation::buildings::allocator::BuildingEntrance;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use std::sync::atomic::AtomicU32;

/// Exact endpoint quote. O(Q) route work for four endpoint pairs, independent of parking
/// candidate count. Drive costs use free-flow weights and fixed swept paths, never live traffic.
#[allow(clippy::too_many_arguments)]
pub(crate) fn quote_parking_leg(
    mode: u8,
    origin: &BuildingEntrance,
    destination: &BuildingEntrance,
    maneuvers: [f32; 2],
    network: &TransitNetwork,
    graph: &RegionGraph,
    count: &AtomicU32,
    shared: Option<&SharedNetworkQuotes>,
) -> Option<ParkingLegQuote> {
    parking_candidate(
        mode,
        origin,
        destination,
        maneuvers,
        network,
        graph,
        count,
        None,
        shared,
    )
    .map(ParkingLegQuote)
}

/// Proven endpoint pair; no persistent route allocation until the phase starts.
#[derive(Clone)]
pub(crate) struct ParkingLegQuote(PlannedTripCandidate);

impl ParkingLegQuote {
    /// The quote's every field as bits, to compare two quotes exactly.
    #[cfg(test)]
    pub(crate) fn bits(&self) -> [u64; 9] {
        let c = &self.0;
        [
            u64::from(c.total_cost_s.to_bits()),
            u64::from(c.mode_choice_cost_s.to_bits()),
            u64::from(c.origin_rank) << 8 | u64::from(c.destination_rank),
            u64::from(c.mode),
            u64::from(c.planned_attach_node) << 32 | u64::from(c.planned_detach_node),
            c.planned_attach_lane_id as u64,
            c.planned_detach_lane_id as u64,
            u64::from(c.planned_attach_lane_d.to_bits()),
            u64::from(c.planned_detach_lane_d.to_bits()),
        ]
    }

    /// Physical travel seconds, excluding the unchanged mode-choice preference penalty.
    pub(crate) fn seconds(&self) -> f32 {
        self.0.total_cost_s
    }

    /// Cost with the existing car preference overhead; walking wins an exact tie.
    pub(crate) fn mode_choice_seconds(&self) -> f32 {
        self.0.mode_choice_cost_s
    }

    // Whether the leg stays on one lane, and whether it joins at a node with no route between.
    fn without_route(&self, network: &TransitNetwork) -> (bool, bool) {
        let c = &self.0;
        let direct = c.planned_attach_lane_id == c.planned_detach_lane_id
            && c.planned_attach_lane_d <= c.planned_detach_lane_d + 1e-6;
        let zero = !direct
            && c.planned_attach_node == c.planned_detach_node
            && (c.mode != MODE_CAR
                || network.cch_graph.vehicle_turn_permitted(
                    c.planned_attach_node,
                    network.lane_system.lanes[c.planned_attach_lane_id].edge_id,
                    network.lane_system.lanes[c.planned_detach_lane_id].edge_id,
                ));
        (direct, zero)
    }

    // The leg's node route, handed to `consume`, in the running thread's scratch. O(Q).
    fn route<T>(
        &self,
        network: &TransitNetwork,
        graph: &RegionGraph,
        consume: impl FnOnce(&[u32]) -> T,
    ) -> Result<T, &'static str> {
        let c = &self.0;
        TripQueryScratch::with_routes(|scratch| {
            scratch
                .network_path(
                    c.mode,
                    c.planned_attach_node,
                    c.planned_detach_node,
                    RouteStart::Lane(c.planned_attach_lane_id),
                    c.planned_detach_lane_id,
                    network,
                    graph,
                    consume,
                    None,
                )
                .map(|(_, value)| value)
        })
    }

    /// Appends the nodes [`Self::materialize`] would write to `nodes`, false when the leg has no
    /// route. O(route length + Q); `nodes` grows only past its largest tick so far.
    pub(crate) fn append_route(
        &self,
        nodes: &mut Vec<u32>,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let (direct, zero) = self.without_route(network);
        direct || zero || self.route(network, graph, |path| nodes.extend_from_slice(path)).is_ok()
    }

    /// Writes the next leg into the agent's reusable route buffer; O(route length + Q), or
    /// O(route length) with `known`, the leg's [`Self::append_route`] answer (`None` inside for
    /// no route). Capacity is prepared at a lifecycle boundary; overflow returns the needed size
    /// without allocating or truncating. The caller schedules preparation before retrying.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn materialize(
        &self,
        target: usize,
        activity: u8,
        route: &mut Vec<u32>,
        prepared_nodes: usize,
        known: Option<Option<&[u32]>>,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Result<BuiltTripPlan, usize> {
        let c = &self.0;
        let (direct, zero) = self.without_route(network);
        route.clear();
        if !direct && !zero {
            let mut accept = |path: &[u32]| {
                if prepared_nodes < path.len() {
                    return Err(path.len());
                }
                assert!(
                    route.capacity() >= path.len(),
                    "parking route preparation must precede movement"
                );
                route.extend_from_slice(path);
                Ok(())
            };
            #[cfg(all(test, debug_assertions))]
            if let Some(known) = known {
                let fresh = self.route(network, graph, <[u32]>::to_vec).ok();
                assert_eq!(known, fresh.as_deref(), "a prepared route differs from the router's");
            }
            match known {
                Some(Some(path)) => accept(path)?,
                Some(None) => return Err(0),
                None => self.route(network, graph, accept).map_err(|_| 0_usize)??,
            }
        }
        Ok(BuiltTripPlan {
            mode: c.mode,
            target_building: target,
            activity,
            planned_attach_node: c.planned_attach_node,
            planned_detach_node: c.planned_detach_node,
            planned_attach_lane_id: c.planned_attach_lane_id,
            planned_detach_lane_id: c.planned_detach_lane_id,
            planned_attach_lane_d: c.planned_attach_lane_d,
            planned_detach_lane_d: c.planned_detach_lane_d,
            current_path: std::mem::take(route),
            access_flags: ACCESS_PLAN_VALID | if zero { ACCESS_ZERO_HOP_NODE_PATH } else { 0 },
        })
    }
}

/// Quotes recovery from the actual road/foot-lane position. O(Q), four endpoint pairs;
/// no relocation and no allocation. Connector recovery waits for the existing road-lane exit.
#[allow(clippy::too_many_arguments)]
pub(crate) fn quote_parking_leg_from_lane(
    mode: u8,
    lane_id: usize,
    distance: f32,
    destination: &BuildingEntrance,
    pull_in_s: f32,
    network: &TransitNetwork,
    graph: &RegionGraph,
    count: &AtomicU32,
) -> Option<ParkingLegQuote> {
    let lane = network.lane_system.lanes.get(lane_id)?;
    let edge = graph.get_edge(lane.edge_id)?;
    let point = crate::simulation::buildings::allocator::BuildingAllocator::sample_pos_on_lane(
        lane, distance,
    );
    let mut origin = BuildingEntrance {
        edge_idx: lane.edge_id,
        side: if lane.is_fwd { -1 } else { 1 },
        door_pos: point,
        curb_pos: point,
        entrance_s_m:
            crate::simulation::buildings::allocator::BuildingAllocator::project_point_to_polyline_s(
                &edge.physical_geometry,
                point,
            ),
        ..Default::default()
    };
    if mode == MODE_CAR {
        if lane.is_fwd {
            origin.car_lane_fwd = lane_id;
        } else {
            origin.car_lane_bkw = lane_id;
        }
    } else if lane.is_fwd {
        origin.foot_lane_fwd = lane_id;
    } else {
        origin.foot_lane_bkw = lane_id;
    }
    parking_candidate(
        mode,
        &origin,
        destination,
        [0.0, pull_in_s],
        network,
        graph,
        count,
        Some((lane_id, distance)),
        None,
    )
    .map(ParkingLegQuote)
}

/// Proven complete car itinerary without claims or route-vector allocation.
/// Destination selection is O(k); the three exact route quotes cost O(Q), independent of k.
#[derive(Clone)]
pub(crate) struct ParkingQuote {
    /// The household car's actual occupied bay.
    pub(crate) source: crate::simulation::core::handles::Handle<crate::simulation::parking::Place>,
    /// First free fitting bay in the destination's published walking catchment.
    pub(crate) destination:
        crate::simulation::core::handles::Handle<crate::simulation::parking::Place>,
    /// First pedestrian leg, ready for the ordinary reusable route materializer.
    pub(crate) to_car: ParkingLegQuote,
    /// Physical travel seconds for all five legs, excluding the mode-choice preference.
    pub(crate) seconds: f32,
    /// Complete cost with the unchanged driving preference penalty.
    pub(crate) mode_choice_seconds: f32,
}

#[cfg(test)]
impl ParkingQuote {
    /// The quote's every field as bits, to compare two quotes exactly.
    pub(crate) fn bits(&self) -> ([u64; 4], [u64; 9]) {
        let plain = [
            self.source.to_bits(),
            self.destination.to_bits(),
            u64::from(self.seconds.to_bits()),
            u64::from(self.mode_choice_seconds.to_bits()),
        ];
        (plain, self.to_car.bits())
    }
}

/// Quotes all five resident legs at current supply and custody, without reserving. O(k + Q)
/// with prepared routing scratch and no per-candidate route queries or allocations. Callers
/// pass adult eligibility separately; a child can own a car but cannot borrow it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn quote_parking_trip(
    car: crate::simulation::core::handles::Handle<crate::simulation::parking::Vehicle>,
    borrower: crate::simulation::economy::agents::AgentHandle,
    eligible: bool,
    origin: usize,
    destination: usize,
    parking: &crate::simulation::parking::ParkingSystem,
    allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    count: &AtomicU32,
    shared: Option<&SharedNetworkQuotes>,
) -> Option<ParkingQuote> {
    let (source, target) = parking_trip_bays(car, borrower, eligible, destination, parking, allocator)?;
    quote_parking_between_places(
        source,
        target,
        origin,
        destination,
        parking,
        allocator,
        network,
        graph,
        count,
        shared,
    )
}

/// The car's bay and the first free bay at `destination`, the only inputs of a trip quote that
/// current supply and custody decide, or `None` when the car cannot make the trip. O(k).
pub(crate) fn parking_trip_bays(
    car: crate::simulation::core::handles::Handle<crate::simulation::parking::Vehicle>,
    borrower: crate::simulation::economy::agents::AgentHandle,
    eligible: bool,
    destination: usize,
    parking: &crate::simulation::parking::ParkingSystem,
    allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
) -> Option<ParkingTripBays> {
    if !eligible || !parking.available_to(car, borrower) {
        return None;
    }
    let source = parking.vehicle_place(car)?;
    let candidate =
        parking.first_free_candidate(allocator.building_handles.handle(destination).to_bits())?;
    Some((source, candidate.place))
}

/// The bay a car trip leaves from and the bay it parks in.
pub(crate) type ParkingTripBays = (
    crate::simulation::core::handles::Handle<crate::simulation::parking::Place>,
    crate::simulation::core::handles::Handle<crate::simulation::parking::Place>,
);

/// Quotes all five legs between two given bays. O(Q), a pure read of the road and of the bays'
/// published access: occupancy plays no part in it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn quote_parking_between_places(
    source: crate::simulation::core::handles::Handle<crate::simulation::parking::Place>,
    target: crate::simulation::core::handles::Handle<crate::simulation::parking::Place>,
    origin: usize,
    destination: usize,
    parking: &crate::simulation::parking::ParkingSystem,
    allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    count: &AtomicU32,
    shared: Option<&SharedNetworkQuotes>,
) -> Option<ParkingQuote> {
    use super::super::super::MODE_WALK;
    let from = parking.place_entrance(source, allocator, network, graph)?;
    let to = parking.place_entrance(target, allocator, network, graph)?;
    let drive = quote_parking_leg(
        MODE_CAR,
        &from,
        &to,
        [
            parking.maneuver_seconds(source),
            parking.maneuver_seconds(target),
        ],
        network,
        graph,
        count,
        shared,
    )?;
    let to_car = quote_parking_leg(
        MODE_WALK,
        allocator.entrances.get(origin)?,
        &from,
        [0.0; 2],
        network,
        graph,
        count,
        shared,
    )?;
    let to_door = quote_parking_leg(
        MODE_WALK,
        &to,
        allocator.entrances.get(destination)?,
        [0.0; 2],
        network,
        graph,
        count,
        shared,
    )?;
    let walking = to_car.seconds() + to_door.seconds();
    Some(ParkingQuote {
        source,
        destination: target,
        seconds: walking + drive.seconds(),
        mode_choice_seconds: walking + drive.mode_choice_seconds(),
        to_car,
    })
}

/// Estimates the same complete itinerary offered at departure, without claiming supply.
/// O(k + Q), with walking winning a mode-cost tie; physical seconds exclude preference overhead.
#[allow(clippy::too_many_arguments)]
pub(crate) fn estimate_parking_trip_seconds(
    origin: usize,
    destination: usize,
    car: crate::simulation::core::handles::Handle<crate::simulation::parking::Vehicle>,
    eligible: bool,
    parking: &crate::simulation::parking::ParkingSystem,
    allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    count: &AtomicU32,
    shared: Option<&SharedNetworkQuotes>,
) -> Option<u16> {
    match estimate_parking_trip_seconds_within(
        origin,
        destination,
        car,
        eligible,
        parking,
        allocator,
        network,
        graph,
        count,
        f32::INFINITY,
        shared,
    ) {
        super::trip::TripEstimate::Seconds(seconds) => seconds,
        super::trip::TripEstimate::OverBudget => None,
    }
}

/// Exact resident mode choice, bounded by physical travel time; O(k + Q), no claims or allocation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn estimate_parking_trip_seconds_within(
    origin: usize,
    destination: usize,
    car: crate::simulation::core::handles::Handle<crate::simulation::parking::Vehicle>,
    eligible: bool,
    parking: &crate::simulation::parking::ParkingSystem,
    allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    count: &AtomicU32,
    budget_s: f32,
    shared: Option<&SharedNetworkQuotes>,
) -> super::trip::TripEstimate {
    use super::trip::TripEstimate;
    let Some(from) = allocator.entrances.get(origin) else {
        return TripEstimate::Seconds(None);
    };
    let Some(to) = allocator.entrances.get(destination) else {
        return TripEstimate::Seconds(None);
    };
    let walk = quote_parking_leg(
        super::super::super::MODE_WALK,
        from,
        to,
        [0.0; 2],
        network,
        graph,
        count,
        shared,
    );
    let borrower = parking
        .borrowed_by(car)
        .unwrap_or(crate::simulation::core::handles::Handle::NONE);
    let car = quote_parking_trip(
        car,
        borrower,
        eligible,
        origin,
        destination,
        parking,
        allocator,
        network,
        graph,
        count,
        shared,
    );
    let seconds = match (walk, car) {
        (Some(walk), Some(car)) if walk.seconds() > car.mode_choice_seconds => car.seconds,
        (Some(walk), _) => walk.seconds(),
        (None, Some(car)) => car.seconds,
        _ => return TripEstimate::Seconds(None),
    };
    if seconds > budget_s {
        return TripEstimate::OverBudget;
    }
    TripEstimate::Seconds(Some(seconds.ceil().min(f32::from(u16::MAX)) as u16))
}

/// Immutable actor context for resident economy quotes. O(1) construction; no custody mutation.
#[derive(Clone, Copy)]
pub(crate) struct ResidentMobility<'a> {
    /// Live parking supply and custody.
    pub(crate) parking: &'a crate::simulation::parking::ParkingSystem,
    /// The household's unique vehicle, including NONE for a carless household.
    pub(crate) car: crate::simulation::core::handles::Handle<crate::simulation::parking::Vehicle>,
    /// Adult borrower eligibility at this planning boundary.
    pub(crate) eligible: bool,
    /// Network legs shared with the parking phase; every leg they hold is exact.
    pub(crate) quotes: &'a SharedNetworkQuotes,
}

impl<'a> ResidentMobility<'a> {
    /// Derives availability from the adult's actual household car and stable identity, O(1).
    pub(crate) fn for_agent(
        parking: &'a crate::simulation::parking::ParkingSystem,
        agents: &'a crate::simulation::economy::agents::AgentSystem,
        agent: usize,
    ) -> Self {
        let car = agents.parking_trip[agent].vehicle;
        let eligible = agents.age_group[agent] != crate::simulation::economy::agents::AGE_CHILD
            && parking.available_to(car, agents.handles.handle(agent));
        Self {
            parking,
            car,
            eligible,
            quotes: &agents.parking_work.quotes,
        }
    }

    /// Actual bay's road components, or the empty set when the car cannot be borrowed.
    /// O(1) entrance/component lookup; the exact quote proves the origin walk.
    pub(crate) fn components(
        &self,
        _origin: usize,
        index: &crate::simulation::economy::accessibility::ModeComponentIndex,
        allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> crate::simulation::economy::accessibility::BuildingModeComponents {
        let source = self
            .eligible
            .then(|| self.parking.vehicle_place(self.car))
            .flatten();
        source
            .and_then(|source| {
                self.parking
                    .place_entrance(source, allocator, network, graph)
            })
            .map_or_default(|entrance| {
                index.entrance_components(
                    &entrance,
                    graph,
                    crate::simulation::network::types::TransitFlags::CAR,
                )
            })
    }

    /// Exact preferred physical time for a resident leg, O(k + Q), without claiming a bay.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn seconds(
        &self,
        origin: usize,
        destination: usize,
        allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        count: &AtomicU32,
    ) -> Option<u16> {
        estimate_parking_trip_seconds(
            origin,
            destination,
            self.car,
            self.eligible,
            self.parking,
            allocator,
            network,
            graph,
            count,
            Some(self.quotes),
        )
    }

    /// Proves outward and return feasibility using the proposed parked bay on the return.
    /// O(k_origin + k_destination + Q), constant route queries, no claims or allocations.
    /// If the outward mode walks, the car remains at its original bay. The departure bay is
    /// free for the hypothetical return; all other occupancy is read from live supply.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn round_trip(
        &self,
        origin: usize,
        destination: usize,
        allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        count: &AtomicU32,
    ) -> bool {
        use super::super::super::MODE_WALK;
        let (Some(from), Some(to)) = (
            allocator.entrances.get(origin),
            allocator.entrances.get(destination),
        ) else {
            return false;
        };
        let shared = Some(self.quotes);
        let walk = quote_parking_leg(MODE_WALK, from, to, [0.0; 2], network, graph, count, shared);
        // Walking both ways proves the round trip whatever the car would do.
        let walk_back =
            quote_parking_leg(MODE_WALK, to, from, [0.0; 2], network, graph, count, shared).is_some();
        if walk.is_some() && walk_back {
            return true;
        }
        let car = quote_parking_trip(
            self.car,
            self.parking
                .borrowed_by(self.car)
                .unwrap_or(crate::simulation::core::handles::Handle::NONE),
            self.eligible,
            origin,
            destination,
            self.parking,
            allocator,
            network,
            graph,
            count,
            shared,
        );
        let driving = car.as_ref().filter(|car| {
            walk.as_ref()
                .is_none_or(|walk| walk.seconds() > car.mode_choice_seconds)
        });
        if walk.is_none() && driving.is_none() {
            return false;
        }
        if walk_back {
            return true;
        }
        if let Some(car) = driving {
            let Some(target) = self.parking.first_free_candidate_after_departure(
                allocator.building_handles.handle(origin).to_bits(),
                car.source,
            ) else {
                return false;
            };
            quote_parking_between_places(
                car.destination,
                target.place,
                destination,
                origin,
                self.parking,
                allocator,
                network,
                graph,
                count,
                shared,
            )
            .is_some()
        } else {
            self.seconds(destination, origin, allocator, network, graph, count)
                .is_some()
        }
    }
}
