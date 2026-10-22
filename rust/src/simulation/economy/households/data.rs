// SPDX-License-Identifier: GPL-2.0-only

//! Household record storage and building-reference maintenance.

use rayon::prelude::*;
use std::sync::atomic::AtomicU32;

use super::metrics::household_supply_resource_runtime_id;
use super::replenishment::{
    REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE, clear_replenishment_request,
    register_replenishment_failure,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::handles::{Handle, HandleTable};
use crate::simulation::core::request_queue::RequestQueue;
use crate::simulation::economy::agents::AgentHandle;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};

/// Daily household accounting, including collected utility payments awaiting settlement.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DailyHouseholdLedger {
    /// Household budget at the start of the current debug ledger window.
    pub(crate) budget_before: f32,
    /// Household budget after daily settlement.
    pub(crate) budget_after: f32,
    /// Wage income received during daily settlement.
    pub(crate) wage_income: f32,
    /// Unemployment benefit received during daily settlement.
    pub(crate) unemployment_benefit_income: f32,
    /// Pension transfer received during daily settlement.
    pub(crate) pension_income: f32,
    /// Child-support transfer received during daily settlement.
    pub(crate) child_support_income: f32,
    /// Eligible transfers before treasury cash limits, across all benefit categories.
    pub(crate) transfer_claim: f32,
    /// Residential property tax paid during daily settlement.
    pub(crate) property_tax_paid: f32,
    /// Net household budget reserved or spent on shopping during the ledger window.
    pub(crate) shopping_spend: f32,
    /// Value of household stock consumed from the home buffer.
    pub(crate) household_supply_consumption_cost: f32,
    /// Residential electricity charge paid from the household budget.
    pub(crate) power_consumption_cost: f32,
    /// Delivered electricity independent of cash; billed at daily settlement.
    pub(crate) electricity: crate::simulation::economy::electricity::ElectricityUsage,
    /// Unpaid electricity after settlement, included in household distress reporting.
    pub(crate) power_unpaid_cost: f32,
    /// Collected electricity payments attributed to local generation, excluding unpaid bills.
    pub(crate) power_local_payment: f32,
    /// Residential water-service charge paid from the household budget.
    pub(crate) water_consumption_cost: f32,
    /// Residential sewage-service charge paid from the household budget.
    pub(crate) sewage_consumption_cost: f32,
    /// Unemployed adult count observed by the daily benefit pass.
    pub(crate) unemployed_adults: u16,
    /// Members paid a wage in this window; the household's other adults draw unemployment
    /// benefit at its daily phase (`SIM-02-G`).
    pub(crate) paid_workers: u16,
    /// Shopper tasks completed during the ledger window.
    pub(crate) shopper_trips_completed: u16,
    /// Shopper tasks failed during the ledger window.
    pub(crate) shopper_trips_failed: u16,
}

impl DailyHouseholdLedger {
    /// Total city transfer income received during the current ledger window.
    pub(crate) fn transfer_income(self) -> f32 {
        self.unemployment_benefit_income + self.pension_income + self.child_support_income
    }
}

/// Completed daily electricity settlement summary kept for city budget reporting.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct DailyPowerSettlementSummary {
    /// Total electricity demand considered by the daily utility settlement.
    pub(crate) demand_units: f32,
    /// Total local electricity units available from active providers.
    pub(crate) supply_units: f32,
    /// Electricity demand served from local providers.
    pub(crate) served_units: f32,
    /// Local electricity coverage ratio in `0.0..=1.0`.
    pub(crate) coverage: f32,
    /// Household payments routed to local city providers.
    pub(crate) household_local_revenue: f32,
    /// Private business payments routed to local city providers.
    pub(crate) private_local_revenue: f32,
    /// City-service payments routed to local city providers.
    pub(crate) city_service_local_cost: f32,
    /// Local utility revenue across all settled services.
    pub(crate) utility_local_revenue: f32,
    /// City-service payments routed to local providers across all settled services.
    pub(crate) city_service_utility_local_cost: f32,
    /// City-service payments routed to OWA fallback across all settled services.
    pub(crate) city_service_utility_owa_cost: f32,
    /// Plant auxiliary electricity fees that are internal transfers, not external sales.
    pub(crate) power_auxiliary_local_cost: f32,
    /// Imported auxiliary electricity and plant water/sewage upkeep paid outside the power service.
    pub(crate) power_upkeep_utility_cost: f32,
}

/// Actual city-funded payroll settled during the latest fiscal day.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct CityServiceWages {
    /// Wages paid by the treasury across all city services.
    pub(crate) total: f32,
    /// Portion paid to power-plant employees, used by the electricity ledger.
    pub(crate) power: f32,
}

