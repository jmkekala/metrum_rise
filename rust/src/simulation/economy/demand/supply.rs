// SPDX-License-Identifier: GPL-2.0-only

//! Construction input quotations and transaction boundaries over the live residual market.

use super::spawn_need::{
    OutputAbsorptionContext, candidate_economy_profile, constructing_area_covers_operation,
    market_covers_operation_with_input_quotes,
};
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::fiscal::{CityFiscalPolicy, daily_property_tax};
use crate::simulation::economy::households::{
    local_utility_service_costs, output_area_scale_for_building,
    physical_worker_capacity_for_profile,
};
use crate::simulation::economy::logistics::{RecurringSupply, ShipmentSystem, freight_goods_multiplier};
use crate::simulation::economy::operation::{
    InputRequirement, InputSupplyCosts, OperatingCosts, OperationPlan, OperationPrices,
};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use crate::simulation::zoning::ZoningSystem;

/// One proposed operation stages inputs until every independent admission check passes.
pub(super) trait ProjectSupply {
    /// Rolls back the preceding probe and prepares the proposed asset's actual door.
    fn begin(&mut self, parcel: u64, asset: &str) -> Option<f32>;
    /// Prices a replacement after temporarily releasing that incumbent's input claims.
    fn begin_replacement(&mut self, _owner: usize, parcel: u64, asset: &str) -> Option<f32> {
        self.begin(parcel, asset)
    }
    /// Values retained input stock at this operation's quoted replacement prices.
    fn input_price(
        &self,
        resource: u16,
        catalog: &RuntimeEconomyCatalog,
        prices: OperationPrices,
    ) -> f32 {
        catalog.unit_price_for_resource(resource).unwrap_or(0.0) * prices.imports
    }
    /// Quotes the complete requirement against residual recurring supplier capacity.
    fn quote(
        &mut self,
        input: InputRequirement,
        catalog: &RuntimeEconomyCatalog,
        prices: OperationPrices,
    ) -> InputSupplyCosts;
    /// Makes this operation's claims unavailable to following proposals.
    fn accept(&mut self);
    /// Restores every staged claim when any later check fails.
    fn reject(&mut self);
}

/// Runtime quotation context borrowing prepared scratch; it owns no per-project heap storage.
pub(super) struct SiteSupply<'a> {
    book: &'a mut RecurringSupply,
    allocator: &'a BuildingAllocator,
    zoning: &'a ZoningSystem,
    network: &'a TransitNetwork,
    graph: &'a RegionGraph,
    logistics: &'a mut ShipmentSystem,
    tuning: &'a RuntimeEconomyTuning,
    borders: &'a [u32],
    entrance: Option<BuildingEntrance>,
    carrier: Option<f32>,
    buyer: Option<usize>,
    catalog: &'a RuntimeEconomyCatalog,
    minute_of_day: u16,
    goods_multiplier: f32,
}

impl<'a> SiteSupply<'a> {
    /// Borrows the pass's residual book after its border route tables have been prepared.
    pub(super) fn new(
        book: &'a mut RecurringSupply,
        allocator: &'a BuildingAllocator,
        zoning: &'a ZoningSystem,
        network: &'a TransitNetwork,
        graph: &'a RegionGraph,
        logistics: &'a mut ShipmentSystem,
        tuning: &'a RuntimeEconomyTuning,
        borders: &'a [u32],
        catalog: &'a RuntimeEconomyCatalog,
        minute_of_day: u16,
    ) -> Self {
        Self {
            book,
            allocator,
            zoning,
            network,
            graph,
            logistics,
            tuning,
            borders,
            entrance: None,
            carrier: None,
            buyer: None,
            catalog,
            minute_of_day,
            goods_multiplier: 1.0,
        }
    }

    /// Uses a current placed or queued door, without inventing a temporary building index.
    pub(super) fn begin_entrance(
        &mut self,
        entrance: Option<BuildingEntrance>,
        buyer: Option<usize>,
        goods_multiplier: f32,
    ) -> Option<f32> {
        self.reject();
        self.goods_multiplier = goods_multiplier;
        self.carrier = entrance.as_ref().and_then(|entrance| {
            self.logistics.site_carrier_quote(
                entrance,
                self.network,
                self.graph,
                self.borders,
                self.tuning,
            )
        }).filter(|_| goods_multiplier.is_finite());
        self.entrance = entrance;
        self.buyer = buyer;
        self.carrier
    }

