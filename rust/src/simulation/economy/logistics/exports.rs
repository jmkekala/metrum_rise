//! Output surplus export planning to `OWA` border terminals.

use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::accessibility::{
    ModeComponentIndex, ReachableBucketScanEvent, lower_bound_travel_seconds,
};
use crate::simulation::economy::definitions::{
    ResourceRuntimeId, RuntimeEconomyCatalog, RuntimeEconomyTuning,
};
use crate::simulation::economy::fiscal::tax_amount;
use crate::simulation::economy::households::scaled_input_inventory_targets_for_building;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

use super::data::{CarrierClass, Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem};
use super::planning::FreightPlanningContext;
use super::quantization::{quantize_export_amount, quantize_requested_amount};
use super::reservations::ReservationViews;
use super::resource::{freight_profile_for_building, required_unit_price};
use super::route_cache::FreightRouteCache;
use super::supplier_index::SupplierCandidateIndex;
use super::timing::{adjusted_travel_seconds, adjusted_unit_price, eta_hours_from_travel_seconds};

#[derive(Clone, Copy)]
struct LocalInputHoldChoice {
    supplier_idx: usize,
    amount: f32,
    total_cost: f32,
    tax_cost: f32,
    travel_seconds: f32,
}

impl ShipmentSystem {
    /// Creates outbound `OWA` export shipments for industrial buildings with surplus output.
    ///
    /// Triggered when a building's unreserved output inventory exceeds one day's production worth
    /// of buffer. The export is priced at `local_unit_price × owa_export_price_multiplier`, which
    /// is always below the local sale price, keeping the `OWA` a safety valve rather than the
    /// primary revenue engine. Only industrial zone buildings may export; commercial buildings do
    /// not export their outputs.
    pub(super) fn create_profile_output_exports(
        &mut self,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
        planning: &mut FreightPlanningContext,
        business_purchase_tax_rate: f32,
    ) {
        let catalog = planning.catalog.clone();
        let tuning = planning.tuning.clone();
        let resource_count = planning.resource_count;
        self.decay_owa_export_saturation(resource_count, &tuning.logistics);
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
            business_purchase_tax_rate,
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
                    || !matches!(building.zone_type, ZoneType::Industrial)
                {
                    return None;
                }
                catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .filter(|profile| !profile.outputs.is_empty())
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
                || !matches!(building.zone_type, ZoneType::Industrial)
            {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if profile.outputs.is_empty() {
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
                let current_inventory =
                    allocator.buildings[src_idx].inventory_units(output_port.resource_runtime_id);
                let unreserved = (current_inventory - reserved).max(0.0);

                // Keep one day of production as a local buffer; export the rest.
                let buffer = output_port.units_per_day;
                let surplus = unreserved - buffer;
                if surplus <= 0.0 {
                    continue;
                }
                let Some(export_amount) = quantize_export_amount(
                    surplus,
                    profile.min_shipment_units,
                    tuning.logistics.truck_load_units,
                ) else {
                    continue;
                };

                let local_price =
                    required_unit_price(&catalog, output_port.resource_runtime_id, &profile.id);
                let saturation_factor = self.owa_export_saturation_factor(
                    output_port.resource_runtime_id,
                    &tuning.logistics,
                );
                let export_unit_price = local_price * export_multiplier * saturation_factor;
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
                let adjusted_eta =
                    adjusted_travel_seconds(best_eta, freight_profile, minute_of_day);

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
                    tax_cost: 0.0,
                    eta_hours: eta_hours_from_travel_seconds(adjusted_eta),
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
                    saturation_factor,
                    eta_hours_from_travel_seconds(adjusted_eta)
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
        business_purchase_tax_rate: f32,
    ) {
        let catalog = planning.catalog.clone();
        let tuning = planning.tuning.clone();
        for (dest_idx, building) in allocator.buildings.iter().enumerate() {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
                || building.edge_idx == usize::MAX
                || !matches!(building.zone_type, ZoneType::Commercial)
            {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            for input_port in &profile.inputs {
                let request_key = Self::request_key(dest_idx, input_port.resource_runtime_id);
                if self.request_is_terminal(request_key)
                    || planning
                        .reservations
                        .has_open_inbound(dest_idx, input_port.resource_runtime_id)
                {
                    continue;
                }
                let (target_units, reorder_units, critical_units) =
                    scaled_input_inventory_targets_for_building(
                        catalog.as_ref(),
                        building,
                        profile,
                        input_port,
                    );
                if target_units <= 0.0 {
                    continue;
                }
                let effective_input_stock = building
                    .inventory_units(input_port.resource_runtime_id)
                    + planning
                        .reservations
                        .reserved_inbound_amount(dest_idx, input_port.resource_runtime_id);
                if reorder_units > 0.0 && effective_input_stock >= reorder_units {
                    continue;
                }
                if reorder_units <= 0.0 && effective_input_stock >= target_units {
                    continue;
                }

                let allow_emergency = effective_input_stock <= critical_units;
                let desired_amount = (target_units - effective_input_stock).max(0.0);
                if desired_amount < profile.min_shipment_units && !allow_emergency {
                    continue;
                }

                let Some(choice) = find_reachable_local_input_hold(
                    dest_idx,
                    desired_amount,
                    allow_emergency,
                    profile.min_shipment_units,
                    input_port.resource_runtime_id,
                    building.operating_budget,
                    allocator,
                    transit_network,
                    graph,
                    &planning.reservations,
                    &planning.supplier_index,
                    &planning.freight_components,
                    &mut planning.route_cache,
                    planning.max_freight_speed,
                    tuning.as_ref(),
                    catalog.as_ref(),
                    minute_of_day,
                    tuning.logistics.truck_load_units,
                    business_purchase_tax_rate,
                ) else {
                    continue;
                };
                planning.reservations.record_local_shipment(
                    choice.supplier_idx,
                    dest_idx,
                    input_port.resource_runtime_id,
                    choice.amount,
                );
            }
        }
    }

    fn decay_owa_export_saturation(
        &mut self,
        resource_count: usize,
        logistics_tuning: &crate::simulation::economy::definitions::LogisticsRuntimeTuning,
    ) {
        self.ensure_owa_export_saturation_len(resource_count);
        let floor_units = saturation_floor_units(logistics_tuning);
        let decay_units = floor_units / logistics_tuning.owa_export_saturation_recovery_hours;
        for units in &mut self.owa_export_saturation_by_resource {
            *units = (*units - decay_units).max(0.0);
        }
    }

    fn ensure_owa_export_saturation_len(&mut self, resource_count: usize) {
        if self.owa_export_saturation_by_resource.len() < resource_count {
            self.owa_export_saturation_by_resource
                .resize(resource_count, 0.0);
        }
    }

    fn owa_export_saturation_factor(
        &self,
        resource_runtime_id: ResourceRuntimeId,
        logistics_tuning: &crate::simulation::economy::definitions::LogisticsRuntimeTuning,
    ) -> f32 {
        let Some(slot) = resource_slot(
            resource_runtime_id,
            self.owa_export_saturation_by_resource.len(),
        ) else {
            return 1.0;
        };
        let saturated_units = self
            .owa_export_saturation_by_resource
            .get(slot)
            .copied()
            .unwrap_or(0.0);
        let ratio = (saturated_units / saturation_floor_units(logistics_tuning)).clamp(0.0, 1.0);
        1.0 - ratio * (1.0 - logistics_tuning.owa_export_saturation_floor_factor)
    }

    /// Records market saturation for an `OWA` export that physically reached the border.
    pub(super) fn record_owa_export_saturation(
        &mut self,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
        resource_count: usize,
    ) {
        self.ensure_owa_export_saturation_len(resource_count);
        if let Some(slot) = resource_slot(resource_runtime_id, resource_count) {
            self.owa_export_saturation_by_resource[slot] += amount.max(0.0);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn find_reachable_local_input_hold(
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
    supplier_index: &SupplierCandidateIndex,
    freight_components: &ModeComponentIndex,
    route_cache: &mut FreightRouteCache,
    max_freight_speed: f32,
    tuning: &RuntimeEconomyTuning,
    catalog: &RuntimeEconomyCatalog,
    minute_of_day: u16,
    truck_load_units: f32,
    business_purchase_tax_rate: f32,
) -> Option<LocalInputHoldChoice> {
    if dest_idx >= allocator.entrances.len() {
        return None;
    }
    let Some(freight_profile) =
        freight_profile_for_building(catalog, tuning, &allocator.buildings[dest_idx])
    else {
        return None;
    };
    let Some(buckets) = supplier_index.buckets_for_resource(resource_runtime_id) else {
        return None;
    };
    let destination_components =
        freight_components.building_components(allocator, graph, dest_idx, TransitFlags::CAR);
    let destination = &allocator.buildings[dest_idx];

    let mut best_choice = None::<LocalInputHoldChoice>;
    buckets.scan_nearest(
        destination_components,
        destination.center_x,
        destination.center_y,
        |event| match event {
            ReachableBucketScanEvent::Item {
                item_idx: candidate_idx,
            } => {
                update_best_local_input_hold_choice(
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
    best_choice
}

#[allow(clippy::too_many_arguments)]
fn update_best_local_input_hold_choice(
    best_choice: &mut Option<LocalInputHoldChoice>,
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
    freight_profile: &crate::simulation::economy::definitions::FreightTimingProfile,
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
    let max_affordable = destination_budget.max(0.0) / taxed_unit_price.max(f32::EPSILON);
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
    let choice = LocalInputHoldChoice {
        supplier_idx: candidate_idx,
        amount,
        total_cost,
        tax_cost: tax_amount(total_cost, business_purchase_tax_rate),
        travel_seconds,
    };
    if best_choice.is_none_or(|best| local_input_hold_choice_precedes(choice, best)) {
        *best_choice = Some(choice);
    }
}

fn local_input_hold_choice_precedes(
    left: LocalInputHoldChoice,
    right: LocalInputHoldChoice,
) -> bool {
    left.travel_seconds
        .total_cmp(&right.travel_seconds)
        .then_with(|| {
            (left.total_cost + left.tax_cost).total_cmp(&(right.total_cost + right.tax_cost))
        })
        .then_with(|| left.supplier_idx.cmp(&right.supplier_idx))
        .is_lt()
}

fn resource_slot(resource_runtime_id: ResourceRuntimeId, resource_count: usize) -> Option<usize> {
    if resource_runtime_id == 0 || resource_runtime_id as usize > resource_count {
        None
    } else {
        Some(resource_runtime_id as usize - 1)
    }
}

fn saturation_floor_units(
    logistics_tuning: &crate::simulation::economy::definitions::LogisticsRuntimeTuning,
) -> f32 {
    logistics_tuning.truck_load_units.max(0.000_1)
        * logistics_tuning
            .owa_export_saturation_loads_to_floor
            .max(0.000_1)
}
