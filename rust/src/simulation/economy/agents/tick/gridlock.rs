// SPDX-License-Identifier: GPL-2.0-only

//! The wait-for cycle check, its detour and the gridlock report (`TRAFFIC-14`,
//! `docs/traffic.md#gridlock-check-traffic-14`).
//!
//! A lane whose head car stands at the lane's end, unable to enter the lane it needs next, waits
//! for that lane: the connector or direct lane its route enters. Every lane waits for at most
//! one lane, so the wait-for graph is functional and its cycles are found by one stamped walk
//! per waiting lane. A cycle is confirmed when two consecutive checks find the same lanes in it
//! and no car in it has moved. For each confirmed cycle the road-lane head that has waited
//! longest (agent index breaks ties) and has another way out that its entry test passes now gets
//! one route query from that exit to its destination; when it succeeds, its remaining route is
//! replaced and the car leaves through the normal claim on a later tick. A cycle with no such
//! head is genuine saturation: nothing is moved or removed, and its junctions are reported.
//!
//! Reads the lane buckets the last tick left, never a rebuilt occupancy. Per check:
//! O(lanes / p) for the head scan, which also stamps the waiting heads, O(lanes) for the serial
//! pass over the flat waited-for array that starts the walks, O(waiting lanes) for the walks,
//! O(cars in cycles) for the fingerprints and one route query per detour. The per-lane arrays
//! are sized when the lane topology changes, never per check, and the scratch keeps its
//! capacity, so a steady state allocates nothing; a detour allocates its new route.

use super::super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_LANE_ATTACH_PENDING, MODE_CAR, TRANSIT_INTERSECTION,
    TRANSIT_NETWORK,
};
use super::lane_nav::{
    NextTarget, connector_targets_edge, connector_targets_lane, direct_vehicle_lane_to_edge,
    direct_vehicle_lane_to_lane, lane_terminal_node, planned_next_target,
};
use super::planning::{BuiltNetworkReplan, plan_border_network_replan, plan_network_replan};
use super::traffic::lane_entry_slot_clear;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::data::{AgentSystem, AgentVec};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneType;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

const NONE: u32 = u32::MAX;
// A waiting head is placed at its lane's end exactly; this tolerance only absorbs rounding.
const AT_END_M: f32 = 0.05;
// The claim keeps only the shortest connectors to an edge (`collect_connector_lanes_to_edge`).
const SHORTEST_CONNECTOR_EPS_M: f32 = 0.001;
// Lanes per Rayon task of the head scan.
const SCAN_CHUNK: usize = 4_096;

/// What the last check found and did, for the bridge and the audits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GridlockReport {
    /// Checks run since the checker was created or loaded with its state.
    pub(crate) checks: u64,
    /// Lanes whose head waited for another lane at the last check.
    pub(crate) waiting_lanes: u32,
    /// Wait-for cycles at the last check.
    pub(crate) cycles: u32,
    /// Cycles the last check confirmed (unchanged since the check before).
    pub(crate) confirmed: u32,
    /// Confirmed cycles the last check found no detour for: genuine saturation.
    pub(crate) saturated: u32,
    /// Detours taken, over all checks.
    pub(crate) detours: u64,
    /// Saturated cycles reported, summed over all checks.
    pub(crate) saturation_reports: u64,
    /// Junction nodes of the last check's saturated cycles, ascending and unique.
    pub(crate) junctions: Vec<u32>,
}

/// The checker's state that carries from one check to the next, as the save file's
/// `gridlock_state` row stores it (save format 109): the report, the stamps of the lanes whose
/// head waited at the last check and that check's cycles, never the per-lane arrays. Lane ids
/// are the live ones at a save and the loaded ones after [`Self::remap_lanes`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SavedGridlock {
    /// The last check's report.
    pub(crate) report: GridlockReport,
    /// `(lane, head agent handle bits, slot it was first seen waiting)` for every lane that
    /// waited at the last check, ascending by lane.
    pub(crate) heads: Vec<(u32, u64, u64)>,
    /// Lanes in each of the last check's cycles.
    pub(crate) cycle_lengths: Vec<u32>,
    /// `(lane, cars on it)` per cycle lane, cycle by cycle, each cycle from its lowest lane.
    pub(crate) cycle_lanes: Vec<(u32, u32)>,
    /// `(agent handle bits, lane distance bits)` per car of the cycle lanes, lane by lane in
    /// bucket order.
    pub(crate) cycle_cars: Vec<(u64, u32)>,
}

