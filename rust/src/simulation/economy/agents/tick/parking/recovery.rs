// SPDX-License-Identifier: GPL-2.0-only

//! Event-driven resident recovery preserves the driver's actual position and vehicle identity.

use super::*;
use crate::simulation::core::handles::Handle;
use crate::simulation::economy::agents::tick::planning::{
    plan_network_replan, quote_parking_leg_from_lane,
};
use crate::simulation::economy::agents::{
    ACCESS_LANE_ATTACH_PENDING, ACTIVITY_HOME, TRANSIT_ACCESS_INGRESS, TRANSIT_NETWORK,
};

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
            if !self.plan_drive_from_node(i, target, destination, allocator, network, graph) {
                self.queue_parking_retry(i, 30);
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

    // Plans car `i`'s drive from the node it stands at to its bay, as the movement pass's replan
    // gate plans any car there (`replan_start`), and installs the route as that gate does; the
    // movement pass then enters the route's first road lane from the node. The car keeps its
    // place and stands, speed 0, until that entry. Without this a car frozen for recovery at a
    // node waited for a lane nothing gave it, a retry every 30 game minutes for good
    // (`TRAFFIC-31`). O(Q), one route query; false, with nothing changed, when no route reaches
    // the bay.
    fn plan_drive_from_node(
        &mut self,
        i: usize,
        target: usize,
        destination: BuildingEntrance,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let Ok(replan) = plan_network_replan(
            self.agents.current_node[i],
            self.agents.current_edge[i],
            None,
            target,
            MODE_CAR,
            0,
            allocator,
            network,
            graph,
            &self.pathfind_count,
            Some((i, "parking-recovery-at-node")),
            None,
            Some(destination),
        ) else {
            return false;
        };
        let a = &mut self.agents;
        a.parking_trip[i].route_replaced(replan.current_path.len());
        a.current_path[i] = replan.current_path;
        a.walk_lanes[i].clear();
        a.current_path_index[i] = usize::from(a.current_path[i].len() >= 2);
        a.planned_detach_node[i] = replan.planned_detach_node;
        a.planned_detach_lane_id[i] = replan.planned_detach_lane_id as u32;
        a.planned_detach_lane_d[i] = replan.planned_detach_lane_d;
        a.access_flags[i] = replan.access_flags;
        a.network_replan_failures[i] = 0;
        a.speed[i] = 0.0;
        true
    }
}
