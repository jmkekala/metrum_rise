// SPDX-License-Identifier: GPL-2.0-only

//! Agent index repair after graph, lane, building, and household remaps.

use super::data::AgentSystem;
use super::tick::lane_attach_slot_clear;
use super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, MODE_CAR, MODE_WALK, TRANSIT_IN_BUILDING,
    TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::{Lane, LaneIdentity, LaneSystem, LaneType};
use crate::simulation::pathing::cch::CchGraph;
use godot::prelude::Vector2;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

const LANE_REATTACH_MAX_DIST_M: f32 = 30.0;

// The edit-sized sets the per-agent tests below probe, up to once per path node of every agent
// on the network. Only membership is read (iteration orders are sorted or tie-broken), so the
// fixed fast hasher answers exactly as SipHash did, at a fraction of the cost per probe.
type EditSet<T> = HashSet<T, foldhash::fast::FixedState>;

// The edit's nodes, which the route-state test probes once per path node of every agent on the
// network. A fixed 4096-bit filter of the ids answers almost every miss without hashing; a set
// bit is confirmed in the set, so membership is exact. O(edit nodes) to build, in any city.
struct EditNodes {
    filter: [u64; 64],
    nodes: EditSet<u32>,
}

impl EditNodes {
    fn new(nodes: EditSet<u32>) -> Self {
        let mut filter = [0; 64];
        for &node in &nodes {
            let bit = node as usize & 4095;
            filter[bit >> 6] |= 1 << (bit & 63);
        }
        Self { filter, nodes }
    }

    fn contains(&self, node: u32) -> bool {
        let bit = node as usize & 4095;
        self.filter[bit >> 6] & (1 << (bit & 63)) != 0 && self.nodes.contains(&node)
    }
}

/// Agent lane references keyed by their pre-rebuild lane id; see
/// [`AgentSystem::capture_lane_identities`].
pub(crate) struct AgentLaneIdentities(HashMap<usize, LaneIdentity>);

impl AgentSystem {
    /// Remaps household indices after a `swap_remove` in `HouseholdSystem`, and moves the
    /// renumbered households' member lists with them. O(A + mapping).
    pub fn remap_household_indices(&mut self, mapping: &HashMap<usize, usize>) {
        if mapping.is_empty() {
            return;
        }
        self.agents
            .household_id
            .par_iter_mut()
            .for_each(|household_id| {
                if let Some(&new_id) = mapping.get(household_id) {
                    *household_id = new_id;
                }
            });
        self.remap_household_members(mapping);
    }

    /// Remaps building indices after a `swap_remove` in `BuildingAllocator`. O(A).
    pub fn remap_building_indices(&mut self, mapping: &HashMap<usize, usize>) {
        if mapping.is_empty() {
            return;
        }
        for column in [
            &mut self.agents.home_building,
            &mut self.agents.work_building,
            &mut self.agents.current_building,
            &mut self.agents.target_building,
            &mut self.agents.planned_target_building,
            &mut self.agents.next_departure_origin_building,
            &mut self.agents.next_departure_target_building,
            &mut self.agents.cached_schedule_work_building,
        ] {
            // A swap removal moves exactly one ID. Compare it directly instead of hashing
            // eight mostly unchanged references per agent. Columns are independent and
            // retain simultaneous-map semantics, including cycles in larger remaps.
            if mapping.len() == 1 {
                let (&old, &new) = mapping.iter().next().unwrap();
                column.par_iter_mut().for_each(|value| {
                    if *value == old {
                        *value = new;
                    }
                });
            } else {
                column
                    .par_iter_mut()
                    .for_each(|value| remap_usize(value, mapping));
            }
        }
    }

    /// [`Self::remap_building_indices`] for one swap-remove moving building `old` to `new`, over
    /// the ascending `candidates` alone: every agent that names `old` must be among them, as
    /// [`Self::agents_referencing_buildings`] found them before a removal that has removed no
    /// agent since (`SIM-02-K`). O(candidates).
    pub(crate) fn remap_building_among(&mut self, old: usize, new: usize, candidates: &[usize]) {
        let agents = &mut self.agents;
        for column in [
            &mut agents.home_building,
            &mut agents.work_building,
            &mut agents.current_building,
            &mut agents.target_building,
            &mut agents.planned_target_building,
            &mut agents.next_departure_origin_building,
            &mut agents.next_departure_target_building,
            &mut agents.cached_schedule_work_building,
        ] {
            for &i in candidates {
                if column[i] == old {
                    column[i] = new;
                }
            }
        }
        // Test builds: no agent outside the candidates named `old`.
        #[cfg(test)]
        if crate::debug::work::city_checks() {
            assert!(
                self.agents_referencing_buildings(old, old).is_empty(),
                "an agent outside the removal's candidates named the moved building"
            );
        }
    }

    /// Sets `current_lane_id = usize::MAX` for every agent whose active lane belongs to the
    /// incremental rebuild closure of `affected_edges`, or whose connector leads into that closure.
    ///
    /// Must be called **before** `LaneSystem::rebuild_edges_incremental` so the old lane IDs
    /// are still valid for lookup — old orphaned lanes retain their original `edge_id` even
    /// after `rebuild_edges_incremental` removes them from `edge_lanes`.
    pub fn invalidate_lane_ids_for_edges(
        &mut self,
        affected_edges: &HashSet<usize>,
        lane_system: &LaneSystem,
        graph: &RegionGraph,
    ) {
        let input_edges = affected_edges;
        let affected_edges = edit_closure(graph, affected_edges);
        let affected_lane_ids = collect_lane_ids_for_edges(&affected_edges, lane_system);
        let traffic_debug = crate::debug::Channel::Traffic.enabled();
        if traffic_debug {
            log_road_edit_affected(
                "lane-invalidate",
                input_edges,
                &affected_edges,
                &affected_lane_ids,
                graph,
            );
        }

        // The test reads only agent `i`'s lane and the edit's sets, and the writes below touch
        // only agent `i`, so the shared pool tests every agent first and the writes follow in
        // agent order, as the serial loop made them. O(A / P + invalidated agents).
        let agents = &self.agents;
        let invalidated: Vec<(usize, usize)> = crate::thread_pools::install(|| {
            (0..agents.len())
                .into_par_iter()
                .filter_map(|i| {
                    let lane = lane_system.lanes.get(agents.current_lane_id[i])?;
                    invalidated_lane_reattach_edge(
                        lane,
                        &affected_edges,
                        &affected_lane_ids,
                        lane_system,
                    )
                    .map(|edge| (i, edge))
                })
                .collect()
        });
        for (i, reattach_edge_id) in invalidated {
            let lid = self.agents.current_lane_id[i];
            let lane = &lane_system.lanes[lid];
            {
                let old_edge = self.agents.current_edge[i];
                let old_node = self.agents.current_node[i];
                let old_lane_d = self.agents.lane_distance[i];
                let old_pos_x = self.agents.pos_x[i];
                let old_pos_y = self.agents.pos_y[i];
                let sampled_pos = sample_lane_position_xz(lane, old_lane_d);
                if let Some(pos) = sampled_pos {
                    self.agents.pos_x[i] = pos.x;
                    self.agents.pos_y[i] = pos.y;
                }
                if traffic_debug {
                    let log_pos_x = sampled_pos.map(|pos| pos.x).unwrap_or(old_pos_x);
                    let log_pos_y = sampled_pos.map(|pos| pos.y).unwrap_or(old_pos_y);
                    crate::traffic_log!(
                        "[ROAD_EDIT_AGENT_INVALIDATE] agent={} transit={} mode={} old_edge={} old_node={} old_lane={} old_lane_d={:.2} reattach_edge={} pos=({:.2},{:.2})",
                        i,
                        self.agents.transit[i],
                        self.agents.transit_mode[i],
                        old_edge,
                        old_node,
                        lid,
                        old_lane_d,
                        reattach_edge_id,
                        log_pos_x,
                        log_pos_y,
                    );
                }
                self.agents.current_edge[i] = reattach_edge_id;
                self.agents.current_lane_id[i] = usize::MAX;
                self.agents.lane_change_from_lane_id[i] = u32::MAX;
                self.agents.lane_change_start_d[i] = 0.0;
                self.agents.lane_change_length_m[i] = 0.0;
                self.agents.overtake_blocked_time_s[i] = 0.0;
                self.agents.overtake_cooldown_s[i] = 0.0;
                // Preserve lane_distance so road edits do not visually snap agents to lane start
                // before the next tick can re-attach them.
            }
        }
        self.invalidate_lane_bucket_snapshot();
    }

