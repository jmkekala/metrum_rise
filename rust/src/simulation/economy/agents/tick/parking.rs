// SPDX-License-Identifier: GPL-2.0-only

//! Ordered private-car departures and parking phase boundaries; movement uses the existing FSM.

use super::super::{
    AGE_CHILD, AgentSystem, MODE_CAR, MODE_WALK, TRANSIT_ACCESS_EGRESS, TRANSIT_IN_BUILDING,
};
use super::planning::{
    BuiltTripPlan, ParkingLegQuote, ParkingQuote, SharedNetworkQuotes, TripQueryScratch,
    parking_trip_bays, quote_parking_between_places, quote_parking_leg,
};
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use crate::simulation::parking::stats::DepartureOutcome;
use crate::simulation::parking::{ParkingSystem, ParkingTrip, ParkingTripPhase};
use std::sync::atomic::AtomicU32;
mod candidates;
mod prepared;
mod recovery;
pub(crate) use candidates::ParkingWork;
#[cfg(test)]
pub(in crate::simulation::economy::agents) use prepared::{AnswerCounts, take_answer_counts};
use prepared::{KnownVisit, LegEnd, LegKey};

// Candidates answered and committed at a time. An answer pass costs about 0.1 ms of waking and
// joining, and a trip quoted again after a bay moved 15 to 30 us on the one thread. At 30k a burst
// of 374 departures had 74 trips quoted again in one block, 18 in blocks of 96.
const ANSWER_BLOCK: usize = 96;

// The leg an agent starts when its current one ends, and everything a quote of it reads.
struct NextParkingLeg {
    mode: u8,
    origin: BuildingEntrance,
    destination: BuildingEntrance,
    maneuvers: [f32; 2],
    key: LegKey,
}

impl NextParkingLeg {
    fn quote(
        &self,
        network: &TransitNetwork,
        graph: &RegionGraph,
        count: &AtomicU32,
        quotes: Option<&SharedNetworkQuotes>,
    ) -> Option<ParkingLegQuote> {
        quote_parking_leg(
            self.mode,
            &self.origin,
            &self.destination,
            self.maneuvers,
            network,
            graph,
            count,
            quotes,
        )
    }
}

