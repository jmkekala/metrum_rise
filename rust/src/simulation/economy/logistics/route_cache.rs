//! Revision-scoped freight route ETA cache.

use std::collections::BTreeMap;

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

/// Caches expensive freight ETA lookups while topology revisions remain unchanged.
#[derive(Clone, Debug, Default)]
pub(super) struct FreightRouteCache {
    building_to_building: BTreeMap<(usize, usize), Option<f32>>,
    border_to_building: BTreeMap<(u32, usize), Option<f32>>,
}

impl FreightRouteCache {
    pub(super) fn clear(&mut self) {
        self.building_to_building.clear();
        self.border_to_building.clear();
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
        *self
            .border_to_building
            .entry((border_node, destination_idx))
            .or_insert_with(|| {
                allocator.freight_car_eta_from_border_node(
                    border_node,
                    destination_idx,
                    transit_network,
                    graph,
                )
            })
    }
}
