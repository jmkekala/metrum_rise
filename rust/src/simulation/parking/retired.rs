// SPDX-License-Identifier: GPL-2.0-only

//! Removed supply retains only the exact geometry still needed by its live borrower.

use super::*;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use godot::prelude::{Vector2, Vector3};

impl ParkingSystem {
    // A borrower can reference the origin, destination and one pedestrian recovery origin.
    // The three reverse links are fixed-size; removing supply never scans vehicles or agents.
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
            let slot = links
                .iter_mut()
                .find(|h| h.is_none())
                .expect("three itinerary endpoints bound retired geometry");
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

    /// Releases one obsolete endpoint in O(1), preserving any supply republished by undo.
    pub(crate) fn forget_retired_place(&mut self, car: Handle<Vehicle>, place: Handle<Place>) {
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

    /// Rebinds only newly retired endpoints, or all of them once on load, through the existing
    /// edge R-tree. O(E × (log roads + local edge geometry)), E dirty retained endpoints.
    /// No new spatial index or simulation-tick search is introduced.
    pub(crate) fn refresh_retired_access(&mut self, network: &TransitNetwork, graph: &RegionGraph) {
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
        }
        dirty.clear();
        self.retired_dirty = dirty;
    }
}