    /// Replays constructing and queued input commitments in priority order. Releasing all
    /// project outputs in a private market first prevents each site from seeing only the
    /// final residual demand. The shared caller market keeps every full output commitment.
    /// O(B + queued projects + quoted ports/routes), never a city scan per proposal.
    pub(super) fn reserve_projects<'q>(
        &mut self,
        projects: impl Iterator<Item = &'q super::queued::QueuedConstruction> + Clone,
        catalog: &RuntimeEconomyCatalog,
        market: &OutputAbsorptionContext,
        replay: &mut OutputAbsorptionContext,
        policy: &CityFiscalPolicy,
        service_capacities: &[u32],
        freight_capital: &[f64],
    ) {
        replay.copy_from(market);
        // Ordered capacity updates share resource totals, like the subsequent finite claims.
        for building in &self.allocator.buildings {
            if building.is_under_construction()
                && !self.allocator.is_city_service_building(building)
                && let Some(profile) =
                    catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            {
                replay
                    .release_candidate(profile, output_area_scale_for_building(building, profile));
            }
        }
        for project in projects.clone() {
            if let Some(profile) = catalog.profile_by_runtime_id(project.profile_runtime_id) {
                replay.release_candidate(profile, 1.0);
            }
        }
        self.reserve_constructing(catalog, replay, policy, service_capacities, freight_capital);
        self.reserve_queued(projects, catalog, replay, policy);
    }

    fn reserve_constructing(
        &mut self,
        catalog: &RuntimeEconomyCatalog,
        market: &mut OutputAbsorptionContext,
        policy: &CityFiscalPolicy,
        service_capacities: &[u32],
        freight_capital: &[f64],
    ) {
        for index in 0..self.allocator.buildings.len() {
            let building = &self.allocator.buildings[index];
            if !building.is_under_construction() {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            let service = self.allocator.is_city_service_building(building);
            if profile.inputs.is_empty() {
                if !service {
                    market.reserve_scaled_candidate(
                        profile,
                        output_area_scale_for_building(building, profile),
                    );
                }
                continue;
            }
            let carrier =
                self.begin_entrance(self.allocator.entrances.get(index).cloned(), Some(index),
                    freight_goods_multiplier(profile, building.zone_type, self.tuning, self.minute_of_day));
            let prices = self.prices(catalog, building.zone_type, building.level, policy, carrier);
            let scale = output_area_scale_for_building(building, profile);
            if service {
                let physical = physical_worker_capacity_for_profile(building, profile);
                let funded = service_capacities
                    .get(index)
                    .copied()
                    .unwrap_or(0)
                    .min(physical);
                // Treasury-backed material invoices do not require private sales. Keep these
                // future input claims separate from the zero live jobs during construction.
                for workers in (1..=funded).rev() {
                    self.book.retry(self.allocator, self.graph);
                    let costs = OperatingCosts::for_plan_with_input_quotes(
                        profile,
                        OperationPlan {
                            workers,
                            activity: workers as f32 / physical.max(1) as f32,
                            area: scale,
                        },
                        prices,
                        |_, _| 0.0,
                        |input| self.quote(input, catalog, prices),
                    );
                    if costs.inputs.is_finite() && costs.freight.is_finite() {
                        self.accept();
                        break;
                    }
                }
                self.reject();
                continue;
            }
            let viable =
                if crate::simulation::work_area::profile_kind_uses_explicit_work_area(profile.kind)
                {
                    constructing_area_covers_operation(
                        building,
                        profile,
                        catalog,
                        market.market(),
                        prices,
                        self.tuning.owa_export_price_multiplier,
                        freight_capital.get(index).copied().unwrap_or(0.0),
                        self,
                    )
                } else {
                    market_covers_operation_with_input_quotes(
                        profile,
                        catalog,
                        market.market(),
                        prices,
                        self.tuning.owa_export_price_multiplier,
                        |input| self.quote(input, catalog, prices),
                    )
                };
            if viable {
                self.accept();
            } else {
                self.reject();
            }
            market.reserve_scaled_candidate(profile, scale);
        }
    }

    fn reserve_queued<'q>(
        &mut self,
        projects: impl Iterator<Item = &'q super::queued::QueuedConstruction>,
        catalog: &RuntimeEconomyCatalog,
        market: &mut OutputAbsorptionContext,
        policy: &CityFiscalPolicy,
    ) {
        for project in projects {
            let Some(profile) = catalog.profile_by_runtime_id(project.profile_runtime_id) else {
                continue;
            };
            if profile.inputs.is_empty() {
                market.reserve_candidate(profile);
                continue;
            }
            let zone = if project.commercial {
                crate::simulation::zoning::ZoneType::Commercial
            } else {
                crate::simulation::zoning::ZoneType::Industrial
            };
            let carrier = self.begin_entrance(project.entrance.clone(), None,
                freight_goods_multiplier(profile, zone, self.tuning, self.minute_of_day));
            let prices = self.prices(catalog, zone, project.level, policy, carrier);
            if market_covers_operation_with_input_quotes(
                profile,
                catalog,
                market.market(),
                prices,
                self.tuning.owa_export_price_multiplier,
                |input| self.quote(input, catalog, prices),
            ) {
                self.accept();
            } else {
                self.reject();
            }
            market.reserve_candidate(profile);
        }
    }

    /// Supplies the same non-input tariffs used by private entry and live hiring.
    pub(super) fn prices(
        &self,
        catalog: &RuntimeEconomyCatalog,
        zone: crate::simulation::zoning::ZoneType,
        level: u8,
        policy: &CityFiscalPolicy,
        carrier: Option<f32>,
    ) -> OperationPrices {
        OperationPrices {
            utilities: local_utility_service_costs(catalog)
                .map(|p| p * self.tuning.owa_import_price_multiplier),
            property_tax: daily_property_tax(zone, level, policy),
            imports: self.tuning.owa_import_price_multiplier * if self.goods_multiplier.is_finite() { self.goods_multiplier } else { 1.0 },
            carrier_quote: carrier.unwrap_or(f32::INFINITY),
            truck_payload: self.tuning.logistics.truck_payload_kg,
        }
    }
}