    /// Reattaches invalidated on-road agents to rebuilt lanes near their preserved world position.
    ///
    /// Call this after rebuilding lane geometry for the same affected edge set passed to
    /// [`Self::invalidate_lane_ids_for_edges`]. Agents keep their physical progress along the road
    /// edit instead of falling back to a graph node start on the next movement tick.
    pub fn reattach_invalidated_lanes_for_edges(
        &mut self,
        affected_edges: &HashSet<usize>,
        lane_system: &LaneSystem,
        graph: &RegionGraph,
    ) {
        let input_edges = affected_edges;
        let affected_edges = edit_closure(graph, affected_edges);
        let affected_lane_ids = collect_lane_ids_for_edges(&affected_edges, lane_system);
        let affected_nodes = EditNodes::new(collect_nodes_for_edges(&affected_edges, graph));
        let max_dist_sq = LANE_REATTACH_MAX_DIST_M * LANE_REATTACH_MAX_DIST_M;
        let traffic_debug = crate::debug::Channel::Traffic.enabled();
        if traffic_debug {
            log_road_edit_affected(
                "lane-reattach",
                input_edges,
                &affected_edges,
                &affected_lane_ids,
                graph,
            );
        }
        // Agent `i`'s reattachment reads only agent `i` and the rebuilt lanes and writes only
        // agent `i`: the shared pool finds the invalidated on-road agents and each one's best
        // rebuilt lane, and they then reattach in agent order, as the serial loop did.
        // O(A / P + invalidated agents × rebuilt lanes / P).
        let agents = &self.agents;
        let detached: Vec<(usize, Option<LaneReattachCandidate>)> =
            crate::thread_pools::install(|| {
                let detached: Vec<usize> = (0..agents.len())
                    .into_par_iter()
                    .filter(|&i| {
                        agents.current_lane_id[i] == usize::MAX
                            && matches!(agents.transit[i], TRANSIT_NETWORK | TRANSIT_INTERSECTION)
                            && affected_edges.contains(&agents.current_edge[i])
                    })
                    .collect();
                detached
                    .into_par_iter()
                    .map(|i| {
                        let desired_lane_type = if agents.transit_mode[i] == MODE_WALK {
                            LaneType::Foot
                        } else {
                            LaneType::Vehicle
                        };
                        let candidate = best_reattach_lane(
                            Vector2::new(agents.pos_x[i], agents.pos_y[i]),
                            desired_lane_type,
                            &affected_edges,
                            lane_system,
                            graph,
                            max_dist_sq,
                        );
                        (i, candidate)
                    })
                    .collect()
            });
        // Vehicles reattached so far, by rebuilt lane, sorted by distance. Every car on the edit's
        // lanes was detached, so these are the rebuilt lanes' whole occupancy: a car whose spot
        // is taken waits there for an attach gap instead of stacking (`TRAFFIC-11`).
        // O(reattached × log) per edit.
        let mut reattached: HashMap<usize, Vec<(f32, usize)>> = HashMap::new();
        for (i, candidate) in detached {
            let old_edge = self.agents.current_edge[i];
            let old_node = self.agents.current_node[i];
            let old_lane_d = self.agents.lane_distance[i];
            let old_pos_x = self.agents.pos_x[i];
            let old_pos_y = self.agents.pos_y[i];
            let Some(candidate) = candidate else {
                if traffic_debug {
                    crate::traffic_log!(
                        "[ROAD_EDIT_AGENT_REATTACH] agent={} action=no-candidate transit={} mode={} old_edge={} old_node={} old_lane_d={:.2} pos=({:.2},{:.2}) max_dist_m={:.2}",
                        i,
                        self.agents.transit[i],
                        self.agents.transit_mode[i],
                        old_edge,
                        old_node,
                        old_lane_d,
                        old_pos_x,
                        old_pos_y,
                        LANE_REATTACH_MAX_DIST_M,
                    );
                }
                continue;
            };

            let room = self.agents.transit_mode[i] == MODE_WALK || {
                let cars = reattached.entry(candidate.lane_id).or_default();
                let clear = lane_attach_slot_clear(cars, candidate.lane_d);
                if clear {
                    let at = cars.partition_point(|car| car.0 < candidate.lane_d);
                    cars.insert(at, (candidate.lane_d, i));
                }
                clear
            };
            if traffic_debug {
                crate::traffic_log!(
                    "[ROAD_EDIT_AGENT_REATTACH] agent={} action={} transit={} mode={} old_edge={} old_node={} old_lane_d={:.2} old_pos=({:.2},{:.2}) new_edge={} new_node={} new_lane={} new_lane_d={:.2} snap_m={:.2} new_pos=({:.2},{:.2})",
                    i,
                    if room { "reattach" } else { "hold-for-gap" },
                    self.agents.transit[i],
                    self.agents.transit_mode[i],
                    old_edge,
                    old_node,
                    old_lane_d,
                    old_pos_x,
                    old_pos_y,
                    candidate.edge_id,
                    candidate.origin_node,
                    candidate.lane_id,
                    candidate.lane_d,
                    candidate.dist_sq.sqrt(),
                    candidate.pos.x,
                    candidate.pos.y,
                );
            }
            if room {
                self.agents.enter_lane(
                    i,
                    lane_system,
                    candidate.lane_id,
                    candidate.lane_d,
                    candidate.origin_node,
                );
            } else {
                self.agents.hold_for_lane_attach(
                    i,
                    lane_system,
                    candidate.lane_id,
                    candidate.lane_d,
                    candidate.origin_node,
                );
            }
            self.agents.pos_x[i] = candidate.pos.x;
            self.agents.pos_y[i] = candidate.pos.y;
            self.agents.current_path[i].clear();
            self.agents.current_path_index[i] = 0;
            self.agents.network_replan_failures[i] = 0;
            self.agents.lane_change_from_lane_id[i] = u32::MAX;
            self.agents.lane_change_start_d[i] = 0.0;
            self.agents.lane_change_length_m[i] = 0.0;
            self.agents.overtake_blocked_time_s[i] = 0.0;
            self.agents.overtake_cooldown_s[i] = 0.0;
        }

        // The test reads only agent `i` and clearing writes only agent `i`, so the shared pool
        // tests every agent first (editor paths arrive outside it, CODE-17) and the routes clear
        // in agent order. O(A + path nodes / P).
        let agents = &*self;
        let touched: Vec<usize> = crate::thread_pools::install(|| {
            (0..agents.agents.len())
                .into_par_iter()
                .filter(|&i| {
                    agents.agents.transit[i] != TRANSIT_IN_BUILDING
                        && agent_route_state_touches_road_edit(
                            agents,
                            i,
                            &affected_edges,
                            &affected_nodes,
                            &affected_lane_ids,
                            lane_system,
                        )
                })
                .collect()
        });
        for i in touched {
            if traffic_debug {
                log_agent_route_clear_after_road_edit(self, i);
            }
            self.clear_route_plan_after_road_edit(i, lane_system);
        }
        self.invalidate_lane_bucket_snapshot();
    }

    /// Records the identity of every lane an agent references, before a full
    /// [`LaneSystem::rebuild`] renumbers all lane ids.
    ///
    /// O(A), plus one O(L) pass when any agent is inside a junction connector.
    pub(crate) fn capture_lane_identities(&self, lane_system: &LaneSystem) -> AgentLaneIdentities {
        let agents = &self.agents;
        let mut lane_ids = HashSet::new();
        for i in 0..agents.len() {
            lane_ids.insert(agents.current_lane_id[i]);
            for id in [
                agents.planned_attach_lane_id[i],
                agents.planned_detach_lane_id[i],
                agents.lane_change_from_lane_id[i],
            ] {
                lane_ids.insert(if id == u32::MAX {
                    usize::MAX
                } else {
                    id as usize
                });
            }
        }
        lane_ids.remove(&usize::MAX);
        AgentLaneIdentities(lane_system.lane_identities(&lane_ids))
    }

