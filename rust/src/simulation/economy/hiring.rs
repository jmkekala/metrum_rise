// SPDX-License-Identifier: GPL-2.0-only

//! Shared live-job forecast: recurring sales pay the operation; assets bridge settlement.

use rayon::prelude::*;

use super::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeEconomyTuning,
};
use super::fiscal::{CityFiscalPolicy, daily_property_tax};
use super::households::{
    Household, local_utility_service_costs, output_area_scale_for_building,
    physical_worker_capacity_for_profile, service_funded_worker_capacity,
};
use super::logistics::{
    RecurringSupply, ShipmentSystem, building_outputs_can_export_to_owa, freight_goods_multiplier,
    freight_price_phase, imported_input_unit_price,
};
use super::operation::{
    FundedOperation, OperatingCosts, OperationPlan, OperationPrices, purchased_input_units,
    working_capital_assets, working_capital_assets_at_prices,
};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};

const MARKET_CHUNK: usize = 256;

mod closure;
mod household_claims;
mod import_memo;
mod import_plan;
mod inputs;
mod rolling;
mod steps;

pub(crate) use inputs::{CensusInputs, FILL_SLOTS, FrozenCensusParameters, SavedCensusInputs};
pub(crate) use rolling::{
    CENSUS_WINDOW_START, CensusKey, PLANNING_PREPARATION_MASKS_SLOT,
    PLANNING_PREPARATION_REST_SLOT, PLANNING_PREPARATION_ROWS_SLOT, PLANNING_PREPARATION_SLOT,
    REMOVAL_WINDOW_START, RollingCensus, SavedRollingCensus, StructureRevisions, WindowAction,
    removals_allowed,
};
pub(crate) use steps::{CensusProgress, StepWork};
#[cfg(test)]
pub(crate) use steps::{STEP_NAMES, STEP_NANOS};

/// Reused building/resource scratch. Employment and admission read the same funded capacities.
#[derive(Clone, Default)]
pub(crate) struct JobFunding {
    /// Maximum live employees supported by both recurring sales and working capital.
    pub(crate) capacities: Vec<u32>,
    // One bit per copied row: complete, not under construction, when the census copied it.
    saw_operating: Vec<u64>,
    /// Complete closure attempts in the latest census, for exact scaling diagnostics.
    #[cfg(test)]
    pub(crate) closure_passes: usize,
    /// Positive customer-limit propagation rounds summed over the latest census.
    #[cfg(test)]
    pub(crate) propagation_rounds: usize,
    // Accepted recipe activity, which can be smaller than the rounded worker fraction.
    activities: Vec<f32>,
    // Recurring profit and working-capital surplus of each accepted operation quote.
    // These are financing limits, not additional sales; customer validation remains separate.
    headroom: Vec<[f64; 2]>,
    // Whether the current-staff quote replaced its import alternative. Only that
    // accepted local operation can restore its receipts as an expansion fallback.
    local_input_backing: Vec<bool>,
    // Registry classification is immutable within one census, including closure retries.
    service_flags: Vec<bool>,
    // Provisional quotes remain private until the joint customer/source proof succeeds.
    closure_ceilings: Vec<u32>,
    // Joint customer limits retain fractional production independently of rounded payroll.
    closure_activity: Vec<f32>,
    // Assets, prices and aggregate market inputs are immutable inside one census. Only
    // closure limits and sparse household quotas can invalidate these import-only results.
    import_memos: Vec<import_memo::ImportMemo>,
    import_memo_units: Vec<f64>,
    // Successful maximal import plans can cross census owners without sharing reservations.
    import_plans: import_plan::Book,
    // Necessary customer upper bounds propagate complete buyer withdrawal and partial
    // purchase limits without a fresh city-wide quotation for every supplier-chain link.
    potential_customers: closure::PotentialCustomers,
    offered_activity: Vec<f32>,
    // A physical repair touches only buyers of withdrawn provisional production.
    input_repairs: Vec<usize>,
    input_repair_seen: Vec<bool>,
    input_repair_headroom: Vec<[f64; 2]>,
    /// Jointly financed customer commitments, forked with the input book for projects.
    pub(crate) business: super::customers::business::BusinessMarket,
    /// Fiscal staffing ceilings, including unfinished services; these are not live jobs.
    pub(crate) service_capacities: Vec<u32>,
    /// Paid goods and unsettled sales available to the corresponding building.
    pub(crate) freight_capital: Vec<f64>,
    carrier_quotes: Vec<Option<f32>>,
    pricing_phase: u16,
    quote_revision: Option<(u64, u64, u32, u64, u64)>,
    /// The live city's key when the census started, for readers that lend its books; `None`
    /// for a census not run on the live city or since invalidated (`SIM-02-I`).
    pub(crate) published: Option<CensusKey>,
    // Identifies the funded rows `published` covers; see [`Self::rows_generation`].
    rows_generation: RowsGeneration,
    /// Whether a planning pass, or its preparation alone, has lent these books since a census
    /// last started refilling them (`SIM-02-K`): their site rows and planning storage persist
    /// across censuses, and a finished census's book is prepared again before it is published.
    pub(crate) planning_prepared: bool,
    border_nodes: Vec<u32>,
    quoted_border_nodes: Vec<u32>,
    // Resource slots contain full output capacity and recurring local demand, respectively.
    market_chunks: Vec<[f64; 2]>,
    market_revision: Option<(u64, u64, u32, u64, usize)>,
    market_catalog: Option<std::sync::Arc<()>>,
    market_production_revision: u64,
    market_changed_owners: Vec<usize>,
    market_dirty_chunks: Vec<bool>,
    #[cfg(test)]
    market_rows: usize,
    market: Vec<[f64; 2]>,
    // Sparse output-port quotas; negative entries retain the non-household buyer path.
    customer_offsets: Vec<usize>,
    customer_units: Vec<f64>,
    // Exact quote/outcome prefixes, retained only across matching budget/access/request inputs.
    household_quotes: household_claims::HouseholdQuotes,
    // Staffed output already promised to households, excluded from business input offers.
    household_production: Vec<f64>,
    // Stable counting-sort scratch: authored supplier phases, then live building index.
    local_order: Vec<usize>,
    phase_offsets: Vec<usize>,
    remapped_capacities: Vec<(usize, u32, u32, f64, f32, bool)>,
    /// Finite recurring supply left after funded live operations have made their input claims.
    pub(crate) input_supply: RecurringSupply,
    /// Individual recurring household accounts prepared with this funding census.
    pub(crate) customers: super::customers::ConsumerMarket,
    #[cfg(test)]
    import_quote_replay_disabled: bool,
    #[cfg(test)]
    import_quote_replays: usize,
    // Household-claim reserve, staffing and finish times of the latest census, for its debug
    // line.
    #[cfg(test)]
    household_detail: [f64; 3],
}

