// SPDX-License-Identifier: GPL-2.0-only

//! Output surplus export planning to `OWA` border terminals.

use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeResourcePort,
};
use crate::simulation::economy::households::{
    building_operation_factors, saleable_output_stock, scaled_output_units_per_day_for_building,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;

use super::data::{CarrierClass, Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem};
use super::local_supplier::find_local_supplier;
use super::planning::FreightPlanningContext;
use super::quantization::{export_dispatch_target_kg, quantize_export_amount};
use super::resource::{
    building_outputs_can_export_to_owa, freight_profile_for_building, required_unit_price,
};
use super::timing::{eta_hours_from_travel_seconds, freight_cost};

impl ShipmentSystem {
    /// Creates outbound `OWA` export shipments for industrial buildings with surplus output.
    ///
    /// Triggered when a building's unreserved output inventory exceeds one day's production worth
    /// of buffer. Outside buyers have a fixed bid without a purchase quota. Only shipments
    /// whose gross sale exceeds the carrier quote are dispatched; revenue requires delivery.
    /// Commercial buildings do not export their outputs.
    pub(super) fn create_profile_output_exports(
        &mut self,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
        planning: &mut FreightPlanningContext,
    ) {
        let catalog = planning.catalog.clone();
        let tuning = planning.tuning.clone();
        let border_nodes = planning.border_nodes.clone();
        if border_nodes.is_empty() {
            return;
        }
        let export_multiplier = tuning.owa_export_price_multiplier;
        self.reserve_reachable_local_input_holds(
            allocator,
            transit_network,
            graph,
            planning,
            minute_of_day,
        );
        let mut eligible_sources: Vec<usize> = allocator
            .buildings
            .par_iter()
            .enumerate()
            .filter_map(|(idx, building)| {
                if building.broken
                    || building.economy_broken
                    || building.edge_idx == usize::MAX
                    || building.shipment_cooldown_hours > 0
                    || building.is_deserted
                    || building.is_under_construction()
                {
                    return None;
                }
                catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .filter(|profile| building_outputs_can_export_to_owa(building, profile))
                    .map(|_| idx)
            })
            .collect();
        eligible_sources.sort_unstable();

        for src_idx in eligible_sources {
            let building = &allocator.buildings[src_idx];
            if building.broken
                || building.economy_broken
                || building.edge_idx == usize::MAX
                || building.shipment_cooldown_hours > 0
                || building.is_deserted
                || building.is_under_construction()
            {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if !building_outputs_can_export_to_owa(building, profile) {
                continue;
            }
            let Some(freight_profile) =
                freight_profile_for_building(&catalog, &tuning, &allocator.buildings[src_idx])
            else {
                continue;
            };

            for output_port in &profile.outputs {
                let reserved = planning
                    .reservations
                    .reserved_outbound_amount(src_idx, output_port.resource_runtime_id);
                let current_inventory = saleable_output_stock(
                    &catalog,
                    &allocator.buildings[src_idx],
                    profile,
                    output_port.resource_runtime_id,
                );
                let unreserved = (current_inventory - reserved).max(0.0);

                // Keep one current day of production as a local buffer; export the rest.
                let buffer = owa_export_buffer_units_for_building(
                    &catalog,
                    &allocator.buildings[src_idx],
                    profile,
                    output_port,
                );
                let surplus = unreserved - buffer;
                if surplus <= 0.0 {
                    continue;
                }
                let Some(export_amount) = quantize_export_amount(
                    surplus,
                    output_port.shipment_batch_kg,
                    tuning.logistics.truck_payload_kg,
                ) else {
                    continue;
                };

                // Hourly review is not hourly dispatch: consolidate a production day
                // into one load where payload permits, retaining the separate local buffer.
                let daily_net_output = if output_port.units_per_day > 0.0 {
                    buffer * profile.net_output_units_per_day(output_port)
                        / output_port.units_per_day
                } else {
                    0.0
                };
                if export_amount
                    < export_dispatch_target_kg(
                        daily_net_output,
                        output_port.shipment_batch_kg,
                        tuning.logistics.truck_payload_kg,
                    )
                {
                    continue;
                }

                let local_price =
                    required_unit_price(&catalog, output_port.resource_runtime_id, &profile.id);
                let export_unit_price = local_price * export_multiplier;
                let total_revenue = export_amount * export_unit_price;

                let active_cap = usize::from(tuning.logistics.border_active_jobs_per_node);
                let queued_cap = usize::from(tuning.logistics.border_queued_jobs_per_node);
                let mut best_active: Option<(u32, f32)> = None;
                let mut best_queued: Option<(u32, f32)> = None;
                for &border_node in &border_nodes {
                    // Reuse the import ETA helper: travel time border↔building is symmetric.
                    let Some(travel_seconds) = planning.route_cache.from_border(
                        border_node,
                        src_idx,
                        allocator,
                        transit_network,
                        graph,
                    ) else {
                        continue;
                    };
                    if total_revenue
                        <= freight_cost(
                            travel_seconds,
                            tuning.operational_clock.seconds_per_day,
                            &tuning.logistics,
                        )
                    {
                        continue;
                    }
                    if planning.reservations.border_active_job_count(border_node) < active_cap {
                        if best_active.is_none_or(|(_, best_eta)| travel_seconds < best_eta) {
                            best_active = Some((border_node, travel_seconds));
                        }
                    } else if planning.reservations.border_queued_job_count(border_node)
                        < queued_cap
                        && best_queued.is_none_or(|(_, best_eta)| travel_seconds < best_eta)
                    {
                        best_queued = Some((border_node, travel_seconds));
                    }
                }
                let (best_border, best_eta, status) =
                    if let Some((border_node, travel_seconds)) = best_active {
                        (border_node, travel_seconds, ShipmentStatus::InTransit)
                    } else if let Some((border_node, travel_seconds)) = best_queued {
                        (border_node, travel_seconds, ShipmentStatus::Queued)
                    } else {
                        continue;
                    };
                let eta_hours = eta_hours_from_travel_seconds(
                    best_eta,
                    freight_profile,
                    minute_of_day,
                    tuning.operational_clock.seconds_per_day,
                );

                let shipment_id = self.allocate_shipment_id();
                self.shipments.push(Shipment {
                    id: shipment_id,
                    resource_runtime_id: output_port.resource_runtime_id,
                    amount: export_amount,
                    source: ShipmentEndpoint::Building(src_idx),
                    destination: ShipmentEndpoint::OwaBorder(best_border),
                    carrier_class: CarrierClass::Truck,
                    status,
                    carrier_agent_id: usize::MAX,
                    total_cost: total_revenue,
                    freight_cost: freight_cost(
                        best_eta,
                        tuning.operational_clock.seconds_per_day,
                        &tuning.logistics,
                    ),
                    eta_hours,
                    queued_hours: 0,
                });
                planning.reservations.record_owa_export(
                    src_idx,
                    best_border,
                    output_port.resource_runtime_id,
                    export_amount,
                    status,
                );

                allocator.buildings[src_idx].shipment_cooldown_hours =
                    tuning.operational_clock.shipment_retry_cooldown_hours;

                debug_log!(
                    "economy",
                    "OWA export initiated index={} resource={} amount={:.1} revenue={:.1} price_factor={:.2} eta={}h",
                    src_idx,
                    catalog
                        .resource_id_for_runtime_id(output_port.resource_runtime_id)
                        .unwrap_or("unknown"),
                    export_amount,
                    total_revenue,
                    export_multiplier,
                    eta_hours
                );
                break;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn reserve_reachable_local_input_holds(
        &self,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        planning: &mut FreightPlanningContext,
        minute_of_day: u16,
    ) {
        let catalog = planning.catalog.clone();
        let tuning = planning.tuning.clone();
        for (dest_idx, building) in allocator.buildings.iter().enumerate() {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
                || building.edge_idx == usize::MAX
            {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            let Some(freight_profile) = freight_profile_for_building(&catalog, &tuning, building)
            else {
                continue;
            };
            let mut remaining_budget = super::resource::input_purchase_budget(allocator, dest_idx);
            for input_port in &profile.inputs {
                if planning
                    .reservations
                    .has_open_inbound(dest_idx, input_port.resource_runtime_id)
                {
                    continue;
                }
                let Some((desired_amount, allow_emergency)) =
                    super::resource::input_restock_request(
                        &catalog,
                        building,
                        profile,
                        input_port,
                        planning
                            .reservations
                            .reserved_inbound_amount(dest_idx, input_port.resource_runtime_id),
                    )
                else {
                    continue;
                };

                let Some(choice) = find_local_supplier(
                    dest_idx,
                    desired_amount,
                    allow_emergency,
                    input_port.shipment_batch_kg,
                    input_port.resource_runtime_id,
                    remaining_budget,
                    allocator,
                    transit_network,
                    graph,
                    &planning.reservations,
                    &planning.supplier_index,
                    &planning.freight_components,
                    &mut planning.route_cache,
                    freight_profile,
                    minute_of_day,
                    catalog.as_ref(),
                    &tuning.logistics,
                    planning.max_freight_speed,
                    tuning.operational_clock.seconds_per_day,
                ) else {
                    continue;
                };
                remaining_budget = (remaining_budget - choice.total_cost).max(0.0);
                planning.reservations.record_local_shipment(
                    choice.supplier_idx,
                    dest_idx,
                    input_port.resource_runtime_id,
                    choice.amount,
                );
            }
        }
    }
}

fn owa_export_buffer_units_for_building(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
    output_port: &RuntimeResourcePort,
) -> f32 {
    let full_output_units =
        scaled_output_units_per_day_for_building(building, profile, output_port);
    if !matches!(
        profile.kind,
        EconomyProfileRuntimeKind::FieldProducer | EconomyProfileRuntimeKind::Extractor
    ) {
        return full_output_units;
    }

    let active_output_units = full_output_units
        * building_operation_factors(catalog, building, profile).output_capacity_factor;
    active_output_units.clamp(0.0, full_output_units)
}
