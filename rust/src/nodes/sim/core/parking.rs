// SPDX-License-Identifier: GPL-2.0-only

//! Local physical parking publication at existing road and site edit boundaries.

use super::SimCore;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::parking::kerb::{kerb_bays, kerb_blockers};
use crate::simulation::parking::{ParkedPublication, PlaceSource};
use crate::simulation::parking::lots::lot_bays;
use rayon::prelude::*;
use std::collections::BTreeSet;

impl SimCore {
    /// O(affected edges' geometry + local candidates × lot validation), never a city-wide supply rebuild.
    pub(crate) fn reconcile_parking_frontages(&mut self, edges: &BTreeSet<usize>) {
        if edges.is_empty() {
            return;
        }
        if self.allocator.building_sites.len() != self.allocator.buildings.len() {
            self.allocator
                .rebuild_building_site_clients(self.zoning.config.zone_cell_m);
        }
        // The site-cell index is maintained by site lifecycle helpers. Parking never rebuilds
        // the economy's zone/vacancy index, whose order affects admission.
        let mut buildings = Vec::new();
        for &edge in edges {
            if let Some(record) = self.region_graph.get_edge(edge) {
                for pair in record.physical_geometry.windows(2) {
                    self.allocator.site_cells.visit(
                        [
                            pair[0].x.min(pair[1].x) - 8.0,
                            pair[0].z.min(pair[1].z) - 8.0,
                            pair[0].x.max(pair[1].x) + 8.0,
                            pair[0].z.max(pair[1].z) + 8.0,
                        ],
                        |i| buildings.push(i),
                    );
                }
            }
        }
        buildings.sort_unstable();
        buildings.dedup();
        let lots: Vec<_> = crate::thread_pools::install(|| {
            buildings
                .into_par_iter()
                .filter_map(|index| {
                    let handle = self.allocator.building_handles.handle(index);
                    if handle.is_none() {
                        return None;
                    }
                    let (bays, issues) = lot_bays(
                        &self.allocator,
                        index,
                        &self.region_graph,
                        &self.transit_network.roadbed,
                    );
                    let blockers = kerb_blockers(&self.allocator, index, &self.region_graph, &bays);
                    Some((handle, bays, issues, blockers))
                })
                .collect()
        });
        // Geometry is independent; live occupancy reconciliation commits in source order.
        let mut points = Vec::new();
        let mut kerb_edges = edges.clone();
        for (handle, bays, issues, blockers) in lots {
            self.parking
                .set_kerb_blockers(handle.to_bits(), blockers, &mut kerb_edges);
            points.extend(self.parking.source_points(handle.to_bits()));
            points.extend(bays.iter().map(|(_, b)| [b.position[0], b.position[2]]));
            if let Some(b) = self.allocator.building_handles.resolve(handle) {
                if let Some(e) = self.allocator.entrances.get(b) {
                    points.push([e.door_pos.x, e.door_pos.y]);
                }
            } else {
                self.parking.remove_destination_candidates(handle.to_bits());
            }
            self.parking.replace_building(handle.to_bits(), &bays);
            if issues.is_empty() {
                self.parking.diagnostics.remove(&handle.to_bits());
            } else {
                self.parking.diagnostics.insert(handle.to_bits(), issues);
            }
        }
        points.extend(
            edges
                .iter()
                .filter_map(|&edge| self.region_graph.get_edge(edge))
                .flat_map(|edge| {
                    edge.physical_geometry.windows(2).flat_map(|w| {
                        let steps = (w[0].distance_to(w[1]) / 200.0).ceil().max(1.0) as usize;
                        (0..=steps).map(move |i| {
                            let p = w[0].lerp(w[1], i as f32 / steps as f32);
                            [p.x, p.z]
                        })
                    })
                }),
        );
        self.reconcile_kerb_parking(&kerb_edges, &mut points);
        self.rebuild_parking_candidates_near(&points);
    }

