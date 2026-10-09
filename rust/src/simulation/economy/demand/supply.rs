// SPDX-License-Identifier: GPL-2.0-only

//! Construction input quotations and transaction boundaries over the live residual market.

use super::spawn_need::{
    OutputAbsorptionContext, candidate_economy_profile, constructing_area_covers_operation,
};
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::customers::ConsumerMarket;
use crate::simulation::economy::customers::business::{BusinessMarket, PreparedBusinessSite};
use crate::simulation::economy::definitions::EconomyProfileRuntime;
use crate::simulation::economy::fiscal::{CityFiscalPolicy, daily_property_tax};
use crate::simulation::economy::households::{
    local_utility_service_costs, output_area_scale_for_building,
    physical_worker_capacity_for_profile,
};
use crate::simulation::economy::logistics::{ProjectedSupplySource, RecurringSupply, ShipmentSystem, freight_goods_multiplier};
use rayon::prelude::*;
use crate::simulation::economy::operation::{
    FundedOperation, InputRequirement, InputSupplyCosts, OperatingCosts, OperationPlan, OperationPrices, export_freight,
};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use crate::simulation::zoning::{ParcelId, ZoningSystem};

mod buyers;
mod replacements;
mod storage;
use replacements::{OperationSite, ReplacementProof};
use storage::StorageRequirement;

/// One proposed operation stages inputs until every independent admission check passes.
pub(super) trait ProjectSupply {
    /// Full reachable output claims and their supported activity/net sales, when available.
    fn customers(&mut self, _profile: &EconomyProfileRuntime, _area: f32,
        _prices: OperationPrices, _exports: f32) -> Option<ProjectCustomers> {
        None
    }
    /// Actual mixed local/export value retained by the customer quotation.
    fn output_price(&self, _resource: u16) -> Option<f64> { None }
    /// Staffing established by the current quote, rather than a second market estimate.
    fn supported_workers(&self, fallback: u32) -> u32 { fallback }
    /// Retains exact activity and area alongside the supported rounded workforce.
    fn set_supported_plan(&mut self, _plan: OperationPlan) {}
    /// Equity supplied by placement, independent of this site's transient input quotes.
    fn startup_capital(&self, profile: &EconomyProfileRuntime, catalog: &RuntimeEconomyCatalog,
        prices: OperationPrices) -> f64 {
        f64::from(profile.zoned_startup_budget(catalog, prices.imports))
    }
    /// Records the candidate's own financing proof before dependency/placement commitment.
    fn fund(&mut self, _profile: &EconomyProfileRuntime, _prices: OperationPrices,
        costs: OperatingCosts, capital: f64) -> bool { capital >= costs.working_capital }
    /// Replayed staffing columns; absent only for isolated numeric fixtures.
    fn reservations(&self) -> Option<&ProjectReservations> { None }
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
    ) -> f64 {
        f64::from(catalog.unit_price_for_resource(resource).unwrap_or(0.0) * prices.imports)
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

/// Revenue and activity proved by shared reachable customer reservations.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct ProjectCustomers {
    /// Recipe activity required by the accepted customers or executable exports.
    pub(super) activity: f32,
    /// Seller receipts, excluding VAT and carrier payments.
    pub(super) revenue: f64,
    /// Export carrier cost added to both daily costs and settlement capital.
    pub(super) export_freight: f64,
}

/// Reusable construction customer accounts and staffing proved during ordered replay.
#[derive(Default)]
pub(super) struct ProjectReservations {
    pub(super) customers: ConsumerMarket,
    /// Established buyers' shared quantities and financing after earlier project commitments.
    pub(super) business: BusinessMarket,
    pub(super) building_workers: Vec<u32>,
    pub(super) queued_workers: Vec<(u64, u32)>,
    /// Accepted live/project plans indexed by the same owner as the reservation books.
    pub(super) operations: Vec<Option<FundedOperation>>,
    // Per-proposal output values; prepared for the catalog's largest output recipe.
    output_prices: Vec<(u16, f64)>,
    owner_base: usize,
    staged_owner: Option<usize>,
    staged_operation: Option<FundedOperation>,
    staged_committed: bool,
    sites: Vec<Option<OperationSite>>,
    freight_capital: Vec<f64>,
    staged_site: Option<OperationSite>,
    replacement_proof: ReplacementProof,
    prepared: bool,
}

