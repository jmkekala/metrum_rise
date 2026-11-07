// SPDX-License-Identifier: GPL-2.0-only

//! Published per-door walking catchments, over the existing directed pedestrian lanes.

use super::*;
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::accessibility::ModeComponentIndex;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use godot::prelude::Vector2;
use rayon::prelude::*;

mod table;
mod update;
mod walk;

pub(super) use update::PlaceChange;
use walk::WALK_LIMIT_S;

/// One 16-byte per-destination entry, without a retained route. At 100k nonempty doors,
/// candidate and economy metadata use about 9.6 MB of map/header storage, plus 1.6 MB per mean
/// candidate capacity, 0.4 MB per mean component count and 1.6 MB per mean target count.
/// Arc headers add up to 1.6 MB. Mean capacities/counts 12/2/4 use about 38 MB total.
/// Empty lists are not retained. Rolling economy fronts share components, adding 16 bytes
/// per retained destination entry instead of copying the component array.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ParkingCandidate {
    pub place: Handle<Place>,
    pub walk_s: f32,
    pub from_door_s: f32,
}

#[derive(Default, Debug)]
pub(super) struct DestinationCandidates {
    pub entries: Vec<ParkingCandidate>,
    pub economy: ParkingEconomyEnds,
}

/// Publication-time superset of the road components/arrival contexts a destination's bays
/// can use. Occupancy only selects among them; no candidate routes are queried per departure.
#[derive(Default, Debug)]
pub(crate) struct ParkingEconomyEnds {
    /// Sorted distinct existing economy component labels, not a separate connectivity index.
    pub components: std::sync::Arc<[u32]>,
    /// Sorted distinct CCH target-node/frontage pairs for shared exact commute quotes.
    pub targets: Box<[(u32, usize)]>,
    /// Maximum distance from the destination centre to any possible bay arrival node.
    pub node_slack_m: f32,
}

impl ParkingSystem {
    /// Rebuilds specified door catchments at publication time. O(Bₗ × ((Lₗ + Cₗ) log Lₗ
    /// plus Pₗ log Pₗ)), visiting only lanes within 400 m walking time and at most nine supply
    /// chunks per door; independent doors use Rayon. Occupancy changes never rebuild lists.
    /// Returns the doors whose list of places changed, for the caller to wake waiting cars:
    /// an edit is new supply for those homes, a load's reconstruction is not.
    pub(crate) fn rebuild_candidates(
        &mut self,
        buildings: &[usize],
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        car_components: Option<&ModeComponentIndex>,
    ) -> Vec<[f32; 2]> {
        crate::thread_pools::install(|| {
            self.place_changes.clear();
            self.rebuild_doors(buildings, true, allocator, network, graph, car_components)
        })
    }

    // The chunks a set of doors can reach and the bounds of that reach.
    fn door_neighbourhood(
        &self,
        buildings: &[usize],
        allocator: &BuildingAllocator,
    ) -> (Vec<(i32, i32)>, [f32; 4]) {
        let mut door_chunks = Vec::new();
        let mut bounds = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
        for door in buildings.iter().filter_map(|&b| allocator.entrances.get(b)) {
            let door = door.door_pos;
            door_chunks.push(chunk_at([door.x, door.y]));
            bounds = [
                bounds[0].min(door.x),
                bounds[1].min(door.y),
                bounds[2].max(door.x),
                bounds[3].max(door.y),
            ];
        }
        door_chunks.sort_unstable();
        door_chunks.dedup();
        let mut chunks: Vec<_> = door_chunks
            .iter()
            .flat_map(|&(x, z)| (z - 1..=z + 1).flat_map(move |z| (x - 1..=x + 1).map(move |x| (x, z))))
            .filter(|key| self.chunks.contains_key(key))
            .collect();
        chunks.sort_unstable();
        chunks.dedup();
        // A metre past the door's reach covers the rounding of the bounds themselves.
        let reach = [
            bounds[0] - 401.0,
            bounds[1] - 401.0,
            bounds[2] + 401.0,
            bounds[3] + 401.0,
        ];
        (chunks, reach)
    }

