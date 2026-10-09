// SPDX-License-Identifier: GPL-2.0-only

//! Reusable exact commute quotations, including directed pedestrian connector fallback.

use super::*;
use std::hash::BuildHasher;
use crate::simulation::pathing::cch::CchPathScratch;

thread_local! {
    // Derived route scratch is private to each Rayon/caller thread, never simulation state.
    static QUERY: std::cell::RefCell<TripQueryScratch> = Default::default();
}

/// Worker-local route storage; preparing once keeps repeated commute estimates allocation-free.
#[derive(Default)]
pub(crate) struct TripQueryScratch {
    cch: CchPathScratch,
    possible: Vec<usize>,
    next: Vec<usize>,
    heap: BinaryHeap<PedestrianSearchState>,
    best: Vec<(u64, f32)>,
    previous: Vec<usize>,
    path: Vec<u32>,
    retained_path: Vec<u32>,
    last_path_is_walking: bool,
    route_revision: Option<(u32, usize)>,
    epoch: u64,
    revision: Option<(u32, usize)>,
    network_quotes: Vec<Option<NetworkQuote>>,
    quote_revision: Option<(u32, u64, usize)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct NetworkQuoteKey {
    mode: u8,
    from_lane: usize,
    to_lane: usize,
    from_node: u32,
    to_node: u32,
}

#[derive(Clone, Copy)]
struct NetworkQuote {
    key: NetworkQuoteKey,
    seconds: Option<f32>,
}

/// Validated source of a pedestrian route, including agents already on the network.
#[derive(Clone, Copy)]
pub(in super::super) enum RouteStart {
    /// A known incoming lane at the attachment node.
    Lane(usize),
    /// Any incoming foot lane on this incident edge.
    Edge(usize),
    /// A free node origin with no incoming edge constraint.
    Node,
}

impl TripQueryScratch {
    /// Borrows route scratch without preparing a graph for invalid or zero-hop plans.
    /// Methods that search or reconstruct paths prepare their required storage lazily.
    pub(in super::super) fn with_routes<T>(plan: impl FnOnce(&mut Self) -> T) -> T {
        QUERY.with_borrow_mut(plan)
    }

    pub(in super::super) fn with<T>(
        network: &TransitNetwork,
        graph: &RegionGraph,
        quote: impl FnOnce(&mut Self) -> T,
    ) -> T {
        QUERY.with_borrow_mut(|scratch| {
            scratch.prepare(network, graph);
            quote(scratch)
        })
    }

    /// Reserves lane/connector scratch and degree-weighted CCH arc storage at a topology
    /// boundary; the CCH heap bound is documented by `CchQueryScratch::prepare`.
    pub(crate) fn prepare(&mut self, network: &TransitNetwork, graph: &RegionGraph) {
        self.cch.prepare(&network.cch_graph, graph);
        let lanes = &network.lane_system.lanes;
        let quote_revision = (network.cch_graph.build_generation,
            network.cch_graph.metric_revision, lanes.len());
        if self.quote_revision != Some(quote_revision) {
            // Direct slots bound retained pairs by road storage, never homes × workplaces.
            // Collisions only replace retained results: every miss uses the exact planner.
            let slots = (2 * graph.edge_count() + graph.node_count()).max(1).next_power_of_two();
            self.network_quotes.resize(slots, None);
            self.network_quotes.fill(None);
            self.quote_revision = Some(quote_revision);
        }
        let revision = (network.cch_graph.build_generation, lanes.len());
        if self.revision == Some(revision) {
            return;
        }
        self.best.resize(lanes.len(), (0, f32::INFINITY));
        self.best.fill((0, f32::INFINITY));
        self.epoch = 0;
        let links = lanes
            .iter()
            .map(|lane| lane.next_lanes.len())
            .sum::<usize>()
            + lanes.len() + 1;
        self.heap.clear();
        self.heap.reserve(links);
        for values in [&mut self.possible, &mut self.next] {
            values.clear();
            values.reserve(links);
        }
        self.revision = Some(revision);
    }

