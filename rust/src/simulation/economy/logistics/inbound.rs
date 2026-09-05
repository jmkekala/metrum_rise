// SPDX-License-Identifier: GPL-2.0-only

//! Input restock request planning for profile-driven buildings.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::households::scaled_input_inventory_targets_for_building;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;

use super::data::ShipmentSystem;
use super::planning::FreightPlanningContext;
use super::resource::{freight_profile_for_building, required_unit_price};

impl ShipmentSystem {
    pub(super) fn create_profile_input_shipments(
        &mut self,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
        planning: &mut FreightPlanningContext,
        treasury_balance: &mut f64,
    ) {
        let catalog = planning.catalog.clone();
        let tuning = planning.tuning.clone();
        let mut eligible_destinations: Vec<usize> = allocator
            .buildings
            .par_iter()
            .enumerate()
            .filter_map(|(idx, building)| {
                if building.broken
                    || building.economy_broken
                    || building.is_deserted
                    || building.is_under_construction()
                    || building.edge_idx == usize::MAX
                    || building.shipment_cooldown_hours > 0
                {
                    return None;
                }
                catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .filter(|profile| !profile.inputs.is_empty())
                    .map(|_| idx)
            })
            .collect();
        eligible_destinations.sort_unstable();

        for dest_idx in eligible_destinations {
            let Some(building) = allocator.buildings.get(dest_idx) else {
                continue;
            };
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
                || building.edge_idx == usize::MAX
                || building.shipment_cooldown_hours > 0
            {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if profile.inputs.is_empty() {
                continue;
            }
            let Some(freight_profile) =
                freight_profile_for_building(&catalog, &tuning, &allocator.buildings[dest_idx])
            else {
                continue;
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

                let (target_units, reorder_units, critical_units) =
                    scaled_input_inventory_targets_for_building(
                        &catalog,
                        &allocator.buildings[dest_idx],
                        profile,
                        input_port,
                    );
                if target_units <= 0.0 {
                    continue;
                }
                let effective_input_stock = allocator.buildings[dest_idx]
                    .inventory_units(input_port.resource_runtime_id)
                    + planning
                        .reservations
                        .reserved_inbound_amount(dest_idx, input_port.resource_runtime_id);
                if reorder_units > 0.0 && effective_input_stock >= reorder_units {
                    self.clear_request_failure(request_key);
                    continue;
                }
                if reorder_units <= 0.0 && effective_input_stock >= target_units {
                    self.clear_request_failure(request_key);
                    continue;
                }
                let allow_emergency = effective_input_stock <= critical_units;
                let desired_amount = (target_units - effective_input_stock).max(0.0);
                if desired_amount <= 0.0 {
                    continue;
                }
                if desired_amount < profile.min_shipment_units && !allow_emergency {
                    continue;
                }

                if self.try_local_supplier_for_resource(
                    dest_idx,
                    desired_amount,
                    allow_emergency,
                    profile.min_shipment_units,
                    input_port.resource_runtime_id,
                    allocator,
                    transit_network,
                    graph,
                    &mut planning.reservations,
                    &planning.supplier_index,
                    &planning.freight_components,
                    &mut planning.route_cache,
                    freight_profile,
                    minute_of_day,
                    &catalog,
                    tuning.logistics.truck_load_units,
                    planning.max_freight_speed,
                    treasury_balance,
                ) {
                    self.clear_request_failure(request_key);
                    continue;
                }

                let import_unit_price =
                    required_unit_price(&catalog, input_port.resource_runtime_id, &profile.id)
                        * tuning.owa_import_price_multiplier;
                if self.try_owa_fallback_for_resource(
                    dest_idx,
                    desired_amount,
                    allow_emergency,
                    profile.min_shipment_units,
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
                    treasury_balance,
                ) {
                    self.clear_request_failure(request_key);
                    continue;
                }

                let became_terminal = self.record_request_failure(
                    request_key,
                    tuning.logistics.terminal_failure_attempts,
                );
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
                allocator.buildings[dest_idx].shipment_cooldown_hours =
                    tuning.operational_clock.shipment_retry_cooldown_hours;
            }
        }
    }
}
