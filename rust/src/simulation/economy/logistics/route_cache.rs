// SPDX-License-Identifier: GPL-2.0-only

//! Revision-scoped network costs; live frontage delay is evaluated at quotation time.

use std::collections::BTreeMap;

use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::pathing::cch::CchCostsFrom;

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
    node_to_node: BTreeMap<(u32, u32), Option<f32>>,
    // Complete quotes remain valid only for the traffic revision at which they were priced.
    building_to_building: BTreeMap<(usize, usize), (u64, Option<f32>)>,
    border_to_building: BTreeMap<(u32, usize), (u64, Option<f32>)>,
    // Car costs from each border to every node, swept once per revision: the network leg of
    // every border ETA, instead of one route query per destination building.
    border_costs: BTreeMap<u32, CchCostsFrom>,
}

impl FreightRouteCache {
    /// Prepares the existing border sweeps for parallel, allocation-free building quotes.
    pub(super) fn prepare_border_costs(
        &mut self,
        borders: &[u32],
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
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

    /// Drops entrance/index-dependent quotes while retaining unchanged network routes.
    pub(super) fn clear_buildings(&mut self) {
        self.building_to_building.clear();
        self.border_to_building.clear();
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
        BuildingAllocator::freight_car_eta_between_entrances(
            allocator.entrances.get(source_idx)?,
            destination,
            network,
            graph,
            |from, to| {
                *self.node_to_node.entry((from, to)).or_insert_with(|| {
                    network
                        .cch_graph
                        .find_path(from, to, usize::MAX, graph, TransitFlags::CAR)
                        .map(|(seconds, _, _)| seconds)
                })
            },
        )
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
        if let Some(&(sampled_revision, eta)) = self.building_to_building.get(&key) {
            if sampled_revision == revision {
                return eta;
            }
        }
        let eta = allocator.freight_car_eta_between_buildings(
            source_idx,
            destination_idx,
            transit_network,
            graph,
            |from, to| {
                *self.node_to_node.entry((from, to)).or_insert_with(|| {
                    transit_network
                        .cch_graph
                        .find_path(from, to, usize::MAX, graph, TransitFlags::CAR)
                        .map(|(seconds, _, _)| seconds)
                })
            },
        );
        self.building_to_building.insert(key, (revision, eta));
        eta
    }

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
        if let Some(&(sampled_revision, eta)) = self.border_to_building.get(&key) {
            if sampled_revision == revision {
                return eta;
            }
        }
        // Traffic changes reprice only requested entrances; network sweeps remain reusable.
        let costs = self
            .border_costs
            .entry(border_node)
            .or_insert_with(|| sweep_border(border_node, transit_network, graph));
        let eta = allocator.freight_car_eta_from_border_costs(
            border_node,
            destination_idx,
            costs,
            transit_network,
            graph,
        );
        self.border_to_building.insert(key, (revision, eta));
        eta
    }
}