    /// Prepares predecessor and retained-path buffers only for full route work. Cost-only
    /// forecasts keep their existing storage; warm route queries merely check the revision.
    pub(in super::super) fn prepare_routes(&mut self, network: &TransitNetwork, graph: &RegionGraph) {
        self.prepare(network, graph);
        if self.route_revision == self.revision { return; }
        let lanes = network.lane_system.lanes.len();
        self.previous.resize(lanes, usize::MAX);
        self.path.clear();
        self.path.reserve((lanes + 1).max(graph.node_count() + 2));
        self.retained_path.clear();
        self.retained_path.reserve((lanes + 1)
            .max(2 * graph.edge_count() + 2 * graph.node_count() + 2));
        self.route_revision = self.revision;
    }

    pub(in super::super) fn combine(
        &mut self,
        mode: u8,
        origin_rank: u8,
        destination_rank: u8,
        origin: &TripEnd,
        destination: &TripEnd,
        network: &TransitNetwork,
        graph: &RegionGraph,
        pathfind_count: &AtomicU32,
    ) -> Option<PlannedTripCandidate> {
        // The caller has already priced shared frontages/nodes without a search.
        debug_assert!(origin.node != destination.node);
        let key = NetworkQuoteKey { mode, from_lane: origin.lane_id, to_lane: destination.lane_id,
            from_node: origin.node, to_node: destination.node };
        let slot = foldhash::fast::FixedState::default().hash_one(key) as usize
            & (self.network_quotes.len() - 1);
        let seconds = if let Some(quote) = self.network_quotes[slot].filter(|quote| quote.key == key) {
            quote.seconds
        } else {
            let seconds = self.network_seconds(mode, origin, destination, network, graph, pathfind_count);
            self.network_quotes[slot] = Some(NetworkQuote { key, seconds });
            seconds
        }?;
        finish_candidate(
            mode,
            origin_rank,
            destination_rank,
            origin,
            destination,
            origin.local_time_s
                + origin.frontage_time_s?
                + seconds
                + destination.frontage_time_s?
                + destination.local_time_s,
        )
    }

    /// Prices a CCH leg in prepared storage without retaining an owned route.
    pub(in super::super) fn cch_cost(&mut self, from: u32, to: u32, edge: usize,
        mode: u8, network: &TransitNetwork, graph: &RegionGraph) -> Option<f32> {
        self.prepare_routes(network, graph);
        self.last_path_is_walking = false;
        network.cch_graph.find_path_in(from, to, edge, graph,
            transit_flags_for_mode(mode), &mut self.cch).map(|(seconds, _, _)| seconds)
    }

    /// Visits the exact selected CCH/connector path while temporary storage stays borrowed.
    /// Candidate pricing discards the slice; only a stored agent plan copies its final route.
    pub(in super::super) fn network_path<T>(&mut self, mode: u8, from: u32, to: u32,
        start: RouteStart, detach: usize, network: &TransitNetwork, graph: &RegionGraph,
        consume: impl FnOnce(&[u32]) -> T) -> Result<(f32, T), &'static str> {
        self.prepare_routes(network, graph);
        self.last_path_is_walking = false;
        let edge = match start {
            RouteStart::Lane(lane) => network.lane_system.lanes.get(lane)
                .map_or(usize::MAX, |lane| lane.edge_id),
            RouteStart::Edge(edge) => edge,
            RouteStart::Node => usize::MAX,
        };
        let (seconds, _, path) = network.cch_graph.find_path_in(from, to, edge, graph,
            transit_flags_for_mode(mode), &mut self.cch).ok_or("cch_no_path")?;
        if path.len() < 2 { return Err("path_too_short"); }
        if mode == MODE_CAR || path_has_connectors_from(path, from, start, detach,
            network, graph, &mut self.possible, &mut self.next) {
            return Ok((seconds, consume(path)));
        }
        self.last_path_is_walking = true;
        let (seconds, path) = self.walking_path(from, to, start, detach, network, graph)
            .ok_or("pedestrian_connector_fallback_missing")?;
        Ok((seconds, consume(path)))
    }

