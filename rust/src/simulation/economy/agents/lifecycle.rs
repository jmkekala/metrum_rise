// SPDX-License-Identifier: GPL-3.0-or-later

//! Agent spawning, removal, and whole-store lifecycle helpers.

use super::data::{Agent, AgentSystem};
use super::determinism::{stable_index, stable_unit_f32};
use super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, AGE_ADULT, AGE_CHILD, MODE_CAR, MODE_WALK,
    TRANSIT_ACCESS_EGRESS, TRANSIT_IMMIGRATING, TRANSIT_IN_BUILDING, TRANSIT_NETWORK,
    VEHICLE_FREIGHT_DELIVERY, age_group_can_work,
};
use crate::config::DEFAULT_URBAN_ROAD_SPEED_MS;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::buildings::allocator::BuildingEntrance;
use crate::simulation::core::handles::Handle;
use crate::simulation::economy::agents::tick::{
    BuiltTripPlan, plan_border_arrival, plan_building_to_border_trip,
    plan_car_trip_between_entrances, plan_entrance_to_border_trip, plan_immigration_trip,
};
use crate::simulation::parking::{ParkingSystem, ParkingTrip, ParkingTripPhase, Place};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::Vector2;

/// Replan time of an agent held for a paced departure; no simulation time reaches it.
const DEPARTURE_HOLD: f32 = f32::MAX;

impl AgentSystem {
    /// Spawns one agent already housed inside a building.
    pub fn spawn_housed_agent(&mut self, home: usize, init_x: f32, init_y: f32) -> usize {
        self.spawn_housed_agent_with_age_group(home, init_x, init_y, AGE_ADULT)
    }

    /// Spawns one housed resident with an explicit lifecycle class.
    pub(crate) fn spawn_housed_agent_with_age_group(
        &mut self,
        home: usize,
        init_x: f32,
        init_y: f32,
        age_group: u8,
    ) -> usize {
        let mut agent = self.new_agent(home, init_x, init_y);
        agent.age_group = age_group;
        self.insert_agent(agent)
    }

    /// Spawns a single border-origin arrival agent for explicit immigration-visualization paths.
    pub fn spawn_border_arrival_agent(
        &mut self,
        home: usize,
        border_node: u32,
        init_x: f32,
        init_y: f32,
    ) -> usize {
        let mut agent = self.new_agent(home, init_x, init_y);
        agent.transit = TRANSIT_IMMIGRATING;
        agent.current_building = usize::MAX;
        agent.target_building = home;
        agent.current_node = border_node;
        agent.speed = DEFAULT_URBAN_ROAD_SPEED_MS;
        agent.transit_mode = MODE_CAR;
        self.insert_agent(agent)
    }

    /// Places a car at the start of road lane `lane_id`, driving `path` from
    /// `path[0]` with no home or destination building. For debugging and bridge tests only: it
    /// changes the simulation outside the tick, so the session is no longer reproducible.
    pub(crate) fn spawn_debug_car(
        &mut self,
        path: Vec<u32>,
        lanes: &crate::simulation::network::lanes::LaneSystem,
        lane_id: usize,
        start: Vector2,
    ) -> usize {
        let idx = self.spawn_border_arrival_agent(usize::MAX, path[0], start.x, start.y);
        self.agents.enter_lane(idx, lanes, lane_id, 0.0, path[0]);
        self.agents.current_path[idx] = path;
        self.agents.current_path_index[idx] = 1;
        idx
    }

    /// Spawns a household carrier with the applicant's `composition` as admission drew it.
    pub(crate) fn spawn_household_arrival_carrier(
        &mut self,
        home: usize,
        household_size: u16,
        composition: super::HouseholdAgeComposition,
        border_node: u32,
        init_x: f32,
        init_y: f32,
    ) -> usize {
        let agent_idx = self.spawn_border_arrival_agent(home, border_node, init_x, init_y);
        self.agents.pending_household_size[agent_idx] = household_size.max(1);
        self.agents.pending_household_adults[agent_idx] = composition.adult_count;
        self.agents.pending_household_elders[agent_idx] = composition.elder_count;
        // The carrier waits at the border for its paced departure (`SIM-02-E`).
        self.hold_departure(agent_idx);
        let handle = self.agent_handle(agent_idx);
        self.arrival_carriers.push(handle);
        self.held_arrivals.push(handle);
        agent_idx
    }

    /// Starts one paced move-in: O(k + Q), one cached candidate scan and one border route.
    /// With no parking the household walks and retains its car off-map; no car ends at a door.
    pub(crate) fn start_parking_move_in(
        &mut self,
        i: usize,
        parking: &mut crate::simulation::parking::ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        use crate::simulation::zoning::profiles::{load_builtin_profile_registry, ZoneDensity};
        let home = self.agents.home_building[i];
        let Some(building) = allocator.buildings.get(home) else {
            return false;
        };
        let home_handle = allocator.building_handles.handle(home).to_bits();
        let border = self.agents.current_node[i];
        if !self.agents.parking_trip[i].arrival_ownership {
            let profiles = load_builtin_profile_registry().expect("built-in zoning profiles");
            let density = profiles.density_for_runtime_id(building.zone_profile_runtime_id).unwrap_or(ZoneDensity::Low);
            let car = parking.arrival_car(self.agent_handle(i).to_bits(), density, [building.center_x, building.center_y]);
            self.agents.parking_trip[i].vehicle = car;
            self.agents.parking_trip[i].arrival_ownership = true;
        }
        let car = self.agents.parking_trip[i].vehicle;
        parking.store_arrival_car(car, home_handle);
        self.agents.journey_start_time[i] = self.sim_time;
        if !car.is_none() {
            // The home catchment's first free place, else the nearest free place it reaches
            // beyond it (`PARK-34`).
            let first = parking.first_free_candidate(home_handle).map(|c| c.place);
            if let Some(place) = first
                && self.drive_own_car_to(i, car, place, border, home, parking, allocator, network, graph)
            {
                return true;
            }
            let (nearest, count) = parking.nearest_free_places([building.center_x, building.center_y]);
            for &place in &nearest[..count] {
                if Some(place) != first
                    && self.drive_own_car_to(i, car, place, border, home, parking, allocator, network, graph)
                {
                    crate::debug::work::add(crate::debug::work::Work::ArrivalWiderPlaces, 1);
                    return true;
                }
            }
        }
        // No household walks in from the border: it is driven in (`PARK-34`).
        self.start_drop_off(i, border, home, parking, allocator, network, graph)
    }

