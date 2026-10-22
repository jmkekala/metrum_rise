// SPDX-License-Identifier: GPL-2.0-only

//! Construction input quotations and transaction boundaries over the live residual market.

use super::spawn_need::{
    OutputAbsorptionContext, candidate_economy_profile, constructing_area_covers_operation,
};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::customers::ConsumerMarket;
use crate::simulation::economy::customers::business::{BusinessMarket, PreparedBusinessSite};
use crate::simulation::economy::definitions::EconomyProfileRuntime;
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::fiscal::{CityFiscalPolicy, daily_property_tax};
use crate::simulation::economy::hiring::JobFunding;
use crate::simulation::economy::households::{
    local_utility_service_costs, output_area_scale_for_building,
    physical_worker_capacity_for_profile,
};
use crate::simulation::economy::logistics::{
    ProjectedSupplySource, RecurringSupply, ShipmentSystem, freight_goods_multiplier,
};
use crate::simulation::economy::operation::{
    FundedOperation, InputRequirement, InputSupplyCosts, OperatingCosts, OperationPlan,
    OperationPrices, export_freight,
};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use crate::simulation::zoning::{ParcelId, ZoningSystem};
use rayon::prelude::*;

mod buyers;
mod replacements;
mod site_rows;
mod storage;
use replacements::{OperationSite, ReplacementProof};
use storage::StorageRequirement;

/// Whether planning treats a building as under construction: it is, or the census did not see
/// it complete (`!census_complete`, `SIM-02-I`), so a completion after the census's copy keeps
/// its constructing claims until a census funds its jobs.
pub(super) fn constructing(building: &Building, census_complete: bool) -> bool {
    building.is_under_construction() || !census_complete
}

/// One proposed operation stages inputs until every independent admission check passes.
pub(super) trait ProjectSupply {
    /// Full reachable output claims and their supported activity/net sales, when available.
    fn customers(
        &mut self,
        _profile: &EconomyProfileRuntime,
        _area: f32,
        _prices: OperationPrices,
        _exports: f32,
    ) -> Option<ProjectCustomers> {
        None
    }
    /// Actual mixed local/export value retained by the customer quotation.
    fn output_price(&self, _resource: u16) -> Option<f64> {
        None
    }
    /// Staffing established by the current quote, rather than a second market estimate.
    fn supported_workers(&self, fallback: u32) -> u32 {
        fallback
    }
    /// Most workers a store's current probe may plan for (`DEM-02-C`); unlimited by default.
    /// Reset by every `begin`, so only an explicit retry at the available crew sets it.
    fn crew_cap(&self) -> u32 {
        u32::MAX
    }
    /// Limits the current probe's store crew to the workers the calendar can supply.
    fn set_crew_cap(&mut self, _workers: u32) {}
    /// Retains exact activity and area alongside the supported rounded workforce.
    fn set_supported_plan(&mut self, _plan: OperationPlan) {}
    /// Equity supplied by placement, independent of this site's transient input quotes.
    fn startup_capital(
        &self,
        profile: &EconomyProfileRuntime,
        catalog: &RuntimeEconomyCatalog,
        prices: OperationPrices,
    ) -> f64 {
        f64::from(profile.zoned_startup_budget(catalog, prices.imports))
    }
    /// Records the candidate's own financing proof before dependency/placement commitment.
    fn fund(
        &mut self,
        _profile: &EconomyProfileRuntime,
        _prices: OperationPrices,
        costs: OperatingCosts,
        capital: f64,
    ) -> bool {
        capital >= costs.working_capital
    }
    /// Replayed staffing columns; absent only for isolated numeric fixtures.
    fn reservations(&self) -> Option<&ProjectReservations> {
        None
    }
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
    // The constructing buildings a pass replays, in building order; reused scratch.
    constructing: Vec<usize>,
    /// Accepted live/project plans indexed by the same owner as the reservation books.
    pub(super) operations: Vec<Option<FundedOperation>>,
    // Only accepted project slots need clearing between passes; unused slots stay empty.
    project_owners: Vec<usize>,
    live_owners: usize,
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
    // The owner slots of free lots for the storage estimates, under the parcel revisions it
    // was counted at (`SIM-02-K`).
    free_lot_estimate: Option<([u64; 3], usize)>,
    prepared: bool,
    // What the live operation and site rows were derived from at the last pass's start, so
    // the next lending of the same book recomputes only what changed (`SIM-02-K`).
    kept_rows: Option<KeptPassRows>,
    // Live owners whose rows a pass overwrote (an accepted replacement, its affected
    // financing), recomputed by the next pass's start.
    touched_live: Vec<usize>,
    // Rows a kept pass start recomputes; reused scratch.
    changed_rows: Vec<usize>,
    // Set by a split minute-59 preparation: the next site preparation prepares the supplier
    // rows and their layout alone and leaves the rest unprepared (`SIM-02-K`). Cleared by it.
    pub(super) suppliers_only: bool,
    // Set with `suppliers_only` by the preparation's first part: the supplier rows alone,
    // without their layout, which the next part lays out (`SIM-02-K`). Cleared by it.
    pub(super) supplier_rows_only: bool,
    // Pass starts that kept the previous rows, for tests.
    #[cfg(test)]
    pub(super) kept_row_starts: usize,
}

// The inputs of every live operation and site row besides the buildings' own (`SIM-02-K`):
// the lent book's funded rows, the freight price phase of the pass's minute, the catalog,
// tuning and fiscal policy. Catalog and tuning are compared by address: a reload builds new
// ones, and a census of their book (a new rows generation) precedes any lending on them.
#[derive(Clone, Copy, PartialEq)]
struct PassRowsKey {
    funding: u64,
    phase: u16,
    catalog: usize,
    tuning: usize,
    fiscal: CityFiscalPolicy,
}

// Kept rows: their key, the building and entrance revisions they cover and the live count.
#[derive(Clone, Copy)]
struct KeptPassRows {
    key: PassRowsKey,
    buildings: u64,
    entrances: u64,
    live: usize,
}

