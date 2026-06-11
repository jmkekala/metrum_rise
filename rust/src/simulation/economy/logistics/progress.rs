//! Shipment arrival, refund, and failure handling.

use std::collections::HashMap;

use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};

use super::data::{Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem};
use super::resource::building_accepts_input_resource;

impl ShipmentSystem {
    pub(super) fn progress_shipments(&mut self, allocator: &mut BuildingAllocator) -> f32 {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let retry_cooldown_hours = tuning.operational_clock.shipment_retry_cooldown_hours;
        dispatch_queued_shipments(
            &mut self.shipments,
            allocator,
            usize::from(tuning.logistics.border_active_jobs_per_node),
            tuning.logistics.queued_shipment_expiry_hours,
            retry_cooldown_hours,
        );

        let mut business_purchase_tax_collected = 0.0;
        for shipment in &mut self.shipments {
            if shipment.status != ShipmentStatus::InTransit {
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
            if let ShipmentEndpoint::OwaBorder(_) = shipment.destination {
                let ShipmentEndpoint::Building(src_idx) = shipment.source else {
                    shipment.status = ShipmentStatus::Failed;
                    continue;
                };
                if src_idx < allocator.buildings.len()
                    && !allocator.buildings[src_idx].broken
                    && !allocator.buildings[src_idx].economy_broken
                    && !allocator.buildings[src_idx].is_deserted
                    && !allocator.buildings[src_idx].is_under_construction()
                    && allocator.buildings[src_idx].inventory_units(shipment.resource_runtime_id)
                        >= shipment.amount
                {
                    allocator.buildings[src_idx]
                        .remove_inventory_units(shipment.resource_runtime_id, shipment.amount);
                    allocator.buildings[src_idx].revenue += shipment.total_cost;
                    allocator.buildings[src_idx].operating_budget += shipment.total_cost;
                    shipment.status = ShipmentStatus::Fulfilled;

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
                    shipment.status = ShipmentStatus::Failed;
                }
                continue;
            }

            let ShipmentEndpoint::Building(dest_idx) = shipment.destination else {
                shipment.status = ShipmentStatus::Failed;
                continue;
            };
            if dest_idx >= allocator.buildings.len() {
                shipment.status = ShipmentStatus::Failed;
                continue;
            }

            match shipment.source {
                ShipmentEndpoint::Building(src_idx) => {
                    if src_idx >= allocator.buildings.len()
                        || allocator.buildings[src_idx].broken
                        || allocator.buildings[src_idx].economy_broken
                        || allocator.buildings[src_idx].is_deserted
                        || allocator.buildings[src_idx].is_under_construction()
                        || allocator.buildings[dest_idx].broken
                        || allocator.buildings[dest_idx].economy_broken
                        || allocator.buildings[dest_idx].is_deserted
                        || allocator.buildings[dest_idx].is_under_construction()
                        || allocator.buildings[src_idx]
                            .inventory_units(shipment.resource_runtime_id)
                            < shipment.amount
                        || !building_accepts_input_resource(
                            &catalog,
                            &allocator.buildings[dest_idx],
                            shipment.resource_runtime_id,
                        )
                    {
                        allocator.buildings[dest_idx].operating_budget +=
                            shipment.total_cost + shipment.tax_cost;
                        allocator.buildings[dest_idx].shipment_cooldown_hours =
                            retry_cooldown_hours;
                        shipment.status = ShipmentStatus::Failed;
                        continue;
                    }

                    allocator.buildings[src_idx]
                        .remove_inventory_units(shipment.resource_runtime_id, shipment.amount);
                    allocator.buildings[src_idx].revenue += shipment.total_cost;
                    allocator.buildings[src_idx].operating_budget += shipment.total_cost;
                    allocator.buildings[dest_idx]
                        .add_inventory_units(shipment.resource_runtime_id, shipment.amount);
                    allocator.buildings[dest_idx].daily_local_input_value += shipment.total_cost;
                    business_purchase_tax_collected += shipment.tax_cost;
                    shipment.status = ShipmentStatus::Fulfilled;
                }
                ShipmentEndpoint::OwaBorder(_) => {
                    if allocator.buildings[dest_idx].broken
                        || allocator.buildings[dest_idx].economy_broken
                        || allocator.buildings[dest_idx].is_deserted
                        || allocator.buildings[dest_idx].is_under_construction()
                        || !building_accepts_input_resource(
                            &catalog,
                            &allocator.buildings[dest_idx],
                            shipment.resource_runtime_id,
                        )
                    {
                        allocator.buildings[dest_idx].operating_budget +=
                            shipment.total_cost + shipment.tax_cost;
                        allocator.buildings[dest_idx].shipment_cooldown_hours =
                            retry_cooldown_hours;
                        shipment.status = ShipmentStatus::Failed;
                        continue;
                    }
                    allocator.buildings[dest_idx]
                        .add_inventory_units(shipment.resource_runtime_id, shipment.amount);
                    allocator.buildings[dest_idx].daily_owa_input_value += shipment.total_cost;
                    business_purchase_tax_collected += shipment.tax_cost;
                    shipment.status = ShipmentStatus::Fulfilled;
                }
            }
        }
        business_purchase_tax_collected
    }
}

fn dispatch_queued_shipments(
    shipments: &mut [Shipment],
    allocator: &mut BuildingAllocator,
    active_cap: usize,
    expiry_hours: u16,
    retry_cooldown_hours: u16,
) {
    let mut active_counts = HashMap::new();
    for shipment in shipments.iter() {
        if shipment.status != ShipmentStatus::InTransit {
            continue;
        }
        if let Some(border_node) = shipment
            .source
            .border_node()
            .or_else(|| shipment.destination.border_node())
        {
            *active_counts.entry(border_node).or_insert(0usize) += 1;
        }
    }

    for shipment in shipments.iter_mut() {
        if shipment.status != ShipmentStatus::Queued {
            continue;
        }
        shipment.queued_hours = shipment.queued_hours.saturating_add(1);
        if shipment.queued_hours >= expiry_hours {
            expire_queued_shipment(shipment, allocator, retry_cooldown_hours);
            continue;
        }
        let Some(border_node) = shipment
            .source
            .border_node()
            .or_else(|| shipment.destination.border_node())
        else {
            shipment.status = ShipmentStatus::Failed;
            continue;
        };
        let active_count = active_counts.entry(border_node).or_insert(0usize);
        if *active_count < active_cap {
            *active_count += 1;
            shipment.status = ShipmentStatus::InTransit;
        }
    }
}

fn expire_queued_shipment(
    shipment: &mut Shipment,
    allocator: &mut BuildingAllocator,
    retry_cooldown_hours: u16,
) {
    if let ShipmentEndpoint::Building(destination_idx) = shipment.destination
        && destination_idx < allocator.buildings.len()
    {
        allocator.buildings[destination_idx].operating_budget +=
            shipment.total_cost + shipment.tax_cost;
        allocator.buildings[destination_idx].shipment_cooldown_hours = retry_cooldown_hours;
    }
    shipment.status = ShipmentStatus::Expired;
}
