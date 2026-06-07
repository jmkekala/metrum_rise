//! Hourly logistics orchestration.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;

use super::data::{SHIPMENT_IN_TRANSIT, ShipmentSystem};

impl ShipmentSystem {
    /// Advances freight deliveries and opens new bounded restock jobs on one operational hour.
    pub fn hourly_tick(
        &mut self,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
    ) {
        self.progress_shipments(allocator);
        self.decrement_building_cooldowns(allocator);
        self.create_profile_input_shipments(allocator, transit_network, graph, minute_of_day);
        self.create_profile_output_exports(allocator, transit_network, graph, minute_of_day);
        self.shipments
            .retain(|shipment| shipment.status == SHIPMENT_IN_TRANSIT);
    }

    pub(super) fn decrement_building_cooldowns(&self, allocator: &mut BuildingAllocator) {
        allocator.buildings.par_iter_mut().for_each(|building| {
            if building.shipment_cooldown_hours > 0 {
                building.shipment_cooldown_hours -= 1;
            }
        });
    }
}
