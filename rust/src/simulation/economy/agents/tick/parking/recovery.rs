// SPDX-License-Identifier: GPL-2.0-only

//! Event-driven resident recovery preserves the driver's actual position and vehicle identity.

use super::*;
use crate::simulation::core::handles::Handle;
use crate::simulation::economy::agents::tick::lane_nav::{lane_origin_node, lane_terminal_node};
use crate::simulation::economy::agents::tick::movement::retry_after_s;
use crate::simulation::economy::agents::tick::planning::{
    BuiltNetworkReplan, ReplanFailure, plan_network_replan, quote_parking_leg_from_lane,
};
use crate::simulation::economy::agents::tick::trip_failures::{FailureCause, FailureOutcome};
use crate::simulation::economy::agents::{
    ACCESS_LANE_ATTACH_PENDING, ACTIVITY_HOME, TRANSIT_ACCESS_INGRESS, TRANSIT_NETWORK,
};
use crate::simulation::network::lanes::LaneType;
use crate::simulation::parking::Place;

// A rewritten itinerary keeps retired geometry only for the endpoints it still names. O(1).
fn release_superseded_links(parking: &mut ParkingSystem, trip: ParkingTrip) {
    parking.retain_retired_places(
        trip.vehicle,
        [trip.walk_origin, trip.source, trip.destination],
    );
}

impl AgentSystem {
    // The building agent `i`'s itinerary leads to, once its trip may have been taken from it
    // (`TRAFFIC-29`): the job or home it went to ended, or the building went. The destination it
    // holds comes first, a move home after a demolition; then its home; with neither it goes on
    // to the building it was going to, which still shelters it. With no building left it leaves
    // the car and `None` comes back. A car still on the road takes a bay at the new building if
    // one is free, else parks where it is booked and walks. O(k).
    fn parking_trip_target(
        &mut self,
        i: usize,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
    ) -> Option<usize> {
        let mut trip = self.agents.parking_trip[i];
        let named = allocator
            .building_handles
            .resolve(Handle::from_bits(trip.target));
        let live = |building: usize| (building < allocator.buildings.len()).then_some(building);
        let held = live(self.agents.target_building[i]);
        if named.is_some() && named == held {
            return named;
        }
        let home = live(self.agents.home_building[i]);
        let Some(target) = held.or(home).or(named) else {
            self.settle_without_car(i, parking);
            return None;
        };
        if Some(target) != named {
            trip.target = allocator.building_handles.handle(target).to_bits();
            if matches!(trip.phase, ParkingTripPhase::PullOut | ParkingTripPhase::Drive)
                && let Some(candidate) = parking.first_free_candidate(trip.target)
            {
                parking.cancel_reservation(trip.vehicle);
                if parking.reserve(trip.vehicle, candidate.place) {
                    trip.destination = candidate.place;
                }
            }
            self.agents.parking_trip[i] = trip;
            release_superseded_links(parking, trip);
        }
        if held.is_none() {
            crate::debug::work::add(crate::debug::work::Work::DriversHomeAfterCancel, 1);
            let a = &mut self.agents;
            a.target_building[i] = target;
            a.planned_target_building[i] = usize::MAX;
            a.journey_start_time[i] = self.sim_time;
            if Some(target) == home {
                a.activity[i] = ACTIVITY_HOME;
                a.planned_activity[i] = ACTIVITY_HOME;
            }
        }
        Some(target)
    }

    // No destination, no home and the building gone: the itinerary ends here. The car stays in
    // its bay, or from the road goes off the map until a bay takes it, and the driver settles
    // where it stands for its household's next home, as an evicted resident does. A delivery's
    // carrier retires instead. O(log W).
    fn settle_without_car(&mut self, i: usize, parking: &mut ParkingSystem) {
        let trip = self.agents.parking_trip[i];
        parking.cancel_reservation(trip.vehicle);
        parking.set_moving(trip.vehicle, false);
        parking.finish_outing(trip.vehicle, self.handles.handle(i));
        parking.clear_retired_places(trip.vehicle);
        parking.store_left_car(trip.vehicle);
        self.leave_lane_buckets(i);
        let a = &mut self.agents;
        a.parking_trip[i] = ParkingTrip {
            vehicle: trip.vehicle,
            arrival_ownership: trip.arrival_ownership,
            ..Default::default()
        };
        a.transit_mode[i] = MODE_WALK;
        a.transit[i] = TRANSIT_ACCESS_INGRESS;
        a.target_building[i] = usize::MAX;
        a.current_lane_id[i] = usize::MAX;
        a.lane_distance[i] = 0.0;
        if trip.delivery_only {
            self.completed_parking_deliveries.push(self.handles.handle(i));
        }
    }

