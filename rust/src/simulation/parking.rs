// SPDX-License-Identifier: GPL-2.0-only

//! Persistent physical parking supply. Sleeping vehicles have no simulation tick pass.
//! Mutations run in deterministic command/transition order, independent geometry uses Rayon.

mod candidates;
mod data;
mod edits;
mod geometry;
pub(crate) mod kerb;
pub(crate) mod validation;
pub(crate) mod lots;
#[cfg(test)]
mod query;
mod render;
mod retired;
pub(crate) mod stats;
mod trips;
pub use trips::ParkingTrip;
pub(crate) use trips::ParkingTripPhase;
pub(crate) mod saved;
#[cfg(test)]
mod tests;

use crate::simulation::core::handles::{Handle, HandleTable};
pub(crate) use data::{Bay, ParkingCost, Place, PlaceSource, Vehicle};
use data::{Chunk, chunk_at};
pub(crate) use render::ParkedPublication;
use std::collections::HashMap;
use std::sync::Arc;

/// SoA parking stores and existing 512 m world-chunk convention. No per-tick iteration.
#[derive(Default, Debug)]
pub(crate) struct ParkingSystem {
    places: data::PlaceVec,
    vehicles: data::VehicleVec,
    place_handles: HandleTable<Place>,
    vehicle_handles: HandleTable<Vehicle>,
    chunks: HashMap<(i32, i32), Chunk>,
    sources: HashMap<PlaceSource, Vec<Handle<Place>>>,
    // At most one dirty entry per prepared chunk. Capacity grows only with supply edits/load.
    dirty: Vec<(i32, i32)>,
    pub(crate) release_events: Vec<(i32, i32)>,
    retired_dirty: Vec<Handle<Place>>,
    usable_places: usize,
    // One update per chunk, in chunk order. A list, because a map would allocate each time the
    // renderer's answer empties it.
    pending: Vec<render::ParkedUpdate>,
    published: std::collections::BTreeMap<(i32, i32), Arc<render::ParkedChunk>>,
    // The renderer's half of the publication; a replaced system starts a new one.
    publication: ParkedPublication,
    pending_changed: bool,
    acknowledged: u64,
    // The update list before the current one, refilled once no snapshot holds it.
    spare_updates: Arc<Vec<render::ParkedUpdate>>,
    // Rebuild slots, reused every frame; `dirty` says how many are in use.
    jobs: Vec<render::ChunkJob>,
    // Chunk buffers and update lists created because no spare was free.
    #[cfg(test)]
    buffer_creations: usize,
    // Only off-map cars are indexed here. Parking release never scans sleeping vehicles.
    pub(crate) diagnostics: std::collections::BTreeMap<u64, Vec<String>>,
    building_sources: HashMap<u64, Vec<PlaceSource>>,
    // Derived at every reconcile, rebuilt by the load-time reconcile of every edge; not saved.
    kerb_blockers: HashMap<u64, Vec<kerb::KerbBlocker>>,
    edge_blockers: HashMap<u32, Vec<u64>>,
    stored: HashMap<(i32, i32), std::collections::BTreeSet<Handle<Vehicle>>>,
    owners: HashMap<u64, Handle<Vehicle>>,
    mobility_seed: u64,
    pub(crate) pending_households: Vec<u64>,
    candidates: HashMap<u64, candidates::DestinationCandidates>,
    candidate_revision: u64,
    pending_economy: Vec<u64>,
    // Bays changed since the last publication; derived, never saved.
    place_changes: Vec<candidates::PlaceChange>,
    // Road edges a commit left to the frame after it, which reconciles them before anything
    // reads supply or a list; derived, never saved.
    pending_frontages: std::collections::BTreeSet<usize>,
    // Inspection-only departure outcomes since load; never saved or read by decisions.
    pub(crate) departures: stats::DepartureStats,
    // Cars whose custody changed since the last agent tick, and the borrowers whose reservation
    // ended: the events `has_car` and the trip recovery follow instead of scanning agents.
    // Derived, never saved; reused between ticks.
    pub(crate) custody_changes: Vec<Handle<Vehicle>>,
    pub(crate) lost_reservations: Vec<crate::simulation::economy::agents::AgentHandle>,
}

impl ParkingSystem {
    /// Counts active supply belonging to one inspected building in O(its anchors).
    pub(crate) fn building_place_count(&self, building: u64) -> usize {
        self.building_sources.get(&building).into_iter().flatten()
            .filter_map(|source| self.sources.get(source)).flatten()
            .filter(|&&h| self.place_handles.resolve(h).is_some_and(|p| self.places.active[p])).count()
    }

