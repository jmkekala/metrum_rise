// SPDX-License-Identifier: GPL-2.0-only

//! The funding census as resumable steps (`SIM-02-I`, `docs/simulation.md#pure-jobs`).
//!
//! [`JobFunding::refresh`] and its variants run every step at once. The hourly census runs the
//! same steps over the hour's second half on the input copy, a bounded slice per slot
//! (`hiring/rolling.rs`). Every step reads only the census inputs and the census's own books,
//! and a slice of a loop processes the same rows in the same order as the whole loop, so a
//! census stepped over any slices equals one run at once bit for bit.
//!
//! **Steps**, in order: route quotes (their start with the border sweeps; when the quote
//! inputs changed, the door access and the quotes, both sliced) and paid freight; the
//! production market (its 256-building chunks sliced); the consumer-market refresh (its start;
//! the household budgets, sliced and serial; the access rows, sliced; after a membership change
//! the index entries, sliced, the index build one mode and the copies one category at least a
//! slice, and the reservation positions, sliced; its finish, which resets the claims of a book
//! not retained); setup (port offsets and counts, sliced; the import plans' layout; the
//! supplier order and provisional shares, sliced; the household-quote preparation); the
//! initial ceilings (sliced). Then each closure pass: import funding (sliced, parallel);
//! household claims (sliced, serial); the input-source reset (its start; after a geometry
//! change the supplier layout, sliced, the index's entries and the index one resource at least
//! a slice; the production rates and availability, sliced); household production (sliced); the
//! current-staff and expansion staffing sweeps (sliced, serial in supplier order); finishing
//! the pass's household quotes; publishing funded activity and household production (both
//! sliced); the provisional-input repair; business validation (seeding the accounts: its start
//! and the buyers, sliced and serial; the book's layout: its start, after a layout change the
//! buyers' access and recipe ports, sliced, the index entries and the indices one resource at
//! least a slice, and the accounts' availability; the claims sliced and serial by seller, the
//! proof sliced, publishing the financing); and settling, or propagating impossible customers
//! into the next pass.
//!
//! **Costs** (`SIM-02-K`). Work is counted in work units, a deterministic estimate of census
//! work in nominal nanoseconds made from counts alone, never from measured time: each step's
//! rows at the step's weight ([`Step::row_work`]: buildings for building passes, households for
//! the consumer-market refresh), and, in the serial claim and staffing sweeps, the searches the
//! row made ([`WORK_WEIGHTS`]: network route searches, supplier searches, candidates and
//! routes, input quotes, consumer candidates and fallbacks, business claims' scan events and
//! carrier quotes, funded-operation searches). A
//! uniform sliced step takes its budget's rows; a serial sweep runs rows until their work
//! reaches the budget, so one slice holds at most one row past it; an unsliced step runs whole
//! once started and starts only in a slot that has done no other step, unless the budget
//! covers its estimate. [`CensusProgress::remaining_work`] estimates the work left, which the
//! hourly driver combines with the previous census's work per step
//! ([`CensusProgress::planned_work`]). Slicing never changes a result; a restart inside the
//! consumer-market refresh leaves its book torn, and the restarted refresh withdraws it, as a
//! load does; route quotes and a supplier index left part way are named valid only once
//! complete, and a business seeding always begins with its book's reset, so a restart redoes
//! each. Steps allocate nothing once the census storage has grown, except the supplier index's
//! buckets after a geometry change.

use std::ops::Range;

use super::*;
use crate::simulation::economy::customers::business::SeedingLayout;
use crate::simulation::economy::customers::{RefreshInputs, RefreshState};

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
pub(crate) const STEP_COUNT: usize = Step::Done as usize + 1;

/// Work units a census used in each step, over all its closure passes (`SIM-02-K`); the next
/// census's slices plan for them. [`Default`] is none.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StepWork([u64; STEP_COUNT]);

impl Default for StepWork {
    fn default() -> Self {
        Self([0; STEP_COUNT])
    }
}

/// Step names in order, for the step-cost measurement.
#[cfg(test)]
pub(crate) const STEP_NAMES: [&str; STEP_COUNT] = [
    "routes",
    "route_borders",
    "route_access",
    "route_quotes",
    "market_begin",
    "market_chunks",
    "market_end",
    "customers_begin",
    "customer_budgets",
    "customer_access",
    "customer_index_entries",
    "customer_index_build",
    "customer_index_copy",
    "customer_reservations",
    "customers_finish",
    "customer_claim_funds",
    "customer_claim_availability",
    "customers_end",
    "setup_count",
    "setup_plans",
    "setup_order",
    "setup_finish",
    "ceilings",
    "pass_start",
    "import_funding",
    "household_claims",
    "sources",
    "sources_layout",
    "sources_entries",
    "sources_index",
    "sources_rates",
    "source_production",
    "current_staff",
    "expansion",
    "finish_pass",
    "publish_activity",
    "publish_production",
    "repair",
    "business_seed",
    "business_seed_rows",
    "business_layout",
    "business_layout_access",
    "business_layout_ports",
    "business_layout_entries",
    "business_layout_indices",
    "business_accounts",
    "business_claims",
    "business_proof",
    "business_publish",
    "settle",
    "done",
];