    pub(super) fn recover_parking_trip(
        &mut self,
        i: usize,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.recover_parking_leg(i, parking, allocator, network, graph);
        // A failure the movement pass handed over and no outcome above reported: the leg was
        // planned again (`TRAFFIC-32`).
        self.record_parking_outcome(i, None, FailureOutcome::Replans);
    }

    fn recover_parking_leg(
        &mut self,
        i: usize,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let Some(target) = self.parking_trip_target(i, parking, allocator) else {
            return;
        };
        let mut trip = self.agents.parking_trip[i];
        // A pull-in that reached its bay needs no route: the walk to the door is next.
        if trip.phase == ParkingTripPhase::PullIn && trip.ready && !trip.replan_needed {
            return;
        }
        if trip.phase == ParkingTripPhase::WalkToCar && trip.ready && !trip.replan_needed {
            let Some(origin) = parking.place_entrance(trip.source, allocator, network, graph)
            else {
                self.queue_parking_retry(i, 30);
                return;
            };
            if let Some(actual) = parking.vehicle_place(trip.vehicle)
                && actual != trip.source
            {
                let Some(destination) = parking.place_entrance(actual, allocator, network, graph)
                else {
                    self.queue_parking_retry(i, 30);
                    return;
                };
                let Some(quote) = quote_parking_leg(
                    MODE_WALK,
                    &origin,
                    &destination,
                    [0.0; 2],
                    network,
                    graph,
                    &self.pathfind_count,
                    None,
                ) else {
                    self.queue_parking_retry(i, 30);
                    return;
                };
                let Some(plan) = self.materialize_parking_leg(
                    i,
                    &quote,
                    target,
                    self.agents.activity[i],
                    None,
                    network,
                    graph,
                ) else {
                    return;
                };
                trip.walk_origin = trip.source;
                trip.source = actual;
                trip.ready = false;
                trip.replan_needed = false;
                self.agents.parking_trip[i] = trip;
                release_superseded_links(parking, trip);
                self.apply_parking_leg(i, plan, &origin);
                return;
            }
            if parking.vehicle_place(trip.vehicle).is_some()
                && let Some(candidate) = parking.first_free_candidate(trip.target)
                && parking.reserve(trip.vehicle, candidate.place)
            {
                trip.destination = candidate.place;
                trip.replan_needed = false;
                self.agents.parking_trip[i] = trip;
                release_superseded_links(parking, trip);
                return;
            }
            // The reservation vanished before driving. Continue on foot from the actual bay.
            let Some(destination) = allocator.entrances.get(target) else {
                self.queue_parking_retry(i, 30);
                return;
            };
            let Some(quote) = quote_parking_leg(
                MODE_WALK,
                &origin,
                destination,
                [0.0; 2],
                network,
                graph,
                &self.pathfind_count,
                None,
            ) else {
                self.queue_parking_retry(i, 30);
                return;
            };
            let Some(plan) = self.materialize_parking_leg(
                i,
                &quote,
                target,
                self.agents.activity[i],
                None,
                network,
                graph,
            ) else {
                return;
            };
            parking.cancel_reservation(trip.vehicle);
            trip.destination = trip.source;
            trip.phase = ParkingTripPhase::WalkToDoor;
            trip.ready = false;
            trip.replan_needed = false;
            self.agents.parking_trip[i] = trip;
            release_superseded_links(parking, trip);
            self.apply_parking_leg(i, plan, &origin);
            return;
        }
        if matches!(
            trip.phase,
            ParkingTripPhase::PullOut | ParkingTripPhase::Drive | ParkingTripPhase::PullIn
        ) && !parking.trip_reserved(trip.vehicle, trip.destination)
        {
            let Some(candidate) = parking.first_free_candidate(trip.target) else {
                self.queue_parking_retry(i, 30);
                return;
            };
            if !parking.reserve(trip.vehicle, candidate.place) {
                self.queue_parking_retry(i, 30);
                return;
            }
            if trip.phase == ParkingTripPhase::PullIn {
                trip.source = trip.destination;
                trip.phase = ParkingTripPhase::PullOut;
            }
            trip.destination = candidate.place;
            self.agents.parking_trip[i] = trip;
            release_superseded_links(parking, trip);
        }
        let destination = if trip.phase == ParkingTripPhase::WalkToDoor {
            allocator.entrances.get(target).cloned()
        } else {
            parking.trip_entrance(trip, false, allocator, network, graph)
        };
        let Some(destination) = destination else {
            self.queue_parking_retry(i, 30);
            return;
        };
        if trip.phase == ParkingTripPhase::PullOut {
            let Some(origin) = parking.place_entrance(trip.source, allocator, network, graph)
            else {
                self.queue_parking_retry(i, 30);
                return;
            };
            let Some(quote) = quote_parking_leg(
                MODE_CAR,
                &origin,
                &destination,
                [
                    parking.maneuver_seconds(trip.source),
                    parking.maneuver_seconds(trip.destination),
                ],
                network,
                graph,
                &self.pathfind_count,
                None,
            ) else {
                self.queue_parking_retry(i, 30);
                return;
            };
            let Some(plan) = self.materialize_parking_leg(
                i,
                &quote,
                target,
                self.agents.activity[i],
                None,
                network,
                graph,
            ) else {
                return;
            };
            let position = (self.agents.pos_x[i], self.agents.pos_y[i]);
            self.apply_parking_leg(i, plan, &origin);
            self.agents.pos_x[i] = position.0;
            self.agents.pos_y[i] = position.1;
        } else if self.stands_at_a_node_on_its_drive(i, trip) {
            let start = (self.agents.current_node[i], self.agents.current_edge[i]);
            if let Err(failure) =
                self.plan_drive_from(i, start, target, destination, allocator, network, graph)
                && !self.drive_cut_off(
                    i, target, start, failure, parking, allocator, network, graph,
                )
            {
                return;
            }
        } else {
            let (lane, distance) = if self.agents.current_lane_id[i] != usize::MAX {
                (self.agents.current_lane_id[i], self.agents.lane_distance[i])
            } else if self.agents.access_flags[i] & ACCESS_LANE_ATTACH_PENDING != 0 {
                (
                    self.agents.planned_attach_lane_id[i] as usize,
                    self.agents.planned_attach_lane_d[i],
                )
            } else {
                self.queue_parking_retry(i, 30);
                return;
            };
            if network
                .lane_system
                .lanes
                .get(lane)
                .is_none_or(|l| l.edge_id == usize::MAX)
            {
                self.queue_parking_retry(i, 30);
                return;
            }
            let mode = self.agents.transit_mode[i];
            let pull_in = if mode == MODE_CAR {
                parking.maneuver_seconds(trip.destination)
            } else {
                0.0
            };
            let Some(quote) = quote_parking_leg_from_lane(
                mode,
                lane,
                distance,
                &destination,
                pull_in,
                network,
                graph,
                &self.pathfind_count,
            ) else {
                if trip.phase != ParkingTripPhase::Drive || mode != MODE_CAR {
                    self.queue_parking_retry(i, 30);
                    return;
                }
                if !self.drive_on_from_lane_end(
                    i,
                    lane,
                    target,
                    destination,
                    parking,
                    allocator,
                    network,
                    graph,
                ) {
                    return;
                }
                self.agents.parking_trip[i].ready = false;
                self.agents.parking_trip[i].replan_needed = false;
                self.agents.next_replan_time[i] = 0.0;
                return;
            };
            let Some(plan) = self.materialize_parking_leg(
                i,
                &quote,
                target,
                self.agents.activity[i],
                None,
                network,
                graph,
            ) else {
                return;
            };
            let a = &mut self.agents;
            a.planned_attach_node[i] = plan.planned_attach_node;
            a.planned_attach_lane_id[i] = plan.planned_attach_lane_id as u32;
            a.planned_attach_lane_d[i] = plan.planned_attach_lane_d;
            a.planned_detach_node[i] = plan.planned_detach_node;
            a.planned_detach_lane_id[i] = plan.planned_detach_lane_id as u32;
            a.planned_detach_lane_d[i] = plan.planned_detach_lane_d;
            a.current_path[i] = plan.current_path;
            a.walk_lanes[i].clear();
            a.current_path_index[i] = usize::from(a.current_path[i].len() >= 2);
            a.access_flags[i] =
                plan.access_flags | (a.access_flags[i] & ACCESS_LANE_ATTACH_PENDING);
            a.transit[i] = TRANSIT_NETWORK;
        }
        self.agents.parking_trip[i].ready = false;
        self.agents.parking_trip[i].replan_needed = false;
        self.agents.next_replan_time[i] = 0.0;
    }

