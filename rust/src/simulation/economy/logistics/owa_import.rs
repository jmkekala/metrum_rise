//! `OWA` border fallback import shipment creation.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{FreightTimingProfile, ResourceRuntimeId};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

use super::data::{
    BORDER_ACTIVE_JOBS_PER_NODE, CARRIER_TRUCK, SHIPMENT_IN_TRANSIT, SHIPMENT_SOURCE_OWA, Shipment,
    ShipmentSystem,
};
use super::reservations::ReservationViews;
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
        freight_profile: &FreightTimingProfile,
        minute_of_day: u16,
    ) -> bool {
        if border_nodes.is_empty() {
            return false;
        }

        let min_amount = if desired_amount < min_shipment_units && allow_emergency {
            desired_amount
        } else {
            min_shipment_units
        };
        // Use the actual charge price (including any outside-window premium) for the
        // affordability check so the building cannot be charged more than it can afford.
        let effective_unit_price = adjusted_unit_price(unit_price, freight_profile, minute_of_day);
        let max_affordable_amount =
            allocator.buildings[dest_idx].operating_budget / effective_unit_price;
        if max_affordable_amount < min_amount {
            return false;
        }
        let amount = desired_amount.max(min_amount).min(max_affordable_amount);
        let total_cost = amount * effective_unit_price;

        let mut best_border = u32::MAX;
        let mut best_cost = f32::MAX;
        for &border_node in border_nodes {
            if reservations.border_job_count(border_node) >= BORDER_ACTIVE_JOBS_PER_NODE {
                continue;
            }
            let Some(travel_seconds) = allocator.freight_car_eta_from_border_node(
                border_node,
                dest_idx,
                transit_network,
                graph,
            ) else {
                continue;
            };
            if travel_seconds < best_cost {
                best_cost = travel_seconds;
                best_border = border_node;
            }
        }

        if best_border == u32::MAX {
            return false;
        }

        allocator.buildings[dest_idx].operating_budget -= total_cost;
        self.shipments.push(Shipment {
            resource_runtime_id,
            amount,
            source_kind: SHIPMENT_SOURCE_OWA,
            source_building_id: usize::MAX,
            source_border_node: best_border,
            destination_building_id: dest_idx,
            carrier_class: CARRIER_TRUCK,
            status: SHIPMENT_IN_TRANSIT,
            total_cost,
            eta_hours: eta_hours_from_travel_seconds(adjusted_travel_seconds(
                best_cost,
                freight_profile,
                minute_of_day,
            )),
        });
        reservations.record_owa_import(dest_idx, best_border, resource_runtime_id, amount);
        true
    }
}
