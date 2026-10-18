// SPDX-License-Identifier: GPL-2.0-only

//! The funding census as resumable steps (`SIM-02-I`, `docs/simulation.md#pure-jobs`).
//!
//! [`JobFunding::refresh`] and its variants run every step at once. The hourly census runs the
//! same steps over the hour's second half on the input copy, a bounded slice per slot
//! (`hiring/rolling.rs`). Every step reads only the census inputs and the census's own books,
//! and a slice of a loop processes the same rows in the same order as the whole loop, so a
//! census stepped over any slices equals one run at once bit for bit.
//!
//! **Steps**, in order: route quotes and paid freight; the production market (its 256-building
//! chunks sliced); the consumer-market refresh; setup (port offsets, supplier order, provisional
//! shares and the household-quote preparation); the initial ceilings (sliced). Then each closure
//! pass: import funding (sliced, parallel); household claims (sliced, serial); the input-source
//! reset; household production (sliced); the current-staff and expansion staffing sweeps
//! (sliced, serial in supplier order); finishing the pass's household quotes; publishing funded
//! activity and household production (both sliced); the provisional-input repair; business
//! validation (seeding the accounts, the claims sliced by seller, the proof sliced, publishing
//! the financing); and settling, or propagating impossible customers into the next pass.
//!
//! **Costs.** Work is counted in rows: buildings for every building pass, households for the
//! consumer-market refresh, nothing for the bookkeeping steps. A sliced step processes exactly
//! its budget of rows; an unsliced one runs whole once started. [`CensusProgress::remaining`]
//! estimates the rows left assuming the current pass is the last, which the hourly driver
//! spreads over the slots left. Steps allocate nothing once the census storage has grown.

use std::ops::Range;

use super::*;

/// What a census reads besides its own books: the city (live or an input copy), the network,
/// the catalog and the census parameters.
pub(super) struct CensusContext<'a, F> {
    pub(super) allocator: &'a BuildingAllocator,
    pub(super) households: &'a [Household],
    pub(super) network: &'a TransitNetwork,
    pub(super) graph: &'a RegionGraph,
    pub(super) catalog: &'a RuntimeEconomyCatalog,
    pub(super) tuning: &'a RuntimeEconomyTuning,
    pub(super) fiscal: &'a CityFiscalPolicy,
    pub(super) treasury_balance: f64,
    pub(super) minute_of_day: u16,
    // A service's fiscal staffing ceiling from `(index, physical capacity, profile)`.
    pub(super) service_capacity: F,
    // The census whose import plans a construction census may replay.
    pub(super) previous: Option<&'a JobFunding>,
    // Each building's paid cargo frozen with an input copy; `None` reads open shipments.
    pub(super) freight_capital: Option<&'a [f64]>,
}

/// Wall time spent in each step, in nanoseconds, summed over every census of the process;
/// for the step-cost measurement only (`populated_city/census_churn.rs`).
#[cfg(test)]
pub(crate) static STEP_NANOS: [std::sync::atomic::AtomicU64; STEP_COUNT] =
    [const { std::sync::atomic::AtomicU64::new(0) }; STEP_COUNT];

/// Number of census steps, `Done` included.
#[cfg(test)]
pub(crate) const STEP_COUNT: usize = Step::Done as usize + 1;

/// Step names in order, for the step-cost measurement.
#[cfg(test)]
pub(crate) const STEP_NAMES: [&str; STEP_COUNT] = [
    "routes",
    "market_begin",
    "market_chunks",
    "market_end",
    "customers",
    "setup",
    "ceilings",
    "pass_start",
    "import_funding",
    "household_claims",
    "sources",
    "source_production",
    "current_staff",
    "expansion",
    "finish_pass",
    "publish_activity",
    "publish_production",
    "repair",
    "business_seed",
    "business_claims",
    "business_proof",
    "business_publish",
    "settle",
    "done",
];

/// A census step; see the module documentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Step {
    #[default]
    Routes,
    MarketBegin,
    MarketChunks,
    MarketEnd,
    Customers,
    Setup,
    Ceilings,
    PassStart,
    ImportFunding,
    HouseholdClaims,
    Sources,
    SourceProduction,
    CurrentStaff,
    Expansion,
    FinishPass,
    PublishActivity,
    PublishProduction,
    Repair,
    BusinessSeed,
    BusinessClaims,
    BusinessProof,
    BusinessPublish,
    Settle,
    Done,
}

impl Step {
    // Rows of work: buildings for building passes, households for the consumer market.
    fn rows(self, buildings: usize, households: usize) -> usize {
        match self {
            Self::MarketBegin
            | Self::MarketEnd
            | Self::PassStart
            | Self::FinishPass
            | Self::BusinessPublish
            | Self::Done => 0,
            Self::Customers => households,
            _ => buildings,
        }
    }

    // The step after this one when the pass continues in order.
    fn next(self) -> Self {
        use Step::*;
        match self {
            Routes => MarketBegin,
            MarketBegin => MarketChunks,
            MarketChunks => MarketEnd,
            MarketEnd => Customers,
            Customers => Setup,
            Setup => Ceilings,
            Ceilings => PassStart,
            PassStart => ImportFunding,
            ImportFunding => HouseholdClaims,
            HouseholdClaims => Sources,
            Sources => SourceProduction,
            SourceProduction => CurrentStaff,
            CurrentStaff => Expansion,
            Expansion => FinishPass,
            FinishPass => PublishActivity,
            PublishActivity => PublishProduction,
            PublishProduction => Repair,
            Repair => BusinessSeed,
            BusinessSeed => BusinessClaims,
            BusinessClaims => BusinessProof,
            BusinessProof => BusinessPublish,
            BusinessPublish => Settle,
            Settle | Done => Done,
        }
    }