impl ProjectSupply for SiteSupply<'_> {
    fn begin_replacement(&mut self, owner: usize, parcel: u64, asset: &str) -> Option<f32> {
        let carrier = self.begin(parcel, asset);
        self.book.release_owner(owner, self.allocator, self.graph);
        carrier
    }

    fn input_price(
        &self,
        resource: u16,
        _catalog: &RuntimeEconomyCatalog,
        _prices: OperationPrices,
    ) -> f32 {
        self.book.input_price(resource)
    }

    fn begin(&mut self, parcel: u64, asset: &str) -> Option<f32> {
        let entrance = self.allocator.demand_site_entrance(
            parcel,
            asset,
            self.zoning,
            self.graph,
            &self.network.lane_system,
        );
        let profile = candidate_economy_profile(self.allocator, self.catalog, asset);
        let zone = self.allocator.registry.get(asset)
            .and_then(|entry| entry.manifest.building.as_ref())
            .and_then(|building| building.zone_type)
            .map_or(crate::simulation::zoning::ZoneType::None, |zone| match zone {
                crate::assets::asset::ZoneClass::Commercial => crate::simulation::zoning::ZoneType::Commercial,
                crate::assets::asset::ZoneClass::Industrial => crate::simulation::zoning::ZoneType::Industrial,
                _ => crate::simulation::zoning::ZoneType::None,
            });
        let multiplier = profile.map_or(f32::INFINITY,
            |profile| freight_goods_multiplier(profile, zone, self.tuning, self.minute_of_day));
        self.begin_entrance(entrance, None, multiplier)
    }

    fn quote(
        &mut self,
        input: InputRequirement,
        catalog: &RuntimeEconomyCatalog,
        prices: OperationPrices,
    ) -> InputSupplyCosts {
        let Some(entrance) = &self.entrance else {
            return input.imported(
                catalog,
                OperationPrices {
                    carrier_quote: f32::INFINITY,
                    ..prices
                },
            );
        };
        self.logistics.quote_recurring_input(
            self.book,
            input,
            self.buyer,
            entrance,
            self.carrier,
            self.allocator,
            catalog,
            self.tuning,
            self.network,
            self.graph, self.goods_multiplier,
        )
    }

    fn accept(&mut self) {
        self.book.accept();
    }
    fn reject(&mut self) {
        self.book.reject(self.allocator, self.graph);
    }
}

// Isolated selection tests inject a tariff without constructing an entire freight world.
// Runtime callers always use SiteSupply and its finite transaction book.
#[cfg(test)]
impl<F: FnMut(u64, &str) -> Option<f32>> ProjectSupply for F {
    fn begin(&mut self, parcel: u64, asset: &str) -> Option<f32> {
        self(parcel, asset)
    }
    fn quote(
        &mut self,
        input: InputRequirement,
        catalog: &RuntimeEconomyCatalog,
        prices: OperationPrices,
    ) -> InputSupplyCosts {
        input.imported(catalog, prices)
    }
    fn accept(&mut self) {}
    fn reject(&mut self) {}
}