    /// Starts carrier `i`'s drop-off from `border` to its stop at `home`: the home catchment's
    /// first free place, without reserving it (the car only stops there), else the kerb stop at
    /// the door ([`super::tick::kerb_stop_entrance`]), else, for a door no car reaches, the
    /// nearest free place a car reaches and a walk leads from to the door, as an own car's wider
    /// search finds one. The carrier keeps its household's car decision; its own car, if any,
    /// stays stored. False, with nothing changed, when no road reaches any of them. O(k + 2Q),
    /// or O(local places + 8 × 2Q) for the door no car reaches, at the paced departure.
    #[allow(clippy::too_many_arguments)]
    fn start_drop_off(
        &mut self,
        i: usize,
        border: u32,
        home: usize,
        parking: &ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let home_handle = allocator.building_handles.handle(home).to_bits();
        let place = parking.first_free_candidate(home_handle).map(|c| c.place);
        let to_place = place.and_then(|place| {
            let entrance = parking.place_entrance(place, allocator, network, graph)?;
            let plan = super::tick::plan_border_arrival(border, home, MODE_CAR, &entrance, network, graph,
                &self.pathfind_count)?;
            Some((place, plan))
        });
        let stop = to_place.or_else(|| {
            let kerb = super::tick::kerb_stop_entrance(allocator.entrances.get(home)?, graph,
                super::tick::DROP_OFF_KERB_INSET_M)?;
            let plan = super::tick::plan_border_arrival(border, home, MODE_CAR, &kerb, network, graph,
                &self.pathfind_count)?;
            Some((Handle::NONE, plan))
        }).or_else(|| {
            let building = allocator.buildings.get(home)?;
            let door = allocator.entrances.get(home)?;
            let (nearest, count) = parking.nearest_free_places([building.center_x, building.center_y]);
            nearest[..count].iter().find_map(|&place| {
                let entrance = parking.place_entrance(place, allocator, network, graph)?;
                super::tick::quote_parking_leg(MODE_WALK, &entrance, door, [0.0; 2], network,
                    graph, &self.pathfind_count, None)?;
                let plan = super::tick::plan_border_arrival(border, home, MODE_CAR, &entrance,
                    network, graph, &self.pathfind_count)?;
                Some((place, plan))
            })
        });
        let Some((place, plan)) = stop else {
            return false;
        };
        let trip = self.agents.parking_trip[i];
        self.agents.parking_trip[i] = ParkingTrip {
            vehicle: trip.vehicle,
            arrival_ownership: true,
            destination: place,
            target: home_handle,
            phase: ParkingTripPhase::Drive,
            drop_off: true,
            route_capacity: plan.current_path.len(),
            ..Default::default()
        };
        self.leave_lane_buckets(i);
        self.apply_border_origin_freight_plan(i, border, home, plan, graph);
        // The border it came from, the first it tries on its way back. Read only for a car bound
        // for a border (`ACCESS_FREIGHT_BORDER_DESTINATION`).
        self.agents.freight_target_border_node[i] = border;
        self.join_lane_buckets(i);
        true
    }

    /// The walk from the border a carrier without a car or a free place set out on before
    /// `PARK-34`, for a test of an older save holding one. O(Q).
    #[cfg(test)]
    pub(crate) fn start_border_walk_for_test(
        &mut self,
        i: usize,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let (home, border) = (self.agents.home_building[i], self.agents.current_node[i]);
        let Some(plan) = allocator.entrances.get(home).and_then(|entrance| {
            super::tick::plan_border_arrival(border, home, MODE_WALK, entrance, network, graph,
                &self.pathfind_count)
        }) else {
            return false;
        };
        self.leave_lane_buckets(i);
        self.apply_border_origin_freight_plan(i, border, home, plan, graph);
        self.join_lane_buckets(i);
        true
    }

    // Sends carrier `i` in its own `car` from `border` to `place` when a road reaches the place,
    // a walk leads from it to the door of `home` and the place takes the reservation; else false
    // with nothing changed. The catchment's own places need no walk proof: the catchment is the
    // walk. O(2Q).
    #[allow(clippy::too_many_arguments)]
    fn drive_own_car_to(
        &mut self,
        i: usize,
        car: crate::simulation::core::handles::Handle<crate::simulation::parking::Vehicle>,
        place: crate::simulation::core::handles::Handle<Place>,
        border: u32,
        home: usize,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let home_handle = allocator.building_handles.handle(home).to_bits();
        let Some(entrance) = parking.place_entrance(place, allocator, network, graph) else {
            return false;
        };
        let walks = parking.walk_to_car_seconds(home_handle, place).is_some()
            || allocator.entrances.get(home).is_some_and(|door| {
                super::tick::quote_parking_leg(MODE_WALK, &entrance, door, [0.0; 2], network,
                    graph, &self.pathfind_count, None).is_some()
            });
        let Some(plan) = walks.then(|| super::tick::plan_border_arrival(border, home, MODE_CAR,
            &entrance, network, graph, &self.pathfind_count)).flatten() else {
            return false;
        };
        if !parking.reserve_arrival(car, self.agent_handle(i), place) {
            return false;
        }
        self.agents.parking_trip[i] = ParkingTrip { vehicle: car, arrival_ownership: true,
            destination: place, target: home_handle, phase: ParkingTripPhase::Drive,
            route_capacity: plan.current_path.len(), ..Default::default() };
        self.leave_lane_buckets(i);
        self.apply_border_origin_freight_plan(i, border, home, plan, graph);
        self.join_lane_buckets(i);
        true
    }

    /// Creates a delivery-only carrier at a border, O(k + BQ) at a scheduled event boundary.
    /// Tries connected borders in stable order; the reserved car stays unavailable until pull-in.
    /// With none that reaches its home the car is towed into the free bay. Returns whether it left
    /// the waiting list.
    pub(crate) fn start_stored_car_arrival(
        &mut self,
        car: crate::simulation::core::handles::Handle<crate::simulation::parking::Vehicle>,
        borders: &[u32],
        parking: &mut crate::simulation::parking::ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        use crate::simulation::parking::{ParkingTrip, ParkingTripPhase};
        let Some(home) = parking.stored_car_home(car) else {
            return false;
        };
        let Some(target) = allocator.building_handles.resolve(crate::simulation::core::handles::Handle::from_bits(home)) else {
            return false;
        };
        let Some(candidate) = parking.first_free_candidate(home) else {
            return false;
        };
        let Some(entrance) = parking.place_entrance(candidate.place, allocator, network, graph) else {
            return false;
        };
        for &border in borders {
            let Some(plan) = super::tick::plan_border_arrival(border, target, MODE_CAR, &entrance,
                network, graph, &self.pathfind_count) else {
                continue;
            };
            let pos = graph.node(border).pos;
            let i = self.spawn_border_arrival_agent(target, border, pos.x, pos.z);
            if !parking.reserve_arrival(car, self.agent_handle(i), candidate.place) {
                self.remove_agent_record(i, "parking delivery cancelled");
                return false;
            }
            self.agents.age_group[i] = AGE_CHILD; // A vehicle carrier never participates in employment.
            self.agents.parking_trip[i] = ParkingTrip { vehicle: car, destination: candidate.place,
                target: home, phase: ParkingTripPhase::Drive, delivery_only: true,
                route_capacity: plan.current_path.len(), ..Default::default() };
            self.apply_border_origin_freight_plan(i, border, target, plan, graph);
            self.agents.journey_start_time[i] = self.sim_time;
            return true;
        }
        // No border reaches the door, so nothing can drive it in: a tow sets it down (`PARK-10`).
        parking.tow_stored_car(car, candidate.place)
    }