    /// Retains the last successful path when its candidate wins, without allocating or
    /// repeating its search. A direct zero-hop candidate withdraws the preceding path.
    pub(in super::super) fn retain_network_path(&mut self, zero_hop: bool) {
        self.retained_path.clear();
        if !zero_hop {
            let path = if self.last_path_is_walking { &self.path } else { self.cch.nodes() };
            assert!(path.len() <= self.retained_path.capacity(), "route storage must be prepared");
            self.retained_path.extend_from_slice(path);
        }
    }

    /// Allocates only the selected persistent route product after candidate comparison ends.
    pub(in super::super) fn finish_path(&self) -> Vec<u32> { self.retained_path.clone() }

    /// Checks an existing route using the prepared lane frontier, including free node origins.
    pub(in super::super) fn has_connectors(&mut self, path: &[u32], from: u32,
        start: RouteStart, detach: usize, network: &TransitNetwork, graph: &RegionGraph) -> bool {
        if path.len() < 2 {
            return match start {
                RouteStart::Lane(lane) => connector_to_lane_exists(lane, detach, network),
                RouteStart::Node => path.first().copied() == Some(from)
                    && lane_origin_node(detach, network, graph) == Some(from),
                RouteStart::Edge(edge) => incoming_lanes(from, edge, network, graph)
                    .any(|lane| lane == detach || connector_to_lane_exists(lane, detach, network)),
            };
        }
        self.prepare(network, graph);
        path_has_connectors_from(path, from, start, detach, network, graph,
            &mut self.possible, &mut self.next)
    }

    /// Visits a flow-field route only after validating its destination, turns and connectors.
    pub(in super::super) fn flow_path<T>(&mut self,
        field: &crate::simulation::pathing::flow_field::FlowField,
        candidate: &PlannedTripCandidate, network: &TransitNetwork, graph: &RegionGraph,
        consume: impl FnOnce(&[u32]) -> T) -> Option<T> {
        self.prepare_routes(network, graph);
        field.build_path_in(candidate.planned_attach_node, graph.node_count() + 1, &mut self.path)?;
        let path = &self.path;
        if path.last().copied() != Some(candidate.planned_detach_node) { return None; }
        let valid = if candidate.mode == MODE_CAR {
            crate::simulation::pathing::cch::CchGraph::path_has_valid_vehicle_turns(path, graph)
        } else {
            path_has_connectors(path, candidate.planned_attach_lane_id,
                candidate.planned_detach_lane_id, network, graph, &mut self.possible, &mut self.next)
        };
        valid.then(|| consume(path))
    }

