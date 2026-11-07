// SPDX-License-Identifier: GPL-2.0-only

//! The door-independent half of a catchment rebuild: every bay's walking ends and economy ends,
//! derived once and read by each door it reaches.

use super::super::*;
use super::walk::{FootEnds, WALK_LIMIT_S, WalkField};
use super::{DestinationCandidates, ParkingCandidate, ParkingEconomyEnds};
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::accessibility::ModeComponentIndex;
use crate::simulation::economy::agents::{MODE_CAR, tick::BuildingTripEnds};
use crate::simulation::network::types::TransitFlags;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use rayon::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

// Straight-line reach of a door; the walking limit then decides.
const DOOR_REACH_M: f32 = 400.0;

// Squared distances beyond the rounding of either formula, so only a bay this close to the limit
// pays for the exact `hypot` test.
const DOOR_REACH_SURE_SQ: f32 = 399.5 * 399.5;
const DOOR_REACH_OUT_SQ: f32 = 400.5 * 400.5;

#[inline]
fn within_door_reach(dx: f32, dz: f32) -> bool {
    let squared = dx * dx + dz * dz;
    squared < DOOR_REACH_SURE_SQ
        || (squared <= DOOR_REACH_OUT_SQ && dx.hypot(dz) <= DOOR_REACH_M)
}

struct PlaceRow {
    place: Handle<Place>,
    foot: FootEnds,
    maneuver_s: f32,
    economy: u32,
}

// The component labels and target pairs bays can share, one row per distinct pair in a chunk.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct EconomyRow {
    components: ([u32; 2], usize),
    targets: [Option<(u32, usize)>; 2],
}

/// Rows of every usable bay in a set of chunks, grouped by chunk so a door walks nine contiguous
/// runs. Positions sit apart from the rows: most bays a door looks at are out of its reach.
pub(super) struct PlaceTable {
    chunks: HashMap<(i32, i32), Range<usize>>,
    positions: Vec<[f32; 2]>,
    rows: Vec<PlaceRow>,
    economy: Vec<EconomyRow>,
}

// One door's search scratch, per worker: a door's lists are built without touching the city.
#[derive(Default)]
struct DoorScratch {
    fields: [WalkField; 2],
    found: Vec<(ParkingCandidate, f32, u32)>,
    economy_seen: Vec<u32>,
    economy_stamp: u32,
    economy_ids: Vec<u32>,
}

thread_local! {
    static SCRATCH: RefCell<DoorScratch> = RefCell::default();
}

impl ParkingSystem {
    /// Rows of the bays in `chunks` that lie inside `bounds` (`[min_x, min_z, max_x, max_z]`).
    /// O(bays there + rows × (lane projection + trip ends)), parallel by chunk.
    pub(super) fn place_table(
        &self,
        chunks: &[(i32, i32)],
        bounds: [f32; 4],
        network: &TransitNetwork,
        graph: &RegionGraph,
        car_components: Option<&ModeComponentIndex>,
    ) -> PlaceTable {
        let parts: Vec<_> = chunks
            .par_iter()
            .map(|&key| self.chunk_rows(key, bounds, network, graph, car_components))
            .collect();
        let total = parts.iter().map(|part| part.1.len()).sum();
        let mut table = PlaceTable {
            chunks: HashMap::with_capacity(parts.len()),
            positions: Vec::with_capacity(total),
            rows: Vec::with_capacity(total),
            economy: Vec::new(),
        };
        for (key, (positions, rows, economy)) in chunks.iter().zip(parts) {
            let first = table.rows.len();
            let offset = table.economy.len() as u32;
            table.positions.extend(positions);
            table.rows.extend(rows.into_iter().map(|mut row| {
                row.economy += offset;
                row
            }));
            table.economy.extend(economy);
            table.chunks.insert(*key, first..table.rows.len());
        }
        table
    }

    #[allow(clippy::type_complexity)]
    fn chunk_rows(
        &self,
        key: (i32, i32),
        bounds: [f32; 4],
        network: &TransitNetwork,
        graph: &RegionGraph,
        car_components: Option<&ModeComponentIndex>,
    ) -> (Vec<[f32; 2]>, Vec<PlaceRow>, Vec<EconomyRow>) {
        let mut positions = Vec::new();
        let mut rows = Vec::new();
        let mut economy = Vec::new();
        let mut interned = HashMap::new();
        for &place in self.chunks.get(&key).map_or(&[][..], |c| &c.places) {
            let Some(p) = self.place_handles.resolve(place) else {
                continue;
            };
            let [x, _, z] = self.places.position[p];
            if !self.places.active[p]
                || x < bounds[0]
                || z < bounds[1]
                || x > bounds[2]
                || z > bounds[3]
            {
                continue;
            }
            let Some(access) = self.live_access(place, network, graph) else {
                continue;
            };
            let id = car_components.map_or(0, |components| {
                let row = economy_row(&access, components, network, graph);
                *interned.entry(row).or_insert_with(|| {
                    economy.push(row);
                    economy.len() as u32 - 1
                })
            });
            positions.push([x, z]);
            rows.push(PlaceRow {
                place,
                foot: FootEnds::of(&access, network, graph),
                maneuver_s: self.maneuver_seconds(place),
                economy: id,
            });
        }
        (positions, rows, economy)
    }
}