    /// Points every lane reference captured by [`Self::capture_lane_identities`] at the rebuilt
    /// lane with the same identity. O(A).
    ///
    /// A reference whose lane the rebuild removed is dropped the way a road edit drops it: the
    /// agent loses its current lane, abandons a junction crossing and replans, and
    /// [`Self::reattach_invalidated_lanes_for_edges`] can then reattach it.
    pub(crate) fn restore_lane_identities(
        &mut self,
        captured: &AgentLaneIdentities,
        lane_system: &LaneSystem,
    ) {
        let resolve = |id: usize| {
            captured
                .0
                .get(&id)
                .and_then(|&identity| lane_system.resolve_lane_identity(identity))
        };
        let resolve_u32 = |id: u32| -> Result<u32, ()> {
            if id == u32::MAX {
                return Ok(u32::MAX);
            }
            resolve(id as usize).map(|id| id as u32).ok_or(())
        };
        for i in 0..self.agents.len() {
            let mut lost = false;
            let current = self.agents.current_lane_id[i];
            if current != usize::MAX {
                if let Some(id) = resolve(current) {
                    self.agents.current_lane_id[i] = id;
                    // The rebuild may have changed heights, and with them lane lengths.
                    self.agents.lane_distance[i] =
                        self.agents.lane_distance[i].min(lane_system.lanes[id].length);
                } else {
                    self.agents.current_lane_id[i] = usize::MAX;
                    lost = true;
                }
            }
            for planned in [
                &mut self.agents.planned_attach_lane_id[i],
                &mut self.agents.planned_detach_lane_id[i],
            ] {
                *planned = resolve_u32(*planned).unwrap_or_else(|()| {
                    lost = true;
                    u32::MAX
                });
            }
            let from = resolve_u32(self.agents.lane_change_from_lane_id[i]).unwrap_or(u32::MAX);
            self.agents.lane_change_from_lane_id[i] = from;
            if from == u32::MAX {
                self.agents.lane_change_start_d[i] = 0.0;
                self.agents.lane_change_length_m[i] = 0.0;
            }
            if lost {
                self.clear_route_plan_after_road_edit(i, lane_system);
            }
        }
        self.invalidate_lane_bucket_snapshot();
    }

    /// Drops every car route on the road that needs a turn the network's lanes do not build,
    /// so the car replans before reaching it. Saved routes may predate the current turn rules
    /// (`NET-07`); road edits already clear routes through edited nodes. A parallel scan,
    /// O(agents × remaining route length), then a serial clear of the affected agents.
    pub(crate) fn clear_routes_with_illegal_turns(
        &mut self,
        lane_system: &LaneSystem,
        graph: &RegionGraph,
    ) {
        let illegal: Vec<_> = (0..self.agents.len())
            .into_par_iter()
            .filter(|&i| !self.route_turns_are_legal(i, lane_system, graph))
            .collect();
        for i in illegal {
            self.clear_route_plan_after_road_edit(i, lane_system);
        }
    }

    // The remaining route's turns, from the current road into the planned exit lane. A car on
    // a junction connector has already committed to the one lane it leads into, so that lane's
    // road is the incoming road at the next turn; the path has already advanced past this node.
    fn route_turns_are_legal(
        &self,
        i: usize,
        lane_system: &LaneSystem,
        graph: &RegionGraph,
    ) -> bool {
        let agents = &self.agents;
        if agents.transit_mode[i] != MODE_CAR
            || agents.access_flags[i] & ACCESS_PLAN_VALID == 0
            || !matches!(agents.transit[i], TRANSIT_NETWORK | TRANSIT_INTERSECTION)
        {
            return true;
        }
        let lane_edge = |lane: usize| {
            lane_system
                .lanes
                .get(lane)
                .map_or(usize::MAX, |lane| lane.edge_id)
        };
        let current = agents.current_lane_id[i];
        let detach = agents.planned_detach_lane_id[i];
        let road_lane = if agents.transit[i] == TRANSIT_NETWORK {
            current
        } else {
            lane_system
                .lanes
                .get(current)
                .and_then(|lane| lane.next_lanes.first().copied())
                .unwrap_or(usize::MAX)
        };
        let incoming = lane_edge(road_lane);
        let outgoing = if detach == u32::MAX {
            usize::MAX
        } else {
            lane_edge(detach as usize)
        };
        let path = &agents.current_path[i];
        let rest = &path[agents.current_path_index[i].min(path.len())..];
        if !rest.is_empty() {
            return CchGraph::path_has_valid_vehicle_turns(rest, incoming, outgoing, graph);
        }
        // A direct frontage stays on its lane; a zero hop turns at the detach node.
        let node = agents.planned_detach_node[i];
        road_lane == detach as usize
            || node == u32::MAX
            || CchGraph::path_has_valid_vehicle_turns(&[node], incoming, outgoing, graph)
    }

    fn clear_route_plan_after_road_edit(&mut self, i: usize, lane_system: &LaneSystem) {
        let freight_border_node = self.agents.freight_target_border_node[i];
        let preserve_freight_border = freight_border_node != u32::MAX
            && (self.agents.access_flags[i] & ACCESS_FREIGHT_BORDER_DESTINATION) != 0;
        let should_drop_intersection_lane = self.agents.transit[i] == TRANSIT_INTERSECTION
            && match lane_system.lanes.get(self.agents.current_lane_id[i]) {
                Some(lane) => lane.edge_id == usize::MAX,
                None => true,
            };

        let pending_attach = self.agents.pending_lane_attach(i);
        self.agents.current_path[i].clear();
        self.agents.current_path_index[i] = 0;
        if should_drop_intersection_lane {
            self.agents.transit[i] = TRANSIT_NETWORK;
            self.agents.current_edge[i] = usize::MAX;
            self.agents.current_lane_id[i] = usize::MAX;
            self.agents.lane_distance[i] = 0.0;
            self.agents.speed[i] = 0.0;
        }
        self.agents.planned_attach_node[i] = u32::MAX;
        self.agents.planned_attach_lane_id[i] = u32::MAX;
        self.agents.planned_attach_lane_d[i] = 0.0;
        self.agents.planned_detach_lane_d[i] = 0.0;
        if preserve_freight_border {
            self.agents.planned_detach_node[i] = freight_border_node;
            self.agents.planned_detach_lane_id[i] = u32::MAX;
            self.agents.access_flags[i] = ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION;
        } else {
            self.agents.planned_detach_node[i] = u32::MAX;
            self.agents.planned_detach_lane_id[i] = u32::MAX;
            self.agents.access_flags[i] = 0;
        }
        self.agents.next_replan_time[i] = 0.0;
        self.agents.network_replan_failures[i] = 0;
        self.agents.lane_change_from_lane_id[i] = u32::MAX;
        self.agents.lane_change_start_d[i] = 0.0;
        self.agents.lane_change_length_m[i] = 0.0;
        self.agents.overtake_blocked_time_s[i] = 0.0;
        self.agents.overtake_cooldown_s[i] = 0.0;
        self.agents.keep_pending_lane_attach(i, pending_attach);
    }
}

// The lane rebuild closure of `affected_edges` (`LaneSystem::incremental_rebuild_edge_closure`).
fn edit_closure(graph: &RegionGraph, affected_edges: &HashSet<usize>) -> EditSet<usize> {
    LaneSystem::incremental_rebuild_edge_closure(graph, affected_edges)
        .into_iter()
        .collect()
}

fn collect_lane_ids_for_edges(
    affected_edges: &EditSet<usize>,
    lane_system: &LaneSystem,
) -> EditSet<usize> {
    let mut affected_lane_ids = EditSet::default();
    for &edge_id in affected_edges {
        if let Some(lane_ids) = lane_system.edge_lanes.get(&edge_id) {
            affected_lane_ids.extend(lane_ids);
        }
    }
    affected_lane_ids
}

fn collect_nodes_for_edges(affected_edges: &EditSet<usize>, graph: &RegionGraph) -> EditSet<u32> {
    let mut affected_nodes = EditSet::default();
    for &edge_id in affected_edges {
        let Some(edge) = graph.get_edge(edge_id) else {
            continue;
        };
        affected_nodes.insert(graph.get_valid_node(edge.start_node));
        affected_nodes.insert(graph.get_valid_node(edge.end_node));
    }
    affected_nodes
}

fn log_road_edit_affected(
    phase: &str,
    input_edges: &HashSet<usize>,
    closure_edges: &EditSet<usize>,
    affected_lane_ids: &EditSet<usize>,
    graph: &RegionGraph,
) {
    let affected_nodes = collect_nodes_for_edges(closure_edges, graph);
    crate::traffic_log!(
        "[ROAD_EDIT_AFFECTED] phase={} input_edges={:?} closure_edges={:?} affected_nodes={:?} affected_lane_count={}",
        phase,
        sorted_ids(input_edges),
        sorted_ids(closure_edges),
        sorted_ids(&affected_nodes),
        affected_lane_ids.len(),
    );
}