/// City-wide flows of one calendar day, posted as each entity settles at its own daily phase
/// (`SIM-02-G`, `docs/simulation.md#phases`). The calendar close keeps the finished day for the
/// budget report and the next day's pooled reads, then starts an empty one. O(1) per posting.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CityDayTotals {
    /// Utility and electricity bills: local receipts, city-service costs and delivered units.
    pub(crate) power: DailyPowerSettlementSummary,
    /// Wages the treasury paid to city-service workers.
    pub(crate) city_service_wages: CityServiceWages,
    /// Gross wages paid by private employers.
    pub(crate) private_payroll: f64,
    /// Unemployment benefits paid from the treasury.
    pub(crate) unemployment_benefits: f64,
    /// Pensions paid from the treasury.
    pub(crate) pensions: f64,
    /// Child support paid from the treasury.
    pub(crate) child_support: f64,
    /// Coal burned by power plants in the day's hourly dispatches.
    pub(crate) coal_consumed_kg: f64,
    /// Operational providers that settled, by utility service; the next day's water and sewage
    /// coverage reads it.
    pub(crate) utility_providers: [u32; 3],
    /// Local water and sewage revenue by service, shared among the next day's providers.
    pub(crate) utility_local_revenue: [f32; 3],
}

impl CityDayTotals {
    /// Treasury transfers paid to households during the day.
    pub(crate) fn transfers(&self) -> f64 {
        self.unemployment_benefits + self.pensions + self.child_support
    }
}

/// Explicit household runtime record anchored to a residential building.
#[derive(Clone, Debug)]
pub struct Household {
    /// Residential building currently anchoring the household.
    pub home_building_id: usize,
    /// Unpaid local/OWA electricity bills carried into the next settlement.
    pub(crate) power_arrears: crate::simulation::economy::electricity::ElectricityBill,
    /// Shared household budget used for essentials in the first-pass loop.
    pub budget: f32,
    /// Current household stock buffer in `household_supplies`.
    pub stock: f32,
    /// Cached member count, recounted from the household's members at its hourly phase.
    pub member_count: u16,
    /// Cached number of child residents, recounted at the household's hourly phase.
    pub child_count: u16,
    /// Cached number of adult residents, recounted at the household's hourly phase.
    pub adult_count: u16,
    /// Assigned adults, kept by the job mutators and recounted at the household's hourly
    /// phase; distinct from funded vacancies.
    pub(crate) employed_adults: u16,
    /// Expected gross daily wages at the household's latest hourly phase; unpaid or inactive
    /// jobs contribute nothing.
    pub(crate) gross_daily_wages: f32,
    /// Whether an adult/elder member can shop by car; recounted at the household's hourly
    /// phase and rebuilt from saved resident agents on load.
    pub(crate) shopping_has_car: bool,
    /// Cached number of elder residents, recounted at the household's hourly phase.
    pub elder_count: u16,
    /// Baseline daily consumption in `household_supplies / day / resident`.
    pub consumption_rate: f32,
    /// Cached derived stock horizon in days at the current consumption rate.
    pub stock_days: f32,
    /// Current replenishment state for diagnostics and cooldown handling.
    pub replenishment_state: u8,
    /// Remaining operational-hour cooldown steps before another replenishment retry.
    pub cooldown_hours: u16,
    /// Consecutive failed replenishment attempts for the current shortage.
    pub replenishment_failure_count: u16,
    /// Reserved source building for the current replenishment request, if any.
    pub reserved_store_building_id: usize,
    /// Reserved amount carried by the active household shopper.
    pub reserved_amount: f32,
    /// Reserved budget waiting to be transferred to the supplying store.
    pub reserved_total_cost: f32,
    /// Selected household member carrying the active shopping request; stale once it leaves.
    pub(crate) shopping_agent: AgentHandle,
    /// Remaining operational-hour budget for the current shopping leg.
    pub shopping_timeout_hours_remaining: u16,
    /// Deterministic store-search continuation cursor used after failed replenishment attempts.
    pub replenishment_search_cursor: u32,
    /// Consecutive daily stay-rule failures for the current home.
    pub stay_failure_days: u32,
    /// Consecutive settled days with no valid home after the daily rehousing attempt.
    pub unhoused_days_elapsed: u32,
    /// Stable authored cadence offset used for periodic replenishment checks.
    pub replenishment_offset_hours: u16,
    /// Days elapsed with at least one unemployed adult. Resets to 0 when all adult members are
    /// employed. Incremented each daily tick while the household is benefit-eligible. Once
    /// this reaches `unemployment_max_days`, the household becomes emigration-eligible and
    /// benefit payments stop.
    pub unemployment_days_elapsed: u32,
}

/// Incoming households counted during the existing arrival-carrier pass.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PendingHouseholdArrivals {
    /// Homes already claimed by carriers which have not arrived.
    pub(crate) households: u32,
    /// Incoming adults which share the labor pool with existing unemployed residents.
    pub(crate) adults: u32,
    /// Incoming pension claimants.
    pub(crate) elders: u32,
    /// Incoming child-support claimants.
    pub(crate) children: u32,
}

