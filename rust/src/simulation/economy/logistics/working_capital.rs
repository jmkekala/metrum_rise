// SPDX-License-Identifier: GPL-2.0-only

//! Unsettled freight assets used by the shared business funding forecast.

use super::{ShipmentEndpoint, ShipmentStatus, ShipmentSystem};
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use rayon::prelude::*;

impl ShipmentSystem {
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

    /// Prepares shared border sweeps once per road revision for prospective-site pricing.
    /// Building placement retains these tables; warm preparation is O(K log K), where K is connected borders.
    pub(crate) fn prepare_site_carrier_quotes(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        borders: &[u32],
    ) {
        self.refresh_freight_route_cache(allocator, network);
        self.freight_route_cache
            .prepare_border_costs(borders, network, graph);
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
        self.freight_route_cache
            .prepare_border_costs(borders, network, graph);
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
