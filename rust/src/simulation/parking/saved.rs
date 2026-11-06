// SPDX-License-Identifier: GPL-2.0-only

//! Explicit saved rows, handle generations, reciprocal validation and index reconstruction.

use super::*;
use crate::simulation::core::handles::SavedHandleTable;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct SavedPlace {
    source: PlaceSource,
    active: bool,
    retired_for: u64,
    road_point: [f32; 2],
    access_side: i8,
    bay: SavedBay,
    occupant: u64,
    reservation: u64,
}
#[derive(Serialize, Deserialize)]
struct SavedBay {
    identity: [u64; 2],
    position: [f32; 3],
    path: [[f32; 2]; 4],
    heading: f32,
    width: f32,
    length: f32,
}
#[derive(Serialize, Deserialize)]
struct SavedVehicle {
    home: [f32; 2],
    home_building: u64,
    owner: u64,
    borrower: u64,
    moving: bool,
    retired_places: [u64; 3],
    place: u64,
    reserved: u64,
    model: u8,
    colour: u8,
}
#[derive(Serialize, Deserialize)]
struct SavedTable {
    slots: Vec<(u32, Option<u32>)>,
    free: Vec<u32>,
}
#[derive(Serialize, Deserialize)]
struct SavedParking {
    pending_households: Vec<u64>,
    release_events: Vec<(i32, i32)>,
    arrival_cursors: Vec<((i32, i32), Option<u64>)>,
    mobility_seed: u64,
    owners: Vec<(u64, u64)>,
    places: Vec<SavedPlace>,
    vehicles: Vec<SavedVehicle>,
    place_handles: SavedTable,
    vehicle_handles: SavedTable,
}

impl SavedTable {
    fn capture<K>(table: &HandleTable<K>) -> Self {
        let SavedHandleTable { slots, free } = table.save();
        Self { slots, free }
    }
    fn restore<K>(self, len: usize) -> Result<HandleTable<K>, String> {
        HandleTable::restore(
            &SavedHandleTable {
                slots: self.slots,
                free: self.free,
            },
            len,
        )
    }
}

impl ParkingSystem {
    /// O(places + vehicles), save boundary only. Source remapping belongs to the save graph map.
    #[cfg(test)]
    pub(crate) fn save_json(
        &self,
        map: impl Fn(PlaceSource) -> PlaceSource + Sync,
    ) -> Result<String, String> {
        self.save_json_with_households(map, &[])
    }

    /// Also persists ownership lifecycle events not yet consumed at the next frame boundary.
    /// O(places + vehicles + pending household events), save boundary only.
    pub(crate) fn save_json_with_households(
        &self,
        map: impl Fn(PlaceSource) -> PlaceSource + Sync,
        pending: &[crate::simulation::core::handles::Handle<
            crate::simulation::economy::households::Household,
        >],
    ) -> Result<String, String> {
        let places = crate::thread_pools::install(|| {
            (0..self.places.len())
                .into_par_iter()
                .map(|p| SavedPlace {
                    source: map(self.places.source[p]),
                    active: self.places.active[p],
                    retired_for: self.places.retired_for[p].to_bits(),
                    road_point: self.places.road_point[p],
                    access_side: self.places.access[p].side,
                    bay: SavedBay {
                        identity: self.places.identity[p],
                        position: self.places.position[p],
                        path: self.places.path[p],
                        heading: self.places.heading[p],
                        width: self.places.width[p],
                        length: self.places.length[p],
                    },
                    occupant: self.places.occupant[p].to_bits(),
                    reservation: self.places.reservation[p].to_bits(),
                })
                .collect()
        });
        let vehicles = crate::thread_pools::install(|| {
            (0..self.vehicles.len())
                .into_par_iter()
                .map(|v| SavedVehicle {
                    home: self.vehicles.home[v],
                    home_building: self.vehicles.home_building[v],
                    owner: self.vehicles.owner[v],
                    borrower: self.vehicles.borrower[v].to_bits(),
                    moving: self.vehicles.moving[v],
                    retired_places: self.vehicles.retired_places[v].map(Handle::to_bits),
                    place: self.vehicles.place[v].to_bits(),
                    reserved: self.vehicles.reserved[v].to_bits(),
                    model: self.vehicles.model[v],
                    colour: self.vehicles.colour[v],
                })
                .collect()
        });
        // JSON emission and handle-table capture retain their single-writer saved order.
        let mut owners: Vec<_> = self
            .owners
            .iter()
            .map(|(&h, &v)| (h, v.to_bits()))
            .collect();
        owners.sort_unstable();
        let mut arrival_cursors: Vec<_> = self
            .chunks
            .iter()
            .filter(|(_, chunk)| chunk.release_pending)
            .map(|(&key, chunk)| (key, chunk.wake_after.map(Handle::to_bits)))
            .collect();
        arrival_cursors.sort_unstable();
        serde_json::to_string(&SavedParking {
            pending_households: pending.iter().map(|h| h.to_bits()).collect(),
            release_events: self.release_events.clone(),
            arrival_cursors,
            mobility_seed: self.mobility_seed,
            owners,
            places,
            vehicles,
            place_handles: SavedTable::capture(&self.place_handles),
            vehicle_handles: SavedTable::capture(&self.vehicle_handles),
        })
        .map_err(|e| e.to_string())
    }