impl SavedGridlock {
    /// Maps every saved lane id through `remap` (saved id to loaded id). A lane the loaded
    /// network has not got fails the load. O(heads + cycle lanes), plus the sort of the heads.
    pub(crate) fn remap_lanes(
        &mut self,
        remap: impl Fn(u32) -> Option<u32>,
    ) -> Result<(), &'static str> {
        let missing = "saved gridlock lane has no loaded lane";
        for (lane, _, _) in &mut self.heads {
            *lane = remap(*lane).ok_or(missing)?;
        }
        self.heads.sort_unstable_by_key(|&(lane, _, _)| lane);
        for (lane, _) in &mut self.cycle_lanes {
            *lane = remap(*lane).ok_or(missing)?;
        }
        Ok(())
    }
}

// Cycles of one check, flat: each cycle's lanes in canonical order (starting at its lowest lane)
// and its cars, `(agent handle bits, lane distance bits)` lane by lane in bucket order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CycleSet {
    lanes: Vec<u32>,
    lane_ends: Vec<u32>,
    cars: Vec<(u64, u32)>,
    // Per entry of `lanes`: the end of that lane's cars in `cars`.
    lane_car_ends: Vec<u32>,
}

impl CycleSet {
    fn clear(&mut self) {
        self.lanes.clear();
        self.lane_ends.clear();
        self.cars.clear();
        self.lane_car_ends.clear();
    }

    fn len(&self) -> usize {
        self.lane_ends.len()
    }

    fn lane_range(&self, cycle: usize) -> std::ops::Range<usize> {
        let start = cycle
            .checked_sub(1)
            .map_or(0, |prev| self.lane_ends[prev] as usize);
        start..self.lane_ends[cycle] as usize
    }

    fn lanes(&self, cycle: usize) -> &[u32] {
        &self.lanes[self.lane_range(cycle)]
    }

    fn cars(&self, cycle: usize) -> &[(u64, u32)] {
        let lanes = self.lane_range(cycle);
        let start = lanes
            .start
            .checked_sub(1)
            .map_or(0, |prev| self.lane_car_ends[prev] as usize);
        &self.cars[start..self.lane_car_ends[lanes.end - 1] as usize]
    }

    // Appends one cycle given as `(lane, cars on it)` from `lanes` and the cars from `cars`,
    // rotated to start at its lowest lane. O(cycle lanes + cycle cars).
    fn push_rotated(&mut self, lanes: &[(u32, u32)], cars: &[(u64, u32)]) {
        let lowest = (0..lanes.len()).min_by_key(|&k| lanes[k].0).unwrap_or(0);
        let mut offsets = Vec::with_capacity(lanes.len());
        let mut end = 0;
        for &(_, count) in lanes {
            offsets.push(end);
            end += count as usize;
        }
        for k in (lowest..lanes.len()).chain(0..lowest) {
            let (lane, count) = lanes[k];
            self.lanes.push(lane);
            self.cars
                .extend_from_slice(&cars[offsets[k]..offsets[k] + count as usize]);
            self.lane_car_ends.push(self.cars.len() as u32);
        }
        self.lane_ends.push(self.lanes.len() as u32);
    }

    // Whether `other` holds a cycle with the same lanes and the same cars at the same places.
    fn contains(&self, lanes: &[u32], cars: &[(u64, u32)]) -> bool {
        (0..self.len()).any(|cycle| self.lanes(cycle) == lanes && self.cars(cycle) == cars)
    }
}