    // Rebuilds whole lists, with the access of the bays around the doors: all of them in their
    // chunks for a road edit, whose lanes moved; those in reach for a site edit, where only a bay
    // no door has reached yet can be out of date.
    fn rebuild_doors(
        &mut self,
        buildings: &[usize],
        whole_chunks: bool,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        car_components: Option<&ModeComponentIndex>,
    ) -> Vec<[f32; 2]> {
        let (chunks, reach) = self.door_neighbourhood(buildings, allocator);
        let refreshed = if whole_chunks {
            [f32::NEG_INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::INFINITY]
        } else {
            reach
        };
        self.refresh_chunk_access(&chunks, refreshed, allocator, network, graph);
        let foot = door_foot_graph(buildings, allocator, network, graph);
        let table = self.place_table(foot, &chunks, reach, network, graph, car_components);
        let built: Vec<_> = buildings
            .par_iter()
            .filter_map(|&b| {
                let entrance = allocator.entrances.get(b)?;
                let handle = allocator.building_handles.handle(b);
                if handle.is_none() {
                    return None;
                }
                let centre = &allocator.buildings[b];
                let list = table.door_candidates(
                    entrance,
                    [centre.center_x, centre.center_y],
                    car_components.is_some(),
                    network,
                    graph,
                );
                let door = [entrance.door_pos.x, entrance.door_pos.y];
                Some((handle.to_bits(), door, list))
            })
            .collect();
        if !built.is_empty() {
            self.candidate_revision = self.candidate_revision.wrapping_add(1);
        }
        let mut changed = Vec::new();
        for (handle, door, entries) in built {
            if !entries.entries.is_empty() && car_components.is_none() {
                self.pending_economy.push(handle);
            }
            let old = self
                .candidates
                .get(&handle)
                .map_or(&[][..], |c| &c.entries[..]);
            if !entries
                .entries
                .iter()
                .map(|e| e.place)
                .eq(old.iter().map(|e| e.place))
            {
                changed.push(door);
            }
            if entries.entries.is_empty() {
                self.candidates.remove(&handle);
            } else {
                self.candidates.insert(handle, entries);
            }
        }
        changed
    }

    // Publishes one bay's access, from the current doors and roads. O(its host's entrance).
    fn refresh_place_access(
        &mut self,
        place: Handle<Place>,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let Some(p) = self.place_handles.resolve(place).filter(|&p| self.places.active[p]) else {
            return;
        };
        let access = self
            .derive_place_entrance(place, allocator, network, graph)
            .unwrap_or_default();
        if let Some(point) = road_point_of(&access, graph) {
            self.places.road_point[p] = point;
        }
        self.places.access[p] = access;
    }

    // Publishes the access of the bays in `chunks` inside `bounds`, from the current doors and
    // roads. Derived in parallel, written in the caller's order. O(bays there).
    fn refresh_chunk_access(
        &mut self,
        chunks: &[(i32, i32)],
        bounds: [f32; 4],
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let this = &*self;
        let derived: Vec<_> = chunks
            .par_iter()
            .flat_map_iter(|key| this.chunks.get(key).map_or(&[][..], |c| &c.places))
            .filter_map(|&place| {
                let p = this.place_handles.resolve(place)?;
                let [x, _, z] = this.places.position[p];
                let inside = x >= bounds[0] && z >= bounds[1] && x <= bounds[2] && z <= bounds[3];
                if !this.places.active[p] || !inside {
                    return None;
                }
                let access = this
                    .derive_place_entrance(place, allocator, network, graph)
                    .unwrap_or_default();
                let point = road_point_of(&access, graph);
                Some((p, access, point))
            })
            .collect();
        for (p, access, point) in derived {
            if let Some(point) = point {
                self.places.road_point[p] = point;
            }
            self.places.access[p] = access;
        }
    }