/// Collection of explicit household records for the live simulation.
pub struct HouseholdSystem {
    /// All known households. Agents reference these by index.
    pub households: Vec<Household>,
    /// Stable handles of the dense households; saved with the store.
    pub(crate) handles: HandleTable<Household>,
    /// Derived before each hourly demand pass; rebuilt from persisted arrival carriers.
    pub(crate) pending_arrivals: PendingHouseholdArrivals,
    /// Households whose shopper was reserved and held since SimCore last paced departures.
    pub(crate) held_shoppers: Vec<Handle<Household>>,
    /// Service visitors held since SimCore last paced departures.
    pub(crate) held_visitors: Vec<AgentHandle>,
    pub(crate) job_funding: crate::simulation::economy::hiring::JobFunding,
    /// The hourly census's rolling copy of the buildings and households it reads (`SIM-02-I`);
    /// saved.
    pub(crate) census_inputs: crate::simulation::economy::hiring::CensusInputs,
    /// The census built over each hour's second half and published at the next hour's start
    /// (`SIM-02-I`); saved with the input copy.
    pub(crate) census_run: crate::simulation::economy::hiring::RollingCensus,
    /// Advance whenever building or household indices shift or are reused: swap-removes,
    /// remaps, clears and undo restores (`SIM-02-I`). Appends and in-place changes keep them,
    /// so a census stays valid for the indices it covers. Not saved: a loaded game counts
    /// afresh.
    pub(crate) structure: crate::simulation::economy::hiring::StructureRevisions,
    /// Advances at every change of the policy the census's copy freezes (`SIM-02-I`); part of
    /// the census key. Not saved: a loaded game counts afresh.
    pub(crate) census_policy_revision: u64,
    pub(super) shopping_car_scratch: Vec<AtomicU32>,
    pub(super) removal_candidates_scratch: Vec<(usize, f32, f32)>,
    pub(super) removal_agent_indices_scratch: Vec<usize>,
    pub(super) workplace_route_cache: super::employment::WorkplaceRouteCache,
    // The quotes in insertion order with the workplace pass that added each; a quote expires
    // a day of passes later, so the cache holds about one day's searches (`SIM-02-G`).
    pub(super) workplace_route_cache_order:
        std::collections::VecDeque<(u32, super::employment::WorkplaceRouteCacheKey)>,
    pub(super) workplace_route_cache_pass: u32,
    pub(super) workplace_route_cache_building_revision: u64,
    pub(super) workplace_route_cache_entrance_revision: u64,
    pub(super) workplace_route_cache_cch_generation: u32,
    // Network legs of workplace commutes; exact across hours while the CCH revision holds.
    pub(super) workplace_network_quotes:
        crate::simulation::economy::agents::tick::SharedNetworkQuotes,
    // Sorted workplace keys keep the same scratch owner regardless of Rayon scheduling.
    pub(super) workplace_query_shards: Vec<super::employment::WorkplaceQueryShard>,
    pub(super) daily_ledgers: Vec<DailyHouseholdLedger>,
    /// Flows of the calendar day so far (`SIM-02-G`).
    pub(crate) day_totals: CityDayTotals,
    /// Flows of the last closed calendar day, removed buildings included.
    pub(crate) last_day_totals: CityDayTotals,
    /// Coal stocked at power plants at the latest hourly dispatch, for the budget report.
    pub(crate) coal_inventory_kg: f64,
    /// Households whose daily phase asked for a housing review (`SIM-02-G`), served by the
    /// `housing requests` queue after the rehousings (`SIM-02-J`). Saved in order.
    pub(crate) housing_reviews: RequestQueue<Household>,
    /// Households whose hourly phase found them without a usable home, served by the
    /// `housing requests` queue first (`SIM-02-J`). Saved in order.
    pub(crate) rehousing: RequestQueue<Household>,
    // Outcomes of the housing requests served this hour, logged at its last slot.
    pub(super) housing_diagnostics: super::relocation::HousingResolutionDiagnostics,
    /// Households that chose to leave at their daily phase, oldest first, removed by the
    /// `departures` queue in the hour's removal window (`SIM-02-J`). Saved in order.
    pub(crate) exits: std::collections::VecDeque<Handle<Household>>,
    /// Households the hour's actions slot admitted, in admission order, until the `admissions`
    /// queue creates them and their arrival carriers (`SIM-02-J`). Saved in order.
    pub(crate) admission_requests: std::collections::VecDeque<super::admission::QueuedAdmission>,
    /// Slots of admission credit: each slot adds [`Self::admission_rate`] and each admission
    /// takes `SLOTS_PER_HOUR`, so the hour's batch is created over the hour (`SIM-02-J`). Saved.
    pub(crate) admission_credit: u32,
    /// Admissions waiting when the hour's actions slot decided its batch. Saved.
    pub(crate) admission_rate: u32,
    // The `admissions` and `departures` queues' service this hour, logged at its last slot.
    pub(crate) admission_stats: crate::simulation::core::request_queue::QueueStats,
    pub(crate) departure_stats: crate::simulation::core::request_queue::QueueStats,
    /// Households whose hourly phase called for a restock, served by the `restock requests`
    /// queue (`SIM-02-J`). Saved in order.
    pub(crate) restock_requests: RequestQueue<Household>,
    /// Households whose hourly phase drew a visible service visit, served by the
    /// `visit requests` queue (`SIM-02-J`). Saved in order.
    pub(crate) visit_requests: RequestQueue<Household>,
    /// Slots of visit credit, one visit per `SLOTS_PER_HOUR` (`SIM-02-J`). Saved.
    pub(crate) visit_credit: u32,
    /// Shopper departures planned per minute, which levels them (`SIM-02-J`). Saved.
    pub(crate) shopper_departures: crate::simulation::core::scheduler::MinuteLeveler,
    /// Foot and car components that destination entries are bucketed by.
    pub(crate) destination_topology: super::destinations::HouseholdTopology,
    /// The rolling index of stores and service stores restock and visit requests read
    /// (`SIM-02-J`). Saved.
    pub(crate) destinations: super::destinations::DestinationRoll,
    /// Agents whose hourly phase let them look for work, served by the `job requests` queue
    /// (`SIM-02-J`). Saved in order.
    pub(crate) job_requests: RequestQueue<crate::simulation::economy::agents::Agent>,
    /// The open workplaces this hour's job searches read, swept over the hour before
    /// (`SIM-02-J`). Saved.
    pub(crate) job_board: super::job_board::JobBoardRoll,
    // The job board's network side, derived and rebuilt when its key changes.
    pub(super) job_supply: Option<super::employment::JobSupplySnapshot>,
    // Reused batches of the slot's restock, visit and job requests.
    pub(super) restock_batch: Vec<(u64, usize)>,
    pub(super) visit_batch: Vec<usize>,
    pub(super) job_batch: Vec<usize>,
    // This hour's job search work: queue service, home keys, plans and milliseconds.
    pub(super) job_queue_stats: super::requests::QueueStats,
    pub(super) job_hour: (u32, u32, f64),
    // Outcomes of this hour's restock and visit requests, logged at its last slot.
    pub(super) replenishment_hour: super::replenishment::ReplenishmentDiagnostics,
    pub(super) restock_queue_stats: super::requests::QueueStats,
    pub(super) visit_hour: super::service_visits::ServiceVisitDiagnostics,
    pub(super) visit_queue_stats: super::requests::QueueStats,
    /// The last finished hour's electricity dispatch and coverage, and this hour's totals so
    /// far (`SIM-02-I`). Saved.
    pub(crate) electricity_hour: super::electricity::ElectricityHour,
    /// Service-store sales aggregates and their money pool between hourly phases (`SIM-02-I`).
    /// Saved.
    pub(crate) service_sales: super::service_sales::ServiceSales,
    /// Closed building accounts retained until the current fiscal day is reported.
    pub(crate) removed_electricity: super::electricity::RemovedBuildingElectricity,
}