/// The checker: per-lane arrays sized to the lane topology plus the previous check's cycles.
#[derive(Default)]
pub(crate) struct GridlockChecker {
    report: GridlockReport,
    // Lane topology the arrays describe: `(lane count, LaneSystem::revision)`.
    bound: Option<(usize, u64)>,
    // Per lane: the lane its head waits for at this check, or `NONE`.
    out: Vec<u32>,
    // Per lane: the check that last walked it, and the walk's start lane.
    walk_check: Vec<u64>,
    walk_start: Vec<u32>,
    // Per lane: the waiting head seen at the last check that found the lane waiting, that check,
    // and the slot the head was first seen waiting.
    head: Vec<u64>,
    head_check: Vec<u64>,
    head_since: Vec<u64>,
    // Saved heads waiting to be placed into the arrays once the loaded lanes are bound.
    restored_heads: Vec<(u32, u64, u64)>,
    cycles: CycleSet,
    previous: CycleSet,
    // Confirmed cycle indices of this check, and one cycle's detour candidates
    // `(waiting since, agent, lane)`.
    confirmed: Vec<u32>,
    candidates: Vec<(u64, usize, usize)>,
}

// The lane a car on `from` enters next to reach `target`, as the movement step chooses it.
enum Entry {
    // An entry passes its test now.
    Free,
    // Every candidate is blocked; the lane that blocks the first candidate.
    Blocked(usize),
    // No lane leads there.
    Missing,
}

impl GridlockChecker {
    /// What the last check found and did.
    pub(crate) fn report(&self) -> &GridlockReport {
        &self.report
    }

    /// The state a save stores for `network`, the lanes the save writes; [`Self::restore`]
    /// continues from it. A checker bound to other lanes (a topology edit since the last check)
    /// saves no heads and no cycles: its next check would forget them. O(lanes) for the head
    /// stamps, O(heads + cycle lanes + cycle cars) for the copy.
    pub(crate) fn save(&self, network: &TransitNetwork) -> SavedGridlock {
        let mut saved = SavedGridlock {
            report: self.report.clone(),
            ..SavedGridlock::default()
        };
        let current = (
            network.lane_system.lanes.len(),
            network.lane_system.revision(),
        );
        let check = self.report.checks;
        match self.bound {
            Some(bound) if bound != current => return saved,
            Some(_) => {
                saved.heads = (0..self.head.len())
                    .filter(|&lane| self.head_check[lane] == check && check > 0)
                    .map(|lane| (lane as u32, self.head[lane], self.head_since[lane]))
                    .collect();
            }
            None => saved.heads = self.restored_heads.clone(),
        }
        let cycles = &self.previous;
        for cycle in 0..cycles.len() {
            let lanes = cycles.lane_range(cycle);
            saved.cycle_lengths.push(lanes.len() as u32);
            for k in lanes {
                let start = k
                    .checked_sub(1)
                    .map_or(0, |prev| cycles.lane_car_ends[prev]);
                saved
                    .cycle_lanes
                    .push((cycles.lanes[k], cycles.lane_car_ends[k] - start));
            }
        }
        saved.cycle_cars = cycles.cars.clone();
        saved
    }