fn economy_row(
    access: &BuildingEntrance,
    components: &ModeComponentIndex,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> EconomyRow {
    EconomyRow {
        components: components
            .entrance_components(access, graph, TransitFlags::CAR)
            .raw_parts(),
        targets: BuildingTripEnds::at_entrance(access, false, network, graph)
            .network_targets(MODE_CAR),
    }
}

impl PlaceTable {
    /// One door's catchment, sorted by walking time, then the fixed free-flow driveway-to-bay
    /// time (never live traffic), then the place. O(walk searches + bays in nine chunks +
    /// reached bays log them); the rows did the rest once for every door.
    pub(super) fn door_candidates(
        &self,
        entrance: &BuildingEntrance,
        centre: [f32; 2],
        with_economy: bool,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> DestinationCandidates {
        let door = [entrance.door_pos.x, entrance.door_pos.y];
        let (cx, cz) = chunk_at(door);
        if !(cz - 1..=cz + 1)
            .any(|z| (cx - 1..=cx + 1).any(|x| self.chunks.get(&(x, z)).is_some_and(|r| !r.is_empty())))
        {
            return DestinationCandidates::default();
        }
        SCRATCH.with_borrow_mut(|scratch| {
            let DoorScratch { fields, found, .. } = scratch;
            let foot = FootEnds::of(entrance, network, graph);
            let [forward, reverse] = fields;
            forward.search(foot, false, network, graph);
            reverse.search(foot, true, network, graph);
            found.clear();
            for z in cz - 1..=cz + 1 {
                for x in cx - 1..=cx + 1 {
                    let Some(range) = self.chunks.get(&(x, z)) else {
                        continue;
                    };
                    for i in range.clone() {
                        let [px, pz] = self.positions[i];
                        if !within_door_reach(px - door[0], pz - door[1]) {
                            continue;
                        }
                        let row = &self.rows[i];
                        let walk_s = reverse.time_at(&row.foot);
                        if walk_s <= WALK_LIMIT_S {
                            let candidate = ParkingCandidate {
                                place: row.place,
                                walk_s,
                                from_door_s: forward.time_at(&row.foot),
                            };
                            found.push((candidate, row.maneuver_s, i as u32));
                        }
                    }
                }
            }
            found.sort_unstable_by(|a, b| {
                a.0.walk_s
                    .total_cmp(&b.0.walk_s)
                    .then(a.1.total_cmp(&b.1))
                    .then(a.0.place.cmp(&b.0.place))
            });
            let economy = if with_economy {
                self.economy_ends(scratch, centre, graph)
            } else {
                ParkingEconomyEnds::default()
            };
            DestinationCandidates {
                entries: scratch.found.iter().map(|f| f.0).collect(),
                economy,
            }
        })
    }

    // The sorted distinct ends of the rows in `scratch.found`, O(found + distinct log distinct).
    fn economy_ends(
        &self,
        scratch: &mut DoorScratch,
        centre: [f32; 2],
        graph: &RegionGraph,
    ) -> ParkingEconomyEnds {
        if scratch.economy_seen.len() < self.economy.len() {
            scratch.economy_seen.resize(self.economy.len(), 0);
        }
        scratch.economy_stamp = scratch.economy_stamp.wrapping_add(1);
        if scratch.economy_stamp == 0 {
            scratch.economy_seen.fill(0);
            scratch.economy_stamp = 1;
        }
        scratch.economy_ids.clear();
        for &(_, _, i) in &scratch.found {
            let id = self.rows[i as usize].economy;
            let seen = &mut scratch.economy_seen[id as usize];
            if *seen != scratch.economy_stamp {
                *seen = scratch.economy_stamp;
                scratch.economy_ids.push(id);
            }
        }
        let mut components = Vec::new();
        let mut targets = Vec::new();
        for &id in &scratch.economy_ids {
            let row = &self.economy[id as usize];
            components.extend_from_slice(&row.components.0[..row.components.1]);
            targets.extend(row.targets.into_iter().flatten());
        }
        finish_economy_ends(components, targets, centre, graph)
    }
}

/// Sorts and dedups one door's ends and measures them from `centre`. O(ends log ends).
pub(super) fn finish_economy_ends(
    mut components: Vec<u32>,
    mut targets: Vec<(u32, usize)>,
    centre: [f32; 2],
    graph: &RegionGraph,
) -> ParkingEconomyEnds {
    components.sort_unstable();
    components.dedup();
    targets.sort_unstable();
    targets.dedup();
    let node_slack_m = targets
        .iter()
        .filter_map(|&(node, _)| graph.nodes().get(node as usize))
        .map(|node| (node.pos.x - centre[0]).hypot(node.pos.z - centre[1]))
        .fold(0.0, f32::max);
    ParkingEconomyEnds {
        components: components.into(),
        targets: targets.into_boxed_slice(),
        node_slack_m,
    }
}
