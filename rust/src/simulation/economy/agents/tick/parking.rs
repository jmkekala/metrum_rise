// SPDX-License-Identifier: GPL-2.0-only

//! Ordered private-car departures and parking phase boundaries; movement uses the existing FSM.

use super::super::{
    AGE_CHILD, AgentSystem, MODE_CAR, MODE_WALK, TRANSIT_ACCESS_EGRESS, TRANSIT_IN_BUILDING,
};
use super::planning::{
    BuiltTripPlan, ParkingLegQuote, TripQueryScratch, quote_parking_leg, quote_parking_trip,
};
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use crate::simulation::parking::stats::DepartureOutcome;
use crate::simulation::parking::{ParkingSystem, ParkingTrip, ParkingTripPhase};
use rayon::prelude::*;
mod recovery;

impl AgentSystem {
    pub(super) fn prepare_parking_movement(
        &mut self,
        parking: &ParkingSystem,
        network: &TransitNetwork,
    ) {
        #[cfg(test)]
        super::contract::add(
            super::contract::Phase::ParkingEligibilityAgents,
            self.agents.len(),
        );
        let handles = &self.handles;
        let ages = &self.agents.age_group;
        let lanes = &self.agents.current_lane_id;
        self.agents
            .has_car
            .par_iter_mut()
            .zip(self.agents.parking_trip.par_iter_mut())
            .enumerate()
            .for_each(|(i, (flag, trip))| {
                *flag =
                    ages[i] != AGE_CHILD && parking.available_to(trip.vehicle, handles.handle(i));
                if matches!(
                    trip.phase,
                    ParkingTripPhase::PullOut | ParkingTripPhase::Drive | ParkingTripPhase::PullIn
                ) && !parking.trip_reserved(trip.vehicle, trip.destination)
                    && !(trip.phase == ParkingTripPhase::Drive
                        && network
                            .lane_system
                            .lanes
                            .get(lanes[i])
                            .is_some_and(|lane| lane.edge_id == usize::MAX))
                {
                    trip.ready = true;
                    trip.replan_needed = true;
                }
            });
    }

    /// O(agents + T × Q) deterministic resource phase. Sleeping cars are never scanned.
    /// Independent ready classification uses Rayon; exclusive resource commits use agent order.
    pub(super) fn commit_parking_transitions(
        &mut self,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        #[cfg(test)]
        super::contract::add(
            super::contract::Phase::ParkingTransitionAgents,
            self.agents.len(),
        );
        // The existing movement phase already prepares route scratch for this revision.
        let mut scratch = std::mem::take(&mut self.movement_serial_query);
        TripQueryScratch::with_owned(&mut scratch, || {
            for i in 0..self.agents.len() {
                let trip = self.agents.parking_trip[i];
                if trip.retry_pending {
                    continue;
                }
                if trip.replan_needed
                    || (trip.phase == ParkingTripPhase::WalkToCar
                        && trip.ready
                        && (!parking.trip_reserved(trip.vehicle, trip.destination)
                            || parking.vehicle_place(trip.vehicle) != Some(trip.source)))
                {
                    self.recover_parking_trip(i, parking, allocator, network, graph);
                    continue;
                }
                if trip.phase == ParkingTripPhase::None {
                    if trip.ready {
                        self.finish_parking_custody_at_home(i, parking, allocator);
                        self.agents.parking_trip[i].ready = false;
                    }
                    if self.agents.freight_shipment_id[i] == u64::MAX
                        && self.agents.pending_household_size[i] == 0
                        && self.agents.transit[i] == TRANSIT_IN_BUILDING
                        && self.agents.planned_target_building[i] != usize::MAX
                        && self.sim_time >= self.agents.next_replan_time[i]
                    {
                        self.depart_with_parking(i, parking, allocator, network, graph);
                    }
                    continue;
                }
                if trip.phase == ParkingTripPhase::Drive {
                    parking.release(trip.vehicle);
                }
                if trip.phase == ParkingTripPhase::WalkToDoor
                    && self.agents.transit[i] == TRANSIT_IN_BUILDING
                {
                    self.finish_parking_custody_at_home(i, parking, allocator);
                    parking.clear_retired_places(trip.vehicle);
                    self.agents.parking_trip[i] = ParkingTrip {
                        vehicle: trip.vehicle,
                        arrival_ownership: trip.arrival_ownership,
                        ..Default::default()
                    };
                    continue;
                }
                if !trip.ready {
                    continue;
                }
                if trip.delivery_only && trip.phase == ParkingTripPhase::PullIn {
                    if parking.occupy(trip.vehicle, trip.destination) {
                        parking.set_moving(trip.vehicle, false);
                        parking.finish_outing(trip.vehicle, self.handles.handle(i));
                        parking.clear_retired_places(trip.vehicle);
                        self.agents.transit[i] = TRANSIT_IN_BUILDING;
                        self.agents.current_building[i] = usize::MAX;
                        self.agents.parking_trip[i].phase = ParkingTripPhase::None;
                        self.agents.parking_trip[i].ready = false;
                        self.completed_parking_deliveries.push(self.handles.handle(i));
                    }
                    continue;
                }
                let target = allocator.building_handles.resolve(
                    crate::simulation::core::handles::Handle::from_bits(trip.target),
                );
                let Some(target) = target else {
                    continue;
                };
                let next = match trip.phase {
                    ParkingTripPhase::WalkToCar => ParkingTripPhase::PullOut,
                    ParkingTripPhase::PullIn => ParkingTripPhase::WalkToDoor,
                    _ => continue,
                };
                let Some(origin) = parking.trip_entrance(
                    ParkingTrip {
                        phase: next,
                        ..trip
                    },
                    true,
                    allocator,
                    network,
                    graph,
                ) else {
                    self.queue_parking_retry(i, 30);
                    continue;
                };
                let destination = if next == ParkingTripPhase::PullOut {
                    parking.place_entrance(trip.destination, allocator, network, graph)
                } else {
                    allocator.entrances.get(target).cloned()
                };
                let Some(destination) = destination else {
                    self.queue_parking_retry(i, 30);
                    continue;
                };
                let mode = if next == ParkingTripPhase::PullOut {
                    MODE_CAR
                } else {
                    MODE_WALK
                };
                let maneuvers = if mode == MODE_CAR {
                    [
                        parking.maneuver_seconds(trip.source),
                        parking.maneuver_seconds(trip.destination),
                    ]
                } else {
                    [0.0; 2]
                };
                let Some(quote) = quote_parking_leg(
                    mode,
                    &origin,
                    &destination,
                    maneuvers,
                    network,
                    graph,
                    &self.pathfind_count,
                    None,
                ) else {
                    self.queue_parking_retry(i, 30);
                    continue;
                };
                let Some(plan) = self.materialize_parking_leg(
                    i,
                    &quote,
                    target,
                    self.agents.activity[i],
                    network,
                    graph,
                ) else {
                    continue;
                };
                if next == ParkingTripPhase::PullOut {
                    if !parking.trip_reserved(trip.vehicle, trip.destination) {
                        self.agents.current_path[i] = plan.current_path;
                        continue;
                    }
                    parking.set_moving(trip.vehicle, true);
                } else {
                    if !parking.occupy(trip.vehicle, trip.destination) {
                        self.agents.current_path[i] = plan.current_path;
                        continue;
                    }
                    parking.set_moving(trip.vehicle, false);
                }
                self.agents.parking_trip[i].phase = next;
                self.agents.parking_trip[i].ready = false;
                self.apply_parking_leg(i, plan, &origin);
            }
        });
        self.movement_serial_query = scratch;
    }