    /// Adds supply at an edit/spawn boundary; O(building anchor sources); may allocate,
    /// never a tick operation.
    pub(crate) fn add_place(&mut self, source: PlaceSource, bay: Bay) -> Handle<Place> {
        if let PlaceSource::Building { building, .. } = source {
            let sources = self.building_sources.entry(building).or_default();
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        let key = chunk_at([bay.position[0], bay.position[2]]);
        let chunk = self.chunks.entry(key).or_default();
        let source_list = self.sources.entry(source).or_default();
        self.places.push(Place {
            source,
            active: true,
            retired_for: Handle::NONE,
            road_point: [0.0; 2],
            identity: bay.identity,
            position: bay.position,
            path: bay.path,
            access: Default::default(),
            heading: bay.heading,
            width: bay.width,
            length: bay.length,
            occupant: Handle::NONE,
            reservation: Handle::NONE,
            chunk_slot: chunk.places.len(),
            source_slot: source_list.len(),
            parked_slot: data::NOT_PARKED,
            ground: [0.0; 12],
            ground_model: data::GROUND_UNSOLVED,
        });
        self.usable_places += 1;
        self.place_handles.register_appended(self.places.len());
        let handle = self.place_handles.handle(self.places.len() - 1);
        chunk.places.push(handle);
        chunk.parked.reserve(chunk.places.len() - chunk.parked.len());
        source_list.push(handle);
        let capacity = self.chunks.len();
        if self.dirty.capacity() < capacity {
            self.dirty.reserve(capacity - self.dirty.len());
        }
        if self.release_events.capacity() < capacity {
            self.release_events
                .reserve(capacity - self.release_events.len());
        }
        self.mark_dirty(key);
        handle
    }

    /// O(9 + local candidates × Q), no allocation or candidate cap. Caller quotes connectivity,
    /// walking distance/time and driving time in Q; None rejects an unreachable place.
    /// Euclidean distance is a lower bound: no candidate outside 400 m is quoted.
    pub(crate) fn nearest_with(
        &self,
        point: [f32; 2],
        mut quote: impl FnMut(Handle<Place>, [f32; 3]) -> Option<ParkingCost>,
    ) -> Option<Handle<Place>> {
        let (cx, cz) = chunk_at(point);
        let mut best: Option<(ParkingCost, Handle<Place>)> = None;
        for z in cz - 1..=cz + 1 {
            for x in cx - 1..=cx + 1 {
                let Some(chunk) = self.chunks.get(&(x, z)) else {
                    continue;
                };
                for &handle in &chunk.places {
                    let p = self.place_handles.resolve(handle)?;
                    if !self.places.active[p]
                        || !self.places.occupant[p].is_none()
                        || !self.places.reservation[p].is_none()
                        || self.places.width[p] < 1.8
                        || self.places.length[p] < 4.5
                    {
                        continue;
                    }
                    let pos = self.places.position[p];
                    let d2 = (pos[0] - point[0]).powi(2) + (pos[2] - point[1]).powi(2);
                    if d2 > 400.0 * 400.0 {
                        continue;
                    }
                    let Some(cost) = quote(handle, pos) else {
                        continue;
                    };
                    if !cost.walk_m.is_finite()
                        || !(0.0..=400.0).contains(&cost.walk_m)
                        || !cost.walk_s.is_finite()
                        || cost.walk_s < 0.0
                        || !cost.drive_s.is_finite()
                        || cost.drive_s < 0.0
                    {
                        continue;
                    }
                    let better = best.as_ref().is_none_or(|(old, old_handle)| {
                        cost.walk_s
                            .total_cmp(&old.walk_s)
                            .then(cost.drive_s.total_cmp(&old.drive_s))
                            .then(handle.cmp(old_handle))
                            .is_lt()
                    });
                    if better {
                        best = Some((cost, handle));
                    }
                }
            }
        }
        best.map(|(_, h)| h)
    }

    /// Foundation query: O(9 + local places), allocation-free; straight-line distance and
    /// the existing walking speed. Round 2 supplies connected route quotes through nearest_with.
    pub(crate) fn nearest(&self, point: [f32; 2]) -> Option<Handle<Place>> {
        self.nearest_with(point, |_, pos| {
            let distance = ((pos[0] - point[0]).powi(2) + (pos[2] - point[1]).powi(2)).sqrt();
            Some(ParkingCost {
                walk_m: distance,
                walk_s: distance / crate::config::AGENT_WALK_SPEED_MS,
                drive_s: 0.0,
            })
        })
    }

    /// Atomically reserves a free bay; O(1), no allocation. A vehicle has at most one claim.
    pub(crate) fn reserve(&mut self, car: Handle<Vehicle>, place: Handle<Place>) -> bool {
        let (Some(v), Some(p)) = (
            self.vehicle_handles.resolve(car),
            self.place_handles.resolve(place),
        ) else {
            return false;
        };
        if !self.places.active[p]
            || !self.vehicles.reserved[v].is_none()
            || !self.places.occupant[p].is_none()
            || !self.places.reservation[p].is_none()
        {
            return false;
        }
        self.vehicles.reserved[v] = place;
        self.places.reservation[p] = car;
        self.custody_changes.push(car);
        true
    }

    /// Turns a free or self-reserved bay into occupancy; O(1), allocation-free.
    /// Existing occupancy must first be released by its lifecycle transition.
    pub(crate) fn occupy(&mut self, car: Handle<Vehicle>, place: Handle<Place>) -> bool {
        let (Some(v), Some(p)) = (
            self.vehicle_handles.resolve(car),
            self.place_handles.resolve(place),
        ) else {
            return false;
        };
        if !self.places.active[p]
            || !self.vehicles.place[v].is_none()
            || !self.places.occupant[p].is_none()
            || (!self.places.reservation[p].is_none() && self.places.reservation[p] != car)
            || (!self.vehicles.reserved[v].is_none() && self.vehicles.reserved[v] != place)
        {
            return false;
        }
        self.places.reservation[p] = Handle::NONE;
        self.note_reservation_ended(v);
        self.vehicles.reserved[v] = Handle::NONE;
        self.vehicles.place[v] = place;
        self.places.occupant[p] = car;
        self.custody_changes.push(car);
        self.set_parked(p, !self.vehicles.moving[v]);
        self.mark_dirty(chunk_at([
            self.places.position[p][0],
            self.places.position[p][2],
        ]));
        true
    }

    /// Clears reciprocal occupancy, preserving an independent destination reservation; O(1),
    /// allocation-free. The ordered release boundary then wakes only local off-map cars.
    pub(crate) fn release(&mut self, car: Handle<Vehicle>) -> Option<[f32; 2]> {
        let v = self.vehicle_handles.resolve(car)?;
        let p = self.place_handles.resolve(self.vehicles.place[v])?;
        let pos = self.places.position[p];
        self.places.occupant[p] = Handle::NONE;
        self.vehicles.place[v] = Handle::NONE;
        self.custody_changes.push(car);
        self.set_parked(p, false);
        let key = chunk_at([pos[0], pos[2]]);
        let chunk = self.chunks.get_mut(&key).expect("occupied parking chunk");
        if !chunk.release_pending {
            chunk.release_pending = true;
            self.release_events.push(key);
        }
        chunk.wake_after = None;
        self.mark_dirty(chunk_at([pos[0], pos[2]]));
        Some([pos[0], pos[2]])
    }

    /// Cancels the vehicle's reciprocal destination claim in O(1), allocation-free.
    pub(crate) fn cancel_reservation(&mut self, car: Handle<Vehicle>) {
        let Some(v) = self.vehicle_handles.resolve(car) else {
            return;
        };
        if let Some(p) = self.place_handles.resolve(self.vehicles.reserved[v]) {
            self.places.reservation[p] = Handle::NONE;
            let pos = self.places.position[p];
            let key = chunk_at([pos[0], pos[2]]);
            let chunk = self.chunks.get_mut(&key).expect("reserved parking chunk");
            chunk.wake_after = None;
            if !chunk.release_pending {
                chunk.release_pending = true;
                self.release_events.push(key);
            }
        }
        self.note_reservation_ended(v);
        self.vehicles.reserved[v] = Handle::NONE;
        self.custody_changes.push(car);
    }

    // The borrower's trip may have depended on the reservation ending now. O(1).
    fn note_reservation_ended(&mut self, v: usize) {
        let borrower = self.vehicles.borrower[v];
        if !self.vehicles.reserved[v].is_none() && !borrower.is_none() {
            self.lost_reservations.push(borrower);
        }
    }

    fn mark_dirty(&mut self, key: (i32, i32)) {
        let chunk = self.chunks.get_mut(&key).expect("place prepares its chunk");
        if !chunk.dirty {
            chunk.dirty = true;
            self.dirty.push(key);
        }
    }

    /// Test/debug boundary only. O(places), creates persistent deterministic models/colours;
    /// never called by normal gameplay or agent ticks.
    pub(crate) fn fill_test_fleet(&mut self) -> usize {
        let mut count = 0;
        for p in 0..self.places.len() {
            if !self.places.active[p]
                || !self.places.occupant[p].is_none()
                || !self.places.reservation[p].is_none()
            {
                continue;
            }
            let pos = self.places.position[p];
            let v = self.vehicles.len();
            self.vehicles.push(Vehicle {
                home: [pos[0], pos[2]],
                home_building: u64::MAX,
                owner: u64::MAX,
                borrower: Handle::NONE,
                moving: false,
                retired_places: [Handle::NONE; 3],
                place: Handle::NONE,
                reserved: Handle::NONE,
                model: (v % 4) as u8,
                colour: (v % 5) as u8,
            });
            self.vehicle_handles.register_appended(self.vehicles.len());
            let car = self.vehicle_handles.handle(v);
            let place = self.place_handles.handle(p);
            self.reserve(car, place);
            self.occupy(car, place);
            count += 1;
        }
        count
    }
}
