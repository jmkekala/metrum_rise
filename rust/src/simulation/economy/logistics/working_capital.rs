// SPDX-License-Identifier: GPL-2.0-only

//! Unsettled freight assets used by the shared business funding forecast.

use super::{ShipmentEndpoint, ShipmentStatus, ShipmentSystem};
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use rayon::prelude::*;

impl ShipmentSystem {
    /// Prices a live or proposed seller's directed delivery to a business customer.
    /// Call `prepare_site_carrier_quotes` once before a batch. Each query reuses bounded
    /// network search storage and applies live frontage delays, without temporary IDs.
    pub(crate) fn customer_carrier_quote(
        &mut self,
        source: &BuildingEntrance,
        destination: &BuildingEntrance,
        source_building: Option<usize>,
        destination_building: Option<usize>,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        tuning: &RuntimeEconomyTuning,
    ) -> Option<f32> {
        // Replacement owners retain their identity while quoting a different door. Only an
        // exact live entrance may enter the allocator-indexed route/geometry caches.
        let source_building = source_building.filter(|&index| allocator.entrances.get(index) == Some(source));
        let destination_building = destination_building.filter(|&index| allocator.entrances.get(index) == Some(destination));
        let seconds = match (source_building, destination_building) {
            (Some(from), Some(to)) => self.freight_route_cache.between_buildings(from, to, allocator, network, graph),
            (Some(from), None) => self.freight_route_cache.to_site(from, destination, allocator, network, graph),
            (None, Some(to)) => self.freight_route_cache.from_site(source, to, allocator, network, graph),
            (None, None) => self.freight_route_cache.between_entrances(source, destination, network, graph),
        };
        seconds.map(|seconds| super::timing::freight_cost(
                seconds, tuning.operational_clock.seconds_per_day, &tuning.logistics))
    }

    /// Stages a finite recurring input quote using the same revision-scoped freight routes.
    /// The caller commits the complete funded operation or rolls every local claim back.
    pub(crate) fn quote_recurring_input(
        &mut self,
        supply: &mut super::RecurringSupply,
        input: crate::simulation::economy::operation::InputRequirement,
        buyer: Option<usize>,
        entrance: &BuildingEntrance,
        outside_quote: Option<f32>,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        network: &TransitNetwork,
        graph: &RegionGraph,
        goods_multiplier: f32,
    ) -> crate::simulation::economy::operation::InputSupplyCosts {
        supply.quote(
            input,
            buyer,
            None,
            entrance,
            outside_quote,
            allocator,
            catalog,
            tuning,
            network,
            graph,
            &mut self.freight_route_cache,
            goods_multiplier,
        )
    }

    /// Repairs only the missing part of an accepted buyer's input after source withdrawal.
    /// Retained tranches keep their quantities/prices; the full requirement prices buffers.
    /// Commit with `extend_inputs_for` inside the enclosing revalidation transaction.
    pub(crate) fn quote_missing_recurring_input(
        &mut self,
        supply: &mut super::RecurringSupply,
        input: crate::simulation::economy::operation::InputRequirement,
        owner: usize,
        buyer: Option<usize>,
        entrance: &BuildingEntrance,
        outside_quote: Option<f32>,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        network: &TransitNetwork,
        graph: &RegionGraph,
        goods_multiplier: f32,
    ) -> crate::simulation::economy::operation::InputSupplyCosts {
        supply.quote(input, buyer, Some(owner), entrance, outside_quote, allocator, catalog,
            tuning, network, graph, &mut self.freight_route_cache, goods_multiplier)
    }

    /// Prepares shared border sweeps once per road revision for prospective-site pricing.
    /// Building placement retains border tables and reserves geometry slots only for added buildings.
    /// Warm preparation is O(K log K), where K is connected borders. Quotes lazily project
    /// each requested door once per revision, without a city scan or query allocation.
    pub(crate) fn prepare_site_carrier_quotes(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        borders: &[u32],
    ) {
        self.refresh_freight_route_cache(allocator, network);
        self.freight_route_cache.prepare_buildings(allocator.buildings.len());
        self.freight_route_cache
            .prepare_border_costs(borders, allocator.buildings.len(), network, graph);
    }

    /// Prices a proposed door using the same physical route and fee as live dispatch.
    /// O(K log K), allocation-free, for K connected borders; unreachable routes return `None`.
    pub(crate) fn site_carrier_quote(
        &self,
        entrance: &BuildingEntrance,
        network: &TransitNetwork,
        graph: &RegionGraph,
        borders: &[u32],
        tuning: &RuntimeEconomyTuning,
    ) -> Option<f32> {
        self.freight_route_cache
            .minimum_site_border_eta(borders, entrance, network, graph)
            .map(|seconds| {
                super::timing::freight_cost(
                    seconds,
                    tuning.operational_clock.seconds_per_day,
                    &tuning.logistics,
                )
            })
    }