/// Prepared project proof: workers, financing headroom bits and input receipts.
#[cfg(test)]
pub(crate) type ProjectProof = (u32, [u64; 2], Vec<(u16, Option<usize>, u64, u32, u32)>);

impl ProjectReservations {
    /// The constructing buildings the pass's project market replayed, in building order: every
    /// building [`constructing`] selects for the census the pass lent.
    pub(super) fn constructing_rows(&self) -> &[usize] {
        &self.constructing
    }

    /// Finds an unplaced operation using the fixed owner namespace retained across appends.
    #[cfg(test)]
    pub(super) fn project_operation(
        &self,
        parcel: u64,
        zoning: &ZoningSystem,
    ) -> Option<FundedOperation> {
        let index = zoning.parcels.storage_index(ParcelId::from_raw(parcel))?;
        self.operations
            .get(self.owner_base + index)
            .copied()
            .flatten()
    }

    /// Prepared project proof for exact reload comparisons: workers, financing headroom bits
    /// and each recorded input source/quantity/price receipt, in receipt order.
    #[cfg(test)]
    pub(super) fn project_proof_for_test(
        &self,
        book: &RecurringSupply,
        parcel: u64,
        zoning: &ZoningSystem,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> Option<ProjectProof> {
        let operation = self.project_operation(parcel, zoning)?;
        let owner = self.owner_base + zoning.parcels.storage_index(ParcelId::from_raw(parcel))?;
        let profile = catalog.profile_by_runtime_id(operation.profile)?;
        let mut sources = Vec::new();
        OperatingCosts::for_plan_with_input_quotes(
            profile,
            operation.plan,
            operation.prices,
            |_, _| 0.0,
            |input| {
                book.visit_input_sources(
                    owner,
                    input,
                    catalog
                        .unit_price_for_resource(input.resource)
                        .unwrap_or(0.0)
                        * tuning.owa_import_price_multiplier,
                    Some(operation.prices.carrier_quote),
                    |source, units, goods, carrier| {
                        sources.push((
                            input.resource,
                            source,
                            units.to_bits(),
                            goods.to_bits(),
                            carrier.to_bits(),
                        ))
                    },
                );
                Default::default()
            },
        );
        Some((
            operation.plan.workers,
            operation.headroom.map(f64::to_bits),
            sources,
        ))
    }

    /// Forks both customer books from the same completed financing proof, for tests: planning
    /// lends the live books instead.
    #[cfg(test)]
    pub(super) fn reset(
        &mut self,
        funding: &crate::simulation::economy::hiring::JobFunding,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        minute: u16,
    ) {
        self.customers.copy_reservations_from(&funding.customers);
        self.business.copy_reservations_from(&funding.business);
        self.prepare_pass_rows(funding, allocator, catalog, tuning, fiscal, minute);
    }

    /// Opens a planning pass on live customer books already moved into this owner, without
    /// copying them. The caller must close it with [`Self::end_live_pass`] before returning
    /// the books. Opening is O(1) amortized beyond the per-pass operation rows.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_live_pass(
        &mut self,
        funding: &crate::simulation::economy::hiring::JobFunding,
        inputs: &mut RecurringSupply,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        minute: u16,
    ) {
        inputs.begin_pass();
        // Buildings appended since the census's copy are live sources without supply, and
        // buyers without accounts at ports laid out after the census's (`SIM-02-I`).
        inputs.cover_live_buildings(allocator.buildings.len());
        if let Some(components) = inputs.freight_components() {
            self.business
                .append_live_buyers(allocator, graph, components, catalog);
        }
        self.begin_book_pass(funding, allocator, catalog, tuning, fiscal, minute);
    }

    /// Opens a pass on lent customer and business books only. Their rollback restores rows
    /// without reading the allocator, so it stays exact across later building edits.
    pub(super) fn begin_book_pass(
        &mut self,
        funding: &crate::simulation::economy::hiring::JobFunding,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        minute: u16,
    ) {
        self.customers.begin_pass();
        self.business.begin_pass();
        self.prepare_pass_rows(funding, allocator, catalog, tuning, fiscal, minute);
    }

    /// Restores the live books exactly as [`Self::begin_live_pass`] found them, in
    /// O(rows the pass touched). Temporary project rows are truncated.
    pub(super) fn end_live_pass(
        &mut self,
        inputs: &mut RecurringSupply,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) {
        self.reject(inputs, allocator, graph);
        self.end_book_pass();
        self.business.restore_census_buyers();
        inputs.end_pass(allocator, graph);
    }

    /// Closes [`Self::begin_book_pass`], restoring both lent books in O(rows touched).
    pub(super) fn end_book_pass(&mut self) {
        self.business.end_pass();
        self.customers.end_pass();
        self.prepared = false;
    }

