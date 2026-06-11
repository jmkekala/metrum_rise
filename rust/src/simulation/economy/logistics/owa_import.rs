//! `OWA` border fallback import shipment creation.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    FreightTimingProfile, LogisticsRuntimeTuning, ResourceRuntimeId,
};
use crate::simulation::economy::fiscal::tax_amount;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

use super::data::{CarrierClass, Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem};
use super::quantization::quantize_requested_amount;
use super::reservations::ReservationViews;
use super::route_cache::FreightRouteCache;
use super::timing::{adjusted_travel_seconds, adjusted_unit_price, eta_hours_from_travel_seconds};

impl ShipmentSystem {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn try_owa_fallback_for_resource(
        &mut self,
        dest_idx: usize,
        desired_amount: f32,
        allow_emergency: bool,
        min_shipment_units: f32,
        unit_price: f32,
        resource_runtime_id: ResourceRuntimeId,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        border_nodes: &[u32],
        reservations: &mut ReservationViews,
        route_cache: &mut FreightRouteCache,
        freight_profile: &FreightTimingProfile,
        minute_of_day: u16,
        logistics_tuning: &LogisticsRuntimeTuning,
        business_purchase_tax_rate: f32,
    ) -> bool {
        if border_nodes.is_empty() {
            return false;
        }

        // Use the actual charge price (including any outside-window premium) for the
        // affordability check so the building cannot be charged more than it can afford.
        let effective_unit_price = adjusted_unit_price(unit_price, freight_profile, minute_of_day);
        let taxed_unit_price =
            effective_unit_price + tax_amount(effective_unit_price, business_purchase_tax_rate);
        let max_affordable_amount =
            allocator.buildings[dest_idx].operating_budget / taxed_unit_price.max(f32::EPSILON);
        let Some(amount) = quantize_requested_amount(
            desired_amount,
            f32::MAX,
            max_affordable_amount,
            min_shipment_units,
            allow_emergency,
            logistics_tuning.truck_load_units,
        ) else {
            return false;
        };
        let total_cost = amount * effective_unit_price;
        let tax_cost = tax_amount(total_cost, business_purchase_tax_rate);

        let active_cap = usize::from(logistics_tuning.border_active_jobs_per_node);
        let queued_cap = usize::from(logistics_tuning.border_queued_jobs_per_node);
        let mut best_active: Option<(u32, f32)> = None;
        let mut best_queued: Option<(u32, f32)> = None;
        for &border_node in border_nodes {
            let Some(travel_seconds) =
                route_cache.from_border(border_node, dest_idx, allocator, transit_network, graph)
            else {
                continue;
            };
            if reservations.border_active_job_count(border_node) < active_cap {
                if best_active.is_none_or(|(_, best_cost)| travel_seconds < best_cost) {
                    best_active = Some((border_node, travel_seconds));
                }
            } else if reservations.border_queued_job_count(border_node) < queued_cap
                && best_queued.is_none_or(|(_, best_cost)| travel_seconds < best_cost)
            {
                best_queued = Some((border_node, travel_seconds));
            }
        }

        let (best_border, best_cost, status) =
            if let Some((border_node, travel_seconds)) = best_active {
                (border_node, travel_seconds, ShipmentStatus::InTransit)
            } else if let Some((border_node, travel_seconds)) = best_queued {
                (border_node, travel_seconds, ShipmentStatus::Queued)
            } else {
                return false;
            };

        allocator.buildings[dest_idx].operating_budget -= total_cost + tax_cost;
        let shipment_id = self.allocate_shipment_id();
        self.shipments.push(Shipment {
            id: shipment_id,
            resource_runtime_id,
            amount,
            source: ShipmentEndpoint::OwaBorder(best_border),
            destination: ShipmentEndpoint::Building(dest_idx),
            carrier_class: CarrierClass::Truck,
            status,
            carrier_agent_id: usize::MAX,
            total_cost,
            tax_cost,
            eta_hours: eta_hours_from_travel_seconds(adjusted_travel_seconds(
                best_cost,
                freight_profile,
                minute_of_day,
            )),
            queued_hours: 0,
        });
        reservations.record_owa_import(dest_idx, best_border, resource_runtime_id, amount, status);
        true
    }
}
