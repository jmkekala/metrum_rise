// SPDX-License-Identifier: GPL-2.0-only

//! Reusable exact commute quotations, including directed pedestrian connector fallback.

use super::*;
use crate::simulation::pathing::cch::{CchBucketSource, CchBuckets, CchPathScratch};
use std::hash::BuildHasher;

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
    connectors: ConnectorTargets,
    heap: BinaryHeap<PedestrianSearchState>,
    best: Vec<(u64, f32)>,
    previous: Vec<usize>,
    path: Vec<u32>,
    retained_path: Vec<u32>,
    last_path_is_walking: bool,
    route_revision: Option<(u32, usize, u64)>,
    epoch: u64,
    revision: Option<(u32, usize, u64)>,
    network_quotes: Vec<Option<NetworkQuote>>,
    quote_revision: Option<NetworkQuoteRevision>,
    deferred_shared: bool,
    dirty_quote_slots: Vec<usize>,
    dirty_quote_flags: Vec<bool>,
    batch_revision: [Option<u64>; 2],
    batch_sources: [[BatchSource; 2]; 2],
}

// Compact copies of the existing directed connector targets, scoped to this worker's
// topology revision. Original connection order and duplicates are preserved. Preparation
// costs O(lanes + connections), as the existing scratch reserve does; queries allocate nothing.
#[derive(Default)]
struct ConnectorTargets {
    offsets: Vec<usize>,
    targets: Vec<ConnectorTarget>,
    outgoing_counts: Vec<usize>,
    seen: Vec<usize>,
    complete: bool,
}

struct ConnectorTarget {
    lane: usize,
    edge: usize,
    node: u32,
}

impl ConnectorTargets {
    fn prepare(&mut self, network: &TransitNetwork, graph: &RegionGraph, links: usize) {
        let lanes = &network.lane_system.lanes;
        self.offsets.clear();
        self.offsets.reserve(lanes.len() + 1);
        self.targets.clear();
        self.targets.reserve(links);
        // Offsets form an ordered prefix over the connector lists; only preparation visits
        // all lanes. Forecasts subsequently read one contiguous local list per path step.
        for lane in lanes {
            self.offsets.push(self.targets.len());
            for &connector in &lane.next_lanes {
                let Some(connector) = lanes.get(connector) else {
                    continue;
                };
                if connector.edge_id != usize::MAX || connector.lane_type != LaneType::Foot {
                    continue;
                }
                let Some(&target) = connector.next_lanes.first() else {
                    continue;
                };
                let Some(value) = lanes.get(target) else {
                    continue;
                };
                if value.lane_type != LaneType::Foot {
                    continue;
                }
                let Some(node) = lane_origin_node(target, network, graph) else {
                    continue;
                };
                self.targets.push(ConnectorTarget {
                    lane: target,
                    edge: value.edge_id,
                    node,
                });
            }
        }
        self.offsets.push(self.targets.len());
        self.complete = self.prove_complete(network, graph);
    }

    fn prove_complete(&mut self, network: &TransitNetwork, graph: &RegionGraph) -> bool {
        let lanes = &network.lane_system.lanes;
        self.outgoing_counts.resize(graph.node_count(), 0);
        self.outgoing_counts.fill(0);
        self.seen.resize(lanes.len(), usize::MAX);
        self.seen.fill(usize::MAX);
        for (id, lane) in lanes.iter().enumerate() {
            if lane.lane_type == LaneType::Foot
                && let Some(node) = lane_origin_node(id, network, graph)
            {
                self.outgoing_counts[node as usize] += 1;
            }
        }
        // Every possible node-pair representative, including parallel edges, must have
        // foot lanes in both directions. This deliberately also checks car-only edges:
        // get_edge_between_nodes can select them while checking a different parallel edge.
        for (edge, value) in graph
            .edges()
            .iter()
            .enumerate()
            .filter(|(_, edge)| !edge.deleted)
        {
            for node in [value.start_node, value.end_node] {
                if outgoing_lanes(node, edge, network, graph).next().is_none() {
                    return false;
                }
            }
        }
        // Count distinct targets at the correct junction. Equality proves that every
        // incoming physical foot lane reaches EVERY outgoing physical foot lane there.
        // Duplicate connections cannot conceal a missing turn. O(lanes + connections).
        for (id, lane) in lanes.iter().enumerate() {
            if lane.lane_type != LaneType::Foot {
                continue;
            }
            let Some(node) = lane_terminal_node(id, network, graph) else {
                continue;
            };
            let mut count = 0;
            for target in &self.targets[self.offsets[id]..self.offsets[id + 1]] {
                if target.node == node && self.seen[target.lane] != id {
                    self.seen[target.lane] = id;
                    count += 1;
                }
            }
            if count != self.outgoing_counts[node as usize] {
                return false;
            }
        }
        true
    }

    fn proves_path(
        &self,
        origin: &TripEnd,
        destination: &TripEnd,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        // By induction along ANY selected road-node path, complete junctions preserve a
        // nonempty lane frontier, and the final junction reaches this exact arrival lane.
        // Only cost forecasts use this proof; stored routes still materialize their witness.
        self.complete
            && [origin.lane_id, destination.lane_id]
                .into_iter()
                .all(|lane| {
                    network
                        .lane_system
                        .lanes
                        .get(lane)
                        .is_some_and(|lane| lane.lane_type == LaneType::Foot)
                })
            && lane_terminal_node(origin.lane_id, network, graph) == Some(origin.node)
            && lane_origin_node(destination.lane_id, network, graph) == Some(destination.node)
    }

    fn collect(&self, lane: usize, node: u32, edge: usize, out: &mut Vec<usize>) {
        // Forecast origins and every retained target have already validated their lane ID.
        out.extend(
            self.targets[self.offsets[lane]..self.offsets[lane + 1]]
                .iter()
                .filter(|target| target.edge == edge && target.node == node)
                .map(|target| target.lane),
        );
    }

    fn follow(
        &self,
        path: &[u32],
        graph: &RegionGraph,
        possible: &mut Vec<usize>,
        next: &mut Vec<usize>,
    ) -> bool {
        follow_connector_path_with(path, graph, possible, next, |lane, node, edge, out| {
            self.collect(lane, node, edge, out)
        })
    }
}

#[derive(Default)]
struct BatchSource {
    key: Option<(u64, u32, usize)>,
    source: CchBucketSource,
    frontier_key: Option<(u64, u32, usize, u64)>,
    frontier_epoch: u64,
    frontiers: Vec<WalkingFrontier>,
}

#[derive(Clone, Copy, Default)]
struct WalkingFrontier {
    epoch: u64,
    lanes: [usize; 2],
    count: usize,
}