    // Per-pass owner rows; neither book is copied here. A lending of the book the previous
    // pass started on, at the same price phase, catalog, tuning and policy, recomputes only
    // the rows of buildings the allocator's change log names since then, buildings appended
    // since and rows that pass overwrote, plus every site after an entrance change:
    // O(changed) then, instead of O(B/p) (`SIM-02-K`). Every row is a function of its own
    // building, entrance and those inputs, so the rows equal a full recomputation.
    fn prepare_pass_rows(
        &mut self,
        funding: &crate::simulation::economy::hiring::JobFunding,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        minute: u16,
    ) {
        self.prepared = false;
        self.staged_owner = None;
        self.staged_operation = None;
        self.staged_committed = false;
        let live = allocator.buildings.len();
        let key = PassRowsKey {
            funding: funding.rows_generation(),
            phase: crate::simulation::economy::logistics::freight_price_phase(tuning, minute),
            catalog: std::ptr::from_ref(catalog) as usize,
            tuning: std::ptr::from_ref(tuning) as usize,
            fiscal: *fiscal,
        };
        let kept = self
            .kept_rows
            .take()
            .filter(|kept| kept.key == key)
            .filter(|kept| {
                allocator.building_changes_since(kept.buildings, &mut self.changed_rows)
            });
        self.reset_owner_slots(live);
        let utilities =
            local_utility_service_costs(catalog).map(|p| p * tuning.owa_import_price_multiplier);
        let operation_row = |index: usize| {
            let building = &allocator.buildings[index];
            catalog
                .profile_by_runtime_id(building.economy_profile_runtime_id)
                .and_then(|profile| {
                    funding.funded_operation(
                        index, building, profile, utilities, tuning, fiscal, minute,
                    )
                })
        };
        if let Some(kept) = &kept {
            self.changed_rows.append(&mut self.touched_live);
            self.changed_rows.extend(kept.live.min(live)..live);
            self.changed_rows.retain(|&row| row < live);
            self.changed_rows.sort_unstable();
            self.changed_rows.dedup();
        }
        // A long change set recomputes in full, on the pool.
        let changed = kept.filter(|_| self.changed_rows.len() <= (live / 8).max(64));
        self.touched_live.clear();
        match changed {
            Some(kept) => {
                #[cfg(test)]
                {
                    self.kept_row_starts += 1;
                }
                for &index in &self.changed_rows {
                    self.operations[index] = operation_row(index);
                }
                if kept.entrances == allocator.entrance_ref_revision() {
                    for &owner in &self.changed_rows {
                        self.sites[owner] =
                            Self::site_row(allocator, catalog, tuning, minute, owner);
                    }
                    self.prepare_site_scratch(funding);
                } else {
                    self.reset_sites(funding, allocator, catalog, tuning, minute);
                }
            }
            None => {
                self.operations[..live]
                    .par_iter_mut()
                    .enumerate()
                    .for_each(|(index, operation)| *operation = operation_row(index));
                self.reset_sites(funding, allocator, catalog, tuning, minute);
            }
        }
        self.kept_rows = Some(KeptPassRows {
            key,
            buildings: allocator.building_ref_revision(),
            entrances: allocator.entrance_ref_revision(),
            live,
        });
        #[cfg(test)]
        if crate::debug::work::city_checks() {
            self.assert_rows_current(&operation_row, allocator, catalog, tuning, minute);
        }
    }

    // Test builds: the kept rows equal a full recomputation, bit for bit.
    #[cfg(test)]
    fn assert_rows_current(
        &self,
        operation_row: &(impl Fn(usize) -> Option<FundedOperation> + Sync),
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        minute: u16,
    ) {
        (0..allocator.buildings.len())
            .into_par_iter()
            .for_each(|index| {
                assert_eq!(
                    self.operations[index].as_ref().map(operation_bits),
                    operation_row(index).as_ref().map(operation_bits),
                    "kept operation row {index} went stale"
                );
                assert!(
                    site_matches(
                        self.sites[index].as_ref(),
                        Self::site_row(allocator, catalog, tuning, minute, index).as_ref()
                    ),
                    "kept site row {index} went stale"
                );
            });
    }

    // Retain initialized backing rows. The old live suffix and accepted projects are
    // the only rows that can become stale, even when the owner namespace shifts.
    fn reset_owner_slots(&mut self, live: usize) {
        for owner in self.project_owners.drain(..) {
            self.operations[owner] = None;
            self.sites[owner] = None;
        }
        if live < self.live_owners {
            self.operations[live..self.live_owners]
                .par_iter_mut()
                .for_each(|row| *row = None);
            self.sites[live..self.live_owners]
                .par_iter_mut()
                .for_each(|row| *row = None);
        }
        crate::utils::storage::grow_rows(&mut self.operations, live, None);
        crate::utils::storage::grow_rows(&mut self.sites, live, None);
        self.live_owners = live;
    }

    /// Commits the exact staged owner after placement or a planning acceptance succeeds.
    pub(super) fn accept(&mut self, book: &mut RecurringSupply) {
        let Some(owner) = self.staged_owner.take() else {
            return;
        };
        assert!(
            self.staged_operation.is_some(),
            "accepted project must retain its financing proof"
        );
        assert!(
            self.staged_committed,
            "capacity-checked inner commits must precede publication"
        );
        book.accept_revalidation();
        self.business.accept_revalidation();
        self.customers.accept_revalidation();
        self.accept_replacement_finance();
        if owner >= self.owner_base && self.operations[owner].is_none() {
            self.project_owners.push(owner);
        } else if owner < self.owner_base {
            self.touched_live.push(owner);
        }
        self.operations[owner] = self.staged_operation.take();
        self.sites[owner] = self.staged_site.take();
        self.staged_committed = false;
    }

