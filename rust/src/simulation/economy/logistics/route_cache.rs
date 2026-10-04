// SPDX-License-Identifier: GPL-2.0-only

//! Revision-scoped freight route ETA cache.

use std::collections::BTreeMap;

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::pathing::cch::CchCostsFrom;

/// Caches expensive freight ETA lookups while topology revisions remain unchanged.
#[derive(Clone, Debug, Default)]
pub(super) struct FreightRouteCache {
    building_to_building: BTreeMap<(usize, usize), Option<f32>>,
    border_to_building: BTreeMap<(u32, usize), Option<f32>>,
    // Car costs from each border to every node, swept once per revision: the network leg of
    // every border ETA, instead of one route query per destination building.
    border_costs: BTreeMap<u32, CchCostsFrom>,
}

impl FreightRouteCache {
    pub(super) fn clear(&mut self) {
        self.building_to_building.clear();
        self.border_to_building.clear();
        self.border_costs.clear();
    }

    pub(super) fn between_buildings(
        &mut self,
        source_idx: usize,
        destination_idx: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        *self
            .building_to_building
            .entry((source_idx, destination_idx))
            .or_insert_with(|| {
                allocator.freight_car_eta_between_buildings(
                    source_idx,
                    destination_idx,
                    transit_network,
                    graph,
                )
            })
    }

    pub(super) fn from_border(
        &mut self,
        border_node: u32,
        destination_idx: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Option<f32> {
        let border_costs = &mut self.border_costs;
        *self
            .border_to_building
            .entry((border_node, destination_idx))
            .or_insert_with(|| {
                // One O(hierarchy arcs × degree) sweep per border, then O(1) per destination.
                let costs = border_costs.entry(border_node).or_insert_with(|| {
                    let mut costs = transit_network.cch_graph.costs_from_each(
                        &[border_node],
                        graph,
                        TransitFlags::CAR,
                    );
                    costs.pop().expect("one start yields one cost table")
                });
                allocator.freight_car_eta_from_border_costs(
                    border_node,
                    destination_idx,
                    costs,
                    transit_network,
                    graph,
                )
            })
    }
}