/// Work-unit weights of the counted searches (`SIM-02-K`), in nominal nanoseconds, fitted to
/// per-row timings of the serial sweeps at 100k (release, eight workers): a network route
/// search, a supplier search, a supplier candidate, a supplier route priced, an input quote,
/// a consumer candidate, a consumer trip fallback, a business claim's scan event, a business
/// carrier quote and a funded-operation search (the last is the staffing sweeps' own).
const WORK_WEIGHTS: [u64; 10] = [15_000, 500, 40, 30, 100, 50, 5_000, 270, 150, 2_000];

/// Work units an expansion operation's priced supplier route adds to its weight in
/// [`WORK_WEIGHTS`] (`SIM-02-K`): fitted to the expansion sweep's slices at 100k (release, eight
/// workers), where a priced route costs about 230 ns against 30 in the current-staff sweep.
const EXPANSION_ROUTE_EXTRA: u64 = 200;

// The weighted counted work of a census's books and route cache so far; differences between
// two readings are a span's counted work. O(1).
fn counted_work(funding: &JobFunding, logistics: &ShipmentSystem) -> u64 {
    let [searches, candidates, routes, quotes] = funding.input_supply.work_counts();
    let [consumers, fallbacks] = funding.customers.claim_work();
    let [scanned, priced] = funding.business.claim_work();
    [
        logistics.route_searches(),
        searches,
        candidates,
        routes,
        quotes,
        consumers,
        fallbacks,
        scanned,
        priced,
    ]
    .iter()
    .zip(WORK_WEIGHTS)
    .fold(0_u64, |sum, (&count, weight)| {
        sum.wrapping_add(count.wrapping_mul(weight))
    })
}

/// A census step; see the module documentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Step {
    #[default]
    Routes,
    RouteBorders,
    RouteAccess,
    RouteQuotes,
    MarketBegin,
    MarketChunks,
    MarketEnd,
    CustomersBegin,
    CustomerBudgets,
    CustomerAccess,
    CustomerIndexEntries,
    CustomerIndexBuild,
    CustomerIndexCopy,
    CustomerReservations,
    CustomersFinish,
    CustomerClaimFunds,
    CustomerClaimAvailability,
    CustomersEnd,
    SetupCount,
    SetupPlans,
    SetupOrder,
    SetupFinish,
    Ceilings,
    PassStart,
    ImportFunding,
    HouseholdClaims,
    Sources,
    SourcesLayout,
    SourcesEntries,
    SourcesIndex,
    SourcesRates,
    SourceProduction,
    CurrentStaff,
    Expansion,
    FinishPass,
    PublishActivity,
    PublishProduction,
    Repair,
    BusinessSeed,
    BusinessSeedRows,
    BusinessLayout,
    BusinessLayoutAccess,
    BusinessLayoutPorts,
    BusinessLayoutEntries,
    BusinessLayoutIndices,
    BusinessAccounts,
    BusinessClaims,
    BusinessProof,
    BusinessPublish,
    Settle,
    Done,
}

impl Step {
    // Work units a row of the step costs (`SIM-02-K`): nominal nanoseconds at 100k, release,
    // eight workers; for the serial sweeps the row alone, before its counted searches.
    fn row_work(self) -> u64 {
        use Step::*;
        match self {
            // The route quotes' start: the cache's storage after a routing change.
            Routes => 10,
            // Per border swept after a routing change; the estimate counts none.
            RouteBorders => 400_000,
            // A door projected after an entrance change costs about 130; most rows only check.
            RouteAccess => 100,
            RouteQuotes => 20,
            MarketChunks => 10,
            CustomersBegin => 2,
            CustomerBudgets => 30,
            // A row that resamples its trip ends costs about 90; most rows only compare.
            CustomerAccess => 90,
            CustomerIndexEntries => 15,
            CustomerIndexBuild => 35,
            CustomerIndexCopy => 30,
            CustomerReservations => 10,
            // The finish's decision and, for a book not retained, the receipt books' reset.
            CustomersFinish => 5,
            CustomerClaimFunds => 15,
            CustomerClaimAvailability => 30,
            CustomersEnd => 2,
            SetupCount => 20,
            SetupPlans => 25,
            SetupOrder => 15,
            SetupFinish => 15,
            Ceilings => 10,
            ImportFunding => 20,
            HouseholdClaims | CurrentStaff | Expansion => 25,
            // The business claims' row alone, before its counted scan and quotes.
            BusinessClaims => 20,
            Sources => 3,
            SourcesLayout => 15,
            // The supplier index's entries, sorted, after a geometry change.
            SourcesEntries => 15,
            // Per supplier index entry built; the estimate counts one a building.
            SourcesIndex => 15,
            // After a geometry change most rows toggle their availability.
            SourcesRates => 40,
            SourceProduction | PublishActivity | PublishProduction => 10,
            Repair => 40,
            BusinessSeed => 2,
            BusinessSeedRows => 50,
            BusinessLayout => 3,
            BusinessLayoutAccess => 20,
            BusinessLayoutPorts => 10,
            BusinessLayoutEntries => 5,
            BusinessLayoutIndices => 15,
            BusinessAccounts => 10,
            BusinessProof => 10,
            Settle => 20,
            MarketBegin | MarketEnd | PassStart | FinishPass | BusinessPublish | Done => 0,
        }
    }