impl ProjectReservations {
    /// Finds an unplaced operation using the fixed owner namespace retained across appends.
    pub(super) fn project_operation(&self, parcel: u64, zoning: &ZoningSystem) -> Option<FundedOperation> {
        let index = zoning.parcels.storage_index(ParcelId::from_raw(parcel))?;
        self.operations.get(self.owner_base + index).copied().flatten()
    }

    /// Prepared project proof for exact reload comparisons: workers, financing headroom bits
    /// and each recorded input source/quantity/price receipt, in receipt order.
    #[cfg(test)]
    pub(super) fn project_proof_for_test(&self, book: &RecurringSupply, parcel: u64,
        zoning: &ZoningSystem, catalog: &RuntimeEconomyCatalog, tuning: &RuntimeEconomyTuning)
        -> Option<(u32, [u64; 2], Vec<(u16, Option<usize>, u64, u32, u32)>)> {
        let operation = self.project_operation(parcel, zoning)?;
        let owner = self.owner_base + zoning.parcels.storage_index(ParcelId::from_raw(parcel))?;
        let profile = catalog.profile_by_runtime_id(operation.profile)?;
        let mut sources = Vec::new();
        OperatingCosts::for_plan_with_input_quotes(profile, operation.plan, operation.prices,
            |_, _| 0.0, |input| {
                book.visit_input_sources(owner, input,
                    catalog.unit_price_for_resource(input.resource).unwrap_or(0.0) * tuning.owa_import_price_multiplier,
                    Some(operation.prices.carrier_quote), |source, units, goods, carrier|
                        sources.push((input.resource, source, units.to_bits(), goods.to_bits(), carrier.to_bits())));
                Default::default()
            });
        Some((operation.plan.workers, operation.headroom.map(f64::to_bits), sources))
    }

    /// Forks both customer books from the same completed live financing proof.
    pub(super) fn reset(&mut self, funding: &crate::simulation::economy::hiring::JobFunding,
        allocator: &BuildingAllocator, catalog: &RuntimeEconomyCatalog, tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy, minute: u16) {
        self.customers.copy_reservations_from(&funding.customers);
        self.business.copy_reservations_from(&funding.business);
        self.prepared = false;
        self.staged_owner = None;
        self.staged_operation = None;
        self.staged_committed = false;
        self.operations.resize(allocator.buildings.len(), None);
        let utilities = local_utility_service_costs(catalog).map(|p| p * tuning.owa_import_price_multiplier);
        self.operations.par_iter_mut().enumerate().for_each(|(index, operation)| {
            let building = &allocator.buildings[index];
            *operation = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                .and_then(|profile| funding.funded_operation(index, building, profile, utilities, tuning, fiscal, minute));
        });
        self.reset_sites(funding, allocator, catalog, tuning, minute);
    }

    /// Commits the exact staged owner after placement or a planning acceptance succeeds.
    pub(super) fn accept(&mut self, book: &mut RecurringSupply) {
        let Some(owner) = self.staged_owner.take() else { return; };
        assert!(self.staged_operation.is_some(), "accepted project must retain its financing proof");
        assert!(self.staged_committed, "capacity-checked inner commits must precede publication");
        book.accept_revalidation();
        self.business.accept_revalidation();
        self.customers.accept_revalidation();
        self.accept_replacement_finance();
        self.operations[owner] = self.staged_operation.take();
        self.sites[owner] = self.staged_site.take();
        self.staged_committed = false;
    }