    /// O(places + vehicles), rejects corrupt reciprocal custody before resuming. Rebuilds
    /// source/chunk indices and dirty buffers, without reallocating on later occupy/release.
    pub(crate) fn load_json(json: &str) -> Result<Self, String> {
        let saved: SavedParking = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut system = Self {
            pending_households: saved.pending_households,
            mobility_seed: saved.mobility_seed,
            owners: saved
                .owners
                .into_iter()
                .map(|(h, v)| (h, Handle::from_bits(v)))
                .collect(),
            place_handles: saved.place_handles.restore(saved.places.len())?,
            vehicle_handles: saved.vehicle_handles.restore(saved.vehicles.len())?,
            ..Self::default()
        };
        for place in saved.places {
            if let PlaceSource::Building { building, .. } = place.source {
                let sources = system.building_sources.entry(building).or_default();
                if !sources.contains(&place.source) {
                    sources.push(place.source);
                }
            }
            let p = system.places.len();
            let handle = system.place_handles.handle(p);
            let bay = place.bay;
            if !bay.path.into_iter().flatten().all(f32::is_finite)
                || !bay.position.into_iter().all(f32::is_finite)
                || !bay.heading.is_finite()
                || !bay.width.is_finite()
                || !bay.length.is_finite()
                || bay.width <= 0.0
                || bay.length <= 0.0
            {
                return Err("invalid parking bay geometry".into());
            }
            let key = chunk_at([bay.position[0], bay.position[2]]);
            let chunk = system.chunks.entry(key).or_default();
            let list = system.sources.entry(place.source).or_default();
            system.places.push(Place {
                source: place.source,
                active: place.active,
                retired_for: Handle::from_bits(place.retired_for),
                road_point: place.road_point,
                identity: bay.identity,
                position: bay.position,
                path: bay.path,
                access: crate::simulation::buildings::allocator::BuildingEntrance {
                    side: place.access_side,
                    ..Default::default()
                },
                heading: bay.heading,
                width: bay.width,
                length: bay.length,
                occupant: Handle::from_bits(place.occupant),
                reservation: Handle::from_bits(place.reservation),
                chunk_slot: chunk.places.len(),
                source_slot: list.len(),
                ground: [0.0; 12],
                ground_model: super::data::GROUND_UNSOLVED,
            });
            if place.active {
                system.usable_places += 1;
            } else {
                system.retired_dirty.push(handle);
            }
            chunk.places.push(handle);
            list.push(handle);
        }
        for car in saved.vehicles {
            if car.model >= 5 || car.colour >= 5 || !car.home.into_iter().all(f32::is_finite) {
                return Err("invalid parked vehicle".into());
            }
            system.vehicles.push(Vehicle {
                home: car.home,
                home_building: car.home_building,
                owner: car.owner,
                borrower: Handle::from_bits(car.borrower),
                moving: car.moving,
                retired_places: car.retired_places.map(Handle::from_bits),
                place: Handle::from_bits(car.place),
                reserved: Handle::from_bits(car.reserved),
                model: car.model,
                colour: car.colour,
            });
        }
        for p in 0..system.places.len() {
            let handle = system.place_handles.handle(p);
            for (car, occupied) in [
                (system.places.occupant[p], true),
                (system.places.reservation[p], false),
            ] {
                if car.is_none() {
                    continue;
                }
                let v = system
                    .vehicle_handles
                    .resolve(car)
                    .ok_or("stale parking vehicle")?;
                if (if occupied {
                    system.vehicles.place[v]
                } else {
                    system.vehicles.reserved[v]
                }) != handle
                {
                    return Err("nonreciprocal parking place".into());
                }
            }
            if !system.places.occupant[p].is_none() && !system.places.reservation[p].is_none() {
                return Err("occupied place also reserved".into());
            }
        }
        for v in 0..system.vehicles.len() {
            let car = system.vehicle_handles.handle(v);
            for (handle, occupied) in [
                (system.vehicles.place[v], true),
                (system.vehicles.reserved[v], false),
            ] {
                if handle.is_none() {
                    continue;
                }
                let p = system
                    .place_handles
                    .resolve(handle)
                    .ok_or("stale vehicle place")?;
                if (if occupied {
                    system.places.occupant[p]
                } else {
                    system.places.reservation[p]
                }) != car
                {
                    return Err("nonreciprocal parked vehicle".into());
                }
            }
            if !system.vehicles.moving[v]
                && system.vehicles.place[v].is_none()
                && system.vehicles.reserved[v].is_none()
            {
                system
                    .stored
                    .entry(chunk_at(system.vehicles.home[v]))
                    .or_default()
                    .insert(car);
            }
        }
        for (key, after) in saved.arrival_cursors {
            let chunk = system.chunks.entry(key).or_default();
            chunk.release_pending = true;
            chunk.wake_after = after.map(Handle::from_bits);
        }
        system.release_events = saved.release_events;
        for key in &system.release_events {
            system.chunks.entry(*key).or_default().release_pending = true;
        }
        system.release_events.reserve(
            system
                .chunks
                .len()
                .saturating_sub(system.release_events.len()),
        );
        system.dirty.reserve(system.chunks.len());
        let mut keys: Vec<_> = system.chunks.keys().copied().collect();
        keys.sort_unstable();
        for key in keys {
            system.mark_dirty(key);
        }

        Ok(system)
    }
}