    // Whether the step runs whole once started.
    fn unsliced(self) -> bool {
        use Step::*;
        matches!(
            self,
            Routes
                | CustomersBegin
                | CustomersFinish
                | CustomersEnd
                | SetupPlans
                | SetupFinish
                | Sources
                | SourcesEntries
                | Repair
                | BusinessSeed
                | BusinessLayout
                | BusinessLayoutEntries
                | BusinessAccounts
                | Settle
        )
    }

    // A lower bound of the step's work: its rows at its weight.
    fn work(self, buildings: usize, households: usize) -> u64 {
        self.rows(buildings, households) as u64 * self.row_work()
    }

    // Rows of work: buildings for building passes, households for the consumer market.
    fn rows(self, buildings: usize, households: usize) -> usize {
        match self {
            Self::RouteBorders
            | Self::MarketBegin
            | Self::MarketEnd
            | Self::PassStart
            | Self::FinishPass
            | Self::BusinessPublish
            | Self::Done => 0,
            Self::CustomerBudgets
            | Self::CustomerAccess
            | Self::CustomerIndexEntries
            | Self::CustomerIndexBuild
            | Self::CustomerIndexCopy
            | Self::CustomerReservations
            | Self::CustomersFinish
            | Self::CustomerClaimFunds
            | Self::CustomerClaimAvailability
            | Self::CustomersEnd => households,
            _ => buildings,
        }
    }

