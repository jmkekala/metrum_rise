// SPDX-License-Identifier: GPL-2.0-only

//! Revision-scoped network costs; live frontage delay is evaluated at quotation time.

use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;

use crate::simulation::buildings::allocator::{
    BuildingAllocator, BuildingEntrance, BuildingEntranceKey, FreightEntranceAccess,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::pathing::cch::{CchCarCostBounds, CchCostsFrom, CchQueryScratch};

fn sweep_border(
    border_node: u32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> CchCostsFrom {
    let mut costs =
        transit_network
            .cch_graph
            .costs_from_each(&[border_node], graph, TransitFlags::CAR);
    costs.pop().expect("one start yields one cost table")
}

// Generation, sampled revision and ETA of one border-to-building quote.
type BorderQuote = (u64, u64, Option<f32>);

/// Caches expensive network searches without freezing changing frontage delays.
#[derive(Clone, Debug, Default)]
pub(super) struct FreightRouteCache {
    // Two most-recent exact entries per fixed hash bucket. Unlike retaining the first
    // pairs forever, bounded replacement follows changing demand without query allocation.
    node_to_node: Vec<[Option<NodeCost>; 2]>,
    node_hash: foldhash::fast::FixedState,
    /// Lifetime node-cost requests, retained hits, and misses requiring replacement.
    #[cfg(test)]
    pub(super) node_cache_counts: [u64; 3],
    /// Building requests, valid hits, replacement misses, and stale retained keys.
    #[cfg(test)]
    pub(super) building_cache_counts: [u64; 4],
    query: CchQueryScratch,
    // Network searches made on cache misses so far, which the funding census counts as work
    // (`SIM-02-K`); monotonic, never read by any result.
    searches: u64,
    cost_bounds: CchCarCostBounds,
    // Complete quotes also compare their own lane prices after a global traffic revision.
    building_to_building: Vec<[Option<BuildingCost>; 2]>,
    border_to_building: HashMap<(u32, usize), BorderQuote, foldhash::fast::FixedState>,
    building_generation: u64,
    access: Vec<Option<BuildingAccess>>,
    // Two current proposal doors, independent of allocator IDs. Repeated buyer/supplier
    // scans reuse projection while their exact frontage times remain live.
    site_access: [Option<(BuildingEntranceKey, FreightEntranceAccess)>; 2],
    site_lane_revision: u64,
    // Car costs from each border to every node, swept once per revision: the network leg of
    // every border ETA, instead of one route query per destination building.
    border_costs: BTreeMap<u32, CchCostsFrom>,
    // Exact minimum of the retained per-border tables, not a differently rounded multi-source
    // sweep. Frontage additions are monotone in the network cost, so minimization commutes
    // with those unchanged additions. Only ETA is returned; dispatch retains border identity.
    minimum_borders: Vec<u32>,
    minimum_border_costs: Vec<f32>,
}

#[derive(Clone, Copy, Debug)]
struct NodeCost {
    key: (u32, u32),
    cost: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
struct BuildingCost {
    key: (usize, usize),
    generation: u64,
    geometry: Option<[u64; 2]>,
    revision: u64,
    prices: [u32; 4],
    cost: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
struct BuildingAccess {
    checked: u64,
    geometry: u64,
    lanes: u64,
    speeds: [Option<u32>; 2],
    entrance: BuildingEntranceKey,
    access: FreightEntranceAccess,
}

impl BuildingAccess {
    fn refresh(
        previous: Option<Self>,
        generation: u64,
        entrance: &BuildingEntrance,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Self {
        let key = BuildingEntranceKey::new(entrance);
        let speeds = [entrance.car_lane_fwd, entrance.car_lane_bkw].map(|lane| {
            network
                .lane_system
                .lanes
                .get(lane)
                .and_then(|lane| graph.get_edge(lane.edge_id))
                .map(|edge| edge.speed_limit.to_bits())
        });
        if let Some(previous) = previous
            && previous.entrance == key
            && previous.lanes == network.lane_system.revision()
            && previous.speeds == speeds
        {
            return Self {
                checked: generation,
                ..previous
            };
        }
        Self {
            checked: generation,
            geometry: generation,
            lanes: network.lane_system.revision(),
            speeds,
            entrance: key,
            access: FreightEntranceAccess::new(entrance, network, graph),
        }
    }
}

fn frontage_prices(
    key: (usize, usize),
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
) -> Option<[u32; 4]> {
    let from = allocator.entrances.get(key.0)?;
    let to = allocator.entrances.get(key.1)?;
    // Entrance generation fixes lane identities/presence, geometry and free-flow inputs.
    // Include both possible lanes at both doors, even when one loses the current minimum.
    Some(
        [
            from.car_lane_fwd,
            from.car_lane_bkw,
            to.car_lane_fwd,
            to.car_lane_bkw,
        ]
        .map(|lane| {
            network
                .lane_system
                .lanes
                .get(lane)
                .map_or(0, |lane| lane.frontage_delay_penalty_s.to_bits())
        }),
    )
}

// `Vec::resize`, with new rows written in parallel so a first fill's page faults spread over
// the workers (`SIM-02-K`). O(new rows / workers).
fn resize_in_parallel<T: Clone + Send>(rows: &mut Vec<T>, len: usize, value: T) {
    match len.checked_sub(rows.len()) {
        Some(new) => rows.par_extend(rayon::iter::repeat_n(value, new)),
        None => rows.truncate(len),
    }
}

fn retained_quote<T: Copy>(entries: &mut [Option<T>; 2], matches: impl Fn(T) -> bool) -> Option<T> {
    for slot in 0..2 {
        if let Some(entry) = entries[slot]
            && matches(entry)
        {
            entries.swap(0, slot);
            return Some(entry);
        }
    }
    None
}

impl FreightRouteCache {
    /// Reserves indexed geometry slots without visiting any unchanged building.
    /// Queries fill requested slots lazily; preparation is O(new building slots), warm O(1).
    // Network searches made on cache misses so far; O(1).
    pub(super) fn searches(&self) -> u64 {
        self.searches
    }

    pub(super) fn prepare_buildings(&mut self, buildings: usize) {
        // Spare rows, so the first building appended after a load, which sized the table
        // exactly, does not copy it (`SIM-02-K`).
        crate::utils::storage::reserve_rows(&mut self.access, buildings);
        resize_in_parallel(&mut self.access, buildings, None);
    }

    /// Fills requested geometry in the existing cache before a parallel quote batch.
    /// O(buildings) tag checks; only changed requested entrances project their lane geometry.
    #[cfg(test)]
    pub(super) fn prepare_access_for(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        wanted: impl Fn(usize) -> bool + Sync,
    ) {
        let buildings = self.access.len();
        self.prepare_access_in(0..buildings, allocator, network, graph, wanted);
    }

    /// [`Self::prepare_access_for`] over buildings `range` only (`SIM-02-K`): each slot depends
    /// on its own entrance alone, so ranges in any order give the whole call's cache.
    /// O(range) tag checks in parallel.
    pub(super) fn prepare_access_in(
        &mut self,
        range: std::ops::Range<usize>,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        wanted: impl Fn(usize) -> bool + Sync,
    ) {
        let generation = self.building_generation;
        let first = range.start;
        self.access[range]
            .par_iter_mut()
            .enumerate()
            .map(|(offset, slot)| (first + offset, slot))
            .for_each(|(index, slot)| {
                if slot
                    .as_ref()
                    .is_some_and(|cached| cached.checked == generation)
                    || !wanted(index)
                {
                    return;
                }
                if let Some(entrance) = allocator.entrances.get(index) {
                    *slot = Some(BuildingAccess::refresh(
                        *slot, generation, entrance, network, graph,
                    ));
                }
            });
    }

    fn building_access(
        &mut self,
        index: usize,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<FreightEntranceAccess> {
        if index >= allocator.buildings.len() {
            return None;
        }
        if let Some(Some(access)) = self.access.get(index)
            && access.checked == self.building_generation
        {
            return Some(access.access);
        }
        let previous = self.access.get(index).copied().flatten();
        let access = BuildingAccess::refresh(
            previous,
            self.building_generation,
            allocator.entrances.get(index)?,
            network,
            graph,
        );
        // Before the next batch reserve, newly appended doors use the same exact fallback
        // without allocating. A retained slot is filled at most once per entrance revision.
        if let Some(slot) = self.access.get_mut(index) {
            *slot = Some(access);
        }
        Some(access.access)
    }

    fn geometry(&self, key: (usize, usize)) -> Option<[u64; 2]> {
        let from = self.access.get(key.0)?.as_ref()?;
        let to = self.access.get(key.1)?.as_ref()?;
        (from.checked == self.building_generation && to.checked == self.building_generation)
            .then_some([from.geometry, to.geometry])
    }

    fn site_access(
        &mut self,
        entrance: &BuildingEntrance,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> FreightEntranceAccess {
        if self.site_lane_revision != network.lane_system.revision() {
            self.site_access = [None; 2];
            self.site_lane_revision = network.lane_system.revision();
        }
        let key = BuildingEntranceKey::new(entrance);
        if let Some(index) = self
            .site_access
            .iter()
            .position(|entry| entry.as_ref().is_some_and(|(cached, _)| *cached == key))
        {
            if index != 0 {
                self.site_access.swap(0, index);
            }
            return self.site_access[0].as_ref().unwrap().1;
        }
        let access = FreightEntranceAccess::new(entrance, network, graph);
        self.site_access[1] = self.site_access[0];
        self.site_access[0] = Some((key, access));
        access
    }

    /// Retained entry counts and capacities for bounded quotation storage regressions.
    #[cfg(test)]
    pub(super) fn storage(&self) -> [(usize, usize); 3] {
        [
            (
                self.node_to_node
                    .iter()
                    .flatten()
                    .filter(|entry| entry.is_some())
                    .count(),
                self.node_to_node.len() * 2,
            ),
            (
                self.building_to_building
                    .iter()
                    .flatten()
                    .filter(|entry| entry.is_some())
                    .count(),
                self.building_to_building.len() * 2,
            ),
            (
                self.border_to_building.len(),
                self.border_to_building.capacity(),
            ),
        ]
    }

    /// Prepares the existing border sweeps for parallel, allocation-free building quotes.
    pub(super) fn prepare_border_costs(
        &mut self,
        borders: &[u32],
        buildings: usize,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.prepare_border_storage(buildings, transit_network, graph);
        self.sweep_borders(borders, transit_network, graph);
        self.prepare_border_minimum(borders, graph);
    }

    /// [`Self::prepare_border_costs`]'s first phase (`SIM-02-K`): the query scratch, the cost
    /// bounds and the bounded quote storage. O(N + E) after a metric change, O(1) warm; the
    /// storage grows only with the road graph and the buildings.
    pub(super) fn prepare_border_storage(
        &mut self,
        buildings: usize,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.query.prepare(&transit_network.cch_graph, graph);
        self.cost_bounds.prepare(&transit_network.cch_graph, graph);
        // Bound retained pair costs by road graph storage rather than all possible N²
        // pairs. Each lookup/replace is worst-case O(1), with fewer than twice this many
        // slots after power-of-two rounding. Preparation alone may allocate or rehash.
        let entries = 2 * graph.edge_count() + graph.node_count();
        let buckets = entries.div_ceil(2).max(1).next_power_of_two();
        if self.node_to_node.len() < buckets {
            self.node_to_node.clear();
            resize_in_parallel(&mut self.node_to_node, buckets, [None; 2]);
        }
        let complete = entries + buildings;
        let buckets = complete.div_ceil(2).max(1).next_power_of_two();
        if self.building_to_building.len() < buckets {
            self.building_to_building.clear();
            resize_in_parallel(&mut self.building_to_building, buckets, [None; 2]);
        }
        if self.border_to_building.capacity() < complete {
            self.border_to_building
                .reserve(complete.saturating_sub(self.border_to_building.len()));
        }
    }

    /// Sweeps the car costs from each of `borders` not swept since the last routing change
    /// (`SIM-02-K`): one CCH sweep each, O(N + shortcuts). Returns the borders swept.
    pub(super) fn sweep_borders(
        &mut self,
        borders: &[u32],
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> usize {
        let mut swept = 0;
        for &border in borders {
            self.border_costs.entry(border).or_insert_with(|| {
                swept += 1;
                sweep_border(border, transit_network, graph)
            });
        }
        swept
    }

    /// The exact minimum over every border's sweep at each node, after [`Self::sweep_borders`]
    /// covered `borders` (`SIM-02-K`). O(N × K) in parallel after a routing or border change.
    pub(super) fn prepare_border_minimum(&mut self, borders: &[u32], graph: &RegionGraph) {
        if self.minimum_borders != borders || self.minimum_border_costs.len() != graph.node_count()
        {
            self.minimum_borders.clear();
            self.minimum_borders.extend_from_slice(borders);
            self.minimum_border_costs
                .resize(graph.node_count(), f32::MAX);
            self.minimum_border_costs
                .par_iter_mut()
                .enumerate()
                .for_each(|(node, cost)| {
                    *cost = borders
                        .iter()
                        .filter_map(|border| {
                            // The original entrance quote treats a source equal to its arrival as zero.
                            if *border == node as u32 {
                                Some(0.0)
                            } else {
                                self.border_costs.get(border)?.cost_to(node as u32)
                            }
                        })
                        .min_by(f32::total_cmp)
                        .unwrap_or(f32::MAX);
                });
        }
    }

    /// Reads prepared cost tables without mutating the per-building dispatch cache.
    pub(super) fn minimum_border_eta(
        &self,
        destination: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        if let Some(Some(access)) = self.access.get(destination)
            && access.checked == self.building_generation
        {
            return self.minimum_border_eta_from_access(&access.access, transit_network, graph);
        }
        self.prepared_minimum_border_eta(
            allocator.entrances.get(destination)?,
            transit_network,
            graph,
        )
    }

    fn prepared_minimum_border_eta(
        &self,
        entrance: &BuildingEntrance,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        self.minimum_border_eta_from_access(
            &FreightEntranceAccess::new(entrance, network, graph),
            network,
            graph,
        )
    }

    fn minimum_border_eta_from_access(
        &self,
        access: &FreightEntranceAccess,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        access.eta_from_network(network, graph, |node| {
            self.minimum_border_costs
                .get(node as usize)
                .copied()
                .filter(|&cost| cost < f32::MAX)
        })
    }

    pub(super) fn clear(&mut self) {
        // The tables hold megabytes at city scale; fill them in parallel (`SIM-02-K`).
        self.node_to_node
            .par_iter_mut()
            .for_each(|entries| *entries = [None; 2]);
        self.clear_buildings();
        // Network/customization changes can alter projected access and every route even
        // when door inputs match. Only building-only invalidation may retain these proofs.
        self.access.par_iter_mut().for_each(|access| *access = None);
        self.building_to_building
            .par_iter_mut()
            .for_each(|entries| *entries = [None; 2]);
        self.border_costs.clear();
        self.minimum_borders.clear();
        self.minimum_border_costs.clear();
    }

    /// Invalidates entrance/index-dependent quotes in O(1), retaining bounded storage and
    /// unchanged network routes. Generation wrap clears stale entries before reusing zero.
    pub(super) fn clear_buildings(&mut self) {
        self.site_access = [None; 2];
        self.building_generation = self.building_generation.wrapping_add(1);
        if self.building_generation == 0 {
            self.access.fill(None);
            self.building_to_building.fill([None; 2]);
            self.border_to_building.clear();
        }
    }

    /// Reads prepared border sweeps for a proposed door without caching a temporary building ID.
    pub(super) fn minimum_site_border_eta(
        &self,
        borders: &[u32],
        entrance: &BuildingEntrance,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        if self.minimum_borders == borders {
            return self.prepared_minimum_border_eta(entrance, network, graph);
        }
        borders
            .iter()
            .filter_map(|border| {
                BuildingAllocator::freight_car_eta_to_entrance(
                    *border,
                    entrance,
                    self.border_costs.get(border)?,
                    network,
                    graph,
                )
            })
            .min_by(f32::total_cmp)
    }

    /// Quotes a source's directed route to a proposed door without inventing a building ID.
    pub(super) fn building_to_site(
        &mut self,
        source_idx: usize,
        destination: &BuildingEntrance,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        let source = self.building_access(source_idx, allocator, network, graph)?;
        let destination = self.site_access(destination, network, graph);
        source.eta_to(&destination, network, graph, |from, to| {
            self.node_cost(from, to, network, graph)
        })
    }

    /// Quotes a proposed source to an existing buyer using the buyer's prepared geometry.
    pub(super) fn site_to_building(
        &mut self,
        source: &BuildingEntrance,
        destination_idx: usize,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        self.site_access(source, network, graph).eta_to(
            &self.building_access(destination_idx, allocator, network, graph)?,
            network,
            graph,
            |from, to| self.node_cost(from, to, network, graph),
        )
    }

    /// Quotes the directed source-to-customer leg, including both current frontage delays.
    /// Proposed sellers have no allocator ID; reversing `to_site` would price the wrong route.
    pub(super) fn between_entrances(
        &mut self,
        source: &BuildingEntrance,
        destination: &BuildingEntrance,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        let source = self.site_access(source, network, graph);
        let destination = self.site_access(destination, network, graph);
        source.eta_to(&destination, network, graph, |from, to| {
            self.node_cost(from, to, network, graph)
        })
    }

    /// Lower bound over a superset of legal entrance endpoints. Same-edge direct frontage
    /// bypasses the network, so it receives zero. Missing/stale proofs cannot prune a quote.
    pub(super) fn minimum_entrance_network_cost(
        &self,
        source: &BuildingEntrance,
        destination: &BuildingEntrance,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        // Negative local delays could cancel a network leg. Ordinary traffic is nonnegative,
        // but arbitrary edited/test inputs must fall back to the unconstrained zero-price check.
        if [
            source.car_lane_fwd,
            source.car_lane_bkw,
            destination.car_lane_fwd,
            destination.car_lane_bkw,
        ]
        .into_iter()
        .any(|lane| {
            network.lane_system.lanes.get(lane).is_some_and(|lane| {
                let penalty = lane.frontage_delay_penalty_s;
                penalty.is_nan() || penalty < 0.0
            })
        }) {
            return None;
        }
        let from = graph.get_edge(source.edge_idx)?;
        let to = graph.get_edge(destination.edge_idx)?;
        self.cost_bounds
            .cost(from.start_node, from.start_node, &network.cch_graph)?;
        if source.edge_idx == destination.edge_idx {
            return Some(0.0);
        }
        let mut minimum = f32::INFINITY;
        for from in [from.start_node, from.end_node] {
            for to in [to.start_node, to.end_node] {
                minimum = minimum.min(self.cost_bounds.cost(from, to, &network.cch_graph)?);
            }
        }
        Some(minimum)
    }

    fn node_cost(
        &mut self,
        from: u32,
        to: u32,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        #[cfg(test)]
        {
            self.node_cache_counts[0] += 1;
        }
        let key = (from, to);
        let bucket = (!self.node_to_node.is_empty())
            .then(|| self.node_hash.hash_one(key) as usize & (self.node_to_node.len() - 1));
        if let Some(index) = bucket {
            let entries = &mut self.node_to_node[index];
            if let Some(entry) = retained_quote(entries, |entry| entry.key == key) {
                #[cfg(test)]
                {
                    self.node_cache_counts[1] += 1;
                }
                return entry.cost;
            }
        }
        self.searches += 1;
        let cost = network.cch_graph.find_cost(
            from,
            to,
            usize::MAX,
            usize::MAX,
            graph,
            TransitFlags::CAR,
            &mut self.query,
        );
        if let Some(index) = bucket {
            let entries = &mut self.node_to_node[index];
            #[cfg(test)]
            if entries[1].is_some() {
                self.node_cache_counts[2] += 1;
            }
            entries[1] = entries[0];
            entries[0] = Some(NodeCost { key, cost });
        }
        cost
    }

    pub(super) fn between_buildings(
        &mut self,
        source_idx: usize,
        destination_idx: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        #[cfg(test)]
        {
            self.building_cache_counts[0] += 1;
        }
        let key = (source_idx, destination_idx);
        let revision = transit_network.frontage_delay_revision;
        let bucket = (!self.building_to_building.is_empty())
            .then(|| self.node_hash.hash_one(key) as usize & (self.building_to_building.len() - 1));
        let cached = bucket.and_then(|index| {
            retained_quote(&mut self.building_to_building[index], |entry| {
                entry.key == key
            })
        });
        let mut prices = None;
        if let (Some(index), Some(entry)) = (bucket, cached) {
            let geometry_matches = entry.generation == self.building_generation || {
                self.building_access(source_idx, allocator, transit_network, graph);
                self.building_access(destination_idx, allocator, transit_network, graph);
                entry.geometry.is_some() && entry.geometry == self.geometry(key)
            };
            let valid = geometry_matches
                && (entry.revision == revision || {
                    prices = frontage_prices(key, allocator, transit_network);
                    prices == Some(entry.prices)
                });
            if valid {
                // The retained key was promoted to slot zero above. Refresh only its
                // revision tag when all four actual price bits survived publication.
                if entry.revision != revision || entry.generation != self.building_generation {
                    self.building_to_building[index][0] = Some(BuildingCost {
                        generation: self.building_generation,
                        revision,
                        ..entry
                    });
                }
                #[cfg(test)]
                {
                    self.building_cache_counts[1] += 1;
                }
                return entry.cost;
            }
            #[cfg(test)]
            {
                self.building_cache_counts[3] += 1;
            }
        }
        let source = self.building_access(source_idx, allocator, transit_network, graph)?;
        let destination =
            self.building_access(destination_idx, allocator, transit_network, graph)?;
        let eta = source.eta_to(&destination, transit_network, graph, |from, to| {
            self.node_cost(from, to, transit_network, graph)
        });
        if let Some(index) = bucket
            && let Some(prices) =
                prices.or_else(|| frontage_prices(key, allocator, transit_network))
        {
            let geometry = self.geometry(key);
            let entries = &mut self.building_to_building[index];
            if cached.is_none() {
                #[cfg(test)]
                if entries[1].is_some() {
                    self.building_cache_counts[2] += 1;
                }
                entries[1] = entries[0];
            }
            entries[0] = Some(BuildingCost {
                key,
                generation: self.building_generation,
                geometry,
                revision,
                prices,
                cost: eta,
            });
        }
        eta
    }

    /// Prices a prepared border without allocating a sweep or retaining unbounded pairs.
    pub(super) fn between_border_and_building(
        &mut self,
        border_node: u32,
        destination_idx: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        let key = (border_node, destination_idx);
        let revision = transit_network.frontage_delay_revision;
        if let Some(&(generation, sampled_revision, eta)) = self.border_to_building.get(&key)
            && generation == self.building_generation
            && sampled_revision == revision
        {
            return eta;
        }
        // Traffic changes reprice only requested entrances; network sweeps remain reusable.
        let costs = self.border_costs.get(&border_node)?;
        let eta = allocator.freight_car_eta_from_border_costs(
            border_node,
            destination_idx,
            costs,
            transit_network,
            graph,
        );
        if let Some(entry) = self.border_to_building.get_mut(&key) {
            *entry = (self.building_generation, revision, eta);
        } else if self.border_to_building.len() < self.border_to_building.capacity() {
            self.border_to_building
                .insert(key, (self.building_generation, revision, eta));
        }
        eta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Loading sizes the table at once; the first building appended after it must not copy it
    // (`SIM-02-K`).
    #[test]
    fn building_access_rows_keep_spare_capacity_for_the_next_building() {
        let mut cache = FreightRouteCache::default();
        cache.prepare_buildings(1_000);
        let rows = cache.access.as_ptr();
        assert!(cache.access.capacity() > 1_000);
        cache.prepare_buildings(1_001);
        assert_eq!(cache.access.len(), 1_001);
        assert_eq!(cache.access.as_ptr(), rows);
        cache.prepare_buildings(999);
        assert_eq!(cache.access.len(), 999);
    }

    #[test]
    fn colliding_node_quotes_keep_exact_bits_and_replace_old_demand_without_growth() {
        let (mut graph, mut network, edge, _, _) = super::super::tests::simple_graph_with_border();
        let mut cache = FreightRouteCache::default();
        // Every pair collides: exercise replacement rather than a lucky hash distribution.
        cache.node_to_node.resize(1, [None; 2]);
        let capacity = cache.node_to_node.capacity();
        let mut scratch = CchQueryScratch::default();
        let mut unreachable = false;
        for price in [0.125, 17.75, f32::INFINITY, 0.125] {
            graph.edge_mut(edge).base_cost = price;
            network.cch_graph.customize(&graph);
            cache.clear();
            for reverse in [false, true, false] {
                for a in 0..graph.node_count() as u32 {
                    for b in 0..graph.node_count() as u32 {
                        let (a, b) = if reverse { (b, a) } else { (a, b) };
                        let expected = network
                            .cch_graph
                            .find_cost(
                                a,
                                b,
                                usize::MAX,
                                usize::MAX,
                                &graph,
                                TransitFlags::CAR,
                                &mut scratch,
                            )
                            .map(f32::to_bits);
                        unreachable |= a != b && expected.is_none();
                        assert_eq!(
                            cache.node_cost(a, b, &network, &graph).map(f32::to_bits),
                            expected
                        );
                        let hits = cache.node_cache_counts[1];
                        assert_eq!(
                            cache.node_cost(a, b, &network, &graph).map(f32::to_bits),
                            expected
                        );
                        assert_eq!(cache.node_cache_counts[1], hits + 1);
                        assert_eq!(cache.node_to_node.capacity(), capacity);
                    }
                }
            }
        }
        assert!(cache.node_cache_counts[2] > 0);
        assert!(
            unreachable,
            "cached None results must also exercise the immediate-hit check"
        );
    }
}