    /// Restores all provisional markets after an economic or physical placement rejection.
    pub(super) fn reject(
        &mut self,
        book: &mut RecurringSupply,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) {
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
    crew_cap: u32,
}

impl<'a> SiteSupply<'a> {
    /// Borrows the pass's residual book after its border route tables have been prepared.
    #[allow(clippy::too_many_arguments)]
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
            let mut phase = crate::debug::Channel::EconomyPhases
                .enabled()
                .then(std::time::Instant::now);
            let mut lap = || {
                phase.as_mut().map_or(0.0, |previous| {
                    let elapsed = previous.elapsed().as_secs_f64() * 1_000.0;
                    *previous = std::time::Instant::now();
                    elapsed
                })
            };
            // A pass can replay a queue and select at most one new project per parcel.
            // Existing construction and replacements retain their live building owner.
            let owners = allocator.buildings.len() + 2 * zoning.parcels().len();
            // New projects open only on free lots, so the warm-start storage estimates count
            // their owner slots alone (`SIM-02-K`); the census book already holds the live
            // buildings' rows, and a replacement grows through the exact preflight
            // (`supply/storage.rs`). Estimates never change a result, so the count is kept
            // while the parcels' layout and occupancy revisions hold. O(parcels / p) after a
            // change, O(1) otherwise.
            let parcels = [
                zoning.parcel_layout_revision(),
                zoning.overlay_occupancy_revision(),
                zoning.parcels().len() as u64,
            ];
            let project_owners = match reservations.free_lot_estimate {
                Some((key, count)) if key == parcels => count,
                _ => {
                    let count = 2 * zoning
                        .parcels()
                        .par_iter()
                        .filter(|parcel| parcel.is_available())
                        .count();
                    reservations.free_lot_estimate = Some((parcels, count));
                    count
                }
            };
            crate::debug::work::add(crate::debug::work::Work::OwnerSlotsPrepared, owners as u64);
            let outputs = catalog
                .all_profiles()
                .iter()
                .map(|profile| profile.outputs.len())
                .max()
                .unwrap_or(0);
            let inputs = catalog
                .all_profiles()
                .iter()
                .map(|profile| profile.inputs.len())
                .max()
                .unwrap_or(0);
            let key = site_rows::site_rows_key(allocator, zoning, network);
            let mut changes = site_rows::SiteRowChanges::default();
            let base = allocator.buildings.len();
            let parcel_count = zoning.parcels().len();
            let entrance = |parcel: u64,
                            asset: &str,
                            previous: Option<(
                crate::simulation::buildings::allocator::DemandSiteEntranceKey,
                &BuildingEntrance,
            )>| {
                allocator.demand_site_entrance_reusing(
                    parcel,
                    asset,
                    zoning,
                    graph,
                    &network.lane_system,
                    network.cch_graph.build_generation,
                    previous,
                )
            };
            // Variants a live building could become, in this building's own recipe order.
            let variants = |building: usize| {
                [
                    Some(allocator.buildings[building].asset_id.as_str()),
                    allocator.downgrade_target(&allocator.buildings[building]),
                    allocator.upgrade_target(&allocator.buildings[building]),
                ]
                .into_iter()
                .flatten()
            };
            let suppliers = |owner: usize,
                             previous: &[ProjectedSupplySource],
                             out: &mut Vec<ProjectedSupplySource>| {
                let building = &allocator.buildings[owner];
                out.extend(variants(owner).filter_map(|asset| {
                    let profile = candidate_economy_profile(allocator, catalog, asset)?;
                    if !crate::simulation::economy::logistics::building_outputs_can_supply_local_inputs(building, profile) {
                        return None;
                    }
                    let (key, entrance) = entrance(building.parcel_id, asset,
                        ProjectedSupplySource::retained_entrance(previous, owner, asset))?;
                    Some(ProjectedSupplySource::new(owner, asset, profile.runtime_id, entrance).with_entrance_key(key))
                }));
            };
            let factory = |previous: Vec<ProjectedSupplySource>, previous_key: Option<&[u64]>| {
                let incremental = changes.collect(previous_key, &key, allocator, zoning);
                let rows = if incremental {
                    changes.merge(
                        previous,
                        base,
                        parcel_count,
                        ProjectedSupplySource::owner,
                        ProjectedSupplySource::set_owner,
                        ProjectedSupplySource::key,
                        |owner, held| {
                            collect_rows(|out| {
                                if owner < base {
                                    suppliers(owner, held, out)
                                }
                            })
                        },
                    )
                } else {
                    let mut rows =
                        par_collect_rows(base, |owner, out| suppliers(owner, &previous, out));
                    rows.sort_unstable_by(|a, b| {
                        a.owner().cmp(&b.owner()).then(a.key().cmp(b.key()))
                    });
                    rows
                };
                crate::debug::work::add(
                    crate::debug::work::Work::SupplierSourcesBuilt,
                    if incremental {
                        changes.changed_owners()
                    } else {
                        rows.len()
                    } as u64,
                );
                rows
            };
            if std::mem::take(&mut reservations.supplier_rows_only) {
                book.prepare_projected_rows(&key, factory);
            } else {
                book.prepare_projected_sources(catalog, graph, &key, factory);
            }
            let suppliers_ms = lap();
            book.prepare_customer_deliveries(reservations.business.receipt_count());
            book.prepare_owners(owners, project_owners * inputs);
            let supply_storage_ms = lap();
            let suppliers_only = std::mem::take(&mut reservations.suppliers_only);
            if let Some(components) = book.freight_components()
                && !suppliers_only
            {
                let assets = std::sync::OnceLock::new();
                // Previous rows are looked up under `old_owner`, their numbering when prepared.
                let parcel_buyers =
                    |index: usize,
                     previous: &[PreparedBusinessSite],
                     old_owner: usize,
                     out: &mut Vec<PreparedBusinessSite>| {
                        let parcel = &zoning.parcels()[index];
                        if !parcel.is_available() {
                            return;
                        }
                        let assets = assets.get_or_init(|| {
                            allocator.collect_spawn_asset_candidates_by_profile(zoning, catalog)
                        });
                        out.extend(
                            assets
                                .get(&parcel.zone_profile_runtime_id())
                                .into_iter()
                                .flat_map(|assets| assets.fitting_assets(parcel, zoning))
                                .filter_map(|asset| {
                                    let profile =
                                        candidate_economy_profile(allocator, catalog, asset)?;
                                    if profile.inputs.is_empty() {
                                        return None;
                                    }
                                    let (key, entrance) = entrance(
                                        parcel.id().raw(),
                                        asset,
                                        PreparedBusinessSite::retained_entrance(
                                            previous, old_owner, asset,
                                        ),
                                    )?;
                                    Some(
                                        PreparedBusinessSite::new(
                                            base + index,
                                            asset,
                                            profile.runtime_id,
                                            entrance,
                                        )
                                        .with_entrance_key(key),
                                    )
                                }),
                        );
                    };
                let building_buyers =
                    |owner: usize,
                     previous: &[PreparedBusinessSite],
                     out: &mut Vec<PreparedBusinessSite>| {
                        let building = &allocator.buildings[owner];
                        out.extend(variants(owner).filter_map(|asset| {
                            let profile = candidate_economy_profile(allocator, catalog, asset)?;
                            if profile.inputs.is_empty() {
                                return None;
                            }
                            let (key, entrance) = entrance(
                                building.parcel_id,
                                asset,
                                PreparedBusinessSite::retained_entrance(previous, owner, asset),
                            )?;
                            Some(
                                PreparedBusinessSite::new(
                                    owner,
                                    asset,
                                    profile.runtime_id,
                                    entrance,
                                )
                                .with_entrance_key(key),
                            )
                        }));
                    };
                reservations.business.prepare_sites(
                    &key,
                    catalog,
                    components,
                    graph,
                    |previous, previous_key| {
                        let incremental = changes.collect(previous_key, &key, allocator, zoning);
                        let rows = if incremental {
                            let old_base = previous_key.map_or(base, |key| key[7] as usize);
                            changes.merge(
                                previous,
                                base,
                                parcel_count,
                                PreparedBusinessSite::owner,
                                PreparedBusinessSite::set_owner,
                                PreparedBusinessSite::key,
                                |owner, held| {
                                    collect_rows(|out| {
                                        if owner < base {
                                            building_buyers(owner, held, out)
                                        } else {
                                            parcel_buyers(
                                                owner - base,
                                                held,
                                                old_base + owner - base,
                                                out,
                                            )
                                        }
                                    })
                                },
                            )
                        } else {
                            let mut rows = par_collect_rows(parcel_count, |index, out| {
                                parcel_buyers(index, &previous, base + index, out)
                            });
                            rows.append(&mut par_collect_rows(base, |owner, out| {
                                building_buyers(owner, &previous, out)
                            }));
                            rows.sort_unstable_by(|a, b| {
                                a.owner().cmp(&b.owner()).then(a.key().cmp(b.key()))
                            });
                            rows
                        };
                        crate::debug::work::add(
                            crate::debug::work::Work::BuyerSitesBuilt,
                            if incremental {
                                changes.changed_owners()
                            } else {
                                rows.len()
                            } as u64,
                        );
                        rows
                    },
                );
            }
            let buyers_ms = lap();
            reservations
                .business
                .prepare_owners(owners, project_owners * outputs);
            reservations.business.prepare_project_inputs(
                book.project_input_tranche_capacity(project_owners * inputs),
            );
            reservations
                .customers
                .prepare_project_owners(owners, project_owners * outputs);
            reservations.output_prices.reserve(outputs);
            // Owner rows follow the city, with spare capacity (`SIM-02-K`).
            crate::utils::storage::grow_rows(&mut reservations.operations, owners, None);
            crate::utils::storage::grow_rows(&mut reservations.sites, owners, None);
            reservations
                .project_owners
                .reserve(owners - allocator.buildings.len());
            reservations.replacement_proof.prepare(owners);
            reservations.owner_base = allocator.buildings.len();
            reservations.prepared = !suppliers_only;
            let owner_storage_ms = lap();
            crate::debug_log!(
                "economy_phases",
                "site_preparation_detail suppliers_ms={:.3} supply_storage_ms={:.3} buyers_ms={:.3} owner_storage_ms={:.3}",
                suppliers_ms,
                supply_storage_ms,
                buyers_ms,
                owner_storage_ms
            );
        }
        Self {
            book,
            reservations,
            vat,
            plan: OperationPlan {
                workers: 0,
                activity: 0.0,
                area: 1.0,
            },
            crew_cap: u32::MAX,
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
        self.plan = OperationPlan {
            workers: 0,
            activity: 0.0,
            area: 1.0,
        };
        self.crew_cap = u32::MAX;
        self.exports = 0.0;
        self.replacement_source = None;
        self.customer_quote = ProjectCustomers::default();
        self.goods_multiplier = goods_multiplier;
        self.carrier = entrance
            .as_ref()
            .and_then(|entrance| {
                self.logistics.site_carrier_quote(
                    entrance,
                    self.network,
                    self.graph,
                    self.borders,
                    self.tuning,
                )
            })
            .filter(|_| goods_multiplier.is_finite());
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
    #[allow(clippy::too_many_arguments)]
    pub(super) fn reserve_projects<'q>(
        &mut self,
        projects: impl Iterator<Item = &'q super::queued::QueuedConstruction> + Clone,
        catalog: &RuntimeEconomyCatalog,
        market: &OutputAbsorptionContext,
        replay: &mut OutputAbsorptionContext,
        policy: &CityFiscalPolicy,
        service_capacities: &[u32],
        freight_capital: &[f64],
        census_complete: impl Fn(usize) -> bool + Sync,
    ) {
        let projects = projects.filter(|project| project.active);
        self.prepare_project_market(
            projects.clone(),
            catalog,
            market,
            replay,
            policy,
            service_capacities,
            freight_capital,
            &census_complete,
        );
        self.reserve_queued(projects, catalog, replay, policy, |_, _| Ok(()));
    }

