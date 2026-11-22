// SPDX-License-Identifier: GPL-3.0-or-later

//! The drop-off (`PARK-34`, docs/traffic.md "Drop-off arrival"): a moving-in household that
//! cannot drive its own car to a place at home is driven in by a car that is no car of the
//! store. The carrier drives it, the household riding as its pending counts; at the stop the
//! household steps out as on a family arrival (`PARK-35`), and the carrier, a carrier no more,
//! drives the car back to a border, where it is removed. The row's `drop_off` flag is the whole
//! of its saved state.

use super::*;
use crate::simulation::economy::agents::tick::planning::plan_entrance_to_border_trip;
use crate::simulation::economy::agents::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_LANE_ATTACH_PENDING, ACCESS_PLAN_VALID,
    ACTIVITY_HOME, TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use godot::prelude::Vector2;

impl AgentSystem {
    /// Member 0's walk from drop-off carrier `i`'s stop to the door of its home, with the
    /// stop's entrance: the leg the family steps out onto. `None` without one. O(Q) twice:
    /// once for the route's length, once into storage of that size (an arrival event, not a
    /// tick).
    pub(crate) fn drop_off_walk(
        &self,
        i: usize,
        parking: &ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<(BuiltTripPlan, BuildingEntrance)> {
        let trip = ParkingTrip {
            phase: ParkingTripPhase::WalkToDoor,
            ..self.agents.parking_trip[i]
        };
        let home = self.agents.home_building[i];
        let origin = parking.trip_entrance(trip, true, allocator, network, graph)?;
        let door = allocator.entrances.get(home)?;
        let quote = quote_parking_leg(
            MODE_WALK,
            &origin,
            door,
            [0.0; 2],
            network,
            graph,
            &self.pathfind_count,
            None,
        )?;
        let mut route = Vec::new();
        let plan = match quote.materialize(home, ACTIVITY_HOME, &mut route, 0, None, network, graph)
        {
            Ok(plan) => plan,
            Err(0) => return None,
            Err(needed) => {
                route.reserve(needed);
                quote
                    .materialize(
                        home,
                        ACTIVITY_HOME,
                        &mut route,
                        needed,
                        None,
                        network,
                        graph,
                    )
                    .ok()?
            }
        };
        Some((plan, origin))
    }

    /// Spawns member 0 of a household dropped off at its stop: a resident of `home` standing at
    /// `pos` on the walk `plan` from the stop (`origin`) to the door, carrying `trip` (its
    /// household's car decision on a `WalkToDoor` leg). The other members copy its leg
    /// ([`Self::spawn_member_beside_car`]). O(route length).
    pub(crate) fn spawn_member_on_walk(
        &mut self,
        home: usize,
        pos: Vector2,
        age_group: u8,
        trip: ParkingTrip,
        plan: BuiltTripPlan,
        origin: &BuildingEntrance,
    ) -> usize {
        let i = self.spawn_housed_agent_with_age_group(home, pos.x, pos.y, age_group);
        self.agents.parking_trip[i] = trip;
        self.agents.journey_start_time[i] = self.sim_time;
        self.apply_parking_leg(i, plan, origin);
        self.agents.pos_x[i] = pos.x;
        self.agents.pos_y[i] = pos.y;
        i
    }

    /// Turns drop-off carrier `i`, its household just stepped out at `stop`, into the drop-off
    /// driver, an agent of nothing (no home, no household, never a worker, no car of the
    /// store), and sends it out of the stop to a border: the one it came from, else the nearest
    /// of `borders` in a straight line, the first of them the router reaches (list order for
    /// ties). Its `ArrivalCheck` removes it there. False when no border is reachable: the
    /// caller removes it where it stands. O(B² + B × Q) for B borders at most, typically one Q.
    pub(crate) fn drive_drop_off_away(
        &mut self,
        i: usize,
        stop: &BuildingEntrance,
        borders: &[u32],
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let trip = self.agents.parking_trip[i];
        let home = self.agents.home_building[i];
        self.become_drop_off_driver(i);
        let came_from = self.agents.freight_target_border_node[i];
        let at = stop.door_pos;
        let distance = |k: usize| {
            let pos = graph.node(borders[k]).pos;
            ((pos.x - at.x).powi(2) + (pos.z - at.y).powi(2), k)
        };
        let order = |a: (f32, usize), b: (f32, usize)| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1));
        // The border it came from first, then the others nearest first: a selection over the
        // few borders on the stack, no allocation.
        let after = |last: Option<(f32, usize)>| {
            (0..borders.len())
                .filter(|&k| borders[k] != came_from)
                .filter(|&k| last.is_none_or(|last| order(distance(k), last).is_gt()))
                .min_by(|&a, &b| order(distance(a), distance(b)))
        };
        let mut last = None;
        // The border it came from while it is still a connected border (any node in a city
        // with none listed, a test road's end).
        let mut next = (came_from != u32::MAX
            && (borders.is_empty() || borders.contains(&came_from)))
        .then_some(came_from)
        .or_else(|| after(None).map(|k| borders[k]));
        for _ in 0..=borders.len() {
            let Some(border) = next else {
                return false;
            };
            if let Some(plan) =
                plan_entrance_to_border_trip(stop, border, network, graph, &self.pathfind_count)
            {
                self.agents.parking_trip[i] = ParkingTrip {
                    source: trip.destination,
                    target: trip.target,
                    phase: ParkingTripPhase::PullOut,
                    drop_off: true,
                    route_capacity: plan.current_path.len(),
                    ..Default::default()
                };
                self.apply_parking_leg(i, plan, stop);
                let a = &mut self.agents;
                // Its egress reads the home's road; its plan ends at the border, as an export
                // carrier's does.
                a.current_building[i] = home;
                a.target_building[i] = usize::MAX;
                a.planned_detach_lane_id[i] = u32::MAX;
                a.freight_target_border_node[i] = border;
                a.journey_start_time[i] = self.sim_time;
                return true;
            }
            if border != came_from
                && let Some(k) = borders.iter().position(|&b| b == border)
            {
                last = Some(distance(k));
            }
            next = after(last).map(|k| borders[k]);
        }
        false
    }

    // A carrier no more: no household to carry, no home, never a worker, no car of the store.
    // O(1).
    fn become_drop_off_driver(&mut self, i: usize) {
        let a = &mut self.agents;
        a.pending_household_size[i] = 0;
        a.pending_household_adults[i] = 0;
        a.pending_household_elders[i] = 0;
        a.home_building[i] = usize::MAX;
        a.planned_target_building[i] = usize::MAX;
        a.age_group[i] = AGE_CHILD;
        a.has_car[i] = false;
        a.parking_trip[i] = ParkingTrip {
            drop_off: true,
            ..Default::default()
        };
    }

    /// Sends drop-off carrier or driver `i` back to the border it came from from where it is,
    /// its stop gone with the home (`ECON-29`) or its trip ended (`PARK-34`): a car bound for a
    /// border, replanned by the movement pass as an export carrier's route is after a road
    /// edit. One still held at its border, never set out, or with no border to go to, stops
    /// in no building, where its check removes it. O(entries on its lanes).
    pub(crate) fn turn_drop_off_back(&mut self, i: usize) {
        let held = self.release_departure(i);
        let border = self.agents.freight_target_border_node[i];
        let before = self.lane_bucket_entry_lanes(i);
        self.become_drop_off_driver(i);
        let a = &mut self.agents;
        a.target_building[i] = usize::MAX;
        a.current_path[i].clear();
        a.current_path_index[i] = 0;
        a.next_replan_time[i] = 0.0;
        a.network_replan_failures[i] = 0;
        let attach_pending = a.access_flags[i] & ACCESS_LANE_ATTACH_PENDING;
        a.planned_detach_lane_id[i] = u32::MAX;
        a.planned_detach_lane_d[i] = 0.0;
        let on_road = matches!(a.transit[i], TRANSIT_NETWORK | TRANSIT_INTERSECTION);
        if held || border == u32::MAX || !on_road {
            // At its border, or pulling over at its stop (no lane to plan from): it is done.
            self.leave_lane_buckets(i);
            let a = &mut self.agents;
            a.access_flags[i] = 0;
            a.planned_detach_node[i] = u32::MAX;
            a.current_lane_id[i] = usize::MAX;
            a.current_edge[i] = usize::MAX;
            a.speed[i] = 0.0;
            a.transit[i] = TRANSIT_IN_BUILDING;
            a.current_building[i] = usize::MAX;
            return;
        }
        a.planned_detach_node[i] = border;
        a.access_flags[i] = ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION | attach_pending;
        self.update_lane_bucket_membership(i, before);
    }

    /// Whether agent `i` is a drop-off driver (`PARK-34`) whose trip is over: it stands at its
    /// border, or in no building, where an edit under its pull-out left it. O(1).
    pub(crate) fn drop_off_finished(&self, i: usize) -> bool {
        let a = &self.agents;
        a.parking_trip[i].drop_off
            && a.pending_household_size[i] == 0
            && a.household_id[i] == usize::MAX
            && (a.transit[i] == TRANSIT_IN_BUILDING
                || (a.access_flags[i] & ACCESS_FREIGHT_BORDER_DESTINATION != 0
                    && a.current_node[i] == a.freight_target_border_node[i]
                    && a.current_lane_id[i] == usize::MAX
                    && a.current_path[i].is_empty()))
    }

    /// Whether agent `i` is a drop-off driver on its way back (`PARK-34`). O(1).
    pub(crate) fn drives_drop_off_back(&self, i: usize) -> bool {
        let a = &self.agents;
        a.parking_trip[i].drop_off
            && a.pending_household_size[i] == 0
            && a.household_id[i] == usize::MAX
    }

    /// Removes drop-off driver `i` with the next completed deliveries, after the next tick.
    /// O(1).
    pub(crate) fn retire_drop_off(&mut self, i: usize) {
        self.leave_lane_buckets(i);
        let a = &mut self.agents;
        a.transit[i] = TRANSIT_IN_BUILDING;
        a.current_building[i] = usize::MAX;
        self.completed_parking_deliveries
            .push(self.handles.handle(i));
    }
}