/// Household records changed when one building is removed.
pub(crate) struct HouseholdBuildingUndo {
    pub(crate) records: Vec<(usize, Household, DailyHouseholdLedger)>,
    pub(crate) mutated_building_ids: Vec<usize>,
    removed_electricity: super::electricity::RemovedBuildingElectricity,
}

impl HouseholdSystem {
    /// Creates an empty household system.
    pub fn new() -> Self {
        Self {
            households: Vec::new(),
            handles: HandleTable::default(),
            pending_arrivals: PendingHouseholdArrivals::default(),
            held_shoppers: Vec::new(),
            held_visitors: Vec::new(),
            job_funding: Default::default(),
            census_inputs: Default::default(),
            census_run: Default::default(),
            structure: Default::default(),
            census_policy_revision: 0,
            shopping_car_scratch: Vec::new(),
            removal_candidates_scratch: Vec::new(),
            removal_agent_indices_scratch: Vec::new(),
            workplace_route_cache: Default::default(),
            workplace_route_cache_order: Default::default(),
            workplace_route_cache_pass: 0,
            workplace_route_cache_building_revision: u64::MAX,
            workplace_route_cache_entrance_revision: u64::MAX,
            workplace_route_cache_cch_generation: u32::MAX,
            workplace_network_quotes: Default::default(),
            workplace_query_shards: Vec::new(),
            daily_ledgers: Vec::new(),
            day_totals: CityDayTotals::default(),
            last_day_totals: CityDayTotals::default(),
            coal_inventory_kg: 0.0,
            housing_reviews: RequestQueue::default(),
            rehousing: RequestQueue::default(),
            housing_diagnostics: Default::default(),
            exits: std::collections::VecDeque::new(),
            admission_requests: std::collections::VecDeque::new(),
            admission_credit: 0,
            admission_rate: 0,
            admission_stats: Default::default(),
            departure_stats: Default::default(),
            restock_requests: RequestQueue::default(),
            visit_requests: RequestQueue::default(),
            visit_credit: 0,
            shopper_departures: Default::default(),
            destination_topology: Default::default(),
            destinations: Default::default(),
            job_requests: RequestQueue::default(),
            job_board: Default::default(),
            job_supply: None,
            restock_batch: Vec::new(),
            visit_batch: Vec::new(),
            job_batch: Vec::new(),
            job_queue_stats: Default::default(),
            job_hour: Default::default(),
            replenishment_hour: Default::default(),
            restock_queue_stats: Default::default(),
            visit_hour: Default::default(),
            visit_queue_stats: Default::default(),
            electricity_hour: Default::default(),
            service_sales: Default::default(),
            removed_electricity: Default::default(),
        }
    }

