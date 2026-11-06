// SPDX-License-Identifier: GPL-2.0-only

//! Event-driven resident recovery preserves the driver's actual position and vehicle identity.

use super::*;
use crate::simulation::core::handles::Handle;
use crate::simulation::economy::agents::tick::planning::quote_parking_leg_from_lane;
use crate::simulation::economy::agents::{ACCESS_LANE_ATTACH_PENDING, TRANSIT_NETWORK};

impl AgentSystem {
    pub(super) fn recover_parking_trip(
        &mut self,
        i: usize,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let mut trip = self.agents.parking_trip[i];
        let Some(target) = allocator
            .building_handles
            .resolve(Handle::from_bits(trip.target))
        else {
            self.queue_parking_retry(i, 30);
            return;
        };
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
                    network,
                    graph,
                ) else {
                    return;
                };
                parking.forget_retired_place(trip.vehicle, trip.walk_origin);
                trip.walk_origin = trip.source;
                trip.source = actual;
                trip.ready = false;
                trip.replan_needed = false;
                self.agents.parking_trip[i] = trip;
                self.apply_parking_leg(i, plan, &origin);
                return;
            }
            if parking.vehicle_place(trip.vehicle).is_some()
                && let Some(candidate) = parking.first_free_candidate(trip.target)
                && parking.reserve(trip.vehicle, candidate.place)
            {
                self.agents.parking_trip[i].destination = candidate.place;
                self.agents.parking_trip[i].replan_needed = false;
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
                parking.forget_retired_place(trip.vehicle, trip.source);
                trip.source = trip.destination;
                trip.phase = ParkingTripPhase::PullOut;
            }
            trip.destination = candidate.place;
            self.agents.parking_trip[i] = trip;
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
                network,
                graph,
            ) else {
                return;
            };
            let position = (self.agents.pos_x[i], self.agents.pos_y[i]);
            self.apply_parking_leg(i, plan, &origin);
            self.agents.pos_x[i] = position.0;
            self.agents.pos_y[i] = position.1;
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
            a.current_path_index[i] = usize::from(a.current_path[i].len() >= 2);
            a.access_flags[i] =
                plan.access_flags | (a.access_flags[i] & ACCESS_LANE_ATTACH_PENDING);
            a.transit[i] = TRANSIT_NETWORK;
        }
        self.agents.parking_trip[i].ready = false;
        self.agents.parking_trip[i].replan_needed = false;
        self.agents.next_replan_time[i] = 0.0;
    }
}
