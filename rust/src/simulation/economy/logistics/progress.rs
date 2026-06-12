//! Shipment arrival, refund, and failure handling.

use std::collections::HashMap;

use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::{
    AgentSystem, TRANSIT_IN_BUILDING, VEHICLE_FREIGHT_DELIVERY,
};
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

use super::data::{Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem};
use super::resource::building_accepts_input_resource;

enum CarrierProgress {
    Arrived,
    Traveling,
    Missing,
}

impl ShipmentSystem {
    pub(super) fn progress_shipments(
        &mut self,
        allocator: &mut BuildingAllocator,
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> f32 {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let retry_cooldown_hours = tuning.operational_clock.shipment_retry_cooldown_hours;
        dispatch_queued_shipments(
            &mut self.shipments,
            allocator,
            usize::from(tuning.logistics.border_active_jobs_per_node),
            tuning.logistics.queued_shipment_expiry_hours,
            retry_cooldown_hours,
        );

        self.dispatch_ready_carriers(
            allocator,
            agents,
            transit_network,
            graph,
            &catalog,
            retry_cooldown_hours,
        );

        let mut business_purchase_tax_collected = 0.0;
        let mut idx = 0;
        while idx < self.shipments.len() {
            if self.shipments[idx].carrier_agent_id == usize::MAX {
                idx += 1;
                continue;
            }

            let status = self.shipments[idx].status;
            let shipment = self.shipments[idx].clone();
            if status == ShipmentStatus::Returning {
                match shipment_carrier_progress_to_endpoint(&shipment, agents, shipment.source) {
                    CarrierProgress::Arrived => {
                        self.shipments[idx].status = ShipmentStatus::Fulfilled;
                        self.remove_carrier_agent(agents, shipment.carrier_agent_id);
                    }
                    CarrierProgress::Traveling => {}
                    CarrierProgress::Missing => {
                        self.shipments[idx].status = ShipmentStatus::Fulfilled;
                    }
                }
                idx += 1;
                continue;
            }

            if status != ShipmentStatus::InTransit {
                idx += 1;
                continue;
            }

            match shipment_carrier_progress_to_endpoint(&shipment, agents, shipment.destination) {
                CarrierProgress::Arrived => {}
                CarrierProgress::Traveling => {
                    idx += 1;
                    continue;
                }
                CarrierProgress::Missing => {
                    fail_shipment_before_dispatch(
                        &mut self.shipments[idx],
                        allocator,
                        retry_cooldown_hours,
                    );
                    idx += 1;
                    continue;
                }
            }

            // OWA export: goods travel from source building to border terminal; no local
            // destination building receives them. Credit revenue to the source on arrival.
            if let ShipmentEndpoint::OwaBorder(_) = shipment.destination {
                let ShipmentEndpoint::Building(src_idx) = shipment.source else {
                    self.shipments[idx].status = ShipmentStatus::Failed;
                    self.remove_carrier_agent(agents, shipment.carrier_agent_id);
                    idx += 1;
                    continue;
                };
                if src_idx < allocator.buildings.len()
                    && !allocator.buildings[src_idx].broken
                    && !allocator.buildings[src_idx].economy_broken
                    && !allocator.buildings[src_idx].is_deserted
                    && !allocator.buildings[src_idx].is_under_construction()
                {
                    self.record_owa_export_saturation(
                        shipment.resource_runtime_id,
                        shipment.amount,
                        catalog.resource_count(),
                    );
                    allocator.buildings[src_idx].revenue += shipment.total_cost;
                    allocator.buildings[src_idx].operating_budget += shipment.total_cost;

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
                        &shipment,
                        allocator,
                        agents,
                        transit_network,
                        graph,
                    );
                } else {
                    self.shipments[idx].status = ShipmentStatus::Failed;
                    self.remove_carrier_agent(agents, shipment.carrier_agent_id);
                }
                idx += 1;
                continue;
            }

            let ShipmentEndpoint::Building(dest_idx) = shipment.destination else {
                self.shipments[idx].status = ShipmentStatus::Failed;
                self.remove_carrier_agent(agents, shipment.carrier_agent_id);
                idx += 1;
                continue;
            };
            if dest_idx >= allocator.buildings.len() {
                self.shipments[idx].status = ShipmentStatus::Failed;
                self.remove_carrier_agent(agents, shipment.carrier_agent_id);
                idx += 1;
                continue;
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
                            &catalog,
                            &allocator.buildings[dest_idx],
                            shipment.resource_runtime_id,
                        )
                    {
                        allocator.buildings[dest_idx].operating_budget +=
                            shipment.total_cost + shipment.tax_cost;
                        allocator.buildings[dest_idx].shipment_cooldown_hours =
                            retry_cooldown_hours;
                        self.shipments[idx].status = ShipmentStatus::Failed;
                        self.remove_carrier_agent(agents, shipment.carrier_agent_id);
                        idx += 1;
                        continue;
                    }

                    allocator.buildings[src_idx].revenue += shipment.total_cost;
                    allocator.buildings[src_idx].operating_budget += shipment.total_cost;
                    allocator.buildings[dest_idx]
                        .add_inventory_units(shipment.resource_runtime_id, shipment.amount);
                    allocator.buildings[dest_idx].daily_local_input_value += shipment.total_cost;
                    business_purchase_tax_collected += shipment.tax_cost;
                    self.start_return_or_finish(
                        idx,
                        &shipment,
                        allocator,
                        agents,
                        transit_network,
                        graph,
                    );
                }
                ShipmentEndpoint::OwaBorder(_) => {
                    if allocator.buildings[dest_idx].broken
                        || allocator.buildings[dest_idx].economy_broken
                        || allocator.buildings[dest_idx].is_deserted
                        || allocator.buildings[dest_idx].is_under_construction()
                        || !building_accepts_input_resource(
                            &catalog,
                            &allocator.buildings[dest_idx],
                            shipment.resource_runtime_id,
                        )
                    {
                        allocator.buildings[dest_idx].operating_budget +=
                            shipment.total_cost + shipment.tax_cost;
                        allocator.buildings[dest_idx].shipment_cooldown_hours =
                            retry_cooldown_hours;
                        self.shipments[idx].status = ShipmentStatus::Failed;
                        self.remove_carrier_agent(agents, shipment.carrier_agent_id);
                        idx += 1;
                        continue;
                    }
                    allocator.buildings[dest_idx]
                        .add_inventory_units(shipment.resource_runtime_id, shipment.amount);
                    allocator.buildings[dest_idx].daily_owa_input_value += shipment.total_cost;
                    business_purchase_tax_collected += shipment.tax_cost;
                    self.start_return_or_finish(
                        idx,
                        &shipment,
                        allocator,
                        agents,
                        transit_network,
                        graph,
                    );
                }
            }
            idx += 1;
        }
        business_purchase_tax_collected
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_ready_carriers(
        &mut self,
        allocator: &mut BuildingAllocator,
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        retry_cooldown_hours: u16,
    ) {
        for idx in 0..self.shipments.len() {
            if self.shipments[idx].status != ShipmentStatus::InTransit
                || self.shipments[idx].carrier_agent_id != usize::MAX
            {
                continue;
            }

            let shipment = self.shipments[idx].clone();
            if !shipment_endpoints_ready(&shipment, allocator, catalog) {
                fail_shipment_before_dispatch(
                    &mut self.shipments[idx],
                    allocator,
                    retry_cooldown_hours,
                );
                continue;
            }

            let carrier = match (shipment.source, shipment.destination) {
                (ShipmentEndpoint::Building(source), ShipmentEndpoint::Building(destination)) => {
                    agents.spawn_city_freight_carrier(
                        shipment.id,
                        source,
                        destination,
                        allocator,
                        transit_network,
                        graph,
                    )
                }
                (
                    ShipmentEndpoint::OwaBorder(border_node),
                    ShipmentEndpoint::Building(destination),
                ) => agents.spawn_import_freight_carrier(
                    shipment.id,
                    border_node,
                    destination,
                    allocator,
                    transit_network,
                    graph,
                ),
                (ShipmentEndpoint::Building(source), ShipmentEndpoint::OwaBorder(border_node)) => {
                    agents.spawn_export_freight_carrier(
                        shipment.id,
                        source,
                        border_node,
                        allocator,
                        transit_network,
                        graph,
                    )
                }
                _ => None,
            };

            let Some(carrier_agent_id) = carrier else {
                fail_shipment_before_dispatch(
                    &mut self.shipments[idx],
                    allocator,
                    retry_cooldown_hours,
                );
                continue;
            };

            if let ShipmentEndpoint::Building(source_idx) = shipment.source {
                allocator.buildings[source_idx]
                    .remove_inventory_units(shipment.resource_runtime_id, shipment.amount);
            }
            self.shipments[idx].carrier_agent_id = carrier_agent_id;
        }
    }

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
            self.shipments[shipment_idx].status = ShipmentStatus::Returning;
        } else {
            self.shipments[shipment_idx].status = ShipmentStatus::Fulfilled;
            self.remove_carrier_agent(agents, shipment.carrier_agent_id);
        }
    }

    fn start_return_trip(
        &mut self,
        shipment: &Shipment,
        allocator: &BuildingAllocator,
        agents: &mut AgentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> bool {
        match (shipment.source, shipment.destination) {
            (ShipmentEndpoint::Building(source), ShipmentEndpoint::Building(destination)) => {
                source < allocator.buildings.len()
                    && destination < allocator.buildings.len()
                    && building_can_participate_in_freight(allocator, source)
                    && building_can_participate_in_freight(allocator, destination)
                    && agents.route_freight_carrier_building_to_building(
                        shipment.carrier_agent_id,
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
                        shipment.carrier_agent_id,
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
                        shipment.carrier_agent_id,
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
        carrier_agent_id: usize,
    ) {
        if let Some((old_idx, new_idx)) = agents.remove_freight_carrier(carrier_agent_id) {
            self.remap_carrier_agent_index(old_idx, new_idx);
        }
    }
}

fn shipment_endpoints_ready(
    shipment: &Shipment,
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
) -> bool {
    if let ShipmentEndpoint::Building(source_idx) = shipment.source {
        if source_idx >= allocator.buildings.len()
            || !building_can_participate_in_freight(allocator, source_idx)
            || allocator.buildings[source_idx].inventory_units(shipment.resource_runtime_id)
                < shipment.amount
        {
            return false;
        }
    }

    if let ShipmentEndpoint::Building(destination_idx) = shipment.destination {
        if destination_idx >= allocator.buildings.len()
            || !building_can_participate_in_freight(allocator, destination_idx)
            || !building_accepts_input_resource(
                catalog,
                &allocator.buildings[destination_idx],
                shipment.resource_runtime_id,
            )
        {
            return false;
        }
    }

    true
}

fn building_can_participate_in_freight(allocator: &BuildingAllocator, building_idx: usize) -> bool {
    let building = &allocator.buildings[building_idx];
    !building.broken
        && !building.economy_broken
        && !building.is_deserted
        && !building.is_under_construction()
}

fn fail_shipment_before_dispatch(
    shipment: &mut Shipment,
    allocator: &mut BuildingAllocator,
    retry_cooldown_hours: u16,
) {
    if let ShipmentEndpoint::Building(destination_idx) = shipment.destination
        && destination_idx < allocator.buildings.len()
    {
        allocator.buildings[destination_idx].operating_budget +=
            shipment.total_cost + shipment.tax_cost;
        allocator.buildings[destination_idx].shipment_cooldown_hours = retry_cooldown_hours;
    }
    if let ShipmentEndpoint::Building(source_idx) = shipment.source
        && source_idx < allocator.buildings.len()
    {
        allocator.buildings[source_idx].shipment_cooldown_hours = retry_cooldown_hours;
    }
    shipment.status = ShipmentStatus::Failed;
}

fn shipment_carrier_progress_to_endpoint(
    shipment: &Shipment,
    agents: &AgentSystem,
    endpoint: ShipmentEndpoint,
) -> CarrierProgress {
    let carrier_idx = shipment.carrier_agent_id;
    if carrier_idx >= agents.len()
        || agents.freight_shipment_id[carrier_idx] != shipment.id
        || agents.vehicle_type[carrier_idx] != VEHICLE_FREIGHT_DELIVERY
    {
        return CarrierProgress::Missing;
    }

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

fn dispatch_queued_shipments(
    shipments: &mut [Shipment],
    allocator: &mut BuildingAllocator,
    active_cap: usize,
    expiry_hours: u16,
    retry_cooldown_hours: u16,
) {
    let mut active_counts = HashMap::new();
    for shipment in shipments.iter() {
        if shipment.status != ShipmentStatus::InTransit {
            continue;
        }
        if let Some(border_node) = shipment
            .source
            .border_node()
            .or_else(|| shipment.destination.border_node())
        {
            *active_counts.entry(border_node).or_insert(0usize) += 1;
        }
    }

    for shipment in shipments.iter_mut() {
        if shipment.status != ShipmentStatus::Queued {
            continue;
        }
        shipment.queued_hours = shipment.queued_hours.saturating_add(1);
        if shipment.queued_hours >= expiry_hours {
            expire_queued_shipment(shipment, allocator, retry_cooldown_hours);
            continue;
        }
        let Some(border_node) = shipment
            .source
            .border_node()
            .or_else(|| shipment.destination.border_node())
        else {
            shipment.status = ShipmentStatus::Failed;
            continue;
        };
        let active_count = active_counts.entry(border_node).or_insert(0usize);
        if *active_count < active_cap {
            *active_count += 1;
            shipment.status = ShipmentStatus::InTransit;
        }
    }
}

fn expire_queued_shipment(
    shipment: &mut Shipment,
    allocator: &mut BuildingAllocator,
    retry_cooldown_hours: u16,
) {
    if let ShipmentEndpoint::Building(destination_idx) = shipment.destination
        && destination_idx < allocator.buildings.len()
    {
        allocator.buildings[destination_idx].operating_budget +=
            shipment.total_cost + shipment.tax_cost;
        allocator.buildings[destination_idx].shipment_cooldown_hours = retry_cooldown_hours;
    }
    shipment.status = ShipmentStatus::Expired;
}