    /// Reconstructs an exact connector fallback in reusable dense lane labels and path storage.
    /// Seed/relaxation order and strict cost comparison match the original heap search.
    pub(in super::super) fn walking_path(&mut self, from: u32, to: u32, start: RouteStart,
        detach: usize, network: &TransitNetwork, graph: &RegionGraph) -> Option<(f32, &[u32])> {
        self.prepare_routes(network, graph);
        self.path.clear();
        if from == to {
            let reachable = match start {
                RouteStart::Node => lane_origin_node(detach, network, graph) == Some(from),
                _ => {
                    seed_incoming_lanes(&mut self.possible, from, start, network, graph);
                    self.possible.iter().any(|&lane| connector_to_lane_exists(lane, detach, network))
                }
            };
            if !reachable { return None; }
            self.path.push(from);
            return Some((0.0, &self.path));
        }
        self.heap.clear();
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 { self.best.fill((0, f32::INFINITY)); self.epoch = 1; }
        if matches!(start, RouteStart::Node) {
            if from as usize >= graph.node_adjacency_count() { return None; }
            for &edge in graph.node_adjacency(from) {
                for lane in outgoing_lanes(from, edge, network, graph) {
                    let Some(node) = lane_terminal_node(lane, network, graph) else { continue; };
                    let value = &network.lane_system.lanes[lane];
                    let cost = graph.edges().get(value.edge_id).map_or(
                        value.length / WALK_CONNECTOR_COST_SPEED_MS, |edge| edge.base_cost).max(0.001);
                    let best = &mut self.best[lane];
                    if cost < if best.0 == self.epoch { best.1 } else { f32::INFINITY } {
                        *best = (self.epoch, cost);
                        self.previous[lane] = usize::MAX;
                        self.heap.push(PedestrianSearchState { cost, node, incoming_lane: lane });
                    }
                }
            }
        } else {
            seed_incoming_lanes(&mut self.possible, from, start, network, graph);
            for &lane in &self.possible {
                let best = self.best.get_mut(lane)?;
                *best = (self.epoch, 0.0);
                self.previous[lane] = usize::MAX;
                self.heap.push(PedestrianSearchState { cost: 0.0, node: from, incoming_lane: lane });
            }
        }
        while let Some(state) = self.heap.pop() {
            if state.cost > self.best[state.incoming_lane].1 { continue; }
            if state.node == to && connector_to_lane_exists(state.incoming_lane, detach, network) {
                self.path.push(state.node);
                let mut lane = state.incoming_lane;
                while self.previous[lane] != usize::MAX {
                    lane = self.previous[lane];
                    self.path.push(lane_terminal_node(lane, network, graph)?);
                }
                if matches!(start, RouteStart::Node) { self.path.push(from); }
                self.path.reverse();
                return Some((state.cost, &self.path));
            }
            visit_pedestrian_successors(&state, network, graph, |next| {
                let best = &mut self.best[next.incoming_lane];
                if next.cost < if best.0 == self.epoch { best.1 } else { f32::INFINITY } {
                    *best = (self.epoch, next.cost);
                    self.previous[next.incoming_lane] = state.incoming_lane;
                    self.heap.push(next);
                }
            });
        }
        None
    }

    fn network_seconds(&mut self, mode: u8, origin: &TripEnd, destination: &TripEnd,
        network: &TransitNetwork, graph: &RegionGraph, pathfind_count: &AtomicU32) -> Option<f32> {
        pathfind_count.fetch_add(1, Ordering::Relaxed);
        let edge = network.lane_system.lanes.get(origin.lane_id)?.edge_id;
        let (mut seconds, _, path) = network.cch_graph.find_path_in(
            origin.node,
            destination.node,
            edge,
            graph,
            transit_flags_for_mode(mode),
            &mut self.cch,
        )?;
        if mode != MODE_CAR
            && !path_has_connectors(
                path,
                origin.lane_id,
                destination.lane_id,
                network,
                graph,
                &mut self.possible,
                &mut self.next,
            )
        {
            seconds = self.walking_cost(origin, destination, network, graph)?;
        }
        Some(seconds)
    }

    fn walking_cost(
        &mut self,
        origin: &TripEnd,
        destination: &TripEnd,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        self.heap.clear();
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.best.fill((0, f32::INFINITY));
            self.epoch = 1;
        }
        self.best[origin.lane_id] = (self.epoch, 0.0);
        self.heap.push(PedestrianSearchState {
            cost: 0.0,
            node: origin.node,
            incoming_lane: origin.lane_id,
        });
        while let Some(state) = self.heap.pop() {
            if state.cost > self.best[state.incoming_lane].1 {
                continue;
            }
            if state.node == destination.node
                && connector_to_lane_exists(state.incoming_lane, destination.lane_id, network)
            {
                return Some(state.cost);
            }
            visit_pedestrian_successors(&state, network, graph, |next| {
                let entry = &mut self.best[next.incoming_lane];
                if next.cost < if entry.0 == self.epoch { entry.1 } else { f32::INFINITY } {
                    *entry = (self.epoch, next.cost);
                    self.heap.push(next);
                }
            });
        }
        None
    }
}

fn path_has_connectors(
    path: &[u32],
    attach: usize,
    detach: usize,
    network: &TransitNetwork,
    graph: &RegionGraph,
    possible: &mut Vec<usize>,
    next: &mut Vec<usize>,
) -> bool {
    path_has_connectors_from(path, path.first().copied().unwrap_or(u32::MAX),
        RouteStart::Lane(attach), detach, network, graph, possible, next)
}