    // Whether car `i` on its drive stands at a node with no lane and no attach point to plan
    // from: a road edit dropped it off the junction connector it was crossing
    // (`clear_route_plan_after_road_edit`), or the next road of its path went (`TRAFFIC-31`). O(1).
    fn stands_at_a_node_on_its_drive(&self, i: usize, trip: ParkingTrip) -> bool {
        let a = &self.agents;
        trip.phase == ParkingTripPhase::Drive
            && a.transit[i] == TRANSIT_NETWORK
            && a.current_lane_id[i] == usize::MAX
            && a.access_flags[i] & ACCESS_LANE_ATTACH_PENDING == 0
            && a.current_node[i] != u32::MAX
    }

    // Plans car `i`'s drive from `start` (a node and the road it came in by) to its bay, as the
    // movement pass's replan gate plans a car there (`replan_start`), and installs the route as
    // that gate does. From the node a car stands at (`TRAFFIC-31`) the movement pass then enters
    // the route's first road lane; from a road lane's end node the car drives on along its lane.
    // The car keeps its place and stands, speed 0, until it moves. O(Q), one route query; the
    // planner's rejection, with nothing changed, when no route reaches the bay.
    #[allow(clippy::too_many_arguments)]
    fn plan_drive_from(
        &mut self,
        i: usize,
        start: (u32, usize),
        target: usize,
        destination: BuildingEntrance,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Result<(), ReplanFailure> {
        let replan = plan_network_replan(
            start.0,
            start.1,
            None,
            target,
            MODE_CAR,
            0,
            allocator,
            network,
            graph,
            &self.pathfind_count,
            Some((i, "parking-recovery")),
            None,
            Some(destination),
        )?;
        self.install_drive(i, replan);
        Ok(())
    }

    // The replan gate's install (`apply_network_replan`) for a car in the parking commit; a car
    // held for its attach slot keeps waiting there under the new route. O(1).
    fn install_drive(&mut self, i: usize, replan: BuiltNetworkReplan) {
        let a = &mut self.agents;
        a.parking_trip[i].route_replaced(replan.current_path.len());
        a.current_path[i] = replan.current_path;
        a.walk_lanes[i].clear();
        a.current_path_index[i] = usize::from(a.current_path[i].len() >= 2);
        a.planned_detach_node[i] = replan.planned_detach_node;
        a.planned_detach_lane_id[i] = replan.planned_detach_lane_id as u32;
        a.planned_detach_lane_d[i] = replan.planned_detach_lane_d;
        a.access_flags[i] = replan.access_flags | (a.access_flags[i] & ACCESS_LANE_ATTACH_PENDING);
        a.network_replan_failures[i] = 0;
        a.speed[i] = 0.0;
    }

    // Car `i` on its drive found no route to its bay from where it stands on road lane `lane`:
    // it plans again from the lane's end node, as the replan gate plans a car on a lane, and with
    // no route from there either the cut-off rule decides (`drive_cut_off`). Returns whether the
    // car has a route. O(Q) plus the cut-off's queries.
    #[allow(clippy::too_many_arguments)]
    fn drive_on_from_lane_end(
        &mut self,
        i: usize,
        lane: usize,
        target: usize,
        destination: BuildingEntrance,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let Some(node) = lane_terminal_node(lane, network, graph) else {
            self.queue_parking_retry(i, 30);
            return false;
        };
        let start = (node, network.lane_system.lanes[lane].edge_id);
        match self.plan_drive_from(i, start, target, destination, allocator, network, graph) {
            Ok(()) => true,
            Err(failure) => self.drive_cut_off(
                i, target, start, failure, parking, allocator, network, graph,
            ),
        }
    }

    // `TRAFFIC-34`, the user's decision of 2026-11-10: car `i` on its drive finds no road route
    // from `start` to its booked bay (`failure`). It releases the booking and
    // (a) drives to the nearest free bay of where it was going that it can reach by road, parks
    //     there and the driver walks on;
    // (b) with none, gives the trip up and drives home, to the nearest free bay of home it can
    //     reach;
    // (c) with home cut off too, books the first free bay of its trip's end again (home, or with
    //     no home where it was going; its old bay when none is free), drives to the end of a road
    //     lane (`wait_at_lane_end`) and stands there for its retry on the stranded walkers'
    //     backoff (5 s, 30 s, then every 2 min; the step in `network_replan_failures`), which
    //     recovery runs as any other: from the lane's end, by these same rules.
    // It never stands in a junction and is never moved. A failure records once, with what it did:
    // a retry on the backoff records nothing. Returns whether the car has a route. Cost: the
    // searches' route queries, at most one per distinct exit lane of the two catchments (an exit
    // lane the router cannot reach from `start` is not asked again), no city scan.
    #[allow(clippy::too_many_arguments)]
    fn drive_cut_off(
        &mut self,
        i: usize,
        target: usize,
        start: (u32, usize),
        failure: ReplanFailure,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let step = self.agents.network_replan_failures[i];
        if step == 0 {
            // Taken where the car stands and where it was going, before the trip changes; the
            // outcome below completes it.
            self.record_parking_outcome(
                i,
                Some(FailureCause::NoRoute(failure)),
                FailureOutcome::ParkingRecovery,
            );
        }
        let trip = self.agents.parking_trip[i];
        let booked = trip.destination;
        parking.cancel_reservation(trip.vehicle);
        // The booked bay itself failed from this very start.
        if let Some(bay) = self.drive_to_nearest_bay(
            i,
            trip.target,
            target,
            start,
            booked,
            parking,
            allocator,
            network,
            graph,
        ) {
            let at = parking.place_curb(bay).unwrap_or_default();
            self.record_parking_outcome(i, None, FailureOutcome::ParksNearby { at });
            return true;
        }
        let home = self.agents.home_building[i];
        if home < allocator.buildings.len() && home != target {
            let handle = allocator.building_handles.handle(home).to_bits();
            self.turn_trip_home(i, home, handle);
            if self
                .drive_to_nearest_bay(
                    i,
                    handle,
                    home,
                    start,
                    Handle::NONE,
                    parking,
                    allocator,
                    network,
                    graph,
                )
                .is_some()
            {
                self.record_parking_outcome(i, None, FailureOutcome::DrivesHome { building: home });
                return true;
            }
        }
        let mut trip = self.agents.parking_trip[i];
        let bay = parking
            .first_free_candidate(trip.target)
            .map_or(booked, |candidate| candidate.place);
        if parking.reserve(trip.vehicle, bay) {
            trip.destination = bay;
        }
        self.agents.parking_trip[i] = trip;
        release_superseded_links(parking, trip);
        let lane = self.wait_at_lane_end(i, network, graph);
        let after_s = retry_after_s(step);
        self.agents.network_replan_failures[i] = step.saturating_add(1);
        self.record_parking_outcome(i, None, FailureOutcome::WaitsAtLaneEnd { lane, after_s });
        self.queue_parking_retry(i, after_s as u16);
        false
    }

    // `TRAFFIC-34`, the user's rule of 2026-11-10: driver `i` has pulled into its bay (`bay`, its
    // entrance) and finds no walk from there to `target`. It takes the car again and drives home,
    // to the first free bay of home, as a trip home books one. With home cut off by road too, or
    // home being where it could not walk to, or no home, the car stays in its bay, off the road,
    // and the pull-in's next leg is planned again on the stranded walkers' backoff (5 s, 30 s,
    // then every 2 min): the walk on if it is back, else the drive home. This replaces the walker
    // rule's walk home or stranding for a driver who came by car. A failure records once: a retry
    // on the backoff records nothing. O(k + Q), one car leg quote.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn walk_cut_off(
        &mut self,
        i: usize,
        target: usize,
        bay: &BuildingEntrance,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let step = self.agents.network_replan_failures[i];
        // Recorded with each outcome below; a retry for the route's buffer records nothing yet.
        let cause = (step == 0).then_some(FailureCause::NoWalkFromBay);
        let mut trip = self.agents.parking_trip[i];
        let home = self.agents.home_building[i];
        let handle = (home < allocator.buildings.len() && home != target)
            .then(|| allocator.building_handles.handle(home).to_bits());
        let drive = handle
            .and_then(|handle| parking.first_free_candidate(handle))
            .and_then(|candidate| {
                let destination =
                    parking.place_entrance(candidate.place, allocator, network, graph)?;
                let maneuvers = [
                    parking.maneuver_seconds(trip.destination),
                    parking.maneuver_seconds(candidate.place),
                ];
                let quote = quote_parking_leg(
                    MODE_CAR,
                    bay,
                    &destination,
                    maneuvers,
                    network,
                    graph,
                    &self.pathfind_count,
                    None,
                )?;
                Some((candidate.place, quote))
            });
        let (Some(handle), Some((home_bay, quote))) = (handle, drive) else {
            let after_s = retry_after_s(step);
            self.agents.network_replan_failures[i] = step.saturating_add(1);
            self.record_parking_outcome(i, cause, FailureOutcome::StaysParked { after_s });
            self.queue_parking_retry(i, after_s as u16);
            return;
        };
        let Some(plan) =
            self.materialize_parking_leg(i, &quote, home, ACTIVITY_HOME, None, network, graph)
        else {
            return;
        };
        // The car parks in its bay, from which the pull-out leaves for home.
        if !parking.occupy(trip.vehicle, trip.destination)
            || !parking.reserve(trip.vehicle, home_bay)
        {
            self.agents.current_path[i] = plan.current_path;
            self.queue_parking_retry(i, 30);
            return;
        }
        parking.set_moving(trip.vehicle, true);
        trip.source = trip.destination;
        trip.destination = home_bay;
        trip.phase = ParkingTripPhase::PullOut;
        trip.ready = false;
        trip.replan_needed = false;
        self.agents.parking_trip[i] = trip;
        release_superseded_links(parking, trip);
        // Recorded with where it was going, before the trip turns home.
        self.record_parking_outcome(i, cause, FailureOutcome::DrivesHome { building: home });
        self.turn_trip_home(i, home, handle);
        self.apply_parking_leg(i, plan, bay);
    }

