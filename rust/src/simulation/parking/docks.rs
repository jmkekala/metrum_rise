// SPDX-License-Identifier: GPL-3.0-or-later

//! Dock rows (`PARK-19`): a building's proven loading bays, kept in the place table beside the
//! bays (handles, source lists, saved rows, cached access) but never as supply. A dock is in no
//! chunk list and no supply count, so no search, door catchment, stored-car wake or parked-car
//! publication sees it. Reconciled at the site and frontage hooks that reconcile bays. In a tick
//! only the parking phase's ordered commit reads and writes a dock's holder (`PARK-20`).

use super::*;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentHandle;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};

impl ParkingSystem {
    /// Replaces one building's docks with `docks` (anchor, geometry), keeping a dock whose pose
    /// is unchanged under its handle. A dock a carrier holds (`freight_holder`) goes inactive
    /// and stays until it is released; callers clear the holders that have left first
    /// ([`Self::clear_left_dock_holders`]). O(old log old + new) for the building's docks; an
    /// edit or load boundary.
    pub(crate) fn replace_docks(&mut self, building: u64, docks: &[(u32, Bay)]) {
        let mut sources = self.building_docks.remove(&building).unwrap_or_default();
        sources.extend(
            docks
                .iter()
                .map(|&(anchor, _)| PlaceSource::Dock { building, anchor }),
        );
        sources.sort_unstable();
        sources.dedup();
        for &source in &sources {
            let PlaceSource::Dock { anchor, .. } = source else {
                continue;
            };
            let bay = docks.iter().find(|(a, _)| *a == anchor).map(|(_, bay)| bay);
            self.replace_dock_source(source, bay);
        }
        sources.retain(|source| self.sources.contains_key(source));
        if !sources.is_empty() {
            self.building_docks.insert(building, sources);
        }
    }

    // One loading bay's rows: the row with `bay`'s pose kept and updated, every other removed,
    // a new row when none kept. O(rows of the source), at most two.
    fn replace_dock_source(&mut self, source: PlaceSource, bay: Option<&Bay>) {
        let mut old = self.sources.get(&source).cloned().unwrap_or_default();
        old.sort_unstable();
        let mut kept = false;
        for handle in old {
            let p = self.place_handles.resolve(handle).expect("dock row");
            let same_pose = bay.is_some_and(|bay| {
                self.places.position[p].map(f32::to_bits) == bay.position.map(f32::to_bits)
                    && self.places.heading[p].to_bits() == bay.heading.to_bits()
            });
            match bay {
                Some(bay) if same_pose && !kept => {
                    kept = true;
                    self.places.active[p] = true;
                    self.places.path[p] = bay.path;
                    self.places.width[p] = bay.width;
                    self.places.length[p] = bay.length;
                }
                _ => self.remove_dock(handle),
            }
        }
        if let (false, Some(bay)) = (kept, bay) {
            self.add_dock(source, *bay);
        }
    }