// O(1) plus the entrances' frontage projection; `None` when an end has no usable entrance.
fn next_parking_leg(
    trip: ParkingTrip,
    phase: ParkingTripPhase,
    target: usize,
    parking: &ParkingSystem,
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<NextParkingLeg> {
    let origin = parking.trip_entrance(ParkingTrip { phase, ..trip }, true, allocator, network, graph)?;
    let pulling_out = phase == ParkingTripPhase::PullOut;
    let destination = if pulling_out {
        parking.place_entrance(trip.destination, allocator, network, graph)?
    } else {
        allocator.entrances.get(target).cloned()?
    };
    let (mode, maneuvers, key) = if pulling_out {
        let (from, to) = (LegEnd::Bay(trip.source), LegEnd::Bay(trip.destination));
        let maneuvers = [
            parking.maneuver_seconds(trip.source),
            parking.maneuver_seconds(trip.destination),
        ];
        (MODE_CAR, maneuvers, LegKey { mode: MODE_CAR, from, to })
    } else {
        let key = LegKey::on_foot(LegEnd::Bay(trip.destination), LegEnd::Door(target));
        (MODE_WALK, [0.0; 2], key)
    };
    Some(NextParkingLeg { mode, origin, destination, maneuvers, key })
}

// Walking wins an exact tie against the car's cost with its preference overhead.
fn walk_is_quicker(walk: Option<&ParkingLegQuote>, car: &ParkingQuote) -> bool {
    walk.is_some_and(|walk| walk.seconds() <= car.mode_choice_seconds)
}

impl ParkingTripPhase {
    // The leg the commit starts when this one ends, for a leg the commit ends.
    fn next_transition(self) -> Option<Self> {
        match self {
            Self::WalkToCar => Some(Self::PullOut),
            Self::PullIn => Some(Self::WalkToDoor),
            _ => None,
        }
    }
}

impl AgentSystem {
    /// O(C log C + T × Q) deterministic resource phase, for C candidates and T transitions:
    /// sleeping cars and idle agents are never visited. Exclusive resource commits use ascending
    /// agent order, whatever the worker count (`candidates.rs`). Routing runs before the walk, on
    /// every worker (`prepared.rs`); the walk costs O(C log C + C × k + R × Q), R the trips an
    /// earlier visit of the tick sent to another bay.
    pub(super) fn commit_parking_transitions(
        &mut self,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let candidates = self.take_parking_candidates();
        #[cfg(all(test, debug_assertions))]
        self.audit_parking_candidates(&candidates, parking);
        // A block's answers see what the blocks before it committed, so a burst of departures for
        // one door loses few to a bay an earlier visit took.
        let mut quotes = std::mem::take(&mut self.parking_work.quotes);
        quotes.sync(network);
        for block in candidates.chunks(ANSWER_BLOCK) {
            let answers =
                self.prepare_parking_answers(block, &quotes, parking, allocator, network, graph);
            // The existing movement phase already prepares route scratch for this revision.
            let mut scratch = std::mem::take(&mut self.movement_serial_query);
            TripQueryScratch::with_owned(&mut scratch, || {
                for (k, &i) in block.iter().enumerate() {
                    let known = answers.visit(k, &quotes);
                    self.commit_parking_visit(i, known, parking, allocator, network, graph);
                }
            });
            self.movement_serial_query = scratch;
            self.parking_work.answers = answers;
        }
        self.parking_work.quotes = quotes;
        self.finish_parking_candidates(candidates, parking);
    }

    // One candidate's turn in the ordered phase. O(k), plus O(Q) for what `known` leaves open.
    fn commit_parking_visit(
        &mut self,
        i: usize,
        known: KnownVisit<'_>,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let trip = self.agents.parking_trip[i];
        if trip.retry_pending {
            return;
        }
        if trip.replan_needed
            || (trip.phase == ParkingTripPhase::WalkToCar
                && trip.ready
                && (!parking.trip_reserved(trip.vehicle, trip.destination)
                    || parking.vehicle_place(trip.vehicle) != Some(trip.source)))
        {
            self.recover_parking_trip(i, parking, allocator, network, graph);
            return;
        }
        if trip.phase == ParkingTripPhase::None {
            if trip.ready {
                self.finish_parking_custody_at_home(i, parking, allocator);
                self.agents.parking_trip[i].ready = false;
            }
            if self.parking_departure_due(i) {
                self.depart_with_parking(i, known, parking, allocator, network, graph);
            }
            return;
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
            return;
        }
        if !trip.ready {
            return;
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
            return;
        }
        let target = allocator.building_handles.resolve(
            crate::simulation::core::handles::Handle::from_bits(trip.target),
        );
        let Some(target) = target else {
            self.recover_parking_trip(i, parking, allocator, network, graph);
            return;
        };
        let Some(next) = trip.phase.next_transition() else {
            return;
        };
        let Some(leg) = next_parking_leg(trip, next, target, parking, allocator, network, graph)
        else {
            self.queue_parking_retry(i, 30);
            return;
        };
        let count = &self.pathfind_count;
        let Some(quote) = known.leg_or(leg.key, |quotes| leg.quote(network, graph, count, quotes))
        else {
            self.queue_parking_retry(i, 30);
            return;
        };
        let Some(plan) = self.materialize_parking_leg(
            i,
            &quote,
            target,
            self.agents.activity[i],
            known.route(leg.key),
            network,
            graph,
        ) else {
            return;
        };
        if next == ParkingTripPhase::PullOut {
            if !parking.trip_reserved(trip.vehicle, trip.destination) {
                self.agents.current_path[i] = plan.current_path;
                return;
            }
            parking.set_moving(trip.vehicle, true);
        } else {
            if !parking.occupy(trip.vehicle, trip.destination) {
                self.agents.current_path[i] = plan.current_path;
                return;
            }
            parking.set_moving(trip.vehicle, false);
        }
        self.agents.parking_trip[i].phase = next;
        self.agents.parking_trip[i].ready = false;
        self.apply_parking_leg(i, plan, &leg.origin);
    }

    // Whether agent `i`, with no itinerary, is due to leave its building. O(1).
    fn parking_departure_due(&self, i: usize) -> bool {
        self.agents.freight_shipment_id[i] == u64::MAX
            && self.agents.pending_household_size[i] == 0
            && self.agents.transit[i] == TRANSIT_IN_BUILDING
            && self.agents.planned_target_building[i] != usize::MAX
            && self.sim_time >= self.agents.next_replan_time[i]
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

    #[allow(clippy::too_many_arguments)]
    fn materialize_parking_leg(
        &mut self,
        i: usize,
        quote: &ParkingLegQuote,
        target: usize,
        activity: u8,
        known: Option<Option<&[u32]>>,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<BuiltTripPlan> {
        match quote.materialize(
            target,
            activity,
            &mut self.agents.current_path[i],
            self.agents.parking_trip[i].route_capacity,
            known,
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
        known: KnownVisit<'_>,
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
        let count = &self.pathfind_count;
        let walk_key = LegKey::on_foot(LegEnd::Door(current), LegEnd::Door(target));
        let walk = known.leg_or(walk_key, |quotes| {
            quote_parking_leg(MODE_WALK, origin, destination, [0.0; 2], network, graph, count, quotes)
        });
        let trip = self.agents.parking_trip[i];
        let borrower = self.handles.handle(i);
        let target_handle = allocator.building_handles.handle(target).to_bits();
        let adult = self.agents.age_group[i] != AGE_CHILD;
        // The bays are read now, from what the visits before this one left: the requote rule.
        let quote = parking_trip_bays(trip.vehicle, borrower, adult, target, parking, allocator)
            .and_then(|bays| {
                let parking = &*parking;
                known.trip_or(bays, |quotes| {
                    quote_parking_between_places(
                        bays.0, bays.1, current, target, parking, allocator, network, graph,
                        count, quotes,
                    )
                })
            });
        // Tentatively `Drove`; corrected below if the drive leg or reservation fails.
        let mut outcome = match &quote {
            None => parking.walk_reason(trip.vehicle, adult, target_handle),
            Some(quote) if walk_is_quicker(walk.as_ref(), quote) => DepartureOutcome::WalkingFaster,
            Some(_) => DepartureOutcome::Drove,
        };
        if let Some(quote) = quote
            && outcome == DepartureOutcome::Drove
            && let Some(plan) = self.materialize_parking_leg(
                i,
                &quote.to_car,
                target,
                activity,
                known.route(LegKey::on_foot(LegEnd::Door(current), LegEnd::Bay(quote.source))),
                network,
                graph,
            )
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
                self.set_off_on_parking_leg(i, plan, origin);
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
            && let Some(plan) = self.materialize_parking_leg(
                i,
                &walk,
                target,
                activity,
                known.route(walk_key),
                network,
                graph,
            )
        {
            self.set_off_on_parking_leg(i, plan, origin);
        } else if !self.agents.parking_trip[i].retry_pending {
            self.queue_parking_retry(i, 30);
        }
    }

    // The trip starts: the plan it was made from is spent. A later leg keeps what a move to a new
    // home planned meanwhile for after the trip.
    fn set_off_on_parking_leg(&mut self, i: usize, plan: BuiltTripPlan, origin: &BuildingEntrance) {
        self.agents.journey_start_time[i] = self.sim_time;
        self.agents.planned_target_building[i] = usize::MAX;
        self.agents.planned_activity[i] = 0;
        self.apply_parking_leg(i, plan, origin);
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
        a.walk_lanes[i].clear();
        a.current_path_index[i] = usize::from(a.current_path[i].len() >= 2);
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