fn outgoing_lanes<'a>(node: u32, edge: usize, network: &'a TransitNetwork,
    graph: &'a RegionGraph) -> impl Iterator<Item = usize> + 'a {
    network.lane_system.edge_lanes.get(&edge).into_iter().flatten().copied()
        .filter(move |&lane| network.lane_system.lanes.get(lane)
            .is_some_and(|value| value.lane_type == LaneType::Foot)
            && lane_origin_node(lane, network, graph) == Some(node))
}

fn seed_incoming_lanes(out: &mut Vec<usize>, node: u32, start: RouteStart,
    network: &TransitNetwork, graph: &RegionGraph) {
    out.clear();
    match start {
        RouteStart::Lane(lane) => out.push(lane),
        RouteStart::Edge(edge) => out.extend(incoming_lanes(node, edge, network, graph)),
        _ => {}
    }
}

fn incoming_lanes<'a>(node: u32, edge: usize, network: &'a TransitNetwork,
    graph: &'a RegionGraph) -> impl Iterator<Item = usize> + 'a {
    network.lane_system.edge_lanes.get(&edge).filter(|_| edge != usize::MAX)
        .into_iter().flatten().copied().filter(move |&lane| network.lane_system.lanes.get(lane)
            .is_some_and(|value| value.lane_type == LaneType::Foot)
            && lane_terminal_node(lane, network, graph) == Some(node))
}