    /// A checker that continues from `saved`, each cycle rotated back to start at its lowest
    /// lane (a load may renumber lanes); the loaded lanes are bound by
    /// [`Self::bind_loaded_lanes`] before the next check. Fails when the cycle counts do not
    /// add up. O(heads + cycle lanes + cycle cars).
    pub(crate) fn restore(saved: SavedGridlock) -> Result<Self, &'static str> {
        let bad = "saved gridlock cycles do not add up";
        let lane_total: usize = saved.cycle_lengths.iter().map(|&n| n as usize).sum();
        let car_total: usize = saved.cycle_lanes.iter().map(|&(_, n)| n as usize).sum();
        if lane_total != saved.cycle_lanes.len() || car_total != saved.cycle_cars.len() {
            return Err(bad);
        }
        let mut previous = CycleSet::default();
        let (mut lane, mut car) = (0, 0);
        for &length in &saved.cycle_lengths {
            let lanes = &saved.cycle_lanes[lane..lane + length as usize];
            let cars: usize = lanes.iter().map(|&(_, n)| n as usize).sum();
            if lanes.is_empty() {
                return Err(bad);
            }
            previous.push_rotated(lanes, &saved.cycle_cars[car..car + cars]);
            lane += lanes.len();
            car += cars;
        }
        Ok(Self {
            report: saved.report,
            restored_heads: saved.heads,
            previous,
            ..Self::default()
        })
    }

    /// Binds the lanes a load built to the restored state: the saved lane ids describe them, so
    /// the arrays are sized and the saved heads placed without the reset a topology change
    /// makes. O(lanes).
    pub(crate) fn bind_loaded_lanes(&mut self, network: &TransitNetwork) {
        let lane_count = network.lane_system.lanes.len();
        self.size_arrays(lane_count);
        self.bound = Some((lane_count, network.lane_system.revision()));
        let check = self.report.checks;
        for (lane, head, since) in std::mem::take(&mut self.restored_heads) {
            let lane = lane as usize;
            if lane < lane_count {
                self.head[lane] = head;
                self.head_check[lane] = check;
                self.head_since[lane] = since;
            }
        }
    }

    // Resizes the per-lane arrays after a topology change and forgets what referred to the
    // old lane ids. O(lanes), only when the topology changed.
    fn bind(&mut self, network: &TransitNetwork) {
        let key = (
            network.lane_system.lanes.len(),
            network.lane_system.revision(),
        );
        if self.bound == Some(key) {
            return;
        }
        if self.bound.is_none() && !self.restored_heads.is_empty() {
            self.bind_loaded_lanes(network);
            return;
        }
        self.size_arrays(key.0);
        self.head_check.fill(u64::MAX);
        self.previous.clear();
        self.restored_heads.clear();
        self.bound = Some(key);
    }

    fn size_arrays(&mut self, lane_count: usize) {
        self.out.clear();
        self.out.resize(lane_count, NONE);
        self.walk_check.clear();
        self.walk_check.resize(lane_count, u64::MAX);
        self.walk_start.clear();
        self.walk_start.resize(lane_count, NONE);
        self.head.clear();
        self.head.resize(lane_count, u64::MAX);
        self.head_check.clear();
        self.head_check.resize(lane_count, u64::MAX);
        self.head_since.clear();
        self.head_since.resize(lane_count, 0);
    }
}

impl AgentSystem {
    /// One gridlock check at absolute slot `now` (`TRAFFIC-14`, the `gridlock check` fixed slot):
    /// finds the wait-for cycles in the lane buckets the last tick left, detours one head of
    /// each confirmed cycle that has a free way out, and reports the junctions of the confirmed
    /// cycles that have none. Skipped while the bucket snapshot does not describe the current
    /// lanes (a topology edit since the last tick). Cost in the module header.
    pub(crate) fn check_gridlock(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        now: u64,
    ) {
        let lane_count = network.lane_system.lanes.len();
        if !self.lane_bucket_snapshot_valid
            || self.lane_bucket_snapshot_lane_count != lane_count
            || self.lane_buckets.len() < lane_count
        {
            return;
        }
        self.handles.register_appended(self.agents.len());
        self.gridlock.bind(network);
        let check = self.gridlock.report.checks + 1;
        self.find_wait_for_cycles(network, graph, now, check);
        self.act_on_confirmed_cycles(allocator, network, graph);
        let checker = &mut self.gridlock;
        std::mem::swap(&mut checker.previous, &mut checker.cycles);
        checker.report.checks = check;
    }

