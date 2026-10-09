// SPDX-License-Identifier: GPL-2.0-only

//! Revision-scoped network costs; live frontage delay is evaluated at quotation time.

use std::collections::{BTreeMap, HashMap};

use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance, FreightEntranceAccess};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::pathing::cch::{CchCostsFrom, CchQueryScratch};

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

/// Caches expensive network searches without freezing changing frontage delays.
#[derive(Clone, Debug, Default)]
pub(super) struct FreightRouteCache {
    node_to_node: HashMap<(u32, u32), Option<f32>, foldhash::fast::FixedState>,
    query: CchQueryScratch,
    // Complete quotes remain valid only for the traffic revision at which they were priced.
    building_to_building: HashMap<(usize, usize), (u64, u64, Option<f32>), foldhash::fast::FixedState>,
    border_to_building: HashMap<(u32, usize), (u64, u64, Option<f32>), foldhash::fast::FixedState>,
    building_generation: u64,
    access: Vec<Option<(u64, FreightEntranceAccess)>>,
    // Car costs from each border to every node, swept once per revision: the network leg of
    // every border ETA, instead of one route query per destination building.
    border_costs: BTreeMap<u32, CchCostsFrom>,
}

impl FreightRouteCache {
    /// Reserves indexed geometry slots without visiting any unchanged building.
    /// Queries fill requested slots lazily; preparation is O(new building slots), warm O(1).
    pub(super) fn prepare_buildings(&mut self, buildings: usize) {
        self.access.resize(buildings, None);
    }

    fn building_access(&mut self, index: usize, allocator: &BuildingAllocator,
        network: &TransitNetwork, graph: &RegionGraph) -> Option<FreightEntranceAccess> {
        if index >= allocator.buildings.len() { return None; }
        if let Some(Some((generation, access))) = self.access.get(index)
            && *generation == self.building_generation { return Some(*access); }
        let access = FreightEntranceAccess::new(allocator.entrances.get(index)?, network, graph);
        // Before the next batch reserve, newly appended doors use the same exact fallback
        // without allocating. A retained slot is filled at most once per entrance revision.
        if let Some(slot) = self.access.get_mut(index) {
            *slot = Some((self.building_generation, access));
        }
        Some(access)
    }

    /// Retained entry counts and capacities for bounded quotation storage regressions.
    #[cfg(test)]
    pub(super) fn storage(&self) -> [(usize, usize); 3] {
        [(self.node_to_node.len(), self.node_to_node.capacity()),
            (self.building_to_building.len(), self.building_to_building.capacity()),
            (self.border_to_building.len(), self.border_to_building.capacity())]
    }

    /// Prepares the existing border sweeps for parallel, allocation-free building quotes.
    pub(super) fn prepare_border_costs(
        &mut self,
        borders: &[u32],
        buildings: usize,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.query.prepare(&transit_network.cch_graph, graph);
        // Bound retained pair costs by road graph storage rather than all possible N²
        // pairs. A full cache still computes the exact query; it only skips retention.
        let entries = 2 * graph.edge_count() + graph.node_count();
        if self.node_to_node.capacity() < entries {
            self.node_to_node.reserve(entries.saturating_sub(self.node_to_node.len()));
        }
        let complete = entries + buildings;
        if self.building_to_building.capacity() < complete {
            self.building_to_building.reserve(complete.saturating_sub(self.building_to_building.len()));
        }
        if self.border_to_building.capacity() < complete {
            self.border_to_building.reserve(complete.saturating_sub(self.border_to_building.len()));
        }
        for &border in borders {
            self.border_costs
                .entry(border)
                .or_insert_with(|| sweep_border(border, transit_network, graph));
        }
    }

    /// Reads prepared cost tables without mutating the per-building dispatch cache.
    pub(super) fn minimum_border_eta(
        &self,
        borders: &[u32],
        destination: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        borders
            .iter()
            .filter_map(|&border| {
                allocator.freight_car_eta_from_border_costs(
                    border,
                    destination,
                    self.border_costs.get(&border)?,
                    transit_network,
                    graph,
                )
            })
            .min_by(f32::total_cmp)
    }

    pub(super) fn clear(&mut self) {
        self.node_to_node.clear();
        self.clear_buildings();
        self.border_costs.clear();
    }

    /// Invalidates entrance/index-dependent quotes in O(1), retaining bounded storage and
    /// unchanged network routes. Generation wrap clears stale entries before reusing zero.
    pub(super) fn clear_buildings(&mut self) {
        self.building_generation = self.building_generation.wrapping_add(1);
        if self.building_generation == 0 {
            self.access.fill(None);
            self.building_to_building.clear();
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
    pub(super) fn to_site(
        &mut self,
        source_idx: usize,
        destination: &BuildingEntrance,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        self.building_access(source_idx, allocator, network, graph)?.eta_to(
            &FreightEntranceAccess::new(destination, network, graph), network, graph,
            |from, to| self.node_cost(from, to, network, graph),
        )
    }

    /// Quotes a proposed source to an existing buyer using the buyer's prepared geometry.
    pub(super) fn from_site(&mut self, source: &BuildingEntrance, destination_idx: usize,
        allocator: &BuildingAllocator, network: &TransitNetwork, graph: &RegionGraph) -> Option<f32> {
        FreightEntranceAccess::new(source, network, graph).eta_to(
            &self.building_access(destination_idx, allocator, network, graph)?, network, graph,
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
        BuildingAllocator::freight_car_eta_between_entrances(
            source,
            destination,
            network,
            graph,
            |from, to| self.node_cost(from, to, network, graph),
        )
    }

    fn node_cost(&mut self, from: u32, to: u32, network: &TransitNetwork, graph: &RegionGraph) -> Option<f32> {
        if let Some(&cost) = self.node_to_node.get(&(from, to)) { return cost; }
        let cost = network.cch_graph.find_cost(from, to, usize::MAX, graph, TransitFlags::CAR, &mut self.query);
        if self.node_to_node.len() < self.node_to_node.capacity() {
            self.node_to_node.insert((from, to), cost);
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
        let key = (source_idx, destination_idx);
        let revision = transit_network.frontage_delay_revision;
        if let Some(&(generation, sampled_revision, eta)) = self.building_to_building.get(&key) {
            if generation == self.building_generation && sampled_revision == revision {
                return eta;
            }
        }
        let source = self.building_access(source_idx, allocator, transit_network, graph)?;
        let destination = self.building_access(destination_idx, allocator, transit_network, graph)?;
        let eta = source.eta_to(&destination, transit_network, graph,
            |from, to| self.node_cost(from, to, transit_network, graph));
        if let Some(entry) = self.building_to_building.get_mut(&key) {
            *entry = (self.building_generation, revision, eta);
        } else if self.building_to_building.len() < self.building_to_building.capacity() {
            self.building_to_building.insert(key, (self.building_generation, revision, eta));
        }
        eta
    }

    /// Prices a prepared border without allocating a sweep or retaining unbounded pairs.
    pub(super) fn from_border(
        &mut self,
        border_node: u32,
        destination_idx: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        let key = (border_node, destination_idx);
        let revision = transit_network.frontage_delay_revision;
        if let Some(&(generation, sampled_revision, eta)) = self.border_to_building.get(&key) {
            if generation == self.building_generation && sampled_revision == revision {
                return eta;
            }
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
            self.border_to_building.insert(key, (self.building_generation, revision, eta));
        }
        eta
    }
}