    // Both parking lanes of each edge, against the blockers already published for it. Geometry
    // runs in parallel; custody reconciles in edge order. O(edges' sections + bays × blockers).
    fn reconcile_kerb_parking(&mut self, edges: &BTreeSet<usize>, points: &mut Vec<[f32; 2]>) {
        let jobs: Vec<_> = edges
            .iter()
            .map(|&edge| (edge, self.parking.edge_kerb_blockers(edge)))
            .collect();
        let bays: Vec<_> = crate::thread_pools::install(|| {
            jobs.into_par_iter()
                .map(|(edge, blockers)| {
                    let bays = kerb_bays(
                        edge,
                        &self.region_graph,
                        &self.transit_network.roadbed,
                        &blockers,
                    );
                    (edge, bays)
                })
                .collect()
        });
        for (edge, bays) in bays {
            points.extend(self.parking.kerb_points(edge));
            points.extend(
                bays.iter()
                    .flatten()
                    .map(|b| [b.position[0], b.position[2]]),
            );
            self.parking.replace_kerb(edge, &bays);
        }
    }

    /// O(changed buildings × lot validation), consuming the allocator's
    /// stable-handle site journal. Removed handles remove their old source lists even after swaps.
    pub(crate) fn publish_parking_sites(&mut self) {
        let mut changes = std::mem::take(&mut self.allocator.parking_site_changes);
        if changes.is_empty() {
            self.allocator.parking_site_changes = changes;
            return;
        }
        changes.sort_unstable();
        changes.dedup();
        let lots: Vec<_> = crate::thread_pools::install(|| {
            changes
                .par_iter()
                .map(|&handle| {
                    let Some(i) = self.allocator.building_handles.resolve(handle) else {
                        return (handle, Vec::new(), Vec::new(), Vec::new());
                    };
                    let (bays, issues) = lot_bays(
                        &self.allocator,
                        i,
                        &self.region_graph,
                        &self.transit_network.roadbed,
                    );
                    let blockers = kerb_blockers(&self.allocator, i, &self.region_graph, &bays);
                    (handle, bays, issues, blockers)
                })
                .collect()
        });
        let mut points = Vec::new();
        let mut kerb_edges = BTreeSet::new();
        for (handle, bays, issues, blockers) in lots {
            self.parking
                .set_kerb_blockers(handle.to_bits(), blockers, &mut kerb_edges);
            points.extend(self.parking.source_points(handle.to_bits()));
            points.extend(bays.iter().map(|(_, b)| [b.position[0], b.position[2]]));
            if let Some(b) = self.allocator.building_handles.resolve(handle) {
                if let Some(e) = self.allocator.entrances.get(b) {
                    points.push([e.door_pos.x, e.door_pos.y]);
                }
            } else {
                self.parking.remove_destination_candidates(handle.to_bits());
            }
            self.parking.replace_building(handle.to_bits(), &bays);
            if issues.is_empty() {
                self.parking.diagnostics.remove(&handle.to_bits());
            } else {
                self.parking.diagnostics.insert(handle.to_bits(), issues);
            }
        }
        self.reconcile_kerb_parking(&kerb_edges, &mut points);
        self.rebuild_parking_candidates_near(&points);
        changes.clear();
        self.allocator.parking_site_changes = changes;
    }
}

impl SimCore {
    /// The sim thread's end of a step: answers the renderer, rebuilds dirty parked chunks and
    /// returns the publication for the snapshot being built, whose moving cars come from the
    /// same custody state. O(1) and allocation-free when nothing is dirty.
    pub(crate) fn publish_parked_chunks(&mut self) -> ParkedPublication {
        self.parking.take_renderer_answers();
        self.build_parked_chunks();
        self.parking.publication()
    }

    /// Rebuilds dirty parked chunks from the cached poses; a place without a pose for its car's
    /// model takes the existing vehicle-contact solve once. Reads the live ground, so it runs
    /// where the site index is ready. O(dirty chunks' places + new poses × Q).
    pub(crate) fn build_parked_chunks(&mut self) {
        if !self.parking.has_dirty_chunks() {
            return;
        }
        let built = self
            .parking
            .solve_dirty_chunks(|model, source, pos, heading| {
                self.parked_ground_pose(model, source, pos, heading)
            });
        self.parking.install_dirty_chunks(built);
    }