// Unique per book state: drawn afresh for every new or cloned book, by every run of the
// census steps (the only writers of the funded rows besides the invalidations) and at every
// change of `published`, which every invalidation makes (`SIM-02-K`). Compared for equality
// only; never saved and never read by a result.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct RowsGeneration(u64);

static NEXT_ROWS_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl RowsGeneration {
    pub(super) fn next() -> Self {
        Self(NEXT_ROWS_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

impl Default for RowsGeneration {
    fn default() -> Self {
        Self::next()
    }
}

impl Clone for RowsGeneration {
    fn clone(&self) -> Self {
        Self::next()
    }
}

impl JobFunding {
    /// Sets the key the book's funded rows are published under; every change draws a new
    /// [`Self::rows_generation`]. O(1).
    pub(crate) fn set_published(&mut self, key: Option<CensusKey>) {
        self.published = key;
        self.rows_generation = RowsGeneration::next();
    }

    /// Identifies this book's funded rows (capacities, activities, carrier quotes, headroom
    /// and freight capital) while it stays published: equal generations mean equal rows, so
    /// planning keeps the operation rows it derived from them across the book's lendings
    /// (`SIM-02-K`). O(1).
    pub(crate) fn rows_generation(&self) -> u64 {
        self.rows_generation.0
    }
}

#[cfg(test)]
thread_local! {
    /// Where each restart of [`JobFunding::refresh_in_slices_for_test`] on this thread left
    /// its census: the next step's name and its rows done.
    pub(crate) static RESTARTED_AT: std::cell::RefCell<Vec<(&'static str, usize)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

impl JobFunding {
    /// Gives this copy of `live` the capacity of the arenas a census refills, the business
    /// receipts and the consumer claim staging (`SIM-02-K`): [`Clone`] gives a copy exact to
    /// its rows. Capacity never changes a result. O(1) without growth.
    pub(crate) fn reserve_census_arenas_like(&mut self, live: &Self) {
        self.business.reserve_like(&live.business);
        self.customers.reserve_like(&live.customers);
    }

    /// Reports exact cross-census import plan hits in the latest refresh.
    #[cfg(test)]
    pub(crate) fn import_plan_replays(&self) -> usize {
        self.import_plans.rows.iter().map(|row| row.replays).sum()
    }

    /// Checks private operation results, including both exact financing surpluses.
    #[cfg(test)]
    pub(crate) fn assert_same_operations_for_test(&self, other: &Self) {
        assert_eq!(self.capacities, other.capacities);
        assert_eq!(self.activities, other.activities);
        assert_eq!(self.headroom, other.headroom);
    }

    /// The customer, business and input-supply book fingerprints without what a census only
    /// caches from earlier censuses (the previous household categories, the business port
    /// index) or keys on network and allocator revisions: what two censuses on the same inputs
    /// produce bit for bit whatever ran before them (`SIM-02-I`).
    #[cfg(test)]
    pub(crate) fn census_book_fingerprints_for_test(&self) -> [Vec<(&'static str, u64)>; 3] {
        let keep = |book: Vec<(&'static str, u64)>, cached: &[&str]| {
            book.into_iter()
                .filter(|(name, _)| !name.contains("revision") && !cached.contains(name))
                .collect::<Vec<_>>()
        };
        [
            keep(
                self.customers.live_fingerprint_for_test(),
                &["previous_categories"],
            ),
            keep(self.business.live_fingerprint_for_test(), &["indices"]),
            keep(self.input_supply.live_fingerprint_for_test(), &[]),
        ]
    }

    /// Forces a complete production-market census on the next comparison refresh.
    #[cfg(test)]
    pub(crate) fn invalidate_market_for_test(&mut self) {
        self.market_revision = None;
    }

    /// Reports production rows evaluated by the latest census.
    #[cfg(test)]
    pub(crate) fn market_rows_for_test(&self) -> usize {
        self.market_rows
    }

    /// Checks exact production totals and original buyer membership after funding closure.
    #[cfg(test)]
    pub(crate) fn assert_same_market_for_test(&self, other: &Self) {
        assert_eq!(self.market, other.market);
        assert_eq!(self.market_chunks, other.market_chunks);
        assert_eq!(
            self.potential_customers.counted,
            other.potential_customers.counted
        );
        assert_eq!(
            self.potential_customers.owners,
            other.potential_customers.owners
        );
    }

    /// Forces ordinary import-only staffing evaluation in comparison tests.
    #[cfg(test)]
    pub(crate) fn disable_import_quote_replay(&mut self) {
        self.import_quote_replay_disabled = true;
    }

    /// Returns import-only staffing replays from the latest refresh.
    #[cfg(test)]
    pub(crate) fn import_quote_replays(&self) -> usize {
        self.import_quote_replays
    }

    /// [`Self::refresh`] run in slices of `budget` work units, as the slots run it (`SIM-02-I`,
    /// `SIM-02-K`); after
    /// `restart_after` slices it starts again from its first step, as a network or structural
    /// change makes it, and records where in [`RESTARTED_AT`]. Returns the slices run.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn refresh_in_slices_for_test(
        &mut self,
        allocator: &BuildingAllocator,
        households: &[Household],
        logistics: &mut ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        service_funding: &[f32],
        treasury_balance: f64,
        minute_of_day: u16,
        budget: usize,
        restart_after: Option<usize>,
    ) -> usize {
        let context = steps::CensusContext {
            allocator,
            households,
            network,
            graph,
            catalog,
            tuning,
            fiscal,
            treasury_balance,
            minute_of_day,
            service_capacity: |index, physical, profile: &EconomyProfileRuntime| {
                service_funded_worker_capacity(physical, profile, index, service_funding)
            },
            previous: None,
            freight_capital: None,
        };
        let mut progress = steps::CensusProgress::default();
        let mut slices = 0;
        loop {
            slices += 1;
            if self.run_census_steps(&context, logistics, &mut progress, budget as u64) {
                return slices;
            }
            if restart_after == Some(slices) {
                RESTARTED_AT.with_borrow_mut(|at| at.push(progress.position_for_test()));
                progress = steps::CensusProgress::default();
            }
        }
    }
}

struct InputFunding<'a> {
    supply: &'a mut RecurringSupply,
    logistics: &'a mut ShipmentSystem,
    allocator: &'a BuildingAllocator,
    network: &'a TransitNetwork,
    graph: &'a RegionGraph,
    tuning: &'a RuntimeEconomyTuning,
    buyer: usize,
    outside: Option<f32>,
    retained: (u32, f32),
    goods_multiplier: f32,
}

// Diagnostic counters are absent from gameplay builds. Aggregate once per operation,
// including Rayon import ceilings; detailed tracing alone enables the extra lower-cost quote.
#[cfg(test)]
static FUNDING_SEARCH_COUNTS: [std::sync::atomic::AtomicU64; 14] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 14];

#[cfg(test)]
struct FundingSearchTrace {
    enabled: bool,
    local: bool,
    profile: u16,
    physical: u32,
    ceiling: u32,
    // Operations, trial plans, capital rejects, profit rejects, free-input profit rejects,
    // accepted plans, missing entrance. Counters are cumulative for phase-trace subtraction.
    counts: [u64; 7],
    // Opt-in exact import-plan attribution; never populated during acceptance runs.
    import_plan: Option<ImportPlanTrace>,
}

#[cfg(test)]
struct ImportPlanTrace {
    owner: u64,
    key: String,
    capital: u64,
    desired: u32,
    workers: u32,
    required: u64,
}

#[cfg(test)]
impl Drop for FundingSearchTrace {
    fn drop(&mut self) {
        if !self.enabled {
            return;
        }
        if let Some(plan) = &self.import_plan {
            crate::debug_log!(
                "economy_phases",
                "import_plan owner={} key={} capital={} desired={} workers={} required={} trials={}",
                plan.owner,
                plan.key,
                plan.capital,
                plan.desired,
                plan.workers,
                plan.required,
                self.counts[1]
            );
        }
        // Per-operation output is deliberately opt-in: it is diagnostic evidence, never
        // acceptance timing or gameplay work. Read the environment once per test process.
        static DETAILS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *DETAILS.get_or_init(|| std::env::var_os("METRUM_TRACE_STAFFING").is_some()) {
            crate::debug_log!(
                "economy_phases",
                "staffing_operation profile={} local={} physical={} ceiling={} counts={:?}",
                self.profile,
                self.local,
                self.physical,
                self.ceiling,
                self.counts
            );
        }
        for (total, count) in FUNDING_SEARCH_COUNTS[usize::from(self.local) * 7..]
            .iter()
            .zip(self.counts)
        {
            total.fetch_add(count, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

impl InputFunding<'_> {
    fn costs(
        &mut self,
        profile: &EconomyProfileRuntime,
        catalog: &RuntimeEconomyCatalog,
        plan: OperationPlan,
        prices: OperationPrices,
        export_units: impl Fn(u16, f32) -> f32,
    ) -> Option<OperatingCosts> {
        self.supply.retry(self.allocator, self.graph);
        let entrance = self.allocator.entrances.get(self.buyer)?;
        Some(OperatingCosts::for_plan_with_input_quotes(
            profile,
            plan,
            prices,
            export_units,
            |input| {
                self.logistics.quote_recurring_input(
                    self.supply,
                    input,
                    Some(self.buyer),
                    entrance,
                    self.outside,
                    self.allocator,
                    catalog,
                    self.tuning,
                    self.network,
                    self.graph,
                    self.goods_multiplier,
                )
            },
        ))
    }
}

// Net-input multiplier of a buyer with `funded` jobs at `activity`; see
// [`JobFunding::input_demand_scale`].
fn input_demand_scale(
    building: &Building,
    profile: &EconomyProfileRuntime,
    service: bool,
    funded: u32,
    activity: f32,
) -> f32 {
    let physical = physical_worker_capacity_for_profile(building, profile).max(1);
    let activity = if service {
        funded as f32 / physical as f32
    } else {
        activity
    };
    super::households::funded_input_demand_scale(building, profile, activity)
}

fn live(building: &Building) -> bool {
    !building.broken
        && !building.economy_broken
        && !building.is_deserted
        && !building.is_under_construction()
        && building.edge_idx != usize::MAX
}

impl JobFunding {
    /// Forces ordinary household quotation in tests comparing the exact closure outcomes.
    #[cfg(test)]
    pub(crate) fn disable_household_quote_replay(&mut self) {
        self.household_quotes.disabled = true;
    }

    /// Uses ordinary full closure replay as the oracle for zero-customer propagation tests.
    #[cfg(test)]
    pub(crate) fn disable_customer_cascade(&mut self) {
        self.potential_customers.disabled = true;
    }

    /// Latest census's reused requests and changed-prefix reconstructions, for regressions.
    #[cfg(test)]
    pub(crate) fn household_quote_replay_counts(&self) -> (usize, usize) {
        (self.household_quotes.reused, self.household_quotes.rebuilt)
    }

    /// Cross-owner rejected quote reuses and those subsequently accepted after real reservation.
    #[cfg(test)]
    pub(crate) fn household_rejected_quote_counts(&self) -> (usize, usize) {
        (
            self.household_quotes.rejected_reused,
            self.household_quotes.rejected_accepted,
        )
    }

    /// Requests replayed from the preceding census, excluding retries inside this census.
    #[cfg(test)]
    pub(crate) fn household_cross_census_replays(&self) -> usize {
        self.household_quotes.census_reused
    }

    /// Retains one accepted live operation for affected-input repricing during construction.
    /// O(1) apart from authored recipe-port area lookup; callers copy rows once per pass.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn funded_operation(
        &self,
        index: usize,
        building: &Building,
        profile: &EconomyProfileRuntime,
        utilities: [f32; 3],
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        minute: u16,
    ) -> Option<FundedOperation> {
        let workers = self.capacities.get(index).copied().unwrap_or(0);
        if workers == 0 {
            return None;
        }
        Some(FundedOperation {
            profile: profile.runtime_id,
            plan: OperationPlan {
                workers,
                activity: self.activities[index],
                area: output_area_scale_for_building(building, profile),
            },
            prices: closure::operation_prices(
                building,
                self.carrier_quotes[index],
                freight_goods_multiplier(profile, building.zone_type, tuning, minute),
                utilities,
                tuning,
                fiscal,
            ),
            headroom: self.headroom[index],
        })
    }

    /// Accepted recipe activity by building index, as a share of authored full output.
    /// Sales-scaled stores produce and restock at this rate; empty before the first census.
    pub(crate) fn activities(&self) -> &[f32] {
        &self.activities
    }

    /// Stands in for a census that accepted `activity` at building `index`.
    #[cfg(test)]
    pub(crate) fn set_activity(&mut self, index: usize, activity: f32) {
        self.rows_generation = RowsGeneration::next();
        self.activities
            .resize(self.activities.len().max(index + 1), 0.0);
        self.activities[index] = activity;
    }

    /// Net-input multiplier for an established funded buyer, capped by its current staff.
    /// Public invoices use the fiscal ceiling independently of material availability; private
    /// buyers use their accepted recipe activity. O(1), including explicit work-area scaling.
    pub(crate) fn input_demand_scale(
        &self,
        index: usize,
        building: &Building,
        profile: &EconomyProfileRuntime,
        service: bool,
    ) -> f32 {
        input_demand_scale(
            building,
            profile,
            service,
            self.capacities.get(index).copied().unwrap_or(0),
            self.activities.get(index).copied().unwrap_or(0.0),
        )
    }

    /// [`Self::input_demand_scale`] from the funded jobs and activity the published census
    /// copied onto `building` (`funded_job_capacity`, `funded_activity`), which saves keep.
    pub(crate) fn published_input_demand_scale(
        building: &Building,
        profile: &EconomyProfileRuntime,
        service: bool,
    ) -> f32 {
        input_demand_scale(
            building,
            profile,
            service,
            building.funded_job_capacity,
            building.funded_activity,
        )
    }

    /// Registers a newly placed service's future input ceiling before the next hourly census.
    /// Existing entries stay intact. O(new indexed slots), O(1) when warm; creates no live job.
    pub(crate) fn register_pending_service(
        &mut self,
        index: usize,
        building: &Building,
        profile: &EconomyProfileRuntime,
        funding: f32,
    ) {
        self.service_capacities
            .resize(self.service_capacities.len().max(index + 1), 0);
        self.service_capacities[index] = service_funded_worker_capacity(
            physical_worker_capacity_for_profile(building, profile),
            profile,
            0,
            &[funding],
        );
        self.quote_revision = None;
        self.set_published(None);
    }

    /// Whether the census saw building `index` complete (`SIM-02-I`): a building that
    /// completed after its copy, or was placed after it, is still under construction for the
    /// readers of its books (workforce, construction supply, open jobs) until a census sees it
    /// complete, so its constructing claim never meets the jobs a later census funds. O(1).
    pub(crate) fn saw_operating(&self, index: usize) -> bool {
        self.saw_operating
            .get(index / 64)
            .is_some_and(|word| word & (1 << (index % 64)) != 0)
    }

    /// Test fixtures with hand-built censuses: the census saw `buildings` as they are now.
    #[cfg(test)]
    pub(crate) fn see_operating_for_test(&mut self, buildings: &[Building]) {
        self.record_operating(buildings);
    }

    // Sets whether the census saw building `index` complete, for remaps. O(1).
    fn set_saw_operating(&mut self, index: usize, seen: bool) {
        let (word, bit) = (index / 64, 1 << (index % 64));
        if seen && self.saw_operating.len() <= word {
            self.saw_operating.resize(word + 1, 0);
        }
        if let Some(bits) = self.saw_operating.get_mut(word) {
            if seen {
                *bits |= bit;
            } else {
                *bits &= !bit;
            }
        }
    }

    // Records which copied rows are complete, from the census's setup step. O(B).
    fn record_operating(&mut self, buildings: &[Building]) {
        self.saw_operating.clear();
        self.saw_operating.resize(buildings.len().div_ceil(64), 0);
        for (index, building) in buildings.iter().enumerate() {
            if !building.is_under_construction() {
                self.saw_operating[index / 64] |= 1 << (index % 64);
            }
        }
    }

    /// Paid cargo assets prepared with the current live-job forecast, excluding carrier charges.
    pub(crate) fn paid_cargo_capital(&self, index: usize) -> f64 {
        self.freight_capital.get(index).copied().unwrap_or(0.0)
    }

    /// Whether an hourly construction pass can reuse the funded source book, including its
    /// frontage-priced freight costs. Geometry alone cannot validate recurring profitability.
    /// The census must have started on the live city's `key`, households removed since aside
    /// (`SIM-02-I`, [`CensusKey::books_valid_for`]). Buildings appended since its copy are not
    /// in its books: readers treat them as unfunded and without supply, and the lent books lay
    /// out their buyer ports for the pass. O(connected borders), called once per planning
    /// pass.
    pub(crate) fn has_current_input_supply(
        &self,
        key: CensusKey,
        borders: &[u32],
        pricing_phase: u16,
    ) -> bool {
        self.published
            .is_some_and(|published| published.books_valid_for(key))
            && self.pricing_phase == pricing_phase
            && self.quoted_border_nodes == borders
    }

    // Keeps the fiscal ceilings `live` registered for services below `buildings` that this
    // census's copy does not cover (`register_pending_service`); buildings appended after the
    // last registered one have none. O(appended buildings).
    fn adopt_pending_services(&mut self, live: &Self, buildings: usize) {
        let covered = self.service_capacities.len();
        let end = buildings.min(live.service_capacities.len());
        if covered < end {
            self.service_capacities
                .extend_from_slice(&live.service_capacities[covered..end]);
        }
    }

    /// Clears the live view without discarding reusable forecast storage.
    pub(crate) fn clear(&mut self) {
        self.set_published(None);
        self.saw_operating.clear();
        self.import_plans.clear();
        self.market_revision = None;
        self.capacities.clear();
        self.activities.clear();
        self.headroom.clear();
        self.service_capacities.clear();
        self.freight_capital.clear();
        self.quote_revision = None;
        self.input_supply.invalidate();
        self.customers.clear();
    }

    /// Withdraws a removed employer immediately, before the next hourly forecast.
    pub(crate) fn invalidate_building(&mut self, index: usize) {
        self.set_published(None);
        self.import_plans.invalidate(index);
        self.market_revision = None;
        if let Some(capacity) = self.capacities.get_mut(index) {
            *capacity = 0;
        }
        if let Some(activity) = self.activities.get_mut(index) {
            *activity = 0.0;
        }
        if let Some(capacity) = self.service_capacities.get_mut(index) {
            *capacity = 0;
        }
        if let Some(capital) = self.freight_capital.get_mut(index) {
            *capital = 0.0;
        }
        if let Some(headroom) = self.headroom.get_mut(index) {
            *headroom = [0.0; 2];
        }
        self.set_saw_operating(index, false);
        self.quote_revision = None;
        self.input_supply.invalidate();
    }

    /// Moves jobs and fiscal/cargo records in O(mapping size + new slots), including undo growth.
    /// Scratch is reused; unmoved buildings need no scan or copy.
    pub(crate) fn remap_buildings(&mut self, mapping: &std::collections::HashMap<usize, usize>) {
        if !mapping.is_empty() {
            self.set_published(None);
        }
        self.import_plans.remap(mapping);
        self.market_revision = None;
        // A remap invalidates the complete input/customer quotation below.
        self.headroom.clear();
        self.remapped_capacities.clear();
        let mut required = 0;
        for (&old, &new) in mapping {
            required = required.max(new + 1);
            self.remapped_capacities.push((
                new,
                self.capacities.get(old).copied().unwrap_or(0),
                self.service_capacities.get(old).copied().unwrap_or(0),
                self.freight_capital.get(old).copied().unwrap_or(0.0),
                self.activities.get(old).copied().unwrap_or(0.0),
                self.saw_operating(old),
            ));
        }
        // Undo can reintroduce a last index after an hourly forecast shrank these arrays.
        // Empty, unprepared views stay empty rather than allocating city-sized scratch.
        for capacities in [&mut self.capacities, &mut self.service_capacities] {
            if !capacities.is_empty() {
                capacities.resize(capacities.len().max(required), 0);
            }
        }
        if !self.freight_capital.is_empty() {
            self.freight_capital
                .resize(self.freight_capital.len().max(required), 0.0);
        }
        if !self.activities.is_empty() {
            self.activities
                .resize(self.activities.len().max(required), 0.0);
        }
        for &old in mapping.keys() {
            if let Some(capacity) = self.capacities.get_mut(old) {
                *capacity = 0;
            }
            if let Some(activity) = self.activities.get_mut(old) {
                *activity = 0.0;
            }
            if let Some(capacity) = self.service_capacities.get_mut(old) {
                *capacity = 0;
            }
            if let Some(capital) = self.freight_capital.get_mut(old) {
                *capital = 0.0;
            }
            self.set_saw_operating(old, false);
        }
        for index in 0..self.remapped_capacities.len() {
            let (new, _, _, _, _, seen) = self.remapped_capacities[index];
            self.set_saw_operating(new, seen);
        }
        for &(new, value, service, capital, activity, _) in &self.remapped_capacities {
            if let Some(capacity) = self.capacities.get_mut(new) {
                *capacity = value;
            }
            if let Some(capacity) = self.service_capacities.get_mut(new) {
                *capacity = service;
            }
            if let Some(value) = self.freight_capital.get_mut(new) {
                *value = capital;
            }
            if let Some(value) = self.activities.get_mut(new) {
                *value = activity;
            }
        }
        self.quote_revision = None;
        self.input_supply.invalidate();
    }

    /// Rebuilds once per employment pass, including passes with no current resident applicants.
    /// Market aggregation is O(B*ports + ceil(B/256)*R + households*consumer categories), with fixed parallel
    /// chunks and ordered f64 merges. Staffing checks descend from the market ceiling:
    /// commonly one operation per business, worst-case O(total job capacity * ports²),
    /// parallel for independent import ceilings. Household and supplier claims commit serially
    /// through finite component/chunk indices, plus directed route queries for visited buyers.
    /// Freight rounding makes costs non-monotone, so binary
    /// search cannot safely reject smaller operations. Consumers read capacities in O(1).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn refresh(
        &mut self,
        allocator: &BuildingAllocator,
        households: &[Household],
        logistics: &mut ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        service_funding: &[f32],
        treasury_balance: f64,
        minute_of_day: u16,
    ) {
        self.refresh_with_service_capacity(
            allocator,
            households,
            logistics,
            network,
            graph,
            catalog,
            tuning,
            fiscal,
            treasury_balance,
            |index, physical, profile| {
                service_funded_worker_capacity(physical, profile, index, service_funding)
            },
            None,
            minute_of_day,
        );
    }

    /// Rebuilds the same live input market using the caller's authoritative service job limits.
    /// Construction execution uses current funded service capacities after index remaps, while
    /// employment supplies the fiscal funding fractions through `refresh` above.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn refresh_with_service_capacity(
        &mut self,
        allocator: &BuildingAllocator,
        households: &[Household],
        logistics: &mut ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        treasury_balance: f64,
        service_capacity: impl Fn(usize, u32, &EconomyProfileRuntime) -> u32 + Sync,
        previous: Option<&Self>,
        minute_of_day: u16,
    ) {
        self.refresh_census(
            allocator,
            households,
            logistics,
            network,
            graph,
            catalog,
            tuning,
            fiscal,
            treasury_balance,
            service_capacity,
            previous,
            minute_of_day,
            None,
        );
    }

    // The census itself, all its steps at once. `freight_capital` is each building's paid
    // cargo, frozen with a census input copy (`SIM-02-I`); `None` reads it from `logistics`'
    // open shipments. Routes are quoted through `logistics`' freight route cache either way.
    #[allow(clippy::too_many_arguments)]
    fn refresh_census(
        &mut self,
        allocator: &BuildingAllocator,
        households: &[Household],
        logistics: &mut ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        treasury_balance: f64,
        service_capacity: impl Fn(usize, u32, &EconomyProfileRuntime) -> u32 + Sync,
        previous: Option<&Self>,
        minute_of_day: u16,
        freight_capital: Option<&[f64]>,
    ) {
        let context = steps::CensusContext {
            allocator,
            households,
            network,
            graph,
            catalog,
            tuning,
            fiscal,
            treasury_balance,
            minute_of_day,
            service_capacity,
            previous,
            freight_capital,
        };
        self.run_census_steps(
            &context,
            logistics,
            &mut steps::CensusProgress::default(),
            u64::MAX,
        );
    }
}

fn log_funding_times(
    category: crate::debug::Channel,
    buildings: usize,
    minute: u16,
    times: &[f64; 11],
) {
    crate::debug_log!(
        category,
        "funding_detail buildings={} minute={} routes_ms={:.3} market_ms={:.3} customers_ms={:.3} import_funding_ms={:.3} household_claims_ms={:.3} sources_ms={:.3} local_funding_ms={:.3} staffing_ms={:.3} publish_ms={:.3} repair_ms={:.3} business_ms={:.3} propagate_ms={:.3}",
        buildings,
        minute,
        times[0],
        times[1],
        times[2],
        times[3],
        times[4],
        times[5],
        times[6..].iter().sum::<f64>(),
        times[6],
        times[7],
        times[8],
        times[9],
        times[10]
    );
}

// Public service input invoices draw on the treasury under the existing fiscal policy.
// They do not require private sales or owner equity. Integer activity probes reuse the same
// finite quote/retry scratch; worst-case O(funded workers * recipe ports² + source searches).
fn reserve_service_inputs(
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    funded: u32,
    physical: u32,
    prices: OperationPrices,
    mut supply: InputFunding<'_>,
) -> f32 {
    for workers in (1..=funded.min(physical)).rev() {
        let Some(costs) = supply.costs(
            profile,
            catalog,
            OperationPlan {
                workers,
                activity: workers as f32 / physical.max(1) as f32,
                area: 1.0,
            },
            prices,
            |_, _| 0.0,
        ) else {
            return 0.0;
        };
        if costs.inputs.is_finite() && costs.freight.is_finite() {
            supply.supply.accept_growing_for(supply.buyer);
            return workers as f32 / physical.max(1) as f32;
        }
    }
    supply.supply.reject(supply.allocator, supply.graph);
    0.0
}

// Keep the full reachable net-output reservation through staffing searches. A rejected
// operation releases every family claim; a cheaper local-input retry can reuse accepted quotas.
#[allow(clippy::too_many_arguments)]
fn reserve_consumer_outputs(
    customers: &mut super::customers::ConsumerMarket,
    units: &mut [f64],
    index: usize,
    building: &Building,
    profile: &EconomyProfileRuntime,
    fiscal: &CityFiscalPolicy,
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
) {
    let area = output_area_scale_for_building(building, profile);
    for (quota, output) in units.iter_mut().zip(&profile.outputs) {
        if !customers.consumes(output.resource_runtime_id) {
            *quota = -1.0;
            continue;
        }
        *quota = allocator.entrances.get(index).map_or(0.0, |entrance| {
            customers.claim_reachable(
                entrance,
                output.resource_runtime_id,
                f64::from(profile.net_output_units_per_day(output) * area),
                profile.unit_price_currency * (1.0 + fiscal.household_vat_rate),
                allocator,
                network,
                graph,
            )
        });
    }
}

// The same provisional sales and stock valuation support both a staffing search and
// an unchanged operation whose withdrawn input supplier is being replaced.
struct ProvisionalSales<'a> {
    profile: &'a EconomyProfileRuntime,
    catalog: &'a RuntimeEconomyCatalog,
    area: f32,
    market: &'a [[f64; 2]],
    household: Option<&'a [f64]>,
    export_multiplier: Option<f32>,
}

impl ProvisionalSales<'_> {
    fn share(&self, resource: u16) -> f32 {
        provisional_local_share(
            self.profile,
            self.area,
            resource,
            self.household,
            self.market,
        )
    }

    fn local_units(&self, resource: u16, units: f32) -> f32 {
        let quota = self.profile.output_port(resource).map_or(0.0, |output| {
            self.profile.net_output_units_per_day(output) * self.area * self.share(resource)
        });
        units.min(quota)
    }

    fn export_units(&self, resource: u16, units: f32) -> f32 {
        if self.export_multiplier.is_some() {
            units - self.local_units(resource, units)
        } else {
            0.0
        }
    }

    fn output_price(&self, resource: u16) -> f64 {
        let share = self.share(resource);
        f64::from(self.profile.unit_price_currency) * f64::from(share)
            + f64::from(
                self.catalog
                    .unit_price_for_resource(resource)
                    .unwrap_or(0.0)
                    * self.export_multiplier.unwrap_or(0.0),
            ) * f64::from(1.0 - share)
    }

    fn revenue(&self, activity: f32) -> f64 {
        self.profile
            .outputs
            .iter()
            .map(|output| {
                let resource = output.resource_runtime_id;
                let units = self.profile.net_output_units_per_day(output) * self.area * activity;
                f64::from(self.profile.unit_price_currency)
                    * f64::from(self.local_units(resource, units))
                    + f64::from(
                        self.catalog
                            .unit_price_for_resource(resource)
                            .unwrap_or(0.0)
                            * self.export_multiplier.unwrap_or(0.0),
                    ) * f64::from(self.export_units(resource, units))
            })
            .sum()
    }
}

// Every private plan's working capital holds at least one wage: input buffers, overhead and
// freight are non-negative. Without stock, capital is cash plus paid cargo, fixed for the whole
// census, so no later market change can fund a single worker. A zero closure ceiling states
// that proof up front; financing would reject each plan anyway, so capacities are unchanged.
fn cannot_fund_one_worker(
    building: &Building,
    profile: &EconomyProfileRuntime,
    cargo: f64,
) -> bool {
    profile
        .inputs
        .iter()
        .chain(&profile.outputs)
        .all(|port| building.inventory_units(port.resource_runtime_id) == 0.0)
        && f64::from(building.operating_budget) + cargo < f64::from(profile.average_daily_wage())
}

// Bounds the rounding of the f32 quotes that `cannot_profit_at_best_prices` dominates.
const PROFIT_BOUND_SLACK: f64 = 64.0 * f32::EPSILON as f64;

// Revenue and purchased inputs are both linear in activity. Per unit of activity, sales earn
// at most every net output at the better of the local and export price, and each purchased
// input costs at least its cheapest authored seller or the outside price, both at this site's
// goods multiplier. Payroll, overhead and carriers only add cost. These prices are fixed for
// the census, so when the best revenue cannot exceed the cheapest inputs, no supplier
// withdrawal, customer change or recovered rival can fund any plan. A zero closure ceiling
// states that proof up front, as for `cannot_fund_one_worker`; capacities are unchanged.
// O(I + O) per building, allocation-free.
fn cannot_profit_at_best_prices(
    building: &Building,
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    carrier_quote: Option<f32>,
    minute_of_day: u16,
) -> bool {
    let goods_multiplier =
        freight_goods_multiplier(profile, building.zone_type, tuning, minute_of_day);
    if !goods_multiplier.is_finite() {
        return false;
    }
    let area = output_area_scale_for_building(building, profile);
    let exports = building_outputs_can_export_to_owa(building, profile)
        && carrier_quote.is_some()
        && tuning.owa_export_price_multiplier > 0.0;
    let revenue: f64 = profile
        .outputs
        .iter()
        .map(|output| {
            let export = if exports {
                catalog
                    .unit_price_for_resource(output.resource_runtime_id)
                    .unwrap_or(0.0)
                    * tuning.owa_export_price_multiplier
            } else {
                0.0
            };
            f64::from(profile.net_output_units_per_day(output) * area)
                * f64::from(profile.unit_price_currency.max(export))
        })
        .sum();
    let production = OperationPlan {
        workers: 0,
        activity: 1.0,
        area,
    }
    .production(profile);
    let mut inputs = 0.0_f64;
    for input in &profile.inputs {
        let units = purchased_input_units(profile, input, production);
        if units <= 0.0 {
            continue;
        }
        let local = catalog.cheapest_seller_price(input.resource_runtime_id) * goods_multiplier;
        let outside = carrier_quote.map_or(f32::INFINITY, |_| {
            imported_input_unit_price(catalog, tuning, input.resource_runtime_id, goods_multiplier)
        });
        inputs += f64::from(units) * f64::from(local.min(outside));
    }
    revenue * (1.0 + PROFIT_BOUND_SLACK) < inputs
}

struct ImportPlanRecord<'a> {
    row: &'a mut import_plan::Row,
    shares: &'a [u32],
    previous: Option<import_plan::Proof<'a>>,
}