    // The trip turns home, as a trip taken on the way does (`parking_trip_target`). O(1).
    fn turn_trip_home(&mut self, i: usize, home: usize, handle: u64) {
        let now = self.sim_time;
        let a = &mut self.agents;
        a.parking_trip[i].target = handle;
        a.target_building[i] = home;
        a.planned_target_building[i] = usize::MAX;
        a.journey_start_time[i] = now;
        a.activity[i] = ACTIVITY_HOME;
        a.planned_activity[i] = ACTIVITY_HOME;
    }

    // The first free bay of `building`'s catchment, in its own order, that car `i` reaches by road
    // from `start`, booked and its route installed; `skip` is not tried. A bay whose exit lanes
    // the router has already found unreachable from `start` (`cch_no_path`) is not asked again:
    // that answer depends only on the start and the exit lane, so the skip is exact. O(k + R × Q)
    // for k catchment entries and R distinct exit lanes tried; no allocation beyond the found
    // route.
    #[allow(clippy::too_many_arguments)]
    fn drive_to_nearest_bay(
        &mut self,
        i: usize,
        handle: u64,
        building: usize,
        start: (u32, usize),
        skip: Handle<Place>,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<Handle<Place>> {
        let mut unreachable = [usize::MAX; 16];
        let mut known = 0;
        let mut found = None;
        for candidate in parking.free_candidates(handle) {
            if candidate.place == skip {
                continue;
            }
            let Some(entrance) = parking.place_entrance(candidate.place, allocator, network, graph)
            else {
                continue;
            };
            if [entrance.car_lane_fwd, entrance.car_lane_bkw]
                .iter()
                .all(|lane| *lane == usize::MAX || unreachable[..known].contains(lane))
            {
                continue;
            }
            match plan_network_replan(
                start.0,
                start.1,
                None,
                building,
                MODE_CAR,
                0,
                allocator,
                network,
                graph,
                &self.pathfind_count,
                Some((i, "parking-cut-off")),
                None,
                Some(entrance),
            ) {
                Ok(replan) => {
                    found = Some((candidate.place, replan));
                    break;
                }
                Err(failure) => {
                    for rejected in failure.candidates.iter().flatten() {
                        let lane = rejected.planned_detach_lane_id;
                        if rejected.reason == "cch_no_path"
                            && known < unreachable.len()
                            && !unreachable[..known].contains(&lane)
                        {
                            unreachable[known] = lane;
                            known += 1;
                        }
                    }
                }
            }
        }
        let (bay, replan) = found?;
        let mut trip = self.agents.parking_trip[i];
        if !parking.reserve(trip.vehicle, bay) {
            return None;
        }
        trip.destination = bay;
        self.agents.parking_trip[i] = trip;
        release_superseded_links(parking, trip);
        self.install_drive(i, replan);
        Some(bay)
    }

    // (c) of `drive_cut_off`: car `i` drops its route and drives to the end of a road lane, where
    // it stands for its retry: the lane it is on or held for, or, at a node, the road lane out of
    // it whose start is nearest the car (ties by lane id), which it enters as a held car enters
    // its attach lane (`hold_for_lane_attach`), so it leaves the junction without being moved.
    // The lane's end freezes it (`car_trip_taken`); one already there, or at a node with no road
    // out, is frozen now. Returns the lane, `usize::MAX` for none. O(roads at the node × lanes).
    fn wait_at_lane_end(
        &mut self,
        i: usize,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> usize {
        let a = &mut self.agents;
        a.current_path[i].clear();
        a.walk_lanes[i].clear();
        a.current_path_index[i] = 0;
        a.planned_detach_node[i] = u32::MAX;
        a.planned_detach_lane_id[i] = u32::MAX;
        a.planned_detach_lane_d[i] = 0.0;
        a.access_flags[i] &= ACCESS_LANE_ATTACH_PENDING;
        a.speed[i] = 0.0;
        let current = a.current_lane_id[i];
        let (lane, there) = if let Some(value) = network.lane_system.lanes.get(current) {
            (current, a.lane_distance[i] >= value.length)
        } else if a.access_flags[i] & ACCESS_LANE_ATTACH_PENDING != 0 {
            (a.planned_attach_lane_id[i] as usize, false)
        } else {
            let node = a.current_node[i];
            match nearest_lane_out(node, (a.pos_x[i], a.pos_y[i]), network, graph) {
                Some(out) => {
                    a.hold_for_lane_attach(i, &network.lane_system, out, 0.0, node);
                    (out, false)
                }
                None => (usize::MAX, true),
            }
        };
        let trip = &mut a.parking_trip[i];
        trip.ready = there;
        trip.replan_needed = there;
        lane
    }
}

// The vehicle road lane out of `node` whose start is nearest `pos`, ties by lane id. O(roads at
// the node × lanes per road), allocation-free.
fn nearest_lane_out(
    node: u32,
    pos: (f32, f32),
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<usize> {
    if node as usize >= graph.node_adjacency_count() {
        return None;
    }
    let lanes = &network.lane_system.lanes;
    let mut best: Option<(f32, usize)> = None;
    for &edge in graph.node_adjacency(node) {
        if graph.get_edge(edge).is_none_or(|value| value.deleted) {
            continue;
        }
        for &lane in network
            .lane_system
            .edge_lanes
            .get(&edge)
            .into_iter()
            .flatten()
        {
            let value = &lanes[lane];
            let Some(first) = value.geometry.first() else {
                continue;
            };
            if value.lane_type != LaneType::Vehicle
                || lane_origin_node(lane, network, graph) != Some(node)
            {
                continue;
            }
            let d2 = (first.x - pos.0).powi(2) + (first.z - pos.1).powi(2);
            if best.is_none_or(|(best_d2, best_lane)| {
                d2.total_cmp(&best_d2).then(lane.cmp(&best_lane)).is_lt()
            }) {
                best = Some((d2, lane));
            }
        }
    }
    best.map(|(_, lane)| lane)
}
