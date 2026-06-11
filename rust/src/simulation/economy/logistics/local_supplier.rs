//! Local supplier search and shipment creation.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::accessibility::{
    ModeComponentIndex, ReachableBucketScanEvent, lower_bound_travel_seconds,
};
use crate::simulation::economy::definitions::{
    FreightTimingProfile, ResourceRuntimeId, RuntimeEconomyCatalog,
};
use crate::simulation::economy::fiscal::tax_amount;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;

use super::data::{CarrierClass, Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem};
use super::quantization::quantize_requested_amount;
use super::reservations::ReservationViews;
use super::route_cache::FreightRouteCache;
use super::supplier_index::SupplierCandidateIndex;
use super::timing::{adjusted_travel_seconds, adjusted_unit_price, eta_hours_from_travel_seconds};

#[derive(Clone, Copy)]
struct LocalSupplierChoice {
    supplier_idx: usize,
    amount: f32,
    total_cost: f32,
    tax_cost: f32,
    travel_seconds: f32,
}

impl ShipmentSystem {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn try_local_supplier_for_resource(
        &mut self,
        dest_idx: usize,
        desired_amount: f32,
        allow_emergency: bool,
        min_shipment_units: f32,
        resource_runtime_id: ResourceRuntimeId,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        reservations: &mut ReservationViews,
        supplier_index: &SupplierCandidateIndex,
        freight_components: &ModeComponentIndex,
        route_cache: &mut FreightRouteCache,
        freight_profile: &FreightTimingProfile,
        minute_of_day: u16,
        catalog: &RuntimeEconomyCatalog,
        truck_load_units: f32,
        max_freight_speed: f32,
        business_purchase_tax_rate: f32,
    ) -> bool {
        if dest_idx >= allocator.entrances.len() {
            return false;
        }
        let destination = &allocator.buildings[dest_idx];
        let destination_budget = destination.operating_budget;
        let Some(buckets) = supplier_index.buckets_for_resource(resource_runtime_id) else {
            return false;
        };
        let destination_components =
            freight_components.building_components(allocator, graph, dest_idx, TransitFlags::CAR);

        let mut best_choice = None::<LocalSupplierChoice>;
        buckets.scan_nearest(
            destination_components,
            destination.center_x,
            destination.center_y,
            |event| match event {
                ReachableBucketScanEvent::Item {
                    item_idx: candidate_idx,
                } => {
                    update_best_local_supplier_choice(
                        &mut best_choice,
                        candidate_idx,
                        dest_idx,
                        desired_amount,
                        allow_emergency,
                        min_shipment_units,
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
                        truck_load_units,
                        business_purchase_tax_rate,
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

        if let Some(choice) = best_choice {
            allocator.buildings[dest_idx].operating_budget -= choice.total_cost + choice.tax_cost;
            self.shipments.push(Shipment {
                resource_runtime_id,
                amount: choice.amount,
                source: ShipmentEndpoint::Building(choice.supplier_idx),
                destination: ShipmentEndpoint::Building(dest_idx),
                carrier_class: CarrierClass::Truck,
                status: ShipmentStatus::InTransit,
                total_cost: choice.total_cost,
                tax_cost: choice.tax_cost,
                eta_hours: eta_hours_from_travel_seconds(adjusted_travel_seconds(
                    choice.travel_seconds,
                    freight_profile,
                    minute_of_day,
                )),
                queued_hours: 0,
            });
            reservations.record_local_shipment(
                choice.supplier_idx,
                dest_idx,
                resource_runtime_id,
                choice.amount,
            );
            return true;
        }

        false
    }
}

#[allow(clippy::too_many_arguments)]
fn update_best_local_supplier_choice(
    best_choice: &mut Option<LocalSupplierChoice>,
    candidate_idx: usize,
    dest_idx: usize,
    desired_amount: f32,
    allow_emergency: bool,
    min_shipment_units: f32,
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
    truck_load_units: f32,
    business_purchase_tax_rate: f32,
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
    let available = (supplier.inventory_units(output_port.resource_runtime_id) - reserved).max(0.0);
    if available <= 0.0 {
        return;
    }

    let effective_unit_price = adjusted_unit_price(
        supplier_profile.unit_price_currency,
        freight_profile,
        minute_of_day,
    );
    let taxed_unit_price =
        effective_unit_price + tax_amount(effective_unit_price, business_purchase_tax_rate);
    let max_affordable = destination_budget / taxed_unit_price.max(f32::EPSILON);
    let Some(amount) = quantize_requested_amount(
        desired_amount,
        available,
        max_affordable,
        min_shipment_units,
        allow_emergency,
        truck_load_units,
    ) else {
        return;
    };
    let Some(travel_seconds) =
        route_cache.between_buildings(candidate_idx, dest_idx, allocator, transit_network, graph)
    else {
        return;
    };

    let total_cost = amount * effective_unit_price;
    let choice = LocalSupplierChoice {
        supplier_idx: candidate_idx,
        amount,
        total_cost,
        tax_cost: tax_amount(total_cost, business_purchase_tax_rate),
        travel_seconds,
    };
    if best_choice.is_none_or(|best| local_supplier_choice_precedes(choice, best)) {
        *best_choice = Some(choice);
    }
}

fn local_supplier_choice_precedes(left: LocalSupplierChoice, right: LocalSupplierChoice) -> bool {
    left.travel_seconds
        .total_cmp(&right.travel_seconds)
        .then_with(|| {
            (left.total_cost + left.tax_cost).total_cmp(&(right.total_cost + right.tax_cost))
        })
        .then_with(|| left.supplier_idx.cmp(&right.supplier_idx))
        .is_lt()
}
