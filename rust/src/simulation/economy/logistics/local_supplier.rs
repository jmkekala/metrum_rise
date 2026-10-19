// SPDX-License-Identifier: GPL-2.0-only

//! Local supplier search and shipment creation.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::handles::Handle;
use crate::simulation::economy::accessibility::{
    ModeComponentIndex, ReachableBucketScanEvent, lower_bound_travel_seconds,
};
use crate::simulation::economy::definitions::{
    FreightTimingProfile, LogisticsRuntimeTuning, ResourceRuntimeId, RuntimeEconomyCatalog,
};
use crate::simulation::economy::households::saleable_output_stock;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;

use super::data::{CarrierClass, Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem};
use super::quantization::quantize_requested_amount;
use super::reservations::ReservationViews;
use super::resource::{input_purchase_budget, reserve_input_payment};
use super::route_cache::FreightRouteCache;
use super::supplier_index::SupplierCandidateIndex;
use super::timing::{adjusted_unit_price, eta_hours_from_travel_seconds, freight_cost};

/// A local freight offer shared by input orders and export protection.
#[derive(Clone, Copy)]
pub(super) struct LocalSupplierChoice {
    /// Supplier selected by freight travel time, cost, then stable building index.
    pub(super) supplier_idx: usize,
    /// Affordable, unreserved stock rounded to an allowed shipment quantity.
    pub(super) amount: f32,
    /// Buyer payment including the freight-window goods adjustment and the carrier quote.
    pub(super) total_cost: f32,
    freight_cost: f32,
    travel_seconds: f32,
}

impl ShipmentSystem {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn try_local_supplier_for_resource(
        &mut self,
        dest_idx: usize,
        desired_amount: f32,
        allow_emergency: bool,
        shipment_batch_kg: f32,
        resource_runtime_id: ResourceRuntimeId,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        border_nodes: &[u32],
        import_unit_price: f32,
        reservations: &mut ReservationViews,
        supplier_index: &SupplierCandidateIndex,
        freight_components: &ModeComponentIndex,
        route_cache: &mut FreightRouteCache,
        freight_profile: &FreightTimingProfile,
        minute_of_day: u16,
        catalog: &RuntimeEconomyCatalog,
        logistics_tuning: &LogisticsRuntimeTuning,
        max_freight_speed: f32,
        seconds_per_day: f64,
        treasury_balance: &mut f64,
    ) -> bool {
        let best_choice = find_local_supplier(
            dest_idx,
            desired_amount,
            allow_emergency,
            shipment_batch_kg,
            resource_runtime_id,
            input_purchase_budget(allocator, dest_idx),
            allocator,
            transit_network,
            graph,
            reservations,
            supplier_index,
            freight_components,
            route_cache,
            freight_profile,
            minute_of_day,
            catalog,
            logistics_tuning,
            max_freight_speed,
            seconds_per_day,
        );

        if let Some(choice) = best_choice {
            // Funding and dispatch both retain the outside option when its delivered
            // price is lower.
            let imported_price =
                adjusted_unit_price(import_unit_price, freight_profile, minute_of_day);
            let cheaper_import = border_nodes.iter().any(|&border| {
                route_cache
                    .between_border_and_building(
                        border,
                        dest_idx,
                        allocator,
                        transit_network,
                        graph,
                    )
                    .is_some_and(|seconds| {
                        choice.amount * imported_price
                            + freight_cost(seconds, seconds_per_day, logistics_tuning)
                            < choice.total_cost
                    })
            });
            if cheaper_import {
                return false;
            }
            reserve_input_payment(allocator, treasury_balance, dest_idx, choice.total_cost);
            let shipment_id = self.allocate_shipment_id();
            let eta_hours = eta_hours_from_travel_seconds(
                choice.travel_seconds,
                freight_profile,
                minute_of_day,
                seconds_per_day,
            );
            self.shipments.push(Shipment {
                id: shipment_id,
                resource_runtime_id,
                amount: choice.amount,
                source: ShipmentEndpoint::Building(choice.supplier_idx),
                destination: ShipmentEndpoint::Building(dest_idx),
                carrier_class: CarrierClass::Truck,
                status: ShipmentStatus::InTransit,
                carrier: crate::simulation::economy::agents::AgentHandle::NONE,
                total_cost: choice.total_cost,
                freight_cost: choice.freight_cost,
                eta_hours,
                leg_started_hour: 0,
                trip_event_slot: 0,
                cargo_loaded: false,
            });
            reservations.record_local_shipment(
                choice.supplier_idx,
                dest_idx,
                resource_runtime_id,
                choice.amount,
            );
            crate::debug_log!(
                "economy",
                "freight input initiated shipment_id={} source=local src_idx={} src_asset={} dest_idx={} dest_asset={} resource={} amount={:.1} cost={:.1} eta={}h",
                shipment_id,
                choice.supplier_idx,
                allocator.buildings[choice.supplier_idx].asset_id,
                dest_idx,
                allocator.buildings[dest_idx].asset_id,
                catalog
                    .resource_id_for_runtime_id(resource_runtime_id)
                    .unwrap_or("unknown"),
                choice.amount,
                choice.total_cost,
                eta_hours
            );
            return true;
        }

        false
    }
}