impl BatchSource {
    fn prepare_frontiers(&mut self, targets: &CchBuckets, origin: &TripEnd, lane_revision: u64) {
        let key = (targets.id, origin.node, origin.lane_id, lane_revision);
        if self.frontier_key == Some(key) {
            return;
        }
        self.frontier_key = Some(key);
        self.frontier_epoch = self.frontier_epoch.wrapping_add(1);
        if self.frontier_epoch == 0 {
            self.frontiers.fill(WalkingFrontier::default());
            self.frontier_epoch = 1;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct NetworkQuoteKey {
    mode: u8,
    // Exact lane constraints for pedestrian fallback; otherwise the canonical CCH
    // incoming and exit edge contexts. Door-local terms stay outside this key.
    from_context: usize,
    to_context: usize,
    from_node: u32,
    to_node: u32,
}

#[derive(Clone, Copy)]
struct NetworkQuote {
    key: NetworkQuoteKey,
    seconds: Option<f32>,
}

type NetworkQuoteRevision = (u32, u64, usize, u64);

fn network_quote_revision(network: &TransitNetwork) -> NetworkQuoteRevision {
    (
        network.cch_graph.build_generation,
        network.cch_graph.metric_revision,
        network.lane_system.lanes.len(),
        network.lane_system.revision(),
    )
}

#[derive(Default)]
struct SharedQuoteSlots {
    // Exact key checks make collisions replacement decisions, never approximate answers.
    buckets: Vec<[Option<NetworkQuote>; 2]>,
    len: usize,
}

// Preserve the original global bucket hash and replacement policy, partitioning only
// synchronization. A query locks one stripe; independent worker keys need no global lock.
const QUOTE_STRIPES: usize = 32;

/// Network-leg quotes shared by every worker of a parallel search, across passes.
///
/// A leg's seconds are a pure function of its key under one CCH/lane revision, so whichever
/// worker prices a leg first cannot change a result; the worker-local slots miss whenever Rayon
/// places a repeated pair on another thread. Lookups take a read lock and only misses write.
/// Fixed buckets bound storage by lane count; [`Self::sync`] empties stale revisions.
#[derive(Default)]
pub(crate) struct SharedNetworkQuotes {
    revision: Option<NetworkQuoteRevision>,
    targets: [Option<CchBuckets>; 2],
    quotes: [std::sync::RwLock<SharedQuoteSlots>; QUOTE_STRIPES],
    bucket_mask: usize,
}

impl SharedNetworkQuotes {
    /// Empties retained legs if the network revision changed since they were priced.
    pub(crate) fn sync(&mut self, network: &TransitNetwork) {
        let revision = network_quote_revision(network);
        if self.revision != Some(revision) {
            self.clear();
            // Fewer than four slots per lane, independent of the number of homes/jobs.
            // Allocation occurs only during network preparation, before parallel queries.
            let buckets = network.lane_system.lanes.len().max(1).next_power_of_two();
            self.bucket_mask = buckets - 1;
            for (stripe, quotes) in self.quotes.iter_mut().enumerate() {
                let count = (buckets + QUOTE_STRIPES - 1 - stripe) / QUOTE_STRIPES;
                let quotes = quotes.get_mut().unwrap();
                if quotes.buckets.len() != count {
                    quotes.buckets = vec![[None; 2]; count];
                }
            }
            self.revision = Some(revision);
        }
    }

    /// Invalidates retained legs and targets, preserving fixed quote storage for the next metric.
    pub(crate) fn clear(&mut self) {
        for quotes in &mut self.quotes {
            let quotes = quotes.get_mut().unwrap();
            quotes.buckets.fill([None; 2]);
            quotes.len = 0;
        }
        self.targets = Default::default();
    }

    /// Prepares open workplace attachment nodes, each with the frontage edge its trips leave
    /// on, once per distinct canonical target set and metric.
    /// Existing exact leg quotes remain valid when only workplace availability changes.
    /// Returns the number of backward target searches for routing-work instrumentation.
    pub(crate) fn prepare_targets(
        &mut self,
        mut nodes: [Vec<(u32, usize)>; 2],
        network: &TransitNetwork,
    ) -> usize {
        self.sync(network);
        let mut searches = 0;
        for (mode, nodes) in nodes.iter_mut().enumerate() {
            CchBuckets::canonical_targets(
                &network.cch_graph,
                nodes,
                transit_flags_for_mode(mode as u8),
            );
            if self.targets[mode].as_ref().is_some_and(|targets| {
                targets.is_current(&network.cch_graph) && targets.targets() == nodes.as_slice()
            }) {
                continue;
            }
            searches += nodes.len();
            self.targets[mode] = Some(CchBuckets::build(
                &network.cch_graph,
                nodes,
                transit_flags_for_mode(mode as u8),
            ));
        }
        searches
    }

    /// Whether the leg storage is sized for the network's current revision.
    #[cfg(test)]
    pub(crate) fn synced_for_test(&self, network: &TransitNetwork) -> bool {
        self.revision == Some(network_quote_revision(network))
    }

    /// Retained leg count.
    pub(crate) fn len(&self) -> usize {
        self.quotes
            .iter()
            .map(|quotes| quotes.read().unwrap().len)
            .sum()
    }

    fn bucket(&self, key: &NetworkQuoteKey) -> (usize, usize) {
        let bucket =
            foldhash::fast::FixedState::default().hash_one(key) as usize & self.bucket_mask;
        (bucket % QUOTE_STRIPES, bucket / QUOTE_STRIPES)
    }

    // `None` means not retained; a quote from another revision is never answered.
    fn get(&self, key: &NetworkQuoteKey, revision: NetworkQuoteRevision) -> Option<Option<f32>> {
        if self.revision != Some(revision) {
            return None;
        }
        let (stripe, bucket) = self.bucket(key);
        let quotes = self.quotes[stripe].read().unwrap();
        quotes.buckets[bucket]
            .iter()
            .flatten()
            .find(|quote| quote.key == *key)
            .map(|quote| quote.seconds)
    }

    fn insert(&self, key: NetworkQuoteKey, seconds: Option<f32>, revision: NetworkQuoteRevision) {
        if self.revision == Some(revision) {
            let (stripe, bucket) = self.bucket(&key);
            let mut quotes = self.quotes[stripe].write().unwrap();
            let entries = &mut quotes.buckets[bucket];
            if entries.iter().flatten().any(|quote| quote.key == key) {
                return;
            }
            let added = entries[1].is_none();
            entries[1] = entries[0];
            entries[0] = Some(NetworkQuote { key, seconds });
            quotes.len += usize::from(added);
        }
    }
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
        QUERY.with_borrow_mut(|scratch| {
            scratch.disable_deferred_quotes();
            plan(scratch)
        })
    }

    pub(in super::super) fn with<T>(
        network: &TransitNetwork,
        graph: &RegionGraph,
        quote: impl FnOnce(&mut Self) -> T,
    ) -> T {
        QUERY.with_borrow_mut(|scratch| {
            scratch.disable_deferred_quotes();
            scratch.prepare(network, graph);
            quote(scratch)
        })
    }

    /// Runs a planning callback with `owned` installed as the current thread-local scratch.
    ///
    /// The previous TLS value is restored when the callback returns or unwinds, so nested
    /// planning helpers retain their existing TLS access without borrowing the TLS cell across
    /// the callback. The caller regains its updated scratch when the scope ends.
    pub(crate) fn with_owned<T>(owned: &mut Self, work: impl FnOnce() -> T) -> T {
        struct Restore<'a> {
            owned: &'a mut TripQueryScratch,
        }

        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                QUERY.with_borrow_mut(|scratch| std::mem::swap(scratch, self.owned));
            }
        }