fn log_agent_route_clear_after_road_edit(agents: &AgentSystem, i: usize) {
    crate::traffic_log!(
        "[ROAD_EDIT_ROUTE_CLEAR] agent={} transit={} mode={} current_edge={} current_node={} current_lane={} target_bldg={} planned_attach_node={} planned_detach_node={} planned_attach_lane={} planned_detach_lane={} path_len={} path_idx={} flags_before=0x{:02x}",
        i,
        agents.agents.transit[i],
        agents.agents.transit_mode[i],
        agents.agents.current_edge[i],
        agents.agents.current_node[i],
        agents.agents.current_lane_id[i],
        agents.agents.target_building[i],
        agents.agents.planned_attach_node[i],
        agents.agents.planned_detach_node[i],
        agents.agents.planned_attach_lane_id[i],
        agents.agents.planned_detach_lane_id[i],
        agents.agents.current_path[i].len(),
        agents.agents.current_path_index[i],
        agents.agents.access_flags[i],
    );
}

fn sorted_ids<T: Copy + Ord, S>(values: &HashSet<T, S>) -> Vec<T> {
    let mut ids: Vec<T> = values.iter().copied().collect();
    ids.sort_unstable();
    ids
}

fn agent_route_state_touches_road_edit(
    agents: &AgentSystem,
    i: usize,
    affected_edges: &EditSet<usize>,
    affected_nodes: &EditNodes,
    affected_lane_ids: &EditSet<usize>,
    lane_system: &LaneSystem,
) -> bool {
    let current_edge = agents.agents.current_edge[i];
    if current_edge != usize::MAX && affected_edges.contains(&current_edge) {
        return true;
    }
    let current_node = agents.agents.current_node[i];
    if current_node != u32::MAX && affected_nodes.contains(current_node) {
        return true;
    }
    if lane_usize_touches_road_edit(
        agents.agents.current_lane_id[i],
        affected_edges,
        affected_lane_ids,
        lane_system,
    ) {
        return true;
    }
    if lane_u32_touches_road_edit(
        agents.agents.planned_attach_lane_id[i],
        affected_edges,
        affected_lane_ids,
        lane_system,
    ) || lane_u32_touches_road_edit(
        agents.agents.planned_detach_lane_id[i],
        affected_edges,
        affected_lane_ids,
        lane_system,
    ) {
        return true;
    }

    let attach_node = agents.agents.planned_attach_node[i];
    if attach_node != u32::MAX && affected_nodes.contains(attach_node) {
        return true;
    }
    let detach_node = agents.agents.planned_detach_node[i];
    if detach_node != u32::MAX && affected_nodes.contains(detach_node) {
        return true;
    }

    agents.agents.current_path[i]
        .iter()
        .any(|&node| affected_nodes.contains(node))
}

fn lane_u32_touches_road_edit(
    lane_id: u32,
    affected_edges: &EditSet<usize>,
    affected_lane_ids: &EditSet<usize>,
    lane_system: &LaneSystem,
) -> bool {
    if lane_id == u32::MAX {
        return false;
    }
    lane_usize_touches_road_edit(
        lane_id as usize,
        affected_edges,
        affected_lane_ids,
        lane_system,
    )
}

fn lane_usize_touches_road_edit(
    lane_id: usize,
    affected_edges: &EditSet<usize>,
    affected_lane_ids: &EditSet<usize>,
    lane_system: &LaneSystem,
) -> bool {
    if lane_id == usize::MAX || affected_lane_ids.contains(&lane_id) {
        return lane_id != usize::MAX;
    }
    lane_system.lanes.get(lane_id).is_some_and(|lane| {
        (lane.edge_id != usize::MAX && affected_edges.contains(&lane.edge_id))
            || (lane.edge_id == usize::MAX
                && lane
                    .next_lanes
                    .iter()
                    .any(|next_lane_id| affected_lane_ids.contains(next_lane_id)))
    })
}

fn invalidated_lane_reattach_edge(
    lane: &Lane,
    affected_edges: &EditSet<usize>,
    affected_lane_ids: &EditSet<usize>,
    lane_system: &LaneSystem,
) -> Option<usize> {
    if lane.edge_id != usize::MAX {
        return affected_edges
            .contains(&lane.edge_id)
            .then_some(lane.edge_id);
    }

    lane.next_lanes.iter().copied().find_map(|next_lane_id| {
        if !affected_lane_ids.contains(&next_lane_id) {
            return None;
        }
        lane_system.lanes.get(next_lane_id).and_then(|target_lane| {
            (target_lane.edge_id != usize::MAX).then_some(target_lane.edge_id)
        })
    })
}

fn remap_usize(value: &mut usize, mapping: &HashMap<usize, usize>) {
    if let Some(&new_id) = mapping.get(value) {
        *value = new_id;
    }
}

struct LaneReattachCandidate {
    lane_id: usize,
    edge_id: usize,
    origin_node: u32,
    lane_d: f32,
    pos: Vector2,
    dist_sq: f32,
}

fn best_reattach_lane<S>(
    agent_pos: Vector2,
    desired_lane_type: LaneType,
    affected_edges: &HashSet<usize, S>,
    lane_system: &LaneSystem,
    graph: &RegionGraph,
    max_dist_sq: f32,
) -> Option<LaneReattachCandidate> {
    let mut best: Option<LaneReattachCandidate> = None;
    for &edge_id in affected_edges {
        if graph
            .get_edge(edge_id)
            .is_none_or(|edge| edge.deleted || edge.physical_geometry.len() < 2)
        {
            continue;
        }
        let Some(lane_ids) = lane_system.edge_lanes.get(&edge_id) else {
            continue;
        };
        for &lane_id in lane_ids {
            let Some(lane) = lane_system.lanes.get(lane_id) else {
                continue;
            };
            if lane.edge_id != edge_id || lane.lane_type != desired_lane_type {
                continue;
            }
            let Some(origin_node) = lane_origin_node_for(lane, graph) else {
                continue;
            };
            let Some((lane_d, pos, dist_sq)) = project_point_to_lane(agent_pos, lane) else {
                continue;
            };
            if dist_sq > max_dist_sq {
                continue;
            }
            let replace = best.as_ref().is_none_or(|best| {
                dist_sq
                    .total_cmp(&best.dist_sq)
                    .then_with(|| lane_id.cmp(&best.lane_id))
                    .is_lt()
            });
            if replace {
                best = Some(LaneReattachCandidate {
                    lane_id,
                    edge_id,
                    origin_node,
                    lane_d,
                    pos,
                    dist_sq,
                });
            }
        }
    }
    best
}

fn lane_origin_node_for(lane: &Lane, graph: &RegionGraph) -> Option<u32> {
    let edge = graph.get_edge(lane.edge_id)?;
    Some(if lane.is_fwd {
        edge.start_node
    } else {
        edge.end_node
    })
}

fn sample_lane_position_xz(lane: &Lane, lane_d: f32) -> Option<Vector2> {
    if lane.geometry.len() < 2 {
        return None;
    }
    Some(BuildingAllocator::sample_pos_on_lane(lane, lane_d))
}

fn project_point_to_lane(point: Vector2, lane: &Lane) -> Option<(f32, Vector2, f32)> {
    if lane.geometry.len() < 2 {
        return None;
    }

    let lane_d = BuildingAllocator::project_point_to_polyline_s(&lane.geometry, point)
        .clamp(0.0, lane.length.max(0.0));
    let pos = BuildingAllocator::sample_pos_on_lane(lane, lane_d);
    let dist_sq = point.distance_squared_to(pos);
    Some((lane_d, pos, dist_sq))
}