    /// Retires only completed delivery carriers through the normal lifecycle helpers.
    /// O(completed carriers + lane entries), after the tick; handles survive each swap removal.
    pub(crate) fn remove_completed_parking_deliveries(&mut self) {
        let mut completed = std::mem::take(&mut self.completed_parking_deliveries);
        for handle in completed.drain(..) {
            if let Some(i) = self.resolve_agent(handle) {
                self.remove_agent_record(i, "car parked after external arrival");
            }
        }
        self.completed_parking_deliveries = completed;
    }

    /// Spawns a resident of `home` standing at `pos` beside the car `walker` just left, on
    /// `walker`'s walk to the door: the same leg plan and parking-trip endpoints, so the existing
    /// access-egress walker moves it, with no custody of the car (`PARK-35`). Walkers never wait
    /// for each other. `walker` must have just started its leg (it stands at the leg's origin).
    /// O(route length) for the copied plan, at the arrival event only.
    pub(crate) fn spawn_member_beside_car(
        &mut self,
        walker: usize,
        home: usize,
        pos: Vector2,
        age_group: u8,
    ) -> usize {
        let i = self.spawn_housed_agent_with_age_group(home, pos.x, pos.y, age_group);
        let trip = self.agents.parking_trip[walker];
        let a = &mut self.agents;
        // The leg as `apply_parking_leg` wrote it for the walker.
        macro_rules! copy {
            ($($field:ident),* $(,)?) => { $( a.$field[i] = a.$field[walker]; )* };
        }
        copy!(
            transit, transit_mode, target_building, current_building, activity, planned_activity,
            planned_target_building, planned_attach_node, planned_detach_node, planned_attach_lane_id,
            planned_detach_lane_id, planned_attach_lane_d, planned_detach_lane_d, access_flags,
            current_node, current_edge, current_lane_id, lane_distance, speed, current_path_index,
            next_replan_time, journey_start_time,
        );
        a.current_path[i] = a.current_path[walker].clone();
        a.parking_trip[i] = ParkingTrip {
            vehicle: trip.vehicle,
            destination: trip.destination,
            target: trip.target,
            phase: ParkingTripPhase::WalkToDoor,
            // From a drop-off's stop (`PARK-34`), whose kerb stop the flag names.
            drop_off: trip.drop_off,
            ..Default::default()
        };
        i
    }

    /// Puts an arrival carrier inside its home at once, as if it had driven there: it leaves the
    /// lane buckets while its lane state still names its entries, enters its home and has its
    /// held departure released, so it then materializes as on an ordinary arrival. Only the
    /// prepopulated economy city (`PopulatedCity`) skips the drive in. O(entries on its lanes).
    pub(crate) fn place_arrival_carrier_at_home(&mut self, agent_idx: usize) {
        self.leave_lane_buckets(agent_idx);
        self.agents.transit[agent_idx] = TRANSIT_IN_BUILDING;
        self.agents.current_building[agent_idx] = self.agents.home_building[agent_idx];
        self.release_departure(agent_idx);
    }

    /// Keeps an agent where it is until [`Self::release_departure`]: its next trip waits for a
    /// paced departure slot (`SIM-02-E`). Uses the replan gate every departure already obeys.
    pub(crate) fn hold_departure(&mut self, agent_idx: usize) {
        self.agents.next_replan_time[agent_idx] = DEPARTURE_HOLD;
    }

    /// Whether the agent waits for a paced departure.
    pub(crate) fn departure_held(&self, agent_idx: usize) -> bool {
        self.agents.next_replan_time[agent_idx] == DEPARTURE_HOLD
    }

    /// Lets a held agent start its trip on the next frame; false when it was not held.
    pub(crate) fn release_departure(&mut self, agent_idx: usize) -> bool {
        let held = self.departure_held(agent_idx);
        if held {
            self.agents.next_replan_time[agent_idx] = 0.0;
        }
        held
    }

    /// Rebuilds [`Self::arrival_carriers`] from the carriers' pending households, in dense
    /// order, after a load replaced the store. O(agents).
    pub(crate) fn rebuild_arrival_carriers(&mut self) {
        if self.completed_parking_deliveries.capacity() < self.agents.len() {
            self.completed_parking_deliveries.reserve(self.agents.len() - self.completed_parking_deliveries.len());
        }
        self.handles.register_appended(self.agents.len());
        self.arrival_carriers = (0..self.agents.len())
            .filter(|&agent_idx| self.agents.pending_household_size[agent_idx] > 0)
            .map(|agent_idx| self.handles.handle(agent_idx))
            .collect();
        self.held_arrivals.clear();
    }

    /// Reads an arrival carrier's approved composition without depending on remapped home ids.
    pub(crate) fn pending_household_composition(
        &self,
        agent_idx: usize,
    ) -> super::HouseholdAgeComposition {
        let adult_count = self.agents.pending_household_adults[agent_idx];
        let elder_count = self.agents.pending_household_elders[agent_idx];
        super::HouseholdAgeComposition {
            adult_count,
            elder_count,
            child_count: self.agents.pending_household_size[agent_idx]
                .saturating_sub(adult_count.saturating_add(elder_count)),
        }
    }

    /// Spawns a physical freight carrier for one building-to-building shipment: standing in the
    /// source's first free dock when it has one, else at its door (`PARK-20`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_city_freight_carrier(
        &mut self,
        shipment_id: u64,
        source_building: usize,
        destination_building: usize,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<usize> {
        let dock = self.free_source_dock(source_building, parking, allocator);
        let ((plan, gate), dock) = self.plan_freight_out_of_dock(
            dock,
            source_building,
            allocator,
            transit_network,
            graph,
            |agents, exit| {
                agents.plan_freight_from_building(
                    source_building,
                    exit,
                    destination_building,
                    allocator,
                    transit_network,
                    graph,
                )
            },
        )?;
        let agent_idx = self.spawn_freight_carrier_base(shipment_id, source_building);
        self.apply_building_origin_freight_plan(agent_idx, source_building, plan, allocator, graph);
        self.start_freight_trip_at_dock(agent_idx, dock, gate, parking);
        if dock.is_none() {
            self.start_in_line_at_kerb(agent_idx, source_building, allocator, graph);
        }
        Some(agent_idx)
    }