    /// Test builds: panics unless the live lists of `doors` and the access of every bay they
    /// reach are what a rebuild from scratch gives, bit for bit, as a load's would be. Economy
    /// ends are read only when `car_components` is given and none waits for the adoption.
    #[cfg(test)]
    pub(crate) fn audit_candidates(
        &self,
        doors: &[usize],
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        car_components: Option<&ModeComponentIndex>,
    ) {
        use crate::simulation::buildings::allocator::BuildingEntranceKey;
        let (chunks, reach) = self.door_neighbourhood(doors, allocator);
        let in_reach = |pos: [f32; 3]| {
            doors.iter().filter_map(|&b| allocator.entrances.get(b)).any(|door| {
                table::within_door_reach(pos[0] - door.door_pos.x, pos[2] - door.door_pos.y)
            })
        };
        for place in chunks.iter().flat_map(|key| &self.chunks[key].places) {
            let p = self.place_handles.resolve(*place).expect("indexed bay");
            if self.places.active[p] && in_reach(self.places.position[p]) {
                let access = self
                    .derive_place_entrance(*place, allocator, network, graph)
                    .unwrap_or_default();
                assert_eq!(
                    BuildingEntranceKey::new(&self.places.access[p]),
                    BuildingEntranceKey::new(&access),
                    "bay {place:?} keeps a stale access",
                );
            }
        }
        let economy = car_components.filter(|_| self.pending_economy.is_empty());
        let foot = door_foot_graph(doors, allocator, network, graph);
        let table = self.place_table(foot, &chunks, reach, network, graph, economy);
        let bits = |list: &[ParkingCandidate]| -> Vec<_> {
            list.iter()
                .map(|c| (c.place, c.walk_s.to_bits(), c.from_door_s.to_bits()))
                .collect()
        };
        for &b in doors {
            let Some(entrance) = allocator.entrances.get(b) else {
                continue;
            };
            let handle = allocator.building_handles.handle(b).to_bits();
            let centre = &allocator.buildings[b];
            let scratch = table.door_candidates(
                entrance,
                [centre.center_x, centre.center_y],
                economy.is_some(),
                network,
                graph,
            );
            let live = self.candidates.get(&handle);
            assert_eq!(
                live.map_or(Vec::new(), |c| bits(&c.entries)),
                bits(&scratch.entries),
                "the door of building {b} keeps a stale list",
            );
            if let Some(live) = live.filter(|_| economy.is_some()) {
                assert_eq!(live.economy.components, scratch.economy.components, "door {b}");
                assert_eq!(live.economy.targets, scratch.economy.targets, "door {b}");
                assert_eq!(
                    live.economy.node_slack_m.to_bits(),
                    scratch.economy.node_slack_m.to_bits(),
                    "door {b}",
                );
            }
        }
    }

    /// Leaves `edges` to the frame after a road commit. O(edges log pending).
    pub(crate) fn defer_frontages(&mut self, edges: &std::collections::BTreeSet<usize>) {
        self.pending_frontages.extend(edges);
    }

    /// Whether a road commit's edges still wait for their publication. O(1).
    pub(crate) fn frontages_deferred(&self) -> bool {
        !self.pending_frontages.is_empty()
    }

    /// The edges waiting for their publication, leaving none. O(1).
    pub(crate) fn take_deferred_frontages(&mut self) -> std::collections::BTreeSet<usize> {
        std::mem::take(&mut self.pending_frontages)
    }

    /// O(1), version of published catchments and their economy reachability metadata.
    pub(crate) fn candidate_revision(&self) -> u64 {
        self.candidate_revision
    }

    /// O(1), read-only publication metadata; missing means no parking-backed driving option.
    pub(crate) fn economy_ends(&self, building: u64) -> Option<&ParkingEconomyEnds> {
        self.candidates.get(&building).map(|c| &c.economy)
    }

    /// O(1); deferred local metadata waits for the existing routing publication, never polls cars.
    pub(crate) fn economy_refresh_pending(&self) -> bool {
        !self.pending_economy.is_empty()
    }

    /// Completes only pending doors after CCH adoption, from one table of their neighbourhood.
    /// O(bays around them + their entries); no walking search or city-wide candidate scan. The
    /// caller supplies the existing car index.
    pub(crate) fn refresh_economy_metadata(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        components: &ModeComponentIndex,
    ) {
        let mut pending = std::mem::take(&mut self.pending_economy);
        pending.sort_unstable();
        pending.dedup();
        let doors: Vec<_> = pending
            .iter()
            .filter_map(|&b| allocator.building_handles.resolve(Handle::from_bits(b)))
            .collect();
        let built: Vec<_> = crate::thread_pools::install(|| {
            let (chunks, reach) = self.door_neighbourhood(&doors, allocator);
            let foot = walk::FootGraph::default();
            let table = self.place_table(foot, &chunks, reach, network, graph, Some(components));
            let index = table.place_index();
            pending
                .par_iter()
                .filter_map(|&building| {
                    let b = allocator
                        .building_handles
                        .resolve(Handle::from_bits(building))?;
                    let entries = &self.candidates.get(&building)?.entries;
                    let centre = &allocator.buildings[b];
                    let centre = [centre.center_x, centre.center_y];
                    Some((building, table.economy_of(entries, &index, centre, graph)))
                })
                .collect()
        });
        if !built.is_empty() {
            self.candidate_revision = self.candidate_revision.wrapping_add(1);
        }
        for (building, economy) in built {
            if let Some(candidates) = self.candidates.get_mut(&building) {
                candidates.economy = economy;
            }
        }
        pending.clear();
        self.pending_economy = pending;
    }