#[allow(clippy::too_many_arguments)]
fn funded_private_operation(
    building: &Building,
    profile: &EconomyProfileRuntime,
    catalog: &RuntimeEconomyCatalog,
    physical: u32,
    prices: OperationPrices,
    market: &[[f64; 2]],
    export_multiplier: Option<f32>,
    freight_capital: f64,
    mut supply: Option<InputFunding<'_>>,
    consumer_units: Option<&[f64]>,
    headroom: &mut [f64; 2],
    ceiling: u32,
    activity_ceiling: f32,
    mut proof: Option<ImportPlanRecord<'_>>,
) -> (u32, f32) {
    #[cfg(test)]
    let mut trace = FundingSearchTrace {
        enabled: crate::debug::Channel::EconomyPhases.enabled(),
        local: supply.is_some(),
        profile: profile.runtime_id,
        physical,
        ceiling,
        counts: [1, 0, 0, 0, 0, 0, 0],
        import_plan: None,
    };
    if let Some(record) = proof.as_mut() {
        record.row.invalidate();
    }
    if supply.is_none() {
        *headroom = [0.0; 2];
    }
    let area = output_area_scale_for_building(building, profile);
    let sales = ProvisionalSales {
        profile,
        catalog,
        area,
        market,
        household: consumer_units,
        export_multiplier,
    };
    let market_activity = (if export_multiplier.is_some() {
        1.0
    } else {
        profile
            .outputs
            .iter()
            .map(|port| sales.share(port.resource_runtime_id))
            .fold(0.0, f32::max)
    })
    .min(activity_ceiling);
    let desired = ((physical as f32 * market_activity).ceil() as u32).min(ceiling);
    if desired == 0 {
        return (0, 0.0);
    }
    crate::debug::work::add(crate::debug::work::Work::StaffingOperations, 1);
    // Local quotes value retained stock at their actual supplier prices below. Computing
    // an import valuation here would visit every inventory port without using the result.
    let imported_capital = if supply.is_none() {
        working_capital_assets(
            building,
            profile,
            catalog,
            prices.imports,
            freight_capital,
            |resource| sales.output_price(resource),
        )
    } else {
        0.0
    };
    #[cfg(test)]
    if trace.enabled && supply.is_none() {
        static DETAILS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *DETAILS.get_or_init(|| std::env::var_os("METRUM_TRACE_IMPORT_PLANS").is_some()) {
            // Exact arithmetic inputs, excluding capital: a successful maximal plan
            // stays successful while capital still covers the same fixed requirement.
            let shares: Vec<_> = profile
                .outputs
                .iter()
                .map(|port| sales.share(port.resource_runtime_id).to_bits())
                .collect();
            trace.import_plan = Some(ImportPlanTrace {
                owner: building.parcel_id,
                key: format!(
                    "{:?}",
                    (
                        profile.runtime_id,
                        physical,
                        area.to_bits(),
                        [
                            prices.utilities[0].to_bits(),
                            prices.utilities[1].to_bits(),
                            prices.utilities[2].to_bits(),
                            prices.property_tax.to_bits(),
                            prices.imports.to_bits(),
                            prices.carrier_quote.to_bits(),
                            prices.truck_payload.to_bits()
                        ],
                        export_multiplier.map(f32::to_bits),
                        market_activity.to_bits(),
                        desired,
                        shares
                    )
                ),
                capital: imported_capital.to_bits(),
                desired,
                workers: 0,
                required: 0,
            });
        }
    }
    let key = import_plan::Key {
        profile: profile.runtime_id,
        physical,
        area: area.to_bits(),
        prices: [
            prices.utilities[0].to_bits(),
            prices.utilities[1].to_bits(),
            prices.utilities[2].to_bits(),
            prices.property_tax.to_bits(),
            prices.imports.to_bits(),
            prices.carrier_quote.to_bits(),
            prices.truck_payload.to_bits(),
        ],
        exports: export_multiplier.map(f32::to_bits),
        market_activity: market_activity.to_bits(),
        desired,
    };
    if let Some(record) = proof.as_mut()
        && let Some((workers, activity, surplus, required)) = record
            .previous
            .as_ref()
            .and_then(|previous| previous.reuse(key, record.shares, imported_capital))
    {
        *headroom = surplus;
        record.row.remember(key, activity, surplus, required);
        #[cfg(test)]
        {
            record.row.replays += 1;
        }
        return (workers, activity);
    }
    let costs = |workers| {
        OperatingCosts::for_plan(
            profile,
            catalog,
            OperationPlan {
                workers,
                activity: market_activity.min(workers as f32 / physical.max(1) as f32),
                area,
            },
            prices,
            |resource, units| sales.export_units(resource, units),
        )
    };
    // Integer payroll, local sales quotas and quantized freight do not form a monotone
    // profitability test. Evaluate the largest feasible operation first, without allowing
    // an unprofitable full-capacity export plan to erase a profitable smaller business.
    // A cheaper source also lowers the replacement value of retained stock. Keep the
    // already validated plan when that repricing cannot finance equal staffing. The caller
    // keeps a local plan's released receipts provisional until replacement is accepted.
    let retained = supply.as_ref().map_or((0, 0.0), |context| context.retained);
    for workers in (retained.0.max(1)..=desired).rev() {
        crate::debug::work::add(crate::debug::work::Work::StaffingTrials, 1);
        #[cfg(test)]
        if trace.enabled {
            trace.counts[1] += 1;
            let activity = market_activity.min(workers as f32 / physical.max(1) as f32);
            let free_inputs = OperatingCosts::for_plan_with_input_quotes(
                profile,
                OperationPlan {
                    workers,
                    activity,
                    area,
                },
                prices,
                |resource, units| sales.export_units(resource, units),
                |_| Default::default(),
            );
            trace.counts[4] += u64::from(sales.revenue(activity) <= free_inputs.total());
        }
        let costs = if let Some(context) = supply.as_mut() {
            let Some(costs) = context.costs(
                profile,
                catalog,
                OperationPlan {
                    workers,
                    activity: market_activity.min(workers as f32 / physical.max(1) as f32),
                    area,
                },
                prices,
                |resource, units| sales.export_units(resource, units),
            ) else {
                #[cfg(test)]
                if trace.enabled {
                    trace.counts[6] += 1;
                }
                break;
            };
            costs
        } else {
            costs(workers)
        };
        let capital = if let Some(context) = supply.as_ref() {
            working_capital_assets_at_prices(
                building,
                profile,
                freight_capital,
                |resource| context.supply.input_price(resource),
                |resource| sales.output_price(resource),
            )
        } else {
            imported_capital
        };
        if costs.working_capital > capital {
            #[cfg(test)]
            if trace.enabled {
                trace.counts[2] += 1;
            }
            continue;
        }
        let activity = market_activity.min(workers as f32 / physical.max(1) as f32);
        let revenue = sales.revenue(activity);
        if revenue > costs.total() {
            #[cfg(test)]
            if let Some(plan) = &mut trace.import_plan {
                plan.workers = workers;
                plan.required = costs.working_capital.to_bits();
            }
            #[cfg(test)]
            if trace.enabled {
                trace.counts[5] += 1;
            }
            *headroom = [revenue - costs.total(), capital - costs.working_capital];
            if workers == desired
                && let Some(record) = proof.as_mut()
            {
                record
                    .row
                    .remember(key, activity, *headroom, costs.working_capital);
            }
            if let Some(context) = supply.as_mut() {
                context.supply.accept_growing_for(context.buyer);
            }
            return (workers, activity);
        }
        #[cfg(test)]
        if trace.enabled {
            trace.counts[3] += 1;
        }
    }
    if let Some(context) = supply.as_mut() {
        context.supply.reject(context.allocator, context.graph);
    }
    retained
}