    // Fills `out` and updates the head stamps in one parallel pass, walks the wait-for graph into
    // `cycles` and marks the cycles the previous check also held.
    fn find_wait_for_cycles(
        &mut self,
        network: &TransitNetwork,
        graph: &RegionGraph,
        now: u64,
        check: u64,
    ) {
        let agents = &self.agents;
        let buckets = &self.lane_buckets;
        let handles = &self.handles;
        let checker = &mut self.gridlock;
        checker
            .out
            .par_chunks_mut(SCAN_CHUNK)
            .zip(checker.head.par_chunks_mut(SCAN_CHUNK))
            .zip(checker.head_check.par_chunks_mut(SCAN_CHUNK))
            .zip(checker.head_since.par_chunks_mut(SCAN_CHUNK))
            .enumerate()
            .for_each(|(chunk, (((out, head), head_check), head_since))| {
                for offset in 0..out.len() {
                    let lane = chunk * SCAN_CHUNK + offset;
                    out[offset] = waited_for(agents, lane, network, graph, buckets);
                    if out[offset] == NONE {
                        continue;
                    }
                    // A waiting lane has a head; its stamp carries while the same car waits.
                    let car = buckets[lane].last().map_or(usize::MAX, |entry| entry.1);
                    let bits = handles.handle(car).to_bits();
                    if !(check > 1 && head_check[offset] == check - 1 && head[offset] == bits) {
                        head_since[offset] = now;
                    }
                    head[offset] = bits;
                    head_check[offset] = check;
                }
            });

        let mut waiting = 0;
        checker.cycles.clear();
        checker.confirmed.clear();
        // Walks start in ascending lane order, so the cycles' order is the lanes'.
        for start in 0..checker.out.len() {
            if checker.out[start] == NONE {
                continue;
            }
            waiting += 1;
            if checker.walk_check[start] == check {
                continue;
            }
            let mut lane = start;
            while checker.out[lane] != NONE {
                if checker.walk_check[lane] == check {
                    if checker.walk_start[lane] == start as u32 {
                        record_cycle(checker, lane, buckets, &self.handles);
                    }
                    break;
                }
                checker.walk_check[lane] = check;
                checker.walk_start[lane] = start as u32;
                lane = checker.out[lane] as usize;
            }
        }
        for cycle in 0..checker.cycles.len() {
            if checker
                .previous
                .contains(checker.cycles.lanes(cycle), checker.cycles.cars(cycle))
            {
                checker.confirmed.push(cycle as u32);
            }
        }
        let report = &mut checker.report;
        report.waiting_lanes = waiting;
        report.cycles = checker.cycles.len() as u32;
        report.confirmed = checker.confirmed.len() as u32;
    }

    // For each confirmed cycle in order, detours its first eligible road-lane head, or reports
    // its junctions when it has none.
    fn act_on_confirmed_cycles(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let lanes = &network.lane_system.lanes;
        self.gridlock.report.saturated = 0;
        self.gridlock.report.junctions.clear();
        for k in 0..self.gridlock.confirmed.len() {
            let cycle = self.gridlock.confirmed[k] as usize;
            let checker = &mut self.gridlock;
            checker.candidates.clear();
            for &lane in checker.cycles.lanes(cycle) {
                let lane = lane as usize;
                if lanes[lane].edge_id == usize::MAX {
                    // Connector heads are committed to their turn and never detour.
                    continue;
                }
                if let Some(&(_, head)) = self.lane_buckets[lane].last() {
                    checker
                        .candidates
                        .push((checker.head_since[lane], head, lane));
                }
            }
            // Waiting longest first; the agent index breaks ties (one head per lane).
            checker.candidates.sort_unstable();
            let mut detoured = false;
            for c in 0..self.gridlock.candidates.len() {
                let (_, head, lane) = self.gridlock.candidates[c];
                if self.try_detour(head, lane, allocator, network, graph) {
                    detoured = true;
                    break;
                }
            }
            let checker = &mut self.gridlock;
            if detoured {
                checker.report.detours += 1;
                continue;
            }
            checker.report.saturated += 1;
            checker.report.saturation_reports += 1;
            for &lane in checker.cycles.lanes(cycle) {
                let lane = lane as usize;
                let junction = if lanes[lane].edge_id == usize::MAX {
                    Some(lanes[lane].node_id as u32)
                } else {
                    lane_terminal_node(lane, network, graph)
                };
                checker.report.junctions.extend(junction);
            }
        }
        let junctions = &mut self.gridlock.report.junctions;
        junctions.sort_unstable();
        junctions.dedup();
    }

