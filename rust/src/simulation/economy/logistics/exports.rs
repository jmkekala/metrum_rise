// SPDX-License-Identifier: GPL-2.0-only

//! Output surplus export planning to `OWA` border terminals: the check a producer makes at its
//! hourly phase and the plan the freight request queue serves (`SIM-02-J`).

use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeEconomyTuning,
    RuntimeResourcePort,
};
use crate::simulation::economy::households::{
    building_operation_factors, saleable_output_stock, scaled_output_units_per_day_for_building,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

use super::data::{CarrierClass, Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem};
use super::planning::FreightPlanningContext;
use super::quantization::{export_dispatch_target_kg, quantize_export_amount};
use super::requests::{freight_ready, planned_cooldown_until};
use super::reservations::{ReservationViews, current_views};
use super::resource::{
    building_outputs_can_export_to_owa, freight_profile_for_building, required_unit_price,
};
use super::timing::{eta_hours_from_travel_seconds, freight_cost};

impl ShipmentSystem {
    /// Whether building `src_idx` has an output to export: an industrial or field/extractor
    /// producer whose unreserved saleable stock above its one-day buffer quantizes to at least
    /// its dispatch target. Routes and the sale's margin over the carrier fee are left to the
    /// plan. O(output ports).
    pub(super) fn export_request_needed(
        &mut self,
        src_idx: usize,
        slot: u64,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> bool {
        let building = &allocator.buildings[src_idx];
        if !freight_ready(building, slot) {
            return false;
        }
        let Some(profile) = catalog
            .profile_by_runtime_id(building.economy_profile_runtime_id)
            .filter(|profile| building_outputs_can_export_to_owa(building, profile))
        else {
            return false;
        };
        if freight_profile_for_building(catalog, tuning, building).is_none() {
            return false;
        }
        let views = current_views(
            &mut self.reservations,
            &self.shipments,
            catalog.resource_count(),
        );
        profile.outputs.iter().any(|output_port| {
            export_amount(
                catalog,
                tuning,
                building,
                profile,
                output_port,
                views,
                src_idx,
            )
            .is_some()
        })
    }

    /// Plans building `src_idx`'s export for its request from phase slot `phase_slot` on the
    /// live state: the first output whose surplus reaches its dispatch target and whose gross
    /// sale beats the carrier quote ships to the fastest such border, and the building waits
    /// until that phase comes round again. Outside buyers have a fixed bid without a purchase
    /// quota; revenue requires delivery. Commercial buildings never export.
    ///
    /// Local buyers keep the one-day buffer: only stock above it ships. Each buyer requests at
    /// its own hourly phase, so between two of a producer's export decisions every local buyer
    /// has had its request (`SIM-02-J`; the hourly pass held stock for buyers it had not
    /// served, which with the one-hour retry cooldown never happened).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan_export_for(
        &mut self,
        src_idx: usize,
        phase_slot: u64,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
        planning: &mut FreightPlanningContext,
    ) {
        if planning.border_nodes.is_empty() {
            return;
        }
        let catalog = planning.catalog.clone();
        let tuning = planning.tuning.clone();
        let export_multiplier = tuning.owa_export_price_multiplier;
        let building = &allocator.buildings[src_idx];
        if !freight_ready(building, phase_slot) {
            return;
        }
        let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
        else {
            return;
        };
        if !building_outputs_can_export_to_owa(building, profile) {
            return;
        }
        let Some(freight_profile) = freight_profile_for_building(&catalog, &tuning, building)
        else {
            return;
        };

        for output_port in &profile.outputs {
            let Some(export_amount) = export_amount(
                &catalog,
                &tuning,
                &allocator.buildings[src_idx],
                profile,
                output_port,
                &planning.reservations,
                src_idx,
            ) else {
                continue;
            };

            let local_price =
                required_unit_price(&catalog, output_port.resource_runtime_id, &profile.id);
            let export_unit_price = local_price * export_multiplier;
            let total_revenue = export_amount * export_unit_price;

            // `OWA` accepts every load; only the roads limit trade.
            let mut best: Option<(u32, f32)> = None;
            for &border_node in &planning.border_nodes {
                // Reuse the import ETA helper: travel time border↔building is symmetric.
                let Some(travel_seconds) = planning.route_cache.between_border_and_building(
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
                if best.is_none_or(|(_, best_eta)| travel_seconds < best_eta) {
                    best = Some((border_node, travel_seconds));
                }
            }
            let Some((best_border, best_eta)) = best else {
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
                status: ShipmentStatus::InTransit,
                carrier: crate::simulation::economy::agents::AgentHandle::NONE,
                total_cost: total_revenue,
                freight_cost: freight_cost(
                    best_eta,
                    tuning.operational_clock.seconds_per_day,
                    &tuning.logistics,
                ),
                eta_hours,
                leg_started_hour: 0,
                trip_event_slot: 0,
                cargo_loaded: false,
            });
            planning.reservations.record_owa_export(
                src_idx,
                output_port.resource_runtime_id,
                export_amount,
            );

            allocator.buildings[src_idx].shipment_cooldown_until_slot = planned_cooldown_until(
                phase_slot,
                tuning.operational_clock.shipment_retry_cooldown_hours,
            );

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

    /// Every building's export plan at once, in dense order, as one hourly pass planned them.
    #[cfg(test)]
    pub(super) fn create_profile_output_exports(
        &mut self,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
        planning: &mut FreightPlanningContext,
    ) {
        let slot = self.hour_start_slot_for_test();
        for src_idx in 0..allocator.buildings.len() {
            self.plan_export_for(
                src_idx,
                slot,
                allocator,
                transit_network,
                graph,
                minute_of_day,
                planning,
            );
        }
    }
}

// The amount `output_port` of building `src_idx` would export: its unreserved saleable stock
// above one current day of production, quantized to whole batches within one payload, when it
// reaches the dispatch target. Hourly review is not hourly dispatch: a production day is
// consolidated into one load where payload permits, retaining the separate local buffer.
fn export_amount(
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    building: &Building,
    profile: &EconomyProfileRuntime,
    output_port: &RuntimeResourcePort,
    reservations: &ReservationViews,
    src_idx: usize,
) -> Option<f32> {
    let resource = output_port.resource_runtime_id;
    let reserved = reservations.reserved_outbound_amount(src_idx, resource);
    let unreserved =
        (saleable_output_stock(catalog, building, profile, resource) - reserved).max(0.0);
    let buffer = owa_export_buffer_units_for_building(catalog, building, profile, output_port);
    let surplus = unreserved - buffer;
    if surplus <= 0.0 {
        return None;
    }
    let export_amount = quantize_export_amount(
        surplus,
        output_port.shipment_batch_kg,
        tuning.logistics.truck_payload_kg,
    )?;
    let daily_net_output = if output_port.units_per_day > 0.0 {
        buffer * profile.net_output_units_per_day(output_port) / output_port.units_per_day
    } else {
        0.0
    };
    (export_amount
        >= export_dispatch_target_kg(
            daily_net_output,
            output_port.shipment_batch_kg,
            tuning.logistics.truck_payload_kg,
        ))
    .then_some(export_amount)
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
