// SPDX-License-Identifier: GPL-2.0-only

//! Shipment arrival, refund, and failure handling.

use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::{AgentHandle, AgentSystem, TRANSIT_IN_BUILDING};
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

use super::data::{Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem};
use super::resource::{building_accepts_input_resource, refund_input_payment};

enum CarrierProgress {
    Arrived,
    Traveling,
    Missing,
}

impl ShipmentSystem {
    /// Shipment `id`'s index in the id-ordered list. O(log S).
    pub(crate) fn shipment_index(&self, id: u64) -> Option<usize> {
        self.shipments.binary_search_by_key(&id, |job| job.id).ok()
    }

    /// Loads every new shipment's cargo at its source and lists it for a paced carrier
    /// departure (`SIM-02-E`); a shipment whose endpoints are no longer ready fails at once.
    /// O(open shipments).
    pub(super) fn load_ready_shipments(
        &mut self,
        allocator: &mut BuildingAllocator,
        treasury_balance: &mut f64,
        retry_cooldown_hours: u16,
        catalog: &RuntimeEconomyCatalog,
    ) {
        for idx in 0..self.shipments.len() {
            let shipment = &self.shipments[idx];
            if shipment.status != ShipmentStatus::InTransit || shipment.cargo_loaded {
                continue;
            }
            if !shipment_endpoints_ready(shipment, allocator, catalog) {
                let cooldown_until_tick = self.hour_ticks + u32::from(retry_cooldown_hours);
                self.update_shipment(idx, |shipment| {
                    fail_shipment_before_dispatch(
                        shipment,
                        allocator,
                        treasury_balance,
                        cooldown_until_tick,
                    )
                });
                continue;
            }
            if let ShipmentEndpoint::Building(source_idx) = shipment.source {
                allocator.buildings[source_idx]
                    .remove_inventory_units(shipment.resource_runtime_id, shipment.amount);
            }
            self.update_shipment(idx, |shipment| shipment.cargo_loaded = true);
            self.held_dispatches.push(self.shipments[idx].id);
        }
    }

    /// Sends loaded shipment `id`'s carrier on its way at its paced slot (`SIM-02-E`). True when
    /// a carrier left; one that cannot route fails the shipment and returns its cargo.
    /// O(log S) plus one route.
    pub(crate) fn dispatch_shipment(
        &mut self,
        id: u64,
        allocator: &mut BuildingAllocator,
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        treasury_balance: &mut f64,
    ) -> bool {
        let Some(idx) = self.shipment_index(id) else {
            return false;
        };
        let shipment = self.shipments[idx].clone();
        if shipment.status != ShipmentStatus::InTransit
            || !shipment.cargo_loaded
            || !shipment.carrier.is_none()
        {
            return false;
        }
        let carrier = match (shipment.source, shipment.destination) {
            (ShipmentEndpoint::Building(source), ShipmentEndpoint::Building(destination)) => agents
                .spawn_city_freight_carrier(
                    shipment.id,
                    source,
                    destination,
                    allocator,
                    transit_network,
                    graph,
                ),
            (ShipmentEndpoint::OwaBorder(border_node), ShipmentEndpoint::Building(destination)) => {
                agents.spawn_import_freight_carrier(
                    shipment.id,
                    border_node,
                    destination,
                    allocator,
                    transit_network,
                    graph,
                )
            }
            (ShipmentEndpoint::Building(source), ShipmentEndpoint::OwaBorder(border_node)) => agents
                .spawn_export_freight_carrier(
                    shipment.id,
                    source,
                    border_node,
                    allocator,
                    transit_network,
                    graph,
                ),
            _ => None,
        };
        let Some(carrier_agent_id) = carrier else {
            let retry_cooldown_hours = load_runtime_economy_tuning()
                .unwrap_or_else(|err| {
                    panic!("could not load built-in economy runtime tuning: {err}")
                })
                .operational_clock
                .shipment_retry_cooldown_hours;
            return_loaded_cargo(&shipment, allocator);
            let cooldown_until_tick = self.hour_ticks + u32::from(retry_cooldown_hours);
            self.update_shipment(idx, |shipment| {
                fail_shipment_before_dispatch(
                    shipment,
                    allocator,
                    treasury_balance,
                    cooldown_until_tick,
                )
            });
            return false;
        };
        self.shipments[idx].carrier = agents.agent_handle(carrier_agent_id);
        self.shipments[idx].queued_hours = 0;
        crate::debug::work::add(crate::debug::work::Work::FreightDispatches, 1);
        true
    }