    // A parked car's row-major instance pose, on its own street or lot. A bay on a bridge or in
    // a tunnel keeps to its own level; every other bay stands on the ground. O(contacts).
    pub(super) fn parked_ground_pose(
        &self,
        model: u8,
        source: PlaceSource,
        pos: [f32; 3],
        heading: f32,
    ) -> [f32; 12] {
        let (s, c) = heading.sin_cos();
        let on_structure = matches!(source, PlaceSource::Kerb { edge, .. }
            if self.region_graph.get_edge(edge as usize)
                .is_some_and(|edge| edge.class != EdgeClass::Standard));
        let (y, basis) = self.vehicle_parked_pose(
            model,
            godot::prelude::Vector2::new(pos[0], pos[2]),
            godot::prelude::Vector3::new(s, 0.0, c),
            on_structure.then_some(pos[1]),
        );
        let [x_axis, y_axis, z_axis] = basis;
        [
            x_axis.x, y_axis.x, z_axis.x, pos[0], x_axis.y, y_axis.y, z_axis.y, y, x_axis.z,
            y_axis.z, z_axis.z, pos[2],
        ]
    }

    /// Forgets the parked poses within `reach` of each changed rectangle, plus the longest car's
    /// contacts. The graded edit journal's records call this, for every change of the ground a
    /// parked car stands on that they see. O(rectangles × their chunks' places).
    pub(crate) fn invalidate_parked_ground(
        &mut self,
        changed: impl IntoIterator<Item = [f64; 4]>,
        reach: f64,
    ) {
        let reach = reach + f64::from(self.vehicle_contact_reach_m());
        for [min_x, min_z, max_x, max_z] in changed {
            self.parking
                .invalidate_ground([min_x - reach, min_z - reach, max_x + reach, max_z + reach]);
        }
    }
}

impl SimCore {
    /// Consumes household lifecycle events, never scans sleeping households or cars. O(events
    /// + affected members + local catchment entries); allocations are lifecycle preparation.
    pub(crate) fn publish_parking_households(&mut self) {
        use crate::simulation::zoning::profiles::{ZoneDensity, load_builtin_profile_registry};
        let mut events = std::mem::take(&mut self.households.parking_changes);
        if events.is_empty() {
            self.households.parking_changes = events;
            return;
        }
        let profiles = load_builtin_profile_registry().expect("built-in zoning profiles");
        for &handle in &events {
            let owner = handle.to_bits();
            let Some(h) = self.households.handles.resolve(handle) else {
                self.parking.remove_household_car(owner);
                continue;
            };
            let household = &self.households.households[h];
            let Some(home) = self.allocator.buildings.get(household.home_building_id) else {
                continue;
            };
            let home_handle = self
                .allocator
                .building_handles
                .handle(household.home_building_id)
                .to_bits();
            let density = profiles
                .density_for_runtime_id(home.zone_profile_runtime_id)
                .unwrap_or(ZoneDensity::Low);
            let members: Vec<_> = self.agents.household_members(h).collect();
            let arrival = members.iter().copied().find(|&i| self.agents.parking_trip[i].arrival_ownership);
            if let Some(carrier) = arrival {
                self.parking.adopt_arrival_car(owner, self.agents.parking_trip[carrier].vehicle);
                self.agents.agents.parking_trip[carrier].arrival_ownership = false;
            } else {
                self.parking.assign_household(owner, density, [home.center_x, home.center_y]);
            }
            let car = self.parking.household_car(owner);
            if arrival.is_none() { self.parking.park_new_car(car, home_handle); }
            for member in members {
                self.agents.agents.parking_trip[member].vehicle = car;
            }
        }
        events.clear();
        self.households.parking_changes = events;
    }