#[cfg(test)]
mod tests {
    use super::super::data::AgentSystem;
    use super::super::{
        ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PATH_FROM_FLOW_FIELD, ACCESS_PLAN_VALID,
        ACCESS_ZERO_HOP_NODE_PATH, MODE_CAR, TRANSIT_IN_BUILDING, TRANSIT_INTERSECTION,
        TRANSIT_NETWORK,
    };
    use super::*;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::graph::data::Edge;
    use crate::simulation::network::lanes::LaneSystem;
    use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};
    use godot::prelude::Vector3;

    fn make_simple_lane_system() -> (RegionGraph, LaneSystem) {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(
            Vector3::new(0.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n1 = graph.add_node(
            Vector3::new(100.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n2 = graph.add_node(
            Vector3::new(200.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );

        graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            ..Default::default()
        });
        graph.add_edge(Edge {
            start_node: n1,
            end_node: n2,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(100.0, 0.0, 0.0), Vector3::new(200.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(100.0, 0.0, 0.0), Vector3::new(200.0, 0.0, 0.0)],
            ..Default::default()
        });
        graph.rebuild_adjacency_list();

        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        (graph, lanes)
    }

    #[test]
    fn reattachment_chooses_nearest_lane_independently_of_candidate_order() {
        let (graph, _) = make_simple_lane_system();
        let mut lanes = LaneSystem::new();
        for (lane_id, z) in [0.012, 0.008, 0.0, 0.0].into_iter().enumerate() {
            lanes.lanes.push(Lane {
                edge_id: 0,
                lane_idx: lane_id as i8,
                geometry: vec![Vector3::new(0.0, 0.0, z), Vector3::new(100.0, 0.0, z)],
                length: 100.0,
                cum_dist: vec![0.0, 100.0],
                ..Lane::default()
            });
        }
        let affected = HashSet::from([0]);
        // Adjacent squared distances differ by less than the old epsilon, while the endpoints
        // differ by more. Approximate tie-breaking can therefore depend on iteration order.
        for order in [
            [0, 1, 2, 3],
            [0, 2, 1, 3],
            [1, 0, 2, 3],
            [1, 2, 0, 3],
            [2, 0, 1, 3],
            [2, 1, 0, 3],
            [3, 2, 1, 0],
        ] {
            lanes.edge_lanes.insert(0, order.to_vec());
            let nearest = best_reattach_lane(
                Vector2::new(50.0, 0.0),
                LaneType::Vehicle,
                &affected,
                &lanes,
                &graph,
                900.0,
            )
            .unwrap();
            assert_eq!(nearest.lane_id, 2, "candidate order {order:?}");
            assert_eq!(nearest.dist_sq, 0.0);
            assert_eq!(nearest.lane_d, 50.0);
        }
    }

    #[test]
    #[ignore = "manual matched release timing of local lane reattachment queries"]
    fn benchmark_lane_reattachment() {
        use std::{hint::black_box, time::Instant};
        let (graph, lanes) = make_simple_lane_system();
        let affected = HashSet::from([0, 1]);
        for count in [1_024, 16_384, 131_072] {
            let positions: Vec<_> = (0..count)
                .map(|index| Vector2::new((index % 201) as f32, (index % 11) as f32 - 5.0))
                .collect();
            let mut samples = [0.0; 21];
            for sample in 0..24 {
                let start = Instant::now();
                for &position in &positions {
                    black_box(best_reattach_lane(
                        black_box(position),
                        LaneType::Vehicle,
                        &affected,
                        &lanes,
                        &graph,
                        900.0,
                    ));
                }
                if sample >= 3 {
                    samples[sample - 3] = start.elapsed().as_secs_f64() * 1_000.0;
                }
            }
            let checksum = positions.iter().fold(0_u64, |hash, &position| {
                let selected = best_reattach_lane(
                    position,
                    LaneType::Vehicle,
                    &affected,
                    &lanes,
                    &graph,
                    900.0,
                )
                .unwrap();
                [
                    selected.lane_id as u64,
                    u64::from(selected.lane_d.to_bits()),
                    u64::from(selected.dist_sq.to_bits()),
                ]
                .into_iter()
                .fold(hash, |hash, value| {
                    hash.wrapping_mul(31).wrapping_add(value)
                })
            });
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "lane_reattachment queries={count} median_ms={:.6} checksum={checksum}",
                samples[10]
            );
        }
    }

    struct TurnConnectorFixture {
        graph: RegionGraph,
        lane_system: LaneSystem,
        n_west: u32,
        n_center: u32,
        north_edge: usize,
        inbound_lane: usize,
        connector_lane: usize,
    }

    fn make_turn_connector_lane_system() -> TurnConnectorFixture {
        let mut graph = RegionGraph::new();
        let n_west = graph.add_node(
            Vector3::new(-100.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n_center = graph.add_node(
            Vector3::ZERO,
            crate::simulation::network::types::NodeType::Junction,
        );
        let n_east = graph.add_node(
            Vector3::new(100.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n_north = graph.add_node(
            Vector3::new(0.0, 0.0, -100.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let add_edge = |graph: &mut RegionGraph, start_node: u32, end_node: u32| {
            let start = graph.node(start_node).pos;
            let end = graph.node(end_node).pos;
            graph.add_edge(Edge {
                start_node,
                end_node,
                primary_type: TransitType::Road,
                allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
                class: EdgeClass::Standard,
                width: 7.0,
                fwd_lanes: 1,
                bkw_lanes: 1,
                speed_limit: 50.0,
                base_cost: 1.0,
                physical_length: start.distance_to(end),
                start_clip: 0.0,
                end_clip: 0.0,
                geometry: vec![start, end],
                physical_geometry: vec![start, end],
                ..Default::default()
            })
        };
        let west_edge = add_edge(&mut graph, n_west, n_center);
        add_edge(&mut graph, n_center, n_east);
        let north_edge = add_edge(&mut graph, n_center, n_north);
        graph.rebuild_adjacency_list();

        let mut lane_system = LaneSystem::new();
        lane_system.rebuild(&mut graph);

        let inbound_lane = lane_system.edge_lanes[&west_edge]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd
            })
            .expect("west inbound vehicle lane");
        let connector_lane = lane_system.lanes[inbound_lane]
            .next_lanes
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.edge_id == usize::MAX
                    && lane.lane_type == LaneType::Vehicle
                    && lane.next_lanes.first().is_some_and(|&target_lane_id| {
                        lane_system
                            .lanes
                            .get(target_lane_id)
                            .is_some_and(|target_lane| target_lane.edge_id == north_edge)
                    })
            })
            .expect("west-to-north vehicle connector");

        TurnConnectorFixture {
            graph,
            lane_system,
            n_west,
            n_center,
            north_edge,
            inbound_lane,
            connector_lane,
        }
    }

    #[test]
    fn remaps_cached_schedule_building_ids() {
        let mut sys = AgentSystem::new();
        let idx = sys.spawn_housed_agent(3, 0.0, 0.0);
        sys.agents.work_building[idx] = 5;
        sys.agents.current_building[idx] = 3;
        sys.agents.target_building[idx] = 5;
        sys.agents.planned_target_building[idx] = 5;
        sys.agents.next_departure_origin_building[idx] = 3;
        sys.agents.next_departure_target_building[idx] = 5;
        sys.agents.cached_schedule_work_building[idx] = 5;

        let mapping = HashMap::from([(3usize, 8usize), (5usize, 9usize)]);
        sys.remap_building_indices(&mapping);

        assert_eq!(sys.agents.home_building[idx], 8);
        assert_eq!(sys.agents.work_building[idx], 9);
        assert_eq!(sys.agents.current_building[idx], 8);
        assert_eq!(sys.agents.target_building[idx], 9);
        assert_eq!(sys.agents.planned_target_building[idx], 9);
        assert_eq!(sys.agents.next_departure_origin_building[idx], 8);
        assert_eq!(sys.agents.next_departure_target_building[idx], 9);
        assert_eq!(sys.agents.cached_schedule_work_building[idx], 9);
    }

    fn building_columns(system: &AgentSystem) -> [&[usize]; 8] {
        [
            &system.agents.home_building,
            &system.agents.work_building,
            &system.agents.current_building,
            &system.agents.target_building,
            &system.agents.planned_target_building,
            &system.agents.next_departure_origin_building,
            &system.agents.next_departure_target_building,
            &system.agents.cached_schedule_work_building,
        ]
    }

    #[test]
    fn building_remaps_match_simultaneous_map_for_single_moves_and_cycles() {
        for mapping in [
            HashMap::new(),
            HashMap::from([(3, 5)]),
            HashMap::from([(3, 5), (5, 3)]),
            HashMap::from([(usize::MAX, 3)]),
            HashMap::from([(3, 3), (5, usize::MAX)]),
        ] {
            let mut sys = AgentSystem::new();
            for home in [3, 5, 9, usize::MAX] {
                let i = sys.spawn_housed_agent(home, 0.0, 0.0);
                sys.agents.work_building[i] = 5;
                sys.agents.target_building[i] = 3;
                sys.agents.planned_target_building[i] = home;
                sys.agents.next_departure_origin_building[i] = home;
                sys.agents.next_departure_target_building[i] = 5;
                sys.agents.cached_schedule_work_building[i] = 9;
            }
            let expected = building_columns(&sys).map(|column| {
                column
                    .iter()
                    .map(|value| mapping.get(value).copied().unwrap_or(*value))
                    .collect::<Vec<_>>()
            });
            sys.remap_building_indices(&mapping);
            for (actual, expected) in building_columns(&sys).into_iter().zip(expected) {
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    #[ignore = "release swap-removal remap comparison; agent setup excluded"]
    fn benchmark_single_building_remap() {
        use std::{hint::black_box, time::Instant};
        for count in [10_000, 100_000] {
            let mut sys = AgentSystem::new();
            for i in 0..count {
                let agent = sys.spawn_housed_agent(i % 1024, 0.0, 0.0);
                sys.agents.work_building[agent] = (i + 1) % 1024;
                sys.agents.target_building[agent] = (i + 2) % 1024;
            }
            let mut ordinary = building_columns(&sys).map(<[usize]>::to_vec);
            let mut before = Vec::new();
            let mut after = Vec::new();
            for repeat in 0..44 {
                let mapping = if repeat % 2 == 0 {
                    HashMap::from([(1023, 1022)])
                } else {
                    HashMap::from([(1022, 1023)])
                };
                for candidate in if repeat % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                } {
                    let start = Instant::now();
                    if candidate {
                        sys.remap_building_indices(black_box(&mapping));
                        after.push(start.elapsed().as_secs_f64() * 1e6);
                    } else {
                        for i in 0..count {
                            for column in &mut ordinary {
                                remap_usize(&mut column[i], black_box(&mapping));
                            }
                        }
                        before.push(start.elapsed().as_secs_f64() * 1e6);
                    }
                }
                for (actual, expected) in building_columns(&sys).into_iter().zip(&ordinary) {
                    assert_eq!(actual, expected);
                }
            }
            eprintln!(
                "BUILDING_REMAP_BENCH {}",
                serde_json::json!({"agents":count,
                "ordinary_us":before,"parallel_single_us":after,"references_equal":true})
            );
        }
    }

    #[test]
    fn test_invalidate_clears_agents_on_incremental_rebuild_closure() {
        let (graph, lane_system) = make_simple_lane_system();
        let e0_lane = lane_system.edge_lanes[&0][0];
        let e1_lane = lane_system.edge_lanes[&1][0];

        let mut sys = AgentSystem::new();
        for (edge_id, lane_id, x) in [(0, e0_lane, 0.0), (1, e1_lane, 150.0)] {
            let i = sys.spawn_border_arrival_agent(usize::MAX, edge_id as u32, x, 0.0);
            sys.agents.transit[i] = TRANSIT_NETWORK;
            sys.agents.current_edge[i] = edge_id;
            sys.agents.current_lane_id[i] = lane_id;
            sys.agents.lane_distance[i] = 10.0;
            sys.agents.speed[i] = 10.0;
        }

        let affected = HashSet::from([0]);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system, &graph);

        assert_eq!(sys.agents.current_lane_id[0], usize::MAX);
        assert_eq!(sys.agents.lane_distance[0], 10.0);
        assert_eq!(
            sys.agents.current_lane_id[1],
            usize::MAX,
            "the adjacent edge is physically rebuilt because it shares the changed junction"
        );
        assert_eq!(sys.agents.lane_distance[1], 10.0);
    }

    #[test]
    fn test_invalidate_skips_already_invalid_agents() {
        let (graph, lane_system) = make_simple_lane_system();

        let mut sys = AgentSystem::new();
        for (transit, lane_id) in [
            (TRANSIT_IN_BUILDING, usize::MAX),
            (TRANSIT_NETWORK, usize::MAX),
            (TRANSIT_NETWORK, lane_system.lanes.len()),
        ] {
            let i = sys.spawn_housed_agent(0, 12.0, -8.0);
            sys.agents.transit[i] = transit;
            sys.agents.current_lane_id[i] = lane_id;
            sys.agents.lane_distance[i] = 7.0;
        }
        let before = format!("{:?}", sys.agents);
        sys.invalidate_lane_ids_for_edges(&HashSet::from([0]), &lane_system, &graph);
        assert_eq!(format!("{:?}", sys.agents), before);
    }

    #[test]
    fn test_reattach_preserves_agent_position_after_edge_split() {
        let (mut graph, mut lane_system) = make_simple_lane_system();
        let old_lane_id = lane_system.edge_lanes[&0]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd
            })
            .expect("forward vehicle lane on edge 0");
        let old_lane_d = 75.0;
        let old_pos = sample_lane_position_xz(&lane_system.lanes[old_lane_id], old_lane_d).unwrap();
        let mut sys = AgentSystem::new();
        let idx = sys.spawn_border_arrival_agent(usize::MAX, 0, old_pos.x, old_pos.y);
        sys.agents.transit[idx] = TRANSIT_NETWORK;
        sys.agents.current_node[idx] = 0;
        sys.agents.current_edge[idx] = 0;
        sys.agents.current_lane_id[idx] = old_lane_id;
        sys.agents.lane_distance[idx] = old_lane_d;
        sys.agents.speed[idx] = 10.0;
        sys.agents.transit_mode[idx] = MODE_CAR;

        let split_node = graph.add_node(
            Vector3::new(50.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let old_edge = graph.edge(0).clone();
        {
            let first = graph.edge_mut(0);
            first.end_node = split_node;
            first.geometry = vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, 0.0, 0.0)];
            first.physical_geometry = first.geometry.clone();
            first.physical_length = 50.0;
            first.base_cost = 0.5;
        }
        let mut second = old_edge;
        second.start_node = split_node;
        second.geometry = vec![Vector3::new(50.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)];
        second.physical_geometry = second.geometry.clone();
        second.physical_length = 50.0;
        second.base_cost = 0.5;
        let second_edge = graph.add_edge(second);
        graph.rebuild_adjacency_list();

        let affected = HashSet::from([0usize, second_edge]);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system, &graph);
        assert_eq!(sys.agents.current_lane_id[idx], usize::MAX);

        lane_system.rebuild_edges_incremental(&mut graph, &affected);
        sys.reattach_invalidated_lanes_for_edges(&affected, &lane_system, &graph);

        let new_lane_id = sys.agents.current_lane_id[idx];
        assert_ne!(new_lane_id, usize::MAX);
        let new_lane = &lane_system.lanes[new_lane_id];
        assert_eq!(new_lane.edge_id, second_edge);
        assert!(new_lane.is_fwd);
        assert_eq!(sys.agents.current_node[idx], split_node);
        assert!(
            (sys.agents.lane_distance[idx] - 25.0).abs() < 0.5,
            "expected preserved progress on the second split edge, got lane_d={:.2}",
            sys.agents.lane_distance[idx],
        );
        let new_pos = sample_lane_position_xz(new_lane, sys.agents.lane_distance[idx]).unwrap();
        assert!(
            old_pos.distance_to(new_pos) < 0.5,
            "reattach moved the car too far: old={old_pos:?} new={new_pos:?}",
        );
    }

    #[test]
    fn reattach_holds_a_car_whose_spot_is_taken() {
        // Two cars 1 m apart, as when an edit merges two lanes into one: the first reattaches,
        // the second waits at its point for an attach gap instead of stacking (`TRAFFIC-11`).
        let (mut graph, mut lane_system) = make_simple_lane_system();
        let lane = lane_system.edge_lanes[&0]
            .iter()
            .copied()
            .find(|&id| {
                let lane = &lane_system.lanes[id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd
            })
            .expect("forward vehicle lane on edge 0");
        let mut sys = AgentSystem::new();
        for d in [75.0, 76.0] {
            let pos = sample_lane_position_xz(&lane_system.lanes[lane], d).unwrap();
            let i = sys.spawn_border_arrival_agent(usize::MAX, 0, pos.x, pos.y);
            sys.agents.transit_mode[i] = MODE_CAR;
            sys.agents.enter_lane(i, &lane_system, lane, d, 0);
            sys.agents.speed[i] = 10.0;
        }
        let affected = HashSet::from([0usize]);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system, &graph);
        lane_system.rebuild_edges_incremental(&mut graph, &affected);
        sys.reattach_invalidated_lanes_for_edges(&affected, &lane_system, &graph);

        let a = &sys.agents;
        assert_ne!(a.current_lane_id[0], usize::MAX);
        assert_eq!(a.pending_lane_attach(0), None);
        assert_eq!(a.current_lane_id[1], usize::MAX);
        let (held_lane, held_d) = a.pending_lane_attach(1).expect("second car holds");
        assert_eq!(held_lane as usize, a.current_lane_id[0]);
        assert!((held_d - 76.0).abs() < 0.5, "{held_d}");
        assert_eq!((a.transit[1], a.speed[1]), (TRANSIT_NETWORK, 0.0));
    }

    #[test]
    fn nearby_road_addition_reattaches_walkers_to_sidewalks() {
        let (mut graph, mut lanes) = make_simple_lane_system();
        let mut sys = AgentSystem::new();
        let old_ids: Vec<_> = lanes.edge_lanes[&0]
            .iter()
            .copied()
            .filter(|&id| lanes.lanes[id].lane_type == LaneType::Foot)
            .collect();
        for &id in &old_ids {
            let lane = &lanes.lanes[id];
            let node = if lane.is_fwd { 0 } else { 1 };
            let pos = sample_lane_position_xz(lane, 40.0).unwrap();
            let i = sys.spawn_housed_agent(usize::MAX, pos.x, pos.y);
            sys.agents.transit_mode[i] = MODE_WALK;
            sys.agents.enter_lane(i, &lanes, id, 40.0, node);
        }
        // Add a branch at the neighbouring junction; the walkers' own edge is unchanged.
        let end = graph.add_node(
            Vector3::new(100.0, 0.0, 100.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let mut branch = graph.edge(1).clone();
        branch.end_node = end;
        branch.geometry = vec![
            Vector3::new(100.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 100.0),
        ];
        branch.physical_geometry = branch.geometry.clone();
        let added = graph.add_edge(branch);
        graph.rebuild_adjacency_list();
        let dirty = HashSet::from([added]);
        sys.invalidate_lane_ids_for_edges(&dirty, &lanes, &graph);
        assert!(sys.current_lane_id.iter().all(|&id| id == usize::MAX));
        lanes.rebuild_edges_incremental(&mut graph, &dirty);
        sys.reattach_invalidated_lanes_for_edges(&dirty, &lanes, &graph);
        for (i, &old_id) in old_ids.iter().enumerate() {
            let id = sys.current_lane_id[i];
            assert_ne!(id, old_id);
            let lane = super::super::lane_state::lane_for_mode(&lanes, id, MODE_WALK).unwrap();
            assert_eq!(lane.edge_id, 0);
            super::super::lane_state::validate_lane_owner(
                lane,
                sys.current_edge[i],
                sys.current_node[i],
                sys.transit[i],
            )
            .unwrap();
        }
    }

    #[test]
    fn test_reattach_clears_stale_access_plan_after_road_edit() {
        let (graph, lane_system) = make_simple_lane_system();
        let current_lane = lane_system.edge_lanes[&0]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd
            })
            .expect("forward vehicle lane on edge 0");
        let detach_lane = lane_system.edge_lanes[&1]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd
            })
            .expect("forward vehicle lane on edge 1");

        let mut sys = AgentSystem::new();
        let idx = sys.spawn_border_arrival_agent(usize::MAX, 0, 10.0, 0.0);
        sys.agents.transit[idx] = TRANSIT_NETWORK;
        sys.agents.transit_mode[idx] = MODE_CAR;
        sys.agents.current_node[idx] = 0;
        sys.agents.current_edge[idx] = 0;
        sys.agents.current_lane_id[idx] = current_lane;
        sys.agents.lane_distance[idx] = 10.0;
        sys.agents.target_building[idx] = 123;
        sys.agents.planned_attach_node[idx] = 1;
        sys.agents.planned_detach_node[idx] = 2;
        sys.agents.planned_attach_lane_id[idx] = current_lane as u32;
        sys.agents.planned_detach_lane_id[idx] = detach_lane as u32;
        sys.agents.planned_attach_lane_d[idx] = 10.0;
        sys.agents.planned_detach_lane_d[idx] = 20.0;
        sys.agents.access_flags[idx] =
            ACCESS_PLAN_VALID | ACCESS_ZERO_HOP_NODE_PATH | ACCESS_PATH_FROM_FLOW_FIELD;
        sys.agents.next_replan_time[idx] = 123.0;
        sys.agents.current_path[idx] = vec![0, 1, 2];
        sys.agents.current_path_index[idx] = 1;

        let affected = HashSet::from([0usize]);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system, &graph);
        sys.reattach_invalidated_lanes_for_edges(&affected, &lane_system, &graph);

        assert_ne!(sys.agents.current_lane_id[idx], usize::MAX);
        assert_eq!(sys.agents.transit[idx], TRANSIT_NETWORK);
        assert!(sys.agents.current_path[idx].is_empty());
        assert_eq!(sys.agents.current_path_index[idx], 0);
        assert_eq!(sys.agents.planned_attach_node[idx], u32::MAX);
        assert_eq!(sys.agents.planned_attach_lane_id[idx], u32::MAX);
        assert_eq!(sys.agents.planned_attach_lane_d[idx], 0.0);
        assert_eq!(sys.agents.planned_detach_node[idx], u32::MAX);
        assert_eq!(sys.agents.planned_detach_lane_id[idx], u32::MAX);
        assert_eq!(sys.agents.planned_detach_lane_d[idx], 0.0);
        assert_eq!(sys.agents.access_flags[idx], 0);
        assert_eq!(sys.agents.next_replan_time[idx], 0.0);
    }

    #[test]
    fn test_route_clear_drops_intersection_connector_lane_after_road_edit() {
        let TurnConnectorFixture {
            lane_system,
            n_west,
            n_center,
            inbound_lane,
            connector_lane,
            ..
        } = make_turn_connector_lane_system();

        let mut sys = AgentSystem::new();
        let idx = sys.spawn_border_arrival_agent(usize::MAX, n_west, 100.0, 0.0);
        sys.agents.transit[idx] = TRANSIT_INTERSECTION;
        sys.agents.transit_mode[idx] = MODE_CAR;
        sys.agents.current_node[idx] = n_center;
        sys.agents.current_edge[idx] = usize::MAX;
        sys.agents.current_lane_id[idx] = connector_lane;
        sys.agents.lane_distance[idx] = lane_system.lanes[connector_lane].length * 0.5;
        sys.agents.speed[idx] = 3.5;
        sys.agents.target_building[idx] = 123;
        sys.agents.planned_attach_node[idx] = u32::MAX;
        sys.agents.planned_detach_node[idx] = n_center;
        sys.agents.planned_attach_lane_id[idx] = u32::MAX;
        sys.agents.planned_detach_lane_id[idx] = inbound_lane as u32;
        sys.agents.planned_detach_lane_d[idx] = 10.0;
        sys.agents.access_flags[idx] = ACCESS_PLAN_VALID | ACCESS_ZERO_HOP_NODE_PATH;
        sys.agents.next_replan_time[idx] = 77.0;
        sys.agents.network_replan_failures[idx] = 2;
        sys.agents.current_path[idx] = vec![n_center];
        sys.agents.current_path_index[idx] = 1;

        sys.clear_route_plan_after_road_edit(idx, &lane_system);

        assert_eq!(sys.agents.transit[idx], TRANSIT_NETWORK);
        assert_eq!(sys.agents.current_node[idx], n_center);
        assert_eq!(sys.agents.current_edge[idx], usize::MAX);
        assert_eq!(sys.agents.current_lane_id[idx], usize::MAX);
        assert_eq!(sys.agents.lane_distance[idx], 0.0);
        assert_eq!(sys.agents.speed[idx], 0.0);
        assert!(sys.agents.current_path[idx].is_empty());
        assert_eq!(sys.agents.current_path_index[idx], 0);
        assert_eq!(sys.agents.planned_attach_node[idx], u32::MAX);
        assert_eq!(sys.agents.planned_attach_lane_id[idx], u32::MAX);
        assert_eq!(sys.agents.planned_detach_node[idx], u32::MAX);
        assert_eq!(sys.agents.planned_detach_lane_id[idx], u32::MAX);
        assert_eq!(sys.agents.planned_detach_lane_d[idx], 0.0);
        assert_eq!(sys.agents.access_flags[idx], 0);
        assert_eq!(sys.agents.next_replan_time[idx], 0.0);
        assert_eq!(sys.agents.network_replan_failures[idx], 0);
    }

    #[test]
    fn test_reattach_preserves_freight_border_replan_marker_after_road_edit() {
        let (graph, lane_system) = make_simple_lane_system();
        let current_lane = lane_system.edge_lanes[&0]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd
            })
            .expect("forward vehicle lane on edge 0");
        let border_node = graph.edge(1).end_node;

        let mut sys = AgentSystem::new();
        let idx = sys.spawn_border_arrival_agent(usize::MAX, 0, 10.0, 0.0);
        sys.agents.transit[idx] = TRANSIT_NETWORK;
        sys.agents.transit_mode[idx] = MODE_CAR;
        sys.agents.current_node[idx] = 0;
        sys.agents.current_edge[idx] = 0;
        sys.agents.current_lane_id[idx] = current_lane;
        sys.agents.lane_distance[idx] = 10.0;
        sys.agents.target_building[idx] = usize::MAX;
        sys.agents.freight_target_border_node[idx] = border_node;
        sys.agents.planned_detach_node[idx] = border_node;
        sys.agents.planned_detach_lane_id[idx] = u32::MAX;
        sys.agents.access_flags[idx] = ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION;
        sys.agents.next_replan_time[idx] = 123.0;
        sys.agents.current_path[idx] = vec![0, 1, border_node];
        sys.agents.current_path_index[idx] = 1;

        let affected = HashSet::from([0usize]);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system, &graph);
        sys.reattach_invalidated_lanes_for_edges(&affected, &lane_system, &graph);

        assert_ne!(sys.agents.current_lane_id[idx], usize::MAX);
        assert!(sys.agents.current_path[idx].is_empty());
        assert_eq!(sys.agents.current_path_index[idx], 0);
        assert_eq!(sys.agents.planned_attach_node[idx], u32::MAX);
        assert_eq!(sys.agents.planned_attach_lane_id[idx], u32::MAX);
        assert_eq!(sys.agents.planned_detach_node[idx], border_node);
        assert_eq!(sys.agents.planned_detach_lane_id[idx], u32::MAX);
        assert_eq!(
            sys.agents.access_flags[idx],
            ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION
        );
        assert_eq!(sys.agents.next_replan_time[idx], 0.0);
    }

    #[test]
    fn test_reattach_repairs_agent_invalidated_inside_connector() {
        let TurnConnectorFixture {
            mut graph,
            mut lane_system,
            n_west,
            n_center,
            north_edge,
            connector_lane,
            ..
        } = make_turn_connector_lane_system();
        let old_lane_d = lane_system.lanes[connector_lane].length * 0.5;
        let old_pos =
            sample_lane_position_xz(&lane_system.lanes[connector_lane], old_lane_d).unwrap();

        let mut sys = AgentSystem::new();
        let idx = sys.spawn_border_arrival_agent(usize::MAX, n_west, old_pos.x, old_pos.y);
        sys.agents.transit[idx] = TRANSIT_INTERSECTION;
        sys.agents.transit_mode[idx] = MODE_CAR;
        sys.agents.current_node[idx] = n_center;
        sys.agents.current_edge[idx] = usize::MAX;
        sys.agents.current_lane_id[idx] = connector_lane;
        sys.agents.lane_distance[idx] = old_lane_d;
        sys.agents.speed[idx] = 6.0;

        let affected = HashSet::from([north_edge]);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system, &graph);
        assert_eq!(sys.agents.current_lane_id[idx], usize::MAX);
        assert_ne!(
            sys.agents.current_edge[idx],
            usize::MAX,
            "connector invalidation must preserve a physical edge for reattach"
        );

        lane_system.rebuild_edges_incremental(&mut graph, &affected);
        sys.reattach_invalidated_lanes_for_edges(&affected, &lane_system, &graph);

        let new_lane_id = sys.agents.current_lane_id[idx];
        assert_ne!(
            new_lane_id,
            usize::MAX,
            "agent invalidated inside a connector should reattach to a rebuilt physical lane"
        );
        assert_ne!(lane_system.lanes[new_lane_id].edge_id, usize::MAX);
        let new_pos = sample_lane_position_xz(
            &lane_system.lanes[new_lane_id],
            sys.agents.lane_distance[idx],
        )
        .unwrap();
        assert!(
            old_pos.distance_to(new_pos) <= LANE_REATTACH_MAX_DIST_M,
            "reattach moved connector agent too far: old={old_pos:?} new={new_pos:?}",
        );
    }

    // The connector from `inbound` toward `target_edge`, as the fixture finds it.
    fn live_connector(lane_system: &LaneSystem, inbound: usize, target_edge: usize) -> usize {
        lane_system.lanes[inbound]
            .next_lanes
            .iter()
            .copied()
            .find(|&id| {
                let lane = &lane_system.lanes[id];
                lane.edge_id == usize::MAX
                    && lane.lane_type == LaneType::Vehicle
                    && lane
                        .next_lanes
                        .first()
                        .is_some_and(|&to| lane_system.lanes[to].edge_id == target_edge)
            })
            .expect("live connector")
    }

    // The live west-to-north connector; the west edge keeps its id across rebuilds.
    fn west_to_north_connector(fixture: &TurnConnectorFixture) -> usize {
        let lanes = &fixture.lane_system;
        let west_edge = lanes.lanes[fixture.inbound_lane].edge_id;
        let inbound = lanes.edge_lanes[&west_edge]
            .iter()
            .copied()
            .find(|&id| lanes.lanes[id].lane_type == LaneType::Vehicle && lanes.lanes[id].is_fwd)
            .expect("west inbound lane");
        live_connector(lanes, inbound, fixture.north_edge)
    }

    // An agent inside a junction, with an earlier incremental rebuild leaving tombstones so the
    // full rebuild renumbers its connector (as terrain edits and undo do in the live game).
    fn agent_in_renumbered_connector() -> (TurnConnectorFixture, AgentSystem, usize, usize) {
        let mut fixture = make_turn_connector_lane_system();
        fixture
            .lane_system
            .rebuild_edges_incremental(&mut fixture.graph, &HashSet::from([fixture.north_edge]));
        let connector = west_to_north_connector(&fixture);

        let mut sys = AgentSystem::new();
        let idx = sys.spawn_border_arrival_agent(usize::MAX, fixture.n_west, 0.0, 0.0);
        sys.agents.transit[idx] = TRANSIT_INTERSECTION;
        sys.agents.transit_mode[idx] = MODE_CAR;
        sys.agents.current_node[idx] = fixture.n_center;
        sys.agents.current_edge[idx] = usize::MAX;
        sys.agents.current_lane_id[idx] = connector;
        sys.agents.lane_distance[idx] = 1.0;
        sys.agents.current_path[idx] = vec![fixture.n_center];
        (fixture, sys, idx, connector)
    }

    #[test]
    fn full_lane_rebuild_keeps_agent_inside_its_junction_connector() {
        let (mut fixture, mut sys, idx, old_connector) = agent_in_renumbered_connector();

        let captured = sys.capture_lane_identities(&fixture.lane_system);
        fixture.lane_system.rebuild(&mut fixture.graph);
        sys.restore_lane_identities(&captured, &fixture.lane_system);

        let expected = west_to_north_connector(&fixture);
        assert_ne!(
            expected, old_connector,
            "the fixture must renumber the connector"
        );
        assert_eq!(sys.agents.current_lane_id[idx], expected);
        assert_eq!(
            fixture.lane_system.lanes[expected].node_id,
            fixture.n_center as usize
        );
        assert_eq!(sys.agents.transit[idx], TRANSIT_INTERSECTION);
        assert_eq!(sys.agents.current_path[idx], vec![fixture.n_center]);
    }

    #[test]
    fn full_lane_rebuild_drops_agent_from_a_connector_that_no_longer_exists() {
        let (mut fixture, mut sys, idx, _) = agent_in_renumbered_connector();

        let captured = sys.capture_lane_identities(&fixture.lane_system);
        // Undo removing the road the agent was turning onto.
        fixture.graph.edge_mut(fixture.north_edge).deleted = true;
        fixture.graph.rebuild_adjacency_list();
        fixture.lane_system.rebuild(&mut fixture.graph);
        sys.restore_lane_identities(&captured, &fixture.lane_system);

        assert_eq!(sys.agents.current_lane_id[idx], usize::MAX);
        assert_eq!(sys.agents.transit[idx], TRANSIT_NETWORK);
        assert!(sys.agents.current_path[idx].is_empty());
    }
}
