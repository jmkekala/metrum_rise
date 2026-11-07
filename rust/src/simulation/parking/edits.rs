// SPDX-License-Identifier: GPL-2.0-only

//! Ordered supply reconciliation and towing; undo never copies vehicle state.

use super::*;

impl ParkingSystem {
    /// Removes one bay and its claims; O(1) index work, plus a bounded local tow query.
    /// Never refuses an edit. Off-map cars remain persistent and are indexed by home chunk.
    pub(crate) fn remove_place(&mut self, handle: Handle<Place>) {
        let Some(p) = self.place_handles.resolve(handle) else {
            return;
        };
        let car = self.places.occupant[p];
        let reservation = self.places.reservation[p];
        if self.places.active[p] {
            self.journal_place(handle);
            self.places.active[p] = false;
            self.usable_places -= 1;
            let retiring_car = if !reservation.is_none() {
                reservation
            } else {
                car
            };
            self.remember_retired_place(handle, retiring_car);
            if !reservation.is_none() {
                self.cancel_reservation(reservation);
            }
            if !car.is_none() {
                self.cancel_reservation(car);
                self.release(car);
            }
            if let Some(v) = self.vehicle_handles.resolve(car)
                && !self.vehicles.moving[v]
            {
                let home = self.vehicles.home[v];
                if let Some(destination) = self.home_place(v) {
                    self.occupy(car, destination);
                } else {
                    self.stored.entry(chunk_at(home)).or_default().insert(car);
                }
            }
        }
        // Only geometry needed by the live borrower remains. It is not supply or occupancy.
        if !self.places.retired_for[p].is_none() {
            return;
        }
        let pos = self.places.position[p];
        let key = chunk_at([pos[0], pos[2]]);
        let slot = self.places.chunk_slot[p];
        let chunk = self.chunks.get_mut(&key).expect("indexed bay");
        chunk.places.swap_remove(slot);
        if let Some(&moved) = chunk.places.get(slot) {
            self.places.chunk_slot[self.place_handles.resolve(moved).expect("live bay")] = slot;
        }
        let source = self.places.source[p];
        let slot = self.places.source_slot[p];
        let list = self.sources.get_mut(&source).expect("indexed source");
        list.swap_remove(slot);
        if let Some(&moved) = list.get(slot) {
            self.places.source_slot[self.place_handles.resolve(moved).expect("live source bay")] =
                slot;
        }
        if list.is_empty() {
            self.sources.remove(&source);
        }
        self.place_handles.swap_remove(p, self.places.len());
        self.places.swap_remove(p);
        self.mark_dirty(key);
    }

    /// Reconciles one owner's geometry, keeping identical poses and live custody. O(old log old + new)
    /// expected map/sort work plus local towing. Called only at topology/asset boundaries.
    pub(crate) fn replace_source(&mut self, source: PlaceSource, bays: &[Bay]) {
        let mut old = self.sources.get(&source).cloned().unwrap_or_default();
        old.sort_unstable();
        let pose_key = |pos: [f32; 3], heading: f32| (pos.map(f32::to_bits), heading.to_bits());
        let mut retained: HashMap<_, _> = old
            .iter()
            .map(|&h| {
                let p = self.place_handles.resolve(h).expect("source bay");
                (pose_key(self.places.position[p], self.places.heading[p]), h)
            })
            .collect();
        for bay in bays {
            // Only added or reactivated bays are new supply. An unchanged bay must not wake
            // waiting cars, or a load's frontage reconcile would schedule arrivals the
            // uninterrupted game never did.
            let added = if let Some(h) = retained.remove(&pose_key(bay.position, bay.heading)) {
                let p = self.place_handles.resolve(h).expect("retained bay");
                let reactivated = !self.places.active[p];
                if reactivated {
                    self.places.active[p] = true;
                    self.usable_places += 1;
                }
                // A bay's path gives its access and the driveway time doors sort it by.
                if reactivated || self.places.path[p] != bay.path {
                    self.journal_place(h);
                }
                self.places.width[p] = bay.width;
                self.places.length[p] = bay.length;
                self.places.path[p] = bay.path;
                reactivated
            } else {
                let place = self.add_place(source, *bay);
                self.journal_place(place);
                true
            };
            if added {
                self.rehouse_near([bay.position[0], bay.position[2]]);
            }
        }
        // Stable handles fix tow order even when load rebuilt different swap-remove lists.
        for h in old {
            let Some(p) = self.place_handles.resolve(h) else {
                continue;
            };
            if retained.contains_key(&pose_key(self.places.position[p], self.places.heading[p])) {
                self.remove_place(h);
            }
        }
    }

