//! Output surplus export planning to `OWA` border terminals.

use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

use super::data::{
    BORDER_ACTIVE_JOBS_PER_NODE, CARRIER_TRUCK, SHIPMENT_DEST_OWA, SHIPMENT_IN_TRANSIT,
    SHIPMENT_SOURCE_LOCAL, Shipment, ShipmentSystem,
};
use super::resource::{freight_profile_for_building, required_unit_price};
use super::routing::connected_border_nodes;
use super::timing::{adjusted_travel_seconds, eta_hours_from_travel_seconds};

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
    ) {
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let resource_count = catalog.resource_count();
        let border_nodes = connected_border_nodes(graph);
        if border_nodes.is_empty() {
            return;
        }
        let export_multiplier = tuning.owa_export_price_multiplier;
        let mut reservations = self.build_reservation_views(resource_count);
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
                let reserved =
                    reservations.reserved_outbound_amount(src_idx, output_port.resource_runtime_id);
                let current_inventory =
                    allocator.buildings[src_idx].inventory_units(output_port.resource_runtime_id);
                let unreserved = (current_inventory - reserved).max(0.0);

                // Keep one day of production as a local buffer; export the rest.
                let buffer = output_port.units_per_day;
                if unreserved <= buffer {
                    continue;
                }
                let export_amount = unreserved - buffer;

                // Only export in meaningful batches.
                if export_amount < profile.min_shipment_units {
                    continue;
                }

                let local_price =
                    required_unit_price(&catalog, output_port.resource_runtime_id, &profile.id);
                let export_unit_price = local_price * export_multiplier;
                let total_revenue = export_amount * export_unit_price;

                // Find the nearest border node with capacity.
                let mut best_border = u32::MAX;
                let mut best_eta = f32::MAX;
                for &border_node in &border_nodes {
                    if reservations.border_job_count(border_node) >= BORDER_ACTIVE_JOBS_PER_NODE {
                        continue;
                    }
                    // Reuse the import ETA helper: travel time border↔building is symmetric.
                    let Some(travel_seconds) = allocator.freight_car_eta_from_border_node(
                        border_node,
                        src_idx,
                        transit_network,
                        graph,
                    ) else {
                        continue;
                    };
                    if travel_seconds < best_eta {
                        best_eta = travel_seconds;
                        best_border = border_node;
                    }
                }
                if best_border == u32::MAX {
                    continue;
                }
                let adjusted_eta =
                    adjusted_travel_seconds(best_eta, freight_profile, minute_of_day);

                self.shipments.push(Shipment {
                    resource_runtime_id: output_port.resource_runtime_id,
                    amount: export_amount,
                    source_kind: SHIPMENT_SOURCE_LOCAL,
                    source_building_id: src_idx,
                    // Repurposed as destination border node for OWA exports.
                    source_border_node: best_border,
                    destination_building_id: SHIPMENT_DEST_OWA,
                    carrier_class: CARRIER_TRUCK,
                    status: SHIPMENT_IN_TRANSIT,
                    total_cost: total_revenue,
                    eta_hours: eta_hours_from_travel_seconds(adjusted_eta),
                });
                reservations.record_owa_export(
                    src_idx,
                    best_border,
                    output_port.resource_runtime_id,
                    export_amount,
                );

                allocator.buildings[src_idx].shipment_cooldown_hours =
                    tuning.operational_clock.shipment_retry_cooldown_hours;

                debug_log!(
                    "economy",
                    "OWA export initiated index={} resource={} amount={:.1} revenue={:.1} eta={}h",
                    src_idx,
                    catalog
                        .resource_id_for_runtime_id(output_port.resource_runtime_id)
                        .unwrap_or("unknown"),
                    export_amount,
                    total_revenue,
                    eta_hours_from_travel_seconds(adjusted_eta)
                );
                break;
            }
        }
    }
}