    /// Captures only household and store records that building invalidation can mutate.
    pub(crate) fn capture_building_undo(
        &mut self,
        removed_building: usize,
    ) -> HouseholdBuildingUndo {
        self.ensure_daily_ledger_len();
        let mut records = Vec::new();
        let mut mutated_building_ids = Vec::new();
        for (household_id, household) in self.households.iter().enumerate() {
            let removed_home = household.home_building_id == removed_building;
            let removed_store = household.reserved_store_building_id == removed_building;
            if !removed_home && !removed_store {
                continue;
            }
            records.push((
                household_id,
                household.clone(),
                self.daily_ledgers[household_id],
            ));
            if household.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE
                && household.reserved_store_building_id != removed_building
            {
                mutated_building_ids.push(household.reserved_store_building_id);
            }
        }
        mutated_building_ids.sort_unstable();
        mutated_building_ids.dedup();
        HouseholdBuildingUndo {
            records,
            mutated_building_ids,
            removed_electricity: self.removed_electricity,
        }
    }

    /// Restores records captured by [`Self::capture_building_undo`].
    pub(crate) fn restore_building_undo(&mut self, undo: HouseholdBuildingUndo) {
        // Restored records replace households in place; the census copy cannot follow them.
        self.census_inputs.invalidate();
        self.note_building_structure_change();
        self.removed_electricity = undo.removed_electricity;
        self.ensure_daily_ledger_len();
        for (household_id, household, ledger) in undo.records {
            if let Some(target) = self.households.get_mut(household_id) {
                *target = household;
            }
            if let Some(target) = self.daily_ledgers.get_mut(household_id) {
                *target = ledger;
            }
        }
        self.invalidate_workplace_route_cache();
    }

    // A building index shifts or is reused (`SIM-02-I`). O(1).
    pub(super) fn note_building_structure_change(&mut self) {
        self.structure.buildings = self.structure.buildings.wrapping_add(1);
        crate::debug::work::add(crate::debug::work::Work::StructuralChanges, 1);
    }

    // A household index shifts or is reused (`SIM-02-I`). O(1).
    pub(super) fn note_household_structure_change(&mut self) {
        self.structure.households = self.structure.households.wrapping_add(1);
        crate::debug::work::add(crate::debug::work::Work::StructuralChanges, 1);
    }

    fn invalidate_workplace_route_cache(&mut self) {
        self.workplace_route_cache.clear();
        self.workplace_route_cache_order.clear();
        self.workplace_route_cache_building_revision = u64::MAX;
        self.workplace_route_cache_entrance_revision = u64::MAX;
        self.workplace_route_cache_cch_generation = u32::MAX;
    }

    /// Returns the topology revisions attached to the complete workplace quotes.
    pub(crate) fn workplace_route_cache_revisions(&self) -> (u64, u64, u32) {
        (
            self.workplace_route_cache_building_revision,
            self.workplace_route_cache_entrance_revision,
            self.workplace_route_cache_cch_generation,
        )
    }

