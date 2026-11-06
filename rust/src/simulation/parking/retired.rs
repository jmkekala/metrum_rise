// SPDX-License-Identifier: GPL-2.0-only

//! Removed supply retains only the exact geometry still needed by its live borrower.

use super::*;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use godot::prelude::{Vector2, Vector3};

impl ParkingSystem {
    // A borrower can reference the origin, destination and one pedestrian recovery origin.
    // The three reverse links are fixed-size; removing supply never scans vehicles or agents.
    // Recovery drops every link its trip no longer names, so links beyond those three are only
    // the bays a towed car passed through since it last ran: no trip names them. One edit that
    // removes a row of bays can tow a borrowed car through more of them than there are slots.
    // With every slot taken the newest bay is such a pass-through or a destination recovery
    // replaces anyway, so it goes unretained. Evicting the oldest link instead would drop the
    // bay the walker is heading for and leave it retrying for good.
    pub(super) fn remember_retired_place(&mut self, place: Handle<Place>, car: Handle<Vehicle>) {
        let (Some(p), Some(v)) = (
            self.place_handles.resolve(place),
            self.vehicle_handles.resolve(car),
        ) else {
            return;
        };
        if self.vehicles.borrower[v].is_none() {
            return;
        }
        let links = &mut self.vehicles.retired_places[v];
        if !links.contains(&place) {
            let Some(slot) = links.iter_mut().find(|h| h.is_none()) else {
                return;
            };
            *slot = place;
        }
        self.places.retired_for[p] = car;
        self.retired_dirty.push(place);
    }

    /// Drops the car's bounded retained endpoints in O(1), without allocating. Undo may have
    /// republished a bay; that live supply remains, with only the obsolete reverse link cleared.
    pub(crate) fn clear_retired_places(&mut self, car: Handle<Vehicle>) {
        let Some(v) = self.vehicle_handles.resolve(car) else {
            return;
        };
        let places = std::mem::replace(&mut self.vehicles.retired_places[v], [Handle::NONE; 3]);
        for handle in places {
            let Some(p) = self.place_handles.resolve(handle) else {
                continue;
            };
            self.places.retired_for[p] = Handle::NONE;
            if !self.places.active[p] {
                self.remove_place(handle);
            }
        }
    }

    // Releases one obsolete endpoint in O(1), preserving any supply republished by undo.
    fn forget_retired_place(&mut self, car: Handle<Vehicle>, place: Handle<Place>) {
        let Some(v) = self.vehicle_handles.resolve(car) else {
            return;
        };
        let Some(slot) = self.vehicles.retired_places[v]
            .iter()
            .position(|&p| p == place)
        else {
            return;
        };
        self.vehicles.retired_places[v][slot] = Handle::NONE;
        if let Some(p) = self.place_handles.resolve(place) {
            self.places.retired_for[p] = Handle::NONE;
            if !self.places.active[p] {
                self.remove_place(place);
            }
        }
    }

    /// Reverse links `car` holds, for tests of the three-slot bound. O(1).
    #[cfg(test)]
    pub(crate) fn retired_links(&self, car: Handle<Vehicle>) -> usize {
        self.vehicle_handles.resolve(car).map_or(0, |v| {
            self.vehicles.retired_places[v]
                .iter()
                .filter(|h| !h.is_none())
                .count()
        })
    }

    /// Keeps only the links `named` by the car's itinerary, in O(1) and without allocating, when
    /// recovery has rewritten that itinerary: a superseded destination, a replaced walk origin
    /// and the bays a tow passed through all release their geometry here.
    pub(crate) fn retain_retired_places(
        &mut self,
        car: Handle<Vehicle>,
        named: [Handle<Place>; 3],
    ) {
        let Some(v) = self.vehicle_handles.resolve(car) else {
            return;
        };
        for place in self.vehicles.retired_places[v] {
            if !place.is_none() && !named.contains(&place) {
                self.forget_retired_place(car, place);
            }
        }
    }

    /// Rebinds only newly retired endpoints, or all of them once on load, through the existing
    /// edge R-tree. O(E × (log roads + local edge geometry)), E dirty retained endpoints.
    /// No new spatial index or simulation-tick search is introduced.
    pub(crate) fn refresh_retired_access(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let mut dirty = std::mem::take(&mut self.retired_dirty);
        dirty.sort_unstable();
        dirty.dedup();
        for &handle in &dirty {
            let Some(p) = self.place_handles.resolve(handle) else {
                continue;
            };
            if self.places.active[p] {
                continue;
            }
            let [x, z] = self.places.road_point[p];
            let point = Vector2::new(x, z);
            let mut chosen = None;
            graph.visit_edges_near_aabb(
                Vector3::new(x - 0.05, 0.0, z - 0.05),
                Vector3::new(x + 0.05, 0.0, z + 0.05),
                |id| {
                    let Some(edge) = graph
                        .get_edge(id)
                        .filter(|e| !e.deleted && e.physical_length > 0.0)
                    else {
                        return;
                    };
                    let station = BuildingAllocator::project_point_to_polyline_s(
                        &edge.physical_geometry,
                        point,
                    );
                    let projected = BuildingAllocator::sample_pos_on_edge(
                        graph,
                        id,
                        station / edge.physical_length,
                    );
                    let distance = projected.distance_squared_to(point);
                    if distance <= 0.0001
                        && chosen.is_none_or(|(d, old): (f32, usize)| {
                            distance.total_cmp(&d).then(id.cmp(&old)).is_lt()
                        })
                    {
                        chosen = Some((distance, id));
                    }
                },
            );
            let side = self.places.access[p].side;
            let pos = self.places.position[p];
            let curb = self.places.path[p][0];
            // Derived, as a save holds no access: a walk across the retired bay's lot stays one.
            let lot_door = match self.places.source[p] {
                PlaceSource::Building { building, .. } => allocator
                    .building_handles
                    .resolve(Handle::from_bits(building))
                    .and_then(|host| allocator.entrances.get(host))
                    .map(|door| door.door_pos),
                PlaceSource::Kerb { .. } => None,
            };
            self.places.access[p] = chosen.map_or_else(
                || crate::simulation::buildings::allocator::BuildingEntrance {
                    side,
                    ..Default::default()
                },
                |(_, edge)| {
                    BuildingAllocator::parking_access_at(
                        Vector2::new(pos[0], pos[2]),
                        Vector2::new(curb[0], curb[1]),
                        edge,
                        side,
                        graph,
                        &network.lane_system,
                    )
                },
            );
            self.places.access[p].lot_door = lot_door;
        }
        dirty.clear();
        self.retired_dirty = dirty;
    }
}