        QUERY.with_borrow_mut(|scratch| std::mem::swap(scratch, owned));
        let _restore = Restore { owned };
        work()
    }

    /// Prepares this worker's query storage for a deferred shared-quote phase.
    ///
    /// Cache misses remain local until [`Self::publish_quotes`] is called. The dirty-slot
    /// vectors and shared batch frontiers are prepared here, outside the query loop, and only
    /// unique changed slots are revisited during publication. A network revision change
    /// discards pending old entries.
    pub(crate) fn prepare_deferred_quotes(
        &mut self,
        network: &TransitNetwork,
        graph: &RegionGraph,
        shared: &SharedNetworkQuotes,
    ) {
        self.discard_deferred_quotes();
        self.deferred_shared = true;
        self.prepare(network, graph);
        if self.dirty_quote_flags.len() != self.network_quotes.len() {
            self.dirty_quote_flags
                .resize(self.network_quotes.len(), false);
        }
        self.dirty_quote_slots.reserve(self.network_quotes.len());
        self.prepare_batch(shared, network);
    }

    /// Publishes this worker's last retained quote for each changed slot in deterministic order.
    ///
    /// The caller publishes shards serially. A revision mismatch drops all pending entries so
    /// stale route costs cannot enter the shared cache.
    pub(crate) fn publish_quotes(&mut self, shared: &mut SharedNetworkQuotes) {
        if let Some(revision) = self.quote_revision
            && shared.revision == Some(revision)
        {
            for &slot in &self.dirty_quote_slots {
                if let Some(Some(quote)) = self.network_quotes.get(slot) {
                    shared.insert(quote.key, quote.seconds, revision);
                }
            }
        }
        self.discard_deferred_quotes();
    }

    fn disable_deferred_quotes(&mut self) {
        self.discard_deferred_quotes();
        self.deferred_shared = false;
    }

    fn discard_deferred_quotes(&mut self) {
        for &slot in &self.dirty_quote_slots {
            if let Some(flag) = self.dirty_quote_flags.get_mut(slot) {
                *flag = false;
            }
        }
        self.dirty_quote_slots.clear();
    }

    fn mark_dirty_quote(&mut self, slot: usize) {
        debug_assert!(slot < self.dirty_quote_flags.len());
        if !self.dirty_quote_flags[slot] {
            self.dirty_quote_flags[slot] = true;
            self.dirty_quote_slots.push(slot);
        }
    }

    /// Reserves lane/connector scratch and CCH ancestor labels at a topology boundary.
    pub(crate) fn prepare(&mut self, network: &TransitNetwork, graph: &RegionGraph) {
        self.cch.prepare(&network.cch_graph, graph);
        let lanes = &network.lane_system.lanes;
        let quote_revision = network_quote_revision(network);
        if self.quote_revision != Some(quote_revision) {
            // Direct slots bound retained pairs by road storage, never homes × workplaces.
            // Collisions only replace retained results: every miss uses the exact planner.
            let slots = (2 * graph.edge_count() + graph.node_count())
                .max(1)
                .next_power_of_two();
            self.network_quotes.resize(slots, None);
            self.network_quotes.fill(None);
            self.quote_revision = Some(quote_revision);
        }
        let revision = (
            network.cch_graph.build_generation,
            lanes.len(),
            network.lane_system.revision(),
        );
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
            + lanes.len()
            + 1;
        self.connectors.prepare(network, graph, links);
        self.heap.clear();
        self.heap.reserve(links);
        for values in [&mut self.possible, &mut self.next] {
            values.clear();
            values.reserve(links);
        }
        self.revision = Some(revision);
    }

    /// Whether this scratch is prepared for the network's current revision, and with its route
    /// buffers when `routes`.
    #[cfg(test)]
    pub(crate) fn prepared_for_test(&self, network: &TransitNetwork, routes: bool) -> bool {
        let revision = (
            network.cch_graph.build_generation,
            network.lane_system.lanes.len(),
            network.lane_system.revision(),
        );
        self.revision == Some(revision) && (!routes || self.route_revision == self.revision)
    }

    /// Prepares predecessor and retained-path buffers only for full route work. Cost-only
    /// forecasts keep their existing storage; warm route queries merely check the revision.
    pub(crate) fn prepare_routes(&mut self, network: &TransitNetwork, graph: &RegionGraph) {
        self.prepare(network, graph);
        if self.route_revision == self.revision {
            return;
        }
        let lanes = network.lane_system.lanes.len();
        self.previous.resize(lanes, usize::MAX);
        self.path.clear();
        self.path.reserve((lanes + 1).max(graph.node_count() + 2));
        self.retained_path.clear();
        self.retained_path
            .reserve((lanes + 1).max(2 * graph.edge_count() + 2 * graph.node_count() + 2));
        self.route_revision = self.revision;
    }

    #[allow(clippy::too_many_arguments)]
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
        shared: Option<&SharedNetworkQuotes>,
    ) -> Option<PlannedTripCandidate> {
        // The caller has already priced shared frontages/nodes without a search, except a
        // car loop back to a node it may not turn at.
        debug_assert!(
            origin.node != destination.node
                || !super::joins_at_shared_node(mode, origin, destination, network)
        );
        let key = self.quote_key(mode, origin, destination, network, graph);
        let slot = foldhash::fast::FixedState::default().hash_one(key) as usize
            & (self.network_quotes.len() - 1);
        let seconds =
            if let Some(quote) = self.network_quotes[slot].filter(|quote| quote.key == key) {
                quote.seconds
            } else {
                let revision = network_quote_revision(network);
                let seconds = match shared.and_then(|shared| shared.get(&key, revision)) {
                    Some(seconds) => seconds,
                    None => {
                        let seconds = self.network_seconds(
                            mode,
                            origin_rank,
                            origin,
                            destination,
                            network,
                            graph,
                            pathfind_count,
                            shared,
                        );
                        if let Some(shared) = shared {
                            if self.deferred_shared {
                                self.mark_dirty_quote(slot);
                            } else {
                                shared.insert(key, seconds, revision);
                            }
                        }
                        seconds
                    }
                };
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

    fn quote_key(
        &self,
        mode: u8,
        origin: &TripEnd,
        destination: &TripEnd,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> NetworkQuoteKey {
        let unconstrained = mode == MODE_CAR
            || self
                .connectors
                .proves_path(origin, destination, network, graph);
        let (from_context, to_context) = if unconstrained {
            let flags = transit_flags_for_mode(mode);
            let cch = &network.cch_graph;
            (
                cch.source_context(origin.node, lane_edge(origin.lane_id, network), flags),
                cch.target_context(
                    destination.node,
                    lane_edge(destination.lane_id, network),
                    flags,
                ),
            )
        } else {
            (origin.lane_id, destination.lane_id)
        };
        NetworkQuoteKey {
            mode,
            from_context,
            to_context,
            from_node: origin.node,
            to_node: destination.node,
        }
    }

    /// Prices a CCH leg in prepared storage without retaining an owned route. `from_edge` is
    /// the road arriving at `from` and `to_edge` the road left on after `to` (`usize::MAX`
    /// for an unconstrained end).
    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn cch_cost(
        &mut self,
        from: u32,
        to: u32,
        from_edge: usize,
        to_edge: usize,
        mode: u8,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        self.prepare_routes(network, graph);
        self.last_path_is_walking = false;
        network
            .cch_graph
            .find_path_in(
                from,
                to,
                from_edge,
                to_edge,
                graph,
                transit_flags_for_mode(mode),
                &mut self.cch,
            )
            .map(|(seconds, _, _)| seconds)
    }

    /// Visits the exact selected CCH/connector path while temporary storage stays borrowed.
    /// Candidate pricing discards the slice; only a stored agent plan copies its final route.
    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn network_path<T>(
        &mut self,
        mode: u8,
        from: u32,
        to: u32,
        start: RouteStart,
        detach: usize,
        network: &TransitNetwork,
        graph: &RegionGraph,
        consume: impl FnOnce(&[u32]) -> T,
    ) -> Result<(f32, T), &'static str> {
        self.prepare_routes(network, graph);
        self.last_path_is_walking = false;
        let edge = match start {
            RouteStart::Lane(lane) => lane_edge(lane, network),
            RouteStart::Edge(edge) => edge,
            RouteStart::Node => usize::MAX,
        };
        let (seconds, _, path) = network
            .cch_graph
            .find_path_in(
                from,
                to,
                edge,
                lane_edge(detach, network),
                graph,
                transit_flags_for_mode(mode),
                &mut self.cch,
            )
            .ok_or("cch_no_path")?;
        if path.len() < 2 {
            return Err("path_too_short");
        }
        if mode == MODE_CAR
            || path_has_connectors_from(
                path,
                from,
                start,
                detach,
                network,
                graph,
                &mut self.possible,
                &mut self.next,
            )
        {
            return Ok((seconds, consume(path)));
        }
        self.last_path_is_walking = true;
        let (seconds, path) = self
            .walking_path(from, to, start, detach, network, graph)
            .ok_or("pedestrian_connector_fallback_missing")?;
        Ok((seconds, consume(path)))
    }

    /// Retains the last successful path when its candidate wins, without allocating or
    /// repeating its search. A direct zero-hop candidate withdraws the preceding path.
    pub(in super::super) fn retain_network_path(&mut self, zero_hop: bool) {
        self.retained_path.clear();
        if !zero_hop {
            let path = if self.last_path_is_walking {
                &self.path
            } else {
                self.cch.nodes()
            };
            assert!(
                path.len() <= self.retained_path.capacity(),
                "route storage must be prepared"
            );
            self.retained_path.extend_from_slice(path);
        }
    }

    /// Allocates only the selected persistent route product after candidate comparison ends.
    pub(in super::super) fn finish_path(&self) -> Vec<u32> {
        self.retained_path.clone()
    }

    /// Checks an existing route using the prepared lane frontier, including free node origins.
    pub(in super::super) fn has_connectors(
        &mut self,
        path: &[u32],
        from: u32,
        start: RouteStart,
        detach: usize,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        if path.len() < 2 {
            return match start {
                RouteStart::Lane(lane) => connector_to_lane_exists(lane, detach, network),
                RouteStart::Node => {
                    path.first().copied() == Some(from)
                        && lane_origin_node(detach, network, graph) == Some(from)
                }
                RouteStart::Edge(edge) => incoming_lanes(from, edge, network, graph)
                    .any(|lane| lane == detach || connector_to_lane_exists(lane, detach, network)),
            };
        }
        self.prepare(network, graph);
        path_has_connectors_from(
            path,
            from,
            start,
            detach,
            network,
            graph,
            &mut self.possible,
            &mut self.next,
        )
    }

    /// Visits a flow-field route only after validating its destination, turns and connectors.
    pub(in super::super) fn flow_path<T>(
        &mut self,
        field: &crate::simulation::pathing::flow_field::FlowField,
        candidate: &PlannedTripCandidate,
        network: &TransitNetwork,
        graph: &RegionGraph,
        consume: impl FnOnce(&[u32]) -> T,
    ) -> Option<T> {
        self.prepare_routes(network, graph);
        field.build_path_in(
            candidate.planned_attach_node,
            graph.node_count() + 1,
            &mut self.path,
        )?;
        let path = &self.path;
        if path.last().copied() != Some(candidate.planned_detach_node) {
            return None;
        }
        let valid = if candidate.mode == MODE_CAR {
            crate::simulation::pathing::cch::CchGraph::path_has_valid_vehicle_turns(
                path,
                lane_edge(candidate.planned_attach_lane_id, network),
                lane_edge(candidate.planned_detach_lane_id, network),
                graph,
            )
        } else {
            path_has_connectors(
                path,
                candidate.planned_attach_lane_id,
                candidate.planned_detach_lane_id,
                network,
                graph,
                &mut self.possible,
                &mut self.next,
            )
        };
        valid.then(|| consume(path))
    }

    /// Reconstructs an exact connector fallback in reusable dense lane labels and path storage.
    /// Seed/relaxation order and strict cost comparison match the original heap search.
    pub(in super::super) fn walking_path(
        &mut self,
        from: u32,
        to: u32,
        start: RouteStart,
        detach: usize,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<(f32, &[u32])> {
        self.prepare_routes(network, graph);
        self.path.clear();
        if from == to {
            let reachable = match start {
                RouteStart::Node => lane_origin_node(detach, network, graph) == Some(from),
                _ => {
                    seed_incoming_lanes(&mut self.possible, from, start, network, graph);
                    self.possible
                        .iter()
                        .any(|&lane| connector_to_lane_exists(lane, detach, network))
                }
            };
            if !reachable {
                return None;
            }
            self.path.push(from);
            return Some((0.0, &self.path));
        }
        self.heap.clear();
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.best.fill((0, f32::INFINITY));
            self.epoch = 1;
        }
        if matches!(start, RouteStart::Node) {
            if from as usize >= graph.node_adjacency_count() {
                return None;
            }
            for &edge in graph.node_adjacency(from) {
                for lane in outgoing_lanes(from, edge, network, graph) {
                    let Some(node) = lane_terminal_node(lane, network, graph) else {
                        continue;
                    };
                    let value = &network.lane_system.lanes[lane];
                    let cost = graph
                        .edges()
                        .get(value.edge_id)
                        .map_or(
                            value.length / WALK_CONNECTOR_COST_SPEED_MS,
                            CostCalculator::walk_cost_s,
                        )
                        .max(0.001);
                    let best = &mut self.best[lane];
                    if cost
                        < if best.0 == self.epoch {
                            best.1
                        } else {
                            f32::INFINITY
                        }
                    {
                        *best = (self.epoch, cost);
                        self.previous[lane] = usize::MAX;
                        self.heap.push(PedestrianSearchState {
                            cost,
                            node,
                            incoming_lane: lane,
                        });
                    }
                }
            }
        } else {
            seed_incoming_lanes(&mut self.possible, from, start, network, graph);
            for &lane in &self.possible {
                let best = self.best.get_mut(lane)?;
                *best = (self.epoch, 0.0);
                self.previous[lane] = usize::MAX;
                self.heap.push(PedestrianSearchState {
                    cost: 0.0,
                    node: from,
                    incoming_lane: lane,
                });
            }
        }
        while let Some(state) = self.heap.pop() {
            if state.cost > self.best[state.incoming_lane].1 {
                continue;
            }
            if state.node == to && connector_to_lane_exists(state.incoming_lane, detach, network) {
                self.path.push(state.node);
                let mut lane = state.incoming_lane;
                while self.previous[lane] != usize::MAX {
                    lane = self.previous[lane];
                    self.path.push(lane_terminal_node(lane, network, graph)?);
                }
                if matches!(start, RouteStart::Node) {
                    self.path.push(from);
                }
                self.path.reverse();
                return Some((state.cost, &self.path));
            }
            visit_pedestrian_successors(&state, network, graph, |next| {
                let best = &mut self.best[next.incoming_lane];
                if next.cost
                    < if best.0 == self.epoch {
                        best.1
                    } else {
                        f32::INFINITY
                    }
                {
                    *best = (self.epoch, next.cost);
                    self.previous[next.incoming_lane] = state.incoming_lane;
                    self.heap.push(next);
                }
            });
        }
        None
    }

    fn prepare_batch(&mut self, shared: &SharedNetworkQuotes, network: &TransitNetwork) {
        for mode in 0..2 {
            let id = shared.targets[mode].as_ref().map(|targets| targets.id);
            if self.batch_revision[mode] == id {
                continue;
            }
            for source in &mut self.batch_sources[mode] {
                source.key = None;
                source.frontier_key = None;
                if let Some(targets) = &shared.targets[mode] {
                    source.source.prepare(targets, &network.cch_graph);
                    if mode != usize::from(MODE_CAR) {
                        source
                            .frontiers
                            .resize(targets.targets().len(), WalkingFrontier::default());
                    }
                }
            }
            self.batch_revision[mode] = id;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn network_seconds(
        &mut self,
        mode: u8,
        origin_rank: u8,
        origin: &TripEnd,
        destination: &TripEnd,
        network: &TransitNetwork,
        graph: &RegionGraph,
        pathfind_count: &AtomicU32,
        shared: Option<&SharedNetworkQuotes>,
    ) -> Option<f32> {
        let edge = network.lane_system.lanes.get(origin.lane_id)?.edge_id;
        let exit = lane_edge(destination.lane_id, network);
        let flags = transit_flags_for_mode(mode);
        let batch = shared.and_then(|shared| {
            let targets = shared.targets[usize::from(mode)].as_ref()?;
            if !targets.is_current(&network.cch_graph) {
                return None;
            }
            let target = targets.target_index(&network.cch_graph, destination.node, exit)?;
            Some((shared, targets, target))
        });
        if let Some((shared, targets, target)) = batch {
            // Each worker owns two origins per mode, matching the existing entrance ranks.
            // Prepare on a target revision, then reuse labels/results for every workplace.
            self.prepare_batch(shared, network);
            let source = &mut self.batch_sources[usize::from(mode)][usize::from(origin_rank)];
            let key = (
                targets.id,
                origin.node,
                network.cch_graph.source_context(origin.node, edge, flags),
            );
            if source.key != Some(key) {
                targets.query_source(&network.cch_graph, origin.node, edge, &mut source.source);
                source.key = Some(key);
                pathfind_count.fetch_add(1, Ordering::Relaxed);
            }
            if mode == MODE_CAR
                || self
                    .connectors
                    .proves_path(origin, destination, network, graph)
            {
                return targets.cost(target, &source.source);
            }
            source.prepare_frontiers(targets, origin, network.lane_system.revision());
            let cached = source.frontiers[target];
            let (seconds, connected) = if cached.epoch == source.frontier_epoch {
                (
                    targets.cost(target, &source.source)?,
                    frontier_reaches(&cached.lanes[..cached.count], destination.lane_id, network),
                )
            } else {
                let (seconds, path) = targets.path_in(
                    target,
                    &network.cch_graph,
                    graph,
                    &source.source,
                    &mut self.cch,
                )?;
                // combine() only asks for distinct nodes. The selected CCH path is independent
                // of the destination lane; only its final connector depends on that lane.
                debug_assert!(path.len() >= 2);
                self.possible.clear();
                self.possible.push(origin.lane_id);
                if !self
                    .connectors
                    .follow(path, graph, &mut self.possible, &mut self.next)
                {
                    self.possible.clear();
                }
                let connected = frontier_reaches(&self.possible, destination.lane_id, network);
                // Ordinary sidewalks arrive on at most two lanes. Larger frontiers still use
                // the complete result, but are recomputed rather than truncated or allocated.
                if self.possible.len() <= cached.lanes.len() {
                    let mut entry = WalkingFrontier {
                        epoch: source.frontier_epoch,
                        count: self.possible.len(),
                        ..Default::default()
                    };
                    entry.lanes[..entry.count].copy_from_slice(&self.possible);
                    source.frontiers[target] = entry;
                }
                (seconds, connected)
            };
            return if connected {
                Some(seconds)
            } else {
                self.walking_cost(origin, destination, network, graph)
            };
        }
        pathfind_count.fetch_add(1, Ordering::Relaxed);
        if mode == MODE_CAR
            || self
                .connectors
                .proves_path(origin, destination, network, graph)
        {
            return network.cch_graph.find_cost(
                origin.node,
                destination.node,
                edge,
                exit,
                graph,
                flags,
                self.cch.query(),
            );
        }
        let (mut seconds, _, path) = network.cch_graph.find_path_in(
            origin.node,
            destination.node,
            edge,
            exit,
            graph,
            flags,
            &mut self.cch,
        )?;
        debug_assert!(
            path.len() >= 2,
            "network_seconds only prices distinct nodes"
        );
        self.possible.clear();
        self.possible.push(origin.lane_id);
        if !(self
            .connectors
            .follow(path, graph, &mut self.possible, &mut self.next)
            && frontier_reaches(&self.possible, destination.lane_id, network))
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
                if next.cost
                    < if entry.0 == self.epoch {
                        entry.1
                    } else {
                        f32::INFINITY
                    }
                {
                    *entry = (self.epoch, next.cost);
                    self.heap.push(next);
                }
            });
        }
        None
    }
}