// Household quotas and raw staffed business needs share an output. The business amount is
// only an upper bound here: joint financing and reachability must prove it before publication.
fn provisional_local_share(
    profile: &EconomyProfileRuntime,
    area: f32,
    resource: u16,
    household: Option<&[f64]>,
    market: &[[f64; 2]],
) -> f32 {
    let Some((port, output)) = profile
        .outputs
        .iter()
        .enumerate()
        .find(|(_, output)| output.resource_runtime_id == resource)
    else {
        return 0.0;
    };
    let capacity = f64::from(profile.net_output_units_per_day(output) * area);
    if capacity <= 0.0 {
        return 0.0;
    }
    let household = household.map_or(0.0, |units| units[port].max(0.0));
    let business = if profile.kind == EconomyProfileRuntimeKind::ServiceStore {
        0.0
    } else {
        market.get(resource as usize).map_or(0.0, |total| total[1])
    };
    ((household + business) / capacity).clamp(0.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    // A service registered in the census window keeps its fiscal ceiling at publication when
    // a building appended after it has none (`SIM-02-I`).
    #[test]
    fn publication_adopts_a_pending_service_followed_by_an_appended_building() {
        let mut census = JobFunding {
            service_capacities: vec![1, 2],
            ..JobFunding::default()
        };
        let live = JobFunding {
            service_capacities: vec![1, 2, 7],
            ..JobFunding::default()
        };
        census.adopt_pending_services(&live, 4);
        assert_eq!(census.service_capacities, [1, 2, 7]);
        // Nothing past the published buildings, nothing already covered.
        census.service_capacities.truncate(2);
        census.adopt_pending_services(&live, 2);
        assert_eq!(census.service_capacities, [1, 2]);
    }

    #[test]
    fn local_business_sales_use_seller_prices_while_exports_use_the_outside_tariff() {
        use crate::simulation::buildings::allocator::indexed_test_building;
        use crate::simulation::economy::definitions::load_runtime_economy_catalog;
        use crate::simulation::zoning::ZoneType;

        let catalog = load_runtime_economy_catalog().unwrap();
        let mut profile = catalog
            .profile_for_id("food_processor_basic")
            .unwrap()
            .clone();
        let mut building = indexed_test_building(String::new(), ZoneType::Industrial, 0);
        building.operating_budget = 1_000_000.0;
        let physical = profile.worker_capacity;
        let prices = OperationPrices {
            utilities: [0.0; 3],
            property_tax: 0.0,
            imports: 1.0,
            carrier_quote: 0.0,
            truck_payload: 20_000.0,
        };
        let mut market = vec![[0.0; 2]; catalog.resource_count() + 1];
        for output in &profile.outputs {
            let units = f64::from(profile.net_output_units_per_day(output));
            market[output.resource_runtime_id as usize] = [units, units];
        }
        profile.unit_price_currency = 0.0;
        let mut headroom = [0.0; 2];
        assert_eq!(
            funded_private_operation(
                &building,
                &profile,
                &catalog,
                physical,
                prices,
                &market,
                None,
                0.0,
                None,
                None,
                &mut headroom,
                physical,
                1.0,
                None
            ),
            (0, 0.0)
        );
        assert_eq!(headroom, [0.0; 2]);
        profile.unit_price_currency = 1_000.0;
        assert_eq!(
            funded_private_operation(
                &building,
                &profile,
                &catalog,
                physical,
                prices,
                &market,
                None,
                0.0,
                None,
                None,
                &mut headroom,
                physical,
                1.0,
                None
            ),
            (physical, 1.0)
        );
        assert!(headroom[0] > 0.0 && headroom[1] >= 0.0);
        market.iter_mut().for_each(|total| total[1] = 0.0);
        let expensive = funded_private_operation(
            &building,
            &profile,
            &catalog,
            physical,
            prices,
            &market,
            Some(1.0),
            0.0,
            None,
            None,
            &mut headroom,
            physical,
            1.0,
            None,
        );
        profile.unit_price_currency = 0.0;
        assert_eq!(
            funded_private_operation(
                &building,
                &profile,
                &catalog,
                physical,
                prices,
                &market,
                Some(1.0),
                0.0,
                None,
                None,
                &mut headroom,
                physical,
                1.0,
                None
            ),
            expensive
        );
        assert!(expensive.0 > 0);
    }

    #[test]
    fn undo_remap_grows_prepared_funding_without_allocating_unprepared_views() {
        let mapping = [(0, 2)].into();
        let mut jobs = JobFunding {
            capacities: vec![7, 3],
            activities: vec![0.25, 0.4],
            service_capacities: vec![7, 0],
            freight_capital: vec![9.0, 2.0],
            ..Default::default()
        };
        jobs.remap_buildings(&mapping);
        assert_eq!(jobs.capacities, [0, 3, 7]);
        assert_eq!(jobs.activities, [0.0, 0.4, 0.25]);
        assert_eq!(jobs.service_capacities, [0, 0, 7]);
        assert_eq!(jobs.freight_capital, [0.0, 2.0, 9.0]);
        jobs.clear();
        jobs.remap_buildings(&mapping);
        assert!(jobs.capacities.is_empty());
        assert!(jobs.activities.is_empty());
        assert!(jobs.service_capacities.is_empty());
        assert!(jobs.freight_capital.is_empty());
    }

    #[test]
    fn funded_jobs_follow_remapped_buildings_and_clear_removed_slots() {
        let mut jobs = JobFunding {
            capacities: vec![3, 7, 11],
            activities: vec![0.25, 0.4, 0.5],
            service_capacities: vec![0, 7, 0],
            freight_capital: vec![2.0, 5.0, 9.0],
            ..Default::default()
        };
        jobs.remap_buildings(&[(0, 1), (1, 0)].into());
        assert_eq!(jobs.capacities, [7, 3, 11]);
        assert_eq!(jobs.activities, [0.4, 0.25, 0.5]);
        assert_eq!(jobs.service_capacities, [7, 0, 0]);
        assert_eq!(jobs.freight_capital, [5.0, 2.0, 9.0]);
        jobs.invalidate_building(0);
        jobs.remap_buildings(&[(2, 0)].into());
        assert_eq!(jobs.capacities, [11, 3, 0]);
        assert_eq!(jobs.activities, [0.5, 0.25, 0.0]);
        assert_eq!(jobs.service_capacities, [0, 0, 0]);
        assert_eq!(jobs.freight_capital, [9.0, 2.0, 0.0]);
        jobs.clear();
        assert!(jobs.capacities.is_empty());
        assert!(jobs.activities.is_empty());
        assert!(jobs.service_capacities.is_empty());
        assert!(jobs.freight_capital.is_empty());
    }
}