fn path_has_connectors_from(mut path: &[u32], from: u32, start: RouteStart, detach: usize,
    network: &TransitNetwork, graph: &RegionGraph, possible: &mut Vec<usize>,
    next: &mut Vec<usize>) -> bool {
    possible.clear();
    match start {
        RouteStart::Node => {
            if path.first().copied() != Some(from) { return false; }
            if path.len() < 2 { return lane_origin_node(detach, network, graph) == Some(from); }
            let Some(edge) = graph.get_edge_between_nodes(from, path[1]) else { return false; };
            possible.extend(outgoing_lanes(from, edge, network, graph));
            path = &path[1..];
        }
        RouteStart::Lane(lane) if path.len() < 2 => return connector_to_lane_exists(lane, detach, network),
        _ => seed_incoming_lanes(possible, from, start, network, graph),
    }
    for pair in path.windows(2) {
        let Some(edge) = graph.get_edge_between_nodes(pair[0], pair[1]) else {
            return false;
        };
        next.clear();
        for &lane in possible.iter() {
            collect_connector_targets_to_edge(lane, pair[0], edge, network, graph, next);
        }
        if next.is_empty() {
            return false;
        }
        next.sort_unstable();
        next.dedup();
        std::mem::swap(possible, next);
    }
    possible
        .iter()
        .any(|&lane| lane == detach || connector_to_lane_exists(lane, detach, network))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::types::NodeType;
    use crate::simulation::pathing::cch::CchGraph;
    use godot::prelude::Vector3;

    #[test]
    fn prepared_commute_queries_match_connector_paths_without_growing_storage() {
        let mut graph = RegionGraph::new();
        for (x, z) in [(0.0, 0.0), (100.0, 0.0), (200.0, 0.0), (300.0, 0.0), (100.0, 100.0), (200.0, 100.0)] {
            graph.add_node(Vector3::new(x, 0.0, z), NodeType::Junction);
        }
        for (from, to) in [(0, 1), (1, 2), (2, 3), (1, 4), (4, 5), (5, 2)] {
            let a = graph.node(from).pos;
            let b = graph.node(to).pos;
            let mut edge = super::super::tests::test_edge(from, to, a.x, b.x);
            edge.geometry = vec![a, b];
            edge.physical_geometry = edge.geometry.clone();
            edge.physical_length = (b - a).length();
            graph.add_edge(edge);
        }
        graph.rebuild_adjacency_list();
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        network.cch_graph = CchGraph::build(&graph);
        let mut scratch = TripQueryScratch::default();
        let foot = network.lane_system.lanes.iter().position(|lane|
            lane.edge_id != usize::MAX && lane.lane_type == LaneType::Foot).unwrap();
        let node = lane_origin_node(foot, &network, &graph).unwrap();
        assert!(scratch.has_connectors(&[node], node, RouteStart::Node, foot, &network, &graph));
        scratch.retain_network_path(true);
        assert!(scratch.finish_path().is_empty());
        assert!(scratch.revision.is_none(), "zero-hop checks must not prepare city-sized scratch");
        scratch.prepare(&network, &graph);
        assert_eq!([scratch.previous.capacity(), scratch.path.capacity(), scratch.retained_path.capacity()], [0; 3],
            "cost forecasts must not allocate full-route buffers");
        scratch.prepare_routes(&network, &graph);
        let capacities = |scratch: &TripQueryScratch| {
            [
                scratch.possible.capacity(),
                scratch.next.capacity(),
                scratch.heap.capacity(),
                scratch.best.capacity(),
                scratch.network_quotes.capacity(),
                scratch.previous.capacity(),
                scratch.path.capacity(),
                scratch.retained_path.capacity(),
            ]
        };
        let prepared = capacities(&scratch);
        let counter = AtomicU32::new(0);
        let lanes: Vec<_> = network
            .lane_system
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| lane.edge_id != usize::MAX)
            .map(|(index, _)| index)
            .collect();
        let mut fallback_count = 0;
        for change in 0..4 {
            if change == 1 {
                graph.edge_mut(1).base_cost += 17.0;
                network.cch_graph.customize(&graph);
            } else if change == 2 {
                let lane = lanes.iter().copied().find(|&lane|
                    network.lane_system.lanes[lane].lane_type == LaneType::Foot).unwrap();
                network.lane_system.lanes[lane].next_lanes.clear();
                network.rebuild_cch_and_check(&graph);
            } else if change == 3 {
                for edge in 0..graph.edge_count() { graph.edge_mut(edge).base_cost = f32::INFINITY; }
                network.cch_graph.customize(&graph);
            }
            scratch.prepare(&network, &graph);
            for &attach in &lanes {
                for &detach in &lanes {
                    let lane_type = network.lane_system.lanes[attach].lane_type;
                    if network.lane_system.lanes[detach].lane_type != lane_type { continue; }
                    let mode = if lane_type == LaneType::Vehicle { MODE_CAR } else { 0 };
                    let make_end = |lane, origin| {
                        let node = if origin {
                            lane_terminal_node(lane, &network, &graph)
                        } else {
                            lane_origin_node(lane, &network, &graph)
                        }
                        .unwrap();
                        TripEnd {
                            edge_idx: network.lane_system.lanes[lane].edge_id,
                            node,
                            node_x: graph.node(node).pos.x,
                            node_z: 0.0,
                            lane_id: lane,
                            lane_d: if origin { 1.0 } else { 0.0 },
                            local_time_s: 1.0,
                            frontage_time_s: Some(2.0),
                        }
                    };
                    let origin = make_end(attach, true);
                    let destination = make_end(detach, false);
                    if mode != MODE_CAR {
                        for (start, expected) in [
                            (RouteStart::Lane(attach), super::owned_oracle::pedestrian_lane_connector_path(
                                origin.node, destination.node, attach, detach, &network, &graph)),
                            (RouteStart::Edge(origin.edge_idx), super::owned_oracle::pedestrian_lane_connector_path_from_edge(
                                origin.node, destination.node, origin.edge_idx, detach, &network, &graph)),
                            (RouteStart::Node, super::owned_oracle::pedestrian_lane_connector_path_from_node(
                                origin.node, destination.node, detach, &network, &graph)),
                        ] {
                            let actual = scratch.walking_path(origin.node, destination.node, start,
                                detach, &network, &graph).map(|(seconds, path)| (seconds.to_bits(), path.to_vec()));
                            assert_eq!(actual, expected.map(|(seconds, path)| (seconds.to_bits(), path)),
                                "connector path: change={change}, attach={attach}, detach={detach}");
                        }
                    }
                    if origin.node == destination.node {
                        assert_eq!(capacities(&scratch), prepared);
                        continue;
                    }
                    let expected_candidate = super::owned_oracle::combine_trip_ends(
                        mode,
                        0,
                        0,
                        &origin,
                        &destination,
                        &network,
                        &graph,
                        &counter,
                    );
                    let expected = expected_candidate.as_ref().map(|candidate| candidate.total_cost_s);
                    let route = scratch.network_path(mode, origin.node, destination.node,
                        RouteStart::Lane(attach), detach, &network, &graph, <[u32]>::to_vec)
                        .ok().map(|(_, path)| path);
                    assert_eq!(route, expected_candidate.and_then(|candidate| candidate.network_path));
                    if let Some(route) = route {
                        scratch.retain_network_path(false);
                        assert_eq!(scratch.finish_path(), route);
                        scratch.retain_network_path(true);
                        assert!(scratch.finish_path().is_empty());
                    }
                    let before = scratch.epoch;
                    assert_eq!(
                        scratch
                            .combine(mode, 0, 0, &origin, &destination, &network, &graph, &counter)
                            .map(|candidate| candidate.total_cost_s),
                        expected
                    );
                    fallback_count += usize::from(scratch.epoch != before);
                    let searches = counter.load(Ordering::Relaxed);
                    assert_eq!(scratch.combine(mode, 0, 0, &origin, &destination, &network, &graph,
                        &counter).map(|candidate| candidate.total_cost_s), expected);
                    assert_eq!(counter.load(Ordering::Relaxed), searches,
                        "an identical lane leg must reuse both successful and failed proofs");
                    // Different doors and live frontage charges share only the network leg.
                    let mut moved_origin = origin;
                    moved_origin.local_time_s = 9.0;
                    moved_origin.frontage_time_s = Some(19.0);
                    let mut moved_destination = destination;
                    moved_destination.local_time_s = 7.0;
                    moved_destination.frontage_time_s = Some(23.0);
                    let moved_expected = super::owned_oracle::combine_trip_ends(mode, 0, 0, &moved_origin,
                        &moved_destination, &network, &graph, &counter)
                        .map(|candidate| candidate.total_cost_s);
                    let searches = counter.load(Ordering::Relaxed);
                    assert_eq!(scratch.combine(mode, 0, 0, &moved_origin, &moved_destination,
                        &network, &graph, &counter).map(|candidate| candidate.total_cost_s), moved_expected);
                    assert_eq!(counter.load(Ordering::Relaxed), searches);
                    assert_eq!(capacities(&scratch), prepared);
                }
            }
        }
        assert!(
            fallback_count > 0,
            "fixture must exercise the directed lane fallback"
        );
    }

    type RouteCase = (u8, [TripEnd; 2], [TripEnd; 2]);
    type RouteResult = (u32, u8, u8, usize, usize, Vec<u32>, u8);

    fn route_cases(edges: usize) -> (RegionGraph, TransitNetwork, Vec<RouteCase>) {
        let mut graph = RegionGraph::new();
        for index in 0..=edges {
            graph.add_node(Vector3::new(index as f32 * 100.0, 0.0, 0.0), NodeType::Junction);
        }
        for index in 0..edges {
            graph.add_edge(super::super::tests::test_edge(index as u32, index as u32 + 1,
                index as f32 * 100.0, (index + 1) as f32 * 100.0));
        }
        graph.rebuild_adjacency_list();
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        network.cch_graph = CchGraph::build(&graph);
        let ends = |edge: usize, mode, origin| std::array::from_fn(|rank| {
            let node = if rank == 0 { graph.edge(edge).start_node } else { graph.edge(edge).end_node };
            let lane = network.lane_system.edge_lanes[&edge].iter().copied().find(|&lane| {
                let value = &network.lane_system.lanes[lane];
                let kind = if mode == MODE_CAR { LaneType::Vehicle } else { LaneType::Foot };
                value.lane_type == kind && (if origin {
                    lane_terminal_node(lane, &network, &graph)
                } else { lane_origin_node(lane, &network, &graph) }) == Some(node)
            }).unwrap();
            TripEnd { edge_idx: edge, node, node_x: graph.node(node).pos.x, node_z: 0.0,
                lane_id: lane, lane_d: 50.0, local_time_s: 1.0, frontage_time_s: Some(2.0) }
        });
        let cases = (0..512).map(|index| {
            let mode = if index % 2 == 0 { MODE_CAR } else { 0 };
            (mode, ends(index * 7 % edges, mode, true),
                ends((index * 17 + edges / 2) % edges, mode, false))
        }).collect();
        (graph, network, cases)
    }

    fn planned_case(owned: bool, case: &RouteCase, network: &TransitNetwork,
        graph: &RegionGraph, counter: &AtomicU32) -> Option<RouteResult> {
        let (mode, origins, destinations) = case;
        let mut best = None;
        for (from, to) in NODE_RANK_PAIRS {
            let quote = if owned { super::owned_oracle::combine_trip_ends } else { combine_trip_ends };
            if let Some(candidate) = quote(*mode, from, to, &origins[from as usize],
                &destinations[to as usize], network, graph, counter)
                && best.as_ref().is_none_or(|best| candidate_better(&candidate, best)) {
                best = Some(candidate);
            }
        }
        let mut candidate = best?;
        let (path, flags) = if owned {
            let direct = candidate.planned_attach_lane_id == candidate.planned_detach_lane_id
                && candidate.planned_attach_lane_d <= candidate.planned_detach_lane_d + 1e-6;
            let flags = ACCESS_PLAN_VALID | if !direct && candidate.planned_attach_node == candidate.planned_detach_node {
                ACCESS_ZERO_HOP_NODE_PATH
            } else { 0 };
            (candidate.network_path.take().unwrap_or_default(), flags)
        } else {
            build_exact_path_for_candidate(&mut candidate, usize::MAX, ZoneType::None,
                network, graph, counter)?
        };
        Some((candidate.total_cost_s.to_bits(), candidate.origin_rank, candidate.destination_rank,
            candidate.planned_attach_lane_id, candidate.planned_detach_lane_id, path, flags))
    }

    #[test]
    fn selected_routes_match_owned_costs_ranks_paths_and_access_flags() {
        for edges in [16, 64] {
            let (graph, network, cases) = route_cases(edges);
            let counter = AtomicU32::new(0);
            for case in &cases {
                let expected = planned_case(true, case, &network, &graph, &counter);
                assert!(expected.is_some());
                assert_eq!(planned_case(false, case, &network, &graph, &counter), expected);
            }
        }
    }

    #[test]
    #[ignore = "manual release comparison of owned candidate paths against prepared route products"]
    fn benchmark_prepared_route_products() {
        use rayon::prelude::*;
        use std::{hint::black_box, time::Instant};
        for edges in [16, 64, 256, 1_024] {
            let (graph, network, cases) = route_cases(edges);
            let counter = AtomicU32::new(0);
            for case in &cases {
                let expected = planned_case(true, case, &network, &graph, &counter);
                assert!(expected.is_some());
                assert_eq!(planned_case(false, case, &network, &graph, &counter), expected);
            }
            for reverse in [false, true] {
                for owned in if reverse { [false, true] } else { [true, false] } {
                    let mut samples = [0.0_f64; 15];
                    for round in 0..18 {
                        let start = Instant::now();
                        cases.par_iter().for_each(|case| {
                            black_box(planned_case(owned, case, &network, &graph, &counter));
                        });
                        if round >= 3 { samples[round - 3] = start.elapsed().as_secs_f64() * 1_000.0; }
                    }
                    samples.sort_by(f64::total_cmp);
                    eprintln!("route_products edges={edges} trips={} owned={owned} reverse={reverse} median_ms={:.3} max_ms={:.3}",
                        cases.len(), samples[7], samples[14]);
                }
            }
        }
    }
}