    /// Rebuilds only door catchments through the maintained site-cell index around changed
    /// supply/doors. O(local chunk buildings + local bounded searches), independent of background.
    pub(crate) fn rebuild_parking_candidates_near(&mut self, points: &[[f32; 2]]) {
        self.parking
            .refresh_retired_access(&self.allocator, &self.transit_network, &self.region_graph);
        let mut buildings = Vec::new();
        for &point in points {
            self.allocator.site_cells.visit(
                [
                    point[0] - 512.0,
                    point[1] - 512.0,
                    point[0] + 512.0,
                    point[1] + 512.0,
                ],
                |b| {
                    if self.allocator.entrances.get(b).is_some_and(|e| {
                        (e.door_pos.x - point[0]).hypot(e.door_pos.y - point[1]) <= 512.0
                    }) {
                        buildings.push(b);
                    }
                },
            );
        }
        buildings.sort_unstable();
        buildings.dedup();
        // Only homes whose reachable places changed have new supply; added bays woke their
        // own chunks in `replace_source`.
        for door in self.publish_parking_catchments(&buildings) {
            self.parking.rehouse_near(door);
        }
    }
}

impl SimCore {
    /// Drains explicit retry requests into the saved timing wheel; no polling timer.
    /// 30 s for unavailable routes, next eligible slot for requested route-buffer preparation.
    /// O(retries × log scheduled events), outside agent movement.
    pub(crate) fn schedule_parking_retries(&mut self) {
        let mut requests = std::mem::take(&mut self.agents.parking_retries);
        for (agent, seconds) in requests.drain(..) {
            if let Some(i) = self.agents.resolve_agent(agent) {
                let needed = self.agents.agents.parking_trip[i].route_capacity;
                let route = &mut self.agents.agents.current_path[i];
                if needed > route.capacity() {
                    route.reserve(needed - route.len());
                }
            }
            let slots = (f64::from(seconds)
                * crate::simulation::core::scheduler::SLOTS_PER_MINUTE as f64
                / self.time.seconds_per_minute())
            .ceil()
            .max(1.0) as u64;
            self.schedule_trip(
                self.time.absolute_slot() + slots,
                super::trips::TripEvent::ParkingRetry { agent },
            );
        }
        self.agents.parking_retries = requests;
    }
}

impl SimCore {
    /// Rebuilds physical route-buffer capacity from the saved deterministic preparation budget.
    /// O(agents + prepared nodes), load only. Allocator spare capacity cannot alter departures.
    pub(crate) fn prepare_loaded_parking_routes(&mut self) {
        self.logistics.prepare_parking_borders(&self.transit_network, &self.region_graph);
        crate::thread_pools::install(|| {
            self.agents
                .agents
                .current_path
                .par_iter_mut()
                .zip(self.agents.agents.parking_trip.par_iter())
                .for_each(|(path, trip)| {
                    trip.prepare_route_buffer(path);
                });
        });
    }
}

impl SimCore {
    /// Explicit capture/test command: admits one adult at an authored fixture home using
    /// household, membership and occupancy mutators. O(local candidate scan + one household).
    /// Ordinary gameplay never calls this constructor.
    pub(crate) fn debug_add_parking_commuter(
        &mut self,
        home: usize,
        target: usize,
    ) -> Option<usize> {
        use crate::simulation::economy::agents::HouseholdAgeComposition;
        use crate::simulation::economy::definitions::{
            load_runtime_economy_catalog, load_runtime_economy_tuning,
        };
        if home >= self.allocator.buildings.len()
            || target >= self.allocator.buildings.len()
            || self.allocator.buildings[home].occupancy >= self.allocator.household_capacity(home)
        {
            return None;
        }
        let door = self.allocator.entrances.get(home)?.door_pos;
        let catalog = load_runtime_economy_catalog().ok()?;
        let tuning = load_runtime_economy_tuning().ok()?;
        self.allocator.claim_vacancy(home);
        let household = self.households.admit_immigrant_household(
            &catalog,
            &tuning,
            home,
            1,
            HouseholdAgeComposition {
                adult_count: 1,
                ..Default::default()
            },
        );
        let agent = self.agents.spawn_housed_agent(home, door.x, door.y);
        self.agents.assign_household_id(agent, household);
        self.publish_parking_households();
        self.parking.seed_fixture_car(self.agents.parking_trip[agent].vehicle,
            self.allocator.building_handles.handle(home).to_bits());
        self.agents.request_parking_fixture_trip(agent, target);
        Some(agent)
    }
}