    /// Restores all provisional markets after an economic or physical placement rejection.
    pub(super) fn reject(&mut self, book: &mut RecurringSupply,
        allocator: &BuildingAllocator, graph: &RegionGraph) {
        book.reject_revalidation(allocator, graph);
        self.customers.reject_revalidation();
        self.business.reject_revalidation();
        self.staged_owner = None;
        self.staged_operation = None;
        self.staged_committed = false;
        self.staged_site = None;
        self.replacement_proof.clear();
    }
}

/// Runtime quotation context borrowing prepared scratch; it owns no per-project heap storage.
pub(super) struct SiteSupply<'a> {
    book: &'a mut RecurringSupply,
    reservations: &'a mut ProjectReservations,
    vat: f32,
    plan: OperationPlan,
    customer_quote: ProjectCustomers,
    replacement: Option<usize>,
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
    buyer_site: Option<usize>,
    catalog: &'a RuntimeEconomyCatalog,
    minute_of_day: u16,
    goods_multiplier: f32,
    exports: f32,
    replacement_source: Option<usize>,
}

impl<'a> SiteSupply<'a> {
    /// Borrows the pass's residual book after its border route tables have been prepared.
    pub(super) fn new(
        book: &'a mut RecurringSupply,
        reservations: &'a mut ProjectReservations,
        allocator: &'a BuildingAllocator,
        zoning: &'a ZoningSystem,
        network: &'a TransitNetwork,
        graph: &'a RegionGraph,
        logistics: &'a mut ShipmentSystem,
        tuning: &'a RuntimeEconomyTuning,
        borders: &'a [u32],
        catalog: &'a RuntimeEconomyCatalog,
        minute_of_day: u16,
        vat: f32,
    ) -> Self {
        if !reservations.prepared {
            // A pass can replay a queue and select at most one new project per parcel.
            // Existing construction and replacements retain their live building owner.
            let owners = allocator.buildings.len() + 2 * zoning.parcels().len();
            let outputs = catalog.all_profiles().iter().map(|profile| profile.outputs.len()).max().unwrap_or(0);
            let inputs = catalog.all_profiles().iter().map(|profile| profile.inputs.len()).max().unwrap_or(0);
            book.prepare_projected_sources(catalog, graph, [zoning.overlay_revision(),
                zoning.overlay_occupancy_revision(), allocator.registry.revision()], || {
                allocator.buildings.par_iter().enumerate().flat_map_iter(|(owner, building)| {
                    [Some(building.asset_id.as_str()), allocator.registry.prev_level(&building.asset_id), allocator.registry.next_level(&building.asset_id)]
                        .into_iter().flatten().filter_map(move |asset| {
                            let profile = candidate_economy_profile(allocator, catalog, asset)?;
                            if !crate::simulation::economy::logistics::building_outputs_can_supply_local_inputs(building, profile) {
                                return None;
                            }
                            let entrance = allocator.demand_site_entrance(building.parcel_id, asset,
                                zoning, graph, &network.lane_system)?;
                            Some(ProjectedSupplySource::new(owner, asset, profile.runtime_id, entrance))
                        })
                }).collect()
            });
            book.prepare_customer_deliveries(reservations.business.receipt_count());
            book.prepare_owners(owners, owners * inputs);
            if let Some(components) = book.freight_components() {
                reservations.business.prepare_sites([zoning.overlay_revision(),
                    zoning.overlay_occupancy_revision(), allocator.registry.revision()],
                    catalog, components, graph, || {
                        let assets = allocator.collect_spawn_asset_candidates_by_profile(zoning, catalog);
                        let mut sites: Vec<_> = zoning.parcels().par_iter().enumerate().flat_map_iter(|(index, parcel)| {
                            assets.get(&parcel.zone_profile_runtime_id()).into_iter()
                                .flat_map(move |assets| assets.fitting_assets(parcel, zoning))
                                .filter_map(move |asset| {
                                    if !parcel.is_available() { return None; }
                                    let profile = candidate_economy_profile(allocator, catalog, asset)?;
                                    if profile.inputs.is_empty() { return None; }
                                    let entrance = allocator.demand_site_entrance(parcel.id().raw(), asset,
                                        zoning, graph, &network.lane_system)?;
                                    Some(PreparedBusinessSite::new(allocator.buildings.len() + index,
                                        asset, profile.runtime_id, entrance))
                                })
                        }).collect();
                        sites.par_extend(allocator.buildings.par_iter().enumerate().flat_map_iter(|(owner, building)| {
                            [Some(building.asset_id.as_str()), allocator.registry.prev_level(&building.asset_id),
                                allocator.registry.next_level(&building.asset_id)].into_iter().flatten().filter_map(move |asset| {
                                let profile = candidate_economy_profile(allocator, catalog, asset)?;
                                if profile.inputs.is_empty() { return None; }
                                let entrance = allocator.demand_site_entrance(building.parcel_id, asset,
                                    zoning, graph, &network.lane_system)?;
                                Some(PreparedBusinessSite::new(owner, asset, profile.runtime_id, entrance))
                            })
                        }));
                        sites
                    });
            }
            reservations.business.prepare_owners(owners, owners * outputs);
            reservations.business.prepare_project_inputs(book.project_input_tranche_capacity(owners * inputs));
            reservations.customers.prepare_project_owners(owners, owners * outputs);
            reservations.output_prices.reserve(outputs);
            reservations.operations.resize(owners, None);
            reservations.sites.resize(owners, None);
            reservations.replacement_proof.prepare(owners);
            reservations.owner_base = allocator.buildings.len();
            reservations.prepared = true;
        }
        Self {
            book,
            reservations,
            vat,
            plan: OperationPlan { workers: 0, activity: 0.0, area: 1.0 },
            customer_quote: ProjectCustomers::default(),
            replacement: None,
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
            buyer_site: None,
            catalog,
            minute_of_day,
            goods_multiplier: 1.0,
            exports: 0.0,
            replacement_source: None,
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
        self.book.begin_revalidation();
        self.reservations.business.begin_revalidation();
        self.reservations.customers.begin_revalidation();
        self.plan = OperationPlan { workers: 0, activity: 0.0, area: 1.0 };
        self.exports = 0.0;
        self.replacement_source = None;
        self.customer_quote = ProjectCustomers::default();
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
        self.buyer_site = None;
        self.reservations.staged_owner = buyer;
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
        let projects = projects.filter(|project| project.active);
        self.prepare_project_market(projects.clone(), catalog, market, replay, policy,
            service_capacities, freight_capital);
        self.reserve_queued(projects, catalog, replay, policy, |_, _| true);
    }

    /// Execution retains only queue purchases backed by workers from already placed homes.
    /// Both books consume projects in queue order; a labour rejection restores its finances.
    /// Preparation is O(B + (Q + H) log H + quoted ports/routes), once per execution epoch.
    pub(super) fn reserve_execution_projects<'q>(
        &mut self,
        projects: impl Iterator<Item = &'q super::queued::QueuedConstruction> + Clone,
        catalog: &RuntimeEconomyCatalog,
        snapshot: &super::snapshot::DailyDemandSnapshot,
        replay: &mut OutputAbsorptionContext,
        policy: &CityFiscalPolicy,
        service_capacities: &[u32],
        freight_capital: &[f64],
        workforce: &mut super::workforce::WorkforceReservations,
        elapsed: u64,
    ) {
        let projects = projects.filter(|project| project.active);
        self.prepare_project_market(projects.clone(), catalog, &snapshot.output_absorption,
            replay, policy, service_capacities, freight_capital);
        workforce.reset(self.allocator, catalog, self.tuning, snapshot, &[], 0,
            Some(self.reservations));
        workforce.reserve_existing();
        self.reserve_queued(projects, catalog, replay, policy, |project, workers| {
            workforce.claim_by_hour(project.completion_hour.saturating_sub(elapsed), workers)
        });
    }