    /// Reroutes an existing freight carrier from one building to another.
    pub(crate) fn route_freight_carrier_building_to_building(
        &mut self,
        agent_idx: usize,
        source_building: usize,
        destination_building: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        if agent_idx >= self.agents.len() {
            return false;
        }
        let dock = self.standing_dock(agent_idx, source_building, allocator);
        if self.back_out_without_a_gate(
            agent_idx,
            dock,
            source_building,
            (destination_building, u32::MAX),
            allocator,
            transit_network,
            graph,
        ) {
            return true;
        }
        let Some(((plan, gate), dock)) = self.plan_freight_out_of_dock(
            dock,
            source_building,
            allocator,
            transit_network,
            graph,
            |agents, exit| {
                agents.plan_freight_from_building(
                    source_building,
                    exit,
                    destination_building,
                    allocator,
                    transit_network,
                    graph,
                )
            },
        ) else {
            return false;
        };
        let stop = [self.agents.pos_x[agent_idx], self.agents.pos_y[agent_idx]];
        self.apply_building_origin_freight_plan(agent_idx, source_building, plan, allocator, graph);
        self.agents.freight_target_border_node[agent_idx] = u32::MAX;
        self.pull_out_of_dock(agent_idx, dock, stop, gate);
        true
    }

    /// Spawns a physical freight carrier entering from an outside-world border node.
    pub(crate) fn spawn_import_freight_carrier(
        &mut self,
        shipment_id: u64,
        border_node: u32,
        destination_building: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<usize> {
        let (plan, gate) = self.plan_freight_from_border(
            border_node,
            destination_building,
            allocator,
            transit_network,
            graph,
        )?;
        if border_node as usize >= graph.node_count() {
            return None;
        }
        let agent_idx = self.spawn_freight_carrier_base(shipment_id, usize::MAX);
        self.apply_border_origin_freight_plan(
            agent_idx,
            border_node,
            destination_building,
            plan,
            graph,
        );
        self.set_freight_trip(agent_idx, gate);
        Some(agent_idx)
    }

    /// Reroutes an existing freight carrier from an outside-world border node to a building.
    pub(crate) fn route_freight_carrier_border_to_building(
        &mut self,
        agent_idx: usize,
        border_node: u32,
        destination_building: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        if agent_idx >= self.agents.len() {
            return false;
        }
        let Some((plan, gate)) = self.plan_freight_from_border(
            border_node,
            destination_building,
            allocator,
            transit_network,
            graph,
        ) else {
            return false;
        };
        if border_node as usize >= graph.node_count() {
            return false;
        }
        self.apply_border_origin_freight_plan(
            agent_idx,
            border_node,
            destination_building,
            plan,
            graph,
        );
        self.set_freight_trip(agent_idx, gate);
        true
    }

    /// Spawns a physical freight carrier for one building-to-border export shipment, in the
    /// source's first free dock when it has one (`PARK-20`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_export_freight_carrier(
        &mut self,
        shipment_id: u64,
        source_building: usize,
        border_node: u32,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<usize> {
        let dock = self.free_source_dock(source_building, parking, allocator);
        let (plan, dock) = self.plan_freight_out_of_dock(
            dock,
            source_building,
            allocator,
            transit_network,
            graph,
            |agents, exit| {
                agents.plan_freight_to_border(
                    source_building,
                    exit,
                    border_node,
                    allocator,
                    transit_network,
                    graph,
                )
            },
        )?;
        let agent_idx = self.spawn_freight_carrier_base(shipment_id, source_building);
        self.apply_building_origin_freight_plan(agent_idx, source_building, plan, allocator, graph);
        self.agents.freight_target_border_node[agent_idx] = border_node;
        self.start_freight_trip_at_dock(agent_idx, dock, None, parking);
        if dock.is_none() {
            self.start_in_line_at_kerb(agent_idx, source_building, allocator, graph);
        }
        Some(agent_idx)
    }

    /// Reroutes an existing freight carrier from a building to an outside-world border node.
    pub(crate) fn route_freight_carrier_building_to_border(
        &mut self,
        agent_idx: usize,
        source_building: usize,
        border_node: u32,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        if agent_idx >= self.agents.len() {
            return false;
        }
        let dock = self.standing_dock(agent_idx, source_building, allocator);
        if self.back_out_without_a_gate(
            agent_idx,
            dock,
            source_building,
            (usize::MAX, border_node),
            allocator,
            transit_network,
            graph,
        ) {
            return true;
        }
        let Some((plan, dock)) = self.plan_freight_out_of_dock(
            dock,
            source_building,
            allocator,
            transit_network,
            graph,
            |agents, exit| {
                agents.plan_freight_to_border(
                    source_building,
                    exit,
                    border_node,
                    allocator,
                    transit_network,
                    graph,
                )
            },
        ) else {
            return false;
        };
        let stop = [self.agents.pos_x[agent_idx], self.agents.pos_y[agent_idx]];
        self.apply_building_origin_freight_plan(agent_idx, source_building, plan, allocator, graph);
        self.agents.freight_target_border_node[agent_idx] = border_node;
        self.pull_out_of_dock(agent_idx, dock, stop, None);
        true
    }

    /// Plans carrier `i`'s way out of the dock it leaves again, from `exit` (the dock's
    /// departure: its exit gate's road connection, or its gate's for a dock it backs out of,
    /// `PARK-21`) to where it was going: a border, or a building's
    /// gate or door (`PARK-20`). For a pull-out whose plan a road edit dropped or left stale. It
    /// keeps its place on the dock's route, its dock and its journey clock. False, with nothing
    /// changed, when no road leaves the gate for it. O(one plan).
    pub(crate) fn replan_freight_pull_out(
        &mut self,
        i: usize,
        exit: &BuildingEntrance,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let source = self.agents.current_building[i];
        let border = self.agents.freight_target_border_node[i];
        let planned = if border != u32::MAX {
            self.plan_freight_to_border(source, Some(exit), border, allocator, transit_network, graph)
                .map(|plan| (plan, None))
        } else {
            let destination = self.agents.target_building[i];
            self.plan_freight_from_building(
                source,
                Some(exit),
                destination,
                allocator,
                transit_network,
                graph,
            )
        };
        let Some((plan, gate)) = planned else {
            return false;
        };
        let dock = self.agents.parking_trip[i].source;
        let a = &self.agents;
        let (x, z, started) = (a.pos_x[i], a.pos_y[i], a.journey_start_time[i]);
        self.apply_building_origin_freight_plan(i, source, plan, allocator, graph);
        let a = &mut self.agents;
        a.pos_x[i] = x;
        a.pos_y[i] = z;
        a.journey_start_time[i] = started;
        a.parking_trip[i] = ParkingTrip::freight_pull_out(dock, gate);
        true
    }

    // A carrier's plan from building `source` to building `destination`: to the destination's
    // freight gate, with its building handle, when it has a published dock and a car route
    // reaches the gate; else a car plan to the door (`PARK-20`, `PARK-36`). It starts at `exit`,
    // the source's freight gate, for a truck leaving a dock, else at the source's door. A city
    // without docks pays one emptiness test. O(the plan).
    fn plan_freight_from_building(
        &self,
        source: usize,
        exit: Option<&BuildingEntrance>,
        destination: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<(BuiltTripPlan, Option<u64>)> {
        if let Some((handle, gate)) =
            allocator.freight_gate_arrival(destination, graph, &transit_network.lane_system)
            && let Some(origin) = exit.or_else(|| allocator.entrances.get(source))
            && let Some(plan) = plan_car_trip_between_entrances(
                origin,
                destination,
                2,
                &gate,
                transit_network,
                graph,
                &self.pathfind_count,
            )
        {
            return Some((plan, Some(handle)));
        }
        // A truck drives to the door's kerb stop, from its dock or its source's door alike: never
        // the walk a resident's door-to-door plan chooses for a short trip, which drew a short
        // delivery as a pedestrian on the sidewalk (`PARK-36`).
        plan_car_trip_between_entrances(
            exit.or_else(|| allocator.entrances.get(source))?,
            destination,
            2,
            allocator.entrances.get(destination)?,
            transit_network,
            graph,
            &self.pathfind_count,
        )
        .map(|plan| (plan, None))
    }

    // [`Self::plan_freight_from_building`] for a carrier entering at border node `border`.
    fn plan_freight_from_border(
        &self,
        border: u32,
        destination: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<(BuiltTripPlan, Option<u64>)> {
        if let Some((handle, gate)) =
            allocator.freight_gate_arrival(destination, graph, &transit_network.lane_system)
            && let Some(plan) = plan_border_arrival(
                border,
                destination,
                MODE_CAR,
                &gate,
                transit_network,
                graph,
                &self.pathfind_count,
            )
        {
            return Some((plan, Some(handle)));
        }
        plan_immigration_trip(
            border,
            destination,
            allocator,
            transit_network,
            graph,
            &self.pathfind_count,
        )
        .map(|plan| (plan, None))
    }

    // A carrier's plan from building `source` to border node `border`: from `exit`, the source's
    // freight gate, for a truck leaving a dock, else from its door (`PARK-20`). O(the plan).
    fn plan_freight_to_border(
        &self,
        source: usize,
        exit: Option<&BuildingEntrance>,
        border: u32,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<BuiltTripPlan> {
        match exit {
            Some(exit) => {
                plan_entrance_to_border_trip(exit, border, transit_network, graph, &self.pathfind_count)
            }
            None => plan_building_to_border_trip(
                source,
                border,
                allocator,
                transit_network,
                graph,
                &self.pathfind_count,
            ),
        }
    }

    // `plan` from `dock` of building `building` out through its freight gate, with the dock; or,
    // with no dock, no gate or no route from the gate, `plan` from the door as today (`PARK-20`).
    // O(one or two plans).
    fn plan_freight_out_of_dock<T>(
        &self,
        dock: Option<Handle<Place>>,
        building: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        plan: impl Fn(&Self, Option<&BuildingEntrance>) -> Option<T>,
    ) -> Option<(T, Option<Handle<Place>>)> {
        if let Some(dock) = dock
            && let Some(exit) =
                allocator.freight_gate_departure(building, graph, &transit_network.lane_system)
            && let Some(planned) = plan(self, Some(&exit))
        {
            return Some((planned, Some(dock)));
        }
        plan(self, None).map(|planned| (planned, None))
    }

    // The first free dock of building `source` for a carrier dispatched from it (owner decision
    // 10, `PARK-20`). A city without docks pays one emptiness test. O(the building's docks).
    fn free_source_dock(
        &self,
        source: usize,
        parking: &ParkingSystem,
        allocator: &BuildingAllocator,
    ) -> Option<Handle<Place>> {
        if allocator.freight_gates.is_empty() || source >= allocator.buildings.len() {
            return None;
        }
        self.first_free_dock(allocator.building_handles.handle(source).to_bits(), parking)
    }

    // A truck standing in `dock` of building `source` whose gate record went under it: an edit
    // retired the building's last dock (the held row stays, inactive); or whose retired row
    // leaves by another gate than the record's (`PARK-21`, `FreightGate::elsewhere`). It leaves
    // its dock all the same, to `to` (a building, or a border node), but its plan needs the
    // dock's own departure, which the parking system holds: it waits in the bay, frozen for the parking
    // phase's ordered commit to plan it (`yard.rs`), which then retries on the wheel if no road
    // leaves the gate. False, with nothing changed, for any other carrier: one test of the
    // dock found before (`PARK-20`). O(1).
    #[allow(clippy::too_many_arguments)]
    fn back_out_without_a_gate(
        &mut self,
        i: usize,
        dock: Option<Handle<Place>>,
        source: usize,
        to: (usize, u32),
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let Some(dock) = dock else {
            return false;
        };
        if allocator
            .freight_gate_departure(source, graph, &transit_network.lane_system)
            .is_some()
            && !allocator.freight_dock_leaves_elsewhere(source, dock.to_bits())
        {
            return false;
        }
        let now = self.sim_time;
        let a = &mut self.agents;
        a.current_building[i] = source;
        (a.target_building[i], a.freight_target_border_node[i]) = to;
        a.journey_start_time[i] = now;
        a.transit[i] = TRANSIT_ACCESS_EGRESS;
        a.access_flags[i] = 0;
        a.current_path[i].clear();
        a.current_path_index[i] = 0;
        a.speed[i] = 0.0;
        a.network_replan_failures[i] = 0;
        a.parking_trip[i] = ParkingTrip {
            ready: true,
            replan_needed: true,
            ..ParkingTrip::freight_pull_out(dock, None)
        };
        true
    }

    // The dock carrier `i` stands in at building `building` after delivering there, which it
    // leaves by pulling out (`PARK-20`). One phase test for a carrier at a door. O(1).
    fn standing_dock(
        &self,
        i: usize,
        building: usize,
        allocator: &BuildingAllocator,
    ) -> Option<Handle<Place>> {
        let trip = &self.agents.parking_trip[i];
        (trip.phase == ParkingTripPhase::AtDock
            && trip.freight_gate
            && building < allocator.buildings.len()
            && trip.target == allocator.building_handles.handle(building).to_bits())
        .then_some(trip.destination)
    }

    // A dispatched carrier's trip: standing in `dock`, which it takes, and pulling out of it at
    // once; or from the door, its plan ending at the gate of building `gate` or not
    // (`PARK-20`). O(1).
    fn start_freight_trip_at_dock(
        &mut self,
        i: usize,
        dock: Option<Handle<Place>>,
        gate: Option<u64>,
        parking: &mut ParkingSystem,
    ) {
        let stop = dock.and_then(|dock| parking.dock_route(dock)).map(|route| route[3]);
        match dock.zip(stop) {
            Some((dock, stop)) => {
                parking.set_dock_holder(dock, self.handles.handle(i));
                self.pull_out_of_dock(i, Some(dock), stop, gate);
            }
            None => self.set_freight_trip(i, gate),
        }
    }

    // A planned carrier's trip out of `dock`, from `stop`, the route's end where it stands
    // (`PARK-20`): it keeps the dock, whose recorded holder it already is, until its pull-out
    // reaches the lane. Without a dock, [`Self::set_freight_trip`]. O(1).
    fn pull_out_of_dock(
        &mut self,
        i: usize,
        dock: Option<Handle<Place>>,
        stop: [f32; 2],
        gate: Option<u64>,
    ) {
        let Some(dock) = dock else {
            self.set_freight_trip(i, gate);
            return;
        };
        self.agents.pos_x[i] = stop[0];
        self.agents.pos_y[i] = stop[1];
        self.agents.parking_trip[i] = ParkingTrip::freight_pull_out(dock, gate);
    }

    // Starts carrier `i`'s yard trip to the building whose handle bits `gate` names, or ends the
    // one it had: a new plan replaces every phase, and a dock the carrier held is free again,
    // since a hold counts only while its holder's trip names it (`PARK-20`). A carrier that never
    // had a yard trip is not written. O(1).
    fn set_freight_trip(&mut self, i: usize, gate: Option<u64>) {
        let trip = &mut self.agents.parking_trip[i];
        if let Some(target) = gate {
            *trip = ParkingTrip {
                target,
                freight_gate: true,
                ..ParkingTrip::default()
            };
        } else if trip.freight_gate || trip.phase != ParkingTripPhase::None {
            // A truck that stood at a kerb stop (`PARK-36`) leaves it with its new plan.
            *trip = ParkingTrip::default();
        }
    }

    /// Sends truck `i`, waiting in the line at the kerb stop of building `building` (`PARK-36`),
    /// to the building's freight gate for `dock`, which the parking phase has just granted it: it
    /// pulls out of its place as from a dispatch at the kerb stop, drives to the gate's lane point
    /// (round the block when the gate lies behind it or on another road) and holds the dock on the
    /// way (`ParkingTrip::held_dock`). False, with nothing changed, when the building has no gate or
    /// no road leads from its kerb to the gate. O(one plan).
    pub(crate) fn drive_from_kerb_to_gate(
        &mut self,
        i: usize,
        building: usize,
        dock: Handle<Place>,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let Some((handle, gate)) =
            allocator.freight_gate_arrival(building, graph, &transit_network.lane_system)
        else {
            return false;
        };
        let Some(plan) = allocator.entrances.get(building).and_then(|door| {
            plan_car_trip_between_entrances(
                door,
                building,
                2,
                &gate,
                transit_network,
                graph,
                &self.pathfind_count,
            )
        }) else {
            return false;
        };
        self.apply_freight_plan_where_it_stands(i, building, plan, allocator, graph);
        self.agents.parking_trip[i] = ParkingTrip {
            target: handle,
            destination: dock,
            freight_gate: true,
            ..ParkingTrip::default()
        };
        true
    }

    /// Sends truck `i`, standing at the lane point of building `building`'s freight gate with
    /// every dock held (`PARK-36`), on to place `slot` of the line at the building's kerb stop
    /// ([`super::tick::kerb_queue_entrance`]): it joins the lane from where it stands and leaves it
    /// again beside its place, round the block when the place lies behind the gate or on another
    /// road. Its place in the queue, its `journey_start_time`, is kept. False, with nothing
    /// changed, when no road leads there. O(one plan).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn drive_from_gate_to_kerb(
        &mut self,
        i: usize,
        building: usize,
        slot: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let Some((handle, gate)) =
            allocator.freight_gate_arrival(building, graph, &transit_network.lane_system)
        else {
            return false;
        };
        let Some(plan) = allocator
            .entrances
            .get(building)
            .and_then(|door| super::tick::kerb_queue_entrance(door, graph, slot))
            .and_then(|place| {
                plan_car_trip_between_entrances(
                    &gate,
                    building,
                    2,
                    &place,
                    transit_network,
                    graph,
                    &self.pathfind_count,
                )
            })
        else {
            return false;
        };
        self.apply_freight_plan_where_it_stands(i, building, plan, allocator, graph);
        self.agents.parking_trip[i] = ParkingTrip {
            target: handle,
            phase: ParkingTripPhase::ToKerb,
            freight_gate: true,
            ..ParkingTrip::default()
        };
        true
    }

    // `plan` out of building `building` for truck `i`, which leaves from where it stands (a
    // place in the line at the kerb stop, or the gate's lane point) rather than the kerb stop,
    // its journey clock, the queue's stamp, kept (`PARK-36`). O(1) beyond the plan.
    fn apply_freight_plan_where_it_stands(
        &mut self,
        i: usize,
        building: usize,
        plan: crate::simulation::economy::agents::tick::BuiltTripPlan,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) {
        let a = &self.agents;
        let (x, z, started) = (a.pos_x[i], a.pos_y[i], a.journey_start_time[i]);
        self.apply_building_origin_freight_plan(i, building, plan, allocator, graph);
        let a = &mut self.agents;
        (a.pos_x[i], a.pos_y[i], a.journey_start_time[i]) = (x, z, started);
    }

    /// Checks that an agent slot still belongs to the expected physical shipment.
    pub(crate) fn freight_carrier_matches(&self, index: usize, shipment_id: u64) -> bool {
        index < self.agents.len()
            && self.agents.vehicle_type[index] == VEHICLE_FREIGHT_DELIVERY
            && self.agents.freight_shipment_id[index] == shipment_id
    }

    /// Removes only the expected freight carrier and returns any swap-remap pair.
    pub(crate) fn remove_freight_carrier(
        &mut self,
        index: usize,
        shipment_id: u64,
    ) -> Option<(usize, usize)> {
        if !self.freight_carrier_matches(index, shipment_id) {
            return None;
        }
        self.remove_agent_record(index, "finished its freight trip")
    }

    /// Permanently removes an agent from the simulation using O(1) swap-remove.
    ///
    /// Returns `(old_idx, new_idx)` when another agent moved; handles follow it automatically.
    pub fn kill_agent(
        &mut self,
        index: usize,
        allocator: &mut BuildingAllocator,
    ) -> Option<(usize, usize)> {
        if index >= self.agents.len() {
            return None;
        }

        // Vacancy for residential is now household-based and managed by the HouseholdSystem.
        // Worker count for non-residential remains agent-based.
        let work = self.agents.work_building[index];
        if age_group_can_work(self.agents.age_group[index])
            && work != usize::MAX
            && work < allocator.buildings.len()
        {
            allocator.set_worker_count(
                work,
                allocator.buildings[work].worker_count.saturating_sub(1),
            );
        }

        self.remove_agent_record(index, "left the city with its household")
    }

    // `reason` explains the removal when it ends an inspection.
    fn remove_agent_record(
        &mut self,
        index: usize,
        reason: &'static str,
    ) -> Option<(usize, usize)> {
        let last = self.agents.len() - 1;
        let moved_household = self.unlink_for_swap_remove(index);
        self.swap_remove_lane_bucket_agent(index);
        self.handles.swap_remove(index, self.agents.len());
        self.agents.swap_remove(index);
        self.relink_after_swap_remove(index, moved_household);
        self.repair_inspection_after_remove(index, last, reason);
        if index != last {
            Some((last, index))
        } else {
            None
        }
    }
}

impl AgentSystem {
    fn new_agent(&mut self, home: usize, init_x: f32, init_y: f32) -> Agent {
        let spawn_index = self.agents.len() as u32;
        let schedule_seed = stable_schedule_seed(home, spawn_index);
        let render_id = self.allocate_render_id();
        let visual_seed = agent_visual_seed(home, spawn_index);
        Agent {
            home_building: home,
            household_id: usize::MAX,
            age_group: AGE_ADULT,
            pending_household_size: 0,
            pending_household_adults: 0,
            pending_household_elders: 0,
            freight_shipment_id: u64::MAX,
            work_building: usize::MAX,
            pos_x: init_x,
            pos_y: init_y,
            render_id,
            activity: 0,
            transit: TRANSIT_IN_BUILDING,
            happiness: 50.0,
            journey_start_time: self.sim_time,
            schedule_seed,
            cached_commute_minutes: 0,
            next_commute_refresh_time: 0.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            current_building: home,
            target_building: usize::MAX,
            planned_target_building: usize::MAX,
            freight_target_border_node: u32::MAX,
            current_node: u32::MAX,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            network_replan_failures: 0,
            current_edge: usize::MAX,
            current_lane_id: usize::MAX,
            lane_distance: 0.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            speed: 0.0,
            transit_mode: MODE_WALK,
            planned_activity: 0,
            current_path: Vec::new(),
            current_path_index: 0,
            walk_lanes: Vec::new(),
            has_car: false,
            parking_trip: Default::default(),
            vehicle_type: stable_index(visual_seed ^ 0xA11C_EC7A, 4) as u8,
            pedestrian_type: stable_index(visual_seed ^ 0x51DE_CAFE, 4) as u8,
            walk_phase: stable_unit_f32(visual_seed ^ 0xBEE5_100D),
            job_lock_until_day: 0,
            consecutive_unpaid_days: 0,
        }
    }

    fn insert_agent(&mut self, agent: Agent) -> usize {
        let index = self.agents.len();
        self.agents.push(agent);
        if self.parking_retries.capacity() < self.agents.len() { self.parking_retries.reserve(self.agents.len() - self.parking_retries.len()); }
        if self.completed_parking_deliveries.capacity() < self.agents.len() {
            self.completed_parking_deliveries.reserve(self.agents.len() - self.completed_parking_deliveries.len());
        }
        self.handles.register_appended(self.agents.len());
        self.register_appended_household_members();
        // The lane-bucket snapshot reads appended agents at the next tick (`SIM-02-K`).
        index
    }

    fn spawn_freight_carrier_base(&mut self, shipment_id: u64, origin_building: usize) -> usize {
        let mut agent = self.new_agent(origin_building, 0.0, 0.0);
        agent.home_building = usize::MAX;
        // Freight is not a resident and must remain outside adult employment accounting.
        agent.age_group = AGE_CHILD;
        agent.freight_shipment_id = shipment_id;
        agent.activity = 2;
        agent.transit_mode = MODE_CAR;
        agent.vehicle_type = VEHICLE_FREIGHT_DELIVERY;
        self.insert_agent(agent)
    }

    // Truck `i`, just dispatched from building `building` with no dock to stand in, starts at
    // the kerb stop; one dispatched from the same building at the same moment as others starts
    // one place behind the last of them in the line along the kerb (`kerb_queue_entrance`), so
    // none is drawn over another (`PARK-36`). Each pulls out from where it stands, as a truck
    // granted a dock leaves the line. Its timing is the dispatch's: nothing waits. Nothing is
    // saved: only this moment's starts count. O(starts at this moment + edge points).
    fn start_in_line_at_kerb(
        &mut self,
        i: usize,
        building: usize,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) {
        let now = self.sim_time.to_bits();
        let starts = &mut self.parking_work.kerb_starts;
        starts.retain(|&(_, at)| at == now);
        let place = starts.iter().filter(|&&(b, _)| b == building).count();
        starts.push((building, now));
        if place == 0 {
            return;
        }
        if let Some(stop) = allocator
            .entrances
            .get(building)
            .and_then(|door| super::tick::kerb_queue_entrance(door, graph, place))
        {
            self.agents.pos_x[i] = stop.door_pos.x;
            self.agents.pos_y[i] = stop.door_pos.y;
        }
    }

    // A truck leaving building `source_building` starts at its kerb stop, standing out of the
    // driving lane, and pulls out from there: never from the door (`PARK-36`). One leaving a
    // dock is put in its bay by its caller.
    fn apply_building_origin_freight_plan(
        &mut self,
        agent_idx: usize,
        source_building: usize,
        plan: crate::simulation::economy::agents::tick::BuiltTripPlan,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) {
        if let Some(origin_entrance) = allocator.entrances.get(source_building) {
            let stop = super::tick::kerb_stop_entrance(
                origin_entrance,
                graph,
                super::tick::FREIGHT_KERB_INSET_M,
            )
            .map_or(origin_entrance.door_pos, |kerb| kerb.door_pos);
            self.agents.pos_x[agent_idx] = stop.x;
            self.agents.pos_y[agent_idx] = stop.y;
        }
        self.agents.current_building[agent_idx] = source_building;
        self.agents.target_building[agent_idx] = plan.target_building;
        self.agents.activity[agent_idx] = plan.activity;
        self.agents.journey_start_time[agent_idx] = self.sim_time;
        self.agents.transit_mode[agent_idx] = plan.mode;
        self.agents.planned_attach_node[agent_idx] = plan.planned_attach_node;
        self.agents.planned_detach_node[agent_idx] = plan.planned_detach_node;
        self.agents.planned_attach_lane_id[agent_idx] = plan.planned_attach_lane_id as u32;
        self.agents.planned_detach_lane_id[agent_idx] = plan.planned_detach_lane_id as u32;
        self.agents.planned_attach_lane_d[agent_idx] = plan.planned_attach_lane_d;
        self.agents.planned_detach_lane_d[agent_idx] = plan.planned_detach_lane_d;
        self.agents.access_flags[agent_idx] = plan.access_flags;
        self.agents.next_replan_time[agent_idx] = 0.0;
        self.agents.network_replan_failures[agent_idx] = 0;
        self.agents.current_node[agent_idx] = u32::MAX;
        self.agents.current_edge[agent_idx] = usize::MAX;
        self.agents.current_lane_id[agent_idx] = usize::MAX;
        self.agents.lane_distance[agent_idx] = 0.0;
        self.agents.lane_change_from_lane_id[agent_idx] = u32::MAX;
        self.agents.lane_change_start_d[agent_idx] = 0.0;
        self.agents.lane_change_length_m[agent_idx] = 0.0;
        self.agents.overtake_blocked_time_s[agent_idx] = 0.0;
        self.agents.overtake_cooldown_s[agent_idx] = 0.0;
        self.agents.speed[agent_idx] = 0.0;
        if (plan.access_flags & ACCESS_FREIGHT_BORDER_DESTINATION) != 0 {
            self.agents.target_building[agent_idx] = usize::MAX;
            self.agents.planned_detach_lane_id[agent_idx] = u32::MAX;
        }
        self.agents.current_path[agent_idx] = plan.current_path;
        self.agents.walk_lanes[agent_idx].clear();
        self.agents.current_path_index[agent_idx] =
            if self.agents.current_path[agent_idx].len() >= 2 {
                1
            } else {
                0
            };
        self.agents.transit[agent_idx] = TRANSIT_ACCESS_EGRESS;
    }

    fn apply_border_origin_freight_plan(
        &mut self,
        agent_idx: usize,
        border_node: u32,
        destination_building: usize,
        plan: crate::simulation::economy::agents::tick::BuiltTripPlan,
        graph: &RegionGraph,
    ) {
        let pos = graph.node(border_node).pos;
        self.agents.pos_x[agent_idx] = pos.x;
        self.agents.pos_y[agent_idx] = pos.z;
        self.agents.current_building[agent_idx] = usize::MAX;
        self.agents.target_building[agent_idx] = destination_building;
        self.agents.planned_target_building[agent_idx] = usize::MAX;
        self.agents.freight_target_border_node[agent_idx] = u32::MAX;
        self.agents.current_node[agent_idx] = border_node;
        self.agents.current_edge[agent_idx] = usize::MAX;
        self.agents.current_lane_id[agent_idx] = usize::MAX;
        self.agents.lane_distance[agent_idx] = 0.0;
        self.agents.lane_change_from_lane_id[agent_idx] = u32::MAX;
        self.agents.lane_change_start_d[agent_idx] = 0.0;
        self.agents.lane_change_length_m[agent_idx] = 0.0;
        self.agents.overtake_blocked_time_s[agent_idx] = 0.0;
        self.agents.overtake_cooldown_s[agent_idx] = 0.0;
        self.agents.transit[agent_idx] = TRANSIT_NETWORK;
        self.agents.transit_mode[agent_idx] = plan.mode;
        self.agents.speed[agent_idx] = 0.0;
        self.agents.planned_attach_node[agent_idx] = plan.planned_attach_node;
        self.agents.planned_detach_node[agent_idx] = plan.planned_detach_node;
        self.agents.planned_attach_lane_id[agent_idx] = u32::MAX;
        self.agents.planned_detach_lane_id[agent_idx] = plan.planned_detach_lane_id as u32;
        self.agents.planned_attach_lane_d[agent_idx] = 0.0;
        self.agents.planned_detach_lane_d[agent_idx] = plan.planned_detach_lane_d;
        self.agents.access_flags[agent_idx] = plan.access_flags;
        self.agents.next_replan_time[agent_idx] = 0.0;
        self.agents.network_replan_failures[agent_idx] = 0;
        self.agents.current_path[agent_idx] = plan.current_path;
        self.agents.walk_lanes[agent_idx].clear();
        self.agents.current_path_index[agent_idx] =
            if self.agents.current_path[agent_idx].len() >= 2 {
                1
            } else {
                0
            };
    }
}

fn stable_schedule_seed(home_building: usize, spawn_index: u32) -> u32 {
    let mixed = (home_building as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(u64::from(spawn_index).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    ((mixed >> 32) as u32) ^ (mixed as u32)
}

fn agent_visual_seed(home_building: usize, spawn_index: u32) -> u64 {
    (home_building as u64)
        .wrapping_mul(0x94D0_49BB_1331_11EB)
        .wrapping_add(u64::from(spawn_index).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fmt::Write, hash::Hasher, hint::black_box, time::Instant};

    #[test]
    #[ignore = "manual matched release timing of agent initialization and SoA insertion"]
    fn benchmark_agent_spawning() {
        struct StateHash(std::collections::hash_map::DefaultHasher);
        impl std::fmt::Write for StateHash {
            fn write_str(&mut self, value: &str) -> std::fmt::Result {
                self.0.write(value.as_bytes());
                Ok(())
            }
        }

        for count in [1_024, 16_384, 131_072] {
            for mode in ["housed", "border", "freight", "mixed"] {
                let mut system = AgentSystem::new();
                system.agents.reserve(count);
                system.sim_time = 123.5;
                let mut samples = [0.0; 21];
                for sample in 0..24 {
                    // Retain column capacity; reset fixture identity outside measured spawning.
                    system.agents.clear();
                    system.next_render_id = 17;
                    let start = Instant::now();
                    for index in 0..black_box(count) {
                        let origin = if index % 7 == 0 {
                            usize::MAX
                        } else {
                            index % 97
                        };
                        let x = index as f32 * 0.25;
                        let kind = match mode {
                            "housed" => 0,
                            "border" => 1,
                            "freight" => 2,
                            _ => index % 3,
                        };
                        match kind {
                            0 => {
                                system.spawn_housed_agent_with_age_group(
                                    origin,
                                    x,
                                    -x,
                                    (index % 3) as u8,
                                );
                            }
                            1 => {
                                system.spawn_household_arrival_carrier(
                                    origin,
                                    (index % 6) as u16,
                                    crate::simulation::economy::agents::household_age_composition(
                                        origin,
                                        index,
                                        (index % 6) as u16,
                                    ),
                                    (index % 31) as u32,
                                    x,
                                    -x,
                                );
                            }
                            _ => {
                                system.spawn_freight_carrier_base(index as u64 + 51, origin);
                            }
                        }
                    }
                    let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
                    if sample >= 3 {
                        samples[sample - 3] = elapsed;
                    }
                    black_box(&system);
                }
                // Hash every generated column, including schedule/visual seeds, outside timing.
                let mut hash = StateHash(Default::default());
                write!(&mut hash, "{:?}", system.agents).unwrap();
                samples.sort_by(f64::total_cmp);
                eprintln!(
                    "agent_spawning agents={count} mode={mode} median_ms={:.6} checksum={}",
                    samples[10],
                    hash.0.finish()
                );
            }
        }
    }
}