    /// One check of shipment `id`'s carrier (`SIM-02-E`): a carrier at its destination delivers
    /// and turns back, one back at its source finishes. True while the carrier is under way.
    /// O(log S) plus the delivery.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn check_shipment(
        &mut self,
        id: u64,
        allocator: &mut BuildingAllocator,
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        treasury_balance: &mut f64,
        on_delivery: &mut impl FnMut(&Shipment, &BuildingAllocator),
    ) -> bool {
        let Some(idx) = self.shipment_index(id) else {
            return false;
        };
        let shipment = self.shipments[idx].clone();
        match shipment.status {
            ShipmentStatus::Returning => {
                match shipment_carrier_progress_to_endpoint(&shipment, agents, shipment.source) {
                    CarrierProgress::Traveling => true,
                    CarrierProgress::Arrived | CarrierProgress::Missing => {
                        self.finish_delivered_shipment(idx, &shipment, agents);
                        false
                    }
                }
            }
            ShipmentStatus::InTransit if !shipment.carrier.is_none() => {
                let catalog = load_runtime_economy_catalog().unwrap_or_else(|err| {
                    panic!("could not load built-in runtime economy catalog: {err}")
                });
                let retry_cooldown_hours = load_runtime_economy_tuning()
                    .unwrap_or_else(|err| {
                        panic!("could not load built-in economy runtime tuning: {err}")
                    })
                    .operational_clock
                    .shipment_retry_cooldown_hours;
                match shipment_carrier_progress_to_endpoint(&shipment, agents, shipment.destination)
                {
                    CarrierProgress::Traveling => true,
                    CarrierProgress::Missing => {
                        self.fail_in_transit_shipment(
                            idx,
                            allocator,
                            agents,
                            treasury_balance,
                            retry_cooldown_hours,
                            &catalog,
                            ShipmentStatus::Failed,
                            "carrier_missing",
                        );
                        false
                    }
                    CarrierProgress::Arrived => {
                        self.deliver_shipment(
                            idx,
                            &shipment,
                            allocator,
                            agents,
                            transit_network,
                            graph,
                            treasury_balance,
                            retry_cooldown_hours,
                            &catalog,
                            on_delivery,
                        );
                        self.shipments[idx].status == ShipmentStatus::Returning
                    }
                }
            }
            _ => false,
        }
    }

    /// Hourly: carriers still under way age by one hour and time out; arrived ones are left to
    /// their checks (`SIM-02-E`). O(open shipments).
    pub(super) fn expire_overdue_shipments(
        &mut self,
        allocator: &mut BuildingAllocator,
        agents: &mut AgentSystem,
        treasury_balance: &mut f64,
        retry_cooldown_hours: u16,
        catalog: &RuntimeEconomyCatalog,
    ) {
        for idx in 0..self.shipments.len() {
            let shipment = &self.shipments[idx];
            let endpoint = match shipment.status {
                ShipmentStatus::Returning => shipment.source,
                ShipmentStatus::InTransit => shipment.destination,
                _ => continue,
            };
            if shipment.carrier.is_none()
                || !matches!(
                    shipment_carrier_progress_to_endpoint(shipment, agents, endpoint),
                    CarrierProgress::Traveling
                )
            {
                continue;
            }
            self.shipments[idx].queued_hours = self.shipments[idx].queued_hours.saturating_add(1);
            let shipment = self.shipments[idx].clone();
            if shipment.queued_hours < in_transit_timeout_hours(&shipment) {
                continue;
            }
            if shipment.status == ShipmentStatus::Returning {
                self.finish_delivered_shipment(idx, &shipment, agents);
            } else {
                self.fail_in_transit_shipment(
                    idx,
                    allocator,
                    agents,
                    treasury_balance,
                    retry_cooldown_hours,
                    catalog,
                    ShipmentStatus::Expired,
                    "carrier_timeout",
                );
            }
        }
    }

    /// Test driver of one logistics hour's trips: loads and dispatches every ready shipment at
    /// once, ages the carriers under way, then checks every carrier once.
    #[cfg(test)]
    pub(super) fn progress_shipments(
        &mut self,
        allocator: &mut BuildingAllocator,
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        treasury_balance: &mut f64,
        mut on_delivery: impl FnMut(&Shipment, &BuildingAllocator),
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let retry_cooldown_hours = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"))
            .operational_clock
            .shipment_retry_cooldown_hours;
        self.load_ready_shipments(allocator, treasury_balance, retry_cooldown_hours, &catalog);
        for id in std::mem::take(&mut self.held_dispatches) {
            self.dispatch_shipment(id, allocator, agents, transit_network, graph, treasury_balance);
        }
        self.expire_overdue_shipments(
            allocator,
            agents,
            treasury_balance,
            retry_cooldown_hours,
            &catalog,
        );
        let ids: Vec<u64> = self.shipments.iter().map(|shipment| shipment.id).collect();
        for id in ids {
            self.check_shipment(
                id,
                allocator,
                agents,
                transit_network,
                graph,
                treasury_balance,
                &mut on_delivery,
            );
        }
        self.assert_reservations_current();
    }

    /// Test driver of one logistics hour with its trips taken at once: the hourly pass, then
    /// every held carrier dispatched and every carrier checked, then closed shipments dropped.
    #[cfg(test)]
    pub(crate) fn hourly_tick_with_trips(
        &mut self,
        allocator: &mut BuildingAllocator,
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
        treasury_balance: &mut f64,
    ) {
        self.hourly_tick(
            allocator,
            agents,
            transit_network,
            graph,
            minute_of_day,
            treasury_balance,
        );
        for id in std::mem::take(&mut self.held_dispatches) {
            self.dispatch_shipment(id, allocator, agents, transit_network, graph, treasury_balance);
        }
        let ids: Vec<u64> = self.shipments.iter().map(|shipment| shipment.id).collect();
        for id in ids {
            self.check_shipment(
                id,
                allocator,
                agents,
                transit_network,
                graph,
                treasury_balance,
                &mut |_, _| {},
            );
        }
        self.shipments.retain(|shipment| shipment.status.is_open());
        self.assert_reservations_current();
    }

    // The carrier reached its destination: settle the cargo, then return or finish.
    #[allow(clippy::too_many_arguments)]
    fn deliver_shipment(
        &mut self,
        idx: usize,
        shipment: &Shipment,
        allocator: &mut BuildingAllocator,
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        treasury_balance: &mut f64,
        retry_cooldown_hours: u16,
        catalog: &RuntimeEconomyCatalog,
        on_delivery: &mut impl FnMut(&Shipment, &BuildingAllocator),
    ) {
        // OWA export: goods travel from source building to border terminal; no local
        // destination building receives them. Credit revenue to the source on arrival.
        if let ShipmentEndpoint::OwaBorder(_) = shipment.destination {
            let ShipmentEndpoint::Building(src_idx) = shipment.source else {
                self.fail_in_transit_shipment(
                    idx,
                    allocator,
                    agents,
                    treasury_balance,
                    retry_cooldown_hours,
                    catalog,
                    ShipmentStatus::Failed,
                    "bad_export_source",
                );
                return;
            };
            if src_idx < allocator.buildings.len()
                && !allocator.buildings[src_idx].broken
                && !allocator.buildings[src_idx].economy_broken
                && !allocator.buildings[src_idx].is_deserted
                && !allocator.buildings[src_idx].is_under_construction()
            {
                allocator.buildings[src_idx].revenue += shipment.total_cost;
                allocator.buildings[src_idx].operating_budget +=
                    shipment.total_cost - shipment.freight_cost;
                crate::debug::money::record_change(
                    crate::debug::money::MoneyFlow::OutsideSales,
                    shipment.freight_cost,
                    shipment.total_cost,
                );

                on_delivery(shipment, allocator);
                crate::debug::work::add(crate::debug::work::Work::FreightDeliveries, 1);

                debug_log!(
                    "economy",
                    "OWA export fulfilled index={} resource={} amount={:.1} revenue={:.1}",
                    src_idx,
                    catalog
                        .resource_id_for_runtime_id(shipment.resource_runtime_id)
                        .unwrap_or("unknown"),
                    shipment.amount,
                    shipment.total_cost
                );
                self.start_return_or_finish(
                    idx,
                    shipment,
                    allocator,
                    agents,
                    transit_network,
                    graph,
                );
            } else {
                self.fail_in_transit_shipment(
                    idx,
                    allocator,
                    agents,
                    treasury_balance,
                    retry_cooldown_hours,
                    catalog,
                    ShipmentStatus::Failed,
                    "source_unavailable",
                );
            }
            return;
        }

        let ShipmentEndpoint::Building(dest_idx) = shipment.destination else {
            self.fail_in_transit_shipment(
                idx,
                allocator,
                agents,
                treasury_balance,
                retry_cooldown_hours,
                catalog,
                ShipmentStatus::Failed,
                "bad_destination",
            );
            return;
        };
        if dest_idx >= allocator.buildings.len() {
            self.fail_in_transit_shipment(
                idx,
                allocator,
                agents,
                treasury_balance,
                retry_cooldown_hours,
                catalog,
                ShipmentStatus::Failed,
                "destination_missing",
            );
            return;
        }

        match shipment.source {
            ShipmentEndpoint::Building(src_idx) => {
                if src_idx >= allocator.buildings.len()
                    || allocator.buildings[src_idx].broken
                    || allocator.buildings[src_idx].economy_broken
                    || allocator.buildings[src_idx].is_deserted
                    || allocator.buildings[src_idx].is_under_construction()
                    || allocator.buildings[dest_idx].broken
                    || allocator.buildings[dest_idx].economy_broken
                    || allocator.buildings[dest_idx].is_deserted
                    || allocator.buildings[dest_idx].is_under_construction()
                    || !building_accepts_input_resource(
                        catalog,
                        &allocator.buildings[dest_idx],
                        shipment.resource_runtime_id,
                    )
                {
                    self.fail_in_transit_shipment(
                        idx,
                        allocator,
                        agents,
                        treasury_balance,
                        retry_cooldown_hours,
                        catalog,
                        ShipmentStatus::Failed,
                        "endpoint_unavailable",
                    );
                    return;
                }

                allocator.buildings[src_idx].revenue +=
                    shipment.total_cost - shipment.freight_cost;
                allocator.buildings[src_idx].operating_budget +=
                    shipment.total_cost - shipment.freight_cost;
                // The buyer's escrow pays the seller; the carrier fee leaves the city.
                crate::debug::money::record_change(
                    crate::debug::money::MoneyFlow::FreightPurchases,
                    shipment.freight_cost,
                    0.0,
                );
                allocator.buildings[dest_idx]
                    .add_inventory_units(shipment.resource_runtime_id, shipment.amount);
                allocator.buildings[dest_idx].daily_local_input_value += shipment.total_cost;
                on_delivery(shipment, allocator);
                crate::debug::work::add(crate::debug::work::Work::FreightDeliveries, 1);
                debug_log!(
                    "economy",
                    "freight input fulfilled shipment_id={} source=local src_idx={} src_asset={} dest_idx={} dest_asset={} resource={} amount={:.1} cost={:.1} dest_inventory={:.1}",
                    shipment.id,
                    src_idx,
                    allocator.buildings[src_idx].asset_id,
                    dest_idx,
                    allocator.buildings[dest_idx].asset_id,
                    catalog
                        .resource_id_for_runtime_id(shipment.resource_runtime_id)
                        .unwrap_or("unknown"),
                    shipment.amount,
                    shipment.total_cost,
                    allocator.buildings[dest_idx].inventory_units(shipment.resource_runtime_id)
                );
                self.start_return_or_finish(
                    idx,
                    shipment,
                    allocator,
                    agents,
                    transit_network,
                    graph,
                );
            }
            ShipmentEndpoint::OwaBorder(border_node) => {
                if allocator.buildings[dest_idx].broken
                    || allocator.buildings[dest_idx].economy_broken
                    || allocator.buildings[dest_idx].is_deserted
                    || allocator.buildings[dest_idx].is_under_construction()
                    || !building_accepts_input_resource(
                        catalog,
                        &allocator.buildings[dest_idx],
                        shipment.resource_runtime_id,
                    )
                {
                    self.fail_in_transit_shipment(
                        idx,
                        allocator,
                        agents,
                        treasury_balance,
                        retry_cooldown_hours,
                        catalog,
                        ShipmentStatus::Failed,
                        "destination_unavailable",
                    );
                    return;
                }
                allocator.buildings[dest_idx]
                    .add_inventory_units(shipment.resource_runtime_id, shipment.amount);
                allocator.buildings[dest_idx].daily_owa_input_value += shipment.total_cost;
                // The buyer's escrow pays the outside seller and carrier.
                crate::debug::money::record_change(
                    crate::debug::money::MoneyFlow::FreightPurchases,
                    shipment.total_cost,
                    0.0,
                );
                on_delivery(shipment, allocator);
                crate::debug::work::add(crate::debug::work::Work::FreightDeliveries, 1);
                debug_log!(
                    "economy",
                    "freight input fulfilled shipment_id={} source=owa border_node={} dest_idx={} dest_asset={} resource={} amount={:.1} cost={:.1} dest_inventory={:.1}",
                    shipment.id,
                    border_node,
                    dest_idx,
                    allocator.buildings[dest_idx].asset_id,
                    catalog
                        .resource_id_for_runtime_id(shipment.resource_runtime_id)
                        .unwrap_or("unknown"),
                    shipment.amount,
                    shipment.total_cost,
                    allocator.buildings[dest_idx].inventory_units(shipment.resource_runtime_id)
                );
                self.start_return_or_finish(
                    idx,
                    shipment,
                    allocator,
                    agents,
                    transit_network,
                    graph,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start_return_or_finish(
        &mut self,
        shipment_idx: usize,
        shipment: &Shipment,
        allocator: &BuildingAllocator,
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        if self.start_return_trip(shipment, allocator, agents, transit_network, graph) {
            self.update_shipment(shipment_idx, |shipment| {
                shipment.status = ShipmentStatus::Returning;
            });
            self.shipments[shipment_idx].queued_hours = 0;
        } else {
            self.finish_delivered_shipment(shipment_idx, shipment, agents);
        }
    }

    fn finish_delivered_shipment(
        &mut self,
        shipment_idx: usize,
        shipment: &Shipment,
        agents: &mut AgentSystem,
    ) {
        // Cargo already settled; retiring an empty carrier must not reverse the sale.
        self.update_shipment(shipment_idx, |shipment| {
            shipment.status = ShipmentStatus::Fulfilled;
        });
        self.shipments[shipment_idx].carrier = AgentHandle::NONE;
        self.remove_carrier_agent(agents, shipment.carrier, shipment.id);
    }

    fn start_return_trip(
        &mut self,
        shipment: &Shipment,
        allocator: &BuildingAllocator,
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        let Some(carrier) = agents.resolve_agent(shipment.carrier) else {
            return false;
        };
        match (shipment.source, shipment.destination) {
            (ShipmentEndpoint::Building(source), ShipmentEndpoint::Building(destination)) => {
                source < allocator.buildings.len()
                    && destination < allocator.buildings.len()
                    && building_can_participate_in_freight(allocator, source)
                    && building_can_participate_in_freight(allocator, destination)
                    && agents.route_freight_carrier_building_to_building(
                        carrier,
                        destination,
                        source,
                        allocator,
                        transit_network,
                        graph,
                    )
            }
            (ShipmentEndpoint::OwaBorder(border_node), ShipmentEndpoint::Building(destination)) => {
                destination < allocator.buildings.len()
                    && building_can_participate_in_freight(allocator, destination)
                    && agents.route_freight_carrier_building_to_border(
                        carrier,
                        destination,
                        border_node,
                        allocator,
                        transit_network,
                        graph,
                    )
            }
            (ShipmentEndpoint::Building(source), ShipmentEndpoint::OwaBorder(border_node)) => {
                source < allocator.buildings.len()
                    && building_can_participate_in_freight(allocator, source)
                    && agents.route_freight_carrier_border_to_building(
                        carrier,
                        border_node,
                        source,
                        allocator,
                        transit_network,
                        graph,
                    )
            }
            _ => false,
        }
    }

    pub(super) fn remove_carrier_agent(
        &mut self,
        agents: &mut AgentSystem,
        carrier: AgentHandle,
        shipment_id: u64,
    ) {
        if let Some(carrier) = agents.resolve_agent(carrier) {
            agents.remove_freight_carrier(carrier, shipment_id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fail_in_transit_shipment(
        &mut self,
        shipment_idx: usize,
        allocator: &mut BuildingAllocator,
        agents: &mut AgentSystem,
        treasury_balance: &mut f64,
        retry_cooldown_hours: u16,
        catalog: &RuntimeEconomyCatalog,
        final_status: ShipmentStatus,
        reason: &'static str,
    ) {
        let shipment = self.shipments[shipment_idx].clone();
        if let ShipmentEndpoint::Building(destination_idx) = shipment.destination
            && destination_idx < allocator.buildings.len()
        {
            refund_input_payment(
                allocator,
                treasury_balance,
                destination_idx,
                shipment.total_cost,
            );
            allocator.buildings[destination_idx].shipment_cooldown_until_tick =
                self.hour_ticks + u32::from(retry_cooldown_hours);
        }
        if shipment.cargo_loaded
            && let ShipmentEndpoint::Building(source_idx) = shipment.source
            && source_idx < allocator.buildings.len()
        {
            allocator.buildings[source_idx]
                .add_inventory_units(shipment.resource_runtime_id, shipment.amount);
            allocator.buildings[source_idx].shipment_cooldown_until_tick =
                self.hour_ticks + u32::from(retry_cooldown_hours);
        }

        let (source_kind, source_id) = shipment_endpoint_log_fields(shipment.source);
        let (destination_kind, destination_id) = shipment_endpoint_log_fields(shipment.destination);
        debug_log!(
            "economy",
            "freight shipment failed shipment_id={} status={:?} reason={} source_kind={} source_id={} destination_kind={} destination_id={} resource={} amount={:.1} age={}h eta={}h timeout={}h cost={:.1}",
            shipment.id,
            final_status,
            reason,
            source_kind,
            source_id,
            destination_kind,
            destination_id,
            catalog
                .resource_id_for_runtime_id(shipment.resource_runtime_id)
                .unwrap_or("unknown"),
            shipment.amount,
            shipment.queued_hours,
            shipment.eta_hours,
            in_transit_timeout_hours(&shipment),
            shipment.total_cost,
        );

        self.update_shipment(shipment_idx, |shipment| shipment.status = final_status);
        self.shipments[shipment_idx].carrier = AgentHandle::NONE;
        self.remove_carrier_agent(agents, shipment.carrier, shipment.id);
    }
}

fn in_transit_timeout_hours(shipment: &Shipment) -> u16 {
    shipment
        .eta_hours
        .saturating_mul(4)
        .saturating_add(2)
        .max(6)
}

fn shipment_endpoint_log_fields(endpoint: ShipmentEndpoint) -> (&'static str, u64) {
    match endpoint {
        ShipmentEndpoint::Building(building_idx) => ("building", building_idx as u64),
        ShipmentEndpoint::OwaBorder(border_node) => ("owa", u64::from(border_node)),
    }
}

fn shipment_endpoints_ready(
    shipment: &Shipment,
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
) -> bool {
    if let ShipmentEndpoint::Building(source_idx) = shipment.source
        && (source_idx >= allocator.buildings.len()
            || !building_can_participate_in_freight(allocator, source_idx)
            || allocator.buildings[source_idx].inventory_units(shipment.resource_runtime_id)
                < shipment.amount)
    {
        return false;
    }

    if let ShipmentEndpoint::Building(destination_idx) = shipment.destination
        && (destination_idx >= allocator.buildings.len()
            || !building_can_participate_in_freight(allocator, destination_idx)
            || !building_accepts_input_resource(
                catalog,
                &allocator.buildings[destination_idx],
                shipment.resource_runtime_id,
            ))
    {
        return false;
    }

    true
}

// Puts a loaded shipment's cargo back into its source building, if it has one.
fn return_loaded_cargo(shipment: &Shipment, allocator: &mut BuildingAllocator) {
    if let ShipmentEndpoint::Building(source_idx) = shipment.source
        && source_idx < allocator.buildings.len()
    {
        allocator.buildings[source_idx]
            .add_inventory_units(shipment.resource_runtime_id, shipment.amount);
    }
}

fn building_can_participate_in_freight(allocator: &BuildingAllocator, building_idx: usize) -> bool {
    let building = &allocator.buildings[building_idx];
    !building.broken
        && !building.economy_broken
        && !building.is_deserted
        && !building.is_under_construction()
}

// `cooldown_until_tick` is the logistics hour tick from which both endpoints may request again.
fn fail_shipment_before_dispatch(
    shipment: &mut Shipment,
    allocator: &mut BuildingAllocator,
    treasury_balance: &mut f64,
    cooldown_until_tick: u32,
) {
    if let ShipmentEndpoint::Building(destination_idx) = shipment.destination
        && destination_idx < allocator.buildings.len()
    {
        refund_input_payment(
            allocator,
            treasury_balance,
            destination_idx,
            shipment.total_cost,
        );
        allocator.buildings[destination_idx].shipment_cooldown_until_tick = cooldown_until_tick;
    }
    if let ShipmentEndpoint::Building(source_idx) = shipment.source
        && source_idx < allocator.buildings.len()
    {
        allocator.buildings[source_idx].shipment_cooldown_until_tick = cooldown_until_tick;
    }
    shipment.status = ShipmentStatus::Failed;
}

fn shipment_carrier_progress_to_endpoint(
    shipment: &Shipment,
    agents: &AgentSystem,
    endpoint: ShipmentEndpoint,
) -> CarrierProgress {
    let Some(carrier_idx) = agents
        .resolve_agent(shipment.carrier)
        .filter(|&carrier| agents.freight_carrier_matches(carrier, shipment.id))
    else {
        return CarrierProgress::Missing;
    };

    let arrived = match endpoint {
        ShipmentEndpoint::Building(destination_idx) => {
            agents.transit[carrier_idx] == TRANSIT_IN_BUILDING
                && agents.current_building[carrier_idx] == destination_idx
        }
        ShipmentEndpoint::OwaBorder(border_node) => {
            agents.current_node[carrier_idx] == border_node
                && agents.current_lane_id[carrier_idx] == usize::MAX
                && agents.current_path[carrier_idx].is_empty()
        }
    };
    if arrived {
        CarrierProgress::Arrived
    } else {
        CarrierProgress::Traveling
    }
}