    fn prepare_project_market<'q>(
        &mut self,
        projects: impl Iterator<Item = &'q super::queued::QueuedConstruction> + Clone,
        catalog: &RuntimeEconomyCatalog,
        market: &OutputAbsorptionContext,
        replay: &mut OutputAbsorptionContext,
        policy: &CityFiscalPolicy,
        service_capacities: &[u32],
        freight_capital: &[f64],
    ) {
        self.reservations.building_workers.resize(self.allocator.buildings.len(), 0);
        self.reservations.building_workers.fill(0);
        self.reservations.queued_workers.clear();
        self.reservations.queued_workers.reserve(projects.clone().count());
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
                        self.set_supported_plan(OperationPlan { workers,
                            activity: workers as f32 / physical.max(1) as f32, area: scale });
                        if !self.fund_operation(profile, prices, costs, f64::INFINITY, true) { break; }
                        self.reservations.building_workers[index] = workers;
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
                    super::spawn_need::replacement_market_covers_operation(
                        profile,
                        catalog,
                        market.market(),
                        prices,
                        self.tuning.owa_export_price_multiplier,
                        building,
                        freight_capital.get(index).copied().unwrap_or(0.0),
                        self,
                    )
                };
            if viable {
                self.reservations.building_workers[index] = self.supported_workers(
                    physical_worker_capacity_for_profile(building, profile));
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
        mut workforce: impl FnMut(&super::queued::QueuedConstruction, u32) -> bool,
    ) {
        for project in projects {
            let Some(profile) = catalog.profile_by_runtime_id(project.profile_runtime_id) else {
                continue;
            };
            let zone = if project.commercial {
                crate::simulation::zoning::ZoneType::Commercial
            } else {
                crate::simulation::zoning::ZoneType::Industrial
            };
            let carrier = self.begin_entrance(project.entrance.clone(), None,
                freight_goods_multiplier(profile, zone, self.tuning, self.minute_of_day));
            self.reservations.staged_owner = self.project_owner(project.parcel_id);
            self.buyer_site = self.reservations.staged_owner.zip(self.entrance.as_ref())
                .and_then(|(owner, entrance)| self.reservations.business
                    .prepared_site_at(owner, profile.runtime_id, entrance));
            let prices = self.prices(catalog, zone, project.level, policy, carrier);
            if super::spawn_need::project_covers_operation(
                profile,
                catalog,
                market.market(),
                prices,
                self.tuning.owa_export_price_multiplier,
                self,
            ) && workforce(project, self.plan.workers) {
                self.reservations.queued_workers.push((project.parcel_id, self.plan.workers));
                self.accept();
            } else {
                self.reservations.queued_workers.push((project.parcel_id, 0));
                self.reject();
            }
            market.reserve_candidate(profile);
        }
        self.reservations.queued_workers.sort_unstable_by_key(|&(parcel, _)| parcel);
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
    fn customers(&mut self, profile: &EconomyProfileRuntime, area: f32,
        prices: OperationPrices, exports: f32) -> Option<ProjectCustomers> {
        self.reservations.staged_operation = None;
        use crate::simulation::economy::definitions::EconomyProfileRuntimeKind;
        let exportable = self.carrier.is_some() && exports > 0.0 && matches!(profile.kind,
            EconomyProfileRuntimeKind::Processor | EconomyProfileRuntimeKind::Producer
            | EconomyProfileRuntimeKind::Extractor | EconomyProfileRuntimeKind::FieldProducer);
        self.exports = if exportable { exports } else { 0.0 };
        let mut quote = ProjectCustomers::default();
        self.reservations.output_prices.clear();
        if let Some(entrance) = &self.entrance {
            for port in &profile.outputs {
                let output = profile.net_output_units_per_day(port) * area;
                if output <= 0.0 { continue; }
                let household = self.reservations.customers.claim_reachable(
                    entrance, port.resource_runtime_id, f64::from(output),
                    profile.unit_price_currency * (1.0 + self.vat),
                    self.allocator, self.network, self.graph);
                let business = if profile.kind != EconomyProfileRuntimeKind::ServiceStore
                    && let Some(components) = self.book.freight_components() {
                    if let Some(owner) = self.replacement {
                        self.reservations.business.claim_reachable_joint(entrance, owner,
                            port.resource_runtime_id, (f64::from(output) - household).max(0.0),
                            profile.unit_price_currency, components, self.allocator, self.logistics,
                            self.network, self.graph, self.tuning)
                    } else { self.reservations.business.claim_reachable(entrance, self.reservations.staged_owner,
                        port.resource_runtime_id, (f64::from(output) - household).max(0.0),
                        profile.unit_price_currency, components, self.allocator, self.logistics,
                        self.network, self.graph, self.tuning) }
                } else { Default::default() };
                let local = household + business.units;
                let exported = if exportable { (f64::from(output) - local).max(0.0) } else { 0.0 };
                let revenue = household * f64::from(profile.unit_price_currency) + business.revenue
                    + exported * f64::from(self.catalog.unit_price_for_resource(port.resource_runtime_id)
                        .unwrap_or(0.0) * exports);
                quote.activity = quote.activity.max(((local + exported) / f64::from(output)) as f32);
                quote.revenue += revenue;
                quote.export_freight += export_freight(profile, port, output, exported as f32, prices);
                self.reservations.output_prices.push((port.resource_runtime_id, revenue / f64::from(output)));
            }
        }
        if self.replacement.is_none() && !self.reservations.business.can_accept() { quote = ProjectCustomers::default(); }
        self.customer_quote = quote;
        Some(quote)
    }
    fn output_price(&self, resource: u16) -> Option<f64> {
        Some(self.reservations.output_prices.iter().find(|&&(id, _)| id == resource).map_or(0.0, |&(_, price)| price))
    }
    fn supported_workers(&self, _fallback: u32) -> u32 { self.plan.workers }
    fn set_supported_plan(&mut self, plan: OperationPlan) {
        self.plan = plan;
        self.reservations.staged_operation = None;
    }
    fn startup_capital(&self, profile: &EconomyProfileRuntime, catalog: &RuntimeEconomyCatalog,
        _prices: OperationPrices) -> f64 {
        f64::from(profile.zoned_startup_budget(catalog, self.tuning.owa_import_price_multiplier))
    }
    fn fund(&mut self, profile: &EconomyProfileRuntime, prices: OperationPrices,
        costs: OperatingCosts, capital: f64) -> bool {
        self.fund_operation(profile, prices, costs, capital, false)
    }

    fn reservations(&self) -> Option<&ProjectReservations> { Some(self.reservations) }
    fn begin_replacement(&mut self, owner: usize, parcel: u64, asset: &str) -> Option<f32> {
        let carrier = self.begin(parcel, asset);
        self.withdraw_replacement(owner);
        self.book.release_owner(owner, self.allocator, self.graph);
        self.reservations.customers.release_owner(owner);
        self.reservations.business.release_owner(owner);
        self.reservations.staged_owner = Some(owner);
        self.replacement = Some(owner);
        self.buyer_site = self.reservations.business.prepared_site(owner, asset);
        self.replacement_source = self.book.projected_source(owner, asset);
        carrier
    }

    fn input_price(
        &self,
        resource: u16,
        _catalog: &RuntimeEconomyCatalog,
        _prices: OperationPrices,
    ) -> f64 {
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
        let carrier = self.begin_entrance(entrance, None, multiplier);
        self.reservations.staged_owner = self.project_owner(parcel);
        self.buyer_site = self.reservations.staged_owner
            .and_then(|owner| self.reservations.business.prepared_site(owner, asset));
        carrier
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
        self.reservations.accept(self.book);
        self.replacement = None;
    }
    fn reject(&mut self) {
        self.reservations.reject(self.book, self.allocator, self.graph);
        self.replacement = None;
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
