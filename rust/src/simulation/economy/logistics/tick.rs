//! Hourly logistics orchestration.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;

use super::data::ShipmentSystem;

impl ShipmentSystem {
    /// Advances freight deliveries and opens new bounded restock jobs on one operational hour.
    pub fn hourly_tick(
        &mut self,
        allocator: &mut BuildingAllocator,
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
    ) -> f32 {
        self.refresh_freight_route_cache(allocator, transit_network);
        self.decrement_building_cooldowns(allocator);
        self.create_profile_input_shipments(allocator, transit_network, graph, minute_of_day);
        self.create_profile_output_exports(allocator, transit_network, graph, minute_of_day);
        let business_purchase_tax_collected =
            self.progress_shipments(allocator, agents, transit_network, graph);
        self.shipments.retain(|shipment| shipment.status.is_open());
        business_purchase_tax_collected
    }

    pub(super) fn decrement_building_cooldowns(&self, allocator: &mut BuildingAllocator) {
        allocator.buildings.par_iter_mut().for_each(|building| {
            if building.shipment_cooldown_hours > 0 {
                building.shipment_cooldown_hours -= 1;
            }
        });
    }

    fn refresh_freight_route_cache(
        &mut self,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
    ) {
        let building_revision = allocator.building_ref_revision();
        let entrance_revision = allocator.entrance_ref_revision();
        let cch_generation = transit_network.cch_graph.build_generation;
        if self.freight_route_cache_building_revision != building_revision
            || self.freight_route_cache_entrance_revision != entrance_revision
            || self.freight_route_cache_cch_generation != cch_generation
        {
            self.freight_route_cache.clear();
            self.freight_route_cache_building_revision = building_revision;
            self.freight_route_cache_entrance_revision = entrance_revision;
            self.freight_route_cache_cch_generation = cch_generation;
        }
    }
}