    // Column of the `funding_detail` debug line the step's time adds to.
    fn timing_slot(self) -> usize {
        use Step::*;
        match self {
            Routes => 0,
            MarketBegin | MarketChunks | MarketEnd => 1,
            Customers => 2,
            Setup | Ceilings | PassStart | ImportFunding => 3,
            HouseholdClaims => 4,
            Sources | SourceProduction => 5,
            CurrentStaff | Expansion => 6,
            FinishPass | PublishActivity | PublishProduction => 7,
            Repair => 8,
            BusinessSeed | BusinessClaims | BusinessProof | BusinessPublish => 9,
            Settle | Done => 10,
        }
    }
}

/// Where a census stands between slots: the next step, its cursor and what earlier steps pass
/// on. [`Default`] is a census not yet started.
#[derive(Clone, Debug, Default)]
pub(crate) struct CensusProgress {
    step: Step,
    // Rows of the current step done: buildings, staffing slots or, for the market, chunks.
    cursor: usize,
    reuse_market: bool,
    retained_customers: bool,
    input_ports: usize,
    first_pass: bool,
    // Closure passes started; slicing only, never read by the census itself.
    passes: usize,
    reduced: bool,
    settled: bool,
    // Diagnostic step times for the `funding_detail` debug line; never read by the census.
    times: [f64; 11],
}

impl CensusProgress {
    /// Whether every step has run.
    pub(crate) fn is_done(&self) -> bool {
        self.step == Step::Done
    }

    /// Rows left assuming the census takes `expected` closure passes, as the previous one
    /// did (`SIM-02-I`): the current pass's and the steps after it, and whole passes for the
    /// ones still expected. A census that needs more catches up in its last slots. O(steps).
    pub(crate) fn remaining(&self, buildings: usize, households: usize, expected: usize) -> usize {
        if self.step == Step::Done {
            return 0;
        }
        let done = if self.step == Step::MarketChunks {
            (self.cursor * MARKET_CHUNK).min(buildings)
        } else {
            self.cursor
        };
        let mut rows = self.step.rows(buildings, households).saturating_sub(done);
        let mut step = self.step.next();
        while step != Step::Done {
            rows += step.rows(buildings, households);
            step = step.next();
        }
        let mut pass = 0;
        let mut step = Step::PassStart;
        while step != Step::Done {
            pass += step.rows(buildings, households);
            step = step.next();
        }
        rows + expected.saturating_sub(self.passes.max(1)) * pass
    }

    /// Closure passes started so far.
    pub(crate) fn passes(&self) -> usize {
        self.passes
    }

    fn advance(&mut self, step: Step) {
        self.step = step;
        self.cursor = 0;
    }

    // The next slice of a sliced step over `total` items within `budget`; finishing it moves
    // to the next step.
    fn slice(&mut self, total: usize, budget: usize) -> Range<usize> {
        let start = self.cursor.min(total);
        let end = start + (total - start).min(budget);
        self.cursor = end;
        start..end
    }

    fn finish_slice(&mut self, total: usize) {
        if self.cursor >= total {
            self.advance(self.step.next());
        }
    }
}

fn timing_category() -> crate::debug::Channel {
    if crate::debug::Channel::EconomyPhases.enabled() {
        crate::debug::Channel::EconomyPhases
    } else {
        crate::debug::Channel::Economy
    }
}

fn market_revision(
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
) -> (u64, u64, u32, u64, usize) {
    (
        allocator.building_ref_revision(),
        allocator.entrance_ref_revision(),
        network.cch_graph.build_generation,
        allocator.registry.revision(),
        allocator.buildings.len(),
    )
}

fn utilities(catalog: &RuntimeEconomyCatalog, tuning: &RuntimeEconomyTuning) -> [f32; 3] {
    local_utility_service_costs(catalog).map(|price| price * tuning.owa_import_price_multiplier)
}