    /// Iterates complete workplace quotes with the pass that added each, for deterministic
    /// save serialization.
    pub(crate) fn workplace_route_cache_entries(
        &self,
    ) -> impl Iterator<Item = ((usize, usize, bool), Option<u16>, u32)> + '_ {
        self.workplace_route_cache_order
            .iter()
            .map(|&(pass, key)| (key, self.workplace_route_cache[&key], pass))
    }

    /// The workplace pass counter quotes are stamped with.
    pub(crate) fn workplace_route_cache_pass(&self) -> u32 {
        self.workplace_route_cache_pass
    }

    /// Restores complete workplace quotes after the loaded topology has been rebuilt.
    pub(crate) fn restore_workplace_route_cache(
        &mut self,
        building_revision: u64,
        entrance_revision: u64,
        cch_generation: u32,
        mut entries: Vec<(super::employment::WorkplaceRouteCacheKey, Option<u16>, u32)>,
        pass: u32,
    ) {
        // Quotes of one pass expire together, so their order within it is immaterial.
        entries.sort_unstable_by_key(|&(key, _, stamp)| (pass.wrapping_sub(stamp), key));
        entries.reverse();
        self.workplace_route_cache.clear();
        self.workplace_route_cache_order.clear();
        self.workplace_route_cache.reserve(entries.len());
        for (key, value, stamp) in entries {
            self.workplace_route_cache.insert(key, value);
            self.workplace_route_cache_order.push_back((stamp, key));
        }
        self.workplace_route_cache_pass = pass;
        self.workplace_route_cache_building_revision = building_revision;
        self.workplace_route_cache_entrance_revision = entrance_revision;
        self.workplace_route_cache_cch_generation = cch_generation;
    }

    /// Clears all households.
    pub fn clear(&mut self) {
        self.households.clear();
        self.job_funding.clear();
        self.census_inputs.invalidate();
        self.census_run.abandon();
        self.note_building_structure_change();
        self.note_household_structure_change();
        self.pending_arrivals = PendingHouseholdArrivals::default();
        self.held_shoppers.clear();
        self.held_visitors.clear();
        self.removal_candidates_scratch.clear();
        self.removal_agent_indices_scratch.clear();
        self.invalidate_workplace_route_cache();
        self.workplace_network_quotes = Default::default();
        self.workplace_query_shards.clear();
        self.daily_ledgers.clear();
        self.day_totals = CityDayTotals::default();
        self.last_day_totals = CityDayTotals::default();
        self.coal_inventory_kg = 0.0;
        self.housing_reviews.clear();
        self.rehousing.clear();
        self.housing_diagnostics = Default::default();
        self.exits.clear();
        self.admission_requests.clear();
        self.admission_credit = 0;
        self.admission_rate = 0;
        self.admission_stats = Default::default();
        self.departure_stats = Default::default();
        self.restock_requests.clear();
        self.visit_requests.clear();
        self.visit_credit = 0;
        self.shopper_departures = Default::default();
        self.destinations = Default::default();
        self.replenishment_hour = Default::default();
        self.restock_queue_stats = Default::default();
        self.visit_hour = Default::default();
        self.visit_queue_stats = Default::default();
        self.job_requests.clear();
        self.job_board = Default::default();
        self.job_supply = None;
        self.job_queue_stats = Default::default();
        self.job_hour = Default::default();
        self.electricity_hour = Default::default();
        self.service_sales = Default::default();
        self.removed_electricity = Default::default();
    }

    /// Households whose home or reserved store is building `first` or `second`, ascending, in
    /// one scan on the pool (`SIM-02-K`): a removal of `first` that moves building `second`
    /// into its index finds here every household its invalidation and remap change, and
    /// passes them to [`Self::invalidate_building_among`] and [`Self::remap_building_among`].
    /// O(H/p).
    pub(crate) fn households_referencing_buildings(
        &self,
        first: usize,
        second: usize,
    ) -> Vec<usize> {
        crate::thread_pools::install(|| {
            self.households
                .par_iter()
                .enumerate()
                .filter(|(_, household)| {
                    [
                        household.home_building_id,
                        household.reserved_store_building_id,
                    ]
                    .iter()
                    .any(|&building| building == first || building == second)
                })
                .map(|(household_id, _)| household_id)
                .collect()
        })
    }

    /// [`Self::remap_building_indices`] for one swap-remove moving building `old` to `new`,
    /// whose household loop visits the ascending `candidates` alone: every household naming
    /// `old` must be among them, as [`Self::households_referencing_buildings`] found them
    /// before the removal's invalidation (`SIM-02-K`). O(candidates) plus the census books'
    /// own remaps.
    pub(crate) fn remap_building_among(&mut self, old: usize, new: usize, candidates: &[usize]) {
        let mapping = std::collections::HashMap::from([(old, new)]);
        self.note_building_structure_change();
        self.job_funding.remap_buildings(&mapping);
        self.census_inputs.remap_buildings(&mapping);
        self.ensure_daily_ledger_len();
        for &household_id in candidates {
            let household = &mut self.households[household_id];
            if household.home_building_id == old {
                household.home_building_id = new;
            }
            if household.reserved_store_building_id == old {
                household.reserved_store_building_id = new;
            }
        }
        // Test builds: no household outside the candidates named `old`.
        #[cfg(test)]
        if crate::debug::work::city_checks() {
            assert!(
                self.households_referencing_buildings(old, old).is_empty(),
                "a household outside the removal's candidates named the moved building"
            );
        }
    }

    /// Remaps building references after a building swap-remove.
    pub fn remap_building_indices(&mut self, mapping: &std::collections::HashMap<usize, usize>) {
        if !mapping.is_empty() {
            self.note_building_structure_change();
        }
        self.job_funding.remap_buildings(mapping);
        self.census_inputs.remap_buildings(mapping);
        self.ensure_daily_ledger_len();
        if mapping.len() == 1 {
            let (&old, &new) = mapping.iter().next().unwrap();
            self.households.par_iter_mut().for_each(|household| {
                if household.home_building_id == old {
                    household.home_building_id = new;
                }
                if household.reserved_store_building_id == old {
                    household.reserved_store_building_id = new;
                }
            });
        } else if !mapping.is_empty() {
            self.households.par_iter_mut().for_each(|household| {
                if let Some(&new_id) = mapping.get(&household.home_building_id) {
                    household.home_building_id = new_id;
                }
                if let Some(&new_id) = mapping.get(&household.reserved_store_building_id) {
                    household.reserved_store_building_id = new_id;
                }
            });
        }
    }

    /// Invalidates references to a building that is being removed.
    pub fn invalidate_building(
        &mut self,
        removed_building: usize,
        allocator: &mut BuildingAllocator,
    ) {
        self.invalidate_building_referencing(removed_building, None, allocator);
    }

    /// [`Self::invalidate_building`] for a removal that found its households with
    /// [`Self::households_referencing_buildings`] just before: only the ascending `candidates`
    /// naming `removed_building` are visited, which is every household that does
    /// (`SIM-02-K`). O(candidates) plus the census books' own invalidations.
    pub(crate) fn invalidate_building_among(
        &mut self,
        removed_building: usize,
        candidates: &[usize],
        allocator: &mut BuildingAllocator,
    ) {
        self.invalidate_building_referencing(removed_building, Some(candidates), allocator);
    }

    // Invalidates the households naming `removed_building`: among `candidates` when given,
    // else found by a scan of every household.
    fn invalidate_building_referencing(
        &mut self,
        removed_building: usize,
        candidates: Option<&[usize]>,
        allocator: &mut BuildingAllocator,
    ) {
        self.note_building_structure_change();
        self.job_funding.invalidate_building(removed_building);
        // Both removal paths swap-remove the building next (`SIM-02-I`).
        self.census_inputs
            .remove_building(removed_building, allocator);
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let household_supply_resource = household_supply_resource_runtime_id(&catalog);
        let retry_cooldown_hours = tuning
            .operational_clock
            .household_replenishment_retry_cooldown_hours;
        let terminal_failure_count = tuning
            .operational_clock
            .household_replenishment_terminal_failure_count;
        self.ensure_daily_ledger_len();
        // Only households whose home or reserved store is the removed building change; they
        // are found on the pool and updated in household order. O(H/p) (`SIM-02-K`).
        let names_removed = |household: &Household| {
            household.home_building_id == removed_building
                || household.reserved_store_building_id == removed_building
        };
        let referencing: Vec<usize> = match candidates {
            Some(candidates) => candidates
                .iter()
                .copied()
                .filter(|&household_id| names_removed(&self.households[household_id]))
                .collect(),
            None => crate::thread_pools::install(|| {
                self.households
                    .par_iter()
                    .enumerate()
                    .filter(|(_, household)| names_removed(household))
                    .map(|(household_id, _)| household_id)
                    .collect()
            }),
        };
        let daily_ledgers = &mut self.daily_ledgers;
        for household_id in referencing {
            let household = &mut self.households[household_id];
            let removed_home = household.home_building_id == removed_building;
            if removed_home {
                household.home_building_id = usize::MAX;
            }

            let removed_store = household.reserved_store_building_id == removed_building;
            let active_to_store = household.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE;
            let active_returning =
                household.replenishment_state == REPLENISHMENT_SHOPPING_RETURNING;

            if removed_store && active_returning && !removed_home {
                household.reserved_store_building_id = usize::MAX;
                continue;
            }

            if removed_store || removed_home {
                if active_to_store {
                    let store_idx = household.reserved_store_building_id;
                    if store_idx < allocator.buildings.len() && store_idx != removed_building {
                        allocator.buildings[store_idx].add_inventory_units(
                            household_supply_resource,
                            household.reserved_amount,
                        );
                    }
                    household.budget += household.reserved_total_cost;
                    if let Some(ledger) = daily_ledgers.get_mut(household_id) {
                        ledger.shopping_spend -= household.reserved_total_cost;
                    }
                }
                if active_to_store || active_returning {
                    if let Some(ledger) = daily_ledgers.get_mut(household_id) {
                        ledger.shopper_trips_failed = ledger.shopper_trips_failed.saturating_add(1);
                    }
                    clear_replenishment_request(household);
                    register_replenishment_failure(
                        household,
                        retry_cooldown_hours,
                        terminal_failure_count,
                    );
                }
            }
        }
    }

    /// Returns daily payments and diagnostic counters for settlement, saves and reporting.
    pub(crate) fn daily_ledgers(&self) -> &[DailyHouseholdLedger] {
        &self.daily_ledgers
    }

    /// Restores validated utility payments for an already loaded household without charging it.
    /// Payment order is power, water, sewage; all other ledger fields start a new report window.
    pub(crate) fn restore_utility_payments(&mut self, household_id: usize, paid: [f32; 3]) {
        self.ensure_daily_ledger_len();
        let ledger = &mut self.daily_ledgers[household_id];
        ledger.power_consumption_cost = paid[0];
        ledger.water_consumption_cost = paid[1];
        ledger.sewage_consumption_cost = paid[2];
    }

    /// Restores the number of members paid a wage in the household's open window.
    pub(crate) fn restore_paid_workers(&mut self, household_id: usize, paid_workers: u16) {
        self.ensure_daily_ledger_len();
        self.daily_ledgers[household_id].paid_workers = paid_workers;
    }

    /// Restores physical power meters and collected local payments without charging again.
    pub(crate) fn restore_electricity_account(
        &mut self,
        household_id: usize,
        usage: crate::simulation::economy::electricity::ElectricityUsage,
        unpaid: f32,
        local_payment: f32,
    ) {
        self.ensure_daily_ledger_len();
        let ledger = &mut self.daily_ledgers[household_id];
        ledger.electricity = usage;
        ledger.power_unpaid_cost = unpaid;
        ledger.power_local_payment = local_payment;
    }

    /// Returns the electricity and utility settlement of the last closed calendar day.
    pub(crate) fn last_power_settlement(&self) -> DailyPowerSettlementSummary {
        self.last_day_totals.power
    }

    /// Returns actual city payroll and its electricity subtotal for the last closed day.
    pub(crate) fn last_city_service_wages(&self) -> CityServiceWages {
        self.last_day_totals.city_service_wages
    }

    /// Ends the calendar day: the day's totals, with the bills of buildings removed during it,
    /// become the last day, and an empty day starts (`SIM-02-G`). O(1).
    pub(crate) fn close_calendar_day(&mut self) {
        let mut closed = std::mem::take(&mut self.day_totals);
        let removed = std::mem::take(&mut self.removed_electricity).settlement;
        let power = &mut closed.power;
        power.demand_units += removed.demand_units;
        power.supply_units += removed.supply_units;
        power.served_units += removed.served_units;
        power.household_local_revenue += removed.household_local_revenue;
        power.private_local_revenue += removed.private_local_revenue;
        power.city_service_local_cost += removed.city_service_local_cost;
        power.utility_local_revenue += removed.utility_local_revenue;
        power.city_service_utility_local_cost += removed.city_service_utility_local_cost;
        power.city_service_utility_owa_cost += removed.city_service_utility_owa_cost;
        power.power_auxiliary_local_cost += removed.power_auxiliary_local_cost;
        power.power_upkeep_utility_cost += removed.power_upkeep_utility_cost;
        power.coverage = if power.demand_units > 0.0 {
            (power.served_units / power.demand_units).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.last_day_totals = closed;
    }

    /// Ends household `household_id`'s 24-hour ledger window at its daily phase: the budget
    /// after settlement. O(1).
    pub(super) fn finish_household_window(&mut self, household_id: usize) {
        self.daily_ledgers[household_id].budget_after = self.households[household_id].budget;
    }

    /// Starts household `household_id`'s next ledger window after its settlement was reported.
    /// O(1).
    pub(crate) fn open_household_window(&mut self, household_id: usize) {
        let budget = self.households[household_id].budget;
        self.daily_ledgers[household_id] = DailyHouseholdLedger {
            budget_before: budget,
            budget_after: budget,
            ..Default::default()
        };
    }

    /// Starts every household's next ledger window at once, as if each had settled.
    #[cfg(test)]
    pub(crate) fn reset_daily_ledgers(&mut self) {
        self.ensure_daily_ledger_len();
        for household_id in 0..self.households.len() {
            self.open_household_window(household_id);
        }
    }

    pub(super) fn ensure_daily_ledger_len(&mut self) {
        if self.daily_ledgers.len() > self.households.len() {
            self.daily_ledgers.truncate(self.households.len());
        }
        if self.daily_ledgers.len() < self.households.len() {
            let start = self.daily_ledgers.len();
            self.daily_ledgers.reserve(self.households.len() - start);
            for household in &self.households[start..] {
                let ledger = DailyHouseholdLedger {
                    budget_before: household.budget,
                    budget_after: household.budget,
                    ..Default::default()
                };
                self.daily_ledgers.push(ledger);
            }
        }
    }

    /// Returns zeroed per-household car-eligibility flags for the load-time resident census.
    pub(super) fn reset_shopping_car_scratch(&mut self) -> &[AtomicU32] {
        resize_atomic_scratch(&mut self.shopping_car_scratch, self.households.len());
        &self.shopping_car_scratch
    }
}

impl std::fmt::Debug for HouseholdSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HouseholdSystem")
            .field("households", &self.households)
            .finish_non_exhaustive()
    }
}

impl Default for HouseholdSystem {
    fn default() -> Self {
        Self::new()
    }
}

fn resize_atomic_scratch(scratch: &mut Vec<AtomicU32>, len: usize) {
    if scratch.len() > len {
        scratch.truncate(len);
    }
    while scratch.len() < len {
        scratch.push(AtomicU32::new(0));
    }
    for slot in scratch {
        slot.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}