impl ParkingSystem {
    /// Load validation is O(places), never a tick path; dangling geometry owners reject the save.
    pub(crate) fn validate_sources(
        &self,
        allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) -> Result<(), String> {
        for (p, source) in self.places.source.iter().enumerate() {
            if !self.places.active[p] {
                continue;
            }
            let valid = match *source {
                PlaceSource::Building { building, .. } => allocator
                    .building_handles
                    .resolve(Handle::from_bits(building))
                    .is_some(),
                PlaceSource::Kerb { edge, right } => graph
                    .get_edge(edge as usize)
                    .filter(|e| !e.deleted)
                    .is_some_and(|e| {
                        if right {
                            e.parking.right
                        } else {
                            e.parking.left
                        }
                    }),
            };
            if !valid {
                return Err(format!("parking place has a missing source: {source:?}"));
            }
        }
        Ok(())
    }
}

impl ParkingSystem {
    /// Validates persistent ownership, exclusive borrowers and live itineraries in
    /// O(households + vehicles + agents + pending lifecycle events). Save/load only.
    /// A queued household deletion is preserved until its ordinary publication event runs.
    pub(crate) fn validate_custody(
        &self,
        agents: &crate::simulation::economy::agents::AgentSystem,
        households: &crate::simulation::economy::households::HouseholdSystem,
    ) -> Result<(), String> {
        use crate::simulation::economy::agents::AGE_CHILD;
        let pending: std::collections::HashSet<_> = households
            .parking_changes
            .iter()
            .map(|h| h.to_bits())
            .collect();
        for (&owner, &car) in &self.owners {
            if households
                .handles
                .resolve(Handle::from_bits(owner))
                .is_none()
                && !pending.contains(&owner)
            {
                return Err("parking owner is missing".into());
            }
            if !car.is_none() {
                let v = self
                    .vehicle_handles
                    .resolve(car)
                    .ok_or("household car is stale")?;
                if self.vehicles.owner[v] != owner {
                    return Err("household car ownership is not reciprocal".into());
                }
            }
        }
        for v in 0..self.vehicles.len() {
            let car = self.vehicle_handles.handle(v);
            let owner = self.vehicles.owner[v];
            if owner != u64::MAX && self.owners.get(&owner) != Some(&car) {
                return Err("vehicle ownership is not reciprocal".into());
            }
            let borrower = self.vehicles.borrower[v];
            if !borrower.is_none() && !pending.contains(&owner) {
                let i = agents
                    .resolve_agent(borrower)
                    .ok_or("parking borrower is missing")?;
                if (agents.age_group[i] == AGE_CHILD && !agents.parking_trip[i].delivery_only)
                    || agents.parking_trip[i].vehicle != car
                    || (households.handles.handle(agents.household_id[i]).to_bits() != owner
                        && !agents.parking_trip[i].delivery_only
                        && !(owner == u64::MAX
                            && agents.pending_household_size[i] > 0
                            && agents.parking_trip[i].arrival_ownership))
                {
                    return Err("invalid parking custody".into());
                }
            }
            for &place in &self.vehicles.retired_places[v] {
                if place.is_none() {
                    continue;
                }
                let p = self
                    .place_handles
                    .resolve(place)
                    .ok_or("retained parking endpoint is stale")?;
                if self.places.retired_for[p] != car {
                    return Err("retained parking endpoint is not reciprocal".into());
                }
            }
        }
        for i in 0..agents.len() {
            let trip = agents.parking_trip[i];
            if trip.phase == ParkingTripPhase::None {
                continue;
            }
            let v = self
                .vehicle_handles
                .resolve(trip.vehicle)
                .ok_or("trip car is stale")?;
            if self.vehicles.borrower[v] != agents.handles.handle(i) {
                return Err("trip has no exclusive custody".into());
            }
        }
        Ok(())
    }
}