impl JobFunding {
    /// Runs census steps until `budget` rows are used or every step has run; returns whether
    /// the census is done. `usize::MAX` runs it to the end.
    pub(super) fn run_census_steps<F>(
        &mut self,
        ctx: &CensusContext<'_, F>,
        logistics: &mut ShipmentSystem,
        progress: &mut CensusProgress,
        mut budget: usize,
    ) -> bool
    where
        F: Fn(usize, u32, &EconomyProfileRuntime) -> u32 + Sync,
    {
        #[cfg(test)]
        let _contract_phase = crate::debug::work::phases::enter("census");
        let timing = timing_category().enabled();
        while progress.step != Step::Done && budget > 0 {
            let started = timing.then(std::time::Instant::now);
            #[cfg(test)]
            let measured = std::time::Instant::now();
            let step = progress.step;
            let used = self.census_step(ctx, logistics, progress, budget);
            if let Some(started) = started {
                progress.times[step.timing_slot()] += started.elapsed().as_secs_f64() * 1000.0;
            }
            #[cfg(test)]
            STEP_NANOS[step as usize].fetch_add(
                measured.elapsed().as_nanos() as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
            budget = budget.saturating_sub(used);
            if progress.step == Step::Done && timing {
                self.log_census(ctx, progress);
            }
        }
        progress.step == Step::Done
    }

    // One step or slice; returns the rows it used.
    fn census_step<F>(
        &mut self,
        ctx: &CensusContext<'_, F>,
        logistics: &mut ShipmentSystem,
        progress: &mut CensusProgress,
        budget: usize,
    ) -> usize
    where
        F: Fn(usize, u32, &EconomyProfileRuntime) -> u32 + Sync,
    {
        let buildings = ctx.allocator.buildings.len();
        let step = progress.step;
        let whole = step.rows(buildings, ctx.households.len());
        match step {
            Step::Routes => self.census_routes(ctx, logistics),
            Step::MarketBegin => self.census_market_begin(ctx, progress),
            Step::MarketChunks => {
                let chunks = buildings.div_ceil(MARKET_CHUNK);
                let slice = progress.slice(chunks, budget.div_ceil(MARKET_CHUNK));
                let used = (slice.len() * MARKET_CHUNK).min(buildings);
                self.census_market_chunks(ctx, slice);
                progress.finish_slice(chunks);
                return used;
            }
            Step::MarketEnd => self.census_market_end(ctx, progress),
            Step::Customers => self.census_customers(ctx, progress),
            Step::Setup => self.census_setup(ctx, progress),
            Step::Ceilings
            | Step::ImportFunding
            | Step::HouseholdClaims
            | Step::SourceProduction
            | Step::CurrentStaff
            | Step::Expansion
            | Step::PublishActivity
            | Step::PublishProduction
            | Step::BusinessClaims
            | Step::BusinessProof => {
                let rows = progress.slice(buildings, budget);
                let used = rows.len();
                match step {
                    Step::Ceilings => self.census_ceilings(ctx, rows),
                    Step::ImportFunding => self.census_import_funding(ctx, rows),
                    Step::HouseholdClaims => self.census_household_claims(ctx, rows),
                    Step::SourceProduction => self.reserve_household_production(
                        ctx.allocator,
                        ctx.catalog,
                        ctx.graph,
                        true,
                        rows,
                    ),
                    Step::CurrentStaff | Step::Expansion => {
                        self.census_staffing(ctx, logistics, step == Step::CurrentStaff, rows)
                    }
                    Step::PublishActivity => {
                        for index in rows {
                            self.input_supply.publish_funded_activity(
                                index,
                                self.offered_activity[index],
                                self.activities[index],
                                ctx.allocator,
                                ctx.catalog,
                                ctx.graph,
                            );
                        }
                    }
                    Step::PublishProduction => self.reserve_household_production(
                        ctx.allocator,
                        ctx.catalog,
                        ctx.graph,
                        false,
                        rows,
                    ),
                    Step::BusinessClaims => {
                        if let Err(required) = self.claim_business_customers(
                            ctx.allocator,
                            logistics,
                            ctx.network,
                            ctx.graph,
                            ctx.catalog,
                            ctx.tuning,
                            rows,
                        ) {
                            // Grow between attempts, then seed and claim again in order.
                            self.business.reserve_receipts(required);
                            progress.advance(Step::BusinessSeed);
                            return used;
                        }
                    }
                    Step::BusinessProof => {
                        progress.reduced |= self.prove_business_customers(
                            ctx.allocator,
                            ctx.catalog,
                            ctx.tuning,
                            ctx.fiscal,
                            ctx.minute_of_day,
                            rows,
                        );
                    }
                    _ => unreachable!(),
                }
                if step == Step::Ceilings && progress.cursor >= buildings {
                    #[cfg(test)]
                    {
                        self.closure_passes = 0;
                        self.propagation_rounds = 0;
                    }
                    progress.first_pass = true;
                }
                progress.finish_slice(buildings);
                return used;
            }
            Step::PassStart => {
                #[cfg(test)]
                {
                    self.closure_passes += 1;
                }
                progress.passes += 1;
                if !progress.first_pass {
                    let household_census = household_claims::Census::new(
                        ctx.allocator,
                        ctx.catalog,
                        ctx.fiscal,
                        ctx.network,
                        ctx.graph,
                    );
                    self.household_quotes
                        .restart(&mut self.customers, &household_census);
                    self.customer_units.fill(-1.0);
                }
                progress.first_pass = false;
            }
            Step::Sources => {
                self.input_supply.reset(
                    ctx.allocator,
                    ctx.graph,
                    ctx.network,
                    ctx.catalog,
                    &self.offered_activity,
                );
            }
            Step::FinishPass => {
                let household_census = household_claims::Census::new(
                    ctx.allocator,
                    ctx.catalog,
                    ctx.fiscal,
                    ctx.network,
                    ctx.graph,
                );
                self.household_quotes
                    .finish_pass(&mut self.customers, &household_census);
            }
            Step::Repair => {
                // Withdrawn provisional suppliers can still own input receipts until the next
                // quotation. Those invalid purchases hide customers from alternative suppliers;
                // using them to lower permanent worker ceilings can strand a fully financed
                // chain. Requote physical claims first, then derive ceilings from a consistent
                // input book.
                let repaired = !self.input_supply.has_overclaims()
                    || self.repair_provisional_inputs(
                        ctx.allocator,
                        logistics,
                        ctx.network,
                        ctx.graph,
                        ctx.catalog,
                        ctx.tuning,
                        ctx.fiscal,
                        ctx.minute_of_day,
                    );
                if !repaired {
                    progress.settled = false;
                    progress.advance(Step::Settle);
                    return whole;
                }
            }
            Step::BusinessSeed => {
                if !self.seed_business_accounts(
                    ctx.allocator,
                    ctx.network,
                    ctx.graph,
                    ctx.catalog,
                    ctx.tuning,
                    ctx.fiscal,
                    ctx.minute_of_day,
                    progress.input_ports,
                ) {
                    progress.settled = false;
                    progress.advance(Step::Settle);
                    return whole;
                }
                progress.reduced = false;
            }
            Step::BusinessPublish => {
                self.publish_business_financing();
                progress.settled = !progress.reduced;
            }
            Step::Settle => {
                if progress.settled {
                    self.publish_business_deliveries(ctx.allocator, ctx.catalog, ctx.graph);
                    progress.advance(Step::Done);
                    return whole;
                }
                self.propagate_impossible_customers(
                    ctx.allocator,
                    ctx.network,
                    ctx.graph,
                    ctx.catalog,
                    ctx.tuning,
                );
                self.offered_activity.clone_from(&self.activities);
                progress.advance(Step::PassStart);
                return whole;
            }
            Step::Done => return 0,
        }
        progress.advance(step.next());
        whole
    }

    fn census_routes<F>(&mut self, ctx: &CensusContext<'_, F>, logistics: &mut ShipmentSystem) {
        #[cfg(test)]
        {
            self.import_quote_replays = 0;
            self.household_detail = [0.0; 3];
        }
        let CensusContext {
            allocator,
            network,
            graph,
            catalog,
            tuning,
            minute_of_day,
            ..
        } = *ctx;
        self.pricing_phase = freight_price_phase(tuning, minute_of_day);
        let count = allocator.buildings.len();
        crate::debug::work::add(crate::debug::work::Work::CensusBuildings, count as u64);
        self.capacities.resize(count, 0);
        self.activities.resize(count, 0.0);
        self.headroom.resize(count, [0.0; 2]);
        self.local_input_backing.resize(count, false);
        self.service_capacities.resize(count, 0);
        self.freight_capital.resize(count, 0.0);
        let revision = (
            allocator.building_ref_revision(),
            allocator.entrance_ref_revision(),
            network.cch_graph.build_generation,
            network.cch_graph.metric_revision,
            network.frontage_delay_revision,
        );
        crate::simulation::economy::logistics::collect_connected_border_nodes(
            graph,
            &mut self.border_nodes,
        );
        if self.quote_revision != Some(revision)
            || self.carrier_quotes.len() != count
            || self.border_nodes != self.quoted_border_nodes
        {
            self.carrier_quotes.resize(count, None);
            logistics.write_border_carrier_quotes(
                allocator,
                network,
                graph,
                &self.border_nodes,
                catalog,
                tuning,
                &mut self.carrier_quotes,
            );
            self.quote_revision = Some(revision);
            self.quoted_border_nodes.clone_from(&self.border_nodes);
        }
        match ctx.freight_capital {
            Some(capital) => {
                debug_assert_eq!(capital.len(), count);
                self.freight_capital.copy_from_slice(capital);
            }
            None => logistics.write_freight_working_capital(&mut self.freight_capital),
        }
    }

    fn census_market_begin<F>(
        &mut self,
        ctx: &CensusContext<'_, F>,
        progress: &mut CensusProgress,
    ) {
        let allocator = ctx.allocator;
        let network = ctx.network;
        let catalog = ctx.catalog;
        let count = allocator.buildings.len();
        let resources = catalog.resource_count() + 1;
        self.potential_customers.prepare(count);
        self.market.resize(resources, [0.0; 2]);
        self.market.fill([0.0; 2]);
        self.market_chunks
            .resize(count.div_ceil(MARKET_CHUNK) * resources, [0.0; 2]);
        let market_revision = market_revision(allocator, network);
        progress.reuse_market = self.market_revision == Some(market_revision)
            && self
                .market_catalog
                .as_ref()
                .is_some_and(|identity| std::sync::Arc::ptr_eq(identity, catalog.cache_identity()))
            && allocator.production_changes_since(
                self.market_production_revision,
                &mut self.market_changed_owners,
            );
        self.market_dirty_chunks
            .resize(count.div_ceil(MARKET_CHUNK), false);
        self.market_dirty_chunks.fill(!progress.reuse_market);
        if progress.reuse_market {
            for &owner in &self.market_changed_owners {
                if let Some(dirty) = self.market_dirty_chunks.get_mut(owner / MARKET_CHUNK) {
                    *dirty = true;
                }
            }
        }
        #[cfg(test)]
        {
            self.market_rows = self
                .market_dirty_chunks
                .iter()
                .enumerate()
                .filter(|(_, dirty)| **dirty)
                .map(|(chunk, _)| MARKET_CHUNK.min(count - chunk * MARKET_CHUNK))
                .sum();
        }
    }

    fn census_market_chunks<F>(&mut self, ctx: &CensusContext<'_, F>, chunks: Range<usize>) {
        let allocator = ctx.allocator;
        let catalog = ctx.catalog;
        let count = allocator.buildings.len();
        let resources = catalog.resource_count() + 1;
        self.market_chunks[chunks.start * resources..chunks.end * resources]
            .par_chunks_mut(resources)
            .zip(
                self.potential_customers.counted
                    [chunks.start * MARKET_CHUNK..(chunks.end * MARKET_CHUNK).min(count)]
                    .par_chunks_mut(MARKET_CHUNK),
            )
            .enumerate()
            .for_each(|(offset, (totals, counted))| {
                let chunk = chunks.start + offset;
                if !self.market_dirty_chunks[chunk] {
                    return;
                }
                totals.fill([0.0; 2]);
                let first = chunk * MARKET_CHUNK;
                for (building, counted) in allocator.buildings
                    [first..(first + MARKET_CHUNK).min(count)]
                    .iter()
                    .zip(counted)
                {
                    *counted = false;
                    if !live(building) {
                        continue;
                    }
                    let Some(profile) =
                        catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                    else {
                        continue;
                    };
                    let area = output_area_scale_for_building(building, profile);
                    let physical = physical_worker_capacity_for_profile(building, profile);
                    let activity = (building.worker_count as f32 / physical.max(1) as f32).min(1.0);
                    *counted =
                        building.worker_count > 0 && physical > 0 && !profile.inputs.is_empty();
                    for output in &profile.outputs {
                        totals[output.resource_runtime_id as usize][0] +=
                            f64::from(profile.net_output_units_per_day(output) * area);
                    }
                    // Existing staffed buyers supply input demand. A proposed job cannot
                    // justify its own revenue by promising another unfunded employer's purchases.
                    for input in &profile.inputs {
                        let internal = profile
                            .output_port(input.resource_runtime_id)
                            .map_or(0.0, |port| port.units_per_day);
                        totals[input.resource_runtime_id as usize][1] +=
                            f64::from((input.units_per_day - internal).max(0.0) * area * activity);
                    }
                }
            });
    }

    fn census_market_end<F>(&mut self, ctx: &CensusContext<'_, F>, progress: &CensusProgress) {
        let allocator = ctx.allocator;
        let network = ctx.network;
        let catalog = ctx.catalog;
        let resources = catalog.resource_count() + 1;
        self.market_revision = Some(market_revision(allocator, network));
        self.market_production_revision = allocator.production_revision();
        if !progress.reuse_market {
            self.market_catalog = Some(catalog.cache_identity().clone());
        }
        self.market
            .par_iter_mut()
            .enumerate()
            .for_each(|(resource, total)| {
                for chunk in self.market_chunks.chunks(resources) {
                    total[0] += chunk[resource][0];
                    total[1] += chunk[resource][1];
                }
            });
    }

    fn census_customers<F>(&mut self, ctx: &CensusContext<'_, F>, progress: &mut CensusProgress) {
        let CensusContext {
            allocator,
            households,
            network,
            graph,
            catalog,
            tuning,
            fiscal,
            treasury_balance,
            ..
        } = *ctx;
        progress.retained_customers = self.customers.refresh(
            households,
            allocator,
            catalog,
            tuning,
            fiscal,
            treasury_balance,
            network,
            graph,
        );
    }

    fn census_setup<F>(&mut self, ctx: &CensusContext<'_, F>, progress: &mut CensusProgress) {
        let allocator = ctx.allocator;
        let catalog = ctx.catalog;
        #[cfg(test)]
        let CensusContext {
            households,
            network,
            graph,
            tuning,
            fiscal,
            treasury_balance,
            ..
        } = *ctx;
        let count = allocator.buildings.len();
        self.record_operating(&allocator.buildings);
        self.customer_offsets.resize(count + 1, 0);
        self.local_order.resize(count, 0);
        self.phase_offsets.resize(catalog.funding_phase_count(), 0);
        self.phase_offsets.fill(0);
        let mut ports = 0;
        let mut inputs = 0;
        let mut consumer_firms = 0;
        let mut consumer_ports = 0;
        let mut maximum_consumer_ports = 0;
        for (index, building) in allocator.buildings.iter().enumerate() {
            self.customer_offsets[index] = ports;
            self.phase_offsets[catalog.funding_phase(building.economy_profile_runtime_id)] += 1;
            if let Some(profile) =
                catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            {
                ports += profile.outputs.len();
                inputs += profile.inputs.len();
                if profile
                    .outputs
                    .iter()
                    .any(|port| self.customers.consumes(port.resource_runtime_id))
                {
                    consumer_firms += 1;
                    consumer_ports += profile.outputs.len();
                    maximum_consumer_ports = maximum_consumer_ports.max(profile.outputs.len());
                }
            }
        }
        self.customer_offsets[count] = ports;
        self.import_plans.prepare(&self.customer_offsets, catalog);
        // Immutable recipe order is prepared once, not once per closure attempt. Counts
        // and scatter cursors are dependent; stable building order keeps claims deterministic.
        let mut first = 0;
        for offset in &mut self.phase_offsets {
            let end = first + *offset;
            *offset = first;
            first = end;
        }
        self.potential_customers.reserve_inputs(inputs);
        for (index, building) in allocator.buildings.iter().enumerate() {
            if self.potential_customers.counted[index] {
                self.potential_customers.owners.push(index);
            }
            let next =
                &mut self.phase_offsets[catalog.funding_phase(building.economy_profile_runtime_id)];
            self.local_order[*next] = index;
            *next += 1;
            if let Some(profile) =
                catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            {
                let area = output_area_scale_for_building(building, profile);
                for (share, port) in self.import_plans.shares
                    [self.customer_offsets[index]..self.customer_offsets[index + 1]]
                    .iter_mut()
                    .zip(&profile.outputs)
                {
                    *share = provisional_local_share(
                        profile,
                        area,
                        port.resource_runtime_id,
                        None,
                        &self.market,
                    )
                    .to_bits();
                }
            }
        }
        self.customer_units.resize(ports, -1.0);
        self.import_memo_units.resize(ports, 0.0);
        #[cfg(test)]
        if self.household_quotes.disabled {
            if progress.retained_customers {
                self.customers.reset(
                    households,
                    allocator,
                    catalog,
                    tuning,
                    fiscal,
                    treasury_balance,
                    network,
                    graph,
                );
            }
            progress.retained_customers = false;
        }
        self.household_quotes.prepare(
            consumer_firms,
            consumer_ports,
            maximum_consumer_ports,
            progress.retained_customers,
        );
        self.household_production.resize(ports, 0.0);
        self.customer_units
            .par_iter_mut()
            .zip(&mut self.household_production)
            .with_min_len(MARKET_CHUNK)
            .for_each(|(units, production)| {
                *units = -1.0;
                *production = 0.0;
            });
        self.closure_ceilings.resize(count, 0);
        self.closure_activity.resize(count, 1.0);
        self.import_memos.resize(count, Default::default());
        self.import_memos.fill(Default::default());
        self.service_flags.resize(count, false);
        self.offered_activity.resize(count, 1.0);
        progress.input_ports = inputs;
    }

    fn census_ceilings<F>(&mut self, ctx: &CensusContext<'_, F>, rows: Range<usize>) {
        let allocator = ctx.allocator;
        let catalog = ctx.catalog;
        let tuning = ctx.tuning;
        let minute_of_day = ctx.minute_of_day;
        #[cfg(test)]
        let profit_proof = !self.potential_customers.disabled;
        #[cfg(not(test))]
        let profit_proof = true;
        self.closure_ceilings[rows.clone()]
            .par_iter_mut()
            .zip(&mut self.closure_activity[rows.clone()])
            .zip(&mut self.offered_activity[rows.clone()])
            .zip(&mut self.service_flags[rows.clone()])
            .zip(&allocator.buildings[rows.clone()])
            .zip(&self.freight_capital[rows.clone()])
            .zip(&self.carrier_quotes[rows])
            .with_min_len(MARKET_CHUNK)
            .for_each(
                |((((((ceiling, activity), offered), service), building), &cargo), &carrier)| {
                    *service = allocator.is_city_service_building(building);
                    *ceiling = catalog
                        .profile_by_runtime_id(building.economy_profile_runtime_id)
                        .filter(|profile| {
                            *service
                                || !(cannot_fund_one_worker(building, profile, cargo)
                                    || (profit_proof
                                        && cannot_profit_at_best_prices(
                                            building,
                                            profile,
                                            catalog,
                                            tuning,
                                            carrier,
                                            minute_of_day,
                                        )))
                        })
                        .map_or(0, |profile| {
                            physical_worker_capacity_for_profile(building, profile)
                        });
                    *activity = 1.0;
                    // A proven zero ceiling never produces, so buyers never stage its output.
                    *offered = if *ceiling == 0 { 0.0 } else { 1.0 };
                },
            );
    }

    fn census_import_funding<F>(&mut self, ctx: &CensusContext<'_, F>, rows: Range<usize>)
    where
        F: Fn(usize, u32, &EconomyProfileRuntime) -> u32 + Sync,
    {
        let allocator = ctx.allocator;
        let catalog = ctx.catalog;
        let tuning = ctx.tuning;
        let fiscal = ctx.fiscal;
        let minute_of_day = ctx.minute_of_day;
        let utilities = utilities(catalog, tuning);
        #[cfg(test)]
        let import_replay_disabled = self.import_quote_replay_disabled;
        #[cfg(test)]
        let import_replays = std::sync::atomic::AtomicUsize::new(0);
        self.capacities[rows.clone()]
            .par_iter_mut()
            .zip(self.service_capacities[rows.clone()].par_iter_mut())
            .zip(self.activities[rows.clone()].par_iter_mut())
            .zip(self.headroom[rows.clone()].par_iter_mut())
            .zip(self.local_input_backing[rows.clone()].par_iter_mut())
            .zip(self.import_memos[rows.clone()].par_iter_mut())
            .zip(self.import_plans.rows[rows.clone()].par_iter_mut())
            .enumerate()
            .for_each(
                |(
                    offset,
                    ((((((capacity, service), activity), headroom), local_backing), memo), row),
                )| {
                    let index = rows.start + offset;
                    *capacity = 0;
                    *service = 0;
                    *activity = 0.0;
                    *headroom = [0.0; 2];
                    *local_backing = false;
                    let building = &allocator.buildings[index];
                    let Some(profile) =
                        catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                    else {
                        return;
                    };
                    let physical = physical_worker_capacity_for_profile(building, profile);
                    let goods_multiplier = freight_goods_multiplier(
                        profile,
                        building.zone_type,
                        tuning,
                        minute_of_day,
                    );
                    let prices = OperationPrices {
                        utilities,
                        property_tax: daily_property_tax(
                            building.zone_type,
                            building.level,
                            fiscal,
                        ),
                        imports: tuning.owa_import_price_multiplier
                            * if goods_multiplier.is_finite() {
                                goods_multiplier
                            } else {
                                1.0
                            },
                        carrier_quote: self.carrier_quotes[index]
                            .filter(|_| goods_multiplier.is_finite())
                            .unwrap_or(f32::INFINITY),
                        truck_payload: tuning.logistics.truck_payload_kg,
                    };
                    if self.service_flags[index] {
                        *service = (ctx.service_capacity)(index, physical, profile).min(physical);
                        if live(building) {
                            *capacity = *service;
                            let planned = *service as f32 / physical.max(1) as f32;
                            let costs = OperatingCosts::for_plan(
                                profile,
                                catalog,
                                OperationPlan {
                                    workers: *service,
                                    activity: planned,
                                    area: output_area_scale_for_building(building, profile),
                                },
                                prices,
                                |_, _| 0.0,
                            );
                            if costs.inputs.is_finite() && costs.freight.is_finite() {
                                *activity = planned;
                            }
                        }
                        return;
                    }
                    if !live(building) || physical == 0 || self.closure_ceilings[index] == 0 {
                        return;
                    }
                    // Finite family claims commit serially below. Independent non-consumer
                    // firms and public funding retain the parallel import-ceiling pass.
                    if profile
                        .outputs
                        .iter()
                        .any(|port| self.customers.consumes(port.resource_runtime_id))
                    {
                        return;
                    }
                    let exports = building_outputs_can_export_to_owa(building, profile)
                        && self.carrier_quotes[index].is_some()
                        && tuning.owa_export_price_multiplier > 0.0;
                    let memo_activity = self.closure_activity[index];
                    #[cfg(test)]
                    let replay_enabled = !import_replay_disabled;
                    #[cfg(not(test))]
                    let replay_enabled = true;
                    if replay_enabled
                        && memo.matches(self.closure_ceilings[index], memo_activity, &[], &[])
                    {
                        (*capacity, *activity, *headroom) = memo.result();
                        #[cfg(test)]
                        import_replays.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        return;
                    }
                    let operation = funded_private_operation(
                        building,
                        profile,
                        catalog,
                        physical,
                        prices,
                        &self.market,
                        exports.then_some(tuning.owa_export_price_multiplier),
                        self.freight_capital[index],
                        None,
                        None,
                        headroom,
                        self.closure_ceilings[index],
                        self.closure_activity[index],
                        Some(ImportPlanRecord {
                            row,
                            shares: &self.import_plans.shares
                                [self.customer_offsets[index]..self.customer_offsets[index + 1]],
                            previous: ctx
                                .previous
                                .and_then(|funding| funding.import_plans.get(index, catalog)),
                        }),
                    );
                    *capacity = operation.0;
                    *activity = operation.1;
                    memo.remember(
                        self.closure_ceilings[index],
                        memo_activity,
                        operation,
                        *headroom,
                    );
                },
            );
        #[cfg(test)]
        {
            self.import_quote_replays += import_replays.load(std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn census_household_claims<F>(&mut self, ctx: &CensusContext<'_, F>, rows: Range<usize>) {
        let CensusContext {
            allocator,
            network,
            graph,
            catalog,
            tuning,
            fiscal,
            minute_of_day,
            ..
        } = *ctx;
        let utilities = utilities(catalog, tuning);
        #[cfg(test)]
        let import_replay_disabled = self.import_quote_replay_disabled;
        #[cfg(test)]
        let timing = timing_category().enabled();
        let household_census =
            household_claims::Census::new(allocator, catalog, fiscal, network, graph);
        for index in rows {
            let building = &allocator.buildings[index];
            if !live(building) || self.service_flags[index] {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if !profile
                .outputs
                .iter()
                .any(|port| self.customers.consumes(port.resource_runtime_id))
            {
                continue;
            }
            let physical = physical_worker_capacity_for_profile(building, profile);
            if physical == 0 || self.closure_ceilings[index] == 0 {
                continue;
            }
            let units = &mut self.customer_units
                [self.customer_offsets[index]..self.customer_offsets[index + 1]];
            #[cfg(test)]
            let quote_start = timing.then(std::time::Instant::now);
            self.household_quotes
                .reserve(index, &mut self.customers, units, &household_census);
            #[cfg(test)]
            if let Some(start) = quote_start {
                self.household_detail[0] += start.elapsed().as_secs_f64() * 1000.0;
            }
            let memo_activity = self.closure_activity[index];
            let memo_units = &self.import_memo_units
                [self.customer_offsets[index]..self.customer_offsets[index + 1]];
            #[cfg(test)]
            let replay_enabled = !import_replay_disabled;
            #[cfg(not(test))]
            let replay_enabled = true;
            if replay_enabled
                && self.import_memos[index].matches(
                    self.closure_ceilings[index],
                    memo_activity,
                    units,
                    memo_units,
                )
            {
                let (capacity, activity, headroom) = self.import_memos[index].result();
                self.capacities[index] = capacity;
                self.activities[index] = activity;
                self.headroom[index] = headroom;
                #[cfg(test)]
                {
                    self.import_quote_replays += 1;
                }
                #[cfg(test)]
                let finish_start = timing.then(std::time::Instant::now);
                self.household_quotes.finish(
                    self.capacities[index] > 0,
                    &mut self.customers,
                    &household_census,
                );
                #[cfg(test)]
                if let Some(start) = finish_start {
                    self.household_detail[2] += start.elapsed().as_secs_f64() * 1000.0;
                }
                continue;
            }
            let goods_multiplier =
                freight_goods_multiplier(profile, building.zone_type, tuning, minute_of_day);
            let prices = OperationPrices {
                utilities,
                property_tax: daily_property_tax(building.zone_type, building.level, fiscal),
                imports: tuning.owa_import_price_multiplier
                    * if goods_multiplier.is_finite() {
                        goods_multiplier
                    } else {
                        1.0
                    },
                carrier_quote: self.carrier_quotes[index]
                    .filter(|_| goods_multiplier.is_finite())
                    .unwrap_or(f32::INFINITY),
                truck_payload: tuning.logistics.truck_payload_kg,
            };
            let exports = building_outputs_can_export_to_owa(building, profile)
                && self.carrier_quotes[index].is_some()
                && tuning.owa_export_price_multiplier > 0.0;
            let shares = &mut self.import_plans.shares
                [self.customer_offsets[index]..self.customer_offsets[index + 1]];
            let area = output_area_scale_for_building(building, profile);
            for (share, port) in shares.iter_mut().zip(&profile.outputs) {
                *share = provisional_local_share(
                    profile,
                    area,
                    port.resource_runtime_id,
                    Some(units),
                    &self.market,
                )
                .to_bits();
            }
            #[cfg(test)]
            let staffing_start = timing.then(std::time::Instant::now);
            (self.capacities[index], self.activities[index]) = funded_private_operation(
                building,
                profile,
                catalog,
                physical,
                prices,
                &self.market,
                exports.then_some(tuning.owa_export_price_multiplier),
                self.freight_capital[index],
                None,
                Some(units),
                &mut self.headroom[index],
                self.closure_ceilings[index],
                self.closure_activity[index],
                Some(ImportPlanRecord {
                    row: &mut self.import_plans.rows[index],
                    shares,
                    previous: ctx
                        .previous
                        .and_then(|funding| funding.import_plans.get(index, catalog)),
                }),
            );
            #[cfg(test)]
            if let Some(start) = staffing_start {
                self.household_detail[1] += start.elapsed().as_secs_f64() * 1000.0;
            }
            self.import_memo_units[self.customer_offsets[index]..self.customer_offsets[index + 1]]
                .copy_from_slice(units);
            self.import_memos[index].remember(
                self.closure_ceilings[index],
                memo_activity,
                (self.capacities[index], self.activities[index]),
                self.headroom[index],
            );
            #[cfg(test)]
            let finish_start = timing.then(std::time::Instant::now);
            self.household_quotes.finish(
                self.capacities[index] > 0,
                &mut self.customers,
                &household_census,
            );
            #[cfg(test)]
            if let Some(start) = finish_start {
                self.household_detail[2] += start.elapsed().as_secs_f64() * 1000.0;
            }
        }
    }

    // Reserves current operations before offering inputs to prospective employees. Supplier
    // phases and building order remain stable within both dependent sweeps. Fully staffed
    // operations already searched their complete range in the current-staff sweep.
    fn census_staffing<F>(
        &mut self,
        ctx: &CensusContext<'_, F>,
        logistics: &mut ShipmentSystem,
        current_staff: bool,
        rows: Range<usize>,
    ) {
        let CensusContext {
            allocator,
            network,
            graph,
            catalog,
            tuning,
            fiscal,
            minute_of_day,
            ..
        } = *ctx;
        let utilities = utilities(catalog, tuning);
        let household_census =
            household_claims::Census::new(allocator, catalog, fiscal, network, graph);
        for slot in rows {
            let index = self.local_order[slot];
            let building = &allocator.buildings[index];
            if !live(building) {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            let physical = physical_worker_capacity_for_profile(building, profile);
            let service = self.service_flags[index];
            let full_ceiling = if service {
                self.capacities[index]
            } else {
                self.closure_ceilings[index]
            };
            if physical == 0
                || full_ceiling == 0
                || (current_staff && building.worker_count == 0)
                || (!current_staff && building.worker_count >= full_ceiling)
            {
                continue;
            }
            if !current_staff {
                // Only this operation's current claim is returned for its larger quote.
                // Other staffed buyers retain their inputs throughout the expansion search.
                self.input_supply.release_owner(index, allocator, graph);
                // A rejected expansion restores the receipts backing its retained plan.
                // A partial local quote that lost to imports has no such backing: its
                // cheaper inventory valuation cannot silently replace the import proof.
                if !self.local_input_backing[index] {
                    self.input_supply.accept();
                }
            }
            if !self.input_supply.offers_inputs(profile) {
                self.input_supply.reject(allocator, graph);
                continue;
            }
            let ceiling = if current_staff {
                full_ceiling.min(building.worker_count)
            } else {
                full_ceiling
            };
            let exports = building_outputs_can_export_to_owa(building, profile)
                && self.carrier_quotes[index].is_some()
                && tuning.owa_export_price_multiplier > 0.0;
            let goods_multiplier =
                freight_goods_multiplier(profile, building.zone_type, tuning, minute_of_day);
            let prices = OperationPrices {
                utilities,
                property_tax: daily_property_tax(building.zone_type, building.level, fiscal),
                imports: tuning.owa_import_price_multiplier
                    * if goods_multiplier.is_finite() {
                        goods_multiplier
                    } else {
                        1.0
                    },
                carrier_quote: self.carrier_quotes[index]
                    .filter(|_| goods_multiplier.is_finite())
                    .unwrap_or(f32::INFINITY),
                truck_payload: tuning.logistics.truck_payload_kg,
            };
            let consumer = profile
                .outputs
                .iter()
                .any(|port| self.customers.consumes(port.resource_runtime_id));
            let units = &mut self.customer_units
                [self.customer_offsets[index]..self.customer_offsets[index + 1]];
            let new_customers = consumer && self.capacities[index] == 0;
            if new_customers {
                self.household_quotes
                    .reserve(index, &mut self.customers, units, &household_census);
            }
            let input_funding = InputFunding {
                supply: &mut self.input_supply,
                logistics,
                allocator,
                network,
                graph,
                tuning,
                buyer: index,
                outside: self.carrier_quotes[index],
                retained: if current_staff {
                    (0, 0.0)
                } else {
                    (self.capacities[index], self.activities[index])
                },
                goods_multiplier,
            };
            if service {
                // Public payroll follows fiscal policy even when material delivery limits
                // production. Reserve only the feasible input-backed part of that operation.
                let activity = reserve_service_inputs(
                    profile,
                    catalog,
                    ceiling,
                    physical,
                    prices,
                    input_funding,
                );
                self.activities[index] = if current_staff {
                    self.activities[index].max(activity)
                } else {
                    activity
                };
                continue;
            }
            let mut headroom = self.headroom[index];
            let operation = funded_private_operation(
                building,
                profile,
                catalog,
                physical,
                prices,
                &self.market,
                exports.then_some(tuning.owa_export_price_multiplier),
                self.freight_capital[index],
                Some(input_funding),
                consumer.then_some(units),
                &mut headroom,
                ceiling,
                self.closure_activity[index],
                None,
            );
            if !current_staff || operation.0 >= self.capacities[index] {
                (self.capacities[index], self.activities[index]) = operation;
                self.headroom[index] = headroom;
                if current_staff {
                    self.local_input_backing[index] = operation.0 > 0;
                }
            } else if building.worker_count >= full_ceiling {
                // The complete local search lost to the existing import alternative.
                self.input_supply.release_owner(index, allocator, graph);
                self.input_supply.accept();
            }
            if new_customers {
                self.household_quotes.finish(
                    self.capacities[index] > 0,
                    &mut self.customers,
                    &household_census,
                );
            }
        }
    }

    fn log_census<F>(&self, ctx: &CensusContext<'_, F>, progress: &CensusProgress) {
        let timing_category = timing_category();
        let count = ctx.allocator.buildings.len();
        let minute_of_day = ctx.minute_of_day;
        log_funding_times(timing_category, count, minute_of_day, &progress.times);
        #[cfg(test)]
        {
            crate::debug_log!(
                timing_category,
                "funding_census_detail passes={} propagation_rounds={}",
                self.closure_passes,
                self.propagation_rounds
            );
            let searches = FUNDING_SEARCH_COUNTS
                .each_ref()
                .map(|count| count.load(std::sync::atomic::Ordering::Relaxed));
            crate::debug_log!(
                timing_category,
                "funding_search_detail counts={:?}",
                searches
            );
            crate::debug_log!(
                timing_category,
                "household_import_funding_detail buildings={} minute={} reserve_ms={:.3} staffing_ms={:.3} finish_ms={:.3}",
                count,
                minute_of_day,
                self.household_detail[0],
                self.household_detail[1],
                self.household_detail[2]
            );
        }
    }
}