    // O(1) only on an actual home-arrival event, never a sleeping borrower poll.
    fn finish_parking_custody_at_home(
        &mut self,
        i: usize,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
    ) {
        let home = self.agents.home_building[i];
        if self.agents.current_building[i] != home || home >= allocator.buildings.len() {
            return;
        }
        let car = self.agents.parking_trip[i].vehicle;
        let borrower = self.handles.handle(i);
        if parking.borrowed_by(car) != Some(borrower) {
            return;
        }
        parking.finish_outing(car, borrower);
        parking.clear_retired_places(car);
    }

    fn materialize_parking_leg(
        &mut self,
        i: usize,
        quote: &ParkingLegQuote,
        target: usize,
        activity: u8,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<BuiltTripPlan> {
        match quote.materialize(
            target,
            activity,
            &mut self.agents.current_path[i],
            self.agents.parking_trip[i].route_capacity,
            network,
            graph,
        ) {
            Ok(plan) => Some(plan),
            Err(needed) => {
                self.agents.parking_trip[i].route_capacity = needed;
                self.queue_parking_retry(i, if needed == 0 { 30 } else { 0 });
                None
            }
        }
    }

    fn depart_with_parking(
        &mut self,
        i: usize,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let current = self.agents.current_building[i];
        let target = self.agents.planned_target_building[i];
        if current == target {
            return;
        }
        let car = self.agents.parking_trip[i].vehicle;
        if parking
            .borrowed_by(car)
            .is_none_or(|borrower| borrower == self.handles.handle(i))
        {
            parking.clear_retired_places(car);
        }
        let (Some(origin), Some(destination)) = (
            allocator.entrances.get(current),
            allocator.entrances.get(target),
        ) else {
            return;
        };
        let activity = self.agents.planned_activity[i];
        let walk = quote_parking_leg(
            MODE_WALK,
            origin,
            destination,
            [0.0; 2],
            network,
            graph,
            &self.pathfind_count,
            None,
        );
        let trip = self.agents.parking_trip[i];
        let borrower = self.handles.handle(i);
        let target_handle = allocator.building_handles.handle(target).to_bits();
        let adult = self.agents.age_group[i] != AGE_CHILD;
        let quote = quote_parking_trip(
            trip.vehicle,
            borrower,
            adult,
            current,
            target,
            parking,
            allocator,
            network,
            graph,
            &self.pathfind_count,
            None,
        );
        // Tentatively `Drove`; corrected below if the drive leg or reservation fails.
        let mut outcome = match &quote {
            None => parking.walk_reason(trip.vehicle, adult, target_handle),
            Some(quote)
                if walk
                    .as_ref()
                    .is_some_and(|walk| walk.seconds() <= quote.mode_choice_seconds) =>
            {
                DepartureOutcome::WalkingFaster
            }
            Some(_) => DepartureOutcome::Drove,
        };
        if let Some(quote) = quote
            && outcome == DepartureOutcome::Drove
            && let Some(plan) =
                self.materialize_parking_leg(i, &quote.to_car, target, activity, network, graph)
        {
            if parking.reserve_trip(trip.vehicle, borrower, quote.destination) {
                parking.departures.record(outcome);
                self.agents.parking_trip[i] = ParkingTrip {
                    vehicle: trip.vehicle,
                    source: quote.source,
                    destination: quote.destination,
                    target: target_handle,
                    phase: ParkingTripPhase::WalkToCar,
                    ready: false,
                    route_capacity: 0,
                    retry_pending: false,
                    arrival_ownership: false,
                    delivery_only: false,
                    replan_needed: false,
                    walk_origin: crate::simulation::core::handles::Handle::NONE,
                };
                self.agents.journey_start_time[i] = self.sim_time;
                self.apply_parking_leg(i, plan, origin);
                return;
            }
            self.agents.current_path[i] = plan.current_path;
            outcome = DepartureOutcome::ReservationLost;
        } else if outcome == DepartureOutcome::Drove {
            outcome = DepartureOutcome::NoDriveRoute;
        }
        parking.departures.record(outcome);
        if self.agents.parking_trip[i].retry_pending {
            return;
        }
        if let Some(walk) = walk
            && let Some(plan) =
                self.materialize_parking_leg(i, &walk, target, activity, network, graph)
        {
            self.agents.journey_start_time[i] = self.sim_time;
            self.apply_parking_leg(i, plan, origin);
        } else if !self.agents.parking_trip[i].retry_pending {
            self.queue_parking_retry(i, 30);
        }
    }

    fn queue_parking_retry(&mut self, i: usize, seconds: u16) {
        if self.agents.parking_trip[i].retry_pending {
            return;
        }
        self.agents.parking_trip[i].retry_pending = true;
        self.agents.next_replan_time[i] = f32::MAX;
        self.parking_retries.push((self.handles.handle(i), seconds));
    }

    fn apply_parking_leg(&mut self, i: usize, plan: BuiltTripPlan, origin: &BuildingEntrance) {
        let a = &mut self.agents;
        a.transit_mode[i] = plan.mode;
        a.target_building[i] = plan.target_building;
        a.activity[i] = plan.activity;
        a.planned_attach_node[i] = plan.planned_attach_node;
        a.planned_detach_node[i] = plan.planned_detach_node;
        a.planned_attach_lane_id[i] = plan.planned_attach_lane_id as u32;
        a.planned_detach_lane_id[i] = plan.planned_detach_lane_id as u32;
        a.planned_attach_lane_d[i] = plan.planned_attach_lane_d;
        a.planned_detach_lane_d[i] = plan.planned_detach_lane_d;
        a.access_flags[i] = plan.access_flags;
        a.current_node[i] = u32::MAX;
        a.current_edge[i] = usize::MAX;
        a.current_lane_id[i] = usize::MAX;
        a.lane_distance[i] = 0.0;
        a.speed[i] = 0.0;
        a.pos_x[i] = origin.door_pos.x;
        a.pos_y[i] = origin.door_pos.y;
        a.transit[i] = TRANSIT_ACCESS_EGRESS;
        a.current_path[i] = plan.current_path;
        a.current_path_index[i] = usize::from(a.current_path[i].len() >= 2);
        a.planned_target_building[i] = usize::MAX;
        a.planned_activity[i] = 0;
        a.next_departure_day[i] = u32::MAX;
        a.next_departure_minute[i] = 0;
        a.next_departure_origin_building[i] = usize::MAX;
        a.next_departure_target_building[i] = usize::MAX;
        a.next_departure_activity[i] = 0;
        a.next_replan_time[i] = 0.0;
        a.network_replan_failures[i] = 0;
        // Access egress retains a valid origin index while physically outside the building.
        if a.current_building[i] == usize::MAX {
            a.current_building[i] = plan.target_building;
        }
    }
}

impl AgentSystem {
    /// Debug/fixture boundary: requests an ordinary resident trip and holds it for a timed
    /// departure event. O(1); planning, borrowing and reservation remain in the live FSM.
    pub(crate) fn request_parking_fixture_trip(&mut self, agent: usize, target: usize) -> bool {
        if agent >= self.agents.len() || self.agents.transit[agent] != TRANSIT_IN_BUILDING {
            return false;
        }
        self.agents.planned_target_building[agent] = target;
        self.agents.planned_activity[agent] = if target == self.agents.home_building[agent] {
            0
        } else {
            2
        };
        self.hold_departure(agent);
        true
    }
}
