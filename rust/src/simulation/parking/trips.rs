// SPDX-License-Identifier: GPL-2.0-only

//! Resident ownership and exclusive custody, committed in stable transition order.

use super::*;
use crate::simulation::economy::agents::AgentHandle;
use serde::{Deserialize, Serialize};

/// Current private-car itinerary leg. Only the last pedestrian leg enters a building.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ParkingTripPhase {
    /// No car-backed trip; the vehicle field still records household access.
    #[default]
    None,
    /// Pedestrian travel from the origin door to the car's actual bay.
    WalkToCar,
    /// The validated bay–aisle–driveway maneuver followed by ordinary lane entry.
    PullOut,
    /// Ordinary network driving between driveway lane endpoints.
    Drive,
    /// Ordinary detach followed by the validated driveway–aisle–bay maneuver.
    PullIn,
    /// Pedestrian travel from the occupied bay to the destination door.
    WalkToDoor,
}

/// Persistent per-agent itinerary, stored as one column in the agent SoA.
/// O(1) leg state, with no separate route vectors; ordinary movement remains O(1) per agent.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct ParkingTrip {
    #[serde(with = "saved_handle")]
    pub(crate) vehicle: Handle<Vehicle>,
    /// Saved node budget prepared by an explicit event; independent of allocator spare capacity.
    pub(crate) route_capacity: usize,
    pub(crate) retry_pending: bool,
    pub(crate) arrival_ownership: bool,
    pub(crate) delivery_only: bool,
    pub(crate) replan_needed: bool,
    #[serde(with = "saved_handle")]
    pub(crate) walk_origin: Handle<Place>,
    #[serde(with = "saved_handle")]
    pub(crate) source: Handle<Place>,
    #[serde(with = "saved_handle")]
    pub(crate) destination: Handle<Place>,
    pub(crate) target: u64,
    pub(crate) phase: ParkingTripPhase,
    pub(crate) ready: bool,
}

impl ParkingTrip {
    /// Reconstructs derived route storage after a saved/cloned record is restored.
    /// O(prepared nodes) allocation at load/undo only; no work when capacity is sufficient.
    pub(crate) fn prepare_route_buffer(&self, route: &mut Vec<u32>) {
        if route.capacity() < self.route_capacity {
            route.reserve(self.route_capacity - route.len());
        }
    }

    /// Keeps the saved preparation budget within a replacement route's guaranteed storage.
    /// O(1), independent of allocator spare capacity, so edits and load make identical retries.
    pub(crate) fn route_replaced(&mut self, nodes: usize) {
        self.route_capacity = self.route_capacity.min(nodes);
    }
}

impl ParkingSystem {
    /// Assigns ownership once from the saved seed and stable household identity. Density is
    /// the zoning registry's classification of `Building::zone_profile_runtime_id`.
    /// O(1) decision; insertion is a household lifecycle boundary and may allocate.
    pub(crate) fn assign_household(
        &mut self,
        owner: u64,
        density: crate::simulation::zoning::profiles::ZoneDensity,
        home: [f32; 2],
    ) -> Handle<Vehicle> {
        if let Some(&car) = self.owners.get(&owner) {
            return car;
        }
        let owns = owns_car(self.mobility_seed, owner, density);
        let car = if owns {
            self.create_car(owner, owner, home)
        } else {
            Handle::NONE
        };
        self.owners.insert(owner, car);
        car
    }

    fn create_car(&mut self, identity: u64, owner: u64, home: [f32; 2]) -> Handle<Vehicle> {
        let v = self.vehicles.len();
        let hash = crate::simulation::core::stable_hash64(self.mobility_seed ^ identity);
        self.vehicles.push(Vehicle {
            home,
            home_building: u64::MAX,
            owner,
            borrower: Handle::NONE,
            moving: false,
            retired_places: [Handle::NONE; 3],
            place: Handle::NONE,
            reserved: Handle::NONE,
            model: (hash % 4) as u8,
            colour: ((hash >> 8) % 5) as u8,
        });
        self.vehicle_handles.register_appended(self.vehicles.len());
        self.vehicle_handles.handle(v)
    }

    /// Decides an arriving household's car once from its stable carrier identity and the saved
    /// seed. O(1), lifecycle allocation only; adoption preserves the decision without rerolling.
    pub(crate) fn arrival_car(
        &mut self,
        identity: u64,
        density: crate::simulation::zoning::profiles::ZoneDensity,
        home: [f32; 2],
    ) -> Handle<Vehicle> {
        if owns_car(self.mobility_seed, identity, density) {
            self.create_car(identity, u64::MAX, home)
        } else {
            Handle::NONE
        }
    }

