//! Shipment arrival, refund, and failure handling.

use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};

use super::data::{
    SHIPMENT_DEST_OWA, SHIPMENT_FAILED, SHIPMENT_FULFILLED, SHIPMENT_IN_TRANSIT,
    SHIPMENT_SOURCE_LOCAL, SHIPMENT_SOURCE_OWA, ShipmentSystem,
};
use super::resource::building_accepts_input_resource;

impl ShipmentSystem {
    pub(super) fn progress_shipments(&mut self, allocator: &mut BuildingAllocator) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let retry_cooldown_hours = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"))
            .operational_clock
            .shipment_retry_cooldown_hours;
        for shipment in &mut self.shipments {
            if shipment.status != SHIPMENT_IN_TRANSIT {
                continue;
            }

            if shipment.eta_hours > 0 {
                shipment.eta_hours -= 1;
            }
            if shipment.eta_hours > 0 {
                continue;
            }

            // OWA export: goods travel from source building to border terminal; no local
            // destination building receives them. Credit revenue to the source on arrival.
            if shipment.destination_building_id == SHIPMENT_DEST_OWA {
                let src_idx = shipment.source_building_id;
                if src_idx < allocator.buildings.len()
                    && !allocator.buildings[src_idx].broken
                    && !allocator.buildings[src_idx].economy_broken
                    && !allocator.buildings[src_idx].is_deserted
                    && allocator.buildings[src_idx].inventory_units(shipment.resource_runtime_id)
                        >= shipment.amount
                {
                    allocator.buildings[src_idx]
                        .remove_inventory_units(shipment.resource_runtime_id, shipment.amount);
                    allocator.buildings[src_idx].revenue += shipment.total_cost;
                    allocator.buildings[src_idx].operating_budget += shipment.total_cost;
                    shipment.status = SHIPMENT_FULFILLED;

                    debug_log!(
                        "economy",
                        "OWA export fulfilled index={} resource={} amount={:.1} revenue={:.1}",
                        src_idx,
                        catalog
                            .resource_id_for_runtime_id(shipment.resource_runtime_id)
                            .unwrap_or("unknown"),
                        shipment.amount,
                        shipment.total_cost
                    );
                } else {
                    shipment.status = SHIPMENT_FAILED;
                }
                continue;
            }

            let dest_idx = shipment.destination_building_id;
            if dest_idx >= allocator.buildings.len() {
                shipment.status = SHIPMENT_FAILED;
                continue;
            }

            match shipment.source_kind {
                SHIPMENT_SOURCE_LOCAL => {
                    let src_idx = shipment.source_building_id;
                    if src_idx >= allocator.buildings.len()
                        || allocator.buildings[src_idx].broken
                        || allocator.buildings[src_idx].economy_broken
                        || allocator.buildings[src_idx].is_deserted
                        || allocator.buildings[dest_idx].broken
                        || allocator.buildings[dest_idx].economy_broken
                        || allocator.buildings[dest_idx].is_deserted
                        || allocator.buildings[src_idx]
                            .inventory_units(shipment.resource_runtime_id)
                            < shipment.amount
                        || !building_accepts_input_resource(
                            &catalog,
                            &allocator.buildings[dest_idx],
                            shipment.resource_runtime_id,
                        )
                    {
                        allocator.buildings[dest_idx].operating_budget += shipment.total_cost;
                        allocator.buildings[dest_idx].shipment_cooldown_hours =
                            retry_cooldown_hours;
                        shipment.status = SHIPMENT_FAILED;
                        continue;
                    }

                    allocator.buildings[src_idx]
                        .remove_inventory_units(shipment.resource_runtime_id, shipment.amount);
                    allocator.buildings[src_idx].revenue += shipment.total_cost;
                    allocator.buildings[src_idx].operating_budget += shipment.total_cost;
                    allocator.buildings[dest_idx]
                        .add_inventory_units(shipment.resource_runtime_id, shipment.amount);
                    allocator.buildings[dest_idx].daily_local_input_value += shipment.total_cost;
                    shipment.status = SHIPMENT_FULFILLED;
                }
                SHIPMENT_SOURCE_OWA => {
                    if allocator.buildings[dest_idx].broken
                        || allocator.buildings[dest_idx].economy_broken
                        || allocator.buildings[dest_idx].is_deserted
                        || !building_accepts_input_resource(
                            &catalog,
                            &allocator.buildings[dest_idx],
                            shipment.resource_runtime_id,
                        )
                    {
                        allocator.buildings[dest_idx].operating_budget += shipment.total_cost;
                        allocator.buildings[dest_idx].shipment_cooldown_hours =
                            retry_cooldown_hours;
                        shipment.status = SHIPMENT_FAILED;
                        continue;
                    }
                    allocator.buildings[dest_idx]
                        .add_inventory_units(shipment.resource_runtime_id, shipment.amount);
                    allocator.buildings[dest_idx].daily_owa_input_value += shipment.total_cost;
                    shipment.status = SHIPMENT_FULFILLED;
                }
                _ => {
                    allocator.buildings[dest_idx].operating_budget += shipment.total_cost;
                    allocator.buildings[dest_idx].shipment_cooldown_hours = retry_cooldown_hours;
                    shipment.status = SHIPMENT_FAILED;
                }
            }
        }
    }
}