    /// Edit-time locality oracle, comparing geometry/time rather than handles remapped by setup.
    #[cfg(test)]
    pub(crate) fn candidate_products(&self, building: u64) -> Vec<([u32; 3], u32, u32)> {
        self.candidates
            .get(&building)
            .into_iter()
            .flat_map(|c| &c.entries)
            .map(|c| {
                let p = self
                    .place_handles
                    .resolve(c.place)
                    .expect("published candidate");
                (
                    self.places.position[p].map(f32::to_bits),
                    c.walk_s.to_bits(),
                    c.from_door_s.to_bits(),
                )
            })
            .collect()
    }

    /// O(k) scan to the first free fitting candidate; no route query, allocation or cap.
    pub(crate) fn first_free_candidate(&self, building: u64) -> Option<ParkingCandidate> {
        self.first_free_candidate_after_departure(building, Handle::NONE)
    }

    /// O(k) hypothetical return scan; only the caller's actual departure bay may be released.
    pub(crate) fn first_free_candidate_after_departure(
        &self,
        building: u64,
        freed: Handle<Place>,
    ) -> Option<ParkingCandidate> {
        self.candidates
            .get(&building)?
            .entries
            .iter()
            .copied()
            .find(|c| {
                self.place_handles.resolve(c.place).is_some_and(|p| {
                    self.places.active[p]
                        && (self.places.occupant[p].is_none() || c.place == freed)
                        && self.places.reservation[p].is_none()
                        && self.places.width[p] >= 1.8
                        && self.places.length[p] >= 4.5
                })
            })
    }

    /// O(k) lookup of the actual car's bay in the origin door's prepared catchment.
    pub(crate) fn walk_to_car_seconds(&self, building: u64, place: Handle<Place>) -> Option<f32> {
        self.candidates
            .get(&building)?
            .entries
            .iter()
            .find(|c| c.place == place)
            .map(|c| c.from_door_s)
            .filter(|&s| s <= WALK_LIMIT_S)
    }