    /// Transfers an arrival's original car/zero-car decision into its materialized household.
    /// O(1), lifecycle allocation only. The car retains pose, reservation, model and colour.
    pub(crate) fn adopt_arrival_car(&mut self, owner: u64, car: Handle<Vehicle>) {
        self.owners.insert(owner, car);
        if let Some(v) = self.vehicle_handles.resolve(car) {
            self.vehicles.owner[v] = owner;
        }
    }

    /// Records off-map custody without claiming supply. O(log W), event boundary.
    pub(crate) fn store_arrival_car(&mut self, car: Handle<Vehicle>, home: u64) {
        let Some(v) = self.vehicle_handles.resolve(car) else {
            return;
        };
        self.vehicles.home_building[v] = home;
        let list = self
            .stored
            .entry(chunk_at(self.vehicles.home[v]))
            .or_default();
        list.insert(car);
    }

    /// Reserves a border-origin vehicle's destination and exclusive carrier in O(log W).
    pub(crate) fn reserve_arrival(
        &mut self,
        car: Handle<Vehicle>,
        borrower: AgentHandle,
        place: Handle<Place>,
    ) -> bool {
        let Some(v) = self.vehicle_handles.resolve(car) else {
            return false;
        };
        if !self.vehicles.place[v].is_none() || self.vehicles.moving[v] || !self.reserve(car, place)
        {
            return false;
        }
        if let Some(waiting) = self.stored.get_mut(&chunk_at(self.vehicles.home[v])) {
            waiting.remove(&car);
        }
        self.vehicles.borrower[v] = borrower;
        self.set_moving(car, true);
        true
    }

    /// O(1), allocation-free lookup of the persistent household car.
    pub(crate) fn household_car(&self, owner: u64) -> Handle<Vehicle> {
        self.owners.get(&owner).copied().unwrap_or(Handle::NONE)
    }

    /// O(1), allocation-free eligibility; custody belongs to one stable agent throughout an outing.
    pub(crate) fn available_to(&self, car: Handle<Vehicle>, borrower: AgentHandle) -> bool {
        self.vehicle_handles.resolve(car).is_some_and(|v| {
            !self.vehicles.place[v].is_none()
                && !self.vehicles.moving[v]
                && self.vehicles.reserved[v].is_none()
                && (self.vehicles.borrower[v].is_none() || self.vehicles.borrower[v] == borrower)
        })
    }

    /// Atomically borrows and reserves at actual departure. O(1), allocation-free.
    pub(crate) fn reserve_trip(
        &mut self,
        car: Handle<Vehicle>,
        borrower: AgentHandle,
        destination: Handle<Place>,
    ) -> bool {
        if !self.available_to(car, borrower) || !self.reserve(car, destination) {
            return false;
        }
        let v = self.vehicle_handles.resolve(car).expect("reserved vehicle");
        self.vehicles.borrower[v] = borrower;
        true
    }

    /// O(1), marks the same publication's parked/moving ownership switch. Occupancy remains
    /// until the pull-out clears the bay; dirty chunks and moving snapshots use this state.
    pub(crate) fn set_moving(&mut self, car: Handle<Vehicle>, moving: bool) {
        let Some(v) = self.vehicle_handles.resolve(car) else {
            return;
        };
        if self.vehicles.moving[v] == moving {
            return;
        }
        self.vehicles.moving[v] = moving;
        if let Some(p) = self.place_handles.resolve(self.vehicles.place[v]) {
            let pos = self.places.position[p];
            self.mark_dirty(chunk_at([pos[0], pos[2]]));
        }
    }

    /// Household lifecycle deletion; O(local off-map waiting cars) plus local wake events.
    pub(crate) fn remove_household_car(&mut self, owner: u64) {
        let Some(car) = self.owners.remove(&owner) else {
            return;
        };
        let Some(v) = self.vehicle_handles.resolve(car) else {
            return;
        };
        let home_chunk = chunk_at(self.vehicles.home[v]);
        if let Some(waiting) = self.stored.get_mut(&home_chunk) {
            waiting.remove(&car);
            if waiting.is_empty() {
                self.stored.remove(&home_chunk);
            }
        }
        self.clear_retired_places(car);
        self.cancel_reservation(car);
        let released = self.release(car);
        self.vehicle_handles.swap_remove(v, self.vehicles.len());
        self.vehicles.swap_remove(v);
        if let Some(point) = released {
            self.rehouse_near(point);
        }
    }
}