    /// Quotes the reachable outside route used by actual freight dispatch, using its cached
    /// border sweeps. Warm work is O(B*K), parallel across buildings; K is connected borders.
    /// The caller reuses quote storage. No per-building path search or allocation is added.
    pub(crate) fn write_border_carrier_quotes(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        borders: &[u32],
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        quotes: &mut [Option<f32>],
    ) {
        self.refresh_freight_route_cache(allocator, network);
        self.freight_route_cache.prepare_buildings(allocator.buildings.len());
        self.freight_route_cache
            .prepare_border_costs(borders, allocator.buildings.len(), network, graph);
        quotes
            .par_iter_mut()
            .enumerate()
            .for_each(|(index, quote)| {
                *quote = None;
                let building = &allocator.buildings[index];
                // Include unfinished/inactive sites: completing construction or recovering
                // a business does not change its cached physical route.
                if !catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .is_some_and(|profile| profile.worker_capacity > 0)
                {
                    return;
                }
                *quote = self
                    .freight_route_cache
                    .minimum_border_eta(borders, index, allocator, network, graph)
                    .map(|seconds| {
                        super::timing::freight_cost(
                            seconds,
                            tuning.operational_clock.seconds_per_day,
                            &tuning.logistics,
                        )
                    });
            });
    }

    /// Writes paid inbound goods and dispatched sales receivables, excluding carrier charges.
    /// On-site stock is valued separately; undeparted outbound goods remain in that stock.
    /// O(B + S), with caller-owned storage and no per-shipment allocation. The scatter follows
    /// stable shipment order so repeated credits to one business sum deterministically.
    pub(crate) fn write_freight_working_capital(&self, capital: &mut [f64]) {
        capital.fill(0.0);
        for shipment in &self.shipments {
            if !shipment.status.reserves_cargo() {
                continue;
            }
            let goods_value = f64::from((shipment.total_cost - shipment.freight_cost).max(0.0));
            if let ShipmentEndpoint::Building(destination) = shipment.destination
                && let Some(value) = capital.get_mut(destination)
            {
                *value += goods_value;
            }
            if shipment.status == ShipmentStatus::InTransit
                && shipment.carrier_agent_id != usize::MAX
                && let ShipmentEndpoint::Building(source) = shipment.source
                && let Some(value) = capital.get_mut(source)
            {
                *value += goods_value;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::logistics::{CarrierClass, Shipment};

    fn shipment(source: ShipmentEndpoint, destination: ShipmentEndpoint) -> Shipment {
        Shipment {
            id: 1,
            resource_runtime_id: 1,
            amount: 10.0,
            source,
            destination,
            carrier_class: CarrierClass::Truck,
            status: ShipmentStatus::Queued,
            carrier_agent_id: usize::MAX,
            total_cost: 108.0,
            freight_cost: 8.0,
            eta_hours: 1,
            queued_hours: 0,
        }
    }

    #[test]
    fn paid_goods_survive_reservation_and_dispatch_without_double_counting_seller_stock() {
        let mut logistics = ShipmentSystem::new();
        logistics.shipments.push(shipment(
            ShipmentEndpoint::Building(0),
            ShipmentEndpoint::Building(1),
        ));
        let mut capital = [0.0; 2];
        logistics.write_freight_working_capital(&mut capital);
        assert_eq!(capital, [0.0, 100.0]);
        logistics.shipments[0].status = ShipmentStatus::InTransit;
        logistics.write_freight_working_capital(&mut capital);
        assert_eq!(
            capital,
            [0.0, 100.0],
            "source stock still exists before carrier dispatch"
        );
        logistics.shipments[0].carrier_agent_id = 7;
        logistics.write_freight_working_capital(&mut capital);
        assert_eq!(
            capital,
            [100.0, 100.0],
            "dispatched source stock becomes a receivable"
        );
        for status in [
            ShipmentStatus::Returning,
            ShipmentStatus::Fulfilled,
            ShipmentStatus::Failed,
            ShipmentStatus::Expired,
        ] {
            logistics.shipments[0].status = status;
            logistics.write_freight_working_capital(&mut capital);
            assert_eq!(
                capital,
                [0.0, 0.0],
                "closed cargo must not survive as a second asset"
            );
        }
    }

    #[test]
    fn export_receivable_excludes_the_unpaid_carrier_bill() {
        let mut logistics = ShipmentSystem::new();
        let mut export = shipment(
            ShipmentEndpoint::Building(0),
            ShipmentEndpoint::OwaBorder(1),
        );
        export.status = ShipmentStatus::InTransit;
        export.carrier_agent_id = 7;
        logistics.shipments.push(export);
        let mut capital = [0.0];
        logistics.write_freight_working_capital(&mut capital);
        assert_eq!(capital, [100.0]);
    }
}