/// Finds an affordable reachable supplier through the existing component/spatial index.
/// Both orders and export holds use this query and the same deterministic tie-breaking. The
/// index names suppliers by building handle; one that has left since it was indexed is skipped.
#[allow(clippy::too_many_arguments)]
pub(super) fn find_local_supplier(
    dest_idx: usize,
    desired_amount: f32,
    allow_emergency: bool,
    shipment_batch_kg: f32,
    resource_runtime_id: ResourceRuntimeId,
    destination_budget: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    reservations: &ReservationViews,
    supplier_index: &SupplierCandidateIndex,
    freight_components: &ModeComponentIndex,
    route_cache: &mut FreightRouteCache,
    freight_profile: &FreightTimingProfile,
    minute_of_day: u16,
    catalog: &RuntimeEconomyCatalog,
    logistics_tuning: &LogisticsRuntimeTuning,
    max_freight_speed: f32,
    seconds_per_day: f64,
) -> Option<LocalSupplierChoice> {
    if dest_idx >= allocator.entrances.len() {
        return None;
    }
    let destination = &allocator.buildings[dest_idx];
    let buckets = supplier_index.buckets_for_resource(resource_runtime_id)?;
    let destination_components =
        freight_components.building_components(allocator, graph, dest_idx, TransitFlags::CAR);

    let mut best_choice = None::<LocalSupplierChoice>;
    buckets.scan_nearest(
        destination_components,
        destination.center_x,
        destination.center_y,
        |event| match event {
            ReachableBucketScanEvent::Item { item_idx } => {
                let Some(candidate_idx) = allocator
                    .building_handles
                    .resolve(Handle::from_bits(item_idx as u64))
                else {
                    return true;
                };
                update_best_local_supplier_choice(
                    &mut best_choice,
                    candidate_idx,
                    dest_idx,
                    desired_amount,
                    allow_emergency,
                    shipment_batch_kg,
                    resource_runtime_id,
                    destination_budget,
                    allocator,
                    transit_network,
                    graph,
                    reservations,
                    route_cache,
                    freight_profile,
                    minute_of_day,
                    catalog,
                    logistics_tuning,
                    seconds_per_day,
                );
                true
            }
            ReachableBucketScanEvent::RingComplete {
                next_min_distance_sq,
            } => {
                let Some(choice) = best_choice else {
                    return true;
                };
                lower_bound_travel_seconds(next_min_distance_sq, max_freight_speed)
                    <= choice.travel_seconds
            }
        },
    );

    best_choice
}

#[allow(clippy::too_many_arguments)]
fn update_best_local_supplier_choice(
    best_choice: &mut Option<LocalSupplierChoice>,
    candidate_idx: usize,
    dest_idx: usize,
    desired_amount: f32,
    allow_emergency: bool,
    shipment_batch_kg: f32,
    resource_runtime_id: ResourceRuntimeId,
    destination_budget: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    reservations: &ReservationViews,
    route_cache: &mut FreightRouteCache,
    freight_profile: &FreightTimingProfile,
    minute_of_day: u16,
    catalog: &RuntimeEconomyCatalog,
    logistics_tuning: &LogisticsRuntimeTuning,
    seconds_per_day: f64,
) {
    if candidate_idx == dest_idx || candidate_idx >= allocator.buildings.len() {
        return;
    }
    let supplier = &allocator.buildings[candidate_idx];
    if supplier.broken
        || supplier.economy_broken
        || supplier.is_deserted
        || supplier.is_under_construction()
    {
        return;
    }
    let Some(supplier_profile) = catalog.profile_by_runtime_id(supplier.economy_profile_runtime_id)
    else {
        return;
    };
    let Some(output_port) = supplier_profile.output_port(resource_runtime_id) else {
        return;
    };
    let reserved = reservations.reserved_outbound_amount(candidate_idx, resource_runtime_id);
    let available = (saleable_output_stock(
        catalog,
        supplier,
        supplier_profile,
        output_port.resource_runtime_id,
    ) - reserved)
        .max(0.0);
    if available <= 0.0 {
        return;
    }

    let effective_unit_price = adjusted_unit_price(
        supplier_profile.unit_price_currency,
        freight_profile,
        minute_of_day,
    );
    let max_affordable = (destination_budget - logistics_tuning.dispatch_fee_currency).max(0.0)
        / effective_unit_price.max(f32::EPSILON);
    let Some(_) = quantize_requested_amount(
        desired_amount,
        available,
        max_affordable,
        shipment_batch_kg,
        allow_emergency,
        logistics_tuning.truck_payload_kg,
    ) else {
        return;
    };
    let Some(travel_seconds) =
        route_cache.between_buildings(candidate_idx, dest_idx, allocator, transit_network, graph)
    else {
        return;
    };

    let freight_cost = freight_cost(travel_seconds, seconds_per_day, logistics_tuning);
    let Some(amount) = quantize_requested_amount(
        desired_amount,
        available,
        (destination_budget - freight_cost).max(0.0) / effective_unit_price.max(f32::EPSILON),
        shipment_batch_kg,
        allow_emergency,
        logistics_tuning.truck_payload_kg,
    ) else {
        return;
    };
    let total_cost = amount * effective_unit_price + freight_cost;
    let choice = LocalSupplierChoice {
        supplier_idx: candidate_idx,
        amount,
        total_cost,
        freight_cost,
        travel_seconds,
    };
    if best_choice.is_none_or(|best| local_supplier_choice_precedes(choice, best)) {
        *best_choice = Some(choice);
    }
}

fn local_supplier_choice_precedes(left: LocalSupplierChoice, right: LocalSupplierChoice) -> bool {
    left.travel_seconds
        .total_cmp(&right.travel_seconds)
        .then_with(|| left.total_cost.total_cmp(&right.total_cost))
        .then_with(|| left.supplier_idx.cmp(&right.supplier_idx))
        .is_lt()
}