    /// The construction epoch's proof of the whole queue (`SIM-02-I`): constructing buildings,
    /// then queued projects in queue order, each claiming workers by its completion from
    /// placed homes and, as the planner counts them, `queued` homes. Both books consume
    /// projects in queue order; a labour rejection restores its finances. Preparation is
    /// O(B + (Q + H) log H + quoted ports/routes), once per planning slot.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn reserve_execution_projects<'q>(
        &mut self,
        projects: impl Iterator<Item = &'q super::queued::QueuedConstruction> + Clone,
        catalog: &RuntimeEconomyCatalog,
        snapshot: &super::snapshot::DailyDemandSnapshot,
        replay: &mut OutputAbsorptionContext,
        policy: &CityFiscalPolicy,
        census: &JobFunding,
        workforce: &mut super::workforce::WorkforceReservations,
        queued: &[super::queued::QueuedConstruction],
        new_start_hour: u64,
        elapsed: u64,
    ) {
        let projects = projects.filter(|project| project.active);
        self.prepare_project_market(
            projects.clone(),
            catalog,
            &snapshot.output_absorption,
            replay,
            policy,
            &census.service_capacities,
            &census.freight_capital,
            &|index| census.saw_operating(index),
        );
        workforce.reset(
            self.allocator,
            catalog,
            self.tuning,
            snapshot,
            queued,
            new_start_hour,
            Some(self.reservations),
            |index| census.saw_operating(index),
        );
        workforce.reserve_existing();
        self.reserve_queued(projects, catalog, replay, policy, |project, workers| {
            let deadline = project.completion_hour.saturating_sub(elapsed);
            if workforce.claim_by_hour(deadline, workers) {
                Ok(())
            } else {
                Err(workforce.available_by_hour(deadline))
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_project_market<'q>(
        &mut self,
        projects: impl Iterator<Item = &'q super::queued::QueuedConstruction> + Clone,
        catalog: &RuntimeEconomyCatalog,
        market: &OutputAbsorptionContext,
        replay: &mut OutputAbsorptionContext,
        policy: &CityFiscalPolicy,
        service_capacities: &[u32],
        freight_capital: &[f64],
        census_complete: &(dyn Fn(usize) -> bool + Sync),
    ) {
        self.reservations
            .building_workers
            .resize(self.allocator.buildings.len(), 0);
        self.reservations.building_workers.fill(0);
        self.reservations.queued_workers.clear();
        self.reservations
            .queued_workers
            .reserve(projects.clone().count());
        replay.copy_from(market);
        // The constructing buildings, found in parallel and kept in building order for the
        // ordered updates below and the replay (`SIM-02-K`): O(B / p) plus O(constructing).
        let allocator = self.allocator;
        let mut rows = std::mem::take(&mut self.reservations.constructing);
        rows.clear();
        rows.par_extend(
            (0..allocator.buildings.len())
                .into_par_iter()
                .filter(|&index| constructing(&allocator.buildings[index], census_complete(index))),
        );
        // Ordered capacity updates share resource totals, like the subsequent finite claims.
        for &index in &rows {
            let building = &allocator.buildings[index];
            if !allocator.is_city_service_building(building)
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
        self.reserve_constructing(
            &rows,
            catalog,
            replay,
            policy,
            service_capacities,
            freight_capital,
        );
        self.reservations.constructing = rows;
    }

    // Replays the constructing buildings `rows`, in building order.
    #[allow(clippy::too_many_arguments)]
    fn reserve_constructing(
        &mut self,
        rows: &[usize],
        catalog: &RuntimeEconomyCatalog,
        market: &mut OutputAbsorptionContext,
        policy: &CityFiscalPolicy,
        service_capacities: &[u32],
        freight_capital: &[f64],
    ) {
        for &index in rows {
            let building = &self.allocator.buildings[index];
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            let service = self.allocator.is_city_service_building(building);
            let carrier = self.begin_entrance(
                self.allocator.entrances.get(index).cloned(),
                Some(index),
                freight_goods_multiplier(
                    profile,
                    building.zone_type,
                    self.tuning,
                    self.minute_of_day,
                ),
            );
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
                        self.set_supported_plan(OperationPlan {
                            workers,
                            activity: workers as f32 / physical.max(1) as f32,
                            area: scale,
                        });
                        if !self.fund_operation(profile, prices, costs, f64::INFINITY, true) {
                            break;
                        }
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
                self.reservations.building_workers[index] =
                    self.supported_workers(physical_worker_capacity_for_profile(building, profile));
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
        mut workforce: impl FnMut(&super::queued::QueuedConstruction, u32) -> Result<(), u32>,
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
            let mut claimed = self
                .probe_queued(project, profile, zone, catalog, market, policy, None)
                .then(|| workforce(project, self.plan.workers));
            // The planner's rule again: a store short only of workers revalidates with the
            // crew the calendar can still house (`DEM-02-C`). No queue state is saved for it.
            if let Some(Err(available)) = claimed
                && project.commercial
                && available > 0
            {
                claimed = self
                    .probe_queued(
                        project,
                        profile,
                        zone,
                        catalog,
                        market,
                        policy,
                        Some(available),
                    )
                    .then(|| workforce(project, self.plan.workers));
            }
            let crew_cap = self.crew_cap;
            if matches!(claimed, Some(Ok(()))) {
                self.reservations
                    .queued_workers
                    .push((project.parcel_id, self.plan.workers));
                self.accept();
            } else {
                self.reservations
                    .queued_workers
                    .push((project.parcel_id, 0));
                self.reject();
            }
            if crew_cap < profile.worker_capacity {
                market.reserve_scaled_candidate(
                    profile,
                    crew_cap as f32 / profile.worker_capacity as f32,
                );
            } else {
                market.reserve_candidate(profile);
            }
        }
        self.reservations
            .queued_workers
            .sort_unstable_by_key(|&(parcel, _)| parcel);
    }

    // Starts a queued project's probe at its prepared door and checks its operation, with the
    // store crew capped when `crew_cap` is set.
    #[allow(clippy::too_many_arguments)]
    fn probe_queued(
        &mut self,
        project: &super::queued::QueuedConstruction,
        profile: &EconomyProfileRuntime,
        zone: crate::simulation::zoning::ZoneType,
        catalog: &RuntimeEconomyCatalog,
        market: &OutputAbsorptionContext,
        policy: &CityFiscalPolicy,
        crew_cap: Option<u32>,
    ) -> bool {
        let carrier = self.begin_entrance(
            project.entrance.clone(),
            None,
            freight_goods_multiplier(profile, zone, self.tuning, self.minute_of_day),
        );
        if let Some(workers) = crew_cap {
            self.crew_cap = workers;
        }
        self.reservations.staged_owner = self.project_owner(project.parcel_id);
        self.buyer_site = self
            .reservations
            .staged_owner
            .zip(self.entrance.as_ref())
            .and_then(|(owner, entrance)| {
                self.reservations
                    .business
                    .prepared_site_at(owner, profile.runtime_id, entrance)
            });
        let prices = self.prices(catalog, zone, project.level, policy, carrier);
        super::spawn_need::project_covers_operation(
            profile,
            catalog,
            market.market(),
            prices,
            self.tuning.owa_export_price_multiplier,
            self,
        )
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
            imports: self.tuning.owa_import_price_multiplier
                * if self.goods_multiplier.is_finite() {
                    self.goods_multiplier
                } else {
                    1.0
                },
            carrier_quote: carrier.unwrap_or(f32::INFINITY),
            truck_payload: self.tuning.logistics.truck_payload_kg,
        }
    }
}

impl ProjectSupply for SiteSupply<'_> {
    fn customers(
        &mut self,
        profile: &EconomyProfileRuntime,
        area: f32,
        prices: OperationPrices,
        exports: f32,
    ) -> Option<ProjectCustomers> {
        self.reservations.staged_operation = None;
        use crate::simulation::economy::definitions::EconomyProfileRuntimeKind;
        let exportable = self.carrier.is_some()
            && exports > 0.0
            && matches!(
                profile.kind,
                EconomyProfileRuntimeKind::Processor
                    | EconomyProfileRuntimeKind::Producer
                    | EconomyProfileRuntimeKind::Extractor
                    | EconomyProfileRuntimeKind::FieldProducer
            );
        self.exports = if exportable { exports } else { 0.0 };
        let mut quote = ProjectCustomers::default();
        self.reservations.output_prices.clear();
        if let Some(entrance) = &self.entrance {
            for port in &profile.outputs {
                let output = profile.net_output_units_per_day(port) * area;
                if output <= 0.0 {
                    continue;
                }
                let household = self.reservations.customers.claim_reachable(
                    entrance,
                    port.resource_runtime_id,
                    f64::from(output),
                    profile.unit_price_currency * (1.0 + self.vat),
                    self.allocator,
                    self.network,
                    self.graph,
                );
                let business = if profile.kind != EconomyProfileRuntimeKind::ServiceStore
                    && let Some(components) = self.book.freight_components()
                {
                    if let Some(owner) = self.replacement {
                        self.reservations.business.claim_reachable_joint(
                            entrance,
                            owner,
                            port.resource_runtime_id,
                            (f64::from(output) - household).max(0.0),
                            profile.unit_price_currency,
                            components,
                            self.allocator,
                            self.logistics,
                            self.network,
                            self.graph,
                            self.tuning,
                        )
                    } else {
                        self.reservations.business.claim_reachable(
                            entrance,
                            self.reservations.staged_owner,
                            port.resource_runtime_id,
                            (f64::from(output) - household).max(0.0),
                            profile.unit_price_currency,
                            components,
                            self.allocator,
                            self.logistics,
                            self.network,
                            self.graph,
                            self.tuning,
                        )
                    }
                } else {
                    Default::default()
                };
                let local = household + business.units;
                let exported = if exportable {
                    (f64::from(output) - local).max(0.0)
                } else {
                    0.0
                };
                let revenue = household * f64::from(profile.unit_price_currency)
                    + business.revenue
                    + exported
                        * f64::from(
                            self.catalog
                                .unit_price_for_resource(port.resource_runtime_id)
                                .unwrap_or(0.0)
                                * exports,
                        );
                quote.activity = quote
                    .activity
                    .max(((local + exported) / f64::from(output)) as f32);
                quote.revenue += revenue;
                quote.export_freight +=
                    export_freight(profile, port, output, exported as f32, prices);
                self.reservations
                    .output_prices
                    .push((port.resource_runtime_id, revenue / f64::from(output)));
            }
        }
        if self.replacement.is_none() && !self.reservations.business.can_accept() {
            quote = ProjectCustomers::default();
        }
        self.customer_quote = quote;
        Some(quote)
    }
    fn output_price(&self, resource: u16) -> Option<f64> {
        Some(
            self.reservations
                .output_prices
                .iter()
                .find(|&&(id, _)| id == resource)
                .map_or(0.0, |&(_, price)| price),
        )
    }
    fn supported_workers(&self, _fallback: u32) -> u32 {
        self.plan.workers
    }
    fn crew_cap(&self) -> u32 {
        self.crew_cap
    }
    fn set_crew_cap(&mut self, workers: u32) {
        self.crew_cap = workers;
    }
    fn set_supported_plan(&mut self, plan: OperationPlan) {
        self.plan = plan;
        self.reservations.staged_operation = None;
    }
    fn startup_capital(
        &self,
        profile: &EconomyProfileRuntime,
        catalog: &RuntimeEconomyCatalog,
        _prices: OperationPrices,
    ) -> f64 {
        f64::from(profile.zoned_startup_budget(catalog, self.tuning.owa_import_price_multiplier))
    }
    fn fund(
        &mut self,
        profile: &EconomyProfileRuntime,
        prices: OperationPrices,
        costs: OperatingCosts,
        capital: f64,
    ) -> bool {
        self.fund_operation(profile, prices, costs, capital, false)
    }

    fn reservations(&self) -> Option<&ProjectReservations> {
        Some(self.reservations)
    }
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
        let zone = self
            .allocator
            .registry
            .get(asset)
            .and_then(|entry| entry.manifest.building.as_ref())
            .and_then(|building| building.zone_type)
            .map_or(
                crate::simulation::zoning::ZoneType::None,
                |zone| match zone {
                    crate::assets::asset::ZoneClass::Commercial => {
                        crate::simulation::zoning::ZoneType::Commercial
                    }
                    crate::assets::asset::ZoneClass::Industrial => {
                        crate::simulation::zoning::ZoneType::Industrial
                    }
                    _ => crate::simulation::zoning::ZoneType::None,
                },
            );
        let multiplier = profile.map_or(f32::INFINITY, |profile| {
            freight_goods_multiplier(profile, zone, self.tuning, self.minute_of_day)
        });
        let carrier = self.begin_entrance(entrance, None, multiplier);
        self.reservations.staged_owner = self.project_owner(parcel);
        self.buyer_site = self
            .reservations
            .staged_owner
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
            self.graph,
            self.goods_multiplier,
        )
    }

    fn accept(&mut self) {
        self.reservations.accept(self.book);
        self.replacement = None;
    }
    fn reject(&mut self) {
        self.reservations
            .reject(self.book, self.allocator, self.graph);
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

// One changed owner's rows; cold incremental path only.
fn collect_rows<T>(fill: impl FnOnce(&mut Vec<T>)) -> Vec<T> {
    let mut rows = Vec::new();
    fill(&mut rows);
    rows
}

// Full rebuild over `owners`, one buffer per Rayon split rather than per owner.
fn par_collect_rows<T: Send>(owners: usize, fill: impl Fn(usize, &mut Vec<T>) + Sync) -> Vec<T> {
    (0..owners)
        .into_par_iter()
        .fold(Vec::new, |mut rows, owner| {
            fill(owner, &mut rows);
            rows
        })
        .reduce(Vec::new, |mut rows, mut more| {
            rows.append(&mut more);
            rows
        })
}

// Every field of an operation row, as bits, for test builds' kept-row checks.
#[cfg(test)]
fn operation_bits(operation: &FundedOperation) -> [u64; 13] {
    let prices = operation.prices;
    [
        u64::from(operation.profile),
        u64::from(operation.plan.workers),
        u64::from(operation.plan.activity.to_bits()),
        u64::from(operation.plan.area.to_bits()),
        u64::from(prices.utilities[0].to_bits()),
        u64::from(prices.utilities[1].to_bits()),
        u64::from(prices.utilities[2].to_bits()),
        u64::from(prices.property_tax.to_bits()),
        u64::from(prices.imports.to_bits()),
        u64::from(prices.carrier_quote.to_bits()),
        u64::from(prices.truck_payload.to_bits()),
        operation.headroom[0].to_bits(),
        operation.headroom[1].to_bits(),
    ]
}

// Whether two site rows are equal, floats by bits, for test builds' kept-row checks.
#[cfg(test)]
fn site_matches(left: Option<&OperationSite>, right: Option<&OperationSite>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.entrance == right.entrance
                && left.buyer == right.buyer
                && left.buyer_site == right.buyer_site
                && left.source == right.source
                && left.goods_multiplier.to_bits() == right.goods_multiplier.to_bits()
                && left.exports.to_bits() == right.exports.to_bits()
                && left.future == right.future
                && left.public == right.public
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn populate(book: &mut ProjectReservations, owner: usize) {
        book.operations[owner] = Some(FundedOperation {
            profile: 1,
            plan: OperationPlan {
                workers: owner as u32,
                activity: 0.5,
                area: 1.0,
            },
            prices: OperationPrices {
                utilities: [1.0; 3],
                property_tax: 2.0,
                imports: 1.0,
                carrier_quote: 3.0,
                truck_payload: 1_000.0,
            },
            headroom: [4.0, 5.0],
        });
        book.sites[owner] = Some(OperationSite {
            entrance: None,
            buyer: Some(owner),
            buyer_site: None,
            source: Some(owner),
            goods_multiplier: 1.0,
            exports: 1.0,
            future: owner >= book.live_owners,
            public: false,
        });
        if owner >= book.live_owners {
            book.project_owners.push(owner);
        }
    }

    #[test]
    fn project_slots_reset_across_live_owner_growth_and_shrink() {
        let mut book = ProjectReservations::default();
        book.operations.resize(24, None);
        book.sites.resize(24, None);
        for live in [4, 2, 8, 0, 6, 6] {
            let previous_live = book.live_owners;
            book.reset_owner_slots(live);
            assert_eq!(book.operations.len(), 24);
            assert_eq!(book.sites.len(), 24);
            for owner in 0..24 {
                let retained = owner < live.min(previous_live);
                assert_eq!(
                    book.operations[owner].is_some(),
                    retained,
                    "operation {owner}, live {live}"
                );
                assert_eq!(
                    book.sites[owner].is_some(),
                    retained,
                    "site {owner}, live {live}"
                );
                if retained {
                    assert_eq!(book.operations[owner].unwrap().plan.workers, owner as u32);
                    assert_eq!(book.sites[owner].as_ref().unwrap().source, Some(owner));
                }
            }
            assert!(book.project_owners.is_empty());
            for owner in 0..live {
                populate(&mut book, owner);
            }
            for owner in [live, live + 3, 23] {
                populate(&mut book, owner);
            }
        }
    }

    #[test]
    #[ignore = "isolated project-slot reset performance comparison"]
    fn benchmark_project_slot_reset() {
        use std::{hint::black_box, time::Instant};
        const LIVE: usize = 4_096;
        const REPEATS: usize = 16;
        for projects in [1_024, 16_384, 131_072] {
            for pair in 0..9 {
                for sparse in if pair % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                } {
                    let mut book = ProjectReservations::default();
                    book.reset_owner_slots(LIVE);
                    book.operations.resize(LIVE + projects, None);
                    book.sites.resize(LIVE + projects, None);
                    book.project_owners.reserve(projects);
                    let start = Instant::now();
                    for _ in 0..REPEATS {
                        for offset in 0..8 {
                            populate(&mut book, LIVE + offset);
                        }
                        if sparse {
                            book.reset_owner_slots(LIVE);
                        } else {
                            book.operations.truncate(LIVE);
                            book.sites.truncate(LIVE);
                            book.project_owners.clear();
                            book.operations.resize(LIVE + projects, None);
                            book.sites.resize(LIVE + projects, None);
                        }
                        black_box(&book);
                    }
                    let reset_ns = start.elapsed().as_nanos() as f64 / REPEATS as f64;
                    assert!(book.operations.iter().all(Option::is_none));
                    assert!(book.sites.iter().all(Option::is_none));
                    eprintln!(
                        "PROJECT_SLOTS {{\"projects\":{projects},\"pair\":{pair},\"sparse\":{sparse},\"reset_ns\":{reset_ns}}}"
                    );
                }
            }
        }
    }
}