    /// A kerbside bay's access on its own road side: entered from the traffic lanes beside it,
    /// or from either direction when a one-way road's lanes serve both kerbs. O(lanes on the
    /// edge), allocation-free.
    fn kerb_place_entrance(
        &self,
        p: usize,
        edge_idx: usize,
        right: bool,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<BuildingEntrance> {
        use crate::simulation::network::types::VehicleFrontageAccess;
        let edge = graph.get_edge(edge_idx).filter(|e| !e.deleted)?;
        network.lane_system.edge_lanes.get(&edge_idx)?;
        let [x, z] = self.places.path[p][0];
        let curb = Vector2::new(x, z);
        let pos = self.places.position[p];
        let serving = if right {
            edge.fwd_lanes
        } else {
            edge.bkw_lanes
        };
        let mut entrance = BuildingEntrance {
            edge_idx,
            side: if right { -1 } else { 1 },
            vehicle_frontage_access: if serving > 0 {
                VehicleFrontageAccess::SameSideOnly
            } else {
                VehicleFrontageAccess::BothSides
            },
            entrance_s_m: BuildingAllocator::project_point_to_polyline_s(
                &edge.physical_geometry,
                curb,
            ),
            door_pos: Vector2::new(pos[0], pos[2]),
            curb_pos: curb,
            ..BuildingEntrance::default()
        };
        BuildingAllocator::refresh_entrance_lanes(&mut entrance, graph, &network.lane_system);
        Some(entrance)
    }

    /// Bitwise geometry stamp for an economy endpoint, O(1), allocation-free. Topology and
    /// door revisions invalidate route caches separately; local bay moves/manoeuvres need this
    /// stamp even when the stable place handle survives an edit. Lane ids and flags are left
    /// out: they are derived live and change only with the lane topology. Derived identically
    /// on load.
    pub(crate) fn quote_geometry(&self, place: Handle<Place>) -> u64 {
        let Some(p) = self.place_handles.resolve(place) else {
            return 0;
        };
        let entrance = &self.places.access[p];
        let mut stamp = place.to_bits();
        for value in [
            entrance.edge_idx as u64,
            entrance.side as u64,
            entrance.entrance_s_m.to_bits() as u64,
            entrance.door_pos.x.to_bits() as u64,
            entrance.door_pos.y.to_bits() as u64,
            entrance.curb_pos.x.to_bits() as u64,
            entrance.curb_pos.y.to_bits() as u64,
            self.maneuver_seconds(place).to_bits() as u64,
        ] {
            stamp = crate::simulation::core::stable_hash64(stamp ^ value);
        }
        stamp
    }

    /// Copy of the driveway access prepared with the local publication, with lane ids derived
    /// from the current lanes: lane rebuilds renumber lanes far from the edited points that
    /// republish access. O(lanes on the access edge), allocation-free.
    pub(crate) fn place_entrance(
        &self,
        place: Handle<Place>,
        _allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<BuildingEntrance> {
        self.live_access(place, network, graph)
    }

    fn live_access(
        &self,
        place: Handle<Place>,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<BuildingEntrance> {
        let p = self.place_handles.resolve(place)?;
        let access = &self.places.access[p];
        if access.edge_idx == usize::MAX {
            return None;
        }
        let mut entrance = access.clone();
        BuildingAllocator::refresh_entrance_lanes(&mut entrance, graph, &network.lane_system);
        Some(entrance)
    }

    /// Uses the validated driveway station and the host's existing lane identities.
    /// O(host entrance copy + frontage lane projection), allocation-free.
    fn derive_place_entrance(
        &self,
        place: Handle<Place>,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<BuildingEntrance> {
        let p = self.place_handles.resolve(place)?;
        let building = match self.places.source[p] {
            PlaceSource::Building { building, .. } => building,
            PlaceSource::Kerb { edge, right } => {
                return self.kerb_place_entrance(p, edge as usize, right, network, graph);
            }
        };
        let host = allocator
            .building_handles
            .resolve(Handle::from_bits(building))?;
        let mut entrance = allocator.entrances.get(host)?.clone();
        let edge = graph.get_edge(entrance.edge_idx).filter(|e| !e.deleted)?;
        let path = self.places.path[p];
        let curb = Vector2::new(path[0][0], path[0][1]);
        entrance.entrance_s_m =
            BuildingAllocator::project_point_to_polyline_s(&edge.physical_geometry, curb);
        entrance.curb_pos = curb;
        let pos = self.places.position[p];
        entrance.door_pos = Vector2::new(pos[0], pos[2]);
        entrance.vehicle_frontage_access =
            crate::simulation::network::types::VehicleFrontageAccess::SameSideOnly;
        if entrance.side == -1 {
            entrance.car_lane_bkw = usize::MAX;
        } else {
            entrance.car_lane_fwd = usize::MAX;
        }
        network.lane_system.edge_lanes.get(&entrance.edge_idx)?;
        Some(entrance)
    }

    /// O(1), fixed swept-path length at driveway speed; physical timing includes this path.
    pub(crate) fn maneuver_seconds(&self, place: Handle<Place>) -> f32 {
        self.place_handles
            .resolve(place)
            .map_or(f32::INFINITY, |p| {
                self.places.path[p]
                    .windows(2)
                    .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
                    .sum::<f32>()
                    / crate::config::AGENT_DRIVEWAY_SPEED_MS
            })
    }
}

// The lanes every search from these doors can touch. O(lanes in their reach).
fn door_foot_graph(
    buildings: &[usize],
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> walk::FootGraph {
    let seeds = buildings
        .iter()
        .filter_map(|&b| allocator.entrances.get(b))
        .flat_map(|door| [door.foot_lane_fwd, door.foot_lane_bkw])
        .filter(|&lane| lane < network.lane_system.lanes.len())
        .collect();
    walk::FootGraph::around(seeds, network, graph)
}

fn road_point_of(access: &BuildingEntrance, graph: &RegionGraph) -> Option<[f32; 2]> {
    graph.get_edge(access.edge_idx).map(|edge| {
        let point = BuildingAllocator::sample_pos_on_edge(
            graph,
            access.edge_idx,
            access.entrance_s_m / edge.physical_length,
        );
        [point.x, point.y]
    })
}

impl ParkingSystem {
    /// O(owner places), edit-boundary invalidation points; no global supply scan.
    pub(crate) fn source_points(&self, building: u64) -> Vec<[f32; 2]> {
        self.building_sources
            .get(&building)
            .into_iter()
            .flatten()
            .flat_map(|s| self.sources.get(s).into_iter().flatten())
            .filter_map(|&h| self.place_handles.resolve(h))
            .map(|p| {
                let pos = self.places.position[p];
                [pos[0], pos[2]]
            })
            .collect()
    }

    /// O(1) expected, removes a destroyed destination's derived catchment.
    pub(crate) fn remove_destination_candidates(&mut self, building: u64) {
        if self.candidates.remove(&building).is_some() {
            self.candidate_revision = self.candidate_revision.wrapping_add(1);
        }
    }
}