impl SimCore {
    /// Schedules explicit fixture commutes so different ordinary itinerary legs can be
    /// inspected at one clock. O(fixture agents × Q), command-only; no gameplay mode override.
    pub(crate) fn debug_schedule_parking_capture(&mut self, minute: u16, returning: bool) -> usize {
        use crate::simulation::core::scheduler::SLOTS_PER_MINUTE;
        use crate::simulation::economy::agents::tick::quote_parking_leg;
        use crate::simulation::economy::agents::{MODE_CAR, TRANSIT_IN_BUILDING};
        let until = (f64::from(minute.saturating_sub(self.time.minute_of_day))
            * self.time.seconds_per_minute()
            - self.time.time_elapsed)
            .max(0.0) as f32;
        let mut count = 0;
        for i in 0..self.agents.len() {
            if self.agents.transit[i] != TRANSIT_IN_BUILDING {
                continue;
            }
            let target = if returning {
                self.agents.home_building[i]
            } else {
                self.agents.planned_target_building[i]
            };
            let origin = self.agents.current_building[i];
            if origin == target || target >= self.allocator.buildings.len() {
                continue;
            }
            let trip = self.agents.parking_trip[i];
            let Some(source) = self.parking.vehicle_place(trip.vehicle) else {
                continue;
            };
            let origin_handle = self.allocator.building_handles.handle(origin).to_bits();
            let target_handle = self.allocator.building_handles.handle(target).to_bits();
            let Some(walk) = self.parking.walk_to_car_seconds(origin_handle, source) else {
                continue;
            };
            let Some(candidate) = self.parking.first_free_candidate(target_handle) else {
                continue;
            };
            let Some(a) = self.parking.place_entrance(
                source,
                &self.allocator,
                &self.transit_network,
                &self.region_graph,
            ) else {
                continue;
            };
            let Some(b) = self.parking.place_entrance(
                candidate.place,
                &self.allocator,
                &self.transit_network,
                &self.region_graph,
            ) else {
                continue;
            };
            let out = self.parking.maneuver_seconds(source);
            let ingress = self.parking.maneuver_seconds(candidate.place);
            let Some(drive) = quote_parking_leg(
                MODE_CAR,
                &a,
                &b,
                [out, ingress],
                &self.transit_network,
                &self.region_graph,
                &self.agents.pathfind_count,
                None,
            ) else {
                continue;
            };
            let age = match i % 6 {
                0 => walk + drive.seconds() + candidate.walk_s + 20.0,
                1 => walk * 0.5,
                2 => walk + out * 0.5,
                3 => walk + out + (drive.seconds() - out - ingress).max(0.0) * 0.5,
                4 => walk + drive.seconds() + 8.0,
                _ => walk + drive.seconds() + 4.0,
            };
            if !self.agents.request_parking_fixture_trip(i, target) {
                continue;
            }
            let slots = ((until - age).max(0.0) as f64 / self.time.seconds_per_minute()
                * SLOTS_PER_MINUTE as f64)
                .ceil() as u64;
            let agent = self.agents.agent_handle(i);
            self.schedule_trip(
                self.time.absolute_slot() + slots.max(1),
                super::trips::TripEvent::ParkingRetry { agent },
            );
            count += 1;
        }
        count
    }

    /// Advances only an explicit isolated capture fixture through the normal simulation.
    /// Work is proportional to requested simulated time; never used by ordinary UI flows.
    pub(crate) fn debug_advance_parking_capture(&mut self, minute: u16) {
        self.time.speed_multiplier = 32.0;
        while self.time.minute_of_day < minute {
            self.step_frame(1.0 / 60.0);
        }
        self.time.speed_multiplier = 0.0;
    }
}

impl SimCore {
    /// Resets the clock only for an empty, explicitly requested capture fixture.
    pub(crate) fn debug_reset_parking_capture_clock(&mut self) {
        if self.agents.is_empty() {
            self.time.minute_of_day = 0;
            self.time.time_elapsed = 0.0;
            self.align_events_to_clock();
        }
    }
}