    // Detours head `i` of road lane `lane` when another way out of its junction passes its
    // entry test now and one route query from that exit's end to the car's destination
    // succeeds. The ways out are taken in the lane's successor order; only the first free one
    // is queried.
    fn try_detour(
        &mut self,
        i: usize,
        lane: usize,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let agents = &self.agents;
        let lanes = &network.lane_system.lanes;
        let Some(planned) = head_target(agents, i, lane, network, graph) else {
            return false;
        };
        let planned_edge = match planned {
            NextTarget::Edge(edge) => edge,
            NextTarget::Lane(target) => lanes[target].edge_id,
        };
        let Some(junction) = lane_terminal_node(lane, network, graph) else {
            return false;
        };
        let way_out = lanes[lane].next_lanes.iter().find_map(|&succ| {
            let next = &lanes[succ];
            if next.lane_type != LaneType::Vehicle {
                return None;
            }
            let exit = if next.edge_id == usize::MAX {
                *next.next_lanes.first()?
            } else {
                succ
            };
            let exit_edge = lanes[exit].edge_id;
            (exit_edge != planned_edge
                && exit_edge != usize::MAX
                && matches!(
                    entry_state(
                        lane,
                        NextTarget::Edge(exit_edge),
                        network,
                        &self.lane_buckets
                    ),
                    Entry::Free
                ))
            .then_some(exit)
        });
        let Some(exit) = way_out else {
            return false;
        };
        let exit_edge = lanes[exit].edge_id;
        let Some(exit_end) = lane_terminal_node(exit, network, graph) else {
            return false;
        };
        let replan = if (agents.access_flags[i] & ACCESS_FREIGHT_BORDER_DESTINATION) != 0 {
            plan_border_network_replan(
                exit_end,
                exit_edge,
                agents.freight_target_border_node[i],
                graph,
                network,
                &self.pathfind_count,
            )
        } else if agents.target_building[i] != usize::MAX {
            plan_network_replan(
                exit_end,
                exit_edge,
                agents.target_building[i],
                agents.transit_mode[i],
                agents.access_flags[i],
                allocator,
                network,
                graph,
                &self.pathfind_count,
                None,
            )
            .ok()
        } else {
            None
        };
        let Some(replan) = replan else {
            return false;
        };
        if replan
            .current_path
            .first()
            .is_some_and(|&first| first != exit_end)
        {
            return false;
        }
        self.replace_route_through(i, junction, exit_end, replan);
        true
    }

    // Replaces car `i`'s remaining route by `junction`, the detour's exit end and the route the
    // query planned from there. Mirrors `apply_network_replan`, which works on the movement
    // pass's raw slices, but leaves the path index on the junction, the lane's terminal node,
    // so the lane end, the speed pass and the claim all read the exit's edge as the next one.
    fn replace_route_through(
        &mut self,
        i: usize,
        junction: u32,
        exit_end: u32,
        replan: BuiltNetworkReplan,
    ) {
        let mut path = Vec::with_capacity(replan.current_path.len() + 2);
        path.push(junction);
        if replan.current_path.is_empty() {
            path.push(exit_end);
        } else {
            path.extend_from_slice(&replan.current_path);
        }
        let agents = &mut self.agents;
        agents.current_path[i] = path;
        agents.current_path_index[i] = 0;
        agents.planned_detach_node[i] = replan.planned_detach_node;
        agents.planned_detach_lane_id[i] = replan.planned_detach_lane_id as u32;
        agents.planned_detach_lane_d[i] = replan.planned_detach_lane_d;
        let hold = agents.access_flags[i] & ACCESS_LANE_ATTACH_PENDING;
        agents.access_flags[i] = replan.access_flags | hold;
        agents.next_replan_time[i] = 0.0;
        agents.network_replan_failures[i] = 0;
    }
}

// Appends the cycle through `lane` to `checker.cycles`, rotated to start at its lowest lane,
// with every car on its lanes. O(cycle lanes + their cars).
fn record_cycle(
    checker: &mut GridlockChecker,
    lane: usize,
    buckets: &[Vec<(f32, usize)>],
    handles: &crate::simulation::core::handles::HandleTable<
        crate::simulation::economy::agents::data::Agent,
    >,
) {
    let cycles = &mut checker.cycles;
    let first = cycles.lanes.len();
    let mut current = lane;
    loop {
        cycles.lanes.push(current as u32);
        current = checker.out[current] as usize;
        if current == lane {
            break;
        }
    }
    let lowest = (first..cycles.lanes.len())
        .min_by_key(|&k| cycles.lanes[k])
        .unwrap_or(first);
    cycles.lanes[first..].rotate_left(lowest - first);
    cycles.lane_ends.push(cycles.lanes.len() as u32);
    for k in first..cycles.lanes.len() {
        for &(d, agent) in &buckets[cycles.lanes[k] as usize] {
            cycles
                .cars
                .push((handles.handle(agent).to_bits(), d.to_bits()));
        }
        cycles.lane_car_ends.push(cycles.cars.len() as u32);
    }
}