// Road edge of a lane, or `usize::MAX` for a missing lane or a junction connector.
fn lane_edge(lane: usize, network: &TransitNetwork) -> usize {
    network
        .lane_system
        .lanes
        .get(lane)
        .map_or(usize::MAX, |lane| lane.edge_id)
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
    path_has_connectors_from(
        path,
        path.first().copied().unwrap_or(u32::MAX),
        RouteStart::Lane(attach),
        detach,
        network,
        graph,
        possible,
        next,
    )
}

fn outgoing_lanes<'a>(
    node: u32,
    edge: usize,
    network: &'a TransitNetwork,
    graph: &'a RegionGraph,
) -> impl Iterator<Item = usize> + 'a {
    network
        .lane_system
        .edge_lanes
        .get(&edge)
        .into_iter()
        .flatten()
        .copied()
        .filter(move |&lane| {
            network
                .lane_system
                .lanes
                .get(lane)
                .is_some_and(|value| value.lane_type == LaneType::Foot)
                && lane_origin_node(lane, network, graph) == Some(node)
        })
}

fn seed_incoming_lanes(
    out: &mut Vec<usize>,
    node: u32,
    start: RouteStart,
    network: &TransitNetwork,
    graph: &RegionGraph,
) {
    out.clear();
    match start {
        RouteStart::Lane(lane) => out.push(lane),
        RouteStart::Edge(edge) => out.extend(incoming_lanes(node, edge, network, graph)),
        _ => {}
    }
}

