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
    /// Replaces one building's docks with `docks` (anchor, geometry), their authored drives
    /// `ways` (anchor, drive; `PARK-21`) and the anchors of those a truck backs into, `tail_in`
    /// (`PARK-23`), keeping a dock whose pose is unchanged under its handle. A dock a carrier holds (`freight_holder`) goes inactive and stays, with its
    /// drive, until it is released; callers clear the holders that have left first
    /// ([`Self::clear_left_dock_holders`]). O(old log old + new × ways) for the building's
    /// docks; an edit or load boundary.
    pub(crate) fn replace_docks(
        &mut self,
        building: u64,
        docks: &[(u32, Bay)],
        ways: &[(u32, DockWay)],
        tail_in: &[u32],
    ) {
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
            let way = ways.iter().find(|(a, _)| *a == anchor).map(|(_, way)| way);
            self.replace_dock_source(source, bay, way, tail_in.contains(&anchor));
        }
        sources.retain(|source| self.sources.contains_key(source));
        if !sources.is_empty() {
            self.building_docks.insert(building, sources);
        }
    }

    // One loading bay's rows: the row with `bay`'s pose kept and updated, its drive and stand
    // with it, every other removed, a new row when none kept. O(rows of the source), at most two.
    fn replace_dock_source(
        &mut self,
        source: PlaceSource,
        bay: Option<&Bay>,
        way: Option<&DockWay>,
        tail_in: bool,
    ) {
        let mut old = self.sources.get(&source).cloned().unwrap_or_default();
        old.sort_unstable();
        let mut kept = false;
        for handle in old {
            let p = self.place_handles.resolve(handle).expect("dock row");
            // A held row keeps its route and its drive (`PARK-21`): its truck may be on them, and
            // the way it drives must not change under it. One whose route or drive changes (an
            // arrival gate or an exit moved) is retired for it, as a moved dock is; so is one
            // whose stand changes (`PARK-23`), so its truck never turns about under the author.
            let same_pose = bay.is_some_and(|bay| {
                self.places.position[p].map(f32::to_bits) == bay.position.map(f32::to_bits)
                    && self.places.heading[p].to_bits() == bay.heading.to_bits()
                    && (self.places.freight_holder[p].is_none()
                        || (self.places.path[p].map(|q| q.map(f32::to_bits))
                            == bay.path.map(|q| q.map(f32::to_bits))
                            && self.dock_ways.get(&handle) == way
                            && self.dock_tail_in.contains(&handle) == tail_in))
            });
            match bay {
                Some(bay) if same_pose && !kept => {
                    kept = true;
                    self.mark_paint(source, bay.position);
                    self.places.active[p] = true;
                    self.places.path[p] = bay.path;
                    self.places.width[p] = bay.width;
                    self.places.length[p] = bay.length;
                    self.set_dock_way(handle, way);
                    self.set_dock_stand(handle, tail_in);
                }
                _ => self.remove_dock(handle),
            }
        }
        if let (false, Some(bay)) = (kept, bay) {
            let handle = self.add_dock(source, *bay);
            self.set_dock_way(handle, way);
            self.set_dock_stand(handle, tail_in);
        }
    }

    // Records whether a truck backs into a dock row (`PARK-23`). O(1).
    fn set_dock_stand(&mut self, handle: Handle<Place>, tail_in: bool) {
        if tail_in {
            self.dock_tail_in.insert(handle);
        } else {
            self.dock_tail_in.remove(&handle);
        }
    }

    // Records or clears a dock row's authored drive; its exit's access is derived again with
    // the row's (`refresh_dock_access`). O(1).
    fn set_dock_way(&mut self, handle: Handle<Place>, way: Option<&DockWay>) {
        self.dock_exits.remove(&handle);
        match way {
            Some(way) => self.dock_ways.insert(handle, *way),
            None => self.dock_ways.remove(&handle),
        };
    }

    // A new dock row: the handle table, its source list and its chunk's dock list (the paint's
    // alone, `PARK-30`). O(1) amortised.
    fn add_dock(&mut self, source: PlaceSource, bay: Bay) -> Handle<Place> {
        self.mark_paint(source, bay.position);
        let source_list = self.sources.entry(source).or_default();
        self.places.push(Place {
            source,
            active: true,
            retired_for: Handle::NONE,
            road_point: [0.0; 2],
            identity: bay.identity,
            position: bay.position,
            path: bay.path,
            way: ways::NO_WAY,
            access: Default::default(),
            heading: bay.heading,
            width: bay.width,
            length: bay.length,
            occupant: Handle::NONE,
            reservation: Handle::NONE,
            freight_holder: Handle::NONE,
            chunk_slot: 0,
            source_slot: source_list.len(),
            parked_slot: data::NOT_PARKED,
            ground: [0.0; 12],
            ground_model: data::GROUND_UNSOLVED,
        });
        self.place_handles.register_appended(self.places.len());
        let handle = self.place_handles.handle(self.places.len() - 1);
        source_list.push(handle);
        let slot = self.list_dock(handle, bay.position);
        let last = self.places.len() - 1;
        self.places.chunk_slot[last] = slot;
        handle
    }

    // Removes a dock row, or retires it while a carrier holds it. O(1).
    fn remove_dock(&mut self, handle: Handle<Place>) {
        let Some(p) = self.place_handles.resolve(handle) else {
            return;
        };
        self.mark_paint(self.places.source[p], self.places.position[p]);
        if !self.places.freight_holder[p].is_none() {
            self.places.active[p] = false;
            return;
        }
        self.unlist_dock(p);
        self.dock_ways.remove(&handle);
        self.dock_exits.remove(&handle);
        self.dock_tail_in.remove(&handle);
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

    /// A dock's authored drive (`PARK-21`): its whole way in and its way out to the exit gate;
    /// `None` for a stage A dock, whose route is [`Self::dock_route`] and which backs out along
    /// it. O(1). Test builds only: movement reads the drive through [`Self::maneuver_path`].
    #[cfg(test)]
    pub(crate) fn dock_way(&self, dock: Handle<Place>) -> Option<&DockWay> {
        self.dock_ways.get(&dock)
    }

    /// Whether a truck backs into `dock` and leaves it forward (`PARK-23`): it travels the
    /// dock's way in tail first and back out nose first. O(1); one emptiness test in a city
    /// without such a dock.
    pub(crate) fn dock_backs_in(&self, dock: Handle<Place>) -> bool {
        !self.dock_tail_in.is_empty() && self.dock_tail_in.contains(&dock)
    }

    /// The road connection of the gate a truck leaves `dock` by (`PARK-21`): its way out's last
    /// point, or its route's first point when it backs out through its arrival gate. O(1).
    pub(crate) fn dock_departure_kerb(&self, dock: Handle<Place>) -> Option<[f32; 2]> {
        match self.dock_ways.get(&dock).map(|way| way.route_out.as_slice()) {
            Some([.., last]) => Some(*last),
            _ => self.dock_kerb(dock),
        }
    }

    /// The access a truck leaving `dock` joins the road by, its lane ids derived from the
    /// current lanes (`PARK-21`): its exit gate's for a dock with a way out, else the dock's own
    /// ([`Self::place_entrance`], its arrival gate's). The origin of every plan out of a dock
    /// made from the dock itself (`plan_way_out_of_dock`) and of its pull-out's attach check.
    /// O(1) plus O(lanes on the gate's edge), allocation-free.
    pub(crate) fn dock_departure(
        &self,
        dock: Handle<Place>,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<crate::simulation::buildings::allocator::BuildingEntrance> {
        let Some(exit) = self.dock_exits.get(&dock) else {
            return self.live_access(dock, network, graph);
        };
        if exit.edge_idx == usize::MAX {
            return None;
        }
        let mut exit = exit.clone();
        BuildingAllocator::refresh_entrance_lanes(&mut exit, graph, &network.lane_system);
        Some(exit)
    }

    /// The loading bay anchor a dock row was proven from. O(1).
    pub(crate) fn dock_anchor(&self, dock: Handle<Place>) -> Option<u32> {
        match self.places.source[self.place_handles.resolve(dock)?] {
            PlaceSource::Dock { anchor, .. } => Some(anchor),
            _ => None,
        }
    }

    /// Every dock row of the building, active or retired, in source and slot order. O(its
    /// rows).
    pub(crate) fn building_dock_rows(&self, building: u64) -> impl Iterator<Item = Handle<Place>> + '_ {
        self.building_docks
            .get(&building)
            .into_iter()
            .flatten()
            .filter_map(|source| self.sources.get(source))
            .flatten()
            .copied()
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
        let docks: Vec<_> = self.building_dock_rows(building).collect();
        for dock in docks {
            self.refresh_place_access(dock, allocator, network, graph);
            self.refresh_dock_exit(dock, graph);
        }
    }

    // Derives the access of a dock row's exit gate (`PARK-21`), as its own access is derived but
    // at its way out's last point; none for a dock without a way out. O(the host's entrance
    // copy + one projection on its road).
    fn refresh_dock_exit(&mut self, dock: Handle<Place>, graph: &RegionGraph) {
        let Some(&[.., [x, z]]) = self.dock_ways.get(&dock).map(|way| way.route_out.as_slice()) else {
            self.dock_exits.remove(&dock);
            return;
        };
        let p = self.place_handles.resolve(dock).expect("dock row");
        let mut exit = self.places.access[p].clone();
        if let Some(edge) = graph.get_edge(exit.edge_idx).filter(|e| !e.deleted) {
            let kerb = godot::prelude::Vector2::new(x, z);
            exit.entrance_s_m = BuildingAllocator::project_point_to_polyline_s(&edge.physical_geometry, kerb);
            exit.curb_pos = kerb;
        } else {
            exit = Default::default();
        }
        self.dock_exits.insert(dock, exit);
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