fn owns_car(
    seed: u64,
    owner: u64,
    density: crate::simulation::zoning::profiles::ZoneDensity,
) -> bool {
    use crate::simulation::zoning::profiles::ZoneDensity;
    let bucket = crate::simulation::core::stable_hash64(seed ^ owner) >> 62;
    match density {
        ZoneDensity::Low => true,
        ZoneDensity::Medium => bucket < 3,
        ZoneDensity::High => bucket < 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::zoning::profiles::ZoneDensity;

    #[test]
    fn ownership_ratios_and_stable_identity() {
        for (density, expected) in [
            (ZoneDensity::Low, 1.0),
            (ZoneDensity::Medium, 0.75),
            (ZoneDensity::High, 0.5),
        ] {
            let count = (0..100_000)
                .filter(|&id| owns_car(317, id, density))
                .count();
            assert!((count as f64 / 100_000.0 - expected).abs() < 0.005);
            assert_eq!(owns_car(317, 52, density), owns_car(317, 52, density));
        }
    }
}

mod saved_handle {
    use super::*;
    pub(super) fn serialize<S: serde::Serializer, K>(
        handle: &Handle<K>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(handle.to_bits())
    }
    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>, K>(
        deserializer: D,
    ) -> Result<Handle<K>, D::Error> {
        u64::deserialize(deserializer).map(Handle::from_bits)
    }
}

impl ParkingSystem {
    /// Registers a new car off-map and queues supply, O(log W), lifecycle boundary only.
    /// Cars enter the city through scheduled driven arrivals, including after save/load.
    pub(crate) fn park_new_car(&mut self, car: Handle<Vehicle>, home: u64) {
        let Some(v) = self.vehicle_handles.resolve(car) else {
            return;
        };
        self.vehicles.home_building[v] = home;
        if !self.vehicles.place[v].is_none() || self.vehicles.moving[v] {
            return;
        }
        self.store_arrival_car(car, home);
        self.rehouse_near(self.vehicles.home[v]);
    }

    /// Explicit fixture genesis only: places a car without simulating its historical arrival.
    /// O(k + log W); ordinary household publication never calls this helper.
    pub(crate) fn seed_fixture_car(&mut self, car: Handle<Vehicle>, home: u64) {
        let Some(candidate) = self.first_free_candidate(home) else {
            return;
        };
        if let Some(v) = self.vehicle_handles.resolve(car)
            && let Some(waiting) = self.stored.get_mut(&chunk_at(self.vehicles.home[v]))
        {
            waiting.remove(&car);
        }
        self.occupy(car, candidate.place);
    }
}

impl ParkingSystem {
    /// O(1) curb lookup, shared by exact pedestrian detach and the validated lot maneuver.
    pub(crate) fn place_curb(&self, place: Handle<Place>) -> Option<godot::prelude::Vector2> {
        let p = self.place_handles.resolve(place)?;
        let [x, z] = self.places.path[p][0];
        Some(godot::prelude::Vector2::new(x, z))
    }

    /// O(1) phase endpoint selection plus the host's existing frontage projection.
    pub(crate) fn trip_entrance(
        &self,
        trip: ParkingTrip,
        origin: bool,
        allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
        network: &crate::simulation::network::TransitNetwork,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) -> Option<crate::simulation::buildings::allocator::BuildingEntrance> {
        use ParkingTripPhase::*;
        let place = match (origin, trip.phase) {
            (true, WalkToCar) if !trip.walk_origin.is_none() => trip.walk_origin,
            (true, PullOut) | (false, WalkToCar) => trip.source,
            (true, WalkToDoor) | (false, PullOut | Drive | PullIn) => trip.destination,
            _ => return Option::None,
        };
        self.place_entrance(place, allocator, network, graph)
    }

    /// O(1) fixed swept-path pose product, with the exact adjacent lane point at the driveway.
    pub(crate) fn maneuver_path(
        &self,
        place: Handle<Place>,
        lane: usize,
        distance: f32,
        network: &crate::simulation::network::TransitNetwork,
        ingress: bool,
    ) -> Option<crate::simulation::economy::agents::tick::ParkingAccessPath> {
        use godot::prelude::Vector2;
        let p = self.place_handles.resolve(place)?;
        let lane = network.lane_system.lanes.get(lane)?;
        let point = crate::simulation::buildings::allocator::BuildingAllocator::sample_pos_on_lane(
            lane, distance,
        );
        let swept = self.places.path[p].map(|[x, z]| Vector2::new(x, z));
        let mut points = [point, swept[0], swept[1], swept[2], swept[3]];
        if !ingress {
            points.reverse();
        }
        Some(crate::simulation::economy::agents::tick::ParkingAccessPath { points, count: 5 })
    }

    /// O(1) inspection of the independent car's current place.
    pub(crate) fn vehicle_place(&self, car: Handle<Vehicle>) -> Option<Handle<Place>> {
        self.vehicle_handles
            .resolve(car)
            .map(|v| self.vehicles.place[v])
            .filter(|p| !p.is_none())
    }

    /// O(1), releases custody after the borrower gets home; the car keeps its actual bay.
    pub(crate) fn finish_outing(&mut self, car: Handle<Vehicle>, borrower: AgentHandle) {
        if let Some(v) = self.vehicle_handles.resolve(car)
            && self.vehicles.borrower[v] == borrower
        {
            self.vehicles.borrower[v] = Handle::NONE;
        }
    }

    /// O(1) reservation proof; a removed destination invalidates the saved itinerary.
    pub(crate) fn trip_reserved(&self, car: Handle<Vehicle>, place: Handle<Place>) -> bool {
        self.vehicle_handles
            .resolve(car)
            .is_some_and(|v| self.vehicles.reserved[v] == place && !place.is_none())
    }
}

#[cfg(test)]
mod custody_tests {
    use super::*;

    #[test]
    fn one_borrower_reservation_conservation_and_handoff() {
        let mut parking = ParkingSystem::default();
        let bay = |x| Bay {
            identity: [0, 0],
            position: [x, 0.0, 0.0],
            path: [[x, 0.0]; 4],
            heading: 0.0,
            width: 2.5,
            length: 5.0,
        };
        let source = parking.add_place(
            PlaceSource::Building {
                building: 0,
                anchor: 0,
            },
            bay(0.0),
        );
        parking.fill_test_fleet();
        let car = parking.vehicle_handles.handle(0);
        let destination = parking.add_place(
            PlaceSource::Building {
                building: 0,
                anchor: 1,
            },
            bay(20.0),
        );
        let a = AgentHandle::from_parts(1, 0);
        let b = AgentHandle::from_parts(2, 0);
        assert!(parking.reserve_trip(car, a, destination));
        assert!(!parking.reserve_trip(car, b, destination));
        assert_eq!(parking.vehicle_place(car), Some(source));
        parking.build_dirty_chunks();
        let parked_count = |p: &ParkingSystem| {
            p.publications(true)
                .iter()
                .map(|(_, c)| c.identities.values().map(Vec::len).sum::<usize>())
                .sum::<usize>()
        };
        assert_eq!(parked_count(&parking), 1);
        parking.set_moving(car, true);
        parking.build_dirty_chunks();
        assert_eq!(
            parked_count(&parking) + usize::from(parking.vehicles.moving[0]),
            1
        );
        parking.release(car);
        assert!(parking.trip_reserved(car, destination));
        assert!(parking.occupy(car, destination));
        parking.set_moving(car, false);
        parking.build_dirty_chunks();
        assert_eq!(
            parked_count(&parking) + usize::from(parking.vehicles.moving[0]),
            1
        );
        assert!(!parking.available_to(car, b));
        let restored = ParkingSystem::load_json(&parking.save_json(|s| s).unwrap()).unwrap();
        assert_eq!(restored.vehicles.borrower[0], a);
        parking.finish_outing(car, a);
        assert!(parking.available_to(car, b));
    }
}

impl ParkingSystem {
    /// O(1) endpoint selection plus frontage projection; a deleted parking endpoint returns
    /// an invalid entrance so ordinary replan failure cannot fall back to a car-to-door leg.
    pub(crate) fn replan_entrance(
        &self,
        trip: ParkingTrip,
        allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
        network: &crate::simulation::network::TransitNetwork,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) -> Option<crate::simulation::buildings::allocator::BuildingEntrance> {
        match trip.phase {
            ParkingTripPhase::WalkToCar
            | ParkingTripPhase::PullOut
            | ParkingTripPhase::Drive
            | ParkingTripPhase::PullIn => Some(
                self.trip_entrance(trip, false, allocator, network, graph)
                    .unwrap_or_default(),
            ),
            _ => None,
        }
    }
}

impl ParkingSystem {
    // Resident recovery uses the published walking catchment. The explicit foundation fleet
    // has no home building and retains its existing test-only Euclidean recovery policy.
    pub(super) fn home_place(&self, v: usize) -> Option<Handle<Place>> {
        let home = self.vehicles.home_building[v];
        if home == u64::MAX {
            self.nearest(self.vehicles.home[v])
        } else {
            self.first_free_candidate(home).map(|c| c.place)
        }
    }
}

impl ParkingSystem {
    /// O(1) stable borrower lookup; absent when the household car is free or unavailable.
    pub(crate) fn borrowed_by(&self, car: Handle<Vehicle>) -> Option<AgentHandle> {
        self.vehicle_handles
            .resolve(car)
            .map(|v| self.vehicles.borrower[v])
            .filter(|h| !h.is_none())
    }
}