fn incoming_lanes<'a>(
    node: u32,
    edge: usize,
    network: &'a TransitNetwork,
    graph: &'a RegionGraph,
) -> impl Iterator<Item = usize> + 'a {
    network
        .lane_system
        .edge_lanes
        .get(&edge)
        .filter(|_| edge != usize::MAX)
        .into_iter()
        .flatten()
        .copied()
        .filter(move |&lane| {
            network
                .lane_system
                .lanes
                .get(lane)
                .is_some_and(|value| value.lane_type == LaneType::Foot)
                && lane_terminal_node(lane, network, graph) == Some(node)
        })
}

#[allow(clippy::too_many_arguments)]
fn path_has_connectors_from(
    mut path: &[u32],
    from: u32,
    start: RouteStart,
    detach: usize,
    network: &TransitNetwork,
    graph: &RegionGraph,
    possible: &mut Vec<usize>,
    next: &mut Vec<usize>,
) -> bool {
    possible.clear();
    match start {
        RouteStart::Node => {
            if path.first().copied() != Some(from) {
                return false;
            }
            if path.len() < 2 {
                return lane_origin_node(detach, network, graph) == Some(from);
            }
            let Some(edge) = graph.get_edge_between_nodes(from, path[1]) else {
                return false;
            };
            possible.extend(outgoing_lanes(from, edge, network, graph));
            path = &path[1..];
        }
        RouteStart::Lane(lane) if path.len() < 2 => {
            return connector_to_lane_exists(lane, detach, network);
        }
        _ => seed_incoming_lanes(possible, from, start, network, graph),
    }
    follow_connector_path(path, network, graph, possible, next)
        && frontier_reaches(possible, detach, network)
}

fn frontier_reaches(possible: &[usize], detach: usize, network: &TransitNetwork) -> bool {
    possible
        .iter()
        .any(|&lane| lane == detach || connector_to_lane_exists(lane, detach, network))
}

fn follow_connector_path(
    path: &[u32],
    network: &TransitNetwork,
    graph: &RegionGraph,
    possible: &mut Vec<usize>,
    next: &mut Vec<usize>,
) -> bool {
    // Stored paths also support immediate raw connectivity checks after an isolated edit.
    // Only revision-scoped cost forecasts use the compact prepared lists above.
    follow_connector_path_with(path, graph, possible, next, |lane, node, edge, out| {
        collect_connector_targets_to_edge(lane, node, edge, network, graph, out)
    })
}

fn follow_connector_path_with(
    path: &[u32],
    graph: &RegionGraph,
    possible: &mut Vec<usize>,
    next: &mut Vec<usize>,
    mut collect: impl FnMut(usize, u32, usize, &mut Vec<usize>),
) -> bool {
    for pair in path.windows(2) {
        let Some(edge) = graph.get_edge_between_nodes(pair[0], pair[1]) else {
            return false;
        };
        next.clear();
        for &lane in possible.iter() {
            collect(lane, pair[0], edge, next);
        }
        if next.is_empty() {
            return false;
        }
        next.sort_unstable();
        next.dedup();
        std::mem::swap(possible, next);
    }
    true
}

#[cfg(test)]
mod tests {
    use std::sync::RwLock;

    struct SingleLockQuotes(RwLock<super::SharedQuoteSlots>);
    impl SingleLockQuotes {
        fn new(buckets: usize) -> Self {
            Self(RwLock::new(super::SharedQuoteSlots {
                buckets: vec![[None; 2]; buckets],
                len: 0,
            }))
        }
        fn get(&self, key: &super::NetworkQuoteKey) -> Option<Option<f32>> {
            use std::hash::BuildHasher;
            let quotes = self.0.read().unwrap();
            let bucket = foldhash::fast::FixedState::default().hash_one(key) as usize
                & (quotes.buckets.len() - 1);
            quotes.buckets[bucket]
                .iter()
                .flatten()
                .find(|quote| quote.key == *key)
                .map(|quote| quote.seconds)
        }
        fn insert(&self, key: super::NetworkQuoteKey, seconds: Option<f32>) {
            use std::hash::BuildHasher;
            let mut quotes = self.0.write().unwrap();
            let bucket = foldhash::fast::FixedState::default().hash_one(key) as usize
                & (quotes.buckets.len() - 1);
            let entries = &mut quotes.buckets[bucket];
            if entries.iter().flatten().any(|quote| quote.key == key) {
                return;
            }
            let added = entries[1].is_none();
            entries[1] = entries[0];
            entries[0] = Some(super::NetworkQuote { key, seconds });
            quotes.len += usize::from(added);
        }
    }

    fn cache_key(index: usize) -> super::NetworkQuoteKey {
        super::NetworkQuoteKey {
            mode: (index % 2) as u8,
            from_context: if index.is_multiple_of(3) {
                usize::MAX
            } else {
                index % 23
            },
            to_context: usize::MAX,
            from_node: (index / 32) as u32,
            to_node: (index % 32) as u32,
        }
    }

    fn cache_value(key: &super::NetworkQuoteKey) -> Option<f32> {
        (!key.from_node.is_multiple_of(7))
            .then_some((key.from_node * 32 + key.to_node) as f32 / 7.0)
    }

    #[test]
    fn deferred_quotes_publish_last_slot_and_drop_stale_revision() {
        let mut network = TransitNetwork::new();
        let graph = RegionGraph::new();
        let mut shared = SharedNetworkQuotes::default();
        shared.sync(&network);
        let mut scratch = TripQueryScratch::default();
        scratch.prepare_deferred_quotes(&network, &graph, &shared);

        let key = cache_key(1);
        scratch.network_quotes[0] = Some(NetworkQuote {
            key,
            seconds: Some(1.0),
        });
        scratch.mark_dirty_quote(0);
        scratch.network_quotes[0] = Some(NetworkQuote {
            key,
            seconds: Some(2.0),
        });
        scratch.mark_dirty_quote(0);
        scratch.publish_quotes(&mut shared);
        assert_eq!(
            shared.get(&key, network_quote_revision(&network)),
            Some(Some(2.0))
        );
        assert!(scratch.dirty_quote_slots.is_empty());
        assert!(scratch.deferred_shared);
        assert!(scratch.dirty_quote_slots.capacity() >= scratch.network_quotes.len());

        scratch.prepare_deferred_quotes(&network, &graph, &shared);
        scratch.network_quotes[0] = Some(NetworkQuote {
            key,
            seconds: Some(3.0),
        });
        scratch.mark_dirty_quote(0);
        network.cch_graph.metric_revision = network.cch_graph.metric_revision.wrapping_add(1);
        shared.sync(&network);
        scratch.prepare_deferred_quotes(&network, &graph, &shared);
        scratch.publish_quotes(&mut shared);
        assert_eq!(shared.len(), 0);
        assert!(scratch.dirty_quote_slots.is_empty());
    }

    #[test]
    fn owned_scratch_restores_tls_after_return_and_unwind() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        let tls_marker = Some((5, 6, 7, 8));
        TripQueryScratch::with_routes(|scratch| scratch.quote_revision = tls_marker);
        let mut owned = TripQueryScratch {
            quote_revision: Some((1, 2, 3, 4)),
            ..Default::default()
        };

        TripQueryScratch::with_owned(&mut owned, || {
            TripQueryScratch::with_routes(|scratch| {
                assert_eq!(scratch.quote_revision, Some((1, 2, 3, 4)));
                scratch.quote_revision = Some((9, 10, 11, 12));
            });
        });
        assert_eq!(owned.quote_revision, Some((9, 10, 11, 12)));
        assert_eq!(
            TripQueryScratch::with_routes(|scratch| scratch.quote_revision),
            tls_marker
        );