    // A new dock row: the handle table and its source list, nothing else. O(1) amortised.
    fn add_dock(&mut self, source: PlaceSource, bay: Bay) -> Handle<Place> {
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
            freight_holder: Handle::NONE,
            chunk_slot: data::NO_CHUNK,
            source_slot: source_list.len(),
            parked_slot: data::NOT_PARKED,
            ground: [0.0; 12],
            ground_model: data::GROUND_UNSOLVED,
        });
        self.place_handles.register_appended(self.places.len());
        let handle = self.place_handles.handle(self.places.len() - 1);
        source_list.push(handle);
        handle
    }

    // Removes a dock row, or retires it while a carrier holds it. O(1).
    fn remove_dock(&mut self, handle: Handle<Place>) {
        let Some(p) = self.place_handles.resolve(handle) else {
            return;
        };
        if !self.places.freight_holder[p].is_none() {
            self.places.active[p] = false;
            return;
        }
        let source = self.places.source[p];
        let slot = self.places.source_slot[p];
        let list = self.sources.get_mut(&source).expect("indexed dock source");
        list.swap_remove(slot);
        if let Some(&moved) = list.get(slot) {
            self.places.source_slot[self.place_handles.resolve(moved).expect("live dock")] = slot;
        }
        if list.is_empty() {
            self.sources.remove(&source);
        }
        self.place_handles.swap_remove(p, self.places.len());
        self.places.swap_remove(p);
    }

    /// The building's active docks, in anchor order. O(its docks).
    pub(crate) fn building_docks(&self, building: u64) -> impl Iterator<Item = Handle<Place>> + '_ {
        self.building_docks
            .get(&building)
            .into_iter()
            .flatten()
            .filter_map(|source| self.sources.get(source))
            .flatten()
            .copied()
            .filter(|&h| {
                self.place_handles
                    .resolve(h)
                    .is_some_and(|p| self.places.active[p])
            })
    }

    /// Every building with a dock row, ascending. O(buildings with docks log them).
    pub(crate) fn dock_buildings(&self) -> Vec<u64> {
        let mut buildings: Vec<_> = self.building_docks.keys().copied().collect();
        buildings.sort_unstable();
        buildings
    }

    /// The road connection of the gate a dock's route starts at: its route's first point.
    /// O(1).
    pub(crate) fn dock_kerb(&self, dock: Handle<Place>) -> Option<[f32; 2]> {
        self.place_handles
            .resolve(dock)
            .map(|p| self.places.path[p][0])
    }

    /// The carrier recorded as holding `dock` (`PARK-20`): none, or one the caller must confirm
    /// by its trip, since a carrier that left or was removed does not clear it. O(1).
    pub(crate) fn dock_holder(&self, dock: Handle<Place>) -> AgentHandle {
        self.place_handles
            .resolve(dock)
            .map_or(AgentHandle::NONE, |p| self.places.freight_holder[p])
    }

    /// Records `holder` (none to release) as the carrier holding `dock`. O(1); an ordered-phase
    /// commit only.
    pub(crate) fn set_dock_holder(&mut self, dock: Handle<Place>, holder: AgentHandle) {
        if let Some(p) = self.place_handles.resolve(dock) {
            self.places.freight_holder[p] = holder;
        }
    }

    /// Clears the holder of each of the building's dock rows, active or retired, that `holds`
    /// says has left it, so a reconcile removes a retired row nobody holds any more instead of
    /// keeping it for a carrier that delivered or was removed (`PARK-20`). O(its dock rows); an
    /// edit or load boundary.
    pub(crate) fn clear_left_dock_holders(
        &mut self,
        building: u64,
        holds: impl Fn(AgentHandle, Handle<Place>) -> bool,
    ) {
        let Some(sources) = self.building_docks.get(&building) else {
            return;
        };
        for &dock in sources.iter().filter_map(|s| self.sources.get(s)).flatten() {
            let p = self.place_handles.resolve(dock).expect("dock row");
            let holder = self.places.freight_holder[p];
            if !holder.is_none() && !holds(holder, dock) {
                self.places.freight_holder[p] = AgentHandle::NONE;
            }
        }
    }

    /// A dock's proven route: the gate's road connection, the approach, the aisle and the stop.
    /// O(1).
    pub(crate) fn dock_route(&self, dock: Handle<Place>) -> Option<[[f32; 2]; 4]> {
        self.place_handles
            .resolve(dock)
            .map(|p| self.places.path[p])
    }

    /// Publishes the access of the building's docks from the current door and roads, as a bay's
    /// (`derive_place_entrance`, whose kerb is the route's first point). O(its docks × the
    /// host's entrance copy).
    pub(crate) fn refresh_dock_access(
        &mut self,
        building: u64,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        // Every row, a retired one a truck still holds included: after a load its access is
        // what that truck leaves the yard by (`PARK-20`).
        let docks: Vec<_> = self
            .building_docks
            .get(&building)
            .into_iter()
            .flatten()
            .filter_map(|source| self.sources.get(source))
            .flatten()
            .copied()
            .collect();
        for dock in docks {
            self.refresh_place_access(dock, allocator, network, graph);
        }
    }

    /// Every dock row's source, for the audits and the older-save derivation's check. O(places),
    /// test builds only.
    #[cfg(test)]
    pub(crate) fn dock_rows(&self) -> Vec<(PlaceSource, bool, Bay)> {
        let mut rows: Vec<_> = (0..self.places.len())
            .filter(|&p| matches!(self.places.source[p], PlaceSource::Dock { .. }))
            .map(|p| {
                let bay = Bay {
                    identity: self.places.identity[p],
                    position: self.places.position[p],
                    path: self.places.path[p],
                    heading: self.places.heading[p],
                    width: self.places.width[p],
                    length: self.places.length[p],
                };
                (self.places.source[p], self.places.active[p], bay)
            })
            .collect();
        rows.sort_unstable_by_key(|row| row.0);
        rows
    }

    /// Test builds: panics if a chunk lists a dock or a dock counts as supply. O(places).
    #[cfg(test)]
    pub(crate) fn audit_docks(&self) {
        for chunk in self.chunks.values() {
            for &handle in &chunk.places {
                let p = self.place_handles.resolve(handle).expect("indexed bay");
                assert!(
                    !matches!(self.places.source[p], PlaceSource::Dock { .. }),
                    "a chunk lists dock {handle:?}"
                );
            }
        }
        let supply = (0..self.places.len())
            .filter(|&p| {
                self.places.active[p] && !matches!(self.places.source[p], PlaceSource::Dock { .. })
            })
            .count();
        assert_eq!(supply, self.usable_places, "a dock counts as supply");
    }
}