impl SimCore {
    /// Publishes specified door catchments with the existing economy component index when
    /// routing is current. O(local searches), plus the shared topology's existing one-time
    /// O(nodes + edges) refresh after routing adoption; no new connectivity/spatial index.
    /// Returns the doors whose reachable places changed.
    pub(crate) fn publish_parking_catchments(&mut self, buildings: &[usize]) -> Vec<[f32; 2]> {
        let car_components = if self.transit_network.cch_current() {
            self.households
                .destination_topology
                .refresh(&self.transit_network, &self.region_graph);
            Some(self.households.destination_topology.car())
        } else {
            None
        };
        self.parking.rebuild_candidates(
            buildings,
            &self.allocator,
            &self.transit_network,
            &self.region_graph,
            car_components,
        )
    }

    /// Finishes only deferred local metadata at routing publication. O(1) without pending
    /// edits; otherwise O(local candidates log candidates), plus the shared topology refresh.
    pub(crate) fn complete_parking_economy_publication(&mut self) {
        if !self.parking.economy_refresh_pending() || !self.transit_network.cch_current() {
            return;
        }
        self.households
            .destination_topology
            .refresh(&self.transit_network, &self.region_graph);
        self.parking.refresh_economy_metadata(
            &self.allocator,
            &self.transit_network,
            &self.region_graph,
            self.households.destination_topology.car(),
        );
    }
}

impl SimCore {
    /// Queues edited/released home chunks, O(E log S). The existing logistics topology cache
    /// refreshes once per graph revision here (O(nodes + edges)), outside delivery events.
    pub(crate) fn schedule_stored_parking_arrivals(&mut self) {
        if self.parking.release_events.is_empty() { return; }
        self.logistics.prepare_parking_borders(&self.transit_network, &self.region_graph);
        let mut events = std::mem::take(&mut self.parking.release_events);
        events.sort_unstable();
        for chunk in events.drain(..) {
            self.schedule_trip(self.time.absolute_slot() + 1,
                super::trips::TripEvent::StoredCarArrival { chunk });
        }
        self.parking.release_events = events;
    }

    /// Considers at most eight waiting cars per event in stable handle order. Per event:
    /// O(8 × (9 log W + k + BQ) + log S), W local waiting cars, k cached home candidates,
    /// B connected borders, Q one exact route, S scheduled events. No city population scan.
    pub(super) fn run_stored_parking_arrivals(&mut self, chunk: (i32, i32), slot: u64) {
        for _ in 0..8 {
            let Some(car) = self.parking.next_stored_car(chunk) else {
                self.parking.finish_supply_event(chunk);
                return;
            };
            self.agents.start_stored_car_arrival(car, self.logistics.parking_borders(),
                &mut self.parking, &self.allocator, &self.transit_network, &self.region_graph);
        }
        self.schedule_trip(slot + 1, super::trips::TripEvent::StoredCarArrival { chunk });
    }
}

impl SimCore {
    /// Isolated capture command: one approved household enters through the ordinary paced
    /// departure event at the next slot. O(shared topology refresh + one household carrier).
    pub(crate) fn debug_add_parking_move_in(&mut self, home: usize) -> Option<usize> {
        use crate::simulation::economy::agents::HouseholdAgeComposition;
        if home >= self.allocator.buildings.len()
            || self.allocator.buildings[home].occupancy >= self.allocator.household_capacity(home) {
            return None;
        }
        self.logistics.prepare_parking_borders(&self.transit_network, &self.region_graph);
        let border = *self.logistics.parking_borders().first()?;
        let pos = self.region_graph.node(border).pos;
        self.allocator.claim_vacancy(home);
        let i = self.agents.spawn_household_arrival_carrier(home, 1,
            HouseholdAgeComposition { adult_count: 1, ..Default::default() }, border, pos.x, pos.z);
        let carrier = self.agents.agent_handle(i);
        self.schedule_trip(self.time.absolute_slot() + 1,
            super::trips::TripEvent::ArrivalDeparture { carrier });
        Some(i)
    }
}