// The lane `lane`'s head waits for, or `NONE`: the head is a car standing at the lane's end and
// the entry it needs next is blocked. Connector heads wait for their exit lane's entry slot,
// the test `exit_connector_lane` makes. O(node degree) per head at its end, O(1) otherwise.
fn waited_for(
    agents: &AgentVec,
    lane: usize,
    network: &TransitNetwork,
    graph: &RegionGraph,
    buckets: &[Vec<(f32, usize)>],
) -> u32 {
    let Some(&(d, i)) = buckets[lane].last() else {
        return NONE;
    };
    // The lane's own fields first: most heads are not at their lane's end, and the agent columns
    // are random reads.
    let lanes = &network.lane_system.lanes;
    if lanes[lane].lane_type != LaneType::Vehicle || d + AT_END_M < lanes[lane].length {
        return NONE;
    }
    let transit = agents.transit[i];
    if agents.current_lane_id[i] != lane
        || agents.transit_mode[i] != MODE_CAR
        || (transit != TRANSIT_NETWORK && transit != TRANSIT_INTERSECTION)
    {
        return NONE;
    }
    let blocker = if lanes[lane].edge_id == usize::MAX {
        lanes[lane]
            .next_lanes
            .first()
            .copied()
            .filter(|&exit| !lane_entry_slot_clear(exit, buckets))
    } else {
        head_target(agents, i, lane, network, graph).and_then(|target| {
            match entry_state(lane, target, network, buckets) {
                Entry::Blocked(by) => Some(by),
                Entry::Free | Entry::Missing => None,
            }
        })
    };
    blocker.map_or(NONE, |by| by as u32)
}

// Where road-lane head `i` goes after `lane`'s end under its route.
fn head_target(
    agents: &AgentVec,
    i: usize,
    lane: usize,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<NextTarget> {
    planned_next_target(
        lane,
        agents.current_node[i],
        &agents.current_path[i],
        agents.current_path_index[i],
        agents.access_flags[i],
        agents.planned_detach_node[i],
        agents.planned_detach_lane_id[i] as usize,
        network,
        graph,
    )
}

// Whether a car at the end of `from` can enter toward `target` now, with the tests the movement
// step makes: a direct pass-through lane's entry slot; else, among the connectors that lead
// there (only the shortest ones toward an edge), one whose entry slot is clear. O(successors of
// `from`), no allocation.
fn entry_state(
    from: usize,
    target: NextTarget,
    network: &TransitNetwork,
    buckets: &[Vec<(f32, usize)>],
) -> Entry {
    let lanes = &network.lane_system.lanes;
    let direct = match target {
        NextTarget::Edge(edge) => direct_vehicle_lane_to_edge(from, edge, network),
        NextTarget::Lane(lane) => direct_vehicle_lane_to_lane(from, lane, network).then_some(lane),
    };
    if let Some(next) = direct {
        return if lane_entry_slot_clear(next, buckets) {
            Entry::Free
        } else {
            Entry::Blocked(next)
        };
    }
    let leads_there = |connector: usize| match target {
        NextTarget::Edge(edge) => connector_targets_edge(connector, edge, network),
        NextTarget::Lane(lane) => connector_targets_lane(connector, lane, network),
    };
    let successors = &lanes[from].next_lanes;
    let shortest = match target {
        NextTarget::Edge(_) => successors
            .iter()
            .copied()
            .filter(|&connector| leads_there(connector))
            .map(|connector| lanes[connector].length)
            .min_by(f32::total_cmp),
        NextTarget::Lane(_) => None,
    };
    let mut blocker = None;
    for &connector in successors.iter() {
        if !leads_there(connector)
            || shortest.is_some_and(|length| {
                (lanes[connector].length - length).abs() > SHORTEST_CONNECTOR_EPS_M
            })
        {
            continue;
        }
        if lane_entry_slot_clear(connector, buckets) {
            return Entry::Free;
        }
        blocker.get_or_insert(connector);
    }
    blocker.map_or(Entry::Missing, Entry::Blocked)
}
