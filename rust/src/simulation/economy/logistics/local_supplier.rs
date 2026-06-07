//! Local supplier search and shipment creation.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    FreightTimingProfile, ResourceRuntimeId, RuntimeEconomyCatalog,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoneType;

use super::data::{
    CARRIER_TRUCK, SHIPMENT_IN_TRANSIT, SHIPMENT_SOURCE_LOCAL, Shipment, ShipmentSystem,
};
use super::reservations::ReservationViews;
use super::timing::{adjusted_travel_seconds, adjusted_unit_price, eta_hours_from_travel_seconds};

const SUPPLIER_SEARCH_MAX_RING: i32 = 3;
pub(super) const SUPPLIER_SEARCH_CANDIDATES: usize = 24;

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
        candidates: &mut Vec<usize>,
        freight_profile: &FreightTimingProfile,
        minute_of_day: u16,
        catalog: &RuntimeEconomyCatalog,
    ) -> bool {
        if dest_idx >= allocator.entrances.len() {
            return false;
        }
        let destination = &allocator.buildings[dest_idx];
        let destination_budget = destination.operating_budget;
        allocator.fill_nearby_buildings(
            destination.center_x,
            destination.center_y,
            SUPPLIER_SEARCH_MAX_RING,
            SUPPLIER_SEARCH_CANDIDATES,
            candidates,
            |candidate_idx, supplier| {
                if candidate_idx == dest_idx
                    || supplier.broken
                    || supplier.economy_broken
                    || supplier.is_deserted
                    || !matches!(
                        supplier.zone_type,
                        ZoneType::Industrial | ZoneType::Commercial
                    )
                {
                    return false;
                }
                let Some(supplier_profile) =
                    catalog.profile_by_runtime_id(supplier.economy_profile_runtime_id)
                else {
                    return false;
                };
                let Some(output_port) = supplier_profile.output_port(resource_runtime_id) else {
                    return false;
                };
                let reserved =
                    reservations.reserved_outbound_amount(candidate_idx, resource_runtime_id);
                let available =
                    (supplier.inventory_units(output_port.resource_runtime_id) - reserved).max(0.0);
                if available <= 0.0 {
                    return false;
                }

                let effective_unit_price = adjusted_unit_price(
                    supplier_profile.unit_price_currency,
                    freight_profile,
                    minute_of_day,
                );
                let max_affordable = destination_budget / effective_unit_price;
                available.min(desired_amount).min(max_affordable) > 0.0
            },
        );

        for &candidate_idx in candidates.iter() {
            if candidate_idx == dest_idx || candidate_idx >= allocator.buildings.len() {
                continue;
            }
            let supplier = &allocator.buildings[candidate_idx];
            if supplier.broken || supplier.economy_broken || supplier.is_deserted {
                continue;
            }
            let Some(supplier_profile) =
                catalog.profile_by_runtime_id(supplier.economy_profile_runtime_id)
            else {
                continue;
            };
            let Some(output_port) = supplier_profile.output_port(resource_runtime_id) else {
                continue;
            };
            let reserved =
                reservations.reserved_outbound_amount(candidate_idx, resource_runtime_id);
            let available =
                (supplier.inventory_units(output_port.resource_runtime_id) - reserved).max(0.0);
            if available <= 0.0 {
                continue;
            }

            let effective_unit_price = adjusted_unit_price(
                supplier_profile.unit_price_currency,
                freight_profile,
                minute_of_day,
            );
            let max_affordable =
                allocator.buildings[dest_idx].operating_budget / effective_unit_price;
            let amount = available.min(desired_amount).min(max_affordable);
            if amount < min_shipment_units && !allow_emergency {
                continue;
            }
            if amount <= 0.0 {
                continue;
            }
            let total_cost = amount * effective_unit_price;

            let Some(travel_seconds) = allocator.freight_car_eta_between_buildings(
                candidate_idx,
                dest_idx,
                transit_network,
                graph,
            ) else {
                continue;
            };

            allocator.buildings[dest_idx].operating_budget -= total_cost;
            self.shipments.push(Shipment {
                resource_runtime_id,
                amount,
                source_kind: SHIPMENT_SOURCE_LOCAL,
                source_building_id: candidate_idx,
                source_border_node: u32::MAX,
                destination_building_id: dest_idx,
                carrier_class: CARRIER_TRUCK,
                status: SHIPMENT_IN_TRANSIT,
                total_cost,
                eta_hours: eta_hours_from_travel_seconds(adjusted_travel_seconds(
                    travel_seconds,
                    freight_profile,
                    minute_of_day,
                )),
            });
            reservations.record_local_shipment(
                candidate_idx,
                dest_idx,
                resource_runtime_id,
                amount,
            );
            return true;
        }

        false
    }
}