    /// Queues a deduplicated supply event in O(1) expected chunk work. No vehicle is relocated.
    /// Local waiting cars are considered only by bounded scheduled delivery events.
    pub(crate) fn rehouse_near(&mut self, point: [f32; 2]) {
        let key = chunk_at(point);
        let chunk = self.chunks.entry(key).or_default();
        chunk.wake_after = None;
        if !chunk.release_pending {
            chunk.release_pending = true;
            self.release_events.push(key);
        }
    }

    /// Next stable waiting handle across nine home chunks: O(9 log W), allocation-free.
    /// The saved cursor bounds each continuation without rescanning an already considered car.
    pub(crate) fn next_stored_car(&mut self, key: (i32, i32)) -> Option<Handle<Vehicle>> {
        use std::ops::Bound::{Excluded, Unbounded};
        let after = self.chunks.get(&key).and_then(|chunk| chunk.wake_after);
        let mut next = None;
        for z in key.1 - 1..=key.1 + 1 {
            for x in key.0 - 1..=key.0 + 1 {
                if let Some(cars) = self.stored.get(&(x, z)) {
                    let car = if let Some(after) = after {
                        cars.range((Excluded(after), Unbounded)).next()
                    } else {
                        cars.first()
                    };
                    if let Some(&car) = car {
                        next = Some(next.map_or(car, |old: Handle<Vehicle>| old.min(car)));
                    }
                }
            }
        }
        if let Some(car) = next {
            self.chunks
                .get_mut(&key)
                .expect("scheduled chunk")
                .wake_after = Some(car);
        }
        next
    }

    /// Ends this finite wake cycle in O(1) expected chunk work; failed cars sleep until supply changes.
    pub(crate) fn finish_supply_event(&mut self, key: (i32, i32)) {
        if let Some(chunk) = self.chunks.get_mut(&key) {
            chunk.release_pending = false;
            chunk.wake_after = None;
        }
    }

    /// O(1) event lookups of a stored car's live home; temporary/debug fleets have none.
    pub(crate) fn stored_car_home(&self, car: Handle<Vehicle>) -> Option<u64> {
        let v = self.vehicle_handles.resolve(car)?;
        (!self.vehicles.moving[v] && self.vehicles.place[v].is_none())
            .then_some(self.vehicles.home_building[v])
            .filter(|&h| h != u64::MAX)
    }
}

impl ParkingSystem {
    /// Replaces only this building: O(old log old + old × new anchors + local towing),
    /// bounded by its asset-local swept-path validation, never city size.
    pub(crate) fn replace_building(&mut self, building: u64, bays: &[(u32, Bay)]) {
        let mut old = self.building_sources.remove(&building).unwrap_or_default();
        old.sort_unstable();
        for source in old {
            let PlaceSource::Building { anchor, .. } = source else {
                continue;
            };
            let bay = bays.iter().find(|(i, _)| *i == anchor).map(|(_, b)| *b);
            self.replace_source(source, bay.as_slice());
        }
        for &(anchor, bay) in bays {
            let source = PlaceSource::Building { building, anchor };
            self.replace_source(source, &[bay]);
        }
        let sources: Vec<_> = bays
            .iter()
            .map(|&(anchor, _)| PlaceSource::Building { building, anchor })
            .collect();
        if !sources.is_empty() {
            self.building_sources.insert(building, sources);
        }
    }
}