    // The step after this one when the pass continues in order.
    fn next(self) -> Self {
        use Step::*;
        match self {
            Routes => RouteBorders,
            RouteBorders => RouteAccess,
            RouteAccess => RouteQuotes,
            RouteQuotes => MarketBegin,
            MarketBegin => MarketChunks,
            MarketChunks => MarketEnd,
            MarketEnd => CustomersBegin,
            CustomersBegin => CustomerBudgets,
            CustomerBudgets => CustomerAccess,
            CustomerAccess => CustomerIndexEntries,
            CustomerIndexEntries => CustomerIndexBuild,
            CustomerIndexBuild => CustomerIndexCopy,
            CustomerIndexCopy => CustomerReservations,
            CustomerReservations => CustomersFinish,
            CustomersFinish => CustomerClaimFunds,
            CustomerClaimFunds => CustomerClaimAvailability,
            CustomerClaimAvailability => CustomersEnd,
            CustomersEnd => SetupCount,
            SetupCount => SetupPlans,
            SetupPlans => SetupOrder,
            SetupOrder => SetupFinish,
            SetupFinish => Ceilings,
            Ceilings => PassStart,
            PassStart => ImportFunding,
            ImportFunding => HouseholdClaims,
            HouseholdClaims => Sources,
            Sources => SourcesLayout,
            SourcesLayout => SourcesEntries,
            SourcesEntries => SourcesIndex,
            SourcesIndex => SourcesRates,
            SourcesRates => SourceProduction,
            SourceProduction => CurrentStaff,
            CurrentStaff => Expansion,
            Expansion => FinishPass,
            FinishPass => PublishActivity,
            PublishActivity => PublishProduction,
            PublishProduction => Repair,
            Repair => BusinessSeed,
            BusinessSeed => BusinessSeedRows,
            BusinessSeedRows => BusinessLayout,
            BusinessLayout => BusinessLayoutAccess,
            BusinessLayoutAccess => BusinessLayoutPorts,
            BusinessLayoutPorts => BusinessLayoutEntries,
            BusinessLayoutEntries => BusinessLayoutIndices,
            BusinessLayoutIndices => BusinessAccounts,
            BusinessAccounts => BusinessClaims,
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
            Routes | RouteBorders | RouteAccess | RouteQuotes => 0,
            MarketBegin | MarketChunks | MarketEnd => 1,
            CustomersBegin
            | CustomerBudgets
            | CustomerAccess
            | CustomerIndexEntries
            | CustomerIndexBuild
            | CustomerIndexCopy
            | CustomerReservations
            | CustomersFinish
            | CustomerClaimFunds
            | CustomerClaimAvailability
            | CustomersEnd => 2,
            SetupCount | SetupPlans | SetupOrder | SetupFinish | Ceilings | PassStart
            | ImportFunding => 3,
            HouseholdClaims => 4,
            Sources | SourcesLayout | SourcesEntries | SourcesIndex | SourcesRates
            | SourceProduction => 5,
            CurrentStaff | Expansion => 6,
            FinishPass | PublishActivity | PublishProduction => 7,
            Repair => 8,
            BusinessSeed
            | BusinessSeedRows
            | BusinessLayout
            | BusinessLayoutAccess
            | BusinessLayoutPorts
            | BusinessLayoutEntries
            | BusinessLayoutIndices
            | BusinessAccounts
            | BusinessClaims
            | BusinessProof
            | BusinessPublish => 9,
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
    // The revision the route quotes are being rewritten for, `None` when they are kept
    // (`SIM-02-K`).
    quote_revision: Option<(u64, u64, u32, u64, u64)>,
    // The consumer-market refresh in progress (`SIM-02-K`).
    customers: RefreshState,
    // The business book's layout in progress after seeding (`SIM-02-K`).
    business_layout: SeedingLayout,
    retained_customers: bool,
    // Setup's running counts (`SIM-02-K`): output ports, input ports, consumer firms, their
    // output ports and the most output ports of one consumer firm.
    setup: [usize; 5],
    input_ports: usize,
    first_pass: bool,
    // Closure passes started; slicing only, never read by the census itself.
    passes: usize,
    // Work units used by the current step, and by each step so far (`SIM-02-K`); slicing
    // only, never read by the census itself.
    step_used: u64,
    step_work: StepWork,
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

    /// An estimate of the work units left assuming the census takes `expected` closure passes,
    /// as the previous one did (`SIM-02-I`): the rows of the current pass's steps and those
    /// after it, and whole passes for the ones still expected, at each step's row weight
    /// (`SIM-02-K`). A serial sweep in progress prices its rows left at the rate its rows done
    /// used, searches included. It is not a bound either way: later sweeps' searches are not
    /// counted, and every step the census may still run is, including steps it decides to
    /// skip once it reaches them (the route quotes and the source index when their inputs are
    /// unchanged, the customer index steps without a rebuild, the steps after a failed repair
    /// or seeding). The hourly driver takes the larger of this and the previous census's work
    /// per step; either error only moves work between slots, never a result. O(steps).
    pub(crate) fn remaining_work(
        &self,
        buildings: usize,
        households: usize,
        expected: usize,
    ) -> u64 {
        if self.step == Step::Done {
            return 0;
        }
        let done = if self.step == Step::MarketChunks {
            (self.cursor * MARKET_CHUNK).min(buildings)
        } else {
            self.cursor
        };
        let rate = if matches!(
            self.step,
            Step::HouseholdClaims | Step::CurrentStaff | Step::Expansion | Step::BusinessClaims
        ) && done > 0
        {
            (self.step_used / done as u64).max(self.step.row_work())
        } else {
            self.step.row_work()
        };
        let mut work = self.step.rows(buildings, households).saturating_sub(done) as u64 * rate;
        let mut step = self.step.next();
        while step != Step::Done {
            work += step.work(buildings, households);
            step = step.next();
        }
        let mut pass = 0;
        let mut step = Step::PassStart;
        while step != Step::Done {
            pass += step.work(buildings, households);
            step = step.next();
        }
        work + expected.saturating_sub(self.passes.max(1)) as u64 * pass
    }

    /// Work units this census has used in each step so far (`SIM-02-K`).
    pub(crate) fn step_work(&self) -> StepWork {
        self.step_work
    }

    /// The work left if each step takes what it took in the census that used `expected`
    /// (`SIM-02-K`): per step, its work there less its work here, summed, so a step that takes
    /// more than it did leaves the other steps' plans as they were. O(steps).
    pub(crate) fn planned_work(&self, expected: &StepWork) -> u64 {
        expected
            .0
            .iter()
            .zip(&self.step_work.0)
            .map(|(&expected, &used)| expected.saturating_sub(used))
            .fold(0_u64, u64::saturating_add)
    }

    /// Closure passes started so far.
    pub(crate) fn passes(&self) -> usize {
        self.passes
    }

    /// The next step's name and its rows done, for restart tests.
    #[cfg(test)]
    pub(crate) fn position_for_test(&self) -> (&'static str, usize) {
        (STEP_NAMES[self.step as usize], self.cursor)
    }

    fn advance(&mut self, step: Step) {
        self.step = step;
        self.cursor = 0;
        self.step_used = 0;
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

// The consumer-market refresh's inputs: the census's own (`SIM-02-K`).
fn refresh_inputs<'a, F>(ctx: &CensusContext<'a, F>) -> RefreshInputs<'a> {
    RefreshInputs {
        households: ctx.households,
        allocator: ctx.allocator,
        catalog: ctx.catalog,
        tuning: ctx.tuning,
        policy: ctx.fiscal,
        treasury: ctx.treasury_balance,
        network: ctx.network,
        graph: ctx.graph,
    }
}

fn utilities(catalog: &RuntimeEconomyCatalog, tuning: &RuntimeEconomyTuning) -> [f32; 3] {
    local_utility_service_costs(catalog).map(|price| price * tuning.owa_import_price_multiplier)
}

impl JobFunding {
    /// Runs census steps until `budget` work units are used or every step has run; returns
    /// whether the census is done. `u64::MAX` runs it to the end. An unsliced step whose rows
    /// alone exceed what is left of the budget waits for the next call unless it is the call's
    /// first step (`SIM-02-K`).
    pub(super) fn run_census_steps<F>(
        &mut self,
        ctx: &CensusContext<'_, F>,
        logistics: &mut ShipmentSystem,
        progress: &mut CensusProgress,
        budget: u64,
    ) -> bool
    where
        F: Fn(usize, u32, &EconomyProfileRuntime) -> u32 + Sync,
    {
        #[cfg(test)]
        let _contract_phase = crate::debug::work::phases::enter("census");
        // The steps rewrite the funded rows (`SIM-02-K`).
        self.rows_generation = super::RowsGeneration::next();
        let timing = timing_category().enabled();
        let mut left = budget;
        while progress.step != Step::Done && left > 0 {
            if left < budget
                && progress.step.unsliced()
                && progress
                    .step
                    .work(ctx.allocator.buildings.len(), ctx.households.len())
                    > left
            {
                break;
            }
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
            progress.step_work.0[step as usize] =
                progress.step_work.0[step as usize].saturating_add(used);
            if progress.step == step {
                progress.step_used = progress.step_used.saturating_add(used);
            }
            left = left.saturating_sub(used);
            if progress.step == Step::Done && timing {
                self.log_census(ctx, progress);
            }
        }
        progress.step == Step::Done
    }

    // One step or slice within `budget` work units; returns the work units it used.
    fn census_step<F>(
        &mut self,
        ctx: &CensusContext<'_, F>,
        logistics: &mut ShipmentSystem,
        progress: &mut CensusProgress,
        budget: u64,
    ) -> u64
    where
        F: Fn(usize, u32, &EconomyProfileRuntime) -> u32 + Sync,
    {
        let buildings = ctx.allocator.buildings.len();
        let step = progress.step;
        let whole = step.work(buildings, ctx.households.len());
        // Rows of a uniform sliced step the budget covers, at least one.
        let budget_rows = usize::try_from(budget / step.row_work().max(1))
            .unwrap_or(usize::MAX)
            .max(1);
        match step {
            Step::Routes => {
                self.census_routes(ctx, logistics, progress);
                if progress.quote_revision.is_none() {
                    progress.advance(Step::MarketBegin);
                    return whole;
                }
            }
            Step::RouteBorders => {
                // Borders until their sweeps reach the budget, one at least; a border swept
                // since the last routing change costs nothing. The last border's slice, or an
                // empty one without borders, takes their minimum.
                let borders = self.border_nodes.len();
                let mut used = 0;
                loop {
                    let range = progress.cursor..(progress.cursor + 1).min(borders);
                    progress.cursor = range.end;
                    let swept = logistics.sweep_carrier_borders(
                        ctx.network,
                        ctx.graph,
                        &self.border_nodes,
                        range,
                    );
                    used += swept as u64 * step.row_work();
                    if progress.cursor >= borders || used >= budget {
                        break;
                    }
                }
                progress.finish_slice(borders);
                return used;
            }
            Step::RouteAccess | Step::RouteQuotes => {
                let rows = progress.slice(buildings, budget_rows);
                let used = rows.len() as u64 * step.row_work();
                let CensusContext {
                    allocator,
                    network,
                    graph,
                    catalog,
                    tuning,
                    ..
                } = *ctx;
                if step == Step::RouteAccess {
                    logistics
                        .prepare_border_carrier_access(allocator, network, graph, catalog, rows);
                } else {
                    logistics.write_border_carrier_quotes_in(
                        allocator,
                        network,
                        graph,
                        catalog,
                        tuning,
                        rows.clone(),
                        &mut self.carrier_quotes[rows],
                    );
                    if progress.cursor >= buildings {
                        // Valid only once every row is written: a census restarted inside
                        // the quotes rewrites them.
                        self.quote_revision = progress.quote_revision;
                        self.quoted_border_nodes.clone_from(&self.border_nodes);
                    }
                }
                progress.finish_slice(buildings);
                return used;
            }
            Step::MarketBegin => self.census_market_begin(ctx, progress),
            Step::MarketChunks => {
                let chunks = buildings.div_ceil(MARKET_CHUNK);
                let slice = progress.slice(chunks, budget_rows.div_ceil(MARKET_CHUNK));
                let used = (slice.len() * MARKET_CHUNK).min(buildings) as u64 * step.row_work();
                self.census_market_chunks(ctx, slice);
                progress.finish_slice(chunks);
                return used;
            }
            Step::MarketEnd => self.census_market_end(ctx, progress),
            Step::CustomersBegin => {
                progress.customers = self.customers.begin_refresh(&refresh_inputs(ctx));
            }
            Step::CustomerIndexEntries
                if progress.cursor == 0
                    && !self
                        .customers
                        .begin_refresh_indices(&mut progress.customers) =>
            {
                // No rebuild: no entries, indices or reservations to write.
                progress.advance(Step::CustomersFinish);
                return 0;
            }
            Step::CustomerBudgets
            | Step::CustomerAccess
            | Step::CustomerIndexEntries
            | Step::CustomerReservations
            | Step::CustomerClaimFunds => {
                let households = ctx.households.len();
                let rows = progress.slice(households, budget_rows);
                let used = rows.len() as u64 * step.row_work();
                match step {
                    Step::CustomerBudgets => self.customers.refresh_budgets(
                        &mut progress.customers,
                        &refresh_inputs(ctx),
                        rows,
                    ),
                    Step::CustomerAccess => self.customers.refresh_access(
                        &mut progress.customers,
                        &refresh_inputs(ctx),
                        rows,
                    ),
                    Step::CustomerIndexEntries => self
                        .customers
                        .refresh_index_entries(&progress.customers, rows),
                    Step::CustomerReservations => self
                        .customers
                        .refresh_reservations(&progress.customers, rows),
                    _ => self
                        .customers
                        .reset_refresh_claims(&progress.customers, rows),
                }
                progress.finish_slice(households);
                return used;
            }
            Step::CustomerIndexBuild
            | Step::CustomerIndexCopy
            | Step::CustomerClaimAvailability => {
                // One mode's build, one category's copy or one index's availability a slice
                // at least (`SIM-02-K`).
                let items = match step {
                    Step::CustomerIndexBuild => 2,
                    Step::CustomerIndexCopy => self.customers.refreshed_index_copies(),
                    _ => self.customers.claim_availability_indices(),
                };
                let item_work = (whole / items.max(1) as u64).max(1);
                let count = usize::try_from(budget / item_work)
                    .unwrap_or(usize::MAX)
                    .max(1);
                let range = progress.slice(items, count);
                let used = range.len() as u64 * item_work;
                match step {
                    Step::CustomerIndexBuild => self
                        .customers
                        .build_refreshed_modes(&progress.customers, range),
                    Step::CustomerIndexCopy => self
                        .customers
                        .copy_refreshed_categories(&progress.customers, range),
                    _ => self
                        .customers
                        .reset_refresh_availability(&progress.customers, range),
                }
                progress.finish_slice(items);
                return used;
            }
            Step::CustomersFinish => {
                self.census_customers(ctx, progress);
                if progress.retained_customers {
                    // A retained book keeps its claims: nothing to reset.
                    progress.advance(Step::CustomersEnd);
                    return whole;
                }
            }
            Step::CustomersEnd => {
                self.customers
                    .end_refresh(&progress.customers, &refresh_inputs(ctx));
            }
            Step::SetupCount | Step::SetupOrder => {
                let rows = progress.slice(buildings, budget_rows);
                let used = rows.len() as u64 * step.row_work();
                if step == Step::SetupCount {
                    self.census_setup_count(ctx, progress, rows);
                } else {
                    self.census_setup_order(ctx, rows);
                }
                progress.finish_slice(buildings);
                return used;
            }
            Step::SetupPlans => self.census_setup_plans(ctx, progress),
            Step::SetupFinish => self.census_setup_finish(ctx, progress),
            Step::HouseholdClaims | Step::CurrentStaff | Step::Expansion => {
                let start = progress.cursor.min(buildings);
                let (end, used) = if step == Step::HouseholdClaims {
                    self.census_household_claims(ctx, logistics, start, budget)
                } else {
                    self.census_staffing(ctx, logistics, step == Step::CurrentStaff, start, budget)
                };
                progress.cursor = end;
                progress.finish_slice(buildings);
                return used;
            }
            Step::BusinessClaims => {
                // A serial sweep: sellers until their counted work reaches the budget.
                let start = progress.cursor.min(buildings);
                let counted = counted_work(self, logistics);
                let work = |funding: &Self, logistics: &ShipmentSystem, rows: usize| {
                    rows as u64 * step.row_work()
                        + counted_work(funding, logistics).wrapping_sub(counted)
                };
                match self.claim_business_customers(
                    ctx.allocator,
                    logistics,
                    ctx.network,
                    ctx.graph,
                    ctx.catalog,
                    ctx.tuning,
                    start,
                    budget,
                    work,
                ) {
                    Ok((end, used)) => {
                        progress.cursor = end;
                        progress.finish_slice(buildings);
                        return used;
                    }
                    Err((required, used)) => {
                        // Grow between attempts, then seed and claim again in order.
                        self.business.reserve_receipts(required);
                        progress.advance(Step::BusinessSeed);
                        return used;
                    }
                }
            }
            Step::Ceilings
            | Step::ImportFunding
            | Step::SourceProduction
            | Step::PublishActivity
            | Step::PublishProduction
            | Step::BusinessProof => {
                let rows = progress.slice(buildings, budget_rows);
                let used = rows.len() as u64 * step.row_work();
                match step {
                    Step::Ceilings => self.census_ceilings(ctx, rows),
                    Step::ImportFunding => self.census_import_funding(ctx, rows),
                    Step::SourceProduction => self.reserve_household_production(
                        ctx.allocator,
                        ctx.catalog,
                        ctx.graph,
                        true,
                        rows,
                    ),
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
                if !self.input_supply.begin_reset(
                    ctx.allocator,
                    ctx.graph,
                    ctx.network,
                    ctx.catalog,
                ) {
                    // The supplier layout and index stand: only the rates change.
                    progress.advance(Step::SourcesRates);
                    return whole;
                }
            }
            Step::SourcesEntries => {
                self.input_supply
                    .reset_entries(ctx.allocator, ctx.graph, ctx.catalog);
            }
            Step::SourcesIndex => {
                // Whole resources until their entries reach the budget, one at least.
                let resources = self.input_supply.reset_index_resources();
                let mut used = 0;
                while progress.cursor < resources && used < budget {
                    let built = self
                        .input_supply
                        .build_reset_index(progress.cursor..progress.cursor + 1);
                    progress.cursor += 1;
                    used += built as u64 * step.row_work();
                }
                progress.finish_slice(resources);
                return used;
            }
            Step::SourcesLayout | Step::SourcesRates | Step::BusinessSeedRows => {
                let rows = progress.slice(buildings, budget_rows);
                let used = rows.len() as u64 * step.row_work();
                if step == Step::SourcesLayout {
                    self.input_supply
                        .reset_layout(ctx.allocator, ctx.catalog, rows);
                } else if step == Step::SourcesRates {
                    self.input_supply.reset_rates(
                        ctx.allocator,
                        ctx.graph,
                        ctx.catalog,
                        &self.offered_activity,
                        rows,
                    );
                } else {
                    self.seed_business_rows(
                        ctx.allocator,
                        ctx.catalog,
                        ctx.tuning,
                        ctx.fiscal,
                        ctx.minute_of_day,
                        rows,
                    );
                }
                progress.finish_slice(buildings);
                return used;
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
                if !self.begin_business_seeding(ctx.allocator, progress.input_ports) {
                    progress.settled = false;
                    progress.advance(Step::Settle);
                    return whole;
                }
                progress.reduced = false;
            }
            Step::BusinessLayout => {
                progress.business_layout =
                    self.business
                        .begin_layout(ctx.allocator, ctx.network, ctx.catalog);
                if !progress.business_layout.rebuilds() {
                    progress.advance(Step::BusinessAccounts);
                    return whole;
                }
                if !progress.business_layout.changes_access() {
                    progress.advance(Step::BusinessLayoutPorts);
                    return whole;
                }
            }
            Step::BusinessLayoutAccess | Step::BusinessLayoutPorts => {
                let rows = progress.slice(buildings, budget_rows);
                let used = rows.len() as u64 * step.row_work();
                let layout = progress.business_layout;
                if step == Step::BusinessLayoutAccess {
                    let components = self
                        .input_supply
                        .freight_components()
                        .expect("seeding begins only with freight components");
                    self.business.layout_access(
                        &layout,
                        ctx.allocator,
                        ctx.graph,
                        components,
                        rows,
                    );
                } else {
                    self.business
                        .layout_ports(&layout, ctx.allocator, ctx.catalog, rows);
                }
                progress.finish_slice(buildings);
                return used;
            }
            Step::BusinessLayoutEntries => {
                self.business
                    .collect_layout_entries(&progress.business_layout);
            }
            Step::BusinessLayoutIndices => {
                // One resource's index a slice at least.
                let items = progress.business_layout.resources();
                let item_work = (whole / items.max(1) as u64).max(1);
                let count = usize::try_from(budget / item_work)
                    .unwrap_or(usize::MAX)
                    .max(1);
                let range = progress.slice(items, count);
                let used = range.len() as u64 * item_work;
                self.business
                    .build_layout_indices(&progress.business_layout, range);
                progress.finish_slice(items);
                return used;
            }
            Step::BusinessAccounts => {
                self.business
                    .finish_layout(&progress.business_layout, ctx.allocator);
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

    // The route quotes' start: the census's rows, the pricing phase and the connected borders;
    // when the quote inputs changed, the quotes' first phase, the rows to rewrite recorded in
    // `progress` (`SIM-02-K`). O(B) resizes, plus the border sweeps after a routing change.
    fn census_routes<F>(
        &mut self,
        ctx: &CensusContext<'_, F>,
        logistics: &mut ShipmentSystem,
        progress: &mut CensusProgress,
    ) {
        #[cfg(test)]
        {
            self.import_quote_replays = 0;
            self.household_detail = [0.0; 3];
        }
        let CensusContext {
            allocator,
            network,
            graph,
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
        progress.quote_revision = None;
        if self.quote_revision != Some(revision)
            || self.carrier_quotes.len() != count
            || self.border_nodes != self.quoted_border_nodes
        {
            self.quote_revision = None;
            self.carrier_quotes.resize(count, None);
            logistics.begin_border_carrier_quotes(allocator, network, graph);
            progress.quote_revision = Some(revision);
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
        progress.retained_customers = self
            .customers
            .begin_refresh_finish(&mut progress.customers, &refresh_inputs(ctx));
        // A refresh that reset the book ends the recorded quotes' validity now, not at setup:
        // a census restarted before setup refreshes again, finds its inputs unchanged and
        // would otherwise replay the previous census's quotes against the reset book.
        if !progress.retained_customers {
            self.household_quotes.forget();
        }
    }

    // Setup's first pass over buildings `rows`, in order: customer port offsets, funding-phase
    // counts and the port counts, carried between slices (`SIM-02-K`). O(rows).
    fn census_setup_count<F>(
        &mut self,
        ctx: &CensusContext<'_, F>,
        progress: &mut CensusProgress,
        rows: Range<usize>,
    ) {
        let allocator = ctx.allocator;
        let catalog = ctx.catalog;
        if rows.start == 0 {
            let count = allocator.buildings.len();
            self.record_operating(&allocator.buildings);
            self.customer_offsets.resize(count + 1, 0);
            self.local_order.resize(count, 0);
            self.phase_offsets.resize(catalog.funding_phase_count(), 0);
            self.phase_offsets.fill(0);
            progress.setup = [0; 5];
        }
        let [
            ports,
            inputs,
            consumer_firms,
            consumer_ports,
            maximum_consumer_ports,
        ] = &mut progress.setup;
        for (index, building) in allocator.buildings[rows.clone()]
            .iter()
            .enumerate()
            .map(|(offset, building)| (rows.start + offset, building))
        {
            self.customer_offsets[index] = *ports;
            self.phase_offsets[catalog.funding_phase(building.economy_profile_runtime_id)] += 1;
            if let Some(profile) =
                catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            {
                *ports += profile.outputs.len();
                *inputs += profile.inputs.len();
                if profile
                    .outputs
                    .iter()
                    .any(|port| self.customers.consumes(port.resource_runtime_id))
                {
                    *consumer_firms += 1;
                    *consumer_ports += profile.outputs.len();
                    *maximum_consumer_ports = (*maximum_consumer_ports).max(profile.outputs.len());
                }
            }
        }
    }

    // The import plans' layout and the funding phases' first slots, between setup's passes.
    fn census_setup_plans<F>(&mut self, ctx: &CensusContext<'_, F>, progress: &CensusProgress) {
        let count = ctx.allocator.buildings.len();
        self.customer_offsets[count] = progress.setup[0];
        self.import_plans
            .prepare(&self.customer_offsets, ctx.catalog);
        // Immutable recipe order is prepared once, not once per closure attempt. Counts
        // and scatter cursors are dependent; stable building order keeps claims deterministic.
        let mut first = 0;
        for offset in &mut self.phase_offsets {
            let end = first + *offset;
            *offset = first;
            first = end;
        }
        self.potential_customers.reserve_inputs(progress.setup[1]);
    }

    // Setup's second pass over buildings `rows`, in order: counted buyers, the supplier order
    // and the provisional local shares. O(rows + their output ports).
    fn census_setup_order<F>(&mut self, ctx: &CensusContext<'_, F>, rows: Range<usize>) {
        let allocator = ctx.allocator;
        let catalog = ctx.catalog;
        for (index, building) in allocator.buildings[rows.clone()]
            .iter()
            .enumerate()
            .map(|(offset, building)| (rows.start + offset, building))
        {
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
    }

    // Setup's end: the port-sized books, the household-quote preparation and the closure
    // rows. O(B + ports) in parallel.
    fn census_setup_finish<F>(
        &mut self,
        ctx: &CensusContext<'_, F>,
        progress: &mut CensusProgress,
    ) {
        let allocator = ctx.allocator;
        #[cfg(test)]
        let CensusContext {
            households,
            network,
            graph,
            catalog,
            tuning,
            fiscal,
            treasury_balance,
            ..
        } = *ctx;
        let count = allocator.buildings.len();
        let [
            ports,
            inputs,
            consumer_firms,
            consumer_ports,
            maximum_consumer_ports,
        ] = progress.setup;
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

    // Household claims from row `start` until the rows' work reaches `budget`; returns the
    // row it stopped before and the work used. O(rows + their counted searches).
    fn census_household_claims<F>(
        &mut self,
        ctx: &CensusContext<'_, F>,
        logistics: &ShipmentSystem,
        start: usize,
        budget: u64,
    ) -> (usize, u64) {
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
        let total = allocator.buildings.len();
        let counted = counted_work(self, logistics);
        let mut operations = 0_u64;
        let work = |funding: &Self, rows: usize, operations: u64| {
            rows as u64 * Step::HouseholdClaims.row_work()
                + operations * WORK_WEIGHTS[9]
                + counted_work(funding, logistics).wrapping_sub(counted)
        };
        for index in start..total {
            let used = work(self, index - start, operations);
            if used >= budget {
                return (index, used);
            }
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
            operations += 1;
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
        (total, work(self, total - start, operations))
    }

    // Reserves current operations before offering inputs to prospective employees. Supplier
    // phases and building order remain stable within both dependent sweeps. Fully staffed
    // operations already searched their complete range in the current-staff sweep. Runs from
    // supplier-order slot `start` until the slots' work reaches `budget`; returns the slot it
    // stopped before and the work used. O(slots + their counted searches).
    fn census_staffing<F>(
        &mut self,
        ctx: &CensusContext<'_, F>,
        logistics: &mut ShipmentSystem,
        current_staff: bool,
        start: usize,
        budget: u64,
    ) -> (usize, u64) {
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
        let total = allocator.buildings.len();
        let counted = counted_work(self, logistics);
        let routes = self.input_supply.work_counts()[2];
        let route_extra = if current_staff {
            0
        } else {
            EXPANSION_ROUTE_EXTRA
        };
        let mut operations = 0_u64;
        let work = |funding: &Self, logistics: &ShipmentSystem, slots: usize, operations: u64| {
            slots as u64 * Step::CurrentStaff.row_work()
                + operations * WORK_WEIGHTS[9]
                + counted_work(funding, logistics).wrapping_sub(counted)
                + funding.input_supply.work_counts()[2].wrapping_sub(routes) * route_extra
        };
        for slot in start..total {
            let used = work(self, logistics, slot - start, operations);
            if used >= budget {
                return (slot, used);
            }
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
            operations += 1;
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
        (total, work(self, logistics, total - start, operations))
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
