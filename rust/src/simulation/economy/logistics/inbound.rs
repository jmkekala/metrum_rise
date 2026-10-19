// SPDX-License-Identifier: GPL-2.0-only

//! Input restock planning for profile-driven buildings: the check a building makes at its
//! hourly phase and the plan the freight request queue serves (`SIM-02-J`).

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

use super::data::ShipmentSystem;
use super::planning::FreightPlanningContext;
use super::requests::{freight_ready, planned_cooldown_until};
use super::reservations::current_views;
use super::resource::{freight_profile_for_building, input_restock_request, required_unit_price};

impl ShipmentSystem {
    /// Whether building `dest_idx` has an input port to restock at its phase slot `slot`: no
    /// open inbound shipment and its stock plus the inbound reservation below the reorder point.
    /// When none has, the ports' failure histories clear, as planning them would. O(input ports).
    pub(super) fn inbound_request_needed(
        &mut self,
        dest_idx: usize,
        slot: u64,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> bool {
        let building = &allocator.buildings[dest_idx];
        if !freight_ready(building, slot) {
            return false;
        }
        let Some(profile) = catalog
            .profile_by_runtime_id(building.economy_profile_runtime_id)
            .filter(|profile| !profile.inputs.is_empty())
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
        let needed = profile.inputs.iter().any(|input_port| {
            let resource = input_port.resource_runtime_id;
            !views.has_open_inbound(dest_idx, resource)
                && input_restock_request(
                    catalog,
                    building,
                    profile,
                    input_port,
                    views.reserved_inbound_amount(dest_idx, resource),
                )
                .is_some()
        });
        if !needed && !self.request_failures.is_empty() {
            for input_port in &profile.inputs {
                self.request_failures
                    .remove(&Self::request_key(dest_idx, input_port.resource_runtime_id));
            }
        }
        needed
    }

    /// Plans building `dest_idx`'s input restock for its request from phase slot `phase_slot`
    /// on the live state: per input port without an open inbound shipment that needs stock, the
    /// nearest affordable local supplier, else an outside-world import. A port that finds
    /// neither counts a failure and puts the building on its freight cooldown until that phase
    /// comes round again.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan_inbound_for(
        &mut self,
        dest_idx: usize,
        phase_slot: u64,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
        planning: &mut FreightPlanningContext,
        treasury_balance: &mut f64,
    ) {
        let catalog = planning.catalog.clone();
        let tuning = planning.tuning.clone();
        let building = &allocator.buildings[dest_idx];
        if !freight_ready(building, phase_slot) {
            return;
        }
        let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
        else {
            return;
        };
        if profile.inputs.is_empty() {
            return;
        }
        let Some(freight_profile) = freight_profile_for_building(&catalog, &tuning, building)
        else {
            return;
        };

        let mut failed_any_request = false;
        for input_port in &profile.inputs {
            let request_key = Self::request_key(dest_idx, input_port.resource_runtime_id);
            if planning
                .reservations
                .has_open_inbound(dest_idx, input_port.resource_runtime_id)
            {
                self.clear_request_failure(request_key);
                continue;
            }

            let Some((desired_amount, allow_emergency)) = input_restock_request(
                &catalog,
                &allocator.buildings[dest_idx],
                profile,
                input_port,
                planning
                    .reservations
                    .reserved_inbound_amount(dest_idx, input_port.resource_runtime_id),
            ) else {
                self.clear_request_failure(request_key);
                continue;
            };

            let import_unit_price =
                required_unit_price(&catalog, input_port.resource_runtime_id, &profile.id)
                    * tuning.owa_import_price_multiplier;
            if self.try_local_supplier_for_resource(
                dest_idx,
                desired_amount,
                allow_emergency,
                input_port.shipment_batch_kg,
                input_port.resource_runtime_id,
                allocator,
                transit_network,
                graph,
                &planning.border_nodes,
                import_unit_price,
                &mut planning.reservations,
                &planning.supplier_index,
                &planning.freight_components,
                &mut planning.route_cache,
                freight_profile,
                minute_of_day,
                &catalog,
                &tuning.logistics,
                planning.max_freight_speed,
                tuning.operational_clock.seconds_per_day,
                treasury_balance,
            ) {
                self.clear_request_failure(request_key);
                continue;
            }

            if self.try_owa_fallback_for_resource(
                dest_idx,
                desired_amount,
                allow_emergency,
                input_port.shipment_batch_kg,
                import_unit_price,
                input_port.resource_runtime_id,
                allocator,
                transit_network,
                graph,
                &planning.border_nodes,
                &mut planning.reservations,
                &mut planning.route_cache,
                freight_profile,
                minute_of_day,
                &catalog,
                &tuning.logistics,
                tuning.operational_clock.seconds_per_day,
                treasury_balance,
            ) {
                self.clear_request_failure(request_key);
                continue;
            }

            let became_terminal = self
                .record_request_failure(request_key, tuning.logistics.terminal_failure_attempts);
            if became_terminal {
                crate::debug_log!(
                    "economy",
                    "freight request terminal: destination={} resource={}",
                    request_key.destination_building_id,
                    request_key.resource_runtime_id
                );
            }
            failed_any_request = true;
        }

        if failed_any_request {
            allocator.buildings[dest_idx].shipment_cooldown_until_slot = planned_cooldown_until(
                phase_slot,
                tuning.operational_clock.shipment_retry_cooldown_hours,
            );
        }
    }

    /// Every building's inbound plan at once, in dense order, as one hourly pass planned them,
    /// at the slot that starts the current logistics hour.
    #[cfg(test)]
    pub(super) fn create_profile_input_shipments(
        &mut self,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
        planning: &mut FreightPlanningContext,
        treasury_balance: &mut f64,
    ) {
        let slot = self.hour_start_slot_for_test();
        for dest_idx in 0..allocator.buildings.len() {
            self.plan_inbound_for(
                dest_idx,
                slot,
                allocator,
                transit_network,
                graph,
                minute_of_day,
                planning,
                treasury_balance,
            );
        }
    }
}