        let panic = catch_unwind(AssertUnwindSafe(|| {
            TripQueryScratch::with_owned(&mut owned, || panic!("restore scratch"));
        }));
        assert!(panic.is_err());
        assert_eq!(owned.quote_revision, Some((9, 10, 11, 12)));
        assert_eq!(
            TripQueryScratch::with_routes(|scratch| scratch.quote_revision),
            tls_marker
        );
    }

    #[test]
    fn striped_quotes_preserve_global_collisions_capacity_and_revision_guards() {
        use super::*;
        use rayon::prelude::*;
        let mut network = TransitNetwork::new();
        let mut shared = SharedNetworkQuotes::default();
        for lanes in [0, 1, 17, 129, 1] {
            network
                .lane_system
                .lanes
                .resize_with(lanes, Default::default);
            network.cch_graph.metric_revision += 1;
            let revision = network_quote_revision(&network);
            assert!(shared.get(&cache_key(0), revision).is_none());
            shared.sync(&network);
            assert_eq!(shared.len(), 0);
            let count = lanes.max(1).next_power_of_two();
            assert_eq!(
                shared
                    .quotes
                    .iter()
                    .map(|quotes| quotes.read().unwrap().buckets.len())
                    .sum::<usize>(),
                count
            );
            let capacities = shared
                .quotes
                .each_mut()
                .map(|quotes| quotes.get_mut().unwrap().buckets.capacity());
            let ordinary = SingleLockQuotes::new(count);
            for index in 0..4096 {
                let key = cache_key(index);
                assert_eq!(shared.get(&key, revision), ordinary.get(&key));
                shared.insert(key, cache_value(&key), revision);
                ordinary.insert(key, cache_value(&key));
                for previous in [0, index / 2, index.saturating_sub(1), index] {
                    let key = cache_key(previous);
                    assert_eq!(shared.get(&key, revision), ordinary.get(&key));
                }
            }
            assert_eq!(shared.len(), ordinary.0.read().unwrap().len);
            let old_revision = (
                revision.0,
                revision.1.wrapping_sub(1),
                revision.2,
                revision.3,
            );
            shared.insert(cache_key(0), Some(999.0), old_revision);
            assert!(shared.get(&cache_key(0), old_revision).is_none());
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(4)
                .build()
                .unwrap();
            pool.install(|| {
                (0..16_384).into_par_iter().for_each(|index| {
                    let key = cache_key(index % 4096);
                    if let Some(value) = shared.get(&key, revision) {
                        assert_eq!(value, cache_value(&key));
                    }
                    shared.insert(key, cache_value(&key), revision);
                })
            });
            assert!(shared.len() <= 2 * count);
            assert_eq!(
                shared
                    .quotes
                    .each_mut()
                    .map(|quotes| quotes.get_mut().unwrap().buckets.capacity()),
                capacities
            );
            shared.clear();
            assert_eq!(shared.len(), 0);
            for index in 0..4096 {
                assert!(shared.get(&cache_key(index), revision).is_none());
            }
        }
    }

    #[test]
    #[ignore = "manual paired release timing of shared quote lock contention"]
    fn benchmark_striped_network_quotes() {
        use super::*;
        use rayon::prelude::*;
        use std::{hint::black_box, time::Instant};
        let mut network = TransitNetwork::new();
        network
            .lane_system
            .lanes
            .resize_with(32_768, Default::default);
        let revision = network_quote_revision(&network);
        let mut shared = SharedNetworkQuotes::default();
        shared.sync(&network);
        let mut ordinary = SingleLockQuotes::new(shared.bucket_mask + 1);
        let keys: Vec<_> = (0..32_768).map(cache_key).collect();
        let checksum = |value: Option<f32>| value.map_or(1, |value| u64::from(value.to_bits()) + 2);
        let expected = keys
            .iter()
            .map(|key| checksum(cache_value(key)))
            .sum::<u64>()
            * 4;
        for workers in [1, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            for warm in [false, true] {
                for pair in 0..11 {
                    for striped in if pair % 2 == 0 {
                        [false, true]
                    } else {
                        [true, false]
                    } {
                        shared.clear();
                        let slots = ordinary.0.get_mut().unwrap();
                        slots.buckets.fill([None; 2]);
                        slots.len = 0;
                        if warm {
                            for &key in &keys {
                                shared.insert(key, cache_value(&key), revision);
                                ordinary.insert(key, cache_value(&key));
                            }
                        }
                        let begin = Instant::now();
                        let actual = pool.install(|| {
                            (0..4)
                                .map(|_| {
                                    keys.par_iter()
                                        .map(|&key| {
                                            let cached = if striped {
                                                shared.get(&key, revision)
                                            } else {
                                                ordinary.get(&key)
                                            };
                                            let value = cached.unwrap_or_else(|| {
                                                let value = cache_value(&key);
                                                if striped {
                                                    shared.insert(key, value, revision);
                                                } else {
                                                    ordinary.insert(key, value);
                                                }
                                                value
                                            });
                                            checksum(black_box(value))
                                        })
                                        .sum::<u64>()
                                })
                                .sum::<u64>()
                        });
                        let query_ns =
                            begin.elapsed().as_secs_f64() * 1e9 / (keys.len() * 4) as f64;
                        assert_eq!(actual, expected);
                        eprintln!(
                            "STRIPED_QUOTES {}",
                            serde_json::json!({"workers":workers,
                            "warm":warm,"pair":pair,"striped":striped,"query_ns":query_ns})
                        );
                    }
                }
            }
        }
    }

    use super::*;
    use crate::simulation::network::types::NodeType;
    use crate::simulation::pathing::cch::CchGraph;
    use godot::prelude::Vector3;

    #[test]
    fn prepared_commute_queries_match_connector_paths_without_growing_storage() {
        let mut graph = RegionGraph::new();
        for (x, z) in [
            (0.0, 0.0),
            (100.0, 0.0),
            (200.0, 0.0),
            (300.0, 0.0),
            (100.0, 100.0),
            (200.0, 100.0),
        ] {
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
        let foot = network
            .lane_system
            .lanes
            .iter()
            .position(|lane| lane.edge_id != usize::MAX && lane.lane_type == LaneType::Foot)
            .unwrap();
        let node = lane_origin_node(foot, &network, &graph).unwrap();
        assert!(scratch.has_connectors(&[node], node, RouteStart::Node, foot, &network, &graph));
        scratch.retain_network_path(true);
        assert!(scratch.finish_path().is_empty());
        assert!(
            scratch.revision.is_none(),
            "zero-hop checks must not prepare city-sized scratch"
        );
        scratch.prepare(&network, &graph);
        assert_eq!(
            [
                scratch.previous.capacity(),
                scratch.path.capacity(),
                scratch.retained_path.capacity()
            ],
            [0; 3],
            "cost forecasts must not allocate full-route buffers"
        );
        scratch.prepare_routes(&network, &graph);
        assert!(scratch.connectors.complete);
        // Replacing a required connector with a duplicate preserves the list's length,
        // but must withdraw the all-paths proof. Restore the exact original topology.
        let changed_lane = network
            .lane_system
            .lanes
            .iter()
            .position(|lane| {
                lane.edge_id != usize::MAX
                    && lane.lane_type == LaneType::Foot
                    && lane.next_lanes.len() >= 2
            })
            .unwrap();
        let original = network.lane_system.lanes[changed_lane].next_lanes.clone();
        let mut duplicate = original.to_vec();
        duplicate[0] = duplicate[1];
        network.lane_system.lanes[changed_lane].next_lanes = duplicate.into();
        let links = network
            .lane_system
            .lanes
            .iter()
            .map(|lane| lane.next_lanes.len())
            .sum();
        scratch.connectors.prepare(&network, &graph, links);
        assert!(!scratch.connectors.complete);
        network.lane_system.lanes[changed_lane].next_lanes = original;
        scratch.connectors.prepare(&network, &graph, links);
        assert!(scratch.connectors.complete);
        let capacities = |scratch: &TripQueryScratch| {
            [
                scratch.possible.capacity(),
                scratch.next.capacity(),
                scratch.connectors.offsets.capacity(),
                scratch.connectors.targets.capacity(),
                scratch.connectors.outgoing_counts.capacity(),
                scratch.connectors.seen.capacity(),
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
        let mut equivalent_leg_hits = 0;
        // A second worker shares legs only through `shared`, never its own slots.
        let mut other = TripQueryScratch::default();
        let mut shared = SharedNetworkQuotes::default();
        let mut bucketed = SharedNetworkQuotes::default();
        let mut batched = TripQueryScratch::default();
        let batch_counter = AtomicU32::new(0);
        let mut last_key = None;
        for change in 0..5 {
            if change == 1 {
                graph.edge_mut(1).base_cost += 17.0;
                network.cch_graph.customize(&graph);
            } else if change == 2 {
                let lane = lanes
                    .iter()
                    .copied()
                    .find(|&lane| network.lane_system.lanes[lane].lane_type == LaneType::Foot)
                    .unwrap();
                network.lane_system.lanes[lane].next_lanes.clear();
                network.rebuild_cch_and_check(&graph);
            } else if change == 3 {
                for edge in 0..graph.edge_count() {
                    graph.edge_mut(edge).base_cost = f32::INFINITY;
                }
                network.cch_graph.customize(&graph);
            } else if change == 4 {
                // Terrain synchronization can rebuild lanes without changing the CCH
                // generation or lane count. The lane revision still expires every proof.
                let previous = network_quote_revision(&network);
                network.lane_system.rebuild(&mut graph);
                let next = network_quote_revision(&network);
                assert_eq!(
                    (previous.0, previous.1, previous.2),
                    (next.0, next.1, next.2)
                );
                assert_ne!(previous.3, next.3);
            }
            scratch.prepare(&network, &graph);
            other.prepare(&network, &graph);
            assert_eq!(scratch.connectors.complete, matches!(change, 0 | 1 | 4));
            let mut expected_targets = Vec::new();
            let mut cached_targets = Vec::new();
            for lane in 0..network.lane_system.lanes.len() {
                for node in 0..graph.node_count() as u32 {
                    for edge in 0..graph.edge_count() {
                        expected_targets.clear();
                        cached_targets.clear();
                        collect_connector_targets_to_edge(
                            lane,
                            node,
                            edge,
                            &network,
                            &graph,
                            &mut expected_targets,
                        );
                        scratch
                            .connectors
                            .collect(lane, node, edge, &mut cached_targets);
                        assert_eq!(
                            cached_targets, expected_targets,
                            "change={change} lane={lane} node={node} edge={edge}"
                        );
                    }
                }
            }
            let revision = network_quote_revision(&network);
            assert!(
                last_key.is_none_or(|key| shared.get(&key, revision).is_none()),
                "legs priced on an older revision must never answer"
            );
            shared.sync(&network);
            assert_eq!(shared.len(), 0, "every edit here changes the revision");
            // Deliberately collide every leg, including unreachable ones; the bucketed
            // owner below separately exercises the ordinary network-sized reserve.
            shared.bucket_mask = 0;
            for (stripe, quotes) in shared.quotes.iter_mut().enumerate() {
                quotes
                    .get_mut()
                    .unwrap()
                    .buckets
                    .truncate(usize::from(stripe == 0));
            }
            let shared_capacity = shared
                .quotes
                .each_mut()
                .map(|quotes| quotes.get_mut().unwrap().buckets.capacity());
            bucketed.prepare_targets(
                std::array::from_fn(|_| {
                    (0..graph.node_count() as u32)
                        .map(|node| (node, usize::MAX))
                        .collect()
                }),
                &network,
            );
            batched.prepare(&network, &graph);
            batched.prepare_batch(&bucketed, &network);
            let batch_capacities = batched.batch_sources.each_ref().map(|mode| {
                mode.each_ref()
                    .map(|source| (source.source.capacities(), source.frontiers.capacity()))
            });

            for &attach in &lanes {
                for &detach in &lanes {
                    let lane_type = network.lane_system.lanes[attach].lane_type;
                    if network.lane_system.lanes[detach].lane_type != lane_type {
                        continue;
                    }
                    let mode = if lane_type == LaneType::Vehicle {
                        MODE_CAR
                    } else {
                        0
                    };
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
                            (
                                RouteStart::Lane(attach),
                                super::owned_oracle::pedestrian_lane_connector_path(
                                    origin.node,
                                    destination.node,
                                    attach,
                                    detach,
                                    &network,
                                    &graph,
                                ),
                            ),
                            (
                                RouteStart::Edge(origin.edge_idx),
                                super::owned_oracle::pedestrian_lane_connector_path_from_edge(
                                    origin.node,
                                    destination.node,
                                    origin.edge_idx,
                                    detach,
                                    &network,
                                    &graph,
                                ),
                            ),
                            (
                                RouteStart::Node,
                                super::owned_oracle::pedestrian_lane_connector_path_from_node(
                                    origin.node,
                                    destination.node,
                                    detach,
                                    &network,
                                    &graph,
                                ),
                            ),
                        ] {
                            let actual = scratch
                                .walking_path(
                                    origin.node,
                                    destination.node,
                                    start,
                                    detach,
                                    &network,
                                    &graph,
                                )
                                .map(|(seconds, path)| (seconds.to_bits(), path.to_vec()));
                            assert_eq!(
                                actual,
                                expected.map(|(seconds, path)| (seconds.to_bits(), path)),
                                "connector path: change={change}, attach={attach}, detach={detach}"
                            );
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
                    let expected = expected_candidate
                        .as_ref()
                        .map(|candidate| candidate.total_cost_s);
                    assert_eq!(
                        batched
                            .combine(
                                mode,
                                0,
                                0,
                                &origin,
                                &destination,
                                &network,
                                &graph,
                                &batch_counter,
                                Some(&bucketed)
                            )
                            .map(|candidate| candidate.total_cost_s),
                        expected
                    );
                    assert_eq!(
                        batched
                            .batch_sources
                            .each_ref()
                            .map(|mode| mode.each_ref().map(|source| (
                                source.source.capacities(),
                                source.frontiers.capacity()
                            ))),
                        batch_capacities
                    );

                    let route = scratch
                        .network_path(
                            mode,
                            origin.node,
                            destination.node,
                            RouteStart::Lane(attach),
                            detach,
                            &network,
                            &graph,
                            <[u32]>::to_vec,
                        )
                        .ok()
                        .map(|(_, path)| path);
                    assert_eq!(
                        route,
                        expected_candidate.and_then(|candidate| candidate.network_path)
                    );
                    if let Some(route) = route {
                        scratch.retain_network_path(false);
                        assert_eq!(scratch.finish_path(), route);
                        scratch.retain_network_path(true);
                        assert!(scratch.finish_path().is_empty());
                    }
                    let before = scratch.epoch;
                    assert_eq!(
                        scratch
                            .combine(
                                mode,
                                0,
                                0,
                                &origin,
                                &destination,
                                &network,
                                &graph,
                                &counter,
                                None
                            )
                            .map(|candidate| candidate.total_cost_s),
                        expected
                    );
                    fallback_count += usize::from(scratch.epoch != before);
                    let searches = counter.load(Ordering::Relaxed);
                    assert_eq!(
                        scratch
                            .combine(
                                mode,
                                0,
                                0,
                                &origin,
                                &destination,
                                &network,
                                &graph,
                                &counter,
                                None
                            )
                            .map(|candidate| candidate.total_cost_s),
                        expected
                    );
                    assert_eq!(
                        counter.load(Ordering::Relaxed),
                        searches,
                        "an identical lane leg must reuse both successful and failed proofs"
                    );
                    let key = other.quote_key(mode, &origin, &destination, &network, &graph);
                    let shared_missing = shared.get(&key, revision).is_none();
                    equivalent_leg_hits += usize::from(!shared_missing);
                    for _ in 0..2 {
                        // Midnight and unchanged-target passes must retain pure network
                        // legs even when the worker's own scratch has no matching quote.
                        shared.sync(&network);
                        other.network_quotes.fill(None);
                        assert_eq!(
                            other
                                .combine(
                                    mode,
                                    0,
                                    0,
                                    &origin,
                                    &destination,
                                    &network,
                                    &graph,
                                    &counter,
                                    Some(&shared)
                                )
                                .map(|candidate| candidate.total_cost_s),
                            expected
                        );
                    }
                    assert_eq!(
                        counter.load(Ordering::Relaxed),
                        searches + u32::from(shared_missing),
                        "a shared leg is searched once, then answered for another worker"
                    );
                    assert!(shared.len() <= 2);
                    assert_eq!(
                        shared.quotes.each_mut().map(|quotes| quotes
                            .get_mut()
                            .unwrap()
                            .buckets
                            .capacity()),
                        shared_capacity
                    );
                    last_key = Some(key);
                    // Different doors and live frontage charges share only the network leg.
                    let mut moved_origin = origin;
                    moved_origin.local_time_s = 9.0;
                    moved_origin.frontage_time_s = Some(19.0);
                    let mut moved_destination = destination;
                    moved_destination.local_time_s = 7.0;
                    moved_destination.frontage_time_s = Some(23.0);
                    let moved_expected = super::owned_oracle::combine_trip_ends(
                        mode,
                        0,
                        0,
                        &moved_origin,
                        &moved_destination,
                        &network,
                        &graph,
                        &counter,
                    )
                    .map(|candidate| candidate.total_cost_s);
                    let searches = counter.load(Ordering::Relaxed);
                    assert_eq!(
                        scratch
                            .combine(
                                mode,
                                0,
                                0,
                                &moved_origin,
                                &moved_destination,
                                &network,
                                &graph,
                                &counter,
                                None
                            )
                            .map(|candidate| candidate.total_cost_s),
                        moved_expected
                    );
                    other.network_quotes.fill(None);
                    assert_eq!(
                        other
                            .combine(
                                mode,
                                0,
                                0,
                                &moved_origin,
                                &moved_destination,
                                &network,
                                &graph,
                                &counter,
                                Some(&shared)
                            )
                            .map(|candidate| candidate.total_cost_s),
                        moved_expected
                    );
                    assert_eq!(counter.load(Ordering::Relaxed), searches);
                    assert_eq!(capacities(&scratch), prepared);
                }
            }
        }
        assert!(
            fallback_count > 0,
            "fixture must exercise the directed lane fallback"
        );
        assert!(
            equivalent_leg_hits > 0,
            "distinct lane pairs must share their proved network-only key"
        );
    }

    type RouteCase = (u8, [TripEnd; 2], [TripEnd; 2]);
    type RouteResult = (u32, u8, u8, usize, usize, Vec<u32>, u8);

    #[test]
    fn shared_entrance_geometry_matches_independent_direction_and_rank_construction() {
        use godot::prelude::Vector2;
        let (graph, mut network, _) = route_cases(4);
        let mut entrances = Vec::new();
        for edge in 0..graph.edge_count() {
            for side in [-1, 1] {
                for position in [0.0, 37.25, 100.0] {
                    let lane = |kind, forward| {
                        network.lane_system.edge_lanes[&edge]
                            .iter()
                            .copied()
                            .find(|&id| {
                                let lane = &network.lane_system.lanes[id];
                                lane.lane_type == kind
                                    && lane.is_fwd == forward
                                    && (kind != LaneType::Foot || lane.lane_idx.signum() == side)
                            })
                            .unwrap()
                    };
                    let door = Vector2::new(
                        graph.edge(edge).start_node as f32 * 100.0 + position,
                        side as f32 * 16.0,
                    );
                    entrances.push(BuildingEntrance {
                        edge_idx: edge,
                        side,
                        entrance_s_m: position,
                        door_pos: door,
                        curb_pos: Vector2::new(door.x, side as f32 * 5.0),
                        foot_lane_fwd: lane(LaneType::Foot, true),
                        foot_lane_bkw: lane(LaneType::Foot, false),
                        car_lane_fwd: lane(LaneType::Vehicle, true),
                        car_lane_bkw: lane(LaneType::Vehicle, false),
                        ..Default::default()
                    });
                }
            }
        }
        entrances.push(BuildingEntrance::default());
        let mut missing = entrances[0].clone();
        missing.foot_lane_bkw = usize::MAX;
        missing.car_lane_fwd = usize::MAX;
        entrances.push(missing);
        let mut mismatched = entrances[0].clone();
        // A foreign lane can validate one endpoint but fail the opposite endpoint.
        mismatched.foot_lane_fwd = entrances[6].foot_lane_bkw;
        mismatched.foot_lane_bkw = entrances[6].foot_lane_fwd;
        entrances.push(mismatched);
        let mut one_side = entrances[0].clone();
        one_side.vehicle_frontage_access =
            crate::simulation::network::types::VehicleFrontageAccess::SameSideOnly;
        entrances.push(one_side);
        let end_key = |end: Option<TripEnd>| {
            end.map(|end| {
                (
                    end.edge_idx,
                    end.node,
                    end.node_x.to_bits(),
                    end.node_z.to_bits(),
                    end.lane_id,
                    end.lane_d.to_bits(),
                    end.local_time_s.to_bits(),
                    end.frontage_time_s.map(f32::to_bits),
                )
            })
        };
        let candidate_key = |candidate: Option<PlannedTripCandidate>| {
            candidate.map(|candidate| {
                (
                    candidate.total_cost_s.to_bits(),
                    candidate.mode_choice_cost_s.to_bits(),
                    candidate.mode,
                    candidate.origin_rank,
                    candidate.destination_rank,
                    candidate.planned_attach_node,
                    candidate.planned_detach_node,
                    candidate.planned_attach_lane_id,
                    candidate.planned_detach_lane_id,
                    candidate.planned_attach_lane_d.to_bits(),
                    candidate.planned_detach_lane_d.to_bits(),
                )
            })
        };
        let counter = AtomicU32::new(0);
        for price in [0.0, 17.25, f32::INFINITY] {
            for lane in &mut network.lane_system.lanes {
                lane.live_frontage_delay_s = price;
            }
            for mode in [0, MODE_CAR] {
                for entrance in &entrances {
                    let pair = TripEnd::pair(mode, entrance, &network, &graph);
                    for (direction, origin) in [true, false].into_iter().enumerate() {
                        for rank in NODE_RANKS {
                            assert_eq!(
                                end_key(pair[direction][usize::from(rank)]),
                                end_key(TripEnd::new(
                                    mode, rank, entrance, origin, &network, &graph
                                ))
                            );
                        }
                    }
                    for destination in &entrances {
                        let mut expected = None;
                        for (from, to) in NODE_RANK_PAIRS {
                            if let Some(candidate) = evaluate_planned_trip_candidate(
                                mode,
                                from,
                                to,
                                entrance,
                                destination,
                                &network,
                                &graph,
                                &counter,
                            ) && expected
                                .as_ref()
                                .is_none_or(|best| candidate_better(&candidate, best))
                            {
                                expected = Some(candidate);
                            }
                        }
                        assert_eq!(
                            candidate_key(best_trip_candidate_for_mode(
                                mode,
                                entrance,
                                destination,
                                &network,
                                &graph,
                                &counter
                            )),
                            candidate_key(expected)
                        );
                    }
                }
            }
        }
    }

    fn route_cases(edges: usize) -> (RegionGraph, TransitNetwork, Vec<RouteCase>) {
        let mut graph = RegionGraph::new();
        for index in 0..=edges {
            graph.add_node(
                Vector3::new(index as f32 * 100.0, 0.0, 0.0),
                NodeType::Junction,
            );
        }
        for index in 0..edges {
            graph.add_edge(super::super::tests::test_edge(
                index as u32,
                index as u32 + 1,
                index as f32 * 100.0,
                (index + 1) as f32 * 100.0,
            ));
        }
        graph.rebuild_adjacency_list();
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        network.cch_graph = CchGraph::build(&graph);
        let ends = |edge: usize, mode, origin| {
            std::array::from_fn(|rank| {
                let node = if rank == 0 {
                    graph.edge(edge).start_node
                } else {
                    graph.edge(edge).end_node
                };
                let lane = network.lane_system.edge_lanes[&edge]
                    .iter()
                    .copied()
                    .find(|&lane| {
                        let value = &network.lane_system.lanes[lane];
                        let kind = if mode == MODE_CAR {
                            LaneType::Vehicle
                        } else {
                            LaneType::Foot
                        };
                        value.lane_type == kind
                            && (if origin {
                                lane_terminal_node(lane, &network, &graph)
                            } else {
                                lane_origin_node(lane, &network, &graph)
                            }) == Some(node)
                    })
                    .unwrap();
                TripEnd {
                    edge_idx: edge,
                    node,
                    node_x: graph.node(node).pos.x,
                    node_z: 0.0,
                    lane_id: lane,
                    lane_d: 50.0,
                    local_time_s: 1.0,
                    frontage_time_s: Some(2.0),
                }
            })
        };
        let cases = (0..512)
            .map(|index| {
                let mode = if index % 2 == 0 { MODE_CAR } else { 0 };
                (
                    mode,
                    ends(index * 7 % edges, mode, true),
                    ends((index * 17 + edges / 2) % edges, mode, false),
                )
            })
            .collect();
        (graph, network, cases)
    }

    fn planned_case(
        owned: bool,
        case: &RouteCase,
        network: &TransitNetwork,
        graph: &RegionGraph,
        counter: &AtomicU32,
    ) -> Option<RouteResult> {
        let (mode, origins, destinations) = case;
        let mut best = None;
        for (from, to) in NODE_RANK_PAIRS {
            let quote = if owned {
                super::owned_oracle::combine_trip_ends
            } else {
                combine_trip_ends
            };
            if let Some(candidate) = quote(
                *mode,
                from,
                to,
                &origins[from as usize],
                &destinations[to as usize],
                network,
                graph,
                counter,
            ) && best
                .as_ref()
                .is_none_or(|best| candidate_better(&candidate, best))
            {
                best = Some(candidate);
            }
        }
        let mut candidate = best?;
        let (path, flags) = if owned {
            let direct = candidate.planned_attach_lane_id == candidate.planned_detach_lane_id
                && candidate.planned_attach_lane_d <= candidate.planned_detach_lane_d + 1e-6;
            let flags = ACCESS_PLAN_VALID
                | if !direct && candidate.planned_attach_node == candidate.planned_detach_node {
                    ACCESS_ZERO_HOP_NODE_PATH
                } else {
                    0
                };
            (candidate.network_path.take().unwrap_or_default(), flags)
        } else {
            build_exact_path_for_candidate(
                &mut candidate,
                usize::MAX,
                ZoneType::None,
                network,
                graph,
                counter,
            )?
        };
        Some((
            candidate.total_cost_s.to_bits(),
            candidate.origin_rank,
            candidate.destination_rank,
            candidate.planned_attach_lane_id,
            candidate.planned_detach_lane_id,
            path,
            flags,
        ))
    }

    #[test]
    fn selected_routes_match_owned_costs_ranks_paths_and_access_flags() {
        for edges in [16, 64] {
            let (graph, network, cases) = route_cases(edges);
            let counter = AtomicU32::new(0);
            for case in &cases {
                let expected = planned_case(true, case, &network, &graph, &counter);
                assert!(expected.is_some());
                assert_eq!(
                    planned_case(false, case, &network, &graph, &counter),
                    expected
                );
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
                assert_eq!(
                    planned_case(false, case, &network, &graph, &counter),
                    expected
                );
            }
            for reverse in [false, true] {
                for owned in if reverse {
                    [false, true]
                } else {
                    [true, false]
                } {
                    let mut samples = [0.0_f64; 15];
                    for round in 0..18 {
                        let start = Instant::now();
                        cases.par_iter().for_each(|case| {
                            black_box(planned_case(owned, case, &network, &graph, &counter));
                        });
                        if round >= 3 {
                            samples[round - 3] = start.elapsed().as_secs_f64() * 1_000.0;
                        }
                    }
                    samples.sort_by(f64::total_cmp);
                    eprintln!(
                        "route_products edges={edges} trips={} owned={owned} reverse={reverse} median_ms={:.3} max_ms={:.3}",
                        cases.len(),
                        samples[7],
                        samples[14]
                    );
                }
            }
        }
    }
}
