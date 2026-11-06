// SPDX-License-Identifier: GPL-2.0-only

//! Published per-door walking catchments, over the existing directed pedestrian lanes.

use super::*;
use crate::config::AGENT_WALK_SPEED_MS;
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::accessibility::ModeComponentIndex;
use crate::simulation::economy::agents::{MODE_CAR, tick::BuildingTripEnds};
use crate::simulation::network::types::TransitFlags;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph, lanes::LaneType};
use godot::prelude::Vector2;
use rayon::prelude::*;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

const WALK_LIMIT_S: f32 = 400.0 / AGENT_WALK_SPEED_MS;

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
            self.rebuild_candidates_in_pool(buildings, allocator, network, graph, car_components)
        })
    }

    fn rebuild_candidates_in_pool(
        &mut self,
        buildings: &[usize],
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        car_components: Option<&ModeComponentIndex>,
    ) -> Vec<[f32; 2]> {
        let mut places = Vec::new();
        for &b in buildings {
            let Some(door) = allocator.entrances.get(b) else {
                continue;
            };
            let (cx, cz) = chunk_at([door.door_pos.x, door.door_pos.y]);
            for z in cz - 1..=cz + 1 {
                for x in cx - 1..=cx + 1 {
                    if let Some(chunk) = self.chunks.get(&(x, z)) {
                        places.extend_from_slice(&chunk.places);
                    }
                }
            }
        }
        places.sort_unstable();
        places.dedup();
        for place in places {
            if self
                .place_handles
                .resolve(place)
                .is_none_or(|p| !self.places.active[p])
            {
                continue;
            }
            let access = self
                .derive_place_entrance(place, allocator, network, graph)
                .unwrap_or_default();
            if let Some(p) = self.place_handles.resolve(place) {
                if let Some(edge) = graph.get_edge(access.edge_idx) {
                    let point = BuildingAllocator::sample_pos_on_edge(
                        graph,
                        access.edge_idx,
                        access.entrance_s_m / edge.physical_length,
                    );
                    self.places.road_point[p] = [point.x, point.y];
                }
                self.places.access[p] = access;
            }
        }
        let built: Vec<_> = buildings
            .par_iter()
            .filter_map(|&b| {
                let entrance = allocator.entrances.get(b)?;
                let handle = allocator.building_handles.handle(b);
                if handle.is_none() {
                    return None;
                }
                let door = [entrance.door_pos.x, entrance.door_pos.y];
                let (cx, cz) = chunk_at(door);
                if !(cz - 1..=cz + 1).any(|z| {
                    (cx - 1..=cx + 1).any(|x| {
                        self.chunks
                            .get(&(x, z))
                            .is_some_and(|c| !c.places.is_empty())
                    })
                }) {
                    return Some((handle.to_bits(), door, DestinationCandidates::default()));
                }
                let forward = WalkField::build(entrance, false, network, graph);
                let reverse = WalkField::build(entrance, true, network, graph);
                let mut entries = Vec::new();
                for z in cz - 1..=cz + 1 {
                    for x in cx - 1..=cx + 1 {
                        let Some(chunk) = self.chunks.get(&(x, z)) else {
                            continue;
                        };
                        for &place in &chunk.places {
                            let Some(p) = self.place_handles.resolve(place) else {
                                continue;
                            };
                            if !self.places.active[p] {
                                continue;
                            }
                            let pos = self.places.position[p];
                            if (pos[0] - entrance.door_pos.x).hypot(pos[2] - entrance.door_pos.y)
                                > 400.0
                            {
                                continue;
                            }
                            let Some(access) =
                                self.place_entrance(place, allocator, network, graph)
                            else {
                                continue;
                            };
                            let walk_s = reverse.time_at(&access, network, graph);
                            let from_door_s = forward.time_at(&access, network, graph);
                            if walk_s <= WALK_LIMIT_S {
                                entries.push(ParkingCandidate {
                                    place,
                                    walk_s,
                                    from_door_s,
                                });
                            }
                        }
                    }
                }
                // Drive tie-break is the fixed, free-flow driveway-to-bay time, never live traffic.
                entries.sort_unstable_by(|a, b| {
                    a.walk_s
                        .total_cmp(&b.walk_s)
                        .then(
                            self.maneuver_seconds(a.place)
                                .total_cmp(&self.maneuver_seconds(b.place)),
                        )
                        .then(a.place.cmp(&b.place))
                });
                let economy =
                    car_components.map_or_else(ParkingEconomyEnds::default, |components| {
                        self.build_economy_ends(&entries, b, allocator, network, graph, components)
                    });
                Some((
                    handle.to_bits(),
                    door,
                    DestinationCandidates { entries, economy },
                ))
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

    /// Completes only pending doors after CCH adoption. O(sum(local candidates log candidates));
    /// no walking search or city-wide candidate scan. The caller supplies the existing car index.
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
        let built: Vec<_> = crate::thread_pools::install(|| {
            pending
                .par_iter()
                .filter_map(|&building| {
                    let b = allocator
                        .building_handles
                        .resolve(Handle::from_bits(building))?;
                    let entries = &self.candidates.get(&building)?.entries;
                    Some((
                        building,
                        self.build_economy_ends(entries, b, allocator, network, graph, components),
                    ))
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

    fn build_economy_ends(
        &self,
        entries: &[ParkingCandidate],
        building: usize,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        car_components: &ModeComponentIndex,
    ) -> ParkingEconomyEnds {
        let centre = &allocator.buildings[building];
        let mut components = Vec::new();
        let mut targets = Vec::new();
        for candidate in entries {
            let Some(access) = self.place_entrance(candidate.place, allocator, network, graph)
            else {
                continue;
            };
            components.extend_from_slice(
                car_components
                    .entrance_components(&access, graph, TransitFlags::CAR)
                    .as_slice(),
            );
            targets.extend(
                BuildingTripEnds::at_entrance(&access, false, network, graph)
                    .network_targets(MODE_CAR)
                    .into_iter()
                    .flatten(),
            );
        }
        components.sort_unstable();
        components.dedup();
        targets.sort_unstable();
        targets.dedup();
        let node_slack_m = targets
            .iter()
            .filter_map(|&(node, _)| graph.nodes().get(node as usize))
            .map(|node| (node.pos.x - centre.center_x).hypot(node.pos.z - centre.center_y))
            .fold(0.0, f32::max);
        ParkingEconomyEnds {
            components: components.into(),
            targets: targets.into_boxed_slice(),
            node_slack_m,
        }
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

// Sparse edit-time labels avoid a city-sized initialization for every local catchment.
struct WalkField {
    costs: HashMap<usize, f32>,
    seeds: [(usize, f32); 2],
    local_s: f32,
    reverse: bool,
}

impl WalkField {
    fn build(
        door: &BuildingEntrance,
        reverse: bool,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Self {
        let local_s = door.door_pos.distance_to(door.curb_pos) / AGENT_WALK_SPEED_MS;
        let seeds = foot_positions(door, network, graph);
        let mut field = Self {
            costs: HashMap::new(),
            seeds,
            local_s,
            reverse,
        };
        let mut heap = BinaryHeap::new();
        for (id, d) in seeds {
            let Some(lane) = network.lane_system.lanes.get(id) else {
                continue;
            };
            let cost = local_s + if reverse { d } else { lane.length - d } / AGENT_WALK_SPEED_MS;
            visit_adjacent(id, reverse, network, graph, |next| {
                relax(&mut field.costs, &mut heap, next, cost)
            });
        }
        while let Some(Reverse((bits, id))) = heap.pop() {
            let cost = f32::from_bits(bits);
            if field.costs.get(&id) != Some(&cost) {
                continue;
            }
            let lane = &network.lane_system.lanes[id];
            let next_cost = cost + lane.length / AGENT_WALK_SPEED_MS;
            if next_cost > WALK_LIMIT_S {
                continue;
            }
            visit_adjacent(id, reverse, network, graph, |next| {
                relax(&mut field.costs, &mut heap, next, next_cost)
            });
        }
        field
    }

    fn time_at(
        &self,
        target: &BuildingEntrance,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> f32 {
        let local_s = target.door_pos.distance_to(target.curb_pos) / AGENT_WALK_SPEED_MS;
        let mut best = f32::INFINITY;
        for (id, d) in foot_positions(target, network, graph) {
            let Some(lane) = network.lane_system.lanes.get(id) else {
                continue;
            };
            if let Some(&cost) = self.costs.get(&id) {
                best = best.min(
                    cost + if self.reverse { lane.length - d } else { d } / AGENT_WALK_SPEED_MS
                        + local_s,
                );
            }
            for (seed, start) in self.seeds {
                let along = if self.reverse { start - d } else { d - start };
                if seed == id && along >= 0.0 {
                    best = best.min(self.local_s + along / AGENT_WALK_SPEED_MS + local_s);
                }
            }
        }
        best
    }
}

fn relax(
    costs: &mut HashMap<usize, f32>,
    heap: &mut BinaryHeap<Reverse<(u32, usize)>>,
    id: usize,
    cost: f32,
) {
    if cost.is_finite() && cost <= WALK_LIMIT_S && costs.get(&id).is_none_or(|&old| cost < old) {
        costs.insert(id, cost);
        heap.push(Reverse((cost.to_bits(), id)));
    }
}

fn foot_positions(
    entrance: &BuildingEntrance,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> [(usize, f32); 2] {
    let point = graph.get_edge(entrance.edge_idx).map(|e| {
        BuildingAllocator::sample_pos_on_edge(
            graph,
            entrance.edge_idx,
            entrance.entrance_s_m / e.physical_length,
        )
    });
    [entrance.foot_lane_fwd, entrance.foot_lane_bkw].map(|id| {
        let d = point
            .zip(network.lane_system.lanes.get(id))
            .map_or(0.0, |(p, l)| {
                BuildingAllocator::project_point_to_polyline_s(&l.geometry, p)
            });
        (id, d)
    })
}

fn visit_adjacent(
    id: usize,
    reverse: bool,
    network: &TransitNetwork,
    graph: &RegionGraph,
    mut visit: impl FnMut(usize),
) {
    let lanes = &network.lane_system.lanes;
    let valid = |i: usize| {
        lanes.get(i).is_some_and(|l| {
            l.lane_type == LaneType::Foot
                && (l.edge_id == usize::MAX
                    || graph.get_edge(l.edge_id).is_some_and(|e| !e.deleted))
        })
    };
    if !valid(id) {
        return;
    }
    if !reverse {
        for &next in &lanes[id].next_lanes {
            if valid(next) {
                visit(next);
            }
        }
        return;
    }
    let lane = &lanes[id];
    let node = if lane.edge_id == usize::MAX {
        lane.node_id as u32
    } else {
        let Some(edge) = graph.get_edge(lane.edge_id) else {
            return;
        };
        if lane.is_fwd {
            edge.start_node
        } else {
            edge.end_node
        }
    };
    // Existing per-node and per-edge lane indices enumerate only this junction's predecessors.
    if let Some(connections) = network.lane_system.node_lanes.get(&(node as usize)) {
        for &candidate in connections {
            if valid(candidate) && lanes[candidate].next_lanes.contains(&id) {
                visit(candidate);
            }
        }
    }
    if (node as usize) < graph.node_adjacency_count() {
        for &edge in graph.node_adjacency(node) {
            if let Some(roads) = network.lane_system.edge_lanes.get(&edge) {
                for &candidate in roads {
                    if valid(candidate) && lanes[candidate].next_lanes.contains(&id) {
                        visit(candidate);
                    }
                }
            }
        }
    }
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
